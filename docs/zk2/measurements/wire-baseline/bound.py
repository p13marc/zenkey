#!/usr/bin/env python3
"""Where zenoh starts to fragment a put across rf0's face (#1136), from a bound.sh run.

Usage: bound.py <run-dir> [<run-dir> ...] [--json out.json]   (one run: its object; more: a list)

Reads each run's air.jsonl (the relay's capture, no layers), marks.json and
configs/params.json. Decodes every A->B SDU as zenoh 1.10.1 writes a batch on a
streamed link, its 2-byte length left out by the driver: a frame's header and
sequence number, each push's header, scope, extensions and put, and each
fragment chain, reassembled. Each put bound.sh made is found by its payload
(the byte of its index, repeated) and stated with what crossed:
- `frame`: its frame's header, the sequence number and its bytes, its QoS
  extension's bytes;
- `put`: the push and put headers, the message less its payload;
- `batch`: what the batch held, the 2-byte length included, against the
  run's `batch_size`, or the fragments it took.

Then, per QoS and sequence width, the largest payload that crossed in one
batch and the smallest that fragmented, and the allowance they bound:
`batch_size` less the largest that fit. Where the run's marks name the
routers' ids (bound.sh's ZIDS), it checks the initial sequence number router
A announced in its OpenAck against zenoh's `compute_sn` over the two ids (unicast/establishment/mod.rs; common/seq_num.rs
masks 16bit to 2^14 - 1 and 32bit to 2^28 - 1). Standard library only.
"""
import hashlib
import json
import sys

MASK = {"8bit": (1 << 7) - 1, "16bit": (1 << 14) - 1, "32bit": (1 << 28) - 1,
        "64bit": (1 << 32) - 1}


class R:
    def __init__(self, b):
        self.b, self.i = b, 0

    def u8(self):
        v = self.b[self.i]
        self.i += 1
        return v

    def vle(self):
        v, s, start = 0, 0, self.i
        while True:
            c = self.u8()
            v |= (c & 0x7F) << s
            if c < 0x80:
                return v, self.i - start
            s += 7

    def take(self, n):
        v = self.b[self.i:self.i + n]
        assert len(v) == n, "short"
        self.i += n
        return v

    def left(self):
        return len(self.b) - self.i


def exts(r, more):
    """Skip extensions while one follows; return their bytes and {id: Z64 value}."""
    start, vals = r.i, {}
    while more:
        h = r.u8()
        enc = (h >> 5) & 0b11
        if enc == 1:
            vals[h & 0x0F] = r.vle()[0]
        elif enc == 2:
            r.take(r.vle()[0])
        more = bool(h & 0x80)
    return r.i - start, vals


def push(r):
    """A push of a put: its header and scope, its extensions, and the put's payload length."""
    start = r.i
    h = r.u8()
    assert h & 0x1F == 0x1D, f"not a push: {h:#x}"
    scope, _ = r.vle()
    suffix = b""
    if h & 0x20:
        suffix = r.take(r.vle()[0])
    ext_bytes, _ = exts(r, bool(h & 0x80))
    body = r.u8()
    assert body & 0x1F == 0x01, f"not a put: {body:#x}"
    assert not body & 0x20, "a timestamp"
    assert not body & 0x40, "an encoding"
    exts(r, bool(body & 0x80))
    n, _ = r.vle()
    payload = r.take(n)
    return {"scope": scope, "suffix": suffix.decode(errors="replace"), "qos_ext": ext_bytes,
            "payload": n, "first": payload[0] if payload else None, "header": r.i - start - n}


def runs(rows):
    """Each A->B SDU, decoded: frames with their pushes, fragments, keep-alives, the OpenAck."""
    out, chain, open_ack = [], None, None
    for row in rows:
        if row.get("dir") != "ab" or "hex" not in row:
            continue
        b = bytes.fromhex(row["hex"])
        r = R(b)
        h = r.u8()
        mid = h & 0x1F
        if mid == 0x02 and h & 0x20:                     # OpenAck: lease, initial_sn
            r.vle()
            open_ack = r.vle()[0]
        elif mid == 0x05:                                # frame
            sn, sn_bytes = r.vle()
            ext_bytes, vals = exts(r, bool(h & 0x80))
            frame = {"sn": sn, "sn_bytes": sn_bytes, "qos_ext": ext_bytes,
                     "header": 1 + sn_bytes + ext_bytes, "priority": vals.get(1, 5)}
            while r.left():
                if r.b[r.i] & 0x1F != 0x1D:
                    break                                # a declaration or other message
                p = push(r)
                out.append({"t": row["t"], "sdu": len(b), "frame": frame, "put": p,
                            "fragments": None})
                frame = dict(frame, header=0, qos_ext=0)  # a second message shares the frame
        elif mid == 0x06:                                # fragment
            sn, sn_bytes = r.vle()
            ext_bytes, _ = exts(r, bool(h & 0x80))
            if chain is None:
                chain = {"sns": [], "sdus": [], "bytes": b"", "t": row["t"],
                         "header": 1 + sn_bytes + ext_bytes}
            chain["sns"].append(sn)
            chain["sdus"].append(len(b))
            chain["bytes"] += r.take(r.left())
            if not h & 0x40:                             # the last fragment
                p = push(R(chain["bytes"]))
                out.append({"t": chain["t"], "sdu": None, "frame": None, "put": p,
                            "fragments": {"sns": chain["sns"], "sdus": chain["sdus"]}})
                chain = None
    return out, open_ack


def compute_sn(a, b, res):
    def le(z):
        n = int(z, 16)
        return n.to_bytes(16, "little")[:(n.bit_length() + 7) // 8]
    h = hashlib.shake_128(le(a) + le(b)).digest(4)
    return int.from_bytes(h, "little") & MASK[res]


def analyse(d):
    marks = json.load(open(f"{d}/marks.json"))
    params = json.load(open(f"{d}/configs/params.json"))
    rows = [json.loads(x) for x in open(f"{d}/air.jsonl")]
    seen, open_ack = runs(rows)
    batch, res = params["batch_size"], params["sequence"]
    puts = []
    for m in marks["puts"]:
        got = [s for s in seen if s["put"]["first"] == m["index"] and s["put"]["payload"] == m["payload"]]
        assert len(got) == 1, f"{d}: put {m['index']} crossed {len(got)} times"
        s = got[0]
        p = {**m, "scope": s["put"]["scope"], "suffix": s["put"]["suffix"],
             "put_header": s["put"]["header"], "push_qos_ext": s["put"]["qos_ext"]}
        if s["fragments"] is None:
            f = s["frame"]
            p.update(sn=f["sn"], sn_bytes=f["sn_bytes"], frame_header=f["header"],
                     frame_qos_ext=f["qos_ext"], batch=s["sdu"] + 2, fits=True)
            assert s["sdu"] + 2 <= batch
            assert p["payload"] + p["put_header"] + p["frame_header"] + 2 == s["sdu"] + 2
        else:
            p.update(sn=s["fragments"]["sns"][0], fragments=len(s["fragments"]["sns"]),
                     fragment_sns=s["fragments"]["sns"], fragment_sdus=s["fragments"]["sdus"],
                     fits=False)
        puts.append(p)
    out = {"run": d.rstrip("/").split("/")[-1], "medium": params["medium"], "sequence": res,
           "batch_size": batch, "open_ack_initial_sn": open_ack, "puts": puts}
    if marks.get("zids"):
        a, b = marks["zids"].split(",")
        out["compute_sn"] = compute_sn(a, b, res)
        assert out["compute_sn"] == open_ack, (out["compute_sn"], open_ack)
    # Per QoS and sequence-number width: the bound.
    bounds = {}
    for p in puts:
        w = None
        if p["fits"]:
            w = p["sn_bytes"]
        else:
            w = (p["sn"].bit_length() + 6) // 7 or 1
        k = f"{p['qos']} sn {w} B"
        e = bounds.setdefault(k, {"largest_fit": None, "smallest_fragmented": None})
        if p["fits"]:
            e["largest_fit"] = max(e["largest_fit"] or 0, p["payload"])
        else:
            e["smallest_fragmented"] = min(e["smallest_fragmented"] or 1 << 30, p["payload"])
    for e in bounds.values():
        if e["largest_fit"] is not None:
            e["allowance"] = batch - e["largest_fit"]
        e["exact"] = (e["largest_fit"] is not None and e["smallest_fragmented"] == (e["largest_fit"] or 0) + 1)
    out["bounds"] = bounds
    out["max_sn"] = max(p["sn"] for p in puts)
    return out


def main():
    args = sys.argv[1:]
    js = None
    if "--json" in args:
        i = args.index("--json")
        js = args[i + 1]
        del args[i:i + 2]
    res = [analyse(d) for d in args]
    for r in res:
        print(f"{r['run']}: {r['medium']}, batch {r['batch_size']}, {r['sequence']}, "
              f"initial SN {r['open_ack_initial_sn']} (compute_sn {r.get('compute_sn')})")
        for p in r["puts"]:
            how = (f"one batch of {p['batch']} B: frame {p['frame_header']} B (SN {p['sn']}, "
                   f"{p['sn_bytes']} B; QoS {p['frame_qos_ext']} B), put {p['put_header']} B "
                   f"(QoS {p['push_qos_ext']} B)") if p["fits"] else \
                  f"{p['fragments']} fragments, SNs {p['fragment_sns']}, SDUs {p['fragment_sdus']}"
            print(f"  {p['index']:2} {p['qos']:24} {p['payload']:4} B: {how}")
        for k, e in sorted(r["bounds"].items()):
            print(f"  bound {k}: fits up to {e['largest_fit']}, fragments from "
                  f"{e['smallest_fragmented']}, allowance {e.get('allowance')}, exact {e['exact']}")
    if js:
        json.dump(res[0] if len(res) == 1 else res, open(js, "w"), indent=1)
        open(js, "a").write("\n")


if __name__ == "__main__":
    main()

#!/usr/bin/env python3
"""Attribute a bare run's air bytes to zk2 elements, by decoding each SDU (#989).

Usage: zdecode.py <run-dir> [--from T0 --to T1] [--json out.json] [--dump]

Reads air.jsonl (the relay's capture) of a run made with no layers, and
decodes each SDU as a Zenoh 1.10 batch, as the fork's codec writes it
(commons/zenoh-codec at zm-1.10.2-r1): transport messages (keep-alive,
init, open, close, frame, fragment), and in a frame the network messages
(push, request, response, response final, declare, interest, oam) with their
zenoh bodies. Extensions are skipped by their own encoding. A key's scope is
resolved through the D_KEYEXPR declarations seen each way: the receiver's
by default, the sender's under the M flag.

Each message's bytes go to one element, by its key (a response's by its
request's): telemetry, events, health, alarms, config, desired, heartbeat
(zk2c's freshness.v2), presence (every declaration under @zk), admin (@/),
declarations (every other declaration), interests, keep-alive, handshake. A frame's header and
extensions go to its first message, and an SDU's bytes are shared among its
messages in proportion; a fragmented message takes every SDU it spans.
Standard library only.
"""
import json
import sys
from collections import defaultdict


class Short(Exception):
    pass


class R:
    def __init__(self, b, i=0):
        self.b, self.i = b, i

    def u8(self):
        if self.i >= len(self.b):
            raise Short()
        v = self.b[self.i]
        self.i += 1
        return v

    def vle(self):
        v, s = 0, 0
        while True:
            c = self.u8()
            v |= (c & 0x7F) << s
            if c < 0x80:
                return v
            s += 7

    def take(self, n):
        if self.i + n > len(self.b):
            raise Short()
        v = self.b[self.i:self.i + n]
        self.i += n
        return v

    def slice(self):
        return self.take(self.vle())

    def left(self):
        return len(self.b) - self.i


def exts(r, more):
    """Skip extensions while the Z flag says one follows; return {id: value} (Z64 only)."""
    out = {}
    while more:
        h = r.u8()
        enc = (h >> 5) & 0b11
        if enc == 1:
            out[h & 0x0F] = r.vle()
        elif enc == 2:
            r.slice()
        more = bool(h & 0x80)
    return out


def wire_expr(r, named):
    scope = r.vle()
    suffix = r.slice().decode(errors="replace") if named else ""
    return scope, suffix


def zenoh_body(r):
    """A Put, Del, Query, Reply or Err: returns its kind and payload length."""
    h = r.u8()
    mid = h & 0x1F
    z = bool(h & 0x80)
    if mid == 0x01:  # PUT: T, E
        if h & 0x20:
            r.vle(); r.slice()
        if h & 0x40:
            e = r.vle()
            if e & 1:
                r.slice()
        exts(r, z)
        return "put", len(r.slice())
    if mid == 0x02:  # DEL: T
        if h & 0x20:
            r.vle(); r.slice()
        exts(r, z)
        return "del", 0
    if mid == 0x03:  # QUERY: C, P
        if h & 0x20:
            r.u8()
        if h & 0x40:
            r.slice()
        exts(r, z)
        return "query", 0
    if mid == 0x04:  # REPLY: C, then a Put or a Del
        if h & 0x20:
            r.u8()
        exts(r, z)
        kind, n = zenoh_body(r)
        return "reply-" + kind, n
    if mid == 0x05:  # ERR: E
        if h & 0x40:
            e = r.vle()
            if e & 1:
                r.slice()
        exts(r, z)
        return "err", len(r.slice())
    raise ValueError(f"zenoh body 0x{h:02x}")


def network(r):
    """One network message: (kind, scope, suffix, mapping_sender, extra)."""
    h = r.u8()
    mid = h & 0x1F
    z = bool(h & 0x80)
    if mid == 0x1D:  # PUSH: N, M
        scope, suffix = wire_expr(r, h & 0x20)
        exts(r, z)
        kind, _ = zenoh_body(r)
        return ("push-" + kind, scope, suffix, bool(h & 0x40), None)
    if mid == 0x1C:  # REQUEST
        rid = r.vle()
        scope, suffix = wire_expr(r, h & 0x20)
        exts(r, z)
        zenoh_body(r)
        return ("request", scope, suffix, bool(h & 0x40), rid)
    if mid == 0x1B:  # RESPONSE
        rid = r.vle()
        scope, suffix = wire_expr(r, h & 0x20)
        exts(r, z)
        kind, _ = zenoh_body(r)
        return ("response-" + kind, scope, suffix, bool(h & 0x40), rid)
    if mid == 0x1A:  # RESPONSE_FINAL
        rid = r.vle()
        exts(r, z)
        return ("response-final", None, "", False, rid)
    if mid == 0x1E:  # DECLARE: I
        if h & 0x20:
            r.vle()
        exts(r, z)
        d = r.u8()
        did = d & 0x1F
        dz = bool(d & 0x80)
        names = {0x00: "keyexpr", 0x01: "unkeyexpr", 0x02: "subscriber", 0x03: "unsubscriber",
                 0x04: "queryable", 0x05: "unqueryable", 0x06: "token", 0x07: "untoken",
                 0x1A: "final"}
        name = names.get(did, f"decl-0x{did:02x}")
        if did == 0x00:
            eid = r.vle()
            scope, suffix = wire_expr(r, d & 0x20)
            return ("declare-keyexpr", scope, suffix, bool(d & 0x40), eid)
        if did in (0x02, 0x04, 0x06):
            r.vle()
            scope, suffix = wire_expr(r, d & 0x20)
            exts(r, dz)
            return ("declare-" + name, scope, suffix, bool(d & 0x40), None)
        if did == 0x1A:
            exts(r, dz)
            return ("declare-final", None, "", False, None)
        r.vle()
        exts(r, dz)
        return ("declare-" + name, None, "", False, None)
    if mid == 0x19:  # INTEREST: mode bits 5-6; RESTRICTED, NAMED and MAPPING in its options
        r.vle()
        scope, suffix, opts = None, "", 0
        if (h >> 5) & 0b11:
            opts = r.u8()
            if opts & 0x10:
                scope, suffix = wire_expr(r, opts & 0x20)
        exts(r, z)
        return ("interest", scope, suffix, bool(opts & 0x40), None)
    if mid == 0x1F:  # OAM
        r.vle()
        exts(r, z)
        enc = (h >> 5) & 0b11
        if enc == 1:
            r.vle()
        elif enc == 2:
            r.slice()
        return ("oam", None, "", False, None)
    raise ValueError(f"network 0x{h:02x}")


def element(key, kind):
    if kind == "keep-alive":
        return "keep-alive"
    if kind in ("init", "open", "close"):
        return "handshake"
    if kind == "interest":
        return "interests"
    if kind == "oam":
        return "oam"
    k = key or ""
    if k.startswith("@/") or "/@/" in k:
        return "admin"
    if kind.startswith("declare"):
        return "presence" if "@zk/" in k else "declarations"
    # zk2c's (#989): freshness.v2's heartbeats, config.v2's rfcfg.v1, modem.v3's events.
    for needle, name in (("/freshness.v2/", "heartbeat"),
                         ("/gendev.v1/stream/", "telemetry"), ("/modem.v3/stream/", "telemetry"),
                         ("/gendev.v1/events/", "events"), ("/modem.v3/events/", "events"),
                         ("/health.v1/", "health"),
                         ("/alarms.v1/", "alarms"), ("/config.v1/", "config"),
                         ("/rfcfg.v1/", "config"),
                         ("/worker.v1/", "desired"), ("/tasking.v1/", "desired"),
                         ("@zk/", "presence"), ("/@adv/", "advanced")):
        if needle in k:
            return name
    return "other"


class Direction:
    def __init__(self):
        self.exprs = {}           # this direction's D_KEYEXPR ids
        self.frag = {}            # reliability -> [bytes, [sdu indexes]]


def resolve(sender, receiver, scope, suffix, msender):
    if scope in (None, 0):
        return suffix
    table = sender.exprs if msender else receiver.exprs
    return table.get(scope, f"<{scope}>") + suffix


def decode(rows, t0=None, t1=None, dump=False, listing=None):
    """Each element's bytes, messages and share of SDUs per direction in [t0, t1).

    With `listing` (a list), every message in the window is appended to it:
    {t, dir, element, kind, key, bytes, sdus}, its bytes and SDUs its share of
    the SDUs that carried it.
    """
    dirs = {"ab": Direction(), "ba": Direction()}
    other = {"ab": "ba", "ba": "ab"}
    requests = {}                 # (direction of request, rid) -> element
    acc = defaultdict(lambda: defaultdict(lambda: [0.0, 0]))   # dir -> element -> [bytes, msgs]
    sdus = defaultdict(lambda: defaultdict(float))              # dir -> element -> share of SDUs
    errors = defaultdict(int)
    opens = {"ab": [], "ba": []}

    def credit(d, parts, total, inwin, share, t, count=True):
        if not inwin or not parts:
            return
        weight = sum(p[1] for p in parts) or 1
        for el, n, kind, key in parts:
            acc[d][el][0] += total * n / weight
            acc[d][el][1] += 1 if count else 0
            sdus[d][el] += share * n / weight
            if listing is not None and count:
                listing.append({"t": t, "dir": d, "element": el, "kind": kind, "key": key,
                                "bytes": total * n / weight, "sdus": share * n / weight})

    def messages(d, r):
        """Decode network messages up to the next transport message; return [(element, bytes)]."""
        out = []
        while r.left() and 0x19 <= (r.b[r.i] & 0x1F) <= 0x1F:
            start = r.i
            kind, scope, suffix, msender, extra = network(r)
            key = resolve(dirs[d], dirs[other[d]], scope, suffix, msender)
            if kind == "declare-keyexpr":
                dirs[d].exprs[extra] = key
            if kind == "request":
                el = element(key, "push")
                requests[(d, extra)] = el
            elif kind.startswith("response"):
                el = requests.get((other[d], extra)) or element(key, "push")
            else:
                el = element(key, kind)
            out.append((el, r.i - start, kind, key))
            if dump:
                print(f"    {kind:22s} {key[:100]!s:100s} -> {el} ({r.i - start} B)")
        return out

    for row in rows:
        if "dir" not in row:
            continue
        d = row["dir"]
        b = bytes.fromhex(row["hex"])
        inwin = (t0 is None or row["t"] >= t0) and (t1 is None or row["t"] < t1)
        if dump and inwin:
            print(f"{row['t']:.3f} {d} {len(b)} B{' DROP' if row['drop'] else ''}")
        try:
            r = R(b)
            parts = []
            while r.left():
                start = r.i
                h = r.u8()
                tid = h & 0x1F
                z = bool(h & 0x80)
                if tid == 0x04:
                    exts(r, z)
                    parts.append(("keep-alive", r.i - start, "keep-alive", None))
                elif tid in (0x01, 0x02, 0x03):
                    if tid == 0x01 and not h & 0x20 and inwin:
                        opens[d].append(row["t"])          # an InitSyn: a transport opening
                    r.i = len(b)
                    parts.append(("handshake", r.i - start, "handshake", None))
                elif tid == 0x05:
                    r.vle()
                    exts(r, z)
                    head = r.i - start
                    got = messages(d, r)
                    if got:
                        el, n, kind, key = got[0]
                        got[0] = (el, n + head, kind, key)
                    parts.extend(got)
                elif tid == 0x06:
                    reliable = bool(h & 0x20)
                    more = bool(h & 0x40)
                    r.vle()
                    exts(r, z)
                    st = dirs[d].frag.setdefault(reliable, [b"", []])
                    st[0] += b[r.i:]
                    st[1].append((len(b), inwin, row["t"]))
                    r.i = len(b)
                    if not more:
                        whole, spans = st
                        dirs[d].frag[reliable] = [b"", []]
                        got = messages(d, R(whole))
                        for i, (n, w, _) in enumerate(spans):
                            credit(d, got, n, w, 1.0, spans[0][2], count=False)
                        if any(w for _, w, _ in spans):
                            n = sum(n for n, w, _ in spans if w)
                            k = sum(1 for _, w, _ in spans if w)
                            credit(d, got, 0, True, 0.0, spans[0][2], count=True)
                            if listing is not None:
                                weight = sum(p[1] for p in got) or 1
                                for item, p in zip(listing[-len(got):], got):
                                    item["bytes"] = n * p[1] / weight
                                    item["sdus"] = k * p[1] / weight
                    parts = None
                    break
                else:
                    raise ValueError(f"transport 0x{h:02x}")
            if parts is not None:
                credit(d, parts, len(b), inwin, 1.0, row["t"])
        except (Short, ValueError, UnicodeError) as e:
            errors[type(e).__name__] += 1
            if inwin:
                acc[d]["undecoded"][0] += len(b)
                acc[d]["undecoded"][1] += 1
                sdus[d]["undecoded"] += 1
    out = {}
    for d in ("ab", "ba"):
        out[d] = {el: {"bytes": round(v[0], 1), "messages": v[1], "sdus": round(sdus[d][el], 2)}
                  for el, v in sorted(acc[d].items(), key=lambda kv: -kv[1][0])}
    out["errors"] = dict(errors)
    out["opens"] = opens
    return out


def main():
    path = sys.argv[1]
    rows = [json.loads(line) for line in open(f"{path}/air.jsonl") if line.strip()]
    arg = lambda k: float(sys.argv[sys.argv.index(k) + 1]) if k in sys.argv else None
    t0, t1 = arg("--from"), arg("--to")
    out = decode(rows, t0, t1, dump="--dump" in sys.argv)
    text = json.dumps(out, indent=1)
    if "--json" in sys.argv:
        open(sys.argv[sys.argv.index("--json") + 1], "w").write(text + "\n")
    if "--dump" not in sys.argv:
        print(text)


if __name__ == "__main__":
    main()

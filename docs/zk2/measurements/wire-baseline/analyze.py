#!/usr/bin/env python3
"""Per-direction figures for one #989 run: the drivers' counters and the relay's capture.

Usage: analyze.py <run-dir> [--json out.json]

Reads, from the run directory measure.sh wrote:
- marks.json: each window's bounds (wall clock, seconds);
- counters-a.jsonl, counters-b.jsonl: each driver's management readings, stamped;
- air.jsonl: every datagram the relay carried, per direction;
- far.jsonl: what the far side's subscribers received.

A window's counter delta is the reading last published at or before its end,
less the one last published at or before its start (published every second).
The relay's figures count exactly the datagrams inside the window.

A->B is driver A's transmit side (the owner's router toward the reader's), and
B->A driver B's. Airtime is rf()'s: 8 bits per SDU byte at 2,400 bit/s, as
fake-medium's SimMedium charges a frame with no ARQ (sim.rs `airtime`).
"""
import json
import sys
from collections import Counter

DAY = 86_400
RF_BPS = 2400
DIRS = (("ab", "a", "b"), ("ba", "b", "a"))


def load_jsonl(path):
    try:
        with open(path) as f:
            return [json.loads(line) for line in f if line.strip()]
    except FileNotFoundError:
        return []


def counters(rows):
    """{suffix: [(t, value), ...]} for numeric readings."""
    out = {}
    for r in rows:
        k, v = r.get("key"), r.get("value")
        if not k or not isinstance(v, (int, float)) or isinstance(v, bool):
            continue
        suffix = k.split("/device/", 1)[1].split("/", 1)[1]
        out.setdefault(suffix, []).append((r["t"], v))
    return out


def at(series, t):
    best = None
    for ts, v in series:
        if ts <= t:
            best = v
        else:
            break
    return best


def delta(series, t0, t1):
    a, b = at(series, t0), at(series, t1)
    return None if a is None or b is None else b - a


def classify(stack, n, hexs):
    if stack == "bare":
        if n == 1 and hexs == "04":
            return "keep-alive"
        return "other"
    # Lean: compress stores a 1-byte keep-alive (0x00 + 0x04), then aead's
    # 1 + 4 + 8 and the 2-byte overhead: 17 bytes on the air.
    return "keep-alive?" if n == 17 else "other"


def window(run, name, t0, t1):
    span = t1 - t0
    scale = DAY / span
    res = {"window": name, "seconds": round(span, 3)}
    stack = run["marks"]["stack"]
    for d, tx, rx in DIRS:
        ctx, crx = run["counters"][tx], run["counters"][rx]
        sdus = delta(ctx.get("sdu/tx_sdus_total", []), t0, t1)
        byts = delta(ctx.get("sdu/tx_bytes_total", []), t0, t1)
        rsdus = delta(crx.get("sdu/rx_sdus_total", []), t0, t1)
        rbyts = delta(crx.get("sdu/rx_bytes_total", []), t0, t1)
        air = [r for r in run["air"] if r["dir"] == d and t0 <= r["t"] < t1]
        kinds = Counter()
        kbytes = Counter()
        sizes = Counter()
        for r in air:
            k = classify(stack, r["n"], r["hex"])
            kinds[k] += 1
            kbytes[k] += r["n"]
            if k == "other":
                sizes[r["n"]] += 1
        layer = {}
        for key in ("layer/compress/compressed_total", "layer/compress/stored_total",
                    "layer/compress/bytes_saved_total", "layer/aead/auth_failures_total",
                    "layer/aead/replays_total"):
            v = delta(ctx.get(key, []), t0, t1)
            if v is not None:
                layer[key.split("/", 1)[1]] = v
        rel = {
            "tx_sdus": sdus, "tx_bytes": byts, "rx_sdus_far": rsdus, "rx_bytes_far": rbyts,
            "relay_sdus": len(air), "relay_bytes": sum(r["n"] for r in air),
            "relay_dropped": sum(1 for r in air if r["drop"]),
            "kinds": dict(kinds), "kind_bytes": dict(kbytes),
            "other_sizes": dict(sorted(sizes.items())),
            "tx_layers": layer,
        }
        if sdus is not None and byts is not None:
            rel["airtime_s"] = round(byts * 8 / RF_BPS, 3)
            rel["per_day"] = {
                "sdus": round(sdus * scale), "bytes": round(byts * scale),
                "airtime_s": round(byts * 8 / RF_BPS * scale, 1),
                "airtime_pct": round(100 * byts * 8 / RF_BPS * scale / DAY, 3),
                "keepalives": round(kinds.get("keep-alive", kinds.get("keep-alive?", 0)) * scale),
            }
        res[d] = rel
    far = Counter(r["key"].rsplit("/stream/", 1)[1] for r in run["far"]
                  if r.get("key") and t0 <= r["t"] < t1)
    if far:
        res["far_samples"] = dict(far)
    return res


def main():
    path = sys.argv[1]
    run = {
        "marks": json.load(open(f"{path}/marks.json")),
        "counters": {x: counters(load_jsonl(f"{path}/counters-{x}.jsonl")) for x in "ab"},
        "air": load_jsonl(f"{path}/air.jsonl"),
        "far": load_jsonl(f"{path}/far.jsonl"),
    }
    m = run["marks"]
    out = {"run": path, "stack": m["stack"], "loss": m.get("loss"), "windows": []}
    for name, a, b in (("idle", "idle_start", "idle_end"), ("w1", "w1_start", "w1_end")):
        if a in m and b in m:
            out["windows"].append(window(run, name, m[a], m[b]))
    text = json.dumps(out, indent=1)
    if "--json" in sys.argv:
        with open(sys.argv[sys.argv.index("--json") + 1], "w") as f:
            f.write(text + "\n")
    print(text)


if __name__ == "__main__":
    main()

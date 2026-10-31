#!/usr/bin/env python3
"""What a declaration lost in the setup costs a bare run, for its whole transport (#989).

Usage: setup-loss.py <run-dir>... [--json out.json]

For each bare run (air.jsonl, far.jsonl, zenohd-{a,b}.log):
- the SDUs the relay dropped, and among them those carrying declarations,
  with the keys they declared (zdecode resolves every key, lost or not);
- per device resource: the puts A sent toward B, those the relay delivered,
  and the samples B's `zk2/**` subscriber received. A message fragmented over
  several SDUs counts as delivered when its last fragment was, so a config
  view that lost an earlier fragment is delivered here and never received;
- the desired target's GETs that crossed A->B (requests in delivered SDUs);
- router B's "Route data with unknown scope" lines (a put whose key scope it
  never learned, dropped) and "Unknown wire expr" lines.
Standard library only.
"""
import collections
import json
import os
import re
import sys

sys.dont_write_bytecode = True
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import zdecode  # noqa: E402

DEVICE = ("telemetry", "health", "alarms", "config", "desired", "events")


def resource(key):
    return "/".join((key or "?").split("/")[3:6])


def one(path):
    rows = [json.loads(line) for line in open(f"{path}/air.jsonl") if line.strip()]
    msgs = []
    zdecode.decode(rows, listing=msgs)
    t0 = rows[0]["t"]
    dropped = {(r["t"], r["dir"]) for r in rows if r.get("drop")}
    decl = collections.defaultdict(list)
    for m in msgs:
        if (m["t"], m["dir"]) in dropped and m["kind"].startswith("declare"):
            decl[(m["t"], m["dir"])].append(f'{m["kind"][8:]} {m["key"] or "?"}')
    res = collections.defaultdict(lambda: {"sent": 0, "delivered": 0, "received": 0})
    gets = 0
    for m in msgs:
        mine = m["dir"] == "ab" and "/tasking.v1/" not in (m["key"] or "")
        if mine and m["kind"].startswith("push-") and m["element"] in DEVICE:
            r = res[resource(m["key"])]
            r["sent"] += 1
            r["delivered"] += (m["t"], m["dir"]) not in dropped
        if m["dir"] == "ab" and m["kind"] == "request" and m["element"] == "desired":
            gets += (m["t"], m["dir"]) not in dropped
    for line in open(f"{path}/far.jsonl"):
        if line.strip():
            k = json.loads(line).get("key")
            if k and "/tasking.v1/" not in k:
                res[resource(k)]["received"] += 1
    logs = collections.Counter()
    for f in ("zenohd-a", "zenohd-b"):
        for line in open(f"{path}/{f}.log", errors="replace"):
            line = re.sub(r"\x1b\[[0-9;]*m", "", line)
            if "unknown scope" in line:
                logs[f"{f}: Route data with unknown scope"] += 1
            elif "Unknown wire expr" in line:
                logs[f"{f}: Unknown wire expr"] += 1
    return {
        "run": os.path.basename(os.path.normpath(path)),
        "sdus_lost": len(dropped),
        "lost_with_declarations": [{"t": round(t - t0, 1), "dir": d, "declared": v}
                                   for (t, d), v in sorted(decl.items())],
        "resources": dict(sorted(res.items())),
        "never_received": sorted(k for k, v in res.items() if v["delivered"] and not v["received"]),
        "desired_gets_crossed": gets,
        "router_lines": dict(logs),
    }


def main():
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    out = [one(p) for p in args if p != (sys.argv[sys.argv.index("--json") + 1] if "--json" in sys.argv else None)]
    text = json.dumps(out, indent=1)
    if "--json" in sys.argv:
        with open(sys.argv[sys.argv.index("--json") + 1], "w") as f:
            f.write(text + "\n")
    for r in out:
        print(f"== {r['run']}: {r['sdus_lost']} SDUs lost, {len(r['lost_with_declarations'])} with "
              f"declarations; never received: {r['never_received']}; desired GETs crossed: "
              f"{r['desired_gets_crossed']}; {r['router_lines']}")
        for x in r["lost_with_declarations"]:
            print(f"   +{x['t']} s {x['dir']}: " + "; ".join(x["declared"]))


if __name__ == "__main__":
    main()

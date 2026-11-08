#!/usr/bin/env python3
"""What the tools across the face said in one honesty.sh run (#989's zk2c half).

Usage: honesty.py <run-dir> [<run-dir>...] [--json out.json]

For each run directory honesty.sh (or measure-w2.sh's --zk2c drive) wrote:
- frames: every frame the far frame reader (`zenctl echo --across-face`) and
  `zenctl watch` read, seconds from the run's first gen: its run, seq, what
  it `follows` (first, continues, gap with the records `lost`, run), its
  payload bytes and how many of its fields read known, absent or unknown;
- telemetry: each `zenctl telemetry` window's judgement and, for one counter
  series, every pair it formed, with its continuity and rate or why none;
- probes: each tool run inside the cut (telemetry, doctor, health), its exit
  code and its judgement;
- relay: when the loss, the cut and the restore were set.
Standard library only.
"""
import json
import os
import sys
from collections import Counter

COUNTER = "sdu_tx_sdus_total"


def jsonl(path):
    try:
        return [json.loads(line) for line in open(path) if line.strip().startswith("{")]
    except FileNotFoundError:
        return []


def document(path):
    """The JSON document of a tool's log: its stderr lines, then the document."""
    try:
        text = open(path).read()
    except FileNotFoundError:
        return None
    i = text.find("\n{") + 1 if not text.startswith("{") else 0
    if i == 0 and not text.startswith("{"):
        return None
    return json.JSONDecoder().raw_decode(text[i:])[0]


def frame(v):
    states = Counter(f.get("state") for rec in v["records"] for f in rec["fields"].values())
    return {"run": v.get("run"), "seq": v.get("seq"), "follows": v.get("follows"),
            "lost": v.get("lost"), "states": dict(states)}


def telemetry(d):
    if not d:
        return None
    row = next((r for r in d.get("rows", []) if r.get("field") == COUNTER), None)
    out = {"judgement": d.get("judgement"), "totals": d.get("totals")}
    if row:
        out[COUNTER] = {"continuity": row["continuity"].get("reason"),
                        "pairs": [{k: p.get(k) for k in ("earlier", "later", "elapsed_s", "continuity",
                                                          "rate", "no_rate")} for p in row["pairs"]]}
    return out


def read(path):
    marks = json.load(open(f"{path}/marks.json"))
    t0 = marks.get("gen1", marks.get("steady_start", 0))
    out = {"run": os.path.basename(os.path.normpath(path)),
           "marks_s": {k: round(v - t0, 1) for k, v in marks.items() if isinstance(v, float)}}
    frames = []
    for r in jsonl(f"{path}/far.jsonl"):
        v = r.get("value")
        if isinstance(v, dict) and "records" in v:
            frames.append({"t": round(r["t"] - t0, 1), "bytes": r.get("n"), **frame(v)})
    if frames:
        out["far_frames"] = frames
    watch = []
    for r in jsonl(f"{path}/watch.log"):
        v = (r.get("payload") or {}).get("value")
        if isinstance(v, dict) and "records" in v:
            watch.append({**(r.get("inband") or {}), "follows": v.get("follows"), "lost": v.get("lost")})
    if watch:
        out["watch_frames"] = watch
    out["telemetry"] = {name: telemetry(document(f"{path}/{name}.log"))
                        for name in ("telemetry", "telemetry2") if os.path.exists(f"{path}/{name}.log")}
    exits = {f"{a['probe']}_exit": a["exit"] for a in jsonl(f"{path}/acts.jsonl") if "probe" in a}
    marks = {**marks, **exits}
    probes = {}
    for name in ("stale-watch", "stale-telemetry", "stale-doctor", "probe-health", "probe-doctor"):
        for ext in (".out", ".json"):
            p = f"{path}/{name}{ext}"
            if not os.path.exists(p):
                continue
            try:
                d = json.load(open(p))
            except ValueError:
                d = None
            entry = {"exit": marks.get(f"{name}_exit")}
            if name == "stale-telemetry":
                entry.update(telemetry(d) or {})
            elif d and "rows" in d and d.get("report") == "health":
                entry.update(judgement=d.get("judgement"), rollup=d.get("rollup"))
            elif d and "rows" in d:
                entry["checks"] = {r["check"]: r["verdict"] for r in d["rows"]
                                   if r.get("verdict", {}).get("answer") != "not_asked"}
                entry["notes"] = [n["text"] for n in d.get("notes", [])][:2]
            elif not d:
                err = open(f"{path}/{name}.err").read() if os.path.exists(f"{path}/{name}.err") else ""
                entry["stderr"] = err.strip().splitlines()[:1]
            probes[name] = entry
    if probes:
        out["probes"] = probes
    out["relay"] = [{"t": round(e["t"] - t0, 1), "event": e["event"]}
                    for e in jsonl(f"{path}/air.jsonl") if "event" in e]
    return out


def main():
    args = sys.argv[1:]
    dest = None
    if "--json" in args:
        dest = args[args.index("--json") + 1]
        args = args[:args.index("--json")] + args[args.index("--json") + 2:]
    text = json.dumps([read(p) for p in args], indent=1)
    if dest:
        open(dest, "w").write(text + "\n")
    print(text)


if __name__ == "__main__":
    main()

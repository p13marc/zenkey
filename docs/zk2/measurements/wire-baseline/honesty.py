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

For a W2 run made with OWNER_HEARTBEAT=1 WATCH=1 (#1110's H4), also:
- owner_heartbeats: every owner heartbeat the device put, decoded from the
  relay's capture (a bare run): its seq, when, and whether the relay dropped it;
- watch_status: what `zenctl watch` on health.v1's status printed with the
  catalog and with none: each sample, each `confirmation` row (the heartbeat's
  seq, its group, the members it confirmed, `lost`, `diverged`), and the GETs
  the watch's session made that timed out (its stderr);
- descriptor_reads: every GET on `@zk/instance/*` that crossed, and its reply's
  bytes and time on the air;
- the probes probe-health-crossing (before the cut) and probe-health-across.
Standard library only.
"""
import json
import os
import re
import sys
from collections import Counter

sys.dont_write_bytecode = True
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import zdecode  # noqa: E402

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


def owner_form(path, t0):
    """#1110's H4: the owner heartbeats put, what the two watches printed, the descriptor reads."""
    out = {}
    air = jsonl(f"{path}/air.jsonl")
    params = f"{path}/configs/params.json"
    bare = os.path.exists(params) and json.load(open(params)).get("stack") != "lean"
    if bare and os.path.exists(f"{path}/watch-catalog.jsonl"):
        pushes, listing = [], []
        zdecode.decode(air, pushes=pushes)
        raw = {r["t"]: bytes.fromhex(r["hex"]) for r in air if "hex" in r}
        cut_at = {r["t"] for r in air if r.get("cut")}
        rows = []
        for x in pushes:
            if x["dir"] != "ab" or "/freshness.v2/stream/heartbeats/" not in (x["key"] or ""):
                continue
            # Alone in its SDU (the 3-byte frame, then the put), its payload ends the SDU:
            # the envelope's CH, its run of 16 or 32 bits, then a 24-bit seq (core §7.5).
            b = raw[x["t"]]
            env = b[len(b) - x["payload"]:] if x["sdu"] == x["message"] + 3 else None
            seq = None
            if env:
                wide = bool(env[0] & 0x10)
                seq = int.from_bytes(env[5:8] if wide else env[3:6], "big")
            rows.append({"t": round(x["t"] - t0, 1), "seq": seq, "payload": x["payload"],
                         "sdu": x["sdu"], "dropped": bool(x["drop"]), "cut": x["t"] in cut_at})
        out["owner_heartbeats"] = {
            "put": len(rows), "dropped_by_loss": sum(r["dropped"] and not r["cut"] for r in rows),
            "dropped_by_loss_seqs": [r["seq"] for r in rows if r["dropped"] and not r["cut"]],
            "dropped_in_the_cut_seqs": [r["seq"] for r in rows if r["cut"]],
            "rows": rows}
        zdecode.decode(air, listing=listing)
        reads, last = [], None
        for x in listing:
            if x["element"] != "presence" or not x["kind"].startswith(("request", "response")):
                continue
            if x["kind"] == "request":
                last = {"t": round(x["t"] - t0, 1), "key": x["key"], "reply_bytes": 0, "reply_sdus": 0.0,
                        "reply_done_s": None}
                reads.append(last)
            elif last is not None:
                last["reply_bytes"] += round(x["bytes"])
                last["reply_sdus"] = round(last["reply_sdus"] + x["sdus"], 2)
                last["reply_done_s"] = round(x["t"] - t0 - last["t"], 1)
        out["descriptor_reads"] = reads
    watches = {}
    for name in ("watch-catalog", "watch-none"):
        if not os.path.exists(f"{path}/{name}.jsonl"):
            continue
        rows = jsonl(f"{path}/{name}.jsonl")
        conf = [{"t": round(r["t"] - t0, 1), "seq": r["heartbeat"]["seq"], "group": r.get("group"),
                 "key": r["key"].split("/freshness.v2/")[-1], "confirmed": len(r["confirmed"]),
                 "lost": r["lost"], "diverged": r["diverged"]}
                for r in rows if r.get("row") == "confirmation"]
        samples = [{"t": round(r["t"] - t0, 1), "kind": r.get("kind"), "seq": (r.get("inband") or {}).get("seq"),
                    "value": (r.get("payload") or {}).get("value")}
                   for r in rows if r.get("row") == "sample"]
        err = open(f"{path}/{name}.err").read() if os.path.exists(f"{path}/{name}.err") else ""
        watches[name] = {
            "samples": samples,
            "confirmations": len(conf), "lost": sum(c["lost"] for c in conf),
            "diverged": sum(c["diverged"] for c in conf),
            "first_confirmation_s": conf[0]["t"] if conf else None,
            "last_confirmation_s": conf[-1]["t"] if conf else None,
            "get_timeouts": re.findall(r"Didn't receive final reply for query [^:]+:\d+:\d+: (Timeout\(\d+s\))", err),
            "rows": conf}
    if watches:
        out["watch_status"] = watches
    return out


def read(path):
    marks = json.load(open(f"{path}/marks.json"))
    t0 = marks.get("gen1", marks.get("steady_start", 0))
    out = {"run": os.path.basename(os.path.normpath(path)),
           # A run's marks before honesty.sh said so read as complete.
           "complete": marks.get("complete", True),
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
    for name in ("stale-watch", "stale-telemetry", "stale-doctor", "probe-health-crossing",
                 "probe-health", "probe-doctor", "probe-health-across"):
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
    out.update(owner_form(path, t0))
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

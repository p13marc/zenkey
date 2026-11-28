#!/usr/bin/env python3
"""Read a soak.py run (rc-023, #975 and #989): each reopen, what crossed after it, each
probe's verdict, and each process's RSS over the run.

Usage: soak-analyze.py <logs-dir> [--json out.json]

- **Reopens.** For each, its kind and when the stopped process was started again; then, to
  the next reopen, what the relay passed each way, the first frames/fast frame the far
  reader got after it, and every far key's rows: a link that came back carries frames
  again, and one that did not carries none.
- **Probes.** Each round's tools, their exit, their time and the judgement or verdict
  their JSON states, so a silence after a reopen is told from an answer.
- **Memory.** Each process by the name wire.sh or soak.py gave it, its samples once a minute,
  each its RssAnon and its VmSwap together, the memory it allocated, whether resident or
  swapped out (a host short of memory shrinks RSS alone, which is no process's doing):
  the first, at 10 minutes, the last and the largest, the least-squares slope over each half
  of its life, KB an hour, and the median of each third. A process grows without levelling
  off when its last third's median is over its middle third's by more than 2 % and 512 KB,
  and over the largest sample of its first third: a process that levels off holds its middle
  third's size, give or take its own noise.
Standard library only.
"""
import argparse
import json
import os
from collections import Counter, defaultdict


def jsonl(path):
    if not os.path.exists(path):
        return []
    out = []
    for line in open(path):
        try:
            out.append(json.loads(line))
        except ValueError:
            pass
    return out


def slope(points):
    """Least squares, KB an hour, over (t seconds, kb)."""
    if len(points) < 3:
        return None
    n = len(points)
    mt = sum(t for t, _ in points) / n
    mv = sum(v for _, v in points) / n
    var = sum((t - mt) ** 2 for t, _ in points)
    if var == 0:
        return None
    return round(sum((t - mt) * (v - mv) for t, v in points) / var * 3600, 1)


def verdict(doc):
    """The judgement a tool's JSON states, in its own words, or None."""
    if not isinstance(doc, dict):
        return None
    for k in ("judgement", "verdict", "answer", "result", "outcome"):
        v = doc.get(k)
        if isinstance(v, dict):
            words = {x: v[x] for x in ("answer", "reason", "verdict", "level") if x in v}
            if words:
                return words
        elif isinstance(v, str):
            return {k: v}
    return None


def summarize_probe(logs, rnd, name):
    stem = f"{logs}/probe-{rnd:02d}-{name}"
    text = open(f"{stem}.json").read() if os.path.exists(f"{stem}.json") else ""
    try:
        doc = json.loads(text) if text.strip() else None
    except ValueError:
        doc = None
        for line in text.splitlines():           # ndjson: the envelope first
            try:
                doc = json.loads(line)
                break
            except ValueError:
                continue
    out = {"verdict": verdict(doc)}
    if isinstance(doc, dict):
        for k in ("state", "status", "health", "phase", "converged", "applied", "outcome"):
            if k in doc and not isinstance(doc[k], (dict, list)):
                out[k] = doc[k]
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("logs")
    ap.add_argument("--json")
    ap.add_argument("--rss", help="the samples, if not LOGS/rss.jsonl (rc-023's sampler with swap)")
    a = ap.parse_args()
    logs = a.logs
    marks = json.load(open(f"{logs}/marks.json"))
    acts = jsonl(f"{logs}/acts.jsonl")
    air = [r for r in jsonl(f"{logs}/air.jsonl") if "dir" in r]
    far = [r for r in jsonl(f"{logs}/far.jsonl") if r.get("key")]
    end = marks.get("end") or max([r["t"] for r in air] + [0])
    # Samples to the end mark: the teardown after it is no process's life.
    rss = [r for r in jsonl(a.rss or f"{logs}/rss.jsonl") if r["t"] <= end]
    soak_start = marks["soak_start"]
    res = {"run": os.path.basename(logs.rstrip("/")),
           "hours": round((end - soak_start) / 3600, 2)}

    reopens = [x for x in acts if str(x.get("mark", "")).startswith("reopen_")]
    restarted = {x["mark"].split("_")[1]: x for x in acts
                 if str(x.get("mark", "")).startswith("restarted_")}
    rows = []
    for k, x in enumerate(reopens):
        i = x["mark"].split("_")[1]
        t0 = restarted.get(i, x)["t"]
        t1 = reopens[k + 1]["t"] if k + 1 < len(reopens) else end
        seg = [r for r in air if t0 <= r["t"] < t1]
        per = {}
        for d in ("ab", "ba"):
            s = [r for r in seg if r["dir"] == d]
            passed = [r for r in s if not r["drop"] and not r.get("cut")]
            per[d] = {"sdus": len(s), "passed": len(passed), "bytes": sum(r["n"] for r in passed),
                      "first_s": round(passed[0]["t"] - t0, 1) if passed else None,
                      "last_s": round(passed[-1]["t"] - t0, 1) if passed else None}
        fr = [r for r in far if t0 <= r["t"] < t1]
        frames = [r for r in fr if r["key"].endswith("/frames/fast")]
        keys = Counter("/".join(r["key"].split("/")[3:]) for r in fr)
        rows.append({"reopen": int(i), "kind": x.get("kind"), "at": round(x["t"] - soak_start, 1),
                     "restart_s": round(t0 - x["t"], 2), "span_s": round(t1 - t0, 1),
                     "air": per, "frames": len(frames),
                     "first_frame_s": round(frames[0]["t"] - t0, 1) if frames else None,
                     "far_keys": dict(sorted(keys.items())),
                     "link_back": bool(frames)})
    res["reopens"] = rows

    probes = defaultdict(dict)
    for x in acts:
        if "probe" in x:
            p = {"exit": x["exit"], "seconds": round(x["done"] - x["t"], 1),
                 **summarize_probe(logs, x["round"], x["probe"])}
            if x["exit"] not in (0, 1):
                p["stderr"] = x.get("stderr", "")[-200:]
            probes[x["round"]][x["probe"]] = p
    res["probes"] = {str(k): v for k, v in sorted(probes.items())}
    tally = defaultdict(Counter)
    for rnd in probes.values():
        for name, p in rnd.items():
            tally[name][str(p["exit"])] += 1
    res["probe_exits"] = {k: dict(v) for k, v in sorted(tally.items())}

    series = defaultdict(list)
    for r in rss:
        # What a process allocated: its anonymous pages, resident or swapped out (its RSS
        # alone where a sample has neither).
        held = r["anon_kb"] + r["swap_kb"] if "anon_kb" in r else r["rss_kb"]
        series[(r["name"], r["comm"], r["pid"])].append((r["t"], held))
    procs = []
    for (name, comm, pid), pts in sorted(series.items(), key=lambda kv: (kv[0][0], kv[0][2])):
        pts.sort()
        t_first = pts[0][0]
        life = pts[-1][0] - t_first
        after10 = [v for t, v in pts if t - t_first >= 600]
        half = t_first + life / 2
        first_half = [(t - t_first, v) for t, v in pts if t <= half]
        second_half = [(t - t_first, v) for t, v in pts if t >= half]
        s1, s2 = slope(first_half), slope(second_half)
        last = pts[-1][1]
        thirds = [sorted(v for t, v in pts if t_first + life * k / 3 <= t <= t_first + life * (k + 1) / 3)
                  for k in range(3)]
        med = [x[len(x) // 2] if x else None for x in thirds]
        grows = (life >= 1800 and None not in med
                 and med[2] - med[1] > max(0.02 * med[1], 512) and med[2] > max(thirds[0]))
        procs.append({"name": name, "comm": comm, "pid": pid, "minutes": round(life / 60, 1),
                      "samples": len(pts), "first_kb": pts[0][1],
                      "at_10min_kb": after10[0] if after10 else None, "last_kb": last,
                      "max_kb": max(v for _, v in pts), "slope_first_half_kb_h": s1,
                      "slope_second_half_kb_h": s2, "median_by_third_kb": med,
                      "grows_without_levelling": grows})
    res["rss"] = procs

    if a.json:
        with open(a.json, "w") as f:
            json.dump(res, f, indent=1)
            f.write("\n")
    print(f"{res['run']}: {res['hours']} h after the setup, {len(rows)} reopens")
    for r in rows:
        print(f"  reopen {r['reopen']:2d} {r['kind']:9s} +{r['at']:7.1f}s: frames {r['frames']:2d} "
              f"(first +{r['first_frame_s']}s), A->B {r['air']['ab']['passed']} SDUs "
              f"(last +{r['air']['ab']['last_s']}s), B->A {r['air']['ba']['passed']}")
    for rnd, ps in res["probes"].items():
        print(f"  round {rnd}: " + ", ".join(f"{n} {p['exit']}" + (f" {p['verdict']}" if p.get('verdict') else "")
                                           for n, p in ps.items()))
    for p in procs:
        flag = "  GROWS" if p["grows_without_levelling"] else ""
        print(f"  rss {p['name']:18s} {p['comm']:15s} {p['minutes']:6.1f} min {p['first_kb']:>8} -> "
              f"{p['last_kb']:>8} KB (max {p['max_kb']}), KB/h {p['slope_first_half_kb_h']} then "
              f"{p['slope_second_half_kb_h']}, thirds {p['median_by_third_kb']}{flag}")


if __name__ == "__main__":
    main()

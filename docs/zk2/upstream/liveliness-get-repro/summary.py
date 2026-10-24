#!/usr/bin/env python3
"""Tables over lgr's `result` lines (README.md, "Reading the results").

    python3 -I summary.py results-1.10.1.txt results-main.txt results-fix.txt

A path ending in `.xz` is read compressed, as the committed results are
(`results-*.txt.xz`).

Prints three markdown tables:

1. misses per cell: router, load, order, wait down the side; shape x zenoh
   version across. A cell is `misses/trials`; `-` was not run.
2. the misses classified and their recovery, per version, shape, family
   and load: in-flight (the GET found at least every token the subscribers
   had seen, and missed only tokens still on their way: family 1) vs
   lost-known (the GET lost tokens the subscribers already had: family 2);
   how many came back with no reply at all; then whether the second broad
   GET, the union of both broad GETs, the narrow GETs (the three instance
   tokens) and a fresh session found every held token.
3. "read twice" per cell: the trials where neither the first nor the
   second broad GET (their union) found every held token.

`seen_at_get` counts every key the trial's subscribers held at the GET; on
a shared router with subs-first it includes other trials' tokens, so those
misses are not classified (column `?`) unless the wait was `samples`, which
waits for every own token (lost-known by construction).
"""

import lzma
import sys
from collections import defaultdict

VERSIONS = ["1.10.1", "main", "1.10.1+fix"]
SHAPES = "ABCD"
WAIT_ORDER = {"0": 0, "10": 1, "100": 2, "samples": 3, "narrow": 4}


def fields(line):
    d = {}
    for tok in line.split()[1:]:
        if "=" in tok:
            k, v = tok.split("=", 1)
            d[k] = v
    return d


def frac(s):
    tag, rest = s.split(":", 1)
    a, b = rest.split("/")
    return tag, int(a), int(b)


def missing_set(s):
    return set() if s in ("-", "") else set(s.split(","))


def load(paths):
    rows = []
    for p in paths:
        opener = lzma.open if p.endswith(".xz") else open
        with opener(p, "rt") as f:
            for line in f:
                if line.startswith("result "):
                    rows.append(fields(line))
    return rows


def main(paths):
    rows = load(paths)
    cells = defaultdict(lambda: {"n": 0, "miss": 0, "inc": 0, "err": 0, "twice": 0})
    rec = defaultdict(lambda: defaultdict(int))
    for r in rows:
        if "class" not in r:
            key = (r.get("ver"), "?", "?", "?", "?", "?")
            cells[key]["err"] += 1
            continue
        key = (r["ver"], r["mode"], r.get("load", "burn0"), r["order"], r["wait"], r["shape"])
        c = cells[key]
        c["n"] += 1
        cls = r["class"]
        if cls == "incomplete":
            c["inc"] += 1
            continue
        if cls != "MISSED":
            continue
        c["miss"] += 1
        m1 = missing_set(r["missing"])
        m2 = missing_set(r["second_missing"])
        both = m1 & m2
        if both:
            c["twice"] += 1
        _, found, held = frac(r["first"])
        seen = int(r["seen_at_get"])
        if r["mode"] == "shared" and r["order"] == "subs-first":
            fam = "lost-known" if r["wait"] == "samples" else "?"
        else:
            fam = "in-flight" if found >= seen else "lost-known"
        R = rec[(r["ver"], r["shape"], fam, r.get("load", "burn0"))]
        R["miss"] += 1
        if found == 0:
            R["empty"] += 1
        if fam == "in-flight" and r["wait"] == "0" and r["order"] == "tokens-first":
            R["tf0"] += 1
        t2, f2, h2 = frac(r["second"])
        if t2 == "c" and f2 == h2:
            R["second"] += 1
        if not both:
            R["union"] += 1
        a, b = r["narrow"].split("/")
        if a == b:
            R["narrow"] += 1
        tf, ff, hf = frac(r["fresh"])
        if tf == "c" and ff == hf:
            R["fresh"] += 1

    # Table 1.
    sides = sorted(
        {(k[1], k[2], k[3], k[4]) for k in cells if k[1] != "?"},
        key=lambda s: (s[0], s[1], s[2] != "tokens-first", WAIT_ORDER.get(s[3], 9)),
    )
    vers = [v for v in VERSIONS if any(k[0] == v for k in cells)]
    cols = [(s, v) for s in SHAPES for v in vers if any(k[0] == v and k[5] == s for k in cells)]
    hdr = ["router", "load", "order", "wait"] + [f"{s} {v}" for s, v in cols]
    print("| " + " | ".join(hdr) + " |")
    print("|" + "---|" * len(hdr))
    for side in sides:
        out = list(side)
        for s, v in cols:
            c = cells.get((v,) + side + (s,))
            if c is None or c["n"] == 0:
                out.append("-")
            else:
                cell = f"{c['miss']}/{c['n']}"
                if c["miss"]:
                    cell = f"**{cell}**"
                if c["inc"]:
                    cell += f" (+{c['inc']} incomplete)"
                out.append(cell)
        print("| " + " | ".join(out) + " |")
    tot = defaultdict(lambda: [0, 0])
    for k, c in cells.items():
        if k[1] == "?":
            continue
        tot[(k[0], k[5])][0] += c["miss"]
        tot[(k[0], k[5])][1] += c["n"]
    out = ["all", "", "", ""]
    for s, v in cols:
        m, n = tot.get((v, s), (0, 0))
        out.append(f"{m}/{n}" if n else "-")
    print("| " + " | ".join(out) + " |")
    errs = sum(c["err"] for c in cells.values()) + sum(c["inc"] for c in cells.values())
    print(f"\n(lines without a verdict or incomplete: {errs})\n")

    # Table 2.
    print("| zenoh | shape | family | load | misses | of which tokens-first wait 0 | GET empty | 2nd GET found all | union of both found all | narrow found all | fresh session found all |")
    print("|---|---|---|---|---|---|---|---|---|---|---|")
    fam_order = {"in-flight": 0, "lost-known": 1, "?": 2}
    for k in sorted(rec, key=lambda k: (vers.index(k[0]), k[1], fam_order[k[2]], k[3])):
        R = rec[k]
        m = R["miss"]
        pct = lambda x: f"{x}/{m} ({100 * x / m:.0f} %)"
        print(
            f"| {' | '.join(k)} | {m} | {R['tf0']} | {R['empty']} | "
            f"{pct(R['second'])} | {pct(R['union'])} | {pct(R['narrow'])} | {pct(R['fresh'])} |"
        )
    print()

    # Table 3.
    print("| zenoh | router | load | order | wait | shape | trials | first GET missed | both GETs missed (union incomplete) |")
    print("|---|---|---|---|---|---|---|---|---|")
    for k in sorted(cells, key=lambda k: (k[0], k[1], k[2], k[3], WAIT_ORDER.get(k[4], 9), k[5])):
        c = cells[k]
        if k[1] == "?" or not c["miss"]:
            continue
        print(f"| {' | '.join(k)} | {c['n']} | {c['miss']} | {c['twice']} |")


if __name__ == "__main__":
    main(sys.argv[1:])

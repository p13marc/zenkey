#!/usr/bin/env python3
"""The README's tables, from `run-matrix.sh`'s result lines.

    python3 -I summary.py results-1.10.1.txt results-fix.txt
"""
import collections
import statistics
import sys


def fields(line):
    out = {}
    for part in line.split()[1:]:
        if "=" in part:
            k, v = part.split("=", 1)
            out[k] = v
    return out


def short(frame):
    """A frame as the README writes it: `Face::send_declare` for the
    trait impl gdb prints as `{impl#10}::send_declare`."""
    return frame.replace("adminspace::", "").replace("{impl#10}::", "Face::")


def main(paths):
    cells = collections.OrderedDict()
    for path in paths:
        for line in open(path):
            if not line.startswith("result "):
                continue
            f = fields(line)
            key = (f.get("ver"), f.get("selector"), f.get("load"), f.get("shape"), f.get("churn"), f.get("every_ms", "0"))
            cells.setdefault(key, []).append(f)
    print("| zenoh | selector | load | shape | churn | pause (ms) | hung | first GET's replies | froze at (ms, median) | replier | writer |")
    print("|---|---|---|---|---|---|---|---|---|---|---|")
    for (ver, sel, load, shape, churn, every), rows in cells.items():
        hung = [r for r in rows if r.get("outcome") == "hung"]
        errors = [r for r in rows if r.get("outcome") == "error"]
        t = [int(r["t_ms"]) for r in hung if "t_ms" in r]
        first = sorted({r.get("first_replies", "?") for r in rows})
        repliers = sorted({short(r.get("replier", "")) for r in hung} - {""})
        writers = sorted({short(r.get("writer", "")) for r in hung} - {""})
        froze = f"{round(statistics.median(t)) + 0}" if t else "—"
        err = f" ({len(errors)} errors)" if errors else ""
        print(
            f"| {ver} | `{sel}` | {load} | {shape} | {churn} | {every} | {len(hung)} of {len(rows)}{err} "
            f"| {', '.join(first)} | {froze} | {', '.join(repliers) or '—'} | {', '.join(writers) or '—'} |"
        )


if __name__ == "__main__":
    main(sys.argv[1:])

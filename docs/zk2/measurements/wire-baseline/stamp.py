#!/usr/bin/env python3
"""Stamp each `zenctl echo --format ndjson` row with the wall clock, keep key and value (#989).

Usage: zenctl echo ... --format ndjson | stamp.py <out.jsonl> [--all]
A row that is not JSON is kept as {"t": ..., "line": ...}. With --all every
member of the row is kept (#989's zk2c runs: a frame's records, its `follows`).
"""
import json
import sys
import time

with open(sys.argv[1], "a", buffering=1) as out:
    for line in sys.stdin:
        t = round(time.time(), 6)
        try:
            row = json.loads(line)
        except ValueError:
            out.write(json.dumps({"t": t, "line": line.rstrip("\n")}) + "\n")
            continue
        keep = {"t": t, "key": row.get("key"), "value": row.get("value"),
                "n": row.get("payload_bytes")}
        if "--all" in sys.argv[2:]:
            keep = {**row, **keep}
        if row.get("delete"):
            keep["delete"] = True
        out.write(json.dumps(keep) + "\n")

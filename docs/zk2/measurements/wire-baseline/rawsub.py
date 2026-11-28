#!/usr/bin/env python3
"""#1185's live check: a plain zenoh subscriber on one key expression, a client of one
router, each sample one JSON line {t, key, n}. zenctl refuses `$*` in a selector (tooling
guide §8.2), so the check's repair subscribers are this.

Usage: rawsub.py <session config json5> <endpoint> <key expression> <out.jsonl> <seconds>
"""
import json
import sys
import time

import zenoh

cfg_path, endpoint, key, out_path, secs = sys.argv[1:6]
conf = zenoh.Config.from_file(cfg_path)
conf.insert_json5("mode", '"client"')
conf.insert_json5("connect/endpoints", json.dumps([endpoint]))
conf.insert_json5("scouting/multicast/enabled", "false")
out = open(out_path, "a", buffering=1)


def heard(s):
    out.write(json.dumps({"t": round(time.time(), 6), "key": str(s.key_expr),
                          "n": len(s.payload.to_bytes())}) + "\n")


with zenoh.open(conf) as session:
    sub = session.declare_subscriber(key, heard)
    out.write(json.dumps({"t": round(time.time(), 6), "declared": key}) + "\n")
    time.sleep(float(secs))

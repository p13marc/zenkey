#!/usr/bin/env python3
"""Appendix A's figures for W1 and the idle link on RF, per direction (#989).

Runs the report's own Appendix A, the Python block of
docs/zk2/constrained-overhead-review.md, for its functions, then states the
model's W1 and idle figures per direction, for the lean stack the report
assumes and for no layers at all. Standard library only.

Usage: model.py [path/to/constrained-overhead-review.md]
"""
import contextlib
import io
import json
import os
import re
import sys

report = sys.argv[1] if len(sys.argv) > 1 else os.path.join(
    os.path.dirname(os.path.abspath(__file__)), "..", "..", "constrained-overhead-review.md")
code = re.search(r"```python\n(.*?)```", open(report).read(), re.S).group(1)
ns = {}
with contextlib.redirect_stdout(io.StringIO()):
    exec(code, ns)

put, sdus, LEAN, FRAME, QOS, KEEPALIVE, RF_MTU, DAY = (
    ns[k] for k in ("put", "sdus", "LEAN", "FRAME", "QOS", "KEEPALIVE", "RF_MTU", "DAY"))
RF_BPS = 2400


def row(sdu_count, byte_count):
    return {"sdus": round(sdu_count), "bytes": round(byte_count),
            "airtime_s": round(byte_count * 8 / RF_BPS, 1)}


out = {}
for stack, layers in (("lean", LEAN), ("bare", 0)):
    link = put(6, hlc=True, enc=1, qos=True)              # the report's 4 W1 readings
    n, b = sdus([link], RF_MTU, layers=layers, qos=True)
    data = row(4 * 1440 * n, 4 * 1440 * b)
    ka = KEEPALIVE + layers
    idle_ka = DAY / 2.5                                    # lease 10 s / keep_alive 4, one way
    # W1's uplink is busy once a minute, which resets its keep-alive timer:
    # 23 keep-alives a minute instead of 24 (the report's W2 reasoning).
    w1_up_ka = 1440 * 23
    out[stack] = {
        "w1_put_bytes": link, "w1_sdu_bytes": b,
        "idle": {"per_direction": row(idle_ka, idle_ka * ka)},
        "w1": {
            "a_to_b": {"data": data, "keepalive": row(w1_up_ka, w1_up_ka * ka)},
            "b_to_a": {"keepalive": row(idle_ka, idle_ka * ka)},
        },
    }

# W2 on RF (Appendix A's section 6): each element's bytes a day, both
# directions together, as the report states them, today and under zk2c.
out["w2_rf"] = {k: {"today_bytes": round(v[0]), "zk2c_bytes": round(v[1])}
                for k, v in ns["rows"].items()}
out["w2_rf"]["total"] = {"today_bytes": round(ns["today"]), "zk2c_bytes": round(ns["after"])}

# W1 and the idle link on LoRa (Appendix A's sections 8 and 9), per direction:
# the 4 readings each in its own SDU under the lean stack (tiles of 51 bytes at
# SF10-12), and one keep-alive per lease / keep_alive on an idle direction.
air, lora_air_s, link_today = ns["air"], ns["lora_air_s"], ns["link_today"]
leases = {7: (60, 2), 9: (60, 2), 12: (540, 2)}
out["lora"] = {}
for sf, (lease, ka) in leases.items():
    per_dir = DAY * ka / lease
    out["lora"][f"sf{sf}"] = {
        "lease_s": lease, "keep_alive": ka,
        "w1_data_a_to_b": {"sdus": 4 * 1440, "airtime_s": round(4 * 1440 * air(link_today, sf), 1)},
        "keepalive_per_direction": {"sdus": round(per_dir),
                                    "airtime_s": round(per_dir * lora_air_s(KEEPALIVE + LEAN, sf), 1)},
        "duty_budget_s_per_day": ns["LORA_DUTY_S"],
    }
print(json.dumps(out, indent=1))

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
import math
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


# The same rows today, per direction: each element under its sender. The
# device's elements, the desired GET's request, the uplink's keep-alives and
# the device side's declarations go A->B; the desired reply, the idle
# downlink's keep-alives and the reader's subscription B->A. The report packs
# a reconnect's declarations into one batch; here each direction packs its own.
def sent(messages, qos=False):
    return sdus(messages, RF_MTU, qos=qos)[1]


rows = ns["rows"]
w2_dir = {
    "a_to_b": {el: rows[el][0] for el in ("telemetry", "health", "alarms", "config", "events")},
    "b_to_a": {},
}
w2_dir["a_to_b"]["desired"] = sent([ns["req"]]) * 288
w2_dir["b_to_a"]["desired"] = sent([ns["rep"]]) * 288
w2_dir["a_to_b"]["keep-alive"] = 3 * ns["cycles"] * (KEEPALIVE + LEAN)
w2_dir["b_to_a"]["keep-alive"] = (DAY // 2.5) * (KEEPALIVE + LEAN)
w2_dir["a_to_b"]["reconnect"] = 4 * (ns["hs"] / 2 + sent([ns["tok"], ns["kexp"], ns["qbl"]]))
w2_dir["b_to_a"]["reconnect"] = 4 * (ns["hs"] / 2 + sent([ns["sub"]]))
out["w2_rf_per_direction"] = {
    d: {**{el: {"today_bytes": round(v), "airtime_s": round(v * 8 / RF_BPS, 1)} for el, v in els.items()},
        "total": {"today_bytes": round(sum(els.values())),
                  "airtime_s": round(sum(els.values()) * 8 / RF_BPS, 1)}}
    for d, els in w2_dir.items()}

# W1 and the idle link on LoRa (Appendix A's sections 8 and 9), per direction:
# the 4 readings each in its own SDU under the lean stack (tiles of 51 bytes at
# SF10-12), and one keep-alive per lease / keep_alive on an idle direction.
air, lora_air_s, link_today = ns["air"], ns["lora_air_s"], ns["link_today"]
# deploy/README.md's timers, and SF7 at keep_alive 4 (the run sf7ka4).
leases = {"sf7": (7, 60, 2), "sf9": (9, 60, 2), "sf12": (12, 540, 2), "sf7ka4": (7, 60, 4)}
out["lora"] = {}
for name, (sf, lease, ka) in leases.items():
    per_dir = DAY * ka / lease
    ka_air = lora_air_s(KEEPALIVE + LEAN, sf)
    # W1's uplink is busy once a minute, which resets its keep-alive timer:
    # one keep-alive falls due between two ticks when lease / keep_alive < 60 s.
    w1_ka = 1440 * (math.ceil(60 / (lease / ka)) - 1)
    out["lora"][name] = {
        "lease_s": lease, "keep_alive": ka,
        "w1_data_a_to_b": {"sdus": 4 * 1440, "airtime_s": round(4 * 1440 * air(link_today, sf), 1)},
        "w1_keepalive_a_to_b": {"sdus": w1_ka, "airtime_s": round(w1_ka * ka_air, 1)},
        "keepalive_per_direction": {"sdus": round(per_dir), "airtime_s": round(per_dir * ka_air, 1)},
        "duty_budget_s_per_day": ns["LORA_DUTY_S"],
    }
print(json.dumps(out, indent=1))

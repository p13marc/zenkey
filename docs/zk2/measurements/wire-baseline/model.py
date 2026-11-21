#!/usr/bin/env python3
"""Appendix A's figures for W1 and the idle link on RF, per direction (#989), and zk2c's.

Runs the report's own Appendix A, the Python block of
docs/zk2/constrained-overhead-review.md, for its functions, then states the
model's W1 and idle figures per direction, for the lean stack the report
assumes and for no layers at all. Standard library only.

Usage: model.py [path/to/constrained-overhead-review.md] [--zk2c]
(--zk2c: only the zk2c figures, results/model-zk2c.json)
"""
import contextlib
import io
import json
import math
import os
import re
import sys

args = [a for a in sys.argv[1:] if not a.startswith("--")]
report = args[0] if args else os.path.join(
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
# zk2c (#989's zk2c runs): Appendix A's zk2c column for W2, each element under
# its sender, as the today rows above; W1's 73 readings; and the idle link at
# link.v2's transport (lease 240 s, keep_alive 4: a keep-alive every 60 s on an
# idle direction).
creq, crep = ns["creq"], ns["crep"]
reopen, zk2c_decl, RECONNECTS = ns["reopen"], ns["zk2c_decl"], ns["RECONNECTS"]
z = {el: v[1] for el, v in rows.items()}
zdir = {
    "a_to_b": {**{el: z[el] for el in ("telemetry", "health", "alarms", "config", "events",
                                       "heartbeat")},
               "desired": sent([creq]) * 288,
               "reconnect": RECONNECTS * sum(reopen("ab", zk2c_decl, admin=False))},
    "b_to_a": {"desired": sent([crep]) * 288, "keep-alive": z["keep-alive"],
               "reconnect": RECONNECTS * sum(reopen("ba", [], admin=False))},
}
assert round(sum(sum(d.values()) for d in zdir.values())) == round(ns["after"])
out["w2_rf_zk2c_per_direction"] = {
    d: {**{el: {"zk2c_bytes": round(v), "airtime_s": round(v * 8 / RF_BPS, 1)} for el, v in els.items()},
        "total": {"zk2c_bytes": round(sum(els.values())),
                  "airtime_s": round(sum(els.values()) * 8 / RF_BPS, 1)}}
    for d, els in zdir.items()}
B, key32, delta = ns["B"], ns["key32"], ns["delta"]
out["w1_73_rf_zk2c"] = {
    "keyframe_payload": B(key32), "delta_payload": B(delta),
    "keyframes_a_day": 96, "deltas_a_day": 1344,
    "a_to_b_bytes": round(ns["ac"]),
    "note": "Appendix A section 8: a keyframe every 15 minutes and a delta each minute, "
            "each a data_low put in its own SDU, under the lean stack",
}
idle = DAY * 4 / 240
out["idle_rf_zk2c"] = {stack: {"per_direction": row(idle, idle * (KEEPALIVE + layers))}
                       for stack, layers in (("lean", LEAN), ("bare", 0))}
# W1-73 across lora0's faces (#989's LoRa runs), Appendix A section 10's frames under the
# lean stack, on the face's 3-byte frame, at each face's cadence and transport:
# - SF7: frames/fast every 60 s; lease 120 s, keep_alive 4, a keep-alive every 30 s; zenoh's
#   batch the driver's 206 B, so a full frame in two SDUs and a delta in one;
# - SF9: frames/fast every 120 s; lease 480 s, a keep-alive every 120 s; 115-B SDUs;
# - SF12: frames/slow every 900 s; lease 3,600 s, a keep-alive every 900 s; 51-B tiles.
# gen's frames alternate a full record of every reading and one of none, a day half of each.
# The corrected frames are section 10's: a keyframe every 15 minutes with 24 readings moving
# (each minute at SF7, every 2 minutes at SF9), an hourly keyframe with 15-minute deltas at
# SF12.
#
# The uplink's keep-alives beside its frames are link.v2 §2.16's, as section 10 states the
# rule (keepalives_beside): ceil(C / I) - 1 between two frames a cadence C apart at an
# interval I, and C / I - 1 to C / I where C is a whole multiple of I, as at each of these
# faces. So each W1 uplink row is a range, fewest to most. The rule does not narrow it, and
# no constant here is fitted to a run.
#
# Each row is then checked against its run's analysis.json, where results/ holds it, and
# each tolerance is asserted:
# - gen's frames' air: within FRAMES_TOL of the model's;
# - the uplink's keep-alives beside the frames: inside the rule's range;
# - the uplink's whole air: inside the range of the model's frames with the fewest and the
#   most keep-alives, each end widened by FRAMES_TOL;
# - an idle direction's keep-alives (the idle window both ways, W1's downlink): the model's
#   count exactly, and their air within IDLE_AIR_TOL_S.
FRAMES_TOL = 0.01
IDLE_AIR_TOL_S = 0.1
toa = ns["lora_toa_s"]
face_air, w1_73_uplink, keepalives_beside = ns["face_air"], ns["w1_73_uplink"], ns["keepalives_beside"]
within = ns["within"]
FRAMES_CASE = {7: "a keyframe every 15 min, a delta every minute",
               9: "a keyframe every 15 min, a delta every 2 min",
               12: "an hourly keyframe, a delta every 15 min"}
RESULTS = os.path.join(os.path.dirname(os.path.abspath(__file__)), "results")


def lora_sdus(message, batch=206, layers=LEAN):
    cap = batch - ns["FRAME16"]
    if message <= cap:
        return [message + ns["FRAME16"] + layers]
    out, rest = [], message + ns["FIRST_FRAG"]
    while rest > 0:
        out.append(min(cap, rest) + ns["FRAME16"] + layers)
        rest -= cap
    return out


def air7(sdu_list):
    return sum(toa(x, 7) for x in sdu_list)


def measured(run):
    """A run's W1 and idle windows, a day, from results/<run>/analysis.json; None without it."""
    path = os.path.join(RESULTS, run, "analysis.json")
    if not os.path.exists(path):
        return None
    wins = {w["window"]: w for w in json.load(open(path))["windows"]}
    up = wins["w1"]["ab"]["per_day"]
    ka = up["by_kind"].get("keep-alive", {"sdus": 0, "airtime_s": 0.0})
    return {
        "run": run,
        # Every SDU of the window that is no keep-alive is a frame's: no link opened again.
        "frames_airtime_s": round(sum(v["airtime_s"] for k, v in up["by_kind"].items()
                                      if k != "keep-alive"), 1),
        "uplink_keepalive": {"sdus": ka["sdus"], "airtime_s": ka["airtime_s"]},
        "uplink_airtime_s": up["airtime_s"],
        "idle_keepalive": {f"{w}_{d}": {"sdus": wins[w][d]["per_day"]["keepalives"],
                                        "airtime_s": wins[w][d]["per_day"]["airtime_s"]}
                           for w, d in (("idle", "ab"), ("idle", "ba"), ("w1", "ba"))},
    }


def face_row(sf, run, gen_frames_s, sdu_lists=None):
    """One face's W1-73 row: the frames, the keep-alives by link.v2 §2.16, and its run."""
    u = w1_73_uplink[sf]
    ka_air = u["keepalive_air_s"]
    fewest, most = u["keepalives"]
    idle_ka = DAY * 4 // u["lease_s"]
    a_gap = keepalives_beside(u["cadence_s"], u["lease_s"])
    assert within(gen_frames_s, u["gen_frames_s"], 0.001)          # section 10's, by another road
    row = {
        "period_s": u["cadence_s"], "lease_s": u["lease_s"], "keep_alive": 4,
        "keepalive_interval_s": u["lease_s"] // 4,
        "gen_frames_a_day": {"full": u["gaps"] // 2, "none": u["gaps"] // 2},
        **({"gen_frame_sdus": sdu_lists} if sdu_lists else {}),
        "gen_frames_airtime_s": round(gen_frames_s, 1),
        "frames_airtime_s": round(u["frames_s"], 1), "frames_case": FRAMES_CASE[sf],
        "uplink_keepalive": {
            "rule": "link.v2 §2.16: " + (
                "the cadence a whole multiple of the interval, C / I - 1 to C / I a gap"
                if a_gap[0] != a_gap[1] else "ceil(C / I) - 1 a gap"),
            "a_gap": list(a_gap),
            "sdus": [fewest, most],
            "airtime_s": [round(fewest * ka_air, 1), round(most * ka_air, 1)]},
        "w1_uplink_airtime_s": {
            "gen_frames": [round(gen_frames_s + k * ka_air, 1) for k in (fewest, most)],
            "frames": [round(u["frames_s"] + k * ka_air, 1) for k in (fewest, most)]},
        "idle_keepalive_per_direction": {"sdus": idle_ka, "airtime_s": round(idle_ka * ka_air, 1)},
        "keepalive_bytes": KEEPALIVE + LEAN + (6 if sf >= 10 else 0),
        "duty_budget_s_per_day": ns["LORA_DUTY_S"],
        "tolerance": {"frames_airtime": FRAMES_TOL, "uplink_keepalive": "inside the rule's range",
                      "uplink_airtime": f"inside the range, each end widened by {FRAMES_TOL}",
                      "idle_keepalive": f"the count exact, its air within {IDLE_AIR_TOL_S} s"},
    }
    m = measured(run)
    if m:
        assert within(gen_frames_s, m["frames_airtime_s"], FRAMES_TOL), (run, m)
        assert fewest <= m["uplink_keepalive"]["sdus"] <= most, (run, m)
        lo, hi = row["w1_uplink_airtime_s"]["gen_frames"]
        assert lo * (1 - FRAMES_TOL) <= m["uplink_airtime_s"] <= hi * (1 + FRAMES_TOL), (run, m)
        for where, ka in m["idle_keepalive"].items():
            assert ka["sdus"] == idle_ka, (run, where, ka)
            assert abs(ka["airtime_s"] - idle_ka * ka_air) <= IDLE_AIR_TOL_S, (run, where, ka)
        row["measured"] = m
    return row


out["w1_73_sf7_zk2c"] = face_row(
    7, "zk2c-sf7-lean-1",
    720 * air7(lora_sdus(put(ns["ENV32"] + ns["full_body"])))
    + 720 * air7(lora_sdus(put(ns["ENV32"] + ns["none_body"]))))
for sf in (9, 12):
    mtu = 115 if sf == 9 else 222
    bodies = {"full": put(ns["ENV16"] + ns["full_body"]), "none": put(ns["ENV16"] + ns["none_body"])}
    out[f"w1_73_sf{sf}_zk2c"] = face_row(
        sf, f"zk2c-sf{sf}-lean-1",
        w1_73_uplink[sf]["gaps"] // 2 * sum(face_air(b, sf) for b in bodies.values()),
        {k: ns["sdu_list16"]([b], mtu=mtu) for k, b in bodies.items()})
# W1-73 across rf0's face: a frame every 60 s at lease 240 s, keep_alive 4. The cadence is
# the interval, so by the same rule the uplink sends none to one keep-alive a gap, where the
# first model had none. Asserted against the bare and the lean run, where results/ holds
# them: each run's count inside the range. The frames' bytes stay section 8's and section
# 10's (a_to_b_bytes, corrected_frames_bytes).
rf_face, (rf_fewest, rf_most) = ns["W1_73_RF_BESIDE"], ns["rf_beside"]
rf_measured = {}
for stack in ("bare", "lean"):
    path = os.path.join(RESULTS, f"zk2c-w1-{stack}-1", "analysis.json")
    if os.path.exists(path):
        w1 = next(w for w in json.load(open(path))["windows"] if w["window"] == "w1")
        ka = w1["ab"]["per_day"]["by_kind"].get("keep-alive", {"sdus": 0, "bytes": 0})
        assert rf_fewest <= ka["sdus"] <= rf_most, (stack, ka)
        assert ka["sdus"] == rf_face["measured"][stack], (stack, ka)   # section 10's constants
        rf_measured[stack] = {"run": f"zk2c-w1-{stack}-1", "sdus": ka["sdus"], "bytes": ka["bytes"]}
out["w1_73_rf_zk2c"].update({
    "corrected_frames_bytes": round(ns["w1_73"]),
    "uplink_keepalive": {
        "rule": "link.v2 §2.16: the cadence a whole multiple of the interval, C / I - 1 to C / I a gap",
        "a_gap": list(keepalives_beside(rf_face["cadence_s"], rf_face["lease_s"])),
        "sdus": [rf_fewest, rf_most],
        "bytes": {"lean": [rf_fewest * (KEEPALIVE + LEAN), rf_most * (KEEPALIVE + LEAN)],
                  "bare": [rf_fewest * KEEPALIVE, rf_most * KEEPALIVE]},
        "tolerance": "inside the rule's range",
        "measured": rf_measured},
    "corrected_a_to_b_bytes": [round(ns["w1_73"] + k * (KEEPALIVE + LEAN)) for k in (rf_fewest, rf_most)],
})
if "--zk2c" in sys.argv:
    # results/model-zk2c.json: the zk2c figures alone, so results/model.json
    # stays the record phases 1 and 2 were tabled against.
    out = {k: v for k, v in out.items() if "zk2c" in k}
print(json.dumps(out, indent=1))

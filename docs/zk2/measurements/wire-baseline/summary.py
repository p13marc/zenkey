#!/usr/bin/env python3
"""Print the README's tables from results/*/analysis.json and model.json (#989).

Usage: summary.py [results-dir]
Standard library only.
"""
import json
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
RES = sys.argv[1] if len(sys.argv) > 1 else os.path.join(HERE, "results")
LEAN = 16


def load(run):
    p = os.path.join(RES, run, "analysis.json")
    return json.load(open(p)) if os.path.exists(p) else None


def win(a, name):
    return next((w for w in a["windows"] if w["window"] == name), None) if a else None


def kb(x):
    return f"{x / 1000:,.1f} KB"


def w2_table(bare, lean, model):
    """W2's elements per day, both directions: bare decoded, lean derived, the model's."""
    sb, sl = win(bare, "steady"), win(lean, "steady")
    rows = {}
    for d in ("ab", "ba"):
        for el, v in sb["elements_per_day"][d].items():
            r = rows.setdefault(el, {"bytes": 0, "sdus": 0})
            r["bytes"] += v["bytes"]
            r["sdus"] += v["sdus"]
    acts = bare["w2"].get("acts", {})
    act_day = {}
    for kind, dirs in acts.items():
        el = {"health": "health", "event": "events", "alarm_raise": "alarms",
              "alarm_resolve": "alarms", "config": "config"}[kind]
        for d, v in dirs.items():
            a = act_day.setdefault(el, {"bytes": 0, "sdus": 0})
            a["bytes"] += v["per_day"]["bytes"]
            a["sdus"] += v["per_day"]["sdus"]
    rec = bare["w2"].get("reconnect", {})
    rc = {"bytes": sum(4 * rec[d]["cost"]["bytes"] for d in rec if "cost" in rec[d]),
          "sdus": sum(4 * rec[d]["cost"]["sdus"] for d in rec if "cost" in rec[d])}
    print("| Element | bare, steady | acts a day | bare total | lean, derived (+16 B an SDU) | model (lean) |")
    print("|---|---:|---:|---:|---:|---:|")
    order = ("telemetry", "health", "alarms", "config", "desired", "events", "keep-alive",
             "presence", "declarations", "admin", "interests", "other")
    tot = {"b": 0, "l": 0}
    for el in order + ("reconnect",):
        if el == "reconnect":
            st, ac = {"bytes": 0, "sdus": 0}, rc
        else:
            st = rows.get(el, {"bytes": 0, "sdus": 0})
            ac = act_day.get(el, {"bytes": 0, "sdus": 0})
        b = st["bytes"] + ac["bytes"]
        n = st["sdus"] + ac["sdus"]
        lean_b = b + LEAN * n
        m = model.get(el, {}).get("today_bytes")
        if not b and m is None:
            continue
        tot["b"] += b
        tot["l"] += lean_b
        print(f"| {el} | {kb(st['bytes'])} | {kb(ac['bytes'])} | {kb(b)} | {kb(lean_b)} | "
              f"{kb(m) if m is not None else '—'} |")
    print(f"| **total** | | | **{kb(tot['b'])}** | **{kb(tot['l'])}** | **{kb(model['total']['today_bytes'])}** |")
    if sl:
        meas = sum(sl[d]["per_day"]["bytes"] for d in ("ab", "ba"))
        print(f"\nlean, measured steady (counters, both directions): {kb(meas)} a day")


def main():
    model = json.load(open(os.path.join(RES, "model.json")))
    bare, lean = load("w2-rf-bare-3"), load("w2-rf-lean-1")
    if bare:
        print("## W2 on RF, per element, both directions, per day\n")
        w2_table(bare, lean, model["w2_rf"])
        for name, a in (("bare", bare), ("lean", lean)):
            s = win(a, "steady")
            if not s:
                continue
            print(f"\n{name} steady:")
            for d in ("ab", "ba"):
                p = s[d]["per_day"]
                print(f"  {d}: {p['sdus']:,} SDUs, {kb(p['bytes'])}, {p['airtime_s']:,} s of air ({p['airtime_pct']} %)")
            print("  per_day with acts and reconnects:", a["w2"]["per_day"])
    print("\n## LoRa, per direction, per day\n")
    print("| SF | window | dir | SDUs | bytes | airtime | duty | InitSyns | model |")
    print("|---|---|---|---:|---:|---:|---:|---:|---|")
    for run in ("sf7-lean-2", "sf9-lean-2", "sf12-lean-2", "sf7ka4-lean-1"):
        a = load(run)
        if not a:
            continue
        lm = model["lora"].get(a["medium"].split("ka")[0], {})
        for w in a["windows"]:
            scale = 86_400 / w["seconds"]
            for d in ("ab", "ba"):
                x = w[d]
                p = x["per_day"]
                opened = x.get("links_opened")
                if w["window"] == "idle":
                    mod = f"{lm.get('keepalive_per_direction', {}).get('sdus', '—')} keep-alives, " \
                          f"{lm.get('keepalive_per_direction', {}).get('airtime_s', '—')} s"
                elif d == "ab":
                    mod = f"data {lm['w1_data_a_to_b']['sdus']} SDUs, {lm['w1_data_a_to_b']['airtime_s']} s"
                else:
                    mod = f"{lm['keepalive_per_direction']['sdus']} keep-alives"
                print(f"| {a['medium']} | {w['window']} ({w['seconds']:.0f} s) | {d} | {p['sdus']:,} | "
                      f"{kb(p['bytes'])} | {p['airtime_s']:,} s | {x.get('duty_pct_of_eu868')} % | "
                      f"{'' if opened is None else round(opened * scale)} | {mod} |")
                bk = p.get("by_kind", {})
                if bk:
                    print("|  |  |  | " + "; ".join(f"{k} {v['sdus']:,}/{kb(v['bytes'])}/{v['airtime_s']} s"
                                                     for k, v in bk.items()) + " | | | | | |")


if __name__ == "__main__":
    main()

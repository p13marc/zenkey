#!/usr/bin/env python3
"""Print the README's tables from results/*/analysis.json and model.json (#989).

Usage: summary.py [results-dir]

- W2 on RF, per direction and element, a day: the bare run decoded (its
  steady window, and its acts at W2's daily counts and 4 reconnects when the
  run measured them), the lean stack derived from it (+16 bytes an SDU), the
  lean run's counters, and the report's model split by sender (model.json's
  w2_rf_per_direction).
- LoRa, per window and direction, a day: the lean runs' counters and airtime,
  the duty used, the keep-alives lost and the links opened again in the
  window, and the model (model.json's lora).
Standard library only.
"""
import json
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
RES = sys.argv[1] if len(sys.argv) > 1 else os.path.join(HERE, "results")
LEAN = 16
DAY = 86_400
NAMES = {"ab": "A->B", "ba": "B->A"}
MODEL_DIR = {"ab": "a_to_b", "ba": "b_to_a"}
ACT_ELEMENT = {"health": "health", "event": "events", "alarm_raise": "alarms",
               "alarm_resolve": "alarms", "config": "config"}
ORDER = ("telemetry", "health", "alarms", "config", "desired", "events", "keep-alive",
         "presence", "declarations", "admin", "interests", "oam", "other", "reconnect")


def load(run):
    p = os.path.join(RES, run, "analysis.json")
    return json.load(open(p)) if os.path.exists(p) else None


def win(a, name):
    return next((w for w in a["windows"] if w["window"] == name), None) if a else None


def kb(x):
    return f"{x / 1000:,.1f} KB"


def w2_tables(bare, lean, model):
    steady = win(bare, "steady")
    acts = bare["w2"].get("acts", {})
    rec = bare["w2"].get("reconnect", {})
    per_reconnect = bare["w2"]["reconnects_per_day"]
    for d in ("ab", "ba"):
        md = model[MODEL_DIR[d]]
        rows = {}
        for el, v in steady["elements_per_day"][d].items():
            rows.setdefault(el, {}).update(steady=v)
        for kind, dirs in acts.items():
            v = dirs.get(d, {}).get("per_day")
            if v:
                r = rows.setdefault(ACT_ELEMENT[kind], {}).setdefault("acts", {"bytes": 0, "sdus": 0})
                r["bytes"] += v["bytes"]
                r["sdus"] += v["sdus"]
        cost = rec.get(d, {}).get("cost")
        if cost:
            rows["reconnect"] = {"reconnects": {k: per_reconnect * cost[k] for k in ("bytes", "sdus")}}
        kinds = ("steady", "acts", "reconnects") if acts or rec else ("steady",)
        heads = {"steady": f"bare, steady ({steady['seconds']:.0f} s)", "acts": "bare, acts",
                 "reconnects": "bare, reconnects"}
        print(f"\n#### W2 {NAMES[d]}, a day\n")
        print("| Element | " + " | ".join(heads[k] for k in kinds) + (" | bare, total" if len(kinds) > 1 else "")
              + " | bare, SDUs | lean, derived | model (lean) |")
        print("|---|" + "---:|" * (len(kinds) + (1 if len(kinds) > 1 else 0) + 3))
        tot = {"b": 0, "n": 0, "m": 0}
        for el in ORDER:
            r = rows.get(el)
            m = md.get(el, {}).get("today_bytes")
            if not r and m is None:
                continue
            r = r or {}
            parts = [r.get(k, {"bytes": 0, "sdus": 0}) for k in kinds]
            b = sum(p["bytes"] for p in parts)
            n = sum(p["sdus"] for p in parts)
            tot["b"] += b
            tot["n"] += n
            cells = [kb(p["bytes"]) if p["bytes"] else "" for p in parts]
            if len(kinds) > 1:
                cells.append(kb(b))
            print(f"| {el} | " + " | ".join(cells) + f" | {n:,.0f} | {kb(b + LEAN * n)} | "
                  f"{kb(m) if m is not None else '—'} |")
        blanks = " | " * (len(kinds) - 1)
        print(f"| **total** |{blanks} **{kb(tot['b'])}** | {tot['n']:,.0f} | "
              f"**{kb(tot['b'] + LEAN * tot['n'])}** | **{kb(md['total']['today_bytes'])}** |")
    for name, a in (("bare", bare), ("lean", lean)):
        s = win(a, "steady")
        if not s:
            continue
        print(f"\n{name}, measured by the counters (steady window, {s['seconds']:.0f} s), a day:")
        for d in ("ab", "ba"):
            p = s[d]["per_day"]
            print(f"- {NAMES[d]}: {p['sdus']:,} SDUs, {kb(p['bytes'])}, {p['airtime_s']:,} s of air "
                  f"({p['airtime_pct']} %); relay {s[d]['relay_sdus']} SDUs, {s[d]['relay_dropped']} lost")
        if a["w2"].get("not_measured"):
            print(f"- not measured: {a['w2']['not_measured']}")


OPEN_S = {"sf12": 40}


def episodes(times, gap=12.5):
    """InitSyns less than `gap` s apart are one reopen: a refused InitSyn is retried 1 s later,
    and one that draws nothing after the open timeout (10 s; 40 s at SF12)."""
    out, last = [], None
    for t in times:
        if last is None or t - last > gap:
            out.append(t)
        last = t
    return out


def lora_table(model):
    print("\n#### LoRa, per window and direction, a day\n")
    print("| Run | Window | Direction | SDUs | Bytes | Airtime | Duty used | "
          "Keep-alives lost; links reopened (s into the window) | Model: SDUs, airtime |")
    print("|---|---|---|---:|---:|---:|---:|---|---|")
    for run in ("sf7-lean-2", "sf7ka4-lean-1", "sf9-lean-2", "sf12-lean-2"):
        a = load(run)
        if not a:
            continue
        lm = model["lora"][a["medium"]]
        for w in a["windows"]:
            for d in ("ab", "ba"):
                x = w[d]
                p = x["per_day"]
                if w["window"] == "w1" and d == "ab":
                    md, mk = lm["w1_data_a_to_b"], lm["w1_keepalive_a_to_b"]
                    mod = (f"{md['sdus'] + mk['sdus']:,}" + (f" ({md['sdus']:,} data)" if mk["sdus"] else " data")
                           + f", {md['airtime_s'] + mk['airtime_s']:,.1f} s")
                else:
                    k = lm["keepalive_per_direction"]
                    mod = f"{k['sdus']:,} keep-alives, {k['airtime_s']:,} s"
                lost = x.get("keepalives_lost_at", [])
                ab = w["ab"].get("links_opened_at", [])
                gap = OPEN_S.get(a["medium"], 10) + 2.5
                lost_s = ", ".join(f"+{t}" for t in lost) or "none"
                opened = ", ".join(f"+{t}" for t in episodes(ab, gap)) or "never"
                ev = f"lost {lost_s}" + (f"; reopened {opened}" if d == "ab" else "")
                duty = f"{p['airtime_s'] / 864 * 100:,.0f} %"
                print(f"| {run} | {w['window']} ({w['seconds']:.0f} s) | {NAMES[d]} | {p['sdus']:,} | "
                      f"{kb(p['bytes'])} | {p['airtime_s']:,} s | {duty} | {ev} | {mod} |")
                if w["window"] == "w1" and d == "ab":
                    for k, v in p.get("by_kind", {}).items():
                        print(f"| | | of which {k} | {v['sdus']:,} | {kb(v['bytes'])} | {v['airtime_s']:,} s "
                              f"| {v['airtime_s'] / 864 * 100:,.0f} % | | |")


def main():
    model = json.load(open(os.path.join(RES, "model.json")))
    bare, lean = load("w2-rf-bare-3"), load("w2-rf-lean-1")
    if bare:
        print("## W2 on RF")
        w2_tables(bare, lean, model["w2_rf_per_direction"])
    print("\n## LoRa")
    lora_table(model)


if __name__ == "__main__":
    main()

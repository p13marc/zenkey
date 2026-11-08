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
- zk2c on RF (#989's zk2c runs), per window, direction and element, a day:
  the bare runs decoded, the lean stack derived from them, the lean runs'
  counters, and the model's zk2c column (model.json's w2_rf_zk2c_per_direction,
  w1_73_rf_zk2c and idle_rf_zk2c).
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


ZK2C_ORDER = ("telemetry", "heartbeat", "health", "alarms", "config", "desired", "events",
              "keep-alive", "handshake", "presence", "declarations", "admin", "interests", "oam",
              "other", "undecoded", "reconnect")


def zk2c_window(a, window, model_dir, title, lean=None):
    """One zk2c window per direction and element, a day: the bare run decoded, its SDUs,
    the lean stack derived (+16 B an SDU), the lean run's counters, and the model."""
    w = win(a, window)
    if not w:
        return
    lw = win(lean, window) if lean else None
    for d in ("ab", "ba"):
        md = (model_dir or {}).get(MODEL_DIR[d], {})
        rows = w["elements_per_day"][d]
        print(f"\n#### {title}, {NAMES[d]}, a day ({w['seconds']:.0f} s)\n")
        print("| Element | Bare | Bare, SDUs | Lean, derived | Model (lean) |")
        print("|---|---:|---:|---:|---:|")
        tot = {"b": 0, "n": 0}
        for el in ZK2C_ORDER:
            r = rows.get(el)
            m = md.get(el, {}).get("zk2c_bytes")
            if not r and m is None:
                continue
            r = r or {"bytes": 0, "sdus": 0}
            tot["b"] += r["bytes"]
            tot["n"] += r["sdus"]
            print(f"| {el} | {kb(r['bytes']) if r['bytes'] else ''} | {r['sdus']:,.0f} | "
                  f"{kb(r['bytes'] + LEAN * r['sdus']) if r['bytes'] else ''} | "
                  f"{kb(m) if m is not None else '—'} |")
        mt = md.get("total", {}).get("zk2c_bytes")
        print(f"| **total** | **{kb(tot['b'])}** | {tot['n']:,.0f} | **{kb(tot['b'] + LEAN * tot['n'])}** | "
              f"{'**' + kb(mt) + '**' if mt is not None else '—'} |")
        p = w[d]["per_day"]
        print(f"\nBare, the counters: {p['sdus']:,} SDUs, {kb(p['bytes'])}, {p['airtime_s']:,} s of air "
              f"({p['airtime_pct']} %); relay {w[d]['relay_sdus']} SDUs, {w[d]['relay_dropped']} lost.")
        if lw:
            q = lw[d]["per_day"]
            print(f"Lean, the counters ({lw['seconds']:.0f} s): {q['sdus']:,} SDUs, {kb(q['bytes'])}, "
                  f"{q['airtime_s']:,} s of air ({q['airtime_pct']} %); relay {lw[d]['relay_sdus']} SDUs, "
                  f"{lw[d]['relay_dropped']} lost.")


def zk2c_tables(model):
    w1, w1l = load("zk2c-w1-bare-1"), load("zk2c-w1-lean-1")
    w2, w2l = load("zk2c-w2-bare-1"), load("zk2c-w2-lean-1")
    if not (w1 or w2):
        return
    print("\n## zk2c on RF")
    if w1:
        idle = model["idle_rf_zk2c"]["lean"]["per_direction"]["bytes"]
        zk2c_window(w1, "idle", {"a_to_b": {"keep-alive": {"zk2c_bytes": idle}},
                                 "b_to_a": {"keep-alive": {"zk2c_bytes": idle}}},
                    "The idle link at link.v2's transport", w1l)
        m = model["w1_73_rf_zk2c"]
        zk2c_window(w1, "w1", {"a_to_b": {"telemetry": {"zk2c_bytes": m["a_to_b_bytes"]}},
                               "b_to_a": {"keep-alive": {"zk2c_bytes": idle}}},
                    "W1-73 across rf0's face", w1l)
    if w2:
        zk2c_window(w2, "steady", model["w2_rf_zk2c_per_direction"], "W2 zk2c, the steady window", w2l)
        w2_day(w2, model["w2_rf_zk2c_per_direction"])
    sf7 = load("zk2c-sf7-lean-1")
    if sf7:
        m = model["w1_73_sf7_zk2c"]
        print("\n#### W1-73 across lora0's SF7 face, lean, a day\n")
        print("| Window | Direction | SDUs | Bytes | Airtime | Duty used | Keep-alives lost; "
              "links reopened | Model: airtime |")
        print("|---|---|---:|---:|---:|---:|---|---|")
        for w in sf7["windows"]:
            for d in ("ab", "ba"):
                x, p = w[d], w[d]["per_day"]
                if w["window"] == "w1" and d == "ab":
                    mod = (f"frames {m['gen_frames_airtime_s']:,} s (gen's), "
                           f"keep-alives {m['uplink_keepalive']['airtime_s']} s")
                else:
                    mod = f"{m['idle_keepalive_per_direction']['airtime_s']} s of keep-alives"
                lost = ", ".join(f"+{t}" for t in x.get("keepalives_lost_at", [])) or "none"
                # Under a face the far side opens: its InitSyns go B->A.
                opened = ", ".join(f"+{t}" for t in episodes(w["ba"].get("links_opened_at", []))) or "never"
                ev = f"lost {lost}" + (f"; reopened {opened}" if d == "ba" else "")
                print(f"| {w['window']} ({w['seconds']:.0f} s) | {NAMES[d]} | {p['sdus']:,} | {kb(p['bytes'])} | "
                      f"{p['airtime_s']:,} s | {p['airtime_s'] / 864 * 100:,.0f} % | {ev} | {mod} |")


W2_EVENTS = 15


def w2_day(a, model):
    """W2's day as the report's §2.2 makes it: the steady window's periodic elements, its
    events at W2's 15 a day (gen puts 60 an hour: the steady window's cost an occurrence),
    the acts at their daily counts and 4 reconnects, bare and lean (+16 B an SDU)."""
    steady, w2 = win(a, "steady"), a["w2"]
    span = steady["seconds"]
    occurrences = steady.get("far_samples", {}).get("events/oper_change", 0) * DAY / span
    for d in ("ab", "ba"):
        md = model[MODEL_DIR[d]]
        rows = {}
        for el, v in steady["elements_per_day"][d].items():
            if el == "events" and occurrences:
                v = {k: v[k] * W2_EVENTS / occurrences for k in ("bytes", "sdus")}
            rows[el] = dict(v)
        for kind, dirs in w2.get("acts", {}).items():
            v = dirs.get(d, {}).get("per_day")
            if v and v["bytes"]:
                r = rows.setdefault(ACT_ELEMENT[kind], {"bytes": 0, "sdus": 0})
                r["bytes"] += v["bytes"]
                r["sdus"] += v["sdus"]
        cost = w2.get("reconnect", {}).get(d, {}).get("cost")
        if cost:
            rows["reconnect"] = {k: w2["reconnects_per_day"] * cost[k] for k in ("bytes", "sdus")}
        print(f"\n#### W2 zk2c, {NAMES[d]}, a day as W2 makes it\n")
        print("| Element | Bare | Bare, SDUs | Lean, derived | Model (lean) | Lean / model |")
        print("|---|---:|---:|---:|---:|---:|")
        tb = tn = tm = 0
        for el in ZK2C_ORDER:
            r, m = rows.get(el), md.get(el, {}).get("zk2c_bytes")
            if not r and m is None:
                continue
            r = r or {"bytes": 0, "sdus": 0}
            lean = r["bytes"] + LEAN * r["sdus"]
            tb, tn, tm = tb + r["bytes"], tn + r["sdus"], tm + (m or 0)
            ratio = f"{lean / m:.2f}" if m and lean else ""
            print(f"| {el} | {kb(r['bytes']) if r['bytes'] else 'not measured'} | {r['sdus']:,.0f} | "
                  f"{kb(lean) if lean else ''} | {kb(m) if m is not None else '—'} | {ratio} |")
        lean = tb + LEAN * tn
        print(f"| **total** | **{kb(tb)}** | {tn:,.0f} | **{kb(lean)}** | **{kb(tm)}** | {lean / tm:.2f} |")


def main():
    model = json.load(open(os.path.join(RES, "model.json")))
    bare, lean = load("w2-rf-bare-3"), load("w2-rf-lean-1")
    if bare:
        print("## W2 on RF")
        w2_tables(bare, lean, model["w2_rf_per_direction"])
    print("\n## LoRa")
    lora_table(model)
    zk2c = os.path.join(RES, "model-zk2c.json")
    if os.path.exists(zk2c):
        zk2c_tables(json.load(open(zk2c)))


if __name__ == "__main__":
    main()

#!/usr/bin/env python3
"""Per-direction figures for one #989 run: the drivers' counters and the relay's capture.

Usage: analyze.py <run-dir> [--json out.json] [--link-failed-at-s S --steady-end-s E]

Reads, from the run directory measure.sh or measure-w2.sh wrote:
- marks.json: each window's bounds (wall clock, seconds);
- configs/params.json: the medium and stack (absent for phase 1's runs: RF);
- counters-a.jsonl, counters-b.jsonl: each driver's management readings, stamped;
- air.jsonl: every datagram the relay carried, per direction;
- far.jsonl: what the far side's subscribers received;
- acts.jsonl (W2): when each act was made.

A window's counter delta is the reading last published at or before its end,
less the one last published at or before its start (published every second).
The relay's figures count exactly the datagrams inside the window.

W1's SDUs other than keep-alives are split into data (from each of gen's
ticks to its burst's end) and the rest; links_opened counts the InitSyns A
sent, each a transport opened again, and keepalives_lost the keep-alives the
relay dropped (on LoRa, with the times of both).

A->B is driver A's transmit side (the owner's router toward the reader's), and
B->A driver B's. Airtime is each medium's, per SDU, as fake-medium's
SimMedium charges a frame with no ARQ (sim.rs `airtime`): on RF 8 bits a byte
at 2,400 bit/s; on LoRa the Semtech formula (LoRa::airtime: 125 kHz, CR 4/5,
an 8-symbol preamble, explicit header, CRC on, low-data-rate optimisation at
SF11-12). On LoRa each direction's duty use is its airtime an hour against
EU868's 1 %, 36 s, which nothing here enforces.

W2 (marks with steady_start): the steady window as above, then the acts and
the reconnect (w2's docstring says how each is costed). When the link failed
in the steady window and never came back, --link-failed-at-s says when (s
into the window) and --steady-end-s where the window is cut, before the
failure: the acts and the reconnect then ran on a link that carried nothing,
and are recorded as not measured, with the reason.
"""
import json
import math
import os
import sys
from collections import Counter

sys.dont_write_bytecode = True
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import zdecode  # noqa: E402

DAY = 86_400
DIRS = (("ab", "a", "b"), ("ba", "b", "a"))
# Daily counts of W2's rare acts (report §2.2): an alarm raise and its resolve
# are each a transition.
W2_DAILY = {"health": 4, "event": 15, "alarm_raise": 5, "alarm_resolve": 5, "config": 5}
W2_RECONNECTS = 4
# What each act moves, and how long it takes: a config change is made from
# the host by `zenctl config set`, a session that reads presence, the
# descriptor and the view before its call, and a 1.4 KB view crosses in 5 s.
ACT_ELEMENTS = {
    "health": ("health",), "event": ("events",), "alarm_raise": ("alarms",),
    "alarm_resolve": ("alarms",),
    "config": ("config", "presence", "declarations", "interests", "admin", "oam", "other"),
}
ACT_SLOT_S = 25.0
SESSION = ("presence", "declarations", "interests", "admin", "oam", "other")


def act_messages(a, msgs):
    """The messages an act made, among those of the 25 s after it.

    health, an event, an alarm raised: its element's first put from the
    device. An alarm resolved: its first put and its first delete. A config
    change: everything zenctl's session made (presence, declarations,
    interests, admin), config's requests and replies, then the first view and
    status put after the call's reply and any config event: the change's own
    re-put, since a change puts both at once.
    """
    if a["act"] in ("health", "event", "alarm_raise", "alarm_resolve"):
        el = {"health": "health", "event": "events"}.get(a["act"], "alarms")
        puts = [x for x in msgs if x["dir"] == "ab" and x["element"] == el]
        first_put = [x for x in puts if x["kind"] == "push-put"][:1]
        if a["act"] == "alarm_resolve":
            return first_put + [x for x in puts if x["kind"] == "push-del"][:1]
        return first_put
    done = a.get("done", a["t"]) + 10
    out = [x for x in msgs if x["t"] < done and (x["element"] in SESSION or (
        x["element"] == "config" and x["kind"].startswith(("request", "response"))))]
    replies = [x["t"] for x in out if x["element"] == "config" and x["kind"].startswith("response")]
    after = max(replies) if replies else a["t"]
    for suffix in ("/view", "/status"):
        out += [x for x in msgs if x["element"] == "config" and x["kind"] == "push-put"
                and x["t"] >= after and (x["key"] or "").endswith(suffix)][:1]
    out += [x for x in msgs if x["element"] == "config" and "/events/" in (x["key"] or "")]
    return out
RECONNECT_ELEMENTS = ("handshake", "declarations", "presence", "admin", "interests", "oam",
                      "desired", "other")


def load_jsonl(path):
    try:
        with open(path) as f:
            return [json.loads(line) for line in f if line.strip()]
    except FileNotFoundError:
        return []


def counters(rows):
    """{suffix: [(t, value), ...]} for numeric readings."""
    out = {}
    for r in rows:
        k, v = r.get("key"), r.get("value")
        if not k or not isinstance(v, (int, float)) or isinstance(v, bool):
            continue
        suffix = k.split("/device/", 1)[1].split("/", 1)[1]
        out.setdefault(suffix, []).append((r["t"], v))
    return out


def at(series, t):
    best = None
    for ts, v in series:
        if ts <= t:
            best = v
        else:
            break
    return best


def delta(series, t0, t1):
    a, b = at(series, t0), at(series, t1)
    return None if a is None or b is None else b - a


def lora(params):
    """A LoRa medium (sf7, sf9, sf12, sf7ka4); RF's are rf and rf-zk2c (#989's zk2c runs)."""
    return not params.get("medium", "rf").startswith("rf")


def airtime_fn(params):
    medium = params.get("medium", "rf")
    if not lora(params):
        rate = params.get("rate", 2400)
        return lambda n: n * 8 / rate
    sf = int("".join(c for c in medium[2:].split("ka")[0].split("-")[0] if c.isdigit()))
    t_sym = 2**sf / 125_000
    de = 1 if t_sym > 0.016 else 0

    def toa(n):
        steps = math.ceil((8 * n - 4 * sf + 28 + 16) / (4 * (sf - 2 * de)))
        return (8 + 4.25) * t_sym + (8 + max(steps, 0) * 5) * t_sym
    return toa


def keepalive_size(params):
    lean = params.get("stack") == "lean"
    frag = "fragment" in params
    # A keep-alive is 1 byte. compress stores it (+1), aead adds 13, the
    # overhead 2; a fragment packet of one tile adds rule, 0xFF and the RCS (6).
    n = 1 + (14 if lean else 0) + (6 if frag else 0) + (2 if lean else 0)
    return n


def classify(params, r):
    ka = keepalive_size(params)
    if params.get("stack") != "lean" and "fragment" not in params:
        return "keep-alive" if r["n"] == 1 and r["hex"] == "04" else "other"
    return "keep-alive" if r["n"] == ka else "other"


def initsyn_size(params):
    """An InitSyn (27 bytes, the first of a link's handshake) under the run's layers. Under a
    face (#989's zk2c runs) it carries usrpwd's extension, 30 bytes, and the far side opens."""
    n = (30 if params.get("face") else 27) + (14 if params.get("stack") == "lean" else 0)
    if "fragment" in params:
        n += 6
    return n + (2 if params.get("stack") == "lean" else 0)


def split_w1(run, d, rows):
    """W1's SDUs that are not keep-alives, as data and the rest (re-opened
    links, their declarations). Data is what A sent in the 2 s (5 s in
    tiles) before a reading reached the far side, or a period of 60 s from
    one (a reading lost on the air reaches nothing)."""
    span = 5 if "fragment" in run["params"] else 2
    if "_ticks" not in run:
        far = sorted(r["t"] for r in run["far"] if r.get("key"))
        run["_ticks"] = sorted({round(f + 60 * k, 3) for f in far for k in (-1, 0, 1)})
    data, rest = [], []
    for r in rows:
        hit = d == "ab" and any(f - span <= r["t"] <= f + 0.5 for f in run["_ticks"])
        (data if hit else rest).append(r)
    return data, rest


def in_window(run, d, t0, t1):
    return [r for r in run["air"] if r.get("dir") == d and t0 <= r["t"] < t1]


def window(run, name, t0, t1):
    span = t1 - t0
    scale = DAY / span
    params = run["params"]
    air_s = run["airtime"]
    res = {"window": name, "seconds": round(span, 3)}
    for d, tx, rx in DIRS:
        ctx, crx = run["counters"][tx], run["counters"][rx]
        sdus = delta(ctx.get("sdu/tx_sdus_total", []), t0, t1)
        byts = delta(ctx.get("sdu/tx_bytes_total", []), t0, t1)
        rsdus = delta(crx.get("sdu/rx_sdus_total", []), t0, t1)
        rbyts = delta(crx.get("sdu/rx_bytes_total", []), t0, t1)
        air = in_window(run, d, t0, t1)
        kinds, kbytes, kair, sizes = Counter(), Counter(), Counter(), Counter()
        for r in air:
            k = classify(params, r)
            kinds[k] += 1
            kbytes[k] += r["n"]
            kair[k] += air_s(r["n"])
            if k == "other":
                sizes[r["n"]] += 1
        layer = {}
        for key in ("layer/compress/compressed_total", "layer/compress/stored_total",
                    "layer/compress/bytes_saved_total", "layer/fragment/fragments_total",
                    "layer/aead/auth_failures_total", "layer/aead/replays_total"):
            v = delta(ctx.get(key, []), t0, t1)
            if v is not None:
                layer[key.split("/", 1)[1]] = v
        airtime = sum(air_s(r["n"]) for r in air)
        opener = "ba" if params.get("face") else "ab"
        reopened = sum(1 for r in air if r["n"] == initsyn_size(params)
                       and classify(params, r) == "other") if d == opener else None
        if name == "w1":
            data, rest = split_w1(run, d, [r for r in air if classify(params, r) == "other"])
            kinds.pop("other", None), kbytes.pop("other", None), kair.pop("other", None)
            for label, rows in (("data", data), ("other", rest)):
                if rows:
                    kinds[label] = len(rows)
                    kbytes[label] = sum(r["n"] for r in rows)
                    kair[label] = sum(air_s(r["n"]) for r in rows)
        rel = {
            "tx_sdus": sdus, "tx_bytes": byts, "rx_sdus_far": rsdus, "rx_bytes_far": rbyts,
            "relay_sdus": len(air), "relay_bytes": sum(r["n"] for r in air),
            "relay_dropped": sum(1 for r in air if r["drop"]),
            "kinds": dict(kinds), "kind_bytes": dict(kbytes),
            "kind_airtime_s": {k: round(v, 3) for k, v in kair.items()},
            "other_sizes": dict(sorted(sizes.items())),
            "tx_layers": layer,
            "airtime_s": round(airtime, 3),
        }
        if reopened is not None:
            rel["links_opened"] = reopened
        rel["keepalives_lost"] = sum(1 for r in air if r["drop"] and classify(params, r) == "keep-alive")
        if lora(params):
            # On LoRa, when each keep-alive was lost and each link opened again
            # (seconds into the window): a close follows a lost keep-alive.
            rel["keepalives_lost_at"] = [round(r["t"] - t0, 1) for r in air
                                         if r["drop"] and classify(params, r) == "keep-alive"]
            if d == opener:
                rel["links_opened_at"] = [round(r["t"] - t0, 1) for r in air
                                          if r["n"] == initsyn_size(params)
                                          and classify(params, r) == "other"]
        if sdus is not None and byts is not None:
            rel["per_day"] = {
                "sdus": round(sdus * scale), "bytes": round(byts * scale),
                "airtime_s": round(airtime * scale, 1),
                "airtime_pct": round(100 * airtime * scale / DAY, 3),
                "keepalives": round(kinds.get("keep-alive", 0) * scale),
                "by_kind": {k: {"sdus": round(kinds[k] * scale), "bytes": round(kbytes[k] * scale),
                                "airtime_s": round(kair[k] * scale, 1)} for k in kinds},
            }
            if lora(params):
                rel["duty_s_per_hour"] = round(airtime * 3600 / span, 2)
                rel["duty_pct_of_eu868"] = round(100 * airtime * 3600 / span / 36, 1)
        res[d] = rel
    def resource(key):
        # An event's last chunk is its occurrence (core §2.6): count by resource.
        rest = key.split("/", 4)[-1]
        return rest.rsplit("/", 1)[0] if rest.startswith("events/") or "/events/" in rest else rest
    far = Counter(resource(r["key"]) for r in run["far"] if r.get("key") and t0 <= r["t"] < t1)
    if far:
        res["far_samples"] = dict(sorted(far.items()))
    # zk2c's (#989): each frame's payload size as the far reader got it, and
    # what its records held (every field known, or every one absent).
    sizes = {}
    for r in run["far"]:
        if r.get("key") and t0 <= r["t"] < t1 and r.get("n") is not None:
            sizes.setdefault(resource(r["key"]), Counter())[r["n"]] += 1
    if sizes and any("/frames/" in k or "/events/" in k for k in sizes):
        res["far_payload_bytes"] = {k: dict(sorted(v.items())) for k, v in sorted(sizes.items())}
    if params.get("stack") != "lean" and "fragment" not in params:
        # A bare run decodes: each element's share of the window, per day.
        got = zdecode.decode(run["air"], t0, t1)
        res["elements_per_day"] = {
            d: {el: {"bytes": round(v["bytes"] * scale), "sdus": round(v["sdus"] * scale, 1)}
                for el, v in got[d].items()} for d in ("ab", "ba")}
        res["decode_errors"] = got["errors"]
        # Each transport opened in the window, by its InitSyn, decoded (a
        # face's usrpwd and 16-bit resolution change the InitSyn's size).
        res["opens_at"] = {d: [round(t - t0, 1) for t in got["opens"][d]] for d in ("ab", "ba")}
    return res


def totals(rows, air_s):
    return {"sdus": len(rows), "bytes": sum(r["n"] for r in rows),
            "airtime_s": sum(air_s(r["n"]) for r in rows)}


def w2(run, steady):
    """Acts and the reconnect, against the steady window.

    Bare runs decode (zdecode): an act's cost is what its elements carried in
    the act's slot, less what the steady window carried of the same elements
    in as long a span. The reconnect's cost is what it carried from the cut to
    the end of the elements a reconnect makes (handshake, declarations,
    presence, admin, interests, oam, desired's re-seed), less the steady
    window's rate of them; what the periodic elements lost while the link was
    down is stated beside it. Every run also gets the net excess: everything
    from the cut to the end, less the steady window's rate over that span.
    """
    m, params, air_s = run["marks"], run["params"], run["airtime"]
    t0, t1 = m["steady_start"], m["steady_end"]
    span0 = t1 - t0
    decodable = params.get("stack") != "lean" and "fragment" not in params
    out = {"daily_counts": W2_DAILY, "reconnects_per_day": W2_RECONNECTS, "decoded": decodable}
    rate = {}
    if decodable:
        st = zdecode.decode(run["air"], t0, t1)
        rate = {d: {el: (v["bytes"] / span0, v["sdus"] / span0) for el, v in st[d].items()}
                for d in ("ab", "ba")}
    acts = [a for a in run["acts"] if "act" in a]
    failed = run.get("link_failed_at_s")
    if failed is not None:
        out["not_measured"] = (
            f"the link failed {failed} s into the steady window and never came back: the acts, "
            "the reconnect and the config changes ran on a link that carried nothing")
        acts, m = [], {k: v for k, v in m.items() if k not in ("cut", "end")}
    if decodable:
        slots, per = [], {}
        for a in acts:
            msgs = []
            zdecode.decode(run["air"], a["t"], a["t"] + ACT_SLOT_S, listing=msgs)
            mine = act_messages(a, msgs)
            row = {"act": a["act"], "t": a["t"]}
            for d in ("ab", "ba"):
                ms = [x for x in mine if x["dir"] == d]
                row[d] = {"bytes": round(sum(x["bytes"] for x in ms), 1),
                          "sdus": round(sum(x["sdus"] for x in ms), 2),
                          "messages": [f'{x["element"]}:{x["kind"]}:{(x["key"] or "")[-40:]}' for x in ms]}
                per.setdefault(a["act"], {}).setdefault(d, []).append(row[d])
            slots.append(row)
        out["act_slots"] = slots
        out["acts"] = {}
        for kind, dirs in per.items():
            out["acts"][kind] = {}
            for d, rows in dirs.items():
                mb = sum(r["bytes"] for r in rows) / len(rows)
                mn = sum(r["sdus"] for r in rows) / len(rows)
                daily = W2_DAILY[kind]
                out["acts"][kind][d] = {"per_act": {"bytes": round(mb, 1), "sdus": round(mn, 2)},
                                        "per_day": {"bytes": round(mb * daily), "sdus": round(mn * daily, 1)}}
    if "cut" in m and "end" in m:
        rec_end = m.get("reconnected", m["end"])
        span = rec_end - m["cut"]
        rec = {}
        for d, _, _ in DIRS:
            r = {k: steady[d]["relay_" + k] / steady["seconds"] for k in ("sdus", "bytes")}
            r["airtime_s"] = steady[d]["airtime_s"] / steady["seconds"]
            cut = totals(in_window(run, d, m["cut"], m["restore"]), air_s)
            after = totals(in_window(run, d, m["restore"], rec_end), air_s)
            gross = {k: cut[k] + after[k] for k in cut}
            rec[d] = {"cut_s": round(m["restore"] - m["cut"], 3), "after_s": round(rec_end - m["restore"], 3),
                      "during_cut": {k: round(v, 3) for k, v in cut.items()},
                      "after_restore": {k: round(v, 3) for k, v in after.items()},
                      "net_excess": {k: round(gross[k] - r[k] * span, 1) for k in gross}}
        if decodable:
            got = zdecode.decode(run["air"], m["cut"], rec_end)
            for d in ("ab", "ba"):
                made, lost = {}, {}
                for el, v in got[d].items():
                    rb, rn = rate[d].get(el, (0, 0))
                    ex = (round(v["bytes"] - rb * span, 1), round(v["sdus"] - rn * span, 2))
                    (made if el in RECONNECT_ELEMENTS else lost)[el] = {"bytes": ex[0], "sdus": ex[1]}
                for el, (rb, rn) in rate[d].items():
                    if el not in got[d] and el not in RECONNECT_ELEMENTS:
                        lost[el] = {"bytes": round(-rb * span, 1), "sdus": round(-rn * span, 2)}
                rec[d]["made"] = made
                rec[d]["periodic_change"] = lost
                rec[d]["cost"] = {"bytes": round(sum(v["bytes"] for v in made.values()), 1),
                                  "sdus": round(sum(v["sdus"] for v in made.values()), 2)}
        out["reconnect"] = rec
    day = {}
    for d, _, _ in DIRS:
        pd = steady[d]["per_day"]
        day[d] = {"steady": {k: pd[k] for k in ("sdus", "bytes")}}
        if "acts" in out:
            day[d]["acts"] = {k: round(sum(v[d]["per_day"][k] for v in out["acts"].values()), 1)
                              for k in ("sdus", "bytes")}
        rc = out.get("reconnect", {}).get(d, {})
        if "cost" in rc:
            day[d]["reconnects"] = {k: round(W2_RECONNECTS * rc["cost"][k], 1) for k in ("sdus", "bytes")}
        day[d]["total"] = {k: round(sum(v[k] for v in day[d].values()), 1) for k in ("sdus", "bytes")}
    out["per_day"] = day
    return out


def main():
    path = sys.argv[1]
    marks = json.load(open(f"{path}/marks.json"))
    pfile = f"{path}/configs/params.json"
    params = json.load(open(pfile)) if os.path.exists(pfile) else {
        "medium": "rf", "rate": 2400, "stack": marks.get("stack", "bare")}
    run = {
        "marks": marks, "params": params, "airtime": airtime_fn(params),
        "counters": {x: counters(load_jsonl(f"{path}/counters-{x}.jsonl")) for x in "ab"},
        "air": load_jsonl(f"{path}/air.jsonl"),
        "far": load_jsonl(f"{path}/far.jsonl"),
        "acts": load_jsonl(f"{path}/acts.jsonl"),
    }
    out = {"run": os.path.basename(os.path.normpath(path)), "medium": params.get("medium"),
           "stack": params.get("stack"), "loss": params.get("loss", marks.get("loss")),
           "windows": []}

    def opt(name):
        return float(sys.argv[sys.argv.index(name) + 1]) if name in sys.argv else None
    if opt("--link-failed-at-s") is not None:
        run["link_failed_at_s"] = out["link_failed_at_s"] = opt("--link-failed-at-s")
        cut = opt("--steady-end-s")
        assert cut is not None and cut < run["link_failed_at_s"], "--steady-end-s before the failure"
        out["steady_window_s"] = {"made": round(marks["steady_end"] - marks["steady_start"], 3), "used": cut}
        marks = dict(marks, steady_end=marks["steady_start"] + cut)
        run["marks"] = marks
    for name, a, b in (("idle", "idle_start", "idle_end"), ("w1", "w1_start", "w1_end"),
                       ("steady", "steady_start", "steady_end")):
        if a in marks and b in marks:
            out["windows"].append(window(run, name, marks[a], marks[b]))
    if "steady_start" in marks:
        out["w2"] = w2(run, out["windows"][-1])
    text = json.dumps(out, indent=1)
    if "--json" in sys.argv:
        with open(sys.argv[sys.argv.index("--json") + 1], "w") as f:
            f.write(text + "\n")
    print(text)


if __name__ == "__main__":
    main()

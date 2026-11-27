"""The runner of ``spec/profiles/alarms/conformance/v2/`` (alarms.v2 text
0.1, #609 round 124), through :mod:`zk2py.alarms_v2`. Each part of that
module was committed before its file here was opened. What this module holds
of a file is its format, read after."""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any, Callable

from . import alarms_v2 as a2
from . import jcs

Result = tuple[str, bool, str]


def _load(path: Path) -> Any:
    return json.loads(path.read_text(encoding="utf-8"))


def _check(case: str, got: Any, want: Any) -> Result:
    ok = got == want
    return case, ok, "" if ok else f"got {json.dumps(got, default=str)}, want {json.dumps(want, default=str)}"


# -- schema.json (§3) ---------------------------------------------------------------------

def schema(path: Path) -> list[Result]:
    """Each schema file's id, the published ids, the types' tags, and each
    case's document -> its id and whether the profile publishes it."""
    data = _load(path)
    repo = path.parents[5]
    here = path.parents[2]
    out = [
        _check("v2 schema: each file's id is its current revision, and §3's",
               [[f["file"], f["kind"], a2.artifact_id(jcs.loads((here / f["file"]).read_bytes()))] for f in data["files"]],
               [["alarms.v2.json", "packed", a2.SCHEMA_ID],
                ["schemas/alarms.v2.json", "jsonschema", a2.WORDS_SCHEMA_ID]]),
        _check("v2 schema: the current ids are the files'", [f["current"] for f in data["files"]],
               [a2.SCHEMA_ID, a2.WORDS_SCHEMA_ID]),
        _check("v2 schema: the published ids are §3's two", [x["id"] for x in data["published"]], list(a2.PUBLISHED_IDS)),
        _check("v2 schema: the types, Alarm's tag §3's 11 by core §7.6's layout form",
               [[t["name"], t["kind"], t.get("tag")] for t in data["types"]],
               [[a2.ALARM, "packed", a2.record().tag], [a2.WORDS, "jsonschema", None]]),
    ]
    for i, c in enumerate(data["cases"]):
        doc = jcs.loads((repo / c["path"]).read_bytes()) if "path" in c else c["document"]
        got = a2.artifact_id(doc)
        out.append(_check(f"v2 schema #{i}: {c['note']}", {"id": got, "published": a2.is_published(got)}, c["expect"]))
    return out


# -- records.json (§2.4, §3) ----------------------------------------------------------------

def _stamp(text: str) -> tuple[int, int, int]:
    """``<run>/<seq>``: "the run in 4 lowercase hexadecimal digits for a
    16-bit run and 8 for a 32-bit run, the seq in decimal"."""
    run, seq = text.split("/")
    return {4: 16, 8: 32}[len(run)], int(run, 16), int(seq)


def _write(value: Any, epoch: int, stamp: str | None) -> dict[str, Any]:
    """A write's expectation: its bits, its pad, its bytes, and its put's
    payload where a stamp is given; or the refusal's token."""
    try:
        data = a2.write(value, epoch)
    except a2.Refused as e:
        return {"refused": e.token}
    b = a2.record().write_bits(value, epoch)
    out: dict[str, Any] = {"bits": b, "pad": -len(b) % 8, "hex": data.hex()}
    if stamp is not None:
        width, run, seq = _stamp(stamp)
        out["payload"] = a2.payload(value, run, seq, width, epoch).hex()
    return out


def records(path: Path) -> list[Result]:
    data = _load(path)
    out = [_check("v2 records: the file's schema is the profile's packed file", data["schema"], a2.schema())]
    rec = a2.record()
    lo, hi = rec.bit_bounds()
    for lay in data["layouts"]:
        out.append(_check(f"v2 records, layout {lay['type']}: form, sha256, tag and bounds",
                          {"form": rec.layout_form(), "sha256": rec.layout_sha256(), "tag": rec.tag,
                           "min_bits": lo, "max_bits": hi}, lay["expect"]))
    for i, c in enumerate(data["writes"]):
        got = _write(c["value"], c.get("epoch", 0), c.get("stamp"))
        out.append(_check(f"v2 records, write #{i}: {c['note']}", got, c["expect"]))
        if "refused" not in c["expect"]:
            back = a2.read(rec.tag, bytes.fromhex(c["expect"]["hex"]), c.get("epoch", 0))
            out.append(_check(f"v2 records, write #{i} read back", back.value, c.get("reads_as", c["value"])))
    for i, c in enumerate(data["reads"]):
        r = a2.read(c["tag"], bytes.fromhex(c["hex"]), c.get("epoch", 0))
        got = {"value": r.value} if r.reads else {"undecodable": r.undecodable}
        out.append(_check(f"v2 records, read #{i}: {c['note']}", got, c["expect"]))
    return out


# -- tables.json (§2.4, §4) ----------------------------------------------------------------

def _table(tables: dict[str, Any], name: Any) -> a2.Table:
    """A case's ``table``: a name of ``tables``, null for none, or
    "differ"."""
    if name is None:
        return a2.NO_TABLE
    if name == a2.DIFFER:
        return a2.Table(None, True)
    return a2.Table(tuple(tables[name]))


def _declared(d: a2.Declared) -> Any:
    return {"invalid": d.invalid} if d.invalid is not None else (None if d.rules is None else list(d.rules))


def _named(n: a2.Named) -> dict[str, Any]:
    if n.unnamed is not None:
        return {"unnamed": n.unnamed}
    if n.by_words:
        return {"by_words": n.alert_key}
    return {"named": {"rule": n.rule, "labels": n.labels, "alert_key": n.alert_key, "summary": n.summary,
                      "category": n.category, "argument": n.argument}}


def tables(path: Path) -> list[Result]:
    data = _load(path)
    tabs = data["tables"]
    out: list[Result] = []
    for i, c in enumerate(data["declarations"]):
        got = a2.contract_table(c["uses"], list(c["resources"].values()))
        out.append(_check(f"v2 tables, declaration #{i}: {c['note']}", _declared(got), c["expect"]))
    for i, c in enumerate(data["instances"]):
        held = {fp: a2.Declared(None if t is None else tuple(t)) for fp, t in c["holds"].items()}
        got = a2.instance_table(c["names"], held)
        want = c["expect"]
        out.append(_check(f"v2 tables, instance #{i}: {c['note']}",
                          a2.DIFFER if got.differ else (None if got.rules is None else list(got.rules)), want))
    for i, c in enumerate(data["writes"]):
        w = a2.write_by(_table(tabs, c["table"]), c["rule"], c["labels"], c["argument"])
        out.append(_check(f"v2 tables, write #{i}: {c['note']}", w.fields(), c["expect"]))
    for i, c in enumerate(data["reads"]):
        n = a2.name_record(_table(tabs, c["table"]), c["record"])
        out.append(_check(f"v2 tables, read #{i}: {c['note']}", _named(n), c["expect"]))
    for i, c in enumerate(data["growth"]):
        out.append(_check(f"v2 tables, growth #{i}: {c['note']}", a2.growth(tabs[c["old"]], tabs[c["new"]]),
                          c["expect"]))
    return out


# -- keys.json (§2.2, §2.12, §4) -----------------------------------------------------------

def keys(path: Path) -> list[Result]:
    """``table`` names under ``tables.json``'s T1, read beside this file."""
    data = _load(path)
    t1 = a2.Table(tuple(_load(path.parent / "tables.json")["tables"]["T1"]))
    out: list[Result] = []
    for i, c in enumerate(data["cases"]):
        op = c["op"]
        if op == "derive":
            from . import alarms as v1

            k = a2.alert_key(c["rule"], c["labels"])
            got: Any = {"key": k["alert_key"], "integer": k["key"], "input": v1.derive(c["rule"], c["labels"]).input}
        elif op == "table":
            got = a2.name_record(t1, {"argument": a2.NOT_AVAILABLE, **c["record"]}).alert_key
        elif op in ("slot", "words"):
            got = a2.slot_of(c["key"], prefix=a2.ALARMS if op == "slot" else a2.WORDS_PREFIX)
        elif op == "collection":
            sel = a2.collection_of(c["system"], c["service"])
            # core §4.2: "A collection's snapshot key is its resource's key up
            # to the" template's first parameter, its last '/' excluded.
            got = None if sel is None else {"selector": sel, "snapshot_key": sel.removesuffix("/**")}
        else:
            got = f"an op the runner does not read: {op}"
        out.append(_check(f"v2 keys #{i} ({op}): {c['note']}", got, c["expect"]))
    return out


# -- slots.json (§2.2, §2.6, §2.8, §2.9) ---------------------------------------------------

_ACTION = {a2.FREE_RESOLVED: "free", a2.FREE_UNREADABLE: "free", a2.FREE_TWICE: "free", a2.PAST_BOUND: "left",
           a2.PUT_RESOLVED: "resolved", a2.ADOPT: "adopted"}


def _holds(slots: a2.Slots) -> dict[str, Any]:
    return {str(s): {"id": h.identity, "state": a2.FIRING if h.firing else a2.RESOLVED}
            for s, h in sorted(slots.held.items())}


def slots(path: Path) -> list[Result]:
    data = _load(path)
    out: list[Result] = []
    for i, c in enumerate(data["cases"]):
        sl = a2.Slots(c["bound"])
        got_e, want_e = [], []
        for e in c["events"]:
            if "raise" in e:
                ident = e["raise"]
                before = sl.slot_of(ident)
                firing_before = before is not None and sl.held[before].firing
                r = sl.raise_(ident)
                if r == a2.REFUSED:
                    g: Any = {"refused": "bound"}
                elif firing_before:
                    g = {"none": "firing"}
                else:
                    g = {"put": r, "state": "firing"}
            elif "resolve" in e:
                r = sl.resolve(e["resolve"])
                g = {"none": "not_firing"} if r is None else {"put": r, "state": "resolved"}
            else:
                rec, bound = e["restart"]["record"], e["restart"]["bound"]
                found = [a2.Found(int(k), v["id"], v["state"] == a2.FIRING, v["reads"], v["derives"], v["evaluated"])
                         for k, v in (rec or {}).items()]
                g = {str(k): _ACTION[v] for k, v in sorted(a2.adoption(found, bound).items())}
                sl = a2.restarted(found, bound)
            got_e.append([g, _holds(sl)] if "holds" in e else [g])
            want_e.append([e["expect"], e["holds"]] if "holds" in e else [e["expect"]])
        out.append(_check(f"v2 slots #{i}: {c['note']}", got_e, want_e))
    for i, v in enumerate(data["views"]):
        # core §3.3: "So "00a1" and "000000a1" are two runs": a run is its
        # width and its value, held here as its hex digits.
        run = v["present_run"]
        by_key: dict[str, list[a2.SlotSeen]] = {}
        for h in v["held"]:
            if h["alert_key"] is not None:
                by_key.setdefault(h["alert_key"], []).append(a2.SlotSeen(h["slot"], h["run"], h["seq"]))
        got = {"takes": {k: a2.reader_slot(xs, run) for k, xs in by_key.items()}, "double": a2.doubles(by_key, run)}
        out.append(_check(f"v2 slots, view #{i}: {v['note']}", got, v["expect"]))
    return out


# -- words.json (§2.4, §5) -----------------------------------------------------------------

def _record_read(r: Any) -> a2.Read | None:
    """A case's ``record``: null for none, "undecodable", or the fields that
    matter, ``rule`` and ``key``; the rest of a record that reads filled in."""
    if r is None:
        return None
    if r == "undecodable":
        return a2.Read(None, "length")
    return a2.Read({"state": a2.FIRING, "severity": "warning", "argument": a2.NOT_AVAILABLE, **r})


def _words_in(w: Any) -> Any:
    """null, a decoded document, or {text}: a payload's text, as bytes."""
    if isinstance(w, dict) and set(w) == {"text"}:
        return w["text"].encode("utf-8", "surrogatepass")
    return w


def words(path: Path) -> list[Result]:
    data = _load(path)
    out: list[Result] = []
    for i, c in enumerate(data["takes"]):
        t = a2.take_words(_record_read(c["record"]), _words_in(c["words"]))
        got = {"taken": True, "how": t.how} if t.taken else {"taken": False, "why": t.reason}
        out.append(_check(f"v2 words, take #{i}: {c['note']}", got, c["expect"]))
    for i, c in enumerate(data["shows"]):
        view = a2.SlotView()
        got_s, want_s = [], []
        for st in c["steps"]:
            if "words" in st:
                view.put_words(_words_in(st["words"]))
            else:
                r = _record_read(st["record"])
                if r is not None:
                    view.put_record(r)
            if view.record is None or view.record.value is None:
                g: Any = None
            elif not a2.by_words(view.record.value):
                g = {"alert_key": None, "words": None}
            else:
                t = a2.take_words(view.record, view.words)
                g = {"alert_key": a2.key_hex(view.record.value["key"]),
                     "words": a2.read_words(view.words) if t.taken else None}
            got_s.append(g)
            want_s.append(st["expect"])
        out.append(_check(f"v2 words, shows #{i}: {c['note']}", got_s, want_s))
    return out


# -- transitions.json (§2.6, §2.16) ----------------------------------------------------------

def transitions(path: Path) -> list[Result]:
    data = _load(path)
    out: list[Result] = []
    for i, c in enumerate(data["cases"]):
        rd = a2.SlotReader()
        nt = a2.Notifier("lab", "svc")
        got_s, want_s = [], []
        for x in c["samples"]:
            sm = x["sample"]
            if sm == "delete":
                st = rd.step(a2.DELETE)
                notes = nt.put(0, a2.DELETE)
            else:
                p = sm["put"]
                rec = a2.Read(None, "length") if p["record"] == "undecodable" else a2.Read(p["record"])
                st = rd.step(rec, p["run"], p["alert_key"])
                notes = nt.put(0, rec, p["run"], p["alert_key"])
            g: dict[str, Any] = {"held": st.held, "reads": st.put}
            if "reports" in x["expect"]:
                g["reports"] = [{"report": "firing" if n.answer == "yes" else "resolved",
                                 "alert_key": None if n.unnamed else n.reference[2]} for n in notes]
            got_s.append(g)
            want_s.append(x["expect"])
        out.append(_check(f"v2 transitions #{i}: {c['note']}", got_s, want_s))
    return out


# -- confirmations.json (§2.7, §3) -----------------------------------------------------------

NS = 1_000_000_000


def _stamp_text(width: int, run: int, seq: int) -> str:
    return f"{run:0{width // 4}x}/{seq}"


def confirmations(path: Path) -> list[Result]:
    from . import zk2c

    data = _load(path)
    out: list[Result] = []
    for i, c in enumerate(data["heartbeats"]):
        o = a2.OwnerAlarms("lab", "svc")
        for v in c["view"]:
            width, run, seq = _stamp(v["stamp"])
            o.put(v["slot"], {"state": v["state"]}, width, run, seq, 0)
        payload = bytes.fromhex(c["payload"])
        got: dict[str, Any] = {}
        try:
            env = zk2c.parse(payload, "put")
        except zk2c.Refused:
            env = None
        if env is None or env.form != "heartbeat":
            got["unread"] = "refused"
        elif env.tag != c["holds_tag"]:
            got["unread"] = "revision"
        else:
            assert env.width is not None and env.run is not None and env.seq is not None
            h = o.heartbeat(env.tag, env.width, env.run, env.seq, env.record or b"", 905 * NS)
            if "refused" in h or "read" in h:
                got["unread"] = "refused" if "refused" in h or "refused" in h.get("read", {}) else "revision"
            else:
                section = "confirmed" if h["in_step"] else h["section"]
                got["read"] = {"live": h["live"], "section": section, "confirmed": sorted(h["confirmed"])}
        got["firing"] = o.firing_count()
        got["judged"] = {str(v["slot"]): o.answer(v["slot"], 905 * NS) for v in c["view"]}
        out.append(_check(f"v2 confirmations, heartbeat #{i}: {c['note']}", got, c["expect"]))
    for i, c in enumerate(data["snapshots"]):
        from . import heartbeat

        raw = bytes.fromhex(c["hex"])
        page = a2.read_page(raw, c["epoch"])
        hdr = (page["run_width"], page["run"])
        snap_ms = heartbeat.instant_of(page["time"], c["epoch"])
        entries = []
        for e in page["live"]:
            w, r = (e["run_width"], e["run"]) if "run" in e else hdr
            conf = snap_ms if e["confirmation"] == "time" else heartbeat.instant_of(e.get("conf"), c["epoch"])
            entries.append({"slot": e["slot"], "stamp": _stamp_text(w, r, e["seq"]), "confirmation": e["confirmation"],
                            "confirmed_at": conf, "record": e["record"]})
        for e in page["tombs"]:
            w, r = (e["run_width"], e["run"]) if "run" in e else hdr
            entries.append({"slot": e["slot"], "stamp": _stamp_text(w, r, e["seq"]), "tombstone": True})
        got = {"size": len(raw), "run": f"{page['run']:0{page['run_width'] // 4}x}", "asof": page["asof"],
               "coll": page["coll"], "time": snap_ms, "entries": entries}
        out.append(_check(f"v2 confirmations, snapshot #{i}: {c['note']}", got, c["expect"]))
    return out


# -- acks.json (§2.5) --------------------------------------------------------------------------

def acks(path: Path) -> list[Result]:
    data = _load(path)
    out: list[Result] = []
    for i, c in enumerate(data["writes"]):
        clock = a2.AtClock(c.get("epoch"))
        held = c.get("known")
        clock.knows("x", held)
        got_a, want_a = [], []
        for act in c["acts"]:
            name = {"raise": a2.RAISE, "escalate": a2.ESCALATE, "refresh": a2.REFRESH, "resolve": a2.RESOLVE,
                    "confirm": a2.CONFIRM, "refresh_words": a2.REFRESH_WORDS}[act["act"]]
            held = clock.act("x", name, act["clock_ms"], held)
            got_a.append(held)
            want_a.append(act["expect"])
        out.append(_check(f"v2 acks, writes #{i}: {c['note']}", got_a, want_a))
    for i, c in enumerate(data["applies"]):
        out.append(_check(f"v2 acks, applies #{i}: {c['note']}", a2.ack_applies(c["acked_at"], c["at"]), c["expect"]))
    for i, c in enumerate(data["moves"]):
        at = a2.from_v1(c["timestamp"], c.get("epoch"))
        out.append(_check(f"v2 acks, moves #{i}: {c['note']}",
                          {"at": at, "acknowledgement_applies": a2.ack_applies(c["timestamp"], at)}, c["expect"]))
    return out


FIXTURES: dict[str, Callable[[Path], list[Result]]] = {
    "schema.json": schema,
    "records.json": records,
    "tables.json": tables,
    "keys.json": keys,
    "slots.json": slots,
    "words.json": words,
    "transitions.json": transitions,
    "confirmations.json": confirmations,
    "acks.json": acks,
}


def run(directory: Path) -> list[Result]:
    out: list[Result] = []
    for path in sorted(directory.iterdir()):
        if path.name == "README.md":
            continue
        fn = FIXTURES.get(path.name)
        if fn is None:
            out.append((f"v2/{path.name}", False, "an unknown fixture file: the runner does not read it"))
            continue
        out += fn(path)
    return out

"""The runner of ``spec/profiles/health/conformance/v2/`` (health.v2 text
0.1, #609 round 120), through :mod:`zk2py.health_v2`. Each part of that
module was committed before its file here was opened; what this module holds
of a file is its format, read after."""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any, Callable

from . import health_v2 as h2
from . import jcs, packed

Result = tuple[str, bool, str]


def _load(path: Path) -> Any:
    return json.loads(path.read_text(encoding="utf-8"))


def _check(case: str, got: Any, want: Any) -> Result:
    ok = got == want
    return case, ok, "" if ok else f"got {json.dumps(got, default=str)}, want {json.dumps(want, default=str)}"


# -- schema.json (§3) ---------------------------------------------------------------------

def schema(path: Path) -> list[Result]:
    """The standard packed schema's id, its types' tags, and each case's
    document -> its id and whether the profile publishes it."""
    data = _load(path)
    repo = path.parents[5]
    out = [
        _check("v2 schema: the profile's file is the current revision",
               h2.artifact_id(jcs.loads((path.parents[2] / data["file"]).read_bytes())), data["current"]),
        _check("v2 schema: zk2py's id is the file's, §3's", h2.SCHEMA_ID, data["current"]),
        _check("v2 schema: the published ids are §3's one", [x["id"] for x in data["published"]], [h2.SCHEMA_ID]),
        _check("v2 schema: each type's tag is §3's, by core §7.6's layout form",
               [[t["name"], t["tag"], t["tag"]] for t in data["types"]],
               [[n, h2.record(n).tag, h2.TAGS[n]] for n in (h2.STATUS, h2.CHECK, h2.FAULT)]),
    ]
    for i, c in enumerate(data["cases"]):
        doc = jcs.loads((repo / c["path"]).read_bytes()) if "path" in c else c["document"]
        got = h2.artifact_id(doc)
        out.append(_check(f"v2 schema #{i}: {c['note']}", {"id": got, "published": got == h2.SCHEMA_ID}, c["expect"]))
    return out


# -- records.json (§2.3, §2.5, §2.6, §3) ----------------------------------------------------

def _stamp(text: str) -> tuple[int, int, int]:
    """``<run>/<seq>``: "the run in 4 lowercase hexadecimal digits for a
    16-bit run and 8 for a 32-bit run, the seq in decimal"."""
    run, seq = text.split("/")
    return {4: 16, 8: 32}[len(run)], int(run, 16), int(seq)


def records(path: Path) -> list[Result]:
    data = _load(path)
    out: list[Result] = []
    for c in data["layouts"]:
        rec = h2.record(c["type"])
        lo, hi = rec.bit_bounds()
        out.append(_check(f"v2 records: layout {c['type']}",
                          {"form": rec.layout_form(), "sha256": rec.layout_sha256(), "tag": rec.tag, "min_bits": lo,
                           "max_bits": hi}, c["expect"]))
        out.append(_check(f"v2 records: layout {c['type']}: §3's tag and sizes",
                          [rec.tag, (lo, hi)], [h2.TAGS[c["type"]], h2.BITS[c["type"]]]))
    for i, c in enumerate(data["writes"]):
        name = f"v2 records: write {i} {c['type']}: {c['note']}"
        rec, epoch = h2.record(c["type"]), c.get("epoch", 0)
        try:
            bits = rec.write_bits(c["value"], epoch)
            raw = h2.write(c["type"], c["value"], epoch)
            got: Any = {"bits": bits, "pad": -len(bits) % 8, "hex": raw.hex()}
            if "stamp" in c:
                width, run, seq = _stamp(c["stamp"])
                got["payload"] = h2.payload(c["type"], c["value"], run, seq, width, epoch).hex()
        except packed.Refused as e:
            got = {"refused": e.token}
        out.append(_check(name, got, c["expect"]))
        if "refused" not in got:
            back = h2.read(c["type"], rec.tag, raw, epoch)
            out.append(_check(f"{name}: reads back", back.expect(), c.get("reads_as", c["value"])))
            if "payload" in got:
                put = h2.read_put(c["type"], bytes.fromhex(got["payload"]), epoch)
                out.append(_check(f"{name}: its put reads back", put.expect(), c.get("reads_as", c["value"])))
    for i, c in enumerate(data["reads"]):
        name = f"v2 records: read {i} {c['type']} tag {c['tag']} {c['hex'] or '(empty)'}: {c['note']}"
        r = h2.read(c["type"], c["tag"], bytes.fromhex(c["hex"]), c.get("epoch", 0))
        out.append(_check(name, {"refused": r.refusal} if r.refused else {"value": r.value}, c["expect"]))
        # §2.6: a refused record is unknown, never a level read from its bits
        if r.refused:
            out.append(_check(f"{name}: read as unknown, undecodable, at no level",
                              [r.level, r.unknown], [None, "undecodable"]))
        else:
            want = r.value["level"] if r.value["level"] in h2.LEVELS else None
            out.append(_check(f"{name}: its level, unknown never ok",
                              [r.level, r.unknown], [want, None if want else "unknown_level"]))
    for i, c in enumerate(data["arguments"]):
        from fractions import Fraction

        off = None if c["offset_ms"] is None else Fraction(c["offset_ms"], 1000)
        out.append(_check(f"v2 records: argument {i}: {c['note']}", {"argument": h2.clock_ahead_argument(off)},
                          c["expect"]))
    return out


# -- tables.json (§2.10, §4) ----------------------------------------------------------------

#: the file names a kind by its table: `reasons`, `details`, `codes`
KIND = {"reasons": h2.REASON, "details": h2.DETAIL, "codes": h2.CODE}


def _declared(d: h2.Declared) -> Any:
    """"a list, null where no resource carries the key, or {invalid: reason}"."""
    return {"invalid": d.invalid} if d.invalid is not None else None if d.table is None else list(d.table)


def _own(names: Any) -> h2.Table | None:
    return None if names is None else h2.Table(tuple(names))


def tables(path: Path) -> list[Result]:
    data = _load(path)
    out = [_check("v2 tables: the profile's three tables, the name at index i the word i + 1",
                  {name: [h2.PROFILE_TABLES[k][n] for n in sorted(h2.PROFILE_TABLES[k])] for name, k in KIND.items()}
                  | {"numbers": [sorted(h2.PROFILE_TABLES[k]) == list(range(1, len(h2.PROFILE_TABLES[k]) + 1))
                                 for k in KIND.values()]},
                  dict(data["profile"]) | {"numbers": [True, True, True]})]
    for i, c in enumerate(data["declarations"]):
        ann = list(c["resources"].values())
        got = {name: _declared(h2.contract_table(k, c["uses"], ann)) for name, k in KIND.items()}
        out.append(_check(f"v2 tables: declaration {i}: {c['note']}", got, c["expect"]))
    for i, c in enumerate(data["instances"]):
        got = {}
        for name, k in KIND.items():
            held = {fp: h2.Declared(None if t[name] is None else tuple(t[name])) for fp, t in c["holds"].items()}
            t = h2.instance_table(k, c["names"], held)
            got[name] = h2.DIFFER if t.differ else None if t.names is None else list(t.names)
        out.append(_check(f"v2 tables: instance {i}: {c['note']}", got, c["expect"]))
    for i, c in enumerate(data["writes"]):
        out.append(_check(f"v2 tables: write {i}: {c['note']}",
                          h2.write_word(KIND[c["kind"]], c["word"], _own(c["own"])), c["expect"]))
    for i, c in enumerate(data["reads"]):
        w = h2.read_word(KIND[c["kind"]], c["number"], _own(c["own"]))
        got = ({"none": True} if w.shown == "none" else {"out_of_range": True} if w.shown == "out_of_range"
               else {"number": w.number} if w.shown == "number"
               else {"name": w.name, "of": "profile" if w.whose == "profile" else "service"})
        out.append(_check(f"v2 tables: read {i}: {c['note']}", got, c["expect"]))
    for i, c in enumerate(data["growth"]):
        g = h2.growth(c["old"], c["new"])
        out.append(_check(f"v2 tables: growth {i}: {c['note']}", g["change"] if g["keeps"] else "moved", c["expect"]))
    return out


# -- snapshots.json (§2.2, §2.4) ------------------------------------------------------------

def _run_text(width: int, run: int) -> str:
    return format(run, "04x" if width == 16 else "08x")


def _ms(t: Any, epoch: int) -> int | None:
    """A snapshot's time, or an entry's conf, in milliseconds since the
    Unix epoch; null for the unknown time."""
    return None if t is None else (epoch + t["s"]) * 1000 + t["ms"]


def _snapshot_expect(snap: dict[str, Any] | None, joined: h2.Joined, status: bool, epoch: int) -> Any:
    if snap is None:
        return None

    def stamp(e: dict[str, Any]) -> str:
        width, run = (e["run_width"], e["run"]) if e.get("run") is not None else (snap["run_width"], snap["run"])
        return f"{_run_text(width, run)}/{e['seq']}"

    def live(e: dict[str, Any], record: h2.Read) -> dict[str, Any]:
        conf = e["confirmation"]
        at = _ms(snap["time"], epoch) if conf == "time" else _ms(e.get("conf"), epoch) if conf == "conf" else None
        return {"stamp": stamp(e), "confirmation": conf, "confirmed_at": at, "record": record.expect()}

    out: dict[str, Any] = {"complete": snap["complete"], "run": _run_text(snap["run_width"], snap["run"]),
                           "asof": snap["asof"], "time": _ms(snap["time"], epoch)}
    if status:
        if snap["live"]:
            assert joined.status is not None
            out["entry"] = live(snap["live"][-1], joined.status)
        elif snap["tombs"]:
            out["entry"] = {"tombstone": True, "stamp": stamp(snap["tombs"][-1])}
        else:
            out["entry"] = None
        return out
    out["live"] = [{"check": h2.check_name(e["id"])} | live(e, joined.checks[h2.check_name(e["id"])])
                   for e in snap["live"]]
    out["retired"] = [{"check": h2.check_name(e["id"]), "stamp": stamp(e)} for e in snap["tombs"]]
    return out


def snapshots(path: Path) -> list[Result]:
    """A GET's replies, each a snapshot's page on its key -> the late
    joiner's reading (:func:`zk2py.health_v2.late_joiner`), whether it is
    one instant, and whether it shows a status better than a check."""
    from . import seed_snapshot

    data = _load(path)
    out: list[Result] = []
    for i, c in enumerate(data["cases"]):
        epoch = c.get("epoch", 0)
        pages: dict[bool, list[dict[str, Any]]] = {True: [], False: []}
        for r in c["replies"]:
            page = seed_snapshot.read_page_of(r["key"], bytes.fromhex(r["hex"]))
            pages[r["key"].endswith("/state/status")].append(page)
        st, ck = h2.held(pages[True]), h2.held(pages[False])
        j = h2.late_joiner(st, ck, epoch)
        better = None if not j.instant or j.status is None or j.status.level is None else j.breaks
        got = {"status": _snapshot_expect(st, j, True, epoch), "checks": _snapshot_expect(ck, j, False, epoch),
               "instant": j.instant, "status_better_than_a_check": better}
        name = f"v2 snapshots #{i}: {c['note']}"
        out.append(_check(name, got, c["expect"]))
        # §3: "the checks are collection 0 and the status collection 1"
        out.append(_check(f"{name}: the collections' indices",
                          [None if st is None else st["coll"], None if ck is None else ck["coll"]],
                          [None if st is None else h2.STATUS_COLL, None if ck is None else h2.CHECKS_COLL]))
        # the two ways a tool reports a break agree with the one instant
        out.append(_check(f"{name}: a break is a finding from one instant, and two readings otherwise",
                          j.report, None if not j.breaks else "finding" if j.instant else "two_readings"))
    return out


# -- judgements.json (§2.1, §2.4, §2.6 to §2.8, §2.11, §2.12) ---------------------------------

#: "A record is read by core §7.6 as records.json reads it, under epoch 1767225600"
JUDGEMENTS_EPOCH = 1767225600
MAJOR = {h2.V2: 2, h2.V1: 1, None: None}


def reading_of(c: dict[str, Any]) -> h2.Reading:
    """One case's reading. ``deployment`` is "the major the deployment's
    word names", given only across a face."""
    st = c.get("status")
    record = None if st is None or st.get("record") is None else h2.read(
        h2.STATUS, st["record"]["tag"], bytes.fromhex(st["record"]["hex"]), JUDGEMENTS_EPOCH)
    fresh = None if st is None else (st["freshness"]["verdict"], st["freshness"]["reason"])
    checks = None if c.get("checks") is None else {
        x["check"]: h2.read(h2.CHECK, x["tag"], bytes.fromhex(x["hex"]), JUDGEMENTS_EPOCH) for x in c["checks"]}
    face = None if c.get("face") is None else dict(c["face"])
    if c["presence"] == "across_face":
        face = dict(face or {}, major=c["deployment"])
    d = c.get("descriptor")
    return h2.Reading(c["presence"], None if d is None else d["interfaces"], face, fresh, record, checks)


def judgements(path: Path) -> list[Result]:
    data = _load(path)
    out: list[Result] = []
    for i, c in enumerate(data["cases"]):
        a = h2.judge(reading_of(c))
        got: dict[str, Any] = {"major": MAJOR[a.major]}
        # "1 … and then the case expects nothing more, health.v1's own
        # judgements.json being the evidence"
        if a.major != h2.V1:
            got |= a.expect()
        out.append(_check(f"v2 judgements #{i}: {c['note']}", got, c["expect"]))
    return out


# -- clock.json (§2.5, §5) ------------------------------------------------------------------

#: clock.json's words for what a reader heard, in this module's
HEARD = {"heartbeat": h2.INSTANT, "heartbeat_ahead": h2.AHEAD, "heartbeat_unknown": h2.TIME_UNKNOWN,
         "heartbeat_unread": h2.NOT_READ}


def clock(path: Path) -> list[Result]:
    from . import heartbeat, zk2c

    data = _load(path)
    lo = h2.standard_layout()
    hb = data["heartbeat"]
    width, run, seq = _stamp(hb["stamp"])
    out = [
        _check("v2 clock: the heartbeat's key, its one collection and its member's digest",
               [heartbeat.heartbeat_key("lab", "svc", h2.IFACE), [c.resource for c in lo.collections],
                f"{heartbeat.digest([(hb['member'], width, run, seq)]):08x}"],
               [hb["key"], [hb["collection"]], hb["digest"]]),
        _check("v2 clock: revision 2.0's heartbeat tag is §3's 1", [lo.tag, lo.fingerprint],
               [h2.HEARTBEAT_TAG, h2.FINGERPRINT]),
    ]
    for i, c in enumerate(data["heartbeats"]):
        # `holds_tag`: "the tag of the revision the reader holds"
        reader = h2.ClockReader(heartbeat.Layout(lo.interface, lo.fingerprint, c["holds_tag"], lo.collections))
        if c["last_read"] is not None:
            reader.last_read(zk2c.InBand(*_stamp(c["last_read"])))
        what = reader.heartbeat(bytes.fromhex(c["payload"]))
        got = {"unread": reader.why} if what == h2.NOT_READ else {"read": what}
        out.append(_check(f"v2 clock: heartbeat {i}: {c['note']}", got, c["expect"]))
    for i, c in enumerate(data["cases"]):
        d = c.get("descriptor")
        interfaces = None if d is None else d["interfaces"]
        major = (h2.V2 if c["presence"] == "across_face" else
                 h2.major_of(interfaces) if c["presence"] == "present" and d is not None else None)
        got: dict[str, Any] = {"major": MAJOR[major]}
        if major != h2.V1:
            answer, reason = h2.clock_question(c["presence"], interfaces, [HEARD.get(x, x) for x in c["heard"]])
            got |= {"answer": answer, "reason": reason}
        out.append(_check(f"v2 clock #{i}: {c['note']}", got, c["expect"]))
    return out


FIXTURES: dict[str, Callable[[Path], list[Result]]] = {
    "schema.json": schema,
    "records.json": records,
    "tables.json": tables,
    "snapshots.json": snapshots,
    "judgements.json": judgements,
    "clock.json": clock,
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

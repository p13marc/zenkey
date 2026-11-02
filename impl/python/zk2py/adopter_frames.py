"""The adopters' frames, written and read by zk2py (#983's last "Done when"
item; #609, round 99): ``modem.v3`` revision 3.2's nine frame resources
(``examples/zk2/zenoh-modem/modem.v3.toml``, over
``schemas/modem-frames.json``) and W2's ``gendev.v1`` 1.1 ``frames/sample``
(``docs/zk2/measurements/wire-baseline/contracts/gendev.v1.toml``, the one
file of ``docs/`` the coordinator opened to zk2py, read as contract TOML; its
schema files beside it are loaded by zk2py's contract reader, not read here).

``python -m zk2py.adopter_frames`` prints one JSON line per frame resource.

For each frame resource, with zk2py's own writer (:class:`zk2py.frame.Framer`)
from its layout, under a 32-bit run, in core §7.5's envelope:
1. **the largest frame link.v1 §2.8 names**, at text 0.16 (F-470's answer):
   ``records`` records, every optional field present, every record after
   record 0 with its offset (the records measured off their schedule), of
   which ``records`` − ⌊``records`` / ``keyframe_every``⌋ are delta records
   that include every field and the rest full (:func:`largest_frame_bytes`).
   Its put's length is checked against :func:`zk2py.link.frame_facts`'
   ``largest_bytes``. The frame of full records alone, 0.15's figure, is
   :func:`full_frame_bytes`;
2. **the largest frame zk2py's writer puts**: the frame after it, every field
   changed, so that every record is a delta record that includes every field,
   but where ``keyframe_every`` asks a full one. A delta record carries one
   inclusion bit per field more than a full record (core §7.7);
3. **each put read back**, envelope first (:func:`zk2py.zk2c.read_put`): its
   body by the layout's tag, written again byte for byte, and through
   :class:`zk2py.telemetry_v2.SeriesReader`: every series field of every
   record a sample, its value the one written (a count modulo its width),
   each frame following the one before in the run.
"""

from __future__ import annotations

import json
import sys
from pathlib import Path
from typing import Any

from . import frame, link, packed, telemetry_v2 as t2, zk2c

REPO = Path(__file__).resolve().parents[3]
CONTRACTS = {
    "modem.v3": REPO / "examples" / "zk2" / "zenoh-modem" / "modem.v3.toml",
    "gendev.v1": REPO / "docs" / "zk2" / "measurements" / "wire-baseline" / "contracts" / "gendev.v1.toml",
}
#: the run every frame is put under: 32 bits, as link.v1 §2.8's envelope
RUN = (32, 0x5EED_0001)
#: the frame's time, record 0's: a known instant
TIME = {"s": 1_800_000_000, "ms": 999}


def _value(f: packed.Field, which: int) -> Any:
    """A value of ``f``'s type with a code, the largest (``which`` 0) or
    another (1): an int's bounds, a fixed's as decimals, a count of the
    width's top or half of it, an enum's last or first value, a bool."""
    if f.type == "int":
        return f.maximum if which == 0 or f.minimum == f.maximum else f.minimum
    if f.type == "fixed":
        return packed.shortest(f.maximum if which == 0 else f.minimum)
    if f.type == "counter":
        assert f.counter_width is not None
        top = (1 << f.counter_width) - 1
        return top if which == 0 else top // 2
    if f.type == "enum":
        return f.values[-1] if which == 0 else f.values[0]
    if f.type == "bool":
        return which == 0
    raise ValueError(f"{f.name}: no value of type {f.type} chosen")


def _code(f: packed.Field, v: Any) -> int:
    return packed.encode(f, v, 0)


def frames_of(contract: Any, r: dict[str, Any]) -> dict[str, Any]:
    """One frame resource's two frames, written, put and read back (the
    module's steps 1 to 3)."""
    lo = frame.layout_of(contract, r)
    assert lo is not None
    decl = t2.contract_declarations(contract)[r["template"]]
    fr_ = frame.Framer(lo)
    d = lo.decl
    # off-schedule records, inside the span: every later record has its offset
    gap_ns = ((d.step_ms or 0) + (d.jitter_ms or 0) + 1) * 1_000_000 if d.step_ms is not None else 0
    puts, values_written = [], []
    mono = 10**12
    for which in (0, 1):
        ready: list[Any] = []
        for i in range(d.records):
            # record i alternates within the frame, so every delta record
            # includes every field
            vals = {f.name: _value(f, (which + i) % 2) for f in lo.fields}
            values_written.append(vals)
            ready += fr_.record(vals, mono, TIME)
            mono += gap_ns if i + 1 < d.records else 10**10
        if len(ready) != 1:
            ready += [x for x in [fr_.end()] if x is not None]
        for rd in ready:
            seq = sum(len(p["value"]["records"]) for p in puts)
            value = fr_.seal(rd, RUN)
            body = lo.write(value)
            puts.append({"value": value, "payload": zk2c.write("frame", RUN[0], RUN[1], seq, lo.tag, body),
                         "body": body})
    key = f"zk2/lab/node/{contract.interface}/stream/{r['template']}"
    reader = t2.SeriesReader(lo, decl) if isinstance(decl, t2.FrameDeclaration) else None
    back, follows, samples_ok = [], [], True
    i_rec = 0
    for p in puts:
        read = zk2c.read_put(key, zk2c.ENCODING, p["payload"])
        got = lo.read(read.tag, read.record or b"")
        back.append(got == p["value"] and lo.write(got) == p["body"] and read.form == "frame"
                    and read.tag == lo.tag)
        if reader is not None:
            g = reader.read_put(read)
            follows.append(g.get("follows"))
            for rec in g.get("records", []):
                written = values_written[i_rec]
                i_rec += 1
                for s in reader.decl.series:
                    f = next(x for x in lo.fields if x.name == s.field)
                    x = rec["fields"][s.field]
                    if x.get("reading") != "sample" or _code(f, x["value"]) != _code(f, written[s.field]):
                        samples_ok = False
    facts = link.frame_facts(contract, r)
    return {"interface": contract.interface, "resource": r["template"], "records": d.records,
            "keyframe_every": d.keyframe_every, "fields": len(lo.fields),
            "series": None if reader is None else len(reader.decl.series),
            "declared": None if reader is None else reader.decl.declaration,
            "largest_bytes": facts["largest_bytes"] if facts else None,
            "largest_frame_bytes": largest_frame_bytes(contract, r),
            "full_frame_bytes": full_frame_bytes(contract, r), "first_frame_bytes": len(puts[0]["payload"]),
            "first_frame_records_full": [rec["full"] for rec in puts[0]["value"]["records"]],
            "delta_frame_bytes": len(puts[-1]["payload"]),
            "delta_frame_records_full": [rec["full"] for rec in puts[-1]["value"]["records"]],
            "read_back": all(back), "follows": follows, "samples": samples_ok}


def run() -> list[dict[str, Any]]:
    from .contract import load_contract

    out = []
    for name, path in CONTRACTS.items():
        c = load_contract(path)
        if not c.valid:
            raise RuntimeError(f"{path}: {c.codes}")
        for r in c.canonical["resources"]:
            if r.get("frame") is not None:
                out.append(frames_of(c, r))
    return out


def full_frame_bytes(contract: Any, r: dict[str, Any]) -> int:
    """A frame of ``records`` full records, every optional field present,
    later records with offsets, written by the layout itself under a 32-bit
    run: link.v1 0.15 §2.8's largest frame, measured."""
    return _frame_bytes(contract, r, 0)


def largest_frame_bytes(contract: Any, r: dict[str, Any]) -> int:
    """link.v1 0.16 §2.8's largest frame, measured: as
    :func:`full_frame_bytes`, but "``records`` − ⌊``records`` /
    ``keyframe_every``⌋ such delta records" that include every field, "and
    full records for the rest". Which records are the deltas changes no
    bit's count."""
    lo = frame.layout_of(contract, r)
    assert lo is not None
    return _frame_bytes(contract, r, lo.decl.records - lo.decl.records // lo.decl.keyframe_every)


def _frame_bytes(contract: Any, r: dict[str, Any], deltas: int) -> int:
    lo = frame.layout_of(contract, r)
    assert lo is not None
    d = lo.decl
    recs = []
    for i in range(d.records):
        rec: dict[str, Any] = {"full": i >= deltas, "values": {f.name: _value(f, 0) for f in lo.fields},
                               "absent": []}
        if i >= 1:
            rec["at"] = min(i * ((d.step_ms or 0) + (d.jitter_ms or 0) + 1), (d.span_ms or 1) - 1)
        recs.append(rec)
    body = lo.write({"time": TIME, "records": recs})
    return len(zk2c.write("frame", RUN[0], RUN[1], 0, lo.tag, body))


if __name__ == "__main__":
    for row in run():
        print(json.dumps(row))
    sys.exit(0)


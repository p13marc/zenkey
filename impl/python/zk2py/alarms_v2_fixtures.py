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


FIXTURES: dict[str, Callable[[Path], list[Result]]] = {
    "schema.json": schema,
    "records.json": records,
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

"""Core 0.117's E051 against the reference (#609, round 126; #1152):
``python -m zk2py.packed_tag_live [--json <out>]``.

- **The survey.** Every packed document in ``spec/``, ``examples/zk2/``
  (its ``.history`` included) and ``zenkey/tests/contracts/``: each JSON
  object whose ``types`` maps names to records, wherever it sits in a file
  (a schema file, a bundle's artifact, a fixture's ``schema``), and each
  record a fixture states in its expectations (``config.v2``'s
  ``derive.json`` writes the derived types so). Each type's layout tag is
  zk2py's (:attr:`zk2py.packed.Record.tag`), and each at 0 is listed.
- **The linters side by side.** Every authoring file under the same roots
  (a TOML file with an ``[interface]`` table) is linted by zk2py
  (:func:`zk2py.contract.load_contract`, W107 on) and by the reference's
  ``zk2 contract lint`` (a black box, run): their codes, sorted with
  repeats, must be equal, and each file's E051 count is printed.
- **A derived type at 0.** ``config.v2``'s ``derive.json`` case 2 derives
  a ``Record`` whose tag is 0. The reference's ``zk2 config derive`` is run
  on a contract that declares that case's groups, the derived types it
  writes are put in the contract's packed file, and both linters read it:
  since core 0.117's "A derived type may be 0", zk2py loads it (the mend
  after round 126), and the reference's codes are printed.
- **A type an explicit stream names** (E044), at the tag 0: the two
  linters' codes.
- **A frame at the tag 0, live:** ``zenctl echo`` and zk2py's echo row,
  holding no declaration, beside a mutation at 0 (:func:`frame_at_zero_live`,
  a zenoh-python router on port 0).

Exit 0 when every comparison holds, 1 when any fails.
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
from collections import Counter
from pathlib import Path
from typing import Any

from . import jcs, packed
from .blackbox import BLACKBOX, workdir

REPO = Path(__file__).resolve().parents[3]
ROOTS = ("spec", "examples/zk2", "zenkey/tests/contracts")
CODE = re.compile(r"^(?:error|warning)\[([EW]\d{3})\]", re.M)

Result = tuple[str, bool, str]


def documents(obj: Any, path: str = ""):
    """Every packed document inside a JSON value: an object whose ``types``
    maps names to objects that hold a ``fields`` list."""
    if isinstance(obj, dict):
        t = obj.get("types")
        if isinstance(t, dict) and t and all(isinstance(r, dict) and isinstance(r.get("fields"), list)
                                             for r in t.values()):
            yield path or "/", obj
            return
        for k, v in obj.items():
            yield from documents(v, f"{path}/{k}")
    elif isinstance(obj, list):
        for i, v in enumerate(obj):
            yield from documents(v, f"{path}[{i}]")


def stated_records(obj: Any, path: str = ""):
    """Every record a fixture states outside a ``types`` member: an object
    of ``fields``, each a field or the marker (``derive.json``'s
    ``expect.derived``)."""
    if isinstance(obj, dict):
        f = obj.get("fields")
        if isinstance(f, list) and f and all(x == packed.MARKER or (isinstance(x, dict) and "type" in x)
                                              for x in f):
            yield path, obj
            return
        for k, v in obj.items():
            if k == "types":
                continue
            yield from stated_records(v, f"{path}/{k}")
    elif isinstance(obj, list):
        for i, v in enumerate(obj):
            yield from stated_records(v, f"{path}[{i}]")


def survey(repo: Path = REPO) -> dict[str, Any]:
    """Each packed type's tag, as zk2py computes it: ``rows`` per type of a
    document, ``stated`` per record a fixture states elsewhere."""
    rows, stated = [], []
    for root in ROOTS:
        for f in sorted((repo / root).rglob("*.json")):
            try:
                data = json.loads(f.read_text(encoding="utf-8"))
            except (ValueError, UnicodeDecodeError):
                continue
            rel = str(f.relative_to(repo))
            for at, doc in documents(data):
                big = bool(jcs.restriction_violations(doc)[1])
                lt = packed.lint(doc)
                for name in doc["types"]:
                    rec = lt.types.get(name)
                    rows.append({"file": rel, "at": at, "type": name, "tag": None if rec is None else rec.tag,
                                 "e028": big})
            for at, r in stated_records(data):
                rec = packed.lint({"types": {"T": r}}).types.get("T")
                stated.append({"file": rel, "at": at, "tag": None if rec is None else rec.tag})
    return {"rows": rows, "stated": stated}


def is_contract(path: Path) -> bool:
    import tomllib

    try:
        return isinstance(tomllib.loads(path.read_text(encoding="utf-8")).get("interface"), dict)
    except (tomllib.TOMLDecodeError, UnicodeDecodeError):
        return path.name.endswith(".toml") and "[interface]" in path.read_text(encoding="utf-8", errors="replace")


def reference_codes(path: Path) -> tuple[list[str], str]:
    p = subprocess.run([str(BLACKBOX / "zk2"), "contract", "lint", str(path)], capture_output=True, text=True,
                       timeout=60)
    return sorted(CODE.findall(p.stdout + p.stderr)), p.stdout + p.stderr


def zk2py_codes(path: Path) -> list[str]:
    from .contract import load_contract

    return sorted(load_contract(path, check_file_name=True).codes)


def linters(repo: Path = REPO) -> tuple[list[Result], dict[str, Any]]:
    out: list[Result] = []
    table: dict[str, Any] = {}
    for root in ROOTS:
        for f in sorted((repo / root).rglob("*.toml")):
            if not is_contract(f):
                continue
            rel = str(f.relative_to(repo))
            ref, _ = reference_codes(f)
            mine = zk2py_codes(f)
            table[rel] = {"zk2py": mine, "reference": ref}
            errors = ([c for c in mine if c.startswith("E")], [c for c in ref if c.startswith("E")])
            if mine != ref and errors[0] == errors[1]:
                print(f"NOTE lint {rel}: the errors agree, the warnings differ: zk2py {mine}, reference {ref}")
            out.append((f"lint {rel}: zk2py's errors {errors[0]} = zk2 contract lint's {errors[1]}",
                        errors[0] == errors[1], ""))
    return out, table


SEED = '''[interface]
name = "{name}"
major = 1
minor = 0
'''


def case2_contract(base: Path, repo: Path = REPO, derived: dict[str, Any] | None = None, *,
                   extra: dict[str, Any] | None = None, config_type: str | None = None,
                   without_profile: bool = False) -> Path:
    """``derive.json`` case 2's groups (one contract group, ``modulation``)
    in ``rfcfg.v1``'s contract, its ``set/<group>`` operations left out, as
    no group is settable, written under ``base``: the group's type and
    ``derived`` (the derived types, ``{name: record}``) in its packed file,
    with ``extra`` types beside them. ``config_type`` names another type
    for ``config``; ``without_profile`` drops ``config.v2`` from ``uses``."""
    data = json.loads((repo / "spec/profiles/config/conformance/v2/derive.json").read_text(encoding="utf-8"))
    case = data["cases"][2]
    rf = (repo / "spec/profiles/config/conformance/contracts/rfcfg.v1.toml").read_text(encoding="utf-8")
    groups = "[\n" + "".join("  { " + ", ".join(f"{k} = {json.dumps(v)}" for k, v in g.items()) + " },\n"
                              for g in case["groups"]) + "]"
    text = re.sub(r'"config.groups"\s*=\s*\[.*?\n\]', lambda _: f'"config.groups"   = {groups}', rf, flags=re.S)
    text = re.sub(r'\[resources\."set/[a-z]+"\][^\[]*', "", text)
    text = text.replace('name    = "rfcfg"', 'name    = "modcfg"').replace('"rfcfg.json"', '"modcfg.json"')
    text = text.replace('"../../config.v2.json"', '"config.v2.json"')
    if config_type is not None:
        text = text.replace('[resources.config]\nkind = "state"\ntype = "packed:Record"',
                            f'[resources.config]\nkind = "state"\ntype = "packed:{config_type}"')
    if without_profile:
        text = text.replace('uses    = ["config.v2", "freshness.v2"]', 'uses    = ["freshness.v2"]')
    (base / "config.v2.json").write_text((repo / "spec/profiles/config/config.v2.json").read_text(
        encoding="utf-8"), encoding="utf-8")
    names = [g["type"].split(":", 1)[1] for g in case["groups"]]
    types = {n: data["schema"]["types"][n] for n in names}
    (base / "modcfg.json").write_text(json.dumps({"types": {**types, **(derived or {}), **(extra or {})}}, indent=1),
                                      encoding="utf-8")
    contract = base / "modcfg.v1.toml"
    contract.write_text(text, encoding="utf-8")
    return contract


def derived_at_zero(repo: Path = REPO) -> list[Result]:
    """``derive.json`` case 2's contract (:func:`case2_contract`): the
    reference's ``zk2 config derive`` writes the derived types, they join
    the group's type in the packed file, and both linters read the contract
    that carries them. Core 0.117 and config.v2 0.6 (the mend after round
    126): "E051 does not read it", so zk2py loads it; the reference's answer
    is printed, its half to follow. Beside it, zk2py alone, three types at
    the tag 0 that no profile derives."""
    out: list[Result] = []
    with workdir("zk2py-e051-") as d:
        base = Path(d)
        contract = case2_contract(base, repo)
        p = subprocess.run([str(BLACKBOX / "zk2"), "config", "derive", str(contract)], capture_output=True,
                           text=True, timeout=60)
        out.append(("zk2 config derive on derive.json case 2's groups writes the derived types",
                    p.returncode == 0, (p.stdout + p.stderr)[:600]))
        try:
            types = json.loads(p.stdout)["types"]
        except (ValueError, KeyError, TypeError):
            return out
        data = json.loads((repo / "spec/profiles/config/conformance/v2/derive.json").read_text(encoding="utf-8"))
        rec = packed.lint({"types": {"R": types["Record"]}}).types.get("R") if "Record" in types else None
        want = data["cases"][2]["expect"]["layouts"]["Record"]
        out.append(("the reference's derived Record is derive.json case 2's, the tag 0 and its sha256, by zk2py's "
                    "reading", rec is not None and [rec.tag, rec.layout_sha256()] == [want["tag"], want["sha256"]],
                    "" if rec is None else f"{rec.tag} {rec.layout_sha256()}"))
        tags = {n: (packed.lint({"types": {n: r}}).types.get(n) or packed.Record(n, [])).tag for n, r in types.items()}
        print(f"NOTE derived: {sorted(types)}, tags {tags}")
        contract = case2_contract(base, repo, types)
        ref, text_out = reference_codes(contract)
        mine = zk2py_codes(contract)
        print(f"NOTE zk2 contract lint on case 2's contract: {ref}: {text_out.strip()[:900]}")
        out.append((f"case 2's contract loads in zk2py, its derived Record at 0 read by no E051 (zk2py {mine}; "
                    f"the reference's, its half to follow: {ref})", mine == [], ""))
        zero = json.loads((repo / "spec/conformance/contracts/schemas/packed/tag.json").read_text(
            encoding="utf-8"))["types"]["Zero"]
        for label, kw, want_codes in (
                ("the derived Record copied under another name, which no resource names", {"extra": {
                    "Copy": types["Record"]}}, ["E051"]),
                ("config's type a written one at 0 (tag.json's Zero), which is no derivation",
                 {"extra": {"Zero": zero}, "config_type": "Zero"}, ["E051"]),
                ("the same contract without config.v2 in uses", {"without_profile": True}, ["E020", "E051"])):
            mine = zk2py_codes(case2_contract(base, repo, types, **kw))
            out.append((f"case 2's contract, {label}: zk2py {mine}", mine == want_codes, f"want {want_codes}"))
    return out


def explicit_stream_at_zero(repo: Path = REPO) -> list[Result]:
    """A type at the tag 0 that only an explicit stream names: E044 at the
    reference, and E051 or not? Both linters read it; the text's "E051
    reads a type with none of E038 to E045" counts E044 in its range."""
    out: list[Result] = []
    tag = json.loads((repo / "spec/conformance/contracts/schemas/packed/tag.json").read_text(encoding="utf-8"))
    only_zero = {"types": {"Zero": tag["types"]["Zero"]}}
    with workdir("zk2py-e044-") as d:
        base = Path(d)
        (base / "schemas").mkdir()
        (base / "schemas" / "tag.json").write_text(json.dumps(only_zero, indent=1), encoding="utf-8")
        # e044-packed-place's two references: an explicit stream's type, and
        # an attachment; and the type that no resource names, beside a state
        for label, body in (("an explicit stream", '[resources.zero]\nkind = "stream"\nexplicit = true\n'
                                                   'type = "packed:Zero"\n'),
                            ("an attachment", '[resources.zero]\nkind = "stream"\ntype = { raw = "text/plain" }\n'
                                              'attachment = "packed:Zero"\n'),
                            ("no resource", '[resources.other]\nkind = "state"\ntype = { raw = "text/plain" }\n')):
            text = SEED.format(name="seed") + '\n[schemas]\npacked = ["schemas/tag.json"]\n'
            f = base / "seed.v1.toml"
            f.write_text(text + "\n" + body, encoding="utf-8")
            ref, text_out = reference_codes(f)
            mine = zk2py_codes(f)
            print(f"NOTE {label}: zk2 contract lint: {text_out.strip()[:800]}")
            out.append((f"Zero, the tag 0, named by {label}: zk2py {mine}, zk2 contract lint {ref}", mine == ref, ""))
        # Cascade 8's "a packed declaration with no finding (E038 to E045)":
        # a frame resource whose type has a marker, its attachment that type.
        frames = repo / "spec/conformance/contracts/schemas/packed/frames.json"
        (base / "schemas" / "frames.json").write_text(frames.read_text(encoding="utf-8"), encoding="utf-8")
        f = base / "seed.v1.toml"
        f.write_text(SEED.format(name="seed") + '\n[schemas]\npacked = ["schemas/frames.json"]\n\n'
                     '[resources.grown]\nkind = "stream"\ntype = "packed:Grown"\nattachment = "packed:Grown"\n'
                     'frame = { records = 1 }\n', encoding="utf-8")
        ref, text_out = reference_codes(f)
        mine = zk2py_codes(f)
        print(f"NOTE a frame resource's marked type as its attachment too: {text_out.strip()[:600]}")
        out.append((f"cascade 8: a frame resource's type with a marker, its attachment the same type: zk2py {mine}, "
                    f"zk2 contract lint {ref}; E047's marker half beside E044", mine == ref == ["E044", "E047"], ""))
    return out


def frame_at_zero_live() -> list[Result]:
    """The open question: a frame's own tag is 0 for one declaration in 16
    (``frame/layouts.json``'s ``tick-pow2``), and nothing lints it. On a
    zenoh-python router listening on port 0, a client puts a frame at CH tag
    0 and one at tag 5, then a mutation at tag 0 and one at tag 5, on a key
    no contract in hand declares. ``zenctl echo`` (a black box, no
    contracts) and zk2py's echo row read each: observer §7.6, "A frame whose
    declaration the tool does not hold is shown by its envelope ... Its body
    is never sniffed as a record (§7.3)", and §7.3's tag 0 sniffed, the
    tag 5 shown as a packed record."""
    import time

    import zenoh

    from . import adminspace, echo_tool, live, zk2c

    key = "zk2/lab/node/tick.v1/stream/tick"
    body = bytes.fromhex("0000000a00301bc080")  # bodies.json's tick-pow2 write
    puts = [("a frame at tag 0", zk2c.write("frame", 16, 0x1234, 7, 0, body)),
            ("a frame at tag 5", zk2c.write("frame", 16, 0x1234, 9, 5, body)),
            ("a mutation at tag 0", zk2c.write("mutation", 16, 0x1234, 11, 0, b'{"a":1}')),
            ("a mutation at tag 5", zk2c.write("mutation", 16, 0x1234, 12, 5, b"\x01\x02"))]
    out: list[Result] = []
    router = zenoh.open(adminspace.node_config())
    try:
        ep = adminspace.own_endpoint(router)
        if ep is None:
            return [("the router's endpoint", False, "own_endpoint read none")]
        echo = subprocess.Popen([str(BLACKBOX / "zenctl"), "echo", "-c", ep, "--format", "ndjson", "--count",
                                 str(len(puts)), key], stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        assert echo.stderr is not None
        end, echoing = time.monotonic() + 30, False
        while time.monotonic() < end and not echoing:
            line = echo.stderr.readline()
            if not line:
                break
            echoing = "echoing" in line
        session = live.open_client(ep)
        try:
            for _, payload in puts:
                session.put(key, payload, encoding=zk2c.ENCODING)
                time.sleep(0.3)
        finally:
            live.close(session)
        try:
            got, _ = echo.communicate(timeout=30)
        except subprocess.TimeoutExpired:
            echo.terminate()
            got, _ = echo.communicate(timeout=10)
    finally:
        router.close()
    rows = [json.loads(x) for x in got.splitlines() if x.startswith("{")]
    out.append(("zenctl echo was listening, and shows the four puts", echoing and len(rows) == len(puts),
                f"echoing {echoing}, {len(rows)} rows"))
    for (what, payload), row in zip(puts, rows):
        mine = echo_tool.row_of(key, False, len(payload), zk2c.ENCODING, payload)
        ours = ("envelope" if mine.get("frame") else "sniffed" if mine.get("sniffed") else
                "packed" if (mine.get("declared_type") or {}).get("packed_record") else "other")
        v = row.get("value")
        theirs = ("envelope" if isinstance(v, str) and v.startswith("<zk2c frame") else
                  "packed" if isinstance(v, str) and v.startswith("<packed record") else
                  "sniffed" if isinstance(v, (dict, list)) else "other")
        want = "envelope" if "frame" in what else ("sniffed" if "tag 0" in what else "packed")
        print(f"NOTE {what}: zenctl echo {v!r}; zk2py {ours}")
        out.append((f"{what}, its declaration not held: zenctl echo and zk2py's echo row show it {want}",
                    ours == theirs == want, f"zk2py {ours}, zenctl {theirs}"))
    return out


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--json", help="write the survey and the linters' table here")
    args = ap.parse_args(argv)
    s = survey()
    rows = s["rows"]
    clean = [r for r in rows if r["tag"] is not None]
    print(f"NOTE survey: {len(rows)} type declarations, {len(clean)} with no finding of their own, in "
          f"{len({r['file'] for r in clean})} files ({len({r['file'] for r in rows})} with any)")
    print(f"NOTE tags: {sorted(Counter(r['tag'] for r in clean).items())}")
    for r in clean:
        if r["tag"] == 0:
            print(f"NOTE tag 0: {r['file']} {r['at']} {r['type']}{' (file has E028)' if r['e028'] else ''}")
    for r in s["stated"]:
        if r["tag"] == 0:
            print(f"NOTE tag 0, a stated record: {r['file']} {r['at']}")
    print(f"NOTE stated records outside a types member: {len(s['stated'])}")
    results, table = linters()
    results += derived_at_zero()
    results += explicit_stream_at_zero()
    try:
        results += frame_at_zero_live()
    except Exception as e:  # noqa: BLE001 - a run that could not finish is a failure
        results.append(("a frame at tag 0, live: the run finished", False, f"{type(e).__name__}: {e}"))
    for name, ok, why in results:
        print(("PASS " if ok else "FAIL ") + name + (f": {why}" if why and not ok else ""))
    e051 = {f: v for f, v in table.items() if "E051" in v["zk2py"] or "E051" in v["reference"]}
    print(f"NOTE {len(table)} contract files linted; E051 in: {json.dumps(e051)}")
    if args.json:
        Path(args.json).write_text(json.dumps({"survey": s, "linters": table}, indent=1), encoding="utf-8")
    failed = sum(1 for _, ok, _ in results if not ok)
    print(f"{len(results) - failed} passed, {failed} failed")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())

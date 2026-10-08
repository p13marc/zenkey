"""The conformance runner: ``python -m zk2py.conformance <spec/conformance>``.

Runs every fixture family of ``spec/conformance/`` (its README lists them)
and prints a pass/fail count per family, then every failure with its case
name. Exit 0 when everything passes, 1 on any failure, 2 when the runner
could not run (a missing directory, no usable protoc, a broken fixture file).

Each family function returns ``(name, [(case, ok, detail)])``.
"""

from __future__ import annotations

import argparse
import json
import sys
import traceback
from pathlib import Path
from typing import Any, Callable

from . import keys, slug, templates

Result = tuple[str, bool, str]


class CannotRun(Exception):
    """The runner itself cannot proceed (exit 2)."""


def _load_json(path: Path) -> Any:
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (OSError, ValueError) as e:
        raise CannotRun(f"{path}: {e}") from e


def _check(case: str, got: Any, want: Any) -> Result:
    ok = got == want
    return case, ok, "" if ok else f"got {json.dumps(got, default=str)}, want {json.dumps(want, default=str)}"


# -- keys.json, slugs.json, templates.json (core.md §1, §2.2) ---------------

def family_keys(root: Path) -> list[Result]:
    out = []
    for c in _load_json(root / "keys.json")["cases"]:
        got = keys.parse(c["key"])
        out.append(_check(c["key"], got, c["expect"]))
        if got is not None:
            # §1.1: every accepted key builds back to the same string.
            out.append(_check(f"{c['key']} (build)", keys.build(got), c["key"]))
    return out


def family_slugs(root: Path) -> list[Result]:
    data = _load_json(root / "slugs.json")
    out = []
    for c in data["slug"]:
        out.append(_check(f"slug {c['value']!r}", slug.slug(c["value"]), c["expect"]))
        # A slug case's chunk must unslug back to the value.
        out.append(_check(f"slug {c['value']!r} (round trip)", slug.unslug(c["expect"]), c["value"]))
    for c in data["unslug"]:
        out.append(_check(f"unslug {c['chunk']!r}", slug.unslug(c["chunk"]), c["expect"]))
    return out


def family_templates(root: Path) -> list[Result]:
    out = []
    for i, c in enumerate(_load_json(root / "templates.json")["cases"]):
        parsed = [templates.parse_template(t) for t in c["templates"]]
        hit = templates.resolve(parsed, c["chunks"])
        got = None if hit is None else {"winner": hit[0].text, "bindings": hit[1]}
        out.append(_check(f"#{i} {'/'.join(c['chunks'])}", got, c["expect"]))
    return out


# -- contracts/ (core.md §9.1–§9.5) -----------------------------------------

def family_contracts(root: Path) -> list[Result]:
    from .contract import load_contract

    d = root / "contracts"
    expect = _load_json(d / "expect.json")["cases"]
    out = []
    for stem, want in sorted(expect.items()):
        c = load_contract(d / f"{stem}.toml", spec_dir=root.parent)
        got = {"codes": c.codes, "fingerprint": c.fingerprint}
        case = _check(stem, got, want)
        if not case[1]:
            case = (stem, False, case[2] + "\n      " + "\n      ".join(
                f"{x.code} {x.message}" for x in c.diagnostics))
        out.append(case)
        canon = d / f"{stem}.canonical.json"
        if canon.exists():
            want_bytes = canon.read_bytes().rstrip(b"\n")
            ok = c.canonical_bytes == want_bytes
            out.append((f"{stem} (canonical bytes)", ok,
                        "" if ok else f"got {c.canonical_bytes!r}"))
    return out


# -- sets/ (core.md §9.2 E035, E036) -----------------------------------------

def family_sets(root: Path) -> list[Result]:
    from .sets import SetLoadError, check_set

    d = root / "sets"
    out = []
    for name, want in sorted(_load_json(d / "expect.json")["sets"].items()):
        try:
            got = check_set(sorted((d / name).glob("*.toml")), spec_dir=root.parent).codes
        except SetLoadError as e:
            out.append((name, False, str(e)))
            continue
        out.append(_check(name, {"codes": got}, want))
    return out


# -- bundles/ (core.md §9.6) -------------------------------------------------

def family_bundles(root: Path) -> list[Result]:
    from . import bundle
    from .contract import load_contract

    d = root / "bundles"
    out = []
    for name, want in sorted(_load_json(d / "expect.json")["cases"].items()):
        data = (d / name).read_bytes()
        try:
            v = bundle.verify(data, want.get("expect_fingerprint"))
            got: dict[str, Any] = {"ok": True, "fingerprint": v.fingerprint}
        except bundle.BundleError as e:
            got = {"ok": False, "error": e.tag}
        want_cmp = {k: v for k, v in want.items() if k != "expect_fingerprint"}
        out.append(_check(name, got, want_cmp))
    # Build: the README says the fixtures are "Built from seed.toml"; the
    # valid bundle is that build, byte for byte (one revision, one bundle).
    seed = load_contract(d / "seed.toml", spec_dir=root.parent)
    built = bundle.build(seed)
    ok = built == (d / "valid.bundle.json").read_bytes()
    out.append(("build seed.toml == valid.bundle.json", ok, "" if ok else f"got {built[:200]!r}…"))
    return out


# -- history/ (core.md §9.7) -------------------------------------------------

def family_history(root: Path) -> list[Result]:
    from .history import check_history

    d = root / "history"
    out = []
    for name, want in sorted(_load_json(d / "expect.json")["cases"].items()):
        got = [list(p) for p in check_history(d / name)]
        out.append(_check(name, got, want))
    return out


# -- descriptors/ (core.md §3.3) ---------------------------------------------

def family_descriptors(root: Path) -> list[Result]:
    from .contract import load_contract
    from .descriptor import check_descriptor

    d = root / "descriptors"
    contract = load_contract(d / "contracts" / "nav.v2.toml", spec_dir=root.parent)
    if not contract.valid:
        raise CannotRun(f"the fixture contract does not load: {contract.codes}")
    out = []
    for name, want in sorted(_load_json(d / "expect.json")["cases"].items()):
        path = d / f"{name}.json"
        if not path.exists():
            out.append((name, False, f"{path.name} is missing"))
            continue
        got = check_descriptor(path.read_bytes(), contract, spec_dir=root.parent)
        out.append(_check(name, {"codes": got}, want))
    return out


# -- errors/ (core.md §5.2) --------------------------------------------------

def family_errors(root: Path) -> list[Result]:
    from .envelope import EnvelopeError, decode

    out = []
    for c in _load_json(root / "errors" / "cases.json")["cases"]:
        payload = c["text"].encode("utf-8") if "text" in c else bytes.fromhex(c["hex"])
        try:
            got: Any = decode(c["encoding"], payload)
        except EnvelopeError as e:
            got = {"refused": e.tag}
        out.append(_check(c["name"], got, c["expect"]))
    return out


# -- compat/ (core.md §9.7 retention, §9.8) ----------------------------------

def _json_revision(path: Path, type_ref: str):
    """One JSON Schema payload revision: (world, stem, node), or None when it
    does not load (any E… finding: the ``invalid`` class)."""
    from .compat import JsonWorld
    from .schemas import SchemaSet

    s = SchemaSet.load(path.parent, [path.name], [], None)
    if any(f.code.startswith("E") for f in s.findings):
        return None
    obj, findings = s.resolve(type_ref)
    if obj is None or findings:
        return None
    art = s.json_files[0]
    return JsonWorld({art.name: art.data}), art.name, art.data["$defs"][obj["name"]]


def _proto_revision(directory: Path) -> bytes | None:
    from . import protoc

    try:
        return protoc.compile_file("m.proto", [directory])
    except protoc.CompileError:
        return None


def _compat_case(root: Path, case: str, want: dict[str, Any]) -> dict[str, Any]:
    from . import compat
    from .contract import load_contract

    d = root / "compat" / case
    got: dict[str, Any] = {}
    family = case.split("/")[0]
    if family == "contract":
        old = load_contract(d / "old.toml", spec_dir=root.parent)
        new = load_contract(d / "new.toml", spec_dir=root.parent)
        if not (old.valid and new.valid):
            return {"class": compat.INVALID, "warnings": []}

        def rev(c):
            return compat.Revision(c.canonical, {a.id: a.data for a in c.schemas.artifacts()})

        v = compat.contract_compare(rev(old), rev(new))
        return {"class": v.cls, "warnings": sorted(v.warnings)}

    kind = case.split("/")[1] if family == "payload" else ("jsonschema" if "jsonschema" in case else "protobuf")
    names = ["old", "new"] if family == "payload" else [*want["history"], want["candidate"]]
    if kind == "jsonschema":
        revs = [_json_revision(d / f"{n}.json", want["type"]) for n in names]
        if any(r is None for r in revs):
            return {"class": compat.INVALID, "warnings": []}

        def compare(a, b):
            return compat.json_compare(a[0], a[1], a[2], b[0], b[1], b[2])
    else:
        sets = [_proto_revision(d / n) for n in names]
        if any(s is None for s in sets):
            return {"class": compat.INVALID, "warnings": []}
        message = "." + want["type"]
        revs = [compat.ProtoWorld.from_sets([s]) for s in sets]

        def compare(a, b):
            return compat.proto_compare_message(a, message, b, message)

        if family == "payload":
            got["same_revision"] = compat.proto_same_revision([sets[0]], [sets[1]])
    total, each = compat.full_transitive(revs[:-1], revs[-1], compare)
    got.update({"class": total.cls, "warnings": total.warnings})
    if family == "transitive":
        got["against"] = {n: v.cls for n, v in zip(names[:-1], each)}
    return got


def family_compat(root: Path) -> list[Result]:
    out = []
    for case, want in sorted(_load_json(root / "compat" / "expect.json")["cases"].items()):
        got = _compat_case(root, case, want)
        out.append(_check(case, got, {k: want[k] for k in got}))
        # Make sure nothing the fixture expects went unchecked.
        unchecked = set(want) - set(got) - {"type", "history", "candidate"}
        if unchecked:
            out.append((case, False, f"expected members not evaluated: {sorted(unchecked)}"))
    return out


FAMILIES: dict[str, Callable[[Path], list[Result]]] = {
    "keys": family_keys,
    "slugs": family_slugs,
    "templates": family_templates,
    "contracts": family_contracts,
    "sets": family_sets,
    "bundles": family_bundles,
    "history": family_history,
    "descriptors": family_descriptors,
    "errors": family_errors,
    "compat": family_compat,
}


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="python -m zk2py.conformance", description=__doc__.split("\n")[0])
    ap.add_argument("conformance", type=Path, help="path to spec/conformance")
    ap.add_argument("--family", action="append", help="run only these families")
    args = ap.parse_args(argv)
    root: Path = args.conformance
    if not (root / "README.md").is_file():
        print(f"error: {root} is not a spec/conformance directory", file=sys.stderr)
        return 2
    failures: list[tuple[str, str, str]] = []
    rows = []
    for name, fn in FAMILIES.items():
        if args.family and name not in args.family:
            continue
        try:
            results = fn(root)
        except CannotRun as e:
            print(f"error: {name}: {e}", file=sys.stderr)
            return 2
        except Exception:  # noqa: BLE001 - report and stop: the runner is broken
            print(f"error: {name}: the family crashed", file=sys.stderr)
            traceback.print_exc()
            return 2
        passed = sum(1 for _, ok, _ in results if ok)
        rows.append((name, passed, len(results) - passed))
        failures += [(name, case, detail) for case, ok, detail in results if not ok]
    width = max(len(r[0]) for r in rows) if rows else 0
    for name, passed, failed in rows:
        print(f"{name:<{width}}  {passed:4d} passed  {failed:4d} failed")
    total_pass = sum(r[1] for r in rows)
    total_fail = sum(r[2] for r in rows)
    print(f"{'total':<{width}}  {total_pass:4d} passed  {total_fail:4d} failed")
    for fam, case, detail in failures:
        print(f"FAIL {fam}: {case}: {detail}")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())

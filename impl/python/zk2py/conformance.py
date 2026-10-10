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

#: compat/README.md: payload and transitive revisions are each "wrapped in a
#: one-resource contract", verbatim.
_WRAPPER = """[interface]
name = "m"
major = 1
minor = 0
[schemas]
{kind} = ["{artifact}"]
[resources.s]
kind = "state"
type = "{type}"
"""


def _revision(contract):
    from . import compat

    return compat.Revision(contract.canonical, {a.id: a.data for a in contract.schemas.artifacts()})


def _wrapped(root: Path, directory: Path, kind: str, artifact: str, type_ref: str):
    """Load one wrapped revision. The wrapper is read as if it were a file in
    ``directory``, so ``artifact`` resolves as the README's ``<artifact>``
    does: ``m.proto`` inside a protobuf revision's directory, or the case's
    ``.json`` file."""
    from .contract import load_contract

    text = _WRAPPER.format(kind=kind, artifact=artifact, type=type_ref)
    return load_contract(directory / "m.v1.toml", text=text, spec_dir=root.parent)


def _compat_case(root: Path, case: str, want: dict[str, Any]) -> dict[str, Any]:
    from . import compat
    from .contract import load_contract

    d = root / "compat" / case
    got: dict[str, Any] = {}
    family = case.split("/")[0]
    if family == "contract":
        contracts = [load_contract(d / f"{n}.toml", spec_dir=root.parent) for n in ("old", "new")]
    else:
        kind = case.split("/")[1] if family == "payload" else (
            "jsonschema" if (d / "v1.json").exists() else "protobuf")
        names = ["old", "new"] if family == "payload" else [*want["history"], want["candidate"]]
        if kind == "protobuf":
            contracts = [_wrapped(root, d / n, kind, "m.proto", want["type"]) for n in names]
        else:
            contracts = [_wrapped(root, d, kind, f"{n}.json", want["type"]) for n in names]
    # compat/README.md: `invalid` "when the new revision must not load (its
    # only error is E037)". A history revision that does not load is
    # reported the same way; compat/README.md (0.5) says only a candidate
    # can be, and every earlier revision loads.
    if not all(c.valid for c in contracts):
        return {"class": compat.INVALID, "warnings": []}
    revs = [_revision(c) for c in contracts]
    total, each = compat.full_transitive(revs[:-1], revs[-1], compat.contract_compare)
    got.update({"class": total.cls, "warnings": total.warnings})
    if family == "transitive":
        got["against"] = {n: v.cls for n, v in zip(names[:-1], each)}
    if "same_revision" in want:
        # §9.7's retention identity (0.5), between the two revisions.
        got["same_revision"] = compat.identical(revs[0], revs[1])
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


# -- examples/zk2 (not a fixture family: extra inputs) -----------------------

def family_examples(root: Path) -> list[Result]:
    """``examples/zk2/``, and the profiles' standard contracts under
    ``spec/profiles/`` (core §10 point 1, 0.23): every contract
    (``<name>.v<major>.toml``) loads with no finding at all, W107 included,
    and its built bundle verifies with its fingerprint. It is published in
    its tree's history root, ``examples/zk2/.history`` or
    ``spec/profiles/.history`` (§9.7), byte-identical, and compatible with
    it. Every bundle under a ``.history`` directory passes the §9.7 history
    check, and a revision held in both roots is one file, byte for byte
    ("One revision, two roots", 0.23)."""
    ex = root.parent.parent / "examples" / "zk2"
    if not ex.is_dir():
        print(f"note: {ex} not found; examples skipped", file=sys.stderr)
        return []
    profiles = root.parent / "profiles"
    out = _walk_contracts(root, ex, ex / ".history", "")
    if profiles.is_dir():
        out += _walk_contracts(root, profiles, profiles / ".history", "spec/profiles/", skip=("conformance",))
        out += _two_roots(ex / ".history", profiles / ".history")
    return out


def _two_roots(old: Path, new: Path) -> list[Result]:
    """§9.7 (0.23): "A contract that moves to a new root keeps its published
    revisions in the old one … and is published in the new one from then
    on, its revisions copied there byte for byte"."""
    out: list[Result] = []
    if not old.is_dir() or not new.is_dir():
        return out
    for d in sorted(x for x in new.iterdir() if x.is_dir() and (old / x.name).is_dir()):
        for b in sorted(d.glob("*.bundle.json")):
            twin = old / d.name / b.name
            if twin.is_file():
                same = twin.read_bytes() == b.read_bytes()
                out.append((f"{d.name}/{b.name[:16]}… in both history roots, byte for byte (§9.7, 0.23)", same,
                            "" if same else "the two copies differ"))
    return out


def _walk_contracts(root: Path, ex: Path, history: Path, label: str, skip: tuple[str, ...] = ()) -> list[Result]:
    import re

    from . import bundle
    from .contract import load_contract
    from .history import check_history

    out = []
    for path in sorted(ex.rglob("*.toml")):
        if ".history" in path.parts or any(s in path.relative_to(ex).parts for s in skip) \
                or not re.fullmatch(r".+\.v[0-9]+\.toml", path.name):
            continue
        rel = label + path.relative_to(ex).as_posix()
        c = load_contract(path, check_file_name=True, spec_dir=root.parent)
        ok = not c.diagnostics
        out.append((rel, ok, "" if ok else "; ".join(f"{d.code} {d.message}" for d in c.diagnostics)))
        if not c.valid:
            continue
        try:
            data = bundle.build(c)
        except NotImplementedError as e:
            out.append((f"{rel} (bundle)", False, str(e)))
            continue
        try:
            v = bundle.verify(data, c.fingerprint)
            out.append((f"{rel} (bundle round trip)", v.fingerprint == c.fingerprint, ""))
        except bundle.BundleError as e:
            out.append((f"{rel} (bundle round trip)", False, e.tag))
            continue
        out += _example_against_history(history, rel, c, data)
    histories = sorted(p for p in ex.rglob(".history") if p.is_dir())
    for h in histories:
        problems = check_history(h)
        out.append((label + h.relative_to(ex).as_posix(), not problems, "" if not problems else str(problems)))
    return out


def _example_against_history(root: Path, rel: str, c, built: bytes) -> list[Result]:
    """An example must be published in its tree's history root
    (``examples/zk2/.history``, or ``spec/profiles/.history`` for a
    profile's standard contract) and be compatible with its history (§9.7,
    §9.8):
    - its fingerprint names a published bundle, and the bundle zk2py builds
      is byte-identical to it (§9.6: "one contract revision has one
      bundle");
    - against every published revision of its interface, FULL_TRANSITIVE,
      the class is ``compatible``."""
    from . import bundle, compat

    hist = root / c.interface
    published = hist / f"{c.fingerprint.removeprefix('sha256:')}.bundle.json"
    out: list[Result] = []
    if not published.is_file():
        return [(f"{rel} (published)", False, f"{published.relative_to(root.parent)} is missing")]
    same = published.read_bytes() == built
    out.append((f"{rel} (published, byte-identical)", same, "" if same else "the built bundle differs"))
    revisions = [bundle.revision(bundle.verify(p.read_bytes()))
                 for p in sorted(hist.glob("*.bundle.json"))]
    candidate = compat.Revision(c.canonical, {a.id: a.data for a in c.schemas.artifacts()})
    total, _ = compat.full_transitive(revisions, candidate, compat.contract_compare)
    ok = total.cls == compat.COMPATIBLE
    out.append((f"{rel} (compatible with {len(revisions)} published)", ok,
                "" if ok else f"{total.cls}: {total.reasons}"))
    return out


# -- profiles/<name>/conformance/ (core §10, 0.19; profiles/README.md) ------

def _hostid_vectors(path: Path) -> list[Result]:
    """hostid.v1 §2.1: "a machine id and a salt -> the minted system, or null
    when the input is refused. The input is a JSON string, whose UTF-8
    encoding is the file's bytes"."""
    from . import hostid

    return [_check(f"vectors: {c['input']!r} with {c['salt']} ({c['note']})",
                   hostid.derive(c["input"].encode("utf-8"), c["salt"]), c["expect"])
            for c in _load_json(path)["cases"]]


def _hostid_shapes(path: Path) -> list[Result]:
    """hostid.v1 §2.11: "a string -> whether it is in the minted shape"."""
    from . import hostid

    return [_check(f"shapes: {c['value']!r} ({c['note']})", hostid.is_minted_shape(c["value"]), c["expect"])
            for c in _load_json(path)["cases"]]


def _freshness_horizons(path: Path) -> list[Result]:
    """freshness.v1 §2.1–§2.3: "a resolved resource's authoring kind … and
    its merged annotations -> its horizon"."""
    from . import freshness

    return [_check(f"horizons: {c['kind']} {json.dumps(c['annotations'])} ({c['note']})",
                   freshness.horizon(c["kind"], c["annotations"]).expect(), c["expect"])
            for c in _load_json(path)["cases"]]


def _freshness_observation(o: dict[str, Any]):
    """judgements.json's observation objects, read into zk2py's, with every
    number of seconds read exactly to the nanosecond."""
    from . import freshness as f

    if o["via"] == "archive":
        return f.Archive()
    if o["via"] == "subscription":
        last = o.get("last")
        return f.Subscription(f.seconds_to_ns(o["listened_s"]), bool(o.get("complete", True)),
                              None if last is None else f.Delivery(last["kind"], f.seconds_to_ns(last["age_s"])))
    if o["via"] == "get":
        reply, clock = o.get("reply"), o.get("clock") or {"trusted": False}
        delta = f.seconds_to_ns(clock["delta_s"]) if clock.get("trusted") else f.DEFAULT_DELTA_NS
        if reply is None:
            r = None
        elif reply["kind"] == "delete":
            r = f.Reply("delete")
        else:
            age = reply.get("stamp_age_s")
            r = f.Reply("put", None if age is None else f.seconds_to_ns(age))
        return f.Get(r, bool(clock.get("trusted")), delta)
    raise ValueError(f"an unknown observation {o!r}")


def _freshness_judgements(path: Path) -> list[Result]:
    """freshness.v1 §2.3, §2.5–§2.8: a resource's kind, its annotations and
    the observations of one member -> the combined verdict and its reason."""
    from . import freshness

    out: list[Result] = []
    for c in _load_json(path)["cases"]:
        got = freshness.judge(c["kind"], c["annotations"], [_freshness_observation(o) for o in c["observations"]])
        out.append(_check(f"judgements: {c['note']}", got.pair(), c["expect"]))
    return out


def _freshness_clock_trust(path: Path) -> list[Result]:
    """freshness.v1 §2.6 ground 2 (0.2): the offsets of one reading, and the
    delta -> whether the clock is trusted."""
    from . import freshness

    out: list[Result] = []
    for c in _load_json(path)["cases"]:
        trust = freshness.ClockTrust(freshness.seconds_to_ns(c["delta_s"]))
        for off in c["offsets_s"]:
            trust.record("clock", freshness.seconds_to_ns(off))
        out.append(_check(f"clock-trust: {c['offsets_s']} at {c['delta_s']} s ({c['note']})",
                          {"trusted": trust.measured("clock")}, c["expect"]))
    return out


def _health_judgements(path: Path) -> list[Result]:
    """health.v1 §2.11: one reading of one service -> the answer to "is this
    service healthy?"."""
    from . import health

    out: list[Result] = []
    for c in _load_json(path)["cases"]:
        st = c.get("status") or {}
        reading = health.Reading(
            presence=c["presence"], descriptor=c.get("descriptor"), face=c.get("face"),
            observations=[_freshness_observation(o) for o in st.get("observations") or []],
            level=health.level_of(st.get("level")),
            checks=None if c.get("checks") is None
            else [(x["check"], health.level_of(x["level"])) for x in c["checks"]])
        out.append(_check(f"judgements: {c['note']}", health.judge(reading).expect(), c["expect"]))
    return out


def _health_rollups(path: Path) -> list[Result]:
    """health.v1 §2.2, §5: judged readings -> a tool's roll-up."""
    from . import health

    return [_check(f"rollups: {c['note']}",
                   health.rollup([health.Answer(j["verdict"], "", j["level"]) for j in c["judged"]]), c["expect"])
            for c in _load_json(path)["cases"]]


def _health_codes(path: Path) -> list[Result]:
    """health.v1 §2.10: a fault's code -> profile, application or
    malformed."""
    from . import health

    return [_check(f"codes: {c['code']!r}", health.code_class(c["code"]), c["expect"])
            for c in _load_json(path)["cases"]]


def _health_clock(path: Path) -> list[Result]:
    """health.v1 §5 (0.3): what one subscriber heard of one service, in
    arrival order, and its presence -> "is this service's clock ahead?"."""
    from . import health

    return [_check(f"clock: {c['heard']} ({c['note']})",
                   dict(zip(("answer", "reason"),
                            health.clock_question(c["presence"], c.get("descriptor"), c["heard"]))),
                   c["expect"])
            for c in _load_json(path)["cases"]]


#: Each profile's fixtures, by file name. profiles/README.md: "An unknown
#: file fails the test, so a fixture cannot land unrun. A README.md there
#: is prose."
PROFILE_FIXTURES: dict[str, dict[str, Callable[[Path], list[Result]]]] = {
    "hostid": {"vectors.json": _hostid_vectors, "shapes.json": _hostid_shapes},
    "freshness": {"horizons.json": _freshness_horizons, "judgements.json": _freshness_judgements,
                  "clock-trust.json": _freshness_clock_trust},
    "health": {"judgements.json": _health_judgements, "rollups.json": _health_rollups,
               "codes.json": _health_codes, "clock.json": _health_clock},
}


def _profile_family(name: str) -> Callable[[Path], list[Result]]:
    def family(root: Path) -> list[Result]:
        directory = root.parent / "profiles" / name / "conformance"
        if not directory.is_dir():
            raise CannotRun(f"{directory} not found")
        out: list[Result] = []
        for path in sorted(directory.iterdir()):
            if path.name == "README.md":
                continue
            run = PROFILE_FIXTURES[name].get(path.name)
            if run is None:
                out.append((f"{path.name}", False, "an unknown fixture file: the runner does not read it"))
                continue
            out += run(path)
        return out

    return family


def _budget_bounds(path: Path) -> list[Result]:
    """Core §2.7 (0.24): the contract's cardinality and each instance's
    stated one -> the bound a tool judges against."""
    from . import budget

    return [_check(f"bounds: {c['note']}", {"bound": budget.bound(c["contract"], c["stated"])}, c["expect"])
            for c in _load_json(path)["cases"]]


def _budget_population(path: Path) -> list[Result]:
    """Core §2.7 (0.24): a kind, a bound, a retention and one reading -> the
    population verdict and its reason."""
    from . import budget
    from .freshness import seconds_to_ns

    out: list[Result] = []
    for c in _load_json(path)["cases"]:
        r = c["reading"]
        if r["via"] == "get":
            reading: Any = budget.GetReading(r["members"], r["complete"])
        else:
            reading = budget.WindowReading({m: [seconds_to_ns(x) for x in ts] for m, ts in r["heard"].items()},
                                           seconds_to_ns(r["listened_s"]), r["lossless"], r["present"])
        ret = None if c.get("retention_s") is None else seconds_to_ns(c["retention_s"])
        out.append(_check(f"population: {c['note']}", budget.population(c["kind"], c["bound"], ret, reading).pair(),
                          c["expect"]))
    return out


def _budget_rate(path: Path) -> list[Result]:
    """Core §2.7 (0.24): an event's rate and a window's occurrences -> the
    rate verdict and its reason."""
    from . import budget
    from .freshness import seconds_to_ns

    out: list[Result] = []
    for c in _load_json(path)["cases"]:
        r = c["reading"]
        members = {m: [seconds_to_ns(x) for x in ts] for m, ts in r["members"].items()}
        got = budget.rate(c["rate"], members, seconds_to_ns(r["listened_s"]), r["complete"])
        out.append(_check(f"rate: {c['note']}", got.pair(), c["expect"]))
    return out


BUDGET_FIXTURES = {"bounds.json": _budget_bounds, "population.json": _budget_population, "rate.json": _budget_rate}


def family_budget(root: Path) -> list[Result]:
    """``budget/`` (core §2.7, 0.24): every file, and an unknown one fails."""
    out: list[Result] = []
    for path in sorted((root / "budget").iterdir()):
        run = BUDGET_FIXTURES.get(path.name)
        if run is None:
            out.append((f"budget/{path.name}", False, "an unknown fixture file: the runner does not read it"))
            continue
        out += run(path)
    return out


#: Every entry of spec/conformance/ that a family reads. Appendix E lists
#: the fixture formats; an entry no family reads fails ``layout``, so a new
#: family cannot land unrun.
KNOWN_ENTRIES = {"README.md", "keys.json", "slugs.json", "templates.json", "contracts", "sets", "bundles",
                 "history", "descriptors", "errors", "compat", "budget"}


def family_layout(root: Path) -> list[Result]:
    return [(f"spec/conformance/{p.name}", p.name in KNOWN_ENTRIES,
             "" if p.name in KNOWN_ENTRIES else "an entry no family reads")
            for p in sorted(root.iterdir())]


FAMILIES: dict[str, Callable[[Path], list[Result]]] = {
    "layout": family_layout,
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
    "budget": family_budget,
    "examples": family_examples,
    # The profiles, one family each (profiles/README.md, "How the harnesses
    # take profiles in").
    **{name: _profile_family(name) for name in PROFILE_FIXTURES},
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

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
import threading
import time
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


def _config_facts(state: dict[str, Any]):
    from . import config

    p, last = state.get("pending"), state.get("last_change")
    return config.Facts(revision=state["revision"], in_flight=state["in_flight"],
                        pending_token=None if p is None else p["token"],
                        pending_applied=True if p is None else p["applied"],
                        deadline_passed=bool(p and p["deadline_passed"]),
                        last_token=None if last is None else last["token"],
                        last_sensitive=bool(last and last["set_sensitive"]), persist=state["persist"])


def _config_validation(path: Path) -> list[Result]:
    """config.v1 §2.5, §2.10, §2.13: one request against a schema and the
    owner's state -> accepted, or the refusal the owner sends first."""
    from . import config

    data = _load_json(path)
    out: list[Result] = []
    for c in data["cases"]:
        f, req = _config_facts(c["state"]), c["request"]
        if c["op"] == "set":
            r = config.decode_set(req) or config.check_set(data["schemas"][c["schema"]], c["group"], req, f)
        elif c["op"] == "persist":
            r = config.persist_unavailable(f) or config.decode_control(req) or config.check_persist(req, f)
        else:
            r = config.decode_control(req) or config.check_control(c["op"], req, f)
        got = {"accepted": True} if r is None else {"refused": r.expect()}
        out.append(_check(f"validation: {c['op']} {c.get('group') or ''} ({c['note']})", got, c["expect"]))
    return out


def _config_window(path: Path) -> list[Result]:
    """config.v1 §2.8: confirm_s, the floor and the instant received -> the
    window and the deadline, written as the owner writes it."""
    from . import config

    return [_check(f"window: {c['note']}",
                   {"window_s": config.window(c["confirm_s"], c["floor_s"]),
                    "deadline": config.write_time(config.deadline(config.parse_time(c["received"]), c["confirm_s"],
                                                                  c["floor_s"]))},
                   c["expect"])
            for c in _load_json(path)["cases"]]


def _config_desired(path: Path) -> list[Result]:
    """config.v1 §2.18: a desired document against a schema -> accepted, or
    refused whole with the first problem."""
    from . import config

    data = _load_json(path)
    out: list[Result] = []
    for c in data["cases"]:
        r = config.check_desired(data["schemas"][c["schema"]], c["document"])
        out.append(_check(f"desired: {c['note']}", {"accepted": True} if r is None else {"refused": r},
                          c["expect"]))
    return out


def _config_core(group: dict[str, Any], running: dict[str, Any], startup: dict[str, Any], device: Any = None):
    from . import config

    core = config.Core(service="lab/rf0", instance="i1", schema=[group],
                       startup={group["name"]: {p: {"value": v, "source": "file"} for p, v in startup.items()}},
                       floor_s=10, device=device)
    for p, rv in running.items():
        core.running[(group["name"], p)] = config.Param(rv["value"], rv["source"])
    return core


def _config_redaction(path: Path) -> list[Result]:
    """config.v1 §2.15: what the view, the status, the event, the log and an
    envelope carry for a sensitive value."""
    from . import config

    out: list[Result] = []
    for c in _load_json(path)["cases"]:
        g, req = c["group"], c["request"]
        core = _config_core(g, c["running"], c["startup"])
        step = core.set(g["name"], req, 0)
        if "refused" in c["expect"]:
            r = step.reply
            if not isinstance(r, config.Refusal):
                got: Any = {"accepted": config.project_reply(r)}
            else:
                name = r.field.rsplit("/", 1)[-1] if r.field else None
                v = req["values"].get(name)
                repeats = json.dumps(v) in r.message or (isinstance(v, str) and v in r.message)
                got = {"refused": {"code": r.code, "field": r.field, "message_repeats_value": repeats}}
            out.append(_check(f"redaction: {c['note']}", got, c["expect"]))
            continue
        reply = step.reply
        doc = reply["applied"] if isinstance(reply, dict) and "applied" in reply else core.view()
        view_params = next(x for x in doc["groups"] if x["name"] == g["name"])["parameters"]
        status = config.status_of(doc)["values"].get(g["name"])
        edits = step.events[0]["edits"] if step.events else None
        logged = json.loads(core.log[-1])["request"] if core.log else None
        detail = None
        if c["device_words"] is not None:
            refusing = _config_core(g, c["running"], c["startup"],
                                    device=lambda act, w=c["device_words"]: {"refused": w})
            r = refusing.set(g["name"], req, 0).reply
            detail = r.detail if isinstance(r, config.Refusal) else "(not refused)"
        got = {"view": view_params, "status": status, "edits": edits, "log": logged, "device_detail": detail}
        out.append(_check(f"redaction: {c['note']}", got, c["expect"]))
    return out


def _config_state(core: Any) -> dict[str, Any]:
    from . import config

    view = core.view()
    params = {g["name"]: {p["name"]: {k: p[k] for k in ("source", "startup") if k in p} for p in g["parameters"]}
              for g in view["groups"]}
    return {"status": config.status_of(view), "params": params,
            "writable": [g["name"] for g in view["groups"] if g["writable"]]}


def _config_lifecycle(path: Path) -> list[Result]:
    """config.v1 §2.3, §2.6 to §2.14, §2.17, §2.19: sequences of calls,
    device results and instants on one owner -> its replies, acts, events
    and state after each step."""
    from . import config
    from .freshness import seconds_to_ns

    data = _load_json(path)
    out: list[Result] = []
    for i, c in enumerate(data["cases"]):
        s = c["setup"]
        results: list[Any] = []

        def device(act: tuple, results: list[Any] = results) -> Any:
            return results.pop(0) if results else "ok"

        events_at = [seconds_to_ns(e["t"]) for e in s["events_at"] for _ in range(e["count"])]
        core = config.Core(service=s["service"], instance=s["instance"], schema=data["schemas"][s["schema"]],
                           startup=s["startup"], overlay=json.loads(json.dumps(s["overlay"])), floor_s=s["floor_s"],
                           apply_after_ns=s["apply_after_ms"] * config.MS, persist=s["persist"], device=device,
                           events_at=events_at)
        name = f"lifecycle {i}: {c['note']}"
        out.append(_check(f"{name}: start", _config_state(core), c["start"]))
        for j, st in enumerate(c["steps"]):
            results[:] = list(st.get("device") or [])
            t = seconds_to_ns(st["t"])
            do = st["do"]
            if do == "set":
                step = core.set(st["group"], st["request"], t, st.get("call"))
            elif do in ("confirm", "cancel", "extend"):
                step = core.control(do, st["request"], t)
            elif do == "persist":
                step = core.persist(st["request"], t)
            elif do == "due":
                step = core.due(t)
            elif do == "complete":
                step = core.complete(results.pop(0) if results else "ok", t)
            elif do == "restart":
                step = core.restart(st["instance"])
            else:
                raise CannotRun(f"{path}: unknown step {do!r}")
            got = {"reply": config.project_reply(step.reply), "acts": step.acts, "events": step.events,
                   "state": _config_state(core)}
            out.append(_check(f"{name}: step {j} ({do} at {st['t']})", got, st["expect"]))
    return out


def _link_interfaces(case: dict[str, Any]) -> dict[str, Any]:
    """plan.json's deployment: the interfaces given inline ("one revision's
    resources, or {revisions: [resources, ...]}", 0.3), or loaded from
    authoring files "relative to the repository root", as core §9 loads
    them; "two files in contracts with one interface id are two revisions
    of it"."""
    from . import link
    from .contract import load_contract

    if "interfaces" in case:
        return case["interfaces"]
    repo = Path(__file__).resolve().parents[3]
    revisions: dict[str, list[list[dict[str, Any]]]] = {}
    for rel in case.get("contracts", []):
        c = load_contract(repo / rel)
        if not c.valid or c.canonical is None:
            raise CannotRun(f"{rel}: {c.codes}")
        revisions.setdefault(c.interface, []).append(link.resources_of(c))
    return {i: {"revisions": revs} for i, revs in revisions.items()}


def _link_exposure(path: Path) -> list[Result]:
    """link.v1 §2.2, §2.7: a resource and the face's expose -> its exposure
    and whether its pushes cross."""
    from . import link

    return [_check(f"exposure: {c['note']}", link.exposure(c["token"], c["template"], c["annotations"], c["expose"]),
                   c["expect"])
            for c in _load_json(path)["cases"]]


def _link_plan(path: Path) -> list[Result]:
    """link.v1 §2.3: a face policy and a deployment -> each face's plan, or
    the findings that refuse it."""
    from . import link

    return [_check(f"plan: {c['note']}", link.plan(c["policy"], c["implements"], _link_interfaces(c)), c["expect"])
            for c in _load_json(path)["cases"]]


def _link_lints(path: Path) -> list[Result]:
    """link.v1 §2.1, §2.3, §2.11, §2.15: a policy, and optionally a
    deployment, its windows and a router's block -> every finding."""
    from . import link

    out: list[Result] = []
    for c in _load_json(path)["cases"]:
        has = "implements" in c
        got = link.lint(c["policy"], implements=c.get("implements") if has else None,
                        interfaces=_link_interfaces(c) if has else None, windows=c.get("windows"),
                        block=c.get("block"))
        out.append(_check(f"lints: {c['note']}", got, c["expect"]))
    return out


def _alarms_keys(path: Path) -> list[Result]:
    """alarms.v1 §2.2, §2.3, §2.12: `derive`, `host_scoped`, `ref` and
    `parse`, each by its `op`. A `derive` case also checks that framed input
    gives one key by both recipes."""
    from . import alarms

    out: list[Result] = []
    for i, c in enumerate(_load_json(path)["cases"]):
        name = f"keys #{i} {c['op']}: {c['note']}"
        if c["op"] == "derive":
            got = alarms.derive(c["rule"], c["labels"])
            out.append(_check(name, got.expect(), c["expect"]))
            strict = alarms.strict_key(c["rule"], c["labels"])
            want_strict = got.key if c["expect"]["framed"] else None
            out.append(_check(f"{name} (strict recipe)", strict, want_strict))
        elif c["op"] == "host_scoped":
            out.append(_check(name, alarms.is_host_scoped(c["name"]), c["expect"]))
        elif c["op"] == "ref":
            r = c["ref"]
            out.append(_check(name, alarms.key_of(r["system"], r["service"], r["alert_key"]), c["expect"]))
        elif c["op"] == "parse":
            got_ref = alarms.parse_key(c["key"])
            out.append(_check(name, got_ref, c["expect"]))
            if got_ref is not None:
                # §2.12: the reference builds back to the key it was read from.
                back = alarms.key_of(got_ref["system"], got_ref["service"], got_ref["alert_key"])
                out.append(_check(f"{name} (builds back)", back, c["key"]))
        else:
            out.append((name, False, f"an unknown op {c['op']!r}"))
    return out


def _alarms_documents(path: Path) -> list[Result]:
    """alarms.v1 §2.4, §2.3: a payload, decoded or (0.2) as its text ->
    readable, the owner's errors and its warnings, in the order checked."""
    from . import alarms

    out: list[Result] = []
    for i, c in enumerate(_load_json(path)["cases"]):
        # A case gives the decoded `document`, or (0.2) its `text`, whose
        # UTF-8 bytes are the payload.
        got = alarms.check_payload(c["text"].encode("utf-8", "surrogatepass")) if "text" in c \
            else alarms.check(c["document"])
        out.append(_check(f"documents #{i}: {c['note']}", got, c["expect"]))
    return out


def _alarms_transitions(path: Path) -> list[Result]:
    """alarms.v1 §2.6: one key's samples in the owner's stamp order -> the
    transition each makes against the document held before it."""
    from . import alarms

    out: list[Result] = []
    for i, c in enumerate(_load_json(path)["cases"]):
        samples = [alarms.DELETE if s == "delete" else s["put"] for s in c["samples"]]
        out.append(_check(f"transitions #{i}: {c['note']}", alarms.transitions(samples), c["expect"]))
    return out


def _alarms_seed(s: Any):
    from . import alarms

    return None if s is None else alarms.Seed(bool(s["complete"]), int(s.get("members", 0)))


def _alarms_window(w: Any):
    from . import alarms

    return None if w is None else alarms.Window(w["listened_s"], bool(w["lossless"]), bool(w["present_throughout"]))


def _alarms_conclusions(path: Path) -> list[Result]:
    """alarms.v1 §2.14, §2.15, §5, by `question`: `reading` (the first and
    last questions of §5 over one reading), `owner_set`, `any_firing` and
    `rederives`. An unknown question fails."""
    from . import alarms

    out: list[Result] = []
    for i, c in enumerate(_load_json(path)["cases"]):
        q, name = c["question"], f"conclusions #{i} {c['question']}: {c['note']}"
        needs = {"reading": ("left_out",), "any_firing": ("scope_whole",)}.get(q, ())
        missing = [k for k in needs if k not in c]
        if missing:
            out.append((name, False, f"the case lacks {missing} (alarms.v1 0.5)"))
            continue
        if q == "reading":
            r = alarms.Reading(presence=c["presence"], descriptor=c.get("descriptor"), face=c.get("face"),
                               observations=[_freshness_observation(o) for o in c["observations"]],
                               latest=c["latest"], previous_instance=bool(c["previous_instance"]),
                               held_whole=bool(c["held_whole"]), left_out=bool(c["left_out"]))
            got: Any = {"firing": alarms.judge(r, alarms.Q_FIRING).pair(),
                        "stale_firing": alarms.judge(r, alarms.Q_STALE).pair()}
        elif q == "owner_set":
            got = alarms.owner_set(c["presence"], c.get("descriptor"), c.get("face"), _alarms_seed(c["seed"]),
                                   _alarms_window(c["window"])).expect()
        elif q == "any_firing":
            owners = [alarms.OwnerRead(o["set"]["verdict"],
                                       tuple(alarms.AlarmRead(a["answer"], a["severity"]) for a in o["alarms"]))
                      for o in c["owners"]]
            got = alarms.any_firing(c["at_least"], owners, scope_whole=bool(c["scope_whole"]))
        elif q == "rederives":
            payload = alarms.DELETE if c["payload"] == "delete" else c["payload"]
            got = alarms.rederives(c["alert_key"], payload).pair()
        else:
            out.append((name, False, f"an unknown question {q!r}"))
            continue
        out.append(_check(name, got, c["expect"]))
    return out


def _telemetry_schema(path: Path) -> list[Result]:
    """telemetry.v1 §2.3, §3: the standard schema's published ids. The
    file's header first (its ``file``, ``current``, ``published`` and
    ``types`` against zk2py's tables), then each case's document, by path
    from the repository's root, as the artifact named ``telemetry.v1``
    inside a bundle, or inline -> its artifact id (sha256 over its JCS
    bytes) and whether the profile publishes it."""
    from . import jcs, telemetry as tm

    data = _load_json(path)
    repo = path.parents[4]
    out = [
        _check("schema: the profile's file is the current revision",
               tm.artifact_id(jcs.loads((path.parents[1] / data["file"]).read_bytes())), data["current"]),
        _check("schema: zk2py's published ids are §3's, oldest first",
               [[i, r] for i, r in tm.PUBLISHED], [[x["id"], x["revision"]] for x in data["published"]]),
        _check("schema: zk2py's four types, each with its first revision",
               sorted(tm.TYPES.items()), sorted((t["name"], t["since"]) for t in data["types"])),
    ]
    for i, c in enumerate(data["cases"]):
        name = f"schema #{i}: {c['note']}"
        if "path" in c:
            doc = jcs.loads((repo / c["path"]).read_bytes())
        elif "bundle" in c:
            doc = tm.bundle_artifact((repo / c["bundle"]).read_bytes())
        else:
            doc = c["document"]
        got = tm.artifact_id(doc) if doc is not None else None
        out.append(_check(name, {"id": got, "published": tm.is_published(got)}, c["expect"]))
    return out


def _telemetry_resource(r: dict[str, Any]):
    from . import telemetry as tm

    ann = r["annotations"] if "annotations" in r else tm.resolve(r["layers"])
    return tm.declare(r["kind"], ann, r["type"], r.get("attachment"))


def _telemetry_declarations(path: Path) -> list[Result]:
    """telemetry.v1 §2.1, §2.2, §2.4: a resolved resource -> its
    declaration."""
    return [_check(f"declarations #{i}: {c['note']}", _telemetry_resource(c["resource"]).expect(), c["expect"])
            for i, c in enumerate(_load_json(path)["cases"])]


def _telemetry_sample(s: dict[str, Any]):
    """A sample as samples.json writes it: decoded (``value``,
    ``attachment``), or, since 0.2, as its bytes came: ``json_text`` read as
    strict JSON, ``cbor_hex`` as CBOR, ``attachment_text`` as JSON, an
    attachment that does not decode being an epoch present and not a
    string."""
    from . import telemetry as tm

    if s.get("undecodable"):
        return tm.Sample(undecodable=True)
    try:
        if "json_text" in s:
            value = tm.decode_json(s["json_text"].encode("utf-8", "surrogatepass"))
        elif "cbor_hex" in s:
            value = tm.decode_cbor(bytes.fromhex(s["cbor_hex"]))
        else:
            value = tm.from_fixture(s["value"])
    except tm.Undecodable:
        return tm.Sample(undecodable=True)
    if "attachment_text" in s:
        try:
            return tm.Sample(value, tm.decode_json(s["attachment_text"].encode("utf-8", "surrogatepass")))
        except tm.Undecodable:
            return tm.Sample(value, attachment_undecodable=True)
    return tm.Sample(value, tm.from_fixture(s["attachment"]) if "attachment" in s else tm.NO_ATTACHMENT)


def _telemetry_samples(path: Path) -> list[Result]:
    """telemetry.v1 §2.5, §2.6: a resolved resource and one sample -> its
    conformance, and the member's kind and unit as the sample gives them."""
    from . import telemetry as tm

    return [_check(f"samples #{i}: {c['note']}",
                   tm.conform(_telemetry_resource(c["resource"]), _telemetry_sample(c["sample"])).expect(),
                   c["expect"])
            for i, c in enumerate(_load_json(path)["cases"])]


def _telemetry_continuity(path: Path) -> list[Result]:
    """telemetry.v1 §2.9 to §2.11: two conforming samples, the counter's
    unit and what the reader saw of presence -> the pair's continuity, how
    its epoch was had, and its rate, compared exactly."""
    from . import telemetry as tm

    def read(x: dict[str, Any]):
        # 0.2: stamp_ntp64 in stamp_ns's place, the raw value, read as
        # zenoh's NTP64::as_nanos reads it, each stamp alone (F-142).
        stamp = tm.ntp64_ns(x["stamp_ntp64"]) if x.get("stamp_ntp64") is not None else x["stamp_ns"]
        return tm.Read(x["kind"], tm.from_fixture(x["value"]), x["epoch"], x["timestamp_ms"], stamp, x["arrival_ns"])

    out = []
    for i, c in enumerate(_load_json(path)["cases"]):
        p = c["presence"]
        presence = None if p is None else tm.Presence(bool(p["alone_throughout"]), p["earlier"], p["later"])
        got = tm.continuity(read(c["earlier"]), read(c["later"]), presence, c["unit"]).expect()
        out.append(_check(f"continuity #{i}: {c['note']}", got, c["expect"]))
    return out


def _desired_schema(path: Path) -> list[Result]:
    """desired.v1 §3: the standard schema's published ids. The file's header
    first (its ``file``, ``current``, ``published`` and ``types`` against
    zk2py's tables), then each case's document, by path from the
    repository's root or inline -> its artifact id (sha256 over its JCS
    bytes) and whether the profile publishes it."""
    from . import desired as ds, jcs

    data = _load_json(path)
    repo = path.parents[4]
    out = [
        _check("schema: the profile's file is the current revision",
               ds.artifact_id(jcs.loads((path.parents[1] / data["file"]).read_bytes())), data["current"]),
        _check("schema: zk2py's current id is the file's", ds.CURRENT, data["current"]),
        _check("schema: zk2py's published ids are §3's, oldest first",
               [[i, r] for i, r in ds.PUBLISHED], [[x["id"], x["revision"]] for x in data["published"]]),
        _check("schema: zk2py's three types, each with its first revision",
               sorted(ds.TYPES.items()), sorted((t["name"], t["since"]) for t in data["types"])),
        _check("schema: each type a $defs member of the current file",
               sorted(ds.load_schema()["$defs"]), sorted(ds.TYPES)),
    ]
    for i, c in enumerate(data["cases"]):
        doc = jcs.loads((repo / c["path"]).read_bytes()) if "path" in c else c["document"]
        got = ds.artifact_id(doc)
        out.append(_check(f"schema #{i}: {c['note']}", {"id": got, "published": ds.is_published(got)}, c["expect"]))
    return out


def _desired_declarations(path: Path) -> list[Result]:
    """desired.v1 §2.2: a resolved resource -> its declaration. An explicit
    state is read as its kind token, ``@state``."""
    from . import desired as ds

    out = []
    for i, c in enumerate(_load_json(path)["cases"]):
        r = c["resource"]
        kind = "@state" if r.get("explicit") and r["kind"] == "state" else r["kind"]
        out.append(_check(f"declarations #{i}: {c['note']}",
                          ds.declare(kind, r["template"], r["annotations"], r["params"]).expect(), c["expect"]))
    return out


def _desired_bindings(path: Path) -> list[Result]:
    """desired.v1 §2.3: a declared resource, its author's interface, the
    target's address and its requirement as its descriptor lists it -> the
    binding: the slice's key, the kill switch, or its findings."""
    from . import desired as ds

    out: list[Result] = []
    for i, c in enumerate(_load_json(path)["cases"]):
        r = c["resource"]
        decl = ds.declare(r["kind"], r["template"], r["annotations"], r["params"])
        if decl.declaration != ds.DECLARED:
            out.append((f"bindings #{i}: {c['note']}", False, f"the resource is not declared: {decl.expect()}"))
            continue
        got = ds.bind(decl, r["template"], bool(r.get("explicit")), c["iface"], c["target"], c["requirement"],
                      also=c.get("also") or ())
        out.append(_check(f"bindings #{i}: {c['note']}", got.expect(), c["expect"]))
    return out


def _desired_order(path: Path) -> list[Result]:
    """desired.v1 §2.6 to §2.10: one role's steps -> what each does, and the
    report after the last. Each report the machine builds must also read
    with no finding (§2.10: the findings are "each one the state machine
    never produces")."""
    from . import desired as ds

    out: list[Result] = []
    data = _load_json(path)
    for i, c in enumerate(data["cases"]):
        name = f"order #{i}: {c['note']}"
        if len(c["steps"]) != len(c["expect"]["steps"]):
            out.append((name, False, "the case's steps and expected words differ in number"))
            continue
        words, report = ds.run_steps(c["key"], c["steps"], data.get("schema"))
        out.append(_check(name, {"steps": words, "report": report}, c["expect"]))
        out.append(_check(f"{name} (its report reads, with no finding)", ds.read_report(report).expect(),
                          {"readable": True, "errors": []}))
    return out


def _desired_reports(path: Path) -> list[Result]:
    """desired.v1 §2.4, §2.10: a report, decoded or as its bytes' text ->
    whether a reader reads it, and its findings in order."""
    from . import desired as ds

    out = []
    for i, c in enumerate(_load_json(path)["cases"]):
        if "text" in c:
            got = ds.read_report_bytes(c["text"].encode("utf-8", "surrogatepass"))
        else:
            got = ds.read_report(c["document"])
        out.append(_check(f"reports #{i}: {c['note']}", got.expect(), c["expect"]))
    return out


def _desired_slices(path: Path) -> list[Result]:
    """desired.v1 §2.10, §2.17 (0.5): a tool's reading of one role before
    §2.17 answers -> its candidate slices, where they came from, §2.3's and
    §2.2's problems, and which reply is the report
    (zk2py.desired.take_slice, which_report)."""
    from . import desired as ds

    out: list[Result] = []
    for i, c in enumerate(_load_json(path)["cases"]):
        rd = c["reading"]
        author = rd.get("author") or {}
        taken = ds.take_slice(rd["target"], rd["iface"], rd["requirement"], author.get("named"),
                              author.get("held") or [])
        replies = [(x["key"], x["text"].encode("utf-8", "surrogatepass")) for x in rd.get("replies") or []]
        found = ds.find_report(taken, replies)
        got = {"slice": list(taken.candidates), "from": taken.source, "binding": list(taken.binding),
               "declaration": list(taken.declaration), "report": found.expect()}
        out.append(_check(f"slices #{i}: {c['note']}", got, c["expect"]))
    return out


def _desired_conclusions(path: Path) -> list[Result]:
    """desired.v1 §2.17, §5: one reading of one target's role -> §5's four
    questions, each an answer and a reason."""
    from . import desired as ds

    out = []
    for i, c in enumerate(_load_json(path)["cases"]):
        got = {q: a.pair() for q, a in ds.conclude(ds.reading_of(c["reading"])).items()}
        out.append(_check(f"conclusions #{i}: {c['note']}", got, c["expect"]))
    return out


def _redundancy_declarations(path: Path) -> list[Result]:
    """redundancy.v1 §2.2: a contract's uses, each resource's resolved
    annotations, and the tokenless set -> its declaration."""
    from . import redundancy as rd

    out = []
    for i, c in enumerate(_load_json(path)["cases"]):
        got = rd.declaration(c["contract"]["uses"], [r["annotations"] for r in c["contract"]["resources"]],
                             c["tokenless"]).as_json()
        out.append(_check(f"declarations #{i}: {c['note']}", got, c["expect"]))
    return out


def _redundancy_descriptors(path: Path) -> list[Result]:
    """redundancy.v1 §2.4, §2.14, core §3.3 (0.39): one interface entry and
    what the reader knows of its governance -> its mark and findings."""
    from . import redundancy as rd

    out = []
    for i, c in enumerate(_load_json(path)["cases"]):
        m, findings = rd.mark(c["entry"], c["governance"])
        out.append(_check(f"descriptors #{i}: {c['note']}", {"mark": m, "findings": findings}, c["expect"]))
    return out


def _redundancy_elections(path: Path) -> list[Result]:
    """redundancy.v1 §2.5, §2.11: one read from own's view -> what it does."""
    from . import redundancy as rd

    out = []
    for i, c in enumerate(_load_json(path)["cases"]):
        got = rd.elect(c["own"], rd.Read.of(c["read"]), c.get("election", rd.CLAIM)).as_json()
        out.append(_check(f"elections #{i}: {c['note']}", got, c["expect"]))
    return out


def _redundancy_heals(path: Path) -> list[Result]:
    """redundancy.v1 §2.9: an incumbent, its election, and two reads'
    holders -> whether it steps down."""
    from . import redundancy as rd

    out = []
    for i, c in enumerate(_load_json(path)["cases"]):
        down, what = rd.heal(c["own"], c["election"], c["first"]["holders"], c["second"]["holders"])
        got = {"step_down": True, "lower": what} if down else {"step_down": False, "why": what}
        out.append(_check(f"heals #{i}: {c['note']}", got, c["expect"]))
    return out


def _redundancy_polls(path: Path) -> list[Result]:
    """redundancy.v1 §2.7 (0.3, F-215): a poll's reads in order -> each read's
    appearances and withdrawals against the last's
    (zk2py.redundancy.poll_read)."""
    from . import redundancy as rd

    out: list[Result] = []
    for i, c in enumerate(_load_json(path)["cases"]):
        last: set[str] | None = None
        got = []
        for r in c["reads"]:
            cmp, last = rd.poll_read(last, r["holders"], bool(r["complete"]))
            got.append(cmp.as_json())
        out.append(_check(f"polls #{i}: {c['note']}", got, c["expect"]))
    return out


def _redundancy_lifecycle(path: Path) -> list[Result]:
    """redundancy.v1 §2.5 to §2.11: one candidate's events -> each step,
    and its role after the last."""
    from . import redundancy as rd

    out = []
    for i, c in enumerate(_load_json(path)["cases"]):
        m = rd.Machine(c["own"], c["election"])
        steps = [m.step(e) for e in c["events"]]
        out.append(_check(f"lifecycle #{i}: {c['note']}", {"steps": steps, "role": m.role}, c["expect"]))
    return out


def _redundancy_conclusions(path: Path) -> list[Result]:
    """redundancy.v1 §2.15, §5: two reads of one address -> §5's four
    questions."""
    from . import redundancy as rd

    out = []
    for i, c in enumerate(_load_json(path)["cases"]):
        got = rd.conclude(rd.Reading.of(c["reading"]))
        got = {"one_incumbent": got["one_incumbent"].as_json(), "vacancy": got["vacancy"].as_json(),
               "standbys": got["standbys"].as_json(), "twins": {k: v.as_json() for k, v in got["twins"].items()}}
        out.append(_check(f"conclusions #{i}: {c['note']}", got, c["expect"]))
    return out


def _timing_declarations(path: Path) -> list[Result]:
    """timing.v1 §2.2, §2.3, §2.5: a resolved resource (or its layers), or a
    requirement -> its declaration."""
    from . import timing as tm

    return [_check(f"declarations #{i}: {c['note']}", tm.declaration_of(c).expect(), c["expect"])
            for i, c in enumerate(_load_json(path)["cases"])]


def _timing_deadlines(path: Path) -> list[Result]:
    """timing.v1 §2.5: one reader's tracker for one provider and resource,
    driven by its events -> its reports, what it holds, its next due."""
    from . import timing as tm

    out = []
    for i, c in enumerate(_load_json(path)["cases"]):
        epoch = bool(c.get("epoch", False))
        if "resource" in c:
            # text 0.2, F-259: the resource in place of a deadline; a tracker
            # only where the declaration judges one
            t = tm.tracker_for(tm.declaration_of({"resource": c["resource"]}), c["bound"], c["known"], epoch)
        else:
            t = tm.Tracker(c["deadline_ms"], c["bound"], c["known"], epoch=epoch)
        if t is None:
            got: dict[str, Any] = {"transitions": [], "held": [], "next_due_ms": None}
        else:
            for e in c["events"]:
                t.event(e)
            due = t.next_due()
            got = {"transitions": [r.expect() for r in t.reports], "held": t.held(),
                   "next_due_ms": None if due is None else tm.ns_to_ms(due)}
            if t.window_end is not None:
                # text 0.5 (F-278, F-280): a case that ends a tool's window
                # states each member's longest unbroken stretch
                got["held_ms"] = {k: tm.ns_to_ms(v) for k, v in t.held_ns().items()}
        if "held_ms" in c["expect"] and "held_ms" not in got:
            got["held_ms"] = None
        if "tracker" in c["expect"]:
            got["tracker"] = t is not None
        out.append(_check(f"deadlines #{i}: {c['note']}", got, c["expect"]))
    return out


def _timing_lifespans(path: Path) -> list[Result]:
    """timing.v1 §2.6: one member's samples in arrival order within one
    reading -> each sample's lifespan judgement."""
    from . import timing as tm

    out = []
    for i, c in enumerate(_load_json(path)["cases"]):
        decl = tm.declaration_of(c)
        stamps = [None if s["stamp"] is None else tm.Stamp(tm.ms_to_ns(s["stamp"]["age_ms"]), s["stamp"]["id"])
                  for s in c["samples"]]
        got = [v.pair() for v in tm.judge_lifespans(decl, c["epsilon_ms"], c["owners"], stamps)]
        out.append(_check(f"lifespans #{i}: {c['note']}", got, c["expect"]))
    return out


def _timing_periods(path: Path) -> list[Result]:
    """timing.v1 §2.7: one member's arrivals over a tool's window -> whether
    the period was kept."""
    from . import timing as tm

    out = []
    for i, c in enumerate(_load_json(path)["cases"]):
        w = c["window"]
        got = tm.period(c["period_ms"], tm.ms_to_ns(w["from_ms"]), tm.ms_to_ns(w["to_ms"]), bool(c["known"]),
                        None if c["ended_ms"] is None else tm.ms_to_ns(c["ended_ms"]),
                        [tm.ms_to_ns(a) for a in c["arrivals_ms"]]).expect()
        out.append(_check(f"periods #{i}: {c['note']}", got, c["expect"]))
    return out


def _timing_conclusions(path: Path) -> list[Result]:
    """timing.v1 §2.7, §5: a tool's reading of one member over one window ->
    §5's three questions."""
    from . import timing as tm

    out = []
    for i, c in enumerate(_load_json(path)["cases"]):
        if "rollup" in c:
            # text 0.2, F-264: one question over a resource or a run; since
            # text 0.5 (F-279) each member's key and reason, and the roll-up's
            # reason, the first member's giving its answer in byte order
            r = c["rollup"]
            if "members" in r:
                a = tm.rollup(r["question"], [(m["key"], tm.Answer(m["answer"], m["reason"])) for m in r["members"]])
                got: dict[str, Any] = {"answer": a.answer, "reason": a.reason}
            else:
                got = {"answer": tm.rollup_answer(r["question"], r["answers"])}
        else:
            got = {q: a.pair() for q, a in tm.conclude(tm.declaration_of(c), tm.Reading.of(c["reading"])).items()}
        out.append(_check(f"conclusions #{i}: {c['note']}", got, c["expect"]))
    return out


def _arbitration_declarations(path: Path) -> list[Result]:
    """arbitration.v1 §2.2, §2.3: one role and the revision of its required
    interface held -> its declaration."""
    from . import arbitration as ab

    return [_check(f"declarations #{i}: {c['note']}", ab.declaration_of(c).expect(), c["expect"])
            for i, c in enumerate(_load_json(path)["cases"])]


def _arbitration_rankings(path: Path) -> list[Result]:
    """arbitration.v1 §2.4: a binding and the addresses heard -> the
    providers in rank order, the unbound, the wildcards and the shadowed."""
    from . import arbitration as ab

    return [_check(f"rankings #{i}: {c['note']}", ab.ranking(c["bindings"], c["heard"]), c["expect"])
            for i, c in enumerate(_load_json(path)["cases"])]


def _arbitration_arbitrations(path: Path) -> list[Result]:
    """arbitration.v1 §2.5 to §2.11: one consumer's role and its events in
    order -> what it decides after each."""
    from . import arbitration as ab

    out = []
    for i, c in enumerate(_load_json(path)["cases"]):
        a = ab.arbiter_of(c.get("role", c))
        got = [a.event(e).expect() for e in c["events"]]
        out.append(_check(f"arbitrations #{i}: {c['note']}", got, c["expect"]))
    return out


def _arbitration_conclusions(path: Path) -> list[Result]:
    """arbitration.v1 §2.14, §5: a tool's reading of one consumer's role ->
    §5's three questions."""
    from . import arbitration as ab

    out = []
    for i, c in enumerate(_load_json(path)["cases"]):
        got = {q: a.expect() for q, a in ab.conclude(*ab.reading_of(c["reading"])).items()}
        out.append(_check(f"conclusions #{i}: {c['note']}", got, c["expect"]))
    return out


#: Each profile's fixtures, by file name. profiles/README.md: "An unknown
#: file fails the test, so a fixture cannot land unrun. A README.md there
#: is prose."
PROFILE_FIXTURES: dict[str, dict[str, Callable[[Path], list[Result]]]] = {
    "hostid": {"vectors.json": _hostid_vectors, "shapes.json": _hostid_shapes},
    "freshness": {"horizons.json": _freshness_horizons, "judgements.json": _freshness_judgements,
                  "clock-trust.json": _freshness_clock_trust},
    "health": {"judgements.json": _health_judgements, "rollups.json": _health_rollups,
               "codes.json": _health_codes, "clock.json": _health_clock},
    "config": {"validation.json": _config_validation, "window.json": _config_window,
               "lifecycle.json": _config_lifecycle, "desired.json": _config_desired,
               "redaction.json": _config_redaction},
    "link": {"exposure.json": _link_exposure, "plan.json": _link_plan, "lints.json": _link_lints},
    "alarms": {"keys.json": _alarms_keys, "documents.json": _alarms_documents,
               "transitions.json": _alarms_transitions, "conclusions.json": _alarms_conclusions},
    "telemetry": {"schema.json": _telemetry_schema, "declarations.json": _telemetry_declarations,
                  "samples.json": _telemetry_samples, "continuity.json": _telemetry_continuity},
    "desired": {"schema.json": _desired_schema, "declarations.json": _desired_declarations,
                "bindings.json": _desired_bindings, "order.json": _desired_order, "reports.json": _desired_reports,
                "conclusions.json": _desired_conclusions, "slices.json": _desired_slices},
    "redundancy": {"declarations.json": _redundancy_declarations, "descriptors.json": _redundancy_descriptors,
                   "elections.json": _redundancy_elections, "heals.json": _redundancy_heals,
                   "lifecycle.json": _redundancy_lifecycle, "conclusions.json": _redundancy_conclusions,
                   "polls.json": _redundancy_polls},
    "timing": {"declarations.json": _timing_declarations, "deadlines.json": _timing_deadlines,
               "lifespans.json": _timing_lifespans, "periods.json": _timing_periods,
               "conclusions.json": _timing_conclusions},
    "arbitration": {"declarations.json": _arbitration_declarations, "rankings.json": _arbitration_rankings,
                    "arbitrations.json": _arbitration_arbitrations, "conclusions.json": _arbitration_conclusions},
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
    """Observer §5.3 (0.24): a kind, a bound, a retention and one reading -> the
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
    """Observer §5.3 (0.24): an event's rate and a window's occurrences -> the
    rate verdict and its reason."""
    from . import budget
    from .freshness import seconds_to_ns

    out: list[Result] = []
    for c in _load_json(path)["cases"]:
        r = c["reading"]
        members = {m: [seconds_to_ns(x) for x in ts] for m, ts in r["members"].items()}
        got = budget.rate(c["rate"], members, seconds_to_ns(r["listened_s"]), r["lossless"], r["present"])
        out.append(_check(f"rate: {c['note']}", got.pair(), c["expect"]))
    return out


def _budget_lossless(path: Path) -> list[Result]:
    """Observer §5.2 (0.25): the declared QoS, the reader's own loss and miss
    detection -> whether a window is shown to have lost no delivery."""
    from . import budget

    data = _load_json(path)
    out: list[Result] = []
    for c in data["cases"]:
        q, m, s = c["qos"], c["miss_detection"], c["reader_stall"]
        ok, reason = budget.lossless(q["reliability"], q["congestion"], c["reader_lost"],
                                     m["declared"], m["used"], m["misses"], stall_gap_ms=s["gap_ms"],
                                     stall_bound_ms=s["bound_ms"], transport_closed=s["transport_closed"])
        out.append(_check(f"lossless: {c['note']}", {"lossless": ok, "reason": reason}, c["expect"]))
    for c in data.get("stall_bounds", []):
        out.append(_check(f"lossless: stall_bounds: {c['note']}",
                          budget.stall_bound(c["wait_before_close_ms"], c["lease_ms"], c["keep_alive"]),
                          c["expect_ms"]))
    return out


#: zk2py's own cases, each naming what it reads: an open finding's reading,
#: so that a later text's fixture can overturn it visibly, or a rule a text
#: states, beyond its fixture's cases. Not a fixture family. Today:
#: alarms.v1 §2.4's whitespace, F-134 at 0.2, stated by 0.3 ("RFC 8259's ws,
#: before the value and after it … Any other character after the value, a
#: form feed or a no-break space included, is text after it"), whose four
#: documents.json cases these extend.
_F134_DOC = b'{"timestamp":1760950000000,"rule":"link_down","severity":"warning","state":"firing","summary":"s"}'
_READABLE = {"readable": True, "errors": [], "warnings": []}
_NOT_JSON = {"readable": False, "errors": ["json"], "warnings": []}
READINGS = [
    ("alarms.v1 0.3 §2.4 (F-134)", "a line feed after the document", _F134_DOC + b"\n", _READABLE),
    ("alarms.v1 0.3 §2.4 (F-134)", "a carriage return and a line feed after it", _F134_DOC + b"\r\n", _READABLE),
    ("alarms.v1 0.3 §2.4 (F-134)", "RFC 8259's four whitespace bytes before and after it", b" \t\r\n" + _F134_DOC + b" \t\r\n",
     _READABLE),
    ("alarms.v1 0.3 §2.4 (F-134)", "a no-break space after it (U+00A0)", _F134_DOC + "\u00a0".encode(), _NOT_JSON),
    ("alarms.v1 0.3 §2.4 (F-134)", "a line separator after it (U+2028)", _F134_DOC + "\u2028".encode(), _NOT_JSON),
    ("alarms.v1 0.3 §2.4 (F-134)", "a vertical tab after it", _F134_DOC + b"\x0b", _NOT_JSON),
    ("alarms.v1 0.3 §2.4 (F-134)", "a form feed after it", _F134_DOC + b"\x0c", _NOT_JSON),
    ("alarms.v1 0.3 §2.4 (F-134)", "a NUL after it", _F134_DOC + b"\x00", _NOT_JSON),
    ("alarms.v1 0.3 §2.4 (F-134)", "text after it", _F134_DOC + b"x", _NOT_JSON),
    ("alarms.v1 0.3 §2.4 (F-134)", "a second value after a line feed", _F134_DOC + b"\n{}", _NOT_JSON),
    ("alarms.v1 0.3 §2.4 (F-134)", "a byte-order mark before it", b"\xef\xbb\xbf" + _F134_DOC, _NOT_JSON),
]


def _window_cases() -> list[tuple[str, Any, Any]]:
    """alarms.v1 0.3 §2.15: a loss or a gap breaks the window until the
    first complete GET after it, asked strictly after the break was seen (a
    stall is seen by the tick that ends it). The window runs from 1 s (the
    declaration plus the reply timeout) to 100 s; instants in seconds."""
    from . import alarms

    def w(losses: list[float], gaps: list[float], gets: list[float]) -> dict[str, Any]:
        ns = 1_000_000_000
        x = alarms.window_span(1 * ns, 100 * ns, [int(t * ns) for t in losses], [int(t * ns) for t in gaps],
                               [int(t * ns) for t in gets])
        return {"listened_s": x.listened_s, "lossless": x.lossless, "present_throughout": x.present_throughout}

    label = "alarms.v1 0.3 §2.15"
    return [
        (f"{label}: a stall seen at 10 s, a GET asked at 9.5 s, inside it: not healed",
         w([10], [], [9.5]), {"listened_s": 99.0, "lossless": False, "present_throughout": True}),
        (f"{label}: a GET asked at the instant the stall was seen: not after it, not healed",
         w([10], [], [10]), {"listened_s": 99.0, "lossless": False, "present_throughout": True}),
        (f"{label}: a GET asked at 10.5 s heals it, and the window runs from it",
         w([10], [], [10.5]), {"listened_s": 89.5, "lossless": True, "present_throughout": True}),
        (f"{label}: a gap at 20 s, no GET after it",
         w([], [20], [5]), {"listened_s": 99.0, "lossless": True, "present_throughout": False}),
        (f"{label}: a gap at 20 s, a GET at 21 s heals it",
         w([], [20], [21]), {"listened_s": 79.0, "lossless": True, "present_throughout": True}),
        (f"{label}: a loss at 10 s healed at 15 s, a gap at 20 s not",
         w([10], [20], [15]), {"listened_s": 85.0, "lossless": True, "present_throughout": False}),
    ]


def _scope_cases() -> list[tuple[str, Any, Any]]:
    """alarms.v1 0.5 §2.15 "Whole" and "An archive's read plays no part in
    it" (F-137): a scope found by a complete presence read and an archive's
    read cut short is whole; one whose presence read was not whole is not."""
    from . import alarms

    quiet = alarms.OwnerRead(alarms.ESTABLISHED)
    label = "alarms.v1 0.5 §2.15 (F-137)"
    presence_whole = alarms.ScopeRead(["lab/a"], whole=True)
    archive_short = alarms.ScopeRead(["lab/svc"], whole=True, archive_short=True, skipped=1)
    both = presence_whole.union(archive_short)
    not_whole = alarms.ScopeRead(["lab/a"], whole=False).union(archive_short)
    return [
        (f"{label}: a complete presence read and an archive's read cut short: whole, the archive said short",
         [both.whole, both.archive_short,
          alarms.any_firing("warning", [quiet, alarms.OwnerRead(alarms.NOT_ASKED)], both.whole)["reason"]],
         [True, True, "none_firing"]),
        (f"{label}: a presence read possibly incomplete: not whole, scope_incomplete",
         [not_whole.whole, alarms.any_firing("warning", [quiet], not_whole.whole)["reason"]],
         [False, "scope_incomplete"]),
    ]


def _reporter_transitions(root: Path) -> list[tuple[str, Any, Any]]:
    """alarms.v1 0.4 §2.16 over transitions.json: each case's samples read
    by a present, listed owner's fresh subscription, judged (§2.14), and
    reported. What must be reported is read from the fixture's transition
    names by §2.16's own sentences: the first raise of a firing alarm, an
    escalation, a resolve of an alarm reported firing, an unreadable put
    after a report; never a refresh, a confirmation, a raise of an alarm
    already reported firing at its severity, nor a retire."""
    from . import alarms, freshness as fr

    out = []
    path = root.parent / "profiles" / "alarms" / "conformance" / "transitions.json"
    for i, c in enumerate(_load_json(path)["cases"]):
        rep, track = alarms.ChangeReporter(), alarms.Transitions()
        got, want = [], []
        reported: tuple[str, str | None] | None = None  # (side, severity) last reported
        for smp in c["samples"]:
            t = track.step(alarms.DELETE if smp == "delete" else smp["put"])
            doc = track.held
            if smp == "delete":
                latest = "deleted"
            elif doc is None:
                latest = "undecodable"
            else:
                latest = doc.state
            r = alarms.Reading("present", {"lists": True, "token": True}, None,
                               [fr.Subscription(1000 * fr.NS, True, fr.Delivery(
                                   "delete" if smp == "delete" else "put", 1 * fr.NS))], latest)
            a = alarms.judge(r)
            sev = None if doc is None else doc.severity
            got.append([x.answer for x in rep.update({"k": (a, sev, t)})])
            # §2.16, from the transition's name
            # refreshed and confirmed name a resolved document put again too
            # (§2.6's table), so their side is the document's
            side = {"raised": alarms.YES, "escalated": alarms.YES, "resolved": alarms.NO,
                    "retired": alarms.NO}.get(t, "neither")
            if t in ("refreshed", "confirmed") and doc is not None:
                side = alarms.YES if doc.firing else alarms.NO
            now = (side, sev if side == alarms.YES else None)
            if reported is None:
                must = side == alarms.YES  # a key's first answer: yes is reported, no and neither not
            else:
                must = now != reported
            if must:
                reported = now  # against the last report (0.6)
            want.append([a.answer] if must else [])
        out.append((f"alarms.v1 0.4 §2.16 over transitions.json #{i}: {c['note']}", got, want))
    return out


def _reporter_conclusions(root: Path) -> list[tuple[str, Any, Any]]:
    """alarms.v1 0.4 §2.16 over conclusions.json's readings: an alarm last
    reported firing at critical, then the case's reading. The report is the
    case's first answer and reason, "the answer after the change", or none
    when the answer is yes at critical. A loss (incomplete) is reported once
    for the scope instead."""
    from . import alarms

    out = []
    path = root.parent / "profiles" / "alarms" / "conformance" / "conclusions.json"
    for i, c in enumerate(_load_json(path)["cases"]):
        if c["question"] != "reading":
            continue
        rep = alarms.ChangeReporter()
        rep.update({"k": (alarms.Answer(alarms.YES, "firing"), "critical", "raised")})
        e = c["expect"]["firing"]
        got = [(x.key, x.answer, x.reason) for x in rep.update({"k": (alarms.Answer(e["answer"], e["reason"]),
                                                                       "critical", None)})]
        if e["reason"] == "incomplete":
            want = [(None, alarms.UNOBSERVABLE, "incomplete")]
        elif e["answer"] == alarms.YES:
            want = []
        else:
            want = [("k", e["answer"], e["reason"])]
        out.append((f"alarms.v1 0.4 §2.16 over conclusions.json #{i}: {c['note']}", got, want))
    return out


def _reporter_loss() -> list[tuple[str, Any, Any]]:
    """alarms.v1 0.4 §2.16, "A loss is the reader's": reported once for the
    scope while any key is unhealed, "observable again" when the last heals,
    no key's answer the loss made unobservable reported, each key reported
    again against its last report once healed (F-139); a key gone from a
    whole scope's readings reported, never dropped silently."""
    from . import alarms

    yes, inc = alarms.Answer(alarms.YES, "firing"), alarms.Answer(alarms.UNOBSERVABLE, "incomplete")
    no = alarms.Answer(alarms.NO, "not_held")

    def run(steps: list[tuple[dict[str, Any], bool]]) -> list[list[tuple[Any, ...]]]:
        rep = alarms.ChangeReporter()
        return [[(x.key, x.answer, x.reason) for x in rep.update(st, partial=partial)] for st, partial in steps]

    a_yes, b_yes, a_inc, b_inc = ("a", (yes, "warning", None)), ("b", (yes, "critical", None)), \
        ("a", (inc, "warning", None)), ("b", (inc, "critical", None))
    start = [("a", "yes", "firing"), ("b", "yes", "firing")]
    loss, healed = (None, "unobservable", "incomplete"), (None, "observable", "healed")
    label = "alarms.v1 0.4 §2.16"
    return [
        (f"{label}: a loss reported once, the keys kept, each reported again once healed (F-139)",
         run([(dict([a_yes, b_yes]), False), (dict([a_inc, b_inc]), False), (dict([a_inc, b_inc]), False),
              ({"a": (yes, "warning", None), "b": (no, None, None)}, False)]),
         [start, [loss], [], [("b", "no", "not_held"), healed]]),
        (f"{label}: a second loss while the first is unhealed is not reported again; healed once, at the last key",
         run([(dict([a_yes, b_yes]), False), (dict([a_inc, b_yes]), False), (dict([a_inc, b_inc]), False),
              (dict([a_yes, b_inc]), False), (dict([a_yes, b_yes]), False)]),
         [start, [loss], [], [], [healed]]),
        (f"{label}: a key gone from the scope's readings during a loss is reported, never dropped silently",
         run([(dict([a_yes, b_yes]), False), (dict([a_inc, b_inc]), False), (dict([a_yes]), False)]),
         [start, [loss], [("b", "unobservable", "not_read"), healed]]),
        (f"{label}: a partial reading (one owner's keys) heals only its own keys",
         run([(dict([a_yes, b_yes]), False), (dict([a_inc, b_inc]), False), (dict([a_yes]), True),
              (dict([b_yes]), True)]),
         [start, [loss], [], [healed]]),
    ]


def _first_neither_cases() -> list[tuple[str, Any, Any]]:
    """alarms.v1 0.8 §2.16, "A key's first answer": every neither reason
    the first question gives, one reading per reason, each checked on its
    own (the answer neither yes nor no, the reason the one intended, the
    list the one 0.8 puts it on); then that the table's reader's-sight
    reasons are exactly 0.8's ten and that it holds twenty distinct
    reasons. freshness.v1's `silent` is on the reader's-sight list since
    0.8 (F-141)."""
    from . import alarms, freshness as fr

    lists = {"lists": True, "token": True}

    def sub(age: Any = 10, complete: bool = True, listened: int = 1200) -> Any:
        return fr.Subscription(listened * fr.NS, complete, None if age is None else fr.Delivery("put", age * fr.NS))

    def get(reply: Any = "put", age: Any = 10, trusted: bool = True) -> Any:
        r = None if reply is None else fr.Reply("put", None if age is None else int(age * fr.NS))
        return fr.Get(r, trusted, fr.DEFAULT_DELTA_NS)

    R = alarms.Reading
    std, invalid = alarms.HORIZON, fr.Horizon("invalid")
    sight, other = "reader_sight", "other"
    # (reason intended, reading, horizon, the list 0.8 puts it on)
    table: list[tuple[str, Any, Any, str]] = [
        ("absent", R("absent"), std, other),
        ("presence_incomplete", R("incomplete"), std, sight),
        ("face_closed", R("across_face", face={"reachable": False}), std, other),
        ("no_descriptor", R("present"), std, sight),
        ("not_listed", R("present", {"lists": False}), std, other),
        ("last_known", R("present", lists, observations=[fr.Archive()], latest="firing"), std, other),
        ("incomplete", R("present", lists, observations=[sub(complete=False)], latest="firing"), std, sight),
        ("not_read", R("present", lists, observations=[get(None)]), std, sight),
        ("nothing_crossed", R("across_face", face={"reachable": True}, observations=[get(None)]), std, sight),
        ("previous_instance", R("present", lists, observations=[sub()], latest="firing", previous_instance=True),
         std, other),
        ("undecodable", R("present", lists, observations=[sub()], latest="undecodable"), std, other),
        ("stale", R("present", lists, observations=[sub(age=1000)], latest="firing"), std, other),
        ("no_observation", R("present", lists, latest="firing"), std, sight),
        ("silent", R("present", lists, observations=[get(None)], latest="firing"), std, sight),
        ("no_stamp", R("present", lists, observations=[get(age=None)], latest="firing"), std, other),
        ("clock_untrusted", R("present", lists, observations=[get(trusted=False)], latest="firing"), std, sight),
        ("clock_disagrees", R("present", lists, observations=[get(age=-5)], latest="firing"), std, other),
        ("near_horizon", R("present", lists, observations=[get(age=899.8)], latest="firing"), std, sight),
        ("listened_too_short", R("present", lists, observations=[sub(age=None, listened=10)], latest="firing"),
         std, sight),
        ("not_a_horizon", R("present", lists, observations=[sub()], latest="firing"), invalid, other),
    ]
    label = "alarms.v1 0.8 §2.16, a key's first answer"
    out: list[tuple[str, Any, Any]] = []
    for want, reading, horizon, want_list in table:
        a = alarms.judge(reading, alarms.Q_FIRING, horizon)
        got_list = alarms.first_neither_list(a.reason) if a.answer not in (alarms.YES, alarms.NO) else None
        out.append((f"{label}: a reading for `{want}` gives neither, `{want}`, on the {want_list} list",
                    {"answer_neither": a.answer not in (alarms.YES, alarms.NO), "reason": a.reason,
                     "list": got_list},
                    {"answer_neither": True, "reason": want, "list": want_list}))
    reasons = [t[0] for t in table]
    out.append((f"{label}: the table holds twenty distinct reasons, and its reader's-sight ones are 0.8's ten",
                {"distinct": len(set(reasons)), "sight": sorted(r for r, _, _, lst in table if lst == sight)},
                {"distinct": 20, "sight": sorted(alarms.READER_SIGHT)}))
    return out


def _reporter_gone() -> list[tuple[str, Any, Any]]:
    """alarms.v1 0.6 §2.16 and a key gone from the whole scope's readings:
    a key with no report (a first neither of the key's own, not reported)
    is forgotten, so a reappearance is a first answer; a key with a report
    keeps it, so a reappearance is compared with it (review of PR #814)."""
    from . import alarms

    yes, no = alarms.Answer(alarms.YES, "firing"), alarms.Answer(alarms.NO, "resolved")
    stale = alarms.Answer(alarms.UNOBSERVABLE, "stale")
    other = {"o": (yes, "info", None)}

    def run(*steps: Any) -> list[list[tuple[Any, ...]]]:
        rep = alarms.ChangeReporter()
        out = []
        for st in steps:
            feed = dict(other)
            if st is not None:
                feed["k"] = (st, "warning", None)
            out.append([(x.key, x.answer, x.reason) for x in rep.update(feed) if x.key != "o"])
        return out

    label = "alarms.v1 0.6 §2.16, a key gone from the scope"
    gone = ("k", "unobservable", "not_read")
    return [
        (f"{label}: last reported yes, gone, back at no: gone reported, and the no clears it",
         run(yes, None, no), [[("k", "yes", "firing")], [gone], [("k", "no", "resolved")]]),
        (f"{label}: last reported neither (stale after yes), gone, back at no: the no clears it",
         run(yes, stale, None, no), [[("k", "yes", "firing")], [("k", "unobservable", "stale")], [],
                                     [("k", "no", "resolved")]]),
        (f"{label}: a first neither of the key's own, not reported, gone, back stale: nothing; then yes",
         run(stale, None, stale, yes), [[], [], [], [("k", "yes", "firing")]]),
        (f"{label}: no report, gone, back at no: a first answer, not reported",
         run(no, None, no), [[], [], []]),
    ]


def _presence_dating_cases() -> list[tuple[str, Any, Any]]:
    """alarms.v1 0.6 §2.15 "Whole": a presence read asked at 10 s; a loss at
    or after its ask counts against it, a loss before it does not."""
    from . import alarms

    label = "alarms.v1 0.6 §2.15, a presence read dated by its ask"
    return [(f"{label}: {note}", alarms.presence_read_whole(complete, 10, losses), want) for note, complete, losses,
            want in [("a loss at 9 s, before the ask: whole", True, [9], True),
                     ("a loss at 10 s, the ask's instant: not whole", True, [10], False),
                     ("a loss at 12 s, its replies perhaps already in: not whole", True, [12], False),
                     ("no loss, but the read possibly incomplete: not whole", False, [], False)]] + [
        (f"{label}: a read with no ask on record cannot be dated: not whole",
         alarms.presence_read_whole(True, None, []), False)]


def _reporter_first_answers() -> list[tuple[str, Any, Any]]:
    """alarms.v1 0.6 §2.16, "A key's first answer … moves from no report":
    yes is reported; no is not; neither of the key's own MAY be (zk2py does
    not); neither of the reader's own sight is not. A key with no report
    keeps none until it reads yes, so a later neither of the reader's sight
    is no change either."""
    from . import alarms

    yes, no = alarms.Answer(alarms.YES, "firing"), alarms.Answer(alarms.NO, "resolved")
    stale, untrusted = alarms.Answer(alarms.UNOBSERVABLE, "stale"), alarms.Answer(alarms.UNOBSERVABLE,
                                                                                  "clock_untrusted")

    def run(*answers: Any) -> list[list[tuple[Any, ...]]]:
        rep = alarms.ChangeReporter()
        return [[(x.answer, x.reason) for x in rep.update({"k": (a, "warning", None)})] for a in answers]

    label = "alarms.v1 0.6 §2.16, a key's first answer"
    return [
        (f"{label}: no, not reported; then a clock not yet trusted (the reader's sight), still nothing",
         run(no, untrusted), [[], []]),
        (f"{label}: stale (the key's own), not reported (the MAY's other branch); then yes, reported",
         run(stale, yes), [[], [("yes", "firing")]]),
        (f"{label}: the reader's sight first, then yes: reported once it is yes",
         run(untrusted, yes), [[], [("yes", "firing")]]),
        (f"{label}: yes, reported; then the reader's sight: a move from the last report, reported",
         run(yes, untrusted), [[("yes", "firing")], [("unobservable", "clock_untrusted")]]),
    ]


def _left_out_cases() -> list[tuple[str, Any, Any]]:
    """alarms.v1 0.5 §2.14 step 5, "left the key out", as zk2py's reader
    decides it (alarms.is_left_out) from what its GET answered."""
    from . import alarms
    from .live import StateReply

    def seed(**replies: bool) -> Any:
        return alarms.SeedGet(100, True, {k: StateReply(k, deleted, b"", "", None, None) for k, deleted in
                                          replies.items()})

    label = "alarms.v1 0.5 §2.14 step 5"
    cases = [
        ("the key answered with a reply_del: not left out (it reads deleted)", seed(k=True), True, [], False),
        ("the key answered with a value: not left out", seed(k=False), True, [], False),
        ("the key not answered, nothing of it since: left out", seed(other=False), True, [50], True),
        ("the key not answered, but heard after the GET was asked: not left out", seed(other=False), True, [150],
         False),
        ("the owner's alarms not established: no GET left it out", seed(other=False), False, [], False),
        ("a GET short of its final reply: not left out", alarms.SeedGet(100, False, {}), True, [], False),
    ]
    return [(f"{label}: {note}", alarms.is_left_out(sd, est, "k", arr), want) for note, sd, est, arr, want in cases]


def _telemetry_adopters(root: Path) -> list[tuple[str, Any, Any]]:
    """telemetry.v1 Appendix B, read over the adopters' real contracts, so
    the contract reader (each type's root found with its $refs followed,
    core §9.8) is checked beside the fixtures, which give roots already
    followed:
    - zenoh-modem: "Bare values, each declared by telemetry.kind, and 23 by
      telemetry.unit (By, bit/s, ms, us, %, dB, dBm): every declaration
      valid"; ``sdu/dropped_total`` a bare counter with no epoch (§2.14);
    - ZenSight: "No resource declares a kind, so each Point says its own;
      netring's two units are declared", in each contract's revision 1.1
      and in the first revision's bundle, which carries the pre-profile id
      (§2.3: "a reader of an old bundle meets an old id");
    - tcgui: "no member is this profile's"."""
    from . import telemetry as tm
    from .contract import load_contract

    ex = root.parent.parent / "examples" / "zk2"
    spec = root.parent
    out: list[tuple[str, Any, Any]] = []
    label = "telemetry.v1 Appendix B"

    def of(path: Path) -> dict[str, Any]:
        return tm.contract_declarations(load_contract(path, spec_dir=spec))

    modem = of(ex / "zenoh-modem" / "modem.v3.toml")
    metric = {k: d for k, d in modem.items() if d.declaration != tm.NOT_THIS}
    out.append((f"{label}: zenoh-modem's 73 metric streams, each a bare declared counter or gauge",
                (len(metric), sorted({(k.split("/")[0], d.declaration, d.carrier, d.kind) for k, d in metric.items()
                                      if d.declaration == tm.DECLARED} - {("stream", tm.DECLARED, "bare", "counter"),
                                                                          ("stream", tm.DECLARED, "bare", "gauge")}),
                 sorted(k for k, d in metric.items() if d.declaration != tm.DECLARED)),
                (73, [], [])))
    units = sorted({d.unit for d in metric.values() if d.unit is not None})
    out.append((f"{label}: zenoh-modem's 23 units, of seven spellings, each valid",
                (sum(d.unit is not None for d in metric.values()), units),
                (23, sorted(["By", "bit/s", "ms", "us", "%", "dB", "dBm"]))))
    d = modem.get("stream/sdu/dropped_total")
    out.append((f"{label}: zenoh-modem's sdu/dropped_total, a bare counter with no epoch (§2.14)",
                None if d is None else (d.declaration, d.carrier, d.kind, d.epoch), (tm.DECLARED, "bare", "counter", None)))
    hist = ex / ".history"
    netring = {"stream/flow/reassembly_gap_bytes_total": "By", "stream/flow/red/{quantile}": "ms"}
    for iface in ("zs.sysinfo.v1", "zs.netring.v1", "zs.snmp.v1", "zs.parallax.v1"):
        revisions = [("revision 1.1", of(ex / "zensight" / f"{iface}.toml"))]
        revisions += [(f"published {b.name[:12]}…", tm.bundle_declarations(b.read_bytes())[1])
                      for b in sorted((hist / iface).glob("*.bundle.json"))]
        for tag, decls in revisions:
            metric = {k: x for k, x in decls.items() if x.declaration != tm.NOT_THIS}
            out.append((f"{label}: {iface} {tag}: every metric resource a stream Point declaring no kind, "
                        "its units as written",
                        (sorted({(x.declaration, x.carrier, x.kind, x.epoch) for x in metric.values()}),
                         all(k.startswith("stream/") for k in metric),
                         {k: x.unit for k, x in metric.items() if x.unit is not None}),
                        ([(tm.DECLARED, "point", None, "point")], True,
                         netring if iface == "zs.netring.v1" else {})))
    tc = {}
    for f in sorted((ex / "tcgui").glob("*.v*.toml")):
        tc.update({f"{f.stem}:{k}": x for k, x in of(f).items()})
    out.append((f"{label}: tcgui's composites and the rest, no member this profile's",
                sorted({x.declaration for x in tc.values()}), [tm.NOT_THIS]))
    return out


def _telemetry_copies() -> list[tuple[str, Any, Any]]:
    """telemetry.v1 §2.3: "A contract that lists a copy of the file, byte for
    byte or only JCS-equal, carries the profile's types: the id is the
    same", and a type of the same name in another document does not."""
    import json as _json

    from . import jcs, telemetry as tm

    doc = tm.load_schema()
    shuffled = _json.dumps(dict(reversed(list(doc.items()))), indent=3).encode()
    changed = _json.loads(_json.dumps(doc))
    changed["$defs"][tm.POINT]["description"] += " (a copy)"
    other = {"kind": "jsonschema", "name": tm.POINT, "schema": tm.artifact_id(changed), "root": None}
    label = "telemetry.v1 §2.3"
    return [
        (f"{label}: a JCS-equal copy (members reordered, re-indented) has the current id",
         tm.artifact_id(jcs.loads(shuffled)), tm.CURRENT),
        (f"{label}: a copy with one description changed has another id, not published",
         tm.is_published(other["schema"]), False),
        (f"{label}: its Point is not the Point: no key of the vocabulary, not a metric resource",
         tm.declare("stream", {}, other).declaration, tm.NOT_THIS),
    ]


def _telemetry_state_order() -> list[tuple[str, Any, Any]]:
    """telemetry.v1 §2.10, "in the order the reader received them", for a
    state member read as core §4.2's late joiner reads it: the live put of
    12 (stamp 2) arrives before the GET's reply of 10 (stamp 1), one
    instance alone throughout. Paired in receipt order, the two read a
    decrease within an established epoch, the finding; through the merge,
    which keeps the newer, the reply is no sample, and nothing pairs
    (F-148; 0.2's rule, scenarios.md §9)."""
    from . import seed, telemetry as tm

    live = tm.Read("counter", 12, None, None, 2_000_000_000, 1)
    reply = tm.Read("counter", 10, None, None, 1_000_000_000, 2)
    alone = tm.Presence(True, "a", "a")
    by_receipt = tm.continuity(live, reply, alone, None)
    kept = tm.kept_reads([(live, seed.Stamp(2 << 32, "1")), (reply, seed.Stamp(1 << 32, "1"))])
    label = "telemetry.v1 §2.10 with core §4.2 (F-148)"
    return [
        (f"{label}: paired in receipt order, a stale reply after a live put reads decreased, established",
         (by_receipt.continuity, by_receipt.epoch), ("decreased", "established")),
        (f"{label}: through the merge, the reply is not kept, and no pair is formed",
         [r.value for r in kept], [12]),
    ]


def _telemetry_nulls() -> list[tuple[str, Any, Any]]:
    """telemetry.v1 §2.6 over a `null` where a Point or its Value holds a
    member, F-149's reading and 0.2's rule: a member written `null` is present, and `null`
    is no value of any kind (§2.4). A Point's `value` written `null` is not
    an object, so the payload is not a Point; a Value's own `value` written
    `null` is of no kind's type, so `wrong_type`, for each of the six kinds,
    never a crash; a Histogram's member written `null` likewise."""
    from . import telemetry as tm

    point = tm.declare("stream", {}, {"kind": "jsonschema", "name": tm.POINT, "schema": tm.CURRENT, "root": None})

    def read(value: Any) -> Any:
        c = tm.conform(point, tm.Sample(value))
        return [c.conformance, list(c.reasons)]

    label = "telemetry.v1 §2.6, a member written null (F-149)"
    out = [(f"{label}: a Point's value written null is not a Point",
            read({"timestamp": 1, "value": None}), [tm.DOES_NOT, ["not_a_point"]])]
    out += [(f"{label}: a {kind} Value's own value written null is wrong_type",
             read({"timestamp": 1, "value": {"type": kind, "value": None}}), [tm.DOES_NOT, ["wrong_type"]])
            for kind in tm.KINDS]
    out += [(f"{label}: a histogram Value whose {member} is null is wrong_type",
             read({"timestamp": 1, "value": {"type": "histogram", "value": {
                 **{"buckets": [1.0], "counts": [1, 0], "count": 1, "sum": 0.5}, member: None}}}),
             [tm.DOES_NOT, ["wrong_type"]]) for member in ("buckets", "counts", "count", "sum")]
    out += [(f"{label}: a Point's {member} written null is present: {want}",
             read({"timestamp": 1, member: None, "value": {"type": "gauge", "value": 1}}), [tm.DOES_NOT, [want]])
            for member, want in (("epoch", "epoch_invalid"), ("unit", "not_a_point"), ("labels", "not_a_point"))]
    return out


def _telemetry_edges() -> list[tuple[str, Any, Any]]:
    """zk2py's readings of telemetry.v1 0.2's F-150 and F-151, which 0.3
    resolved as zk2py read them (samples.json cases 125 to 130 and 136 now
    pin both).
    - F-150, "finite" by rounding: an integer's text one past the largest
      double, and a float's text above it that rounds to it, are finite;
      the halfway point's text is +Infinity, undecodable.
    - F-151, an Epoch attachment read as strictly as a sample: a lone
      surrogate escape does not decode, so `epoch_invalid`."""
    import sys as _sys

    from . import telemetry as tm

    gauge = tm.declare("stream", {tm.KIND: "gauge"}, {"kind": "jsonschema", "name": "Level",
                                                      "schema": "sha256:" + "a" * 64, "root": {"type": "number"}})
    counter = tm.declare("stream", {tm.KIND: "counter"},
                         {"kind": "jsonschema", "name": "Count", "schema": "sha256:" + "a" * 64,
                          "root": {"type": "integer"}},
                         {"kind": "jsonschema", "name": tm.EPOCH, "schema": tm.CURRENT, "root": None})

    def text(t: str) -> Any:
        try:
            c = tm.conform(gauge, tm.Sample(tm.decode_json(t.encode())))
        except tm.Undecodable:
            c = tm.conform(gauge, tm.Sample(undecodable=True))
        return [c.conformance, list(c.reasons)]

    def attached(t: str) -> Any:
        try:
            c = tm.conform(counter, tm.Sample(5, tm.decode_json(t.encode("utf-8", "surrogatepass"))))
        except tm.Undecodable:
            c = tm.conform(counter, tm.Sample(5, attachment_undecodable=True))
        return [c.conformance, list(c.reasons)]

    edge = str(int(_sys.float_info.max) + 1)
    return [
        ("telemetry.v1 0.2 §2.6 (F-150): an integer's text one past the largest double rounds to it: finite, a gauge",
         text(edge), [tm.CONFORMS, []]),
        ("telemetry.v1 0.2 §2.6 (F-150): 1.7976931348623158e308, above the largest double, rounds to it: a gauge",
         text("1.7976931348623158e308"), [tm.CONFORMS, []]),
        ("telemetry.v1 0.2 §2.6 (F-150): the halfway point's text rounds to +Infinity: undecodable",
         text("1.797693134862315808e308"), [tm.UNOBSERVABLE, ["undecodable"]]),
        ("telemetry.v1 0.2 §2.6 check 2 (F-151): an Epoch attachment \"\\ud800\", a lone surrogate, does not "
         "decode as strict JSON: epoch_invalid", attached('"\\ud800"'), [tm.DOES_NOT, ["epoch_invalid"]]),
    ]


def _telemetry_deep() -> list[tuple[str, Any, Any]]:
    """telemetry.v1 §2.6, a sample nested deeper than zk2py's reader goes
    (review of PR #826): read as undecodable, never a crash. RFC 8259 §9
    lets a parser limit nesting; zk2py's limit is Python's recursion's. A
    JSON Point whose `labels` nest 100,000 objects deep, a JSON array as
    deep in a gauge, and a CBOR array as deep."""
    from . import telemetry as tm

    point = tm.declare("stream", {}, {"kind": "jsonschema", "name": tm.POINT, "schema": tm.CURRENT, "root": None})
    gauge = tm.declare("stream", {tm.KIND: "gauge"}, {"kind": "jsonschema", "name": "Level",
                                                      "schema": "sha256:" + "a" * 64, "root": {"type": "number"}})
    n = 100_000

    def read(decl: Any, decode: Any, payload: bytes) -> Any:
        try:
            c = tm.conform(decl, tm.Sample(decode(payload)))
        except tm.Undecodable:
            c = tm.conform(decl, tm.Sample(undecodable=True))
        return [c.conformance, list(c.reasons)]

    labels = b'{"timestamp": 1, "value": {"type": "gauge", "value": 1}, "labels": ' + b'{"a": ' * n + b'"x"' \
        + b"}" * n + b"}"
    want = [tm.UNOBSERVABLE, ["undecodable"]]
    label = "telemetry.v1 §2.6, nested 100,000 deep (review of PR #826)"
    return [
        (f"{label}: a JSON Point's labels: undecodable, no crash", read(point, tm.decode_json, labels), want),
        (f"{label}: a JSON array in a gauge: undecodable, no crash",
         read(gauge, tm.decode_json, b"[" * n + b"1" + b"]" * n), want),
        (f"{label}: a CBOR array in a gauge: undecodable, no crash",
         read(gauge, tm.decode_cbor, b"\x81" * n + b"\x01"), want),
    ]


def _telemetry_verdicts() -> list[tuple[str, Any, Any]]:
    """telemetry.v1 0.4 §5 "Over many pairs", zk2py's tool answer over a
    window of members (`telemetry.verdict`, `member_answer`): a finding;
    "could not tell" of any counter or histogram read that formed no pair
    (F-152, resolved by 0.4 as zk2py read it) or only assumed ones, whatever
    another formed; no otherwise, a window of gauges alone included."""
    from . import telemetry as tm

    def conf(kind: str, ok: bool = True) -> Any:
        return tm.Conformance(tm.CONFORMS if ok else tm.DOES_NOT, () if ok else ("wrong_type",), kind, "declared",
                              None, None)

    def pair(cont: str, epoch: str) -> Any:
        return tm.Pair(cont, epoch, "presence" if epoch == "established" else None, None, None, "kind")

    label = "telemetry.v1 0.4 and 0.5 §5, over many pairs and many samples"
    est = ([conf("counter")] * 2, [pair("same_epoch", "established")])
    invalid = tm.Conformance(tm.UNOBSERVABLE, ("declaration_invalid",), None, None, None, None)
    return [
        (f"{label}: a sample that does not conform is a finding", tm.verdict([([conf("counter", False)], [])]),
         tm.FINDING),
        (f"{label}: a decrease within an established epoch is a finding",
         tm.verdict([([conf("counter")] * 2, [pair("decreased", "established")])]), tm.FINDING),
        (f"{label}: every pair of a counter assumed: could not tell",
         tm.verdict([([conf("counter")] * 3, [pair("same_epoch", "assumed"), pair("reset", "assumed")])]),
         tm.COULD_NOT_TELL),
        (f"{label}: one member, an established pair beside an assumed one: no",
         tm.verdict([([conf("counter")] * 3, [pair("same_epoch", "assumed"), pair("same_epoch", "established")])]),
         tm.NO),
        (f"{label} (F-152, as 0.4 resolved it): a counter's one sample, no pair at all: could not tell",
         tm.verdict([([conf("counter")], [])]), tm.COULD_NOT_TELL),
        (f"{label} (0.4): one counter established, another with no pair: could not tell of the second",
         tm.verdict([est, ([conf("counter")], [])]), tm.COULD_NOT_TELL),
        (f"{label} (0.4): one counter established, another only assumed: could not tell",
         tm.verdict([est, ([conf("counter")] * 2, [pair("same_epoch", "assumed")])]), tm.COULD_NOT_TELL),
        (f"{label} (0.4): a finding stands beside a counter that could not tell",
         tm.verdict([([conf("counter")], []), ([conf("counter")] * 2, [pair("decreased", "established")])]),
         tm.FINDING),
        (f"{label}: a gauge's samples alone, the continuity question not asked: no",
         tm.verdict([([conf("gauge")] * 2, [pair("not_asked", None)])]), tm.NO),
        (f"{label}: two counters established, a gauge with one sample: no",
         tm.verdict([est, est, ([conf("gauge")], [])]), tm.NO),
        (f"{label}: no sample at all: could not tell (silence is never a verdict)", tm.verdict([]),
         tm.COULD_NOT_TELL),
        (f"{label} (F-157, as 0.5 resolved it): a gauge's conforming sample beside an undecodable one: could "
         "not tell (§5, over many samples)",
         tm.verdict([([conf("gauge"), tm.Conformance(tm.UNOBSERVABLE, ("undecodable",), "gauge", "declared", None,
                                                     None)], [])]), tm.COULD_NOT_TELL),
        (f"{label} (0.5): a member read under an invalid declaration, beside a conforming gauge: could not tell",
         tm.verdict([([conf("gauge")], []), ([invalid], [])]), tm.COULD_NOT_TELL),
        (f"{label} (0.5): every question of such a member is unanswered, its continuity included",
         tm.member_answer([invalid], []), tm.UNOBSERVABLE),
        (f"{label} (0.5): a finding stands beside a member under an invalid declaration",
         tm.verdict([([conf("counter", False)], []), ([invalid], [])]), tm.FINDING),
        (f"{label} (0.5): an undeclared Point never decoded is still not asked of continuity",
         tm.member_answer([tm.Conformance(tm.UNOBSERVABLE, ("undecodable",), None, None, None, None)], []),
         tm.NOT_ASKED),
    ]


def _telemetry_tag_chains() -> list[tuple[str, Any, Any]]:
    """telemetry.v1 0.3 §2.6 "CBOR" over chains of tags (review of PR #828),
    read as ``zk2py.cbor`` reads them, innermost first and without
    recursion: an outer tag 2 or 3 around what an inner bignum already made
    an integer is "a tag 2 or 3 around anything but a byte string, … read as
    its content"; a chain of 100,000 tags adds no depth, so the integer it
    wraps is a counter."""
    from . import telemetry as tm

    counter = tm.declare("stream", {tm.KIND: "counter"}, {"kind": "jsonschema", "name": "Count",
                                                          "schema": "sha256:" + "a" * 64,
                                                          "root": {"type": "integer"}})

    def read(payload: bytes) -> Any:
        try:
            c = tm.conform(counter, tm.Sample(tm.decode_cbor(payload)))
            return [c.conformance, list(c.reasons), c.value]
        except tm.Undecodable as e:
            return ["undecodable", str(e)]

    label = "telemetry.v1 0.3 §2.6, a chain of CBOR tags (review of PR #828)"
    return [
        (f"{label}: tag 2 around tag 2 around 0x05: the inner bignum is 5, the outer tag its content",
         read(bytes.fromhex("c2c24105")), [tm.CONFORMS, [], 5]),
        (f"{label}: tag 3 around tag 2 around 0x05: still 5, the outer tag 3 around an integer its content",
         read(bytes.fromhex("c3c24105")), [tm.CONFORMS, [], 5]),
        (f"{label}: tag 2 around tag 3 around 0x05: -6, and a counter below 0 is out of range",
         read(bytes.fromhex("c2c34105")), [tm.DOES_NOT, ["out_of_range"], None]),
        (f"{label}: 100,000 tags 55799 around the integer 7: no depth, a counter, never a crash",
         read(bytes.fromhex("d9d9f7") * 100_000 + b"\x07"), [tm.CONFORMS, [], 7]),
        (f"{label}: 100,000 tags around a bignum of 2^64: out of range, as without them",
         read(bytes.fromhex("d9d9f7") * 100_000 + bytes.fromhex("c249010000000000000000")),
         [tm.DOES_NOT, ["out_of_range"], None]),
    ]


def _presence_fold() -> list[tuple[str, Any, Any]]:
    """telemetry.v1 0.3 §2.9, a presence view seeded by a complete read
    (review of PR #828): the read's instances folded into the subscription's
    moves as of the read's end, the subscription's word winning
    (``telemetry_scenarios.fold_seed``)."""
    from .telemetry_scenarios import fold_seed

    def held(tokens: list[tuple[int, str, str]]) -> list[str]:
        h: set[str] = set()
        for _, kind, inst in sorted(tokens):
            (h.add if kind == "put" else h.discard)(inst)
        return sorted(h)

    label = "telemetry.v1 0.3 §2.9, a complete read folded into the subscription's moves (review of PR #828)"
    withdrawn = [(10, "put", "a"), (20, "delete", "a")]
    return [
        (f"{label}: a withdrawal the subscription saw before the read ended stands; the read's stale reply "
         "brings nothing back", held(fold_seed(withdrawn, ["a"], 30)), []),
        (f"{label}: an instance the subscription never mentioned is held from the read's end",
         fold_seed([], ["b"], 30), [(30, "put", "b")]),
        (f"{label}: one the subscription saw appear is not added twice",
         fold_seed([(10, "put", "a")], ["a"], 30), [(10, "put", "a")]),
    ]


def _desired_adopters(root: Path) -> list[tuple[str, Any, Any]]:
    """desired.v1 Appendix B and §2.3, read over the adopters' real
    contracts and bindings, where the fixtures give resources already
    resolved:
    - the walkthrough: ``plans/{vehicle}`` one target parameter, and the
      executor's binding (``acl/walkthrough.enrollment.toml``, core
      ``bindings.md`` §2) to its own plan, ``…/plans/vehicle-01``;
    - ZenSight: ``zs.desired.v1``'s seven templates, ``["host",
      "service"]``, never stale; ``zs.thresholds.v1``'s requirement bound
      for the sysinfo sensor to §2.3's example key; and its
      ``applied/{topic}``, still ``AppliedConfig``, no Report (Appendix B:
      "a tool's §5 finds no report of it");
    - every other resource under ``examples/zk2/`` not this profile's, and
      none invalid;
    - the scenarios' ``tasking.v1`` (scenarios.md, "The contracts"), its
      slice ``D`` for ``lab/worker``."""
    from . import desired as ds, freshness as fr
    from .contract import load_contract

    ex = root.parent.parent / "examples" / "zk2"
    spec = root.parent
    label = "desired.v1 Appendix B"
    out: list[tuple[str, Any, Any]] = []
    decls: dict[str, ds.Declaration] = {}
    contracts: dict[str, Any] = {}
    for f in sorted(ex.rglob("*.v*.toml")):
        c = load_contract(f, spec_dir=spec)
        contracts[f.stem] = c
        for r in c.canonical["resources"]:
            decls[f"{f.stem}:{r['template']}"] = ds.declare(r["token"] if r["kind"] == "state" else r["kind"],
                                                            r["template"], r["annotations"], r["params"])
    mine = {k: d for k, d in decls.items() if d.declaration != ds.NOT_THIS}
    out.append((f"{label}: the examples' desired resources are the walkthrough's one and zs.desired.v1's seven, "
                "none invalid",
                (sorted({k.split(":")[0] for k in mine}), len(mine), sorted(k for k, d in mine.items()
                                                                            if d.declaration != ds.DECLARED)),
                (["mission_plan.v1", "zs.desired.v1"], 8, [])))
    plan = decls.get("mission_plan.v1:plans/{vehicle}")
    out.append((f"{label}: the walkthrough's plans/{{vehicle}}, one target parameter, either identity, no horizon",
                None if plan is None else (plan.expect(), plan.horizon), (
                    {"declaration": "declared", "targets": [{"param": "vehicle", "binds": "either"}]}, None)))
    if plan is not None:
        got = ds.bind(plan, "plans/{vehicle}", False, "mission_plan.v1", "vehicle-01/executor",
                      {"bindings": ["ground/fleet-mgr"], "params": {"vehicle": "self.system"}, "optional": False})
        out.append((f"{label}: the executor's binding (acl/walkthrough.enrollment.toml) is its own plan "
                    "(core bindings.md §2)", got.expect(),
                    {"binding": "bound", "key": "zk2/ground/fleet-mgr/mission_plan.v1/state/plans/vehicle-01"}))
    zs = {k: d for k, d in mine.items() if k.startswith("zs.desired.v1:")}
    out.append((f"{label}: zs.desired.v1's seven templates, each [host, service] bound self.system then "
                "self.service, never stale",
                sorted({(json.dumps(d.expect(), sort_keys=True), d.horizon.horizon if d.horizon else None)
                        for d in zs.values()}),
                [(json.dumps({"declaration": "declared", "targets": [
                    {"param": "host", "binds": "self.system"}, {"param": "service", "binds": "self.service"}]},
                    sort_keys=True), "never")]))
    th = zs.get("zs.desired.v1:desired/{host}/{service}/thresholds")
    req = contracts["zs.thresholds.v1"].canonical["requires"].get("desired")
    if th is not None and req is not None:
        got = ds.bind(th, "desired/{host}/{service}/thresholds", False, req["interface"], "h-3fa9c2d41b7e/sysinfo",
                      {"bindings": ["fleet/desired"], "params": {"host": "self.system", "service": "self.service"},
                       "optional": req["optional"]})
        out.append((f"{label}: zs.thresholds.v1's requirement, bound for the sysinfo sensor, is §2.3's example key",
                    (req["resources"], got.expect()),
                    (["desired/{host}/{service}/thresholds"], {
                        "binding": "bound",
                        "key": "zk2/fleet/desired/zs.desired.v1/state/desired/h-3fa9c2d41b7e/sysinfo/thresholds"})))
        got = ds.bind(th, "desired/{host}/{service}/thresholds", False, req["interface"], "h-3fa9c2d41b7e/sysinfo",
                      {"bindings": [], "params": {}, "optional": req["optional"]})
        out.append((f"{label}: zs.thresholds.v1's requirement is optional, so unbound it is the kill switch (§2.12)",
                    got.expect(), {"binding": "unbound"}))
    applied = next((r for r in contracts["zs.thresholds.v1"].canonical["resources"]
                    if r["template"] == "applied/{topic}"), None)
    reports = [f"{s}:{r['template']}" for s, c in contracts.items() for r in c.canonical["resources"]
               if ds.is_profile_type(r.get("type"))]
    out.append((f"{label}: zs.thresholds.v1's applied/{{topic}} is AppliedConfig, not the Report, and no example "
                "resource is typed desired.v1.Report", (None if applied is None else
                                                        (applied["type"].get("name"), fr.horizon("state",
                                                         applied["annotations"]).ttl_s), reports),
                (("AppliedConfig", 900), [])))
    task = ds.declare("state", "tasks/{system}/{service}",
                      {"desired.target_param": ["system", "service"], "freshness.ttl_s": 0},
                      {"system": "string", "service": "string"})
    got = ds.bind(task, "tasks/{system}/{service}", False, "tasking.v1", "lab/worker",
                  {"bindings": ["lab/author"], "params": {"system": "self.system", "service": "self.service"},
                   "optional": True})
    out.append(("desired.v1 scenarios.md, \"The contracts\": tasking.v1's slice for lab/worker is D", got.expect(),
                {"binding": "bound", "key": "zk2/lab/author/tasking.v1/state/tasks/lab/worker"}))
    for bad in ("lab", "lab/worker/x", "lab/", "Lab/worker", "*/worker"):
        got = ds.bind(task, "tasks/{system}/{service}", False, "tasking.v1", bad,
                      {"bindings": ["lab/author"], "params": {"system": "self.system", "service": "self.service"},
                       "optional": True})
        out.append((f"desired.v1 §2.3, zk2py's own refusal: a target address {bad!r} is not <system>/<service>",
                    got.expect(), {"binding": "invalid", "reasons": ["target_address"]}))
    return out


def _desired_readings() -> list[tuple[str, Any, Any]]:
    """zk2py's readings of desired.v1 beyond its fixtures: the readings of
    round 52's findings (F-162, F-163, F-166, F-167, F-168), now 0.2's rules,
    and F-181's, §2.7's equality
    over a payload's bytes, §2.10's report puts, and a report's bytes that
    no fixture gives."""
    from . import desired as ds

    K = "zk2/lab/author/tasking.v1/state/tasks/lab/worker"

    def put(time: int, sid: int, doc: Any, origin: str = ds.OWNER) -> ds.Sample:
        rev = doc.get("revision") if isinstance(doc, dict) else None
        return ds.Sample(origin, False, ds.Stamp(time, sid), ds.Payload(value=doc), rev)

    def run(steps: list[Any]) -> tuple[list[str], dict[str, Any], int]:
        t = ds.TargetRole(K)
        words = []
        for s in steps:
            if isinstance(s, ds.Sample):
                words.append(t.receive(s))
            elif s == "done":
                words.append(t.finish(ds.Finish(ds.DONE)))
            elif s == "call":
                words.append(t.call())
            else:
                words.append(t.finish(s))
        return words, t.report(), t.report_puts

    v = {"mode": "run", "rate_hz": 10, "revision": 1}
    w = {"mode": "run", "rate_hz": 10, "revision": 2}
    out: list[tuple[str, Any, Any]] = []
    lab = "desired.v1 0.1 §2.7 (F-162): judged against the running act's, then the held one"
    out.append((f"{lab}: the running act's version again, another id held, is the same version, never held",
                run([put(100, 0xa1, v), put(50, 0xb2, w), put(100, 0xa1, v), "done", "done"])[0],
                ["act", "held", "same_version", "act", "done"]))
    out.append((f"{lab}: older than the running act's, another id held, is older",
                run([put(100, 0xa1, v), put(50, 0xb2, w), put(90, 0xa1, v)])[0], ["act", "held", "older"]))
    out.append(("desired.v1 0.2 §2.6 step 3 (F-184): the author's own sample of a version it holds waiting from the "
                "author itself is the same version, never confirmed",
                run([put(100, 0xa1, v), put(200, 0xa1, w), put(200, 0xa1, w), "done", "done"]),
                (["act", "held", "same_version", "act", "done"],
                 {"key": K, "source": "desired", "in_force": {"stamp": "200/a1", "revision": 2},
                  "observed": {"stamp": "200/a1", "revision": 2}, "outcome": "converged"}, 3)))
    lab = "desired.v1 0.2 §2.6 (F-163): every id it remembers, at least the 16 most recent"
    words, rep, _ = run([put(500, 0xa1, v), "done", put(100, 0xb2, w), "done",
                         put(400, 0xa1, {"mode": "halt", "rate_hz": 1, "revision": 0}, ds.ARCHIVE)])
    out.append((f"{lab}: an archive's older version of the old id, after the new id's first, is older",
                (words, rep["observed"], rep.get("last_known")),
                (["act", "done", "act", "done", "older"], {"stamp": "100/b2", "revision": 2}, None)))
    ids = [put(500, 0xa0 + i, {"mode": "run", "rate_hz": 10, "revision": i}) for i in range(17)]
    seq: list[Any] = []
    for x in ids:
        seq += [x, "done"]
    out.append((f"{lab}: zk2py remembers exactly 16, so a 17th id forgets the first, whose stale value is then "
                "read as new: the residual the text accepts",
                run(seq + [put(400, 0xa0, {"mode": "halt", "rate_hz": 1, "revision": 99}, ds.ARCHIVE)])[0][-1], "act"))
    out.append((f"{lab}: the 16 most recent are still remembered",
                run(seq + [put(400, 0xa1, {"mode": "halt", "rate_hz": 1, "revision": 99}, ds.ARCHIVE)])[0][-1],
                "older"))
    out.append((f"{lab}: a remembered id's latest time again, by the archive's path, is the same version",
                run([put(500, 0xa1, v), "done", put(100, 0xb2, w), "done", put(500, 0xa1, v, ds.ARCHIVE)])[0][-1],
                "same_version"))
    out.append((f"{lab}: a restart forgets every id", _restart_forgets(ds), "act"))
    lab = "desired.v1 0.2 §2.7 (F-166): JCS-equal, every number by its exact value"
    out.append((f"{lab}: revisions 2^53 and 2^53+1 are two documents",
                ds.jcs_equal({"revision": 2**53}, {"revision": 2**53 + 1}), False))
    out.append((f"{lab}: 10^16 written as an integer and as 1e16 are one document",
                ds.jcs_equal({"n": 10**16}, {"n": 1e16}), True))
    out.append((f"{lab}: a boolean is never a number", ds.jcs_equal({"n": True}, {"n": 1}), False))
    out.append((f"{lab}: past 2^53 on both sides and otherwise equal, one document",
                ds.jcs_equal({"revision": 2**64 - 1, "b": [1.5]}, {"b": [1.5], "revision": 2**64 - 1}), True))
    lab = "desired.v1 0.2 §2.7, exact values in time bounded by the number's text (review of PR #845)"
    import time

    def timed(f: Any) -> tuple[Any, bool]:
        t0 = time.perf_counter()
        got = f()
        return got, time.perf_counter() - t0 < 0.5

    def j(text: str) -> Any:
        return ds.Payload.of_bytes(text.encode(), True)

    E = "9" * 100_000
    out += [
        (f"{lab}: 1e999999999 and 1e999999998 are not finite, so byte for byte: two, at once",
         timed(lambda: ds.same_document(j('{"a":1e999999999}'), j('{"a":1e999999998}'))), (False, True)),
        (f"{lab}: 1e-999999999 is finite (0.0) and reads; against 1e-999999998, two, at once",
         timed(lambda: ds.same_document(j('{"a":1e-999999999}'), j('{"a":1e-999999998}'))), (False, True)),
        (f"{lab}: 1e-999999999 against 0: two", ds.same_document(j('{"a":1e-999999999}'), j('{"a":0}')), False),
        (f"{lab}: 10e-1000000000 and 1e-999999999 are one value, normalised, at once",
         timed(lambda: ds.same_document(j('{"a":10e-1000000000}'), j('{"a":1e-999999999}'))), (True, True)),
        (f"{lab}: an exponent past 10^22, which Python's decimal refuses, reads and compares by its text",
         timed(lambda: ds.same_document(j('{"a":1e-99999999999999999999999}'), j('{"a":1e-99999999999999999999999}'))),
         (True, True)),
        (f"{lab}: a 100,000-digit exponent: the same text one value, at once",
         timed(lambda: ds.same_document(j('{"a":1e-%s}' % E), j('{"a":1.0e-%s}' % E))), (True, True)),
        (f"{lab}: a 100,000-digit exponent written another way: two, which costs an act, at once",
         timed(lambda: ds.same_document(j('{"a":1e-%s}' % E), j('{"a":10e-%s}' % (E[:-1] + "8")))), (False, True)),
        (f"{lab}: an exponent's leading zeros carry nothing: 1e-0000000000000000000000000000001 is 0.1",
         ds.same_document(j('{"a":1e-0000000000000000000000000000001}'), j('{"a":0.1}')), True),
        (f"{lab}: -0.0e5 is 0", ds.same_document(j('{"a":-0.0e5}'), j('{"a":0}')), True),
        (f"{lab}: 1e2 against the integer 100: one", ds.same_document(j('{"a":1e2}'), j('{"a":100}')), True),
        (f"{lab}: an integer of 4301 digits does not read (Python's bound, far past the double's)",
         ds.same_document(j('{"a":%s}' % ("1" * 4301)), j('{"a":%s}' % ("1" * 4300 + "2"))), False),
    ]
    edge = 2**1024 - 2**970
    lab = "desired.v1 0.2 §2.10, finite by rounding at the edge, 2^1024 - 2^970 (review of PR #845)"

    def reads(text: str) -> bool:
        try:
            ds.strict_json(text.encode())
            exact = True
            ds.strict_json(text.encode(), exact=True)
        except ds.NotStrict:
            return False
        return exact

    out += [
        (f"{lab}: the edge written as an integer is not finite, one below it is",
         [reads(str(edge)), reads(str(edge - 1))], [False, True]),
        (f"{lab}: the edge written with a fraction is not finite, one below it is",
         [reads(str(edge) + ".0"), reads(str(edge - 1) + ".0")], [False, True]),
        (f"{lab}: 1.7976931348623159e308 is not, 1.7976931348623158e308 is",
         [reads("1.7976931348623159e308"), reads("1.7976931348623158e308")], [False, True]),
        (f"{lab}: -1e309 is not, 1e-99999999999999999999999 is (0.0), in a report too",
         [reads("-1e309"), reads("1e-99999999999999999999999"),
          ds.read_report_bytes(b'{"source":"baseline","x":1e-99999999999999999999999}').readable], [False, True, True]),
    ]
    lab = "desired.v1 0.2 §2.7: a value strict JSON cannot yield equals nothing, itself included"
    inf, nan = float("inf"), float("nan")
    out += [
        (f"{lab}: a non-finite float (FloatDomainError)", ds.jcs_equal({"n": inf}, {"n": inf}), False),
        (f"{lab}: NaN", ds.jcs_equal({"n": nan}, {"n": nan}), False),
        (f"{lab}: a string with no UTF-8 form", ds.jcs_equal({"s": "\ud800"}, {"s": "\ud800"}), False),
        (f"{lab}: a key that is not a string", ds.jcs_equal({1: 2}, {1: 2}), False),
        (f"{lab}: a value that is not JSON", ds.jcs_equal({"b": b"x"}, {"b": b"x"}), False),
        (f"{lab}: an integer past 2^53 beside a non-finite float: the exact fallback refuses it too",
         ds.jcs_equal({"r": 2**53, "n": inf}, {"r": 2**53, "n": inf}), False),
        (f"{lab}: such a document is acted on, never adopted",
         run([put(100, 0xa1, {"n": inf}), "done", put(200, 0xa1, {"n": inf})])[0], ["act", "done", "act"]),
    ]
    out.append((f"{lab}: a revision moved above 2^53 is acted on, not adopted",
                run([put(100, 0xa1, {"revision": 2**53}), "done", put(200, 0xa1, {"revision": 2**53 + 1})])[0],
                ["act", "done", "act"]))
    lab = "desired.v1 0.2 §2.7: a JSON payload's bytes read as strict JSON, exactly, else compared byte for byte"
    out += [
        (f"{lab}: members in another order and 10.0 for 10 are one document",
         ds.same_document(j('{"rate_hz": 10, "mode": "run"}'), j('{"mode":"run","rate_hz":10.0}')), True),
        (f"{lab}: a trailing newline is RFC 8259's whitespace", ds.same_document(j('{"a":1}'), j('{"a":1}\n')),
         True),
        (f"{lab}: two equal bytes that do not read are one document", ds.same_document(j('{"a":'), j('{"a":')),
         True),
        (f"{lab}: one that reads and one that does not are two",
         ds.same_document(j('{"a":1}'), j('{"a":1,"a":1}')), False),
        (f"{lab}: CBOR, protobuf or raw: byte for byte, so one value's two encodings are two documents",
         ds.same_document(ds.Payload.of_bytes(b"\x18\x0a", False), ds.Payload.of_bytes(b"\x0a", False)), False),
        (f"{lab}: a put is never a delete", ds.same_document(j("null"), None), False),
        (f"{lab}: -0 and 0 are one value", ds.same_document(j('{"a":-0}'), j('{"a":0.0}')), True),
        (f"{lab}: 1e1, 10, 10.0 and 100e-1 are one value",
         [ds.same_document(j('{"a":10}'), j('{"a":%s}' % t)) for t in ("1e1", "10.0", "100e-1")], [True, True, True]),
        (f"{lab}: decimals equal to the 30th digit and apart after it are two",
         ds.same_document(j('{"a":0.1000000000000000000000000000001}'), j('{"a":0.1000000000000000000000000000002}')),
         False),
        (f"{lab}: 256 deep reads and compares by value",
         ds.same_document(j("[" * 256 + "]" * 256), j("[ " * 256 + "]" * 256)), True),
        (f"{lab}: 257 deep does not read, so byte for byte: one space apart, two",
         ds.same_document(j("[" * 257 + "]" * 257), j("[ " * 257 + "]" * 257)), False),
        (f"{lab}: 257 deep, the same bytes: one document",
         ds.same_document(j("[" * 257 + "]" * 257), j("[" * 257 + "]" * 257)), True),
    ]
    lab = "desired.v1 0.1 §2.10 \"put it again whenever\""
    out += [
        (f"{lab}: the first report, then an act's end, an adoption, a call: four puts",
         run([put(100, 0xa1, v), "done", put(200, 0xa1, v), "call"])[2], 4),
        (f"{lab}: the same version, an older one, an unstamped one and a held one put nothing",
         run([put(200, 0xa1, v), put(200, 0xa1, v), put(100, 0xa1, v),
              ds.Sample(ds.OWNER, False, None, ds.Payload(value=v)), put(300, 0xa1, w)])[2], 1),
        (f"{lab}: the author's confirmation of a last-known version is a put",
         run([put(100, 0xa1, v, ds.ARCHIVE), "done", put(100, 0xa1, v)])[1:], (
             {"key": K, "source": "desired", "in_force": {"stamp": "100/a1", "revision": 1},
              "observed": {"stamp": "100/a1", "revision": 1}, "outcome": "converged"}, 3)),
    ]
    lab = "desired.v1 0.1 §2.10, a report's bytes"
    deep = b'{"source":"baseline","x":' + b"[" * 100_000 + b"]" * 100_000 + b"}"
    out += [
        (f"{lab}: nested 100,000 deep under a member the schema does not name: not read, never a crash",
         ds.read_report_bytes(deep).expect(), {"readable": False, "errors": ["json"]}),
        (f"{lab} (0.2): nested 300 deep there is past 256: not read",
         ds.read_report_bytes(b'{"source":"baseline","x":' + b"[" * 300 + b"]" * 300 + b"}").expect(),
         {"readable": False, "errors": ["json"]}),
        (f"{lab} (0.3, F-181): a revision 18446744073709551615.0 is read by its exact value, 2^64-1: in range",
         ds.read_report_bytes(('{"key":"%s","source":"desired","observed":{"stamp":"1/1",'
                               '"revision":18446744073709551615.0},"in_force":{"stamp":"1/1",'
                               '"revision":18446744073709551615},"outcome":"converged"}' % K).encode()).expect(),
         {"readable": True, "errors": []}),
        (f"{lab} (0.3, F-181): in_force's 9007199254740993 and observed's 9007199254740993.0, one revision "
         "exactly",
         ds.read_report_bytes(('{"key":"%s","source":"desired","observed":{"stamp":"1/1",'
                               '"revision":9007199254740993.0},"in_force":{"stamp":"1/1",'
                               '"revision":9007199254740993},"outcome":"converged"}' % K).encode()).expect(),
         {"readable": True, "errors": []}),
        (f"{lab}: a byte-order mark before the value is not RFC 8259's whitespace",
         ds.read_report_bytes(b'\xef\xbb\xbf{"source":"baseline"}').expect(), {"readable": False, "errors": ["json"]}),
        (f"{lab}: a lone surrogate escape has no UTF-8 form",
         ds.read_report_bytes(b'{"source":"baseline","detail":"\\ud800"}').expect(),
         {"readable": False, "errors": ["json"]}),
        (f"{lab} (F-167): a revision of 10^400 written out is not finite, as telemetry.v1 §2.6 reads it",
         ds.read_report_bytes(('{"key":"%s","source":"desired","observed":{"stamp":"1/1","revision":1%s},'
                               '"in_force":{"stamp":"1/1"},"outcome":"converged"}' % (K, "0" * 400)).encode()).expect(),
         {"readable": False, "errors": ["json"]}),
    ]
    lab = "desired.v1 0.2 §2.17 step 2 (F-168): across a face, the deployment's word, never the descriptor"

    def across(descriptor: Any, word: Any) -> dict[str, Any]:
        r = ds.Reading(ds.ACROSS_FACE, descriptor, None, "not_asked", ds.ACROSS_FACE, ("put", ds.Stamp(100, 0xa1)),
                       word)
        return {q: a.pair() for q, a in ds.conclude(r).items()}

    out.append((f"{lab}: a descriptor read across a face, and no word: nothing stands for it",
                across((True, True), None)[ds.Q_IN_FORCE], {"answer": "unobservable", "reason": "no_descriptor"}))
    out.append((f"{lab}: the word unbound beside a descriptor bound: the word answers",
                across((True, True), (True, False))[ds.Q_IN_FORCE], {"answer": "no", "reason": "unbound"}))
    out.append((f"{lab}: present, the word is ignored",
                {q: a.pair() for q, a in ds.conclude(ds.Reading(ds.PRESENT, (True, False), None, "not_asked",
                                                                 ds.PRESENT, None, (True, True))).items()}
                [ds.Q_IN_FORCE], {"answer": "no", "reason": "unbound"}))
    return out


def _restart_forgets(ds: Any) -> str:
    """desired.v1 0.2 §2.6: "A restart forgets every id, as it forgets every
    act (§2.7)": an old id's older version after a restart is acted on."""
    t = ds.TargetRole("zk2/lab/author/tasking.v1/state/tasks/lab/worker")
    doc = {"mode": "run", "rate_hz": 10, "revision": 1}
    t.receive(ds.Sample(ds.OWNER, False, ds.Stamp(500, 0xa1), ds.Payload(value=doc), 1))
    t.finish(ds.Finish(ds.DONE))
    t.restart()
    return t.receive(ds.Sample(ds.OWNER, False, ds.Stamp(400, 0xa1), ds.Payload(value=doc), 1))


def _presence_view() -> list[tuple[str, Any, Any]]:
    """Core 0.37 §8.1, zk2py's session-free presence view
    (:mod:`zk2py.presence`): "Absence is established only by a complete
    read", a withdrawal unconfirmed until one, one read for a burst, none
    for a make-before-break re-mint, and a read not made leaving the
    withdrawal unconfirmed; and the three kinds of a read (:mod:`zk2py.live`)."""
    from . import live
    from . import presence as pr

    I = "zk2/lab/o/@zk/instance/" + "a" * 16
    J = "zk2/lab/o/@zk/instance/" + "b" * 16
    K = "zk2/lab/p/@zk/instance/" + "c" * 16
    seed = [("ask", None), ("read", ("complete", [I, K]))]

    def at(ev: list[Any], addr: tuple[str, str] = ("lab", "o")) -> tuple[Any, bool]:
        v = pr.fold(ev)
        return v.address(*addr), v.owes_read

    label = "core 0.37 §8.1, zk2py's presence view"
    return [
        (f"{label}: seeded by a complete read: up, nothing owed", at(seed), ((pr.UP, ""), False)),
        (f"{label}: a withdrawal heard live is unobservable, unconfirmed, never down, and owes a read",
         at(seed + [("delete", I)]), ((pr.UNOBSERVABLE, pr.WITHDRAWN), True)),
        (f"{label}: the re-read, complete and showing none: down",
         at(seed + [("delete", I), ("ask", None), ("read", ("complete", [K]))]), ((pr.DOWN, ""), False)),
        (f"{label}: the re-read showing it again: up (its router restarted, the owner never left)",
         at(seed + [("delete", I), ("ask", None), ("read", ("complete", [I, K]))]), ((pr.UP, ""), False)),
        (f"{label}: the re-read not made (cut off): still unconfirmed, the read still owed",
         at(seed + [("delete", I), ("ask", None), ("read", ("not made", []))]), ((pr.UNOBSERVABLE, pr.WITHDRAWN), True)),
        (f"{label}: the re-read possibly incomplete and showing none: still unconfirmed, still owed",
         at(seed + [("delete", I), ("ask", None), ("read", ("possibly incomplete", []))]),
         ((pr.UNOBSERVABLE, pr.WITHDRAWN), True)),
        (f"{label}: a make-before-break re-mint, the new token heard first: up, nothing owed",
         at(seed + [("put", J), ("delete", I)]), ((pr.UP, ""), False)),
        (f"{label}: a burst, two addresses withdrawn: one read asked answers both",
         [at(seed + [("delete", I), ("delete", K), ("ask", None)], a) for a in (("lab", "o"), ("lab", "p"))],
         [((pr.UNOBSERVABLE, pr.WITHDRAWN), False)] * 2),
        (f"{label}: a withdrawal heard after the read was asked keeps its word over the read's reply",
         at(seed + [("ask", None), ("delete", I), ("read", ("complete", [I, K]))]), ((pr.UNOBSERVABLE, pr.WITHDRAWN), True)),
        (f"{label}: put, deleted and put again after the read was asked: the latest move decides, held (review "
         "of PR #848)", at(seed + [("ask", None), ("put", I), ("delete", I), ("put", I), ("read", ("complete", [K]))]),
         ((pr.UP, ""), False)),
        (f"{label}: put and deleted after the read was asked, the read showing it: withdrawn, unconfirmed",
         at(seed + [("ask", None), ("put", I), ("delete", I), ("read", ("complete", [I, K]))]),
         ((pr.UNOBSERVABLE, pr.WITHDRAWN), True)),
        (f"{label}: withdrawn, then heard again before any read: up, and no read owed any more",
         at(seed + [("delete", I), ("put", I)]), ((pr.UP, ""), False)),
        (f"{label}: one of two withdrawn back, the other still unconfirmed: a read still owed",
         [at(seed + [("delete", I), ("delete", K), ("put", I)], a) for a in (("lab", "o"), ("lab", "p"))],
         [((pr.UP, ""), True), ((pr.UNOBSERVABLE, pr.WITHDRAWN), True)]),
        (f"{label}: an address never heard of, under a complete read showing none: down",
         at([("ask", None), ("read", ("complete", []))]), ((pr.DOWN, ""), False)),
        (f"{label}: an address never heard of, under a read not made: unobservable",
         at([("ask", None), ("read", ("not made", []))]), ((pr.UNOBSERVABLE, pr.READ_NOT_MADE), False)),
        (f"{label}: holders are an address's instances, an interface's tokens, a member's epochs",
         [pr.holder_of("zk2/a/b/@zk/instance/" + "1" * 16), pr.holder_of("zk2/a/b/@zk/alive/x.v1/" + "1" * 16 + "/" + "2" * 16),
          pr.holder_of("zk2/a/b/@zk/member/x.v1/m/" + "3" * 16), pr.holder_of("zk2/a/b/x.v1/state/s")],
         [("instance", "a", "b"), ("alive", "a", "b", "x.v1"), ("member", "a", "b", "x.v1", "m"), None]),
        (f"{label}: a read not made is a third kind, never complete nor possibly incomplete",
         [live.Presence(not_made=live.CUT_OFF).kind, live.Presence(not_made=live.BESIDE_SUBSCRIBER).kind,
          live.Presence(complete=True).kind, live.Presence().kind,
          live.Presence(complete=False, cut_off_during=True).kind],
         [live.NOT_MADE, live.NOT_MADE, live.COMPLETE, live.POSSIBLY_INCOMPLETE, live.POSSIBLY_INCOMPLETE]),
    ] + _live_guards() + _desired_tool_readings() + _settle_readings()


def _settle_readings() -> list[tuple[str, Any, Any]]:
    """redundancy.v1 0.2's kept events (F-199), where zk2py.redundancy's
    ``_settle`` re-enters ``step`` for the events after the first that asks
    something. What a list decided partway can do: no kept event moves the
    machine to taking over or stepping down, since only a read's decision, a
    finish and the lock do, and none of them is kept. A decision that does
    is an act's phase, which keeps what it keeps. So the rest is re-entered
    in a state that keeps it, and asks nothing."""
    import copy as _copy

    from . import redundancy as rd

    own, low, high, high2 = "00000000000000b2", "00000000000000a1", "00000000000000c3", "00000000000000d4"
    S = rd.Seen

    def read(complete: bool, *held: str, claims: tuple[str, ...] = ()) -> dict[str, Any]:
        return {"read": rd.Read(complete, tuple([S(own, own in held, "standby")]
                                                + [S(i, True, "standby") for i in held if i != own]
                                                + [S(i, False, "standby") for i in claims]))}

    def run(events: list[Any]) -> tuple[list[str], str, list[str], bool]:
        m = rd.Machine(own)
        steps = [m.step(e)["step"] for e in events]
        return steps, m.state, list(m.kept_appeared), m.kept_withdrawn

    serving = [rd.START, rd.FINISH, read(True), rd.FINISH]
    heal_second = serving + [{"appeared": low}, read(True, own, low), rd.TIMER]

    class Probe(rd.Machine):
        """A machine recording each step ``_settle`` re-enters, and what it gave."""

        def step(self, event: Any) -> dict[str, Any]:
            depth = self.__dict__.get("_depth", 0)
            self.__dict__["_depth"] = depth + 1
            try:
                out = super().step(event)
            finally:
                self.__dict__["_depth"] = depth
            if depth:
                self.__dict__.setdefault("_re", set()).add((self.state, out["step"]))
            return out

    def explore() -> list[tuple[str, str]]:
        reads = [read(c, *h, claims=cl) for c in (True, False)
                 for h, cl in (((own,), ()), ((own, low), ()), ((own, high), ()), ((), ()), ((own,), (high,)))]
        events = [rd.START, rd.TIMER, rd.FINISH, {"appeared": low}, {"appeared": high}, {"appeared": high2},
                  rd.WITHDRAWN, {"lock": True}, {"lock": False}] + reads
        found: set[tuple[str, str]] = set()
        for election in (rd.CLAIM, rd.EXTERNAL):
            seen: set[Any] = set()
            todo = [Probe(own, election)]
            while todo:
                m = todo.pop()
                k = (m.state, m.lock, m.retry, tuple(m.kept_appeared), m.kept_withdrawn, tuple(sorted(m.first)),
                     m.again)
                if k in seen:
                    continue
                seen.add(k)
                for e in events:
                    m2 = _copy.deepcopy(m)
                    m2.__dict__["_re"] = set()
                    m2.step(e)
                    found |= m2.__dict__["_re"]
                    if len(m2.kept_appeared) <= 3:
                        todo.append(m2)
        return sorted(found)

    label = "redundancy.v1 0.2, kept events decided partway (F-199, zk2py's _settle)"
    return [
        (f"{label}: a campaign's read decides take_over: the appearance kept through the act, the withdrawal "
         "nothing while taking over; at the finish the appearance asks a heal read",
         run([rd.START, rd.FINISH, {"appeared": high}, rd.WITHDRAWN, read(True), rd.FINISH]),
         (["claim", "read", "nothing", "nothing", "take_over", "read"], rd.HEAL_FIRST, [], False)),
        (f"{label}: … the act's phase keeps the appearance and drops the withdrawal",
         run([rd.START, rd.FINISH, {"appeared": high}, rd.WITHDRAWN, read(True)]),
         (["claim", "read", "nothing", "nothing", "take_over"], rd.TAKING_OVER, [high], False)),
        (f"{label}: a heal's second read decides step_down: the withdrawal kept through the act, the appearance "
         "nothing while stepping down; at the finish the withdrawal campaigns",
         run(heal_second + [{"appeared": high}, rd.WITHDRAWN, read(True, own, low), rd.FINISH]),
         (["claim", "read", "take_over", "nothing", "read", "grace", "read", "nothing", "nothing", "step_down",
           "read"], rd.CAMPAIGNING, [], False)),
        (f"{label}: a heal read ending the heal, three kept: the first appearance asks a heal read, and the rest "
         "is re-entered in heal_first, which keeps it",
         run(serving + [{"appeared": low}, {"appeared": high}, {"appeared": high2}, rd.WITHDRAWN, read(True, own)]),
         (["claim", "read", "take_over", "nothing", "read", "nothing", "nothing", "nothing", "read"], rd.HEAL_FIRST,
          [high2], True)),
        (f"{label}: over every state reached under both elections, each step re-entered is in heal_first and "
         "asks nothing", explore(), [(rd.HEAL_FIRST, "nothing")]),
    ]


def _desired_tool_readings() -> list[tuple[str, Any, Any]]:
    """desired.v1 0.4's tool rules, pure: §2.10's "Which reply is the
    report" over a target's report replies, and §2.17's slice taken from
    the target alone (zk2py.desired_tool)."""
    import json as _json

    from . import desired as ds, desired_tool as dt

    key = "zk2/lab/author/tasking.v1/state/tasks/lab/worker"

    def rep(stamp: str, k: str = key) -> bytes:
        return _json.dumps({"key": k, "source": "desired", "in_force": {"stamp": stamp},
                            "observed": {"stamp": stamp}, "outcome": "converged"}).encode()

    def pick(*payloads: bytes) -> Any:
        r = dt.pick_report([(f"r{i}", x) for i, x in enumerate(payloads)], key)
        return None if r is None else (r.readable and not r.errors, (r.report or {}).get("in_force"))

    other = "zk2/lab/author/tasking.v1/state/tasks/lab/other"
    label = "desired.v1 0.4 §2.10, which reply is the report"
    req = {"role": "task", "interface": "tasking.v1", "bindings": ["lab/author"],
           "params": {"system": "self.system", "service": "self.service"}, "optional": True}
    out = [
        (f"{label}: the readable one whose key is the slice's", pick(rep("1/a"), rep("2/a", other)),
         (True, {"stamp": "1/a"})),
        (f"{label}: two readable ones that claim it and differ: unreadable", pick(rep("1/a"), rep("2/a")),
         (False, None)),
        (f"{label}: the same report twice is one", pick(rep("1/a"), rep("1/a")), (True, {"stamp": "1/a"})),
        (f"{label}: none readable claiming it, one that does not read: unreadable", pick(b"{x", rep("2/a", other)),
         (False, None)),
        (f"{label}: a readable one claiming it beside one that does not read: the report",
         pick(b"{x", rep("1/a")), (True, {"stamp": "1/a"})),
        (f"{label}: none claiming it, every one readable: none read", pick(rep("2/a", other)), None),
    ]
    label = "desired.v1 0.5 §2.17, the slice from the target alone (F-210)"
    req5 = {"resources": ["tasks/{system}/{service}"], "bindings": ["lab/author"],
            "params": {"system": "self.system", "service": "self.service"}, "optional": True}
    at = key.replace("/state/", "/@state/")

    def taken(r: dict[str, Any]) -> Any:
        t = ds.take_slice("lab/worker", "tasking.v1", r, None, [])
        return list(t.candidates), t.source, list(t.binding)

    label8 = "desired.v1 0.8 §2.17, a requirement unread beside a revision that does not read"
    unread = {**req5, "resources": None, "unread": True}
    status = {"kind": "state", "template": "status", "params": {}, "annotations": {}}

    def found(r: dict[str, Any], named: Any, held: list[Any]) -> Any:
        return ds.find_report(ds.take_slice("lab/worker", "tasking.v1", r, named, held), []).expect()

    out += [
        (f"{label8}: the named revision without resources shows nothing: unread, never not_a_target",
         found(unread, {}, []), {"found": "requirement_unread"}),
        (f"{label8}: a held revision whose resources are not a list shows nothing: unread",
         found(unread, None, [{"resources": None}]), {"found": "requirement_unread"}),
        (f"{label8}: a readable held revision declaring no desired resource beside one that does not read: "
         "not_a_target", found(unread, None, [{}, {"resources": [status]}]), {"found": "not_a_target"}),
        (f"{label8}: read, a held revision that does not read is not in hand: the target alone, no candidate "
         "claimed", found(req5, None, [{}]), {"found": "none"}),
    ]
    out += [
        (f"{label}: its requirement's template under either kind token, each parameter its binding's self",
         taken(req5), ([key, at], ds.REQUIREMENT, [])),
        (f"{label}: the report's key choosing the kind token, @state",
         ds.which_report([("r", rep("1/a", at))], [key, at]).expect(), {"found": "one", "claims": at}),
        (f"{label}: a parameter its binding leaves unbound: no slice, target_unbound",
         taken({**req5, "params": {"system": "self.system"}}), ([], None, ["target_unbound"])),
    ]
    out += _desired_window_readings()
    return out


def _desired_window_readings() -> list[tuple[str, Any, Any]]:
    """desired.v1 0.4 §2.17 "How long behind", over zk2py.desired_tool.watch
    fed readings of its own (F-211: "every reading over the window; any
    other answer starts it again"; on a trusted clock, one reading)."""
    import time as _time

    from . import desired as ds, desired_tool as dt

    def row(answer: str, stamp: str | None) -> Any:
        a = {q: ds.Answer(answer if q == ds.Q_CONVERGED else ds.NO, "behind" if answer == ds.NOT_YET else "x")
             for q in ds.QUESTIONS}
        r = ds.Reading(ds.PRESENT, (True, True), None, "not_asked", ds.PRESENT, None)
        return dt.Row("lab/worker", "task", "bound", r, a, author_stamp=stamp)

    def run(seq: list[tuple[str, str | None]], for_s: float, clocks: bool = False) -> Any:
        it = iter(seq)
        last = [seq[-1]]

        def reader(*_: Any, **__: Any) -> list[Any]:
            try:
                last[0] = next(it)
            except StopIteration:
                pass
            return [row(*last[0])]

        return dt.watch(None, "lab/worker", for_s, "task", every_s=0.01, reader=reader,  # type: ignore[arg-type]
                        clocks_synced=clocks).behind

    now_ntp = int(_time.time() * 2 ** 32)
    old = f"{now_ntp - 10 * 2 ** 32}/a1"  # stamped 10 s ago
    new = f"{now_ntp}/a1"
    nyt, yes = ds.NOT_YET, ds.YES
    label = "desired.v1 0.4 §2.17, how long behind (F-211)"
    return [
        (f"{label}: not yet on one version over the whole window: behind, by the window",
         run([(nyt, new)] * 50, 0.05), {"task": (new, "window")}),
        (f"{label}: a later reading converged clears it", run([(nyt, old)] * 3 + [(yes, old)] * 50, 0.05,
                                                               clocks=True), {}),
        (f"{label}: a later reading of another answer starts the window again",
         run([(nyt, new)] * 3 + [(ds.UNOBSERVABLE, new)] + [(nyt, new)] * 2, 0.04), {}),
        (f"{label}: another version starts it again", run([(nyt, new)] * 3 + [(nyt, old)] * 2, 0.04), {}),
        (f"{label}: on a trusted clock, one reading finds a version stamped longer ago than the window",
         run([(nyt, old)], 0.0, clocks=True), {"task": (old, "clock")}),
        (f"{label}: without one, a single reading never does", run([(nyt, old)], 0.0), {}),
    ]


class _FakeSession:
    """A session for :func:`zk2py.live.list_presence`, :func:`zk2py.live.settle`
    and a :class:`zk2py.presence.PresenceFollower`'s watch: its liveliness
    GET answers ``tokens`` and its final reply at once; ``links`` is what
    its info's links say before and after, or raises; its transports say
    ``transports`` throughout (core 0.38: never what the check reads). Its
    liveliness subscribers and transport listeners are recorded, never
    called (round 67). With ``gate``, a GET's final reply waits until the
    gate is set (at most 30 s), so a case decides when a GET ends rather
    than racing a delay (review-bot on PR #885)."""

    def __init__(self, tokens: list[str], links: list[Any], transports: list[Any] | None = None,
                 delay_s: float = 0.0, errors: int = 0, gate: threading.Event | None = None) -> None:
        self.tokens, self.links, self.closed = tokens, list(links), False
        self.gate = gate
        self.held_transports = [object()] if transports is None else transports
        self.delay_s, self.errors = delay_s, errors
        self.asked: list[str] = []
        #: (selector, handler, history) of each liveliness subscriber declared
        self.subscribers: list[tuple[str, Any, bool]] = []
        #: each transport events listener's handler
        self.listeners: list[Any] = []
        outer = self

        class _Info:
            def links(self) -> list[Any]:
                t = outer.links.pop(0) if len(outer.links) > 1 else outer.links[0]
                if isinstance(t, Exception):
                    raise t
                return t

            def transports(self) -> list[Any]:
                return outer.held_transports

            def declare_transport_events_listener(self, handler: Any, history: bool = False) -> Any:
                outer.listeners.append(handler)
                return _Undeclarable()

        class _Undeclarable:
            def undeclare(self) -> None:
                pass

        class _Ok:
            def __init__(self, key: str) -> None:
                self.key_expr = key

        class _Reply:
            def __init__(self, key: str) -> None:
                self.ok, self.err = _Ok(key), None

        class _Err:
            def __init__(self) -> None:
                self.ok, self.err = None, type("E", (), {"encoding": "zenoh/string",
                                                          "payload": type("P", (), {"to_bytes": lambda s: b"Timeout"})()})()

        class _Liveliness:
            def get(self, selector: str, handler: Any, timeout: float = 1.0) -> None:
                outer.asked.append(selector)
                for k in outer.tokens:
                    handler.callback(_Reply(k))
                for _ in range(outer.errors):
                    handler.callback(_Err())
                if outer.delay_s:
                    time.sleep(outer.delay_s)
                if outer.gate is not None:
                    outer.gate.wait(30.0)
                handler.drop()

            def declare_subscriber(self, selector: str, handler: Any, history: bool = False) -> Any:
                outer.subscribers.append((selector, handler, history))
                return _Undeclarable()

        self.info, self._lv = _Info(), _Liveliness()

    def liveliness(self) -> Any:
        return self._lv

    def is_closed(self) -> bool:
        return self.closed


def _live_guards() -> list[tuple[str, Any, Any]]:
    """Core 0.37 and 0.38 §8.1 in :func:`zk2py.live.list_presence`, on fake
    sessions (review of PR #848): a read whose session's links cannot be
    read, before or after, is possibly incomplete with that reason, never
    complete; one whose links are gone at its end is possibly incomplete,
    cut off while it ran (0.38, F-185), whatever its transports list; a
    closed watch session is never handed back."""
    from . import live

    tok = "zk2/lab/o/@zk/instance/" + "a" * 16
    up = [object()]

    def read(links: list[Any], transports: list[Any] | None = None) -> tuple[str, int]:
        p = live.list_presence(_FakeSession([tok], links, transports))  # type: ignore[arg-type]
        return p.kind, len(p.instances)

    label = "core 0.38 §8.1, zk2py's guarded read"
    out = [
        (f"{label}: links held before and after: complete", read([up, up]), (live.COMPLETE, 1)),
        (f"{label}: none before: not made, cut off", read([[]]), (live.NOT_MADE, 0)),
        (f"{label}: none at its end (F-185): possibly incomplete, its tokens kept as seen",
         read([up, []]), (live.POSSIBLY_INCOMPLETE, 1)),
        (f"{label}: the accessor raises before: possibly incomplete, never complete",
         read([RuntimeError("no links"), up]), (live.POSSIBLY_INCOMPLETE, 1)),
        (f"{label}: the accessor raises at its end: possibly incomplete",
         read([up, RuntimeError("no links")]), (live.POSSIBLY_INCOMPLETE, 1)),
        (f"{label}: what the check reads: no link at its end while the closing transport is still listed, "
         "possibly incomplete", read([up, []], [object()]), (live.POSSIBLY_INCOMPLETE, 1)),
        (f"{label}: no link before, a transport still listed: not made", read([[]], [object()]), (live.NOT_MADE, 0)),
    ]
    p = live.list_presence(_FakeSession([tok], [RuntimeError("x")]))  # type: ignore[arg-type]
    out.append((f"{label}: and says why", live.LINKS_UNREADABLE in p.reading, True))
    p = live.list_presence(_FakeSession([tok], [up, []]))  # type: ignore[arg-type]
    out.append((f"{label}: a read cut off while it ran says so", live.CUT_OFF_DURING in p.reading, True))
    # the registry: a closed watch session is dropped, never handed back
    reader, old = _FakeSession([], [up]), _FakeSession([], [up])
    live._OPENED[id(reader)] = (reader, "tcp/127.0.0.1:1", None)
    live._WATCHES[id(reader)] = (reader, old)
    old.closed = True
    live._prune()
    out.append((f"{label}: a closed watch session is pruned, so watch_session opens another",
                id(reader) in live._WATCHES, False))
    live._MARKED[id(old)] = old
    live._prune()
    out.append((f"{label}: a closed session's mark is pruned", live.marked(old), False))
    other, open_watch = _FakeSession([], [up]), _FakeSession([], [up])
    live._WATCHES[id(other)] = (reader, open_watch)  # an entry whose id another session now has
    live._MARKED[id(other)] = reader
    out.append((f"{label}: an entry under a reused id is never handed to another session",
                [live.watch_session(other), live.marked(other)], [None, False]))
    live._WATCHES.pop(id(other), None)
    live._MARKED.pop(id(other), None)
    gone, its_watch = _FakeSession([], [up]), _FakeSession([], [up])
    live._OPENED[id(gone)] = (gone, "tcp/127.0.0.1:1", None)
    live._WATCHES[id(gone)] = (gone, its_watch)
    gone.closed = True
    its_watch.close = lambda: setattr(its_watch, "closed", True)  # type: ignore[attr-defined]
    live._prune()
    out.append((f"{label}: a closed reading session takes its watch session with it",
                [id(gone) in live._WATCHES, id(gone) in live._OPENED, its_watch.closed], [False, False, True]))
    live.forget(reader)
    out.append((f"{label}: forget drops a session's entries", id(reader) in live._OPENED, False))
    return out


def _round_trip_readings() -> list[tuple[str, Any, Any]]:
    """Core 0.41 §8.1 in :mod:`zk2py.live` and :mod:`zk2py.presence`, on fake
    sessions and folds (#609, round 66): a read whose time reached its
    session's interests timeout is possibly incomplete; a round trip of a key
    under the subscribers' subtree, standing only when it ended at its final
    reply, no error reply, before the interests timeout, its session holding
    a link before and after; a read made while no round trip stands folded
    unordered; a transport's move unconfirming what the reader held."""
    from . import live
    from . import presence as pr

    tok = "zk2/lab/o/@zk/instance/" + "a" * 16
    up = [object()]
    out: list[tuple[str, Any, Any]] = []
    label = "core 0.41 §8.1, zk2py's reads at the interests timeout"

    def read(delay: float, its: float | None) -> tuple[str, bool]:
        f = _FakeSession([tok], [up, up], delay_s=delay)
        if its is not None:
            live.register(f, interests_timeout_s=its)
        p = live.list_presence(f)  # type: ignore[arg-type]
        live.forget(f)
        return p.kind, p.at_interests_timeout

    out += [
        (f"{label}: a read ended before its session's interests timeout is complete", read(0.0, 0.05),
         (live.COMPLETE, False)),
        (f"{label}: a read whose time reached it, its final reply and no error reply, is possibly incomplete",
         read(0.06, 0.05), (live.POSSIBLY_INCOMPLETE, True)),
        (f"{label}: a session zk2py did not record is taken at zenoh's default, 10 s",
         [live.interests_timeout_of(_FakeSession([], [up])), live.ZENOH_INTERESTS_TIMEOUT_S], [10.0, 10.0]),
        (f"{label}: zk2py's own sessions run with 120 s, the reference's tools' value", live.INTERESTS_TIMEOUT_S,
         120.0),
    ]
    f = _FakeSession([tok], [up, up], delay_s=0.06)
    live.register(f, interests_timeout_s=0.05)
    p = live.list_presence(f)  # type: ignore[arg-type]
    live.forget(f)
    out.append((f"{label}: and says why", live.AT_INTERESTS_TIMEOUT in p.reading, True))
    f = _FakeSession([], [up])
    live.register(f, namespace="ns", interests_timeout_s=60.0)
    out.append((f"{label}: what a session was opened with is recorded, since zenoh-python reads none back",
                [live.namespace_of(f), live.interests_timeout_of(f), live.namespace_of(_FakeSession([], [up]))],
                ["ns", 60.0, None]))
    live.forget(f)
    out.append((f"{label}: forget drops it", live.interests_timeout_of(f), 10.0))

    label = "core 0.41 §8.1, zk2py's round trip"

    def rt(links: list[Any], *, delay: float = 0.0, its: float | None = None, errors: int = 0,
               tokens: list[str] | None = None, selector: str | None = "zk2/*/*/@zk/instance/*") -> tuple[Any, ...]:
        f = _FakeSession(tokens or [], links, delay_s=delay, errors=errors)
        if its is not None:
            live.register(f, interests_timeout_s=its)
        r = live.settle(f, selector=selector)  # type: ignore[arg-type]
        live.forget(f)
        return r.stands, r.made, r.why, f.asked

    key = "zk2/*/*/@zk/settle"
    out += [
        (f"{label}: its key lies under the subscribers' presence subtree and matches no token",
         [live.settle_key("zk2/*/*/@zk/instance/*"), live.settle_key("zk2/lab/*/@zk/**"),
          live.settle_key("ns/zk2/*/*/@zk/instance/*"), live.settle_key(None), live.settle_key("zk2/**")],
         [key, "zk2/lab/*/@zk/settle", "ns/zk2/*/*/@zk/settle", key, key]),
        (f"{label}: its final reply, no error reply, links held before and after: it stands",
         rt([up, up]), (True, True, "", [key])),
        (f"{label}: a watch session cut off makes none: no GET, not made, cut off",
         rt([[]]), (False, False, live.CUT_OFF, [])),
        (f"{label}: one cut off while it ran does not stand", rt([up, []]),
         (False, True, live.CUT_OFF_DURING, [key])),
        (f"{label}: one that ended with an error reply (its timeout) does not stand", rt([up, up], errors=1)[:2],
         (False, True)),
        (f"{label}: one ended at the interests timeout does not stand, and was made: asked, it establishes nothing "
         "(0.44, F-240: \"as not standing\"; 0.41 said \"as not made\")",
         rt([up, up], delay=0.06, its=0.05), (False, True, live.AT_INTERESTS_TIMEOUT, [key])),
        (f"{label}: links that cannot be read before it: no GET, not standing (review of PR #876)",
         rt([RuntimeError("no links")]), (False, False, live.LINKS_UNREADABLE, [])),
        (f"{label}: links that cannot be read at its end leave it not standing",
         rt([up, RuntimeError("no links")]), (False, True, live.LINKS_UNREADABLE, [key])),
    ]

    label = "core 0.41 §8.1, zk2py's presence view"
    I = "zk2/lab/o/@zk/instance/" + "a" * 16
    K = "zk2/lab/p/@zk/instance/" + "c" * 16
    seed = [("ask", None), ("read", ("complete", [I, K]))]

    def at(ev: list[Any], addr: tuple[str, str] = ("lab", "o")) -> tuple[Any, bool]:
        v = pr.fold(ev)
        return v.address(*addr), v.owes_read

    out += [
        (f"{label}: a seed made while no round trip stands: what it shows is up, and an ordered read owed "
         "(review of PR #876)", at([("ask", None), ("read", ("complete", [I], False))]), ((pr.UP, ""), True)),
        (f"{label}: and an address it does not show is unobservable, never down",
         at([("ask", None), ("read", ("complete", [I], False))], ("lab", "p")), ((pr.UNOBSERVABLE, pr.NO_ROUND_TRIP), True)),
        (f"{label}: an unordered read not showing an address a complete read had held: unobservable, never down",
         at(seed + [("ask", None), ("read", ("complete", [I], False))], ("lab", "p")),
         ((pr.UNOBSERVABLE, pr.NO_ROUND_TRIP), True)),
        (f"{label}: an unordered read does not settle a withdrawal: still unconfirmed, still owed",
         at(seed + [("delete", I), ("ask", None), ("read", ("complete", [K], False))]),
         ((pr.UNOBSERVABLE, pr.WITHDRAWN), True)),
        (f"{label}: once a round trip stands, the complete read establishes it down",
         at(seed + [("delete", I), ("ask", None), ("read", ("complete", [K], False)), ("ask", None),
                    ("read", ("complete", [K]))]), ((pr.DOWN, ""), False)),
        (f"{label}: the watch session's transport moves: what the reader held is unobservable, a read owed (F-229)",
         [at(seed + [("move", None)]), at(seed + [("ask", None), ("read", ("complete", [I])), ("move", None)], ("lab", "p"))],
         [((pr.UNOBSERVABLE, pr.MOVED), True), ((pr.UNOBSERVABLE, pr.MOVED), True)]),
        (f"{label}: after a move, a read not made leaves it unobservable, still owed",
         at(seed + [("move", None), ("ask", None), ("read", ("not made", []))]), ((pr.UNOBSERVABLE, pr.MOVED), True)),
        (f"{label}: after a move, an unordered read showing it: up, an ordered read still owed",
         at(seed + [("move", None), ("ask", None), ("read", ("complete", [I], False))]), ((pr.UP, ""), True)),
        (f"{label}: after a move, an unordered read not showing it: unobservable, its latest read unordered, an "
         "ordered read still owed (review of PR #876)",
         at(seed + [("move", None), ("ask", None), ("read", ("complete", [K], False))]),
         ((pr.UNOBSERVABLE, pr.NO_ROUND_TRIP), True)),
        (f"{label}: then a complete read, a round trip standing: down, nothing owed",
         at(seed + [("move", None), ("ask", None), ("read", ("complete", [K], False)), ("ask", None),
                    ("read", ("complete", [K]))]), ((pr.DOWN, ""), False)),
        (f"{label}: after a move, a token heard put again: up",
         at(seed + [("move", None), ("put", I)]), ((pr.UP, ""), True)),
        (f"{label}: a withdrawal heard keeps its reason across a move",
         at(seed + [("delete", I), ("move", None)]), ((pr.UNOBSERVABLE, pr.WITHDRAWN), True)),
        (f"{label}: an address never heard of, under a complete read before a move: unobservable",
         at([("ask", None), ("read", ("complete", [])), ("move", None)]), ((pr.UNOBSERVABLE, pr.MOVED), True)),
    ]
    return out


def _watch_wire_readings() -> list[tuple[str, Any, Any]]:
    """Core 0.42 §8.1 in :mod:`zk2py.live` and :mod:`zk2py.presence`, on fake
    sessions (#609, round 67): a watch session in no namespace on the wire
    keys, each key heard stripped and one outside the namespace counted,
    never folded; a follower whose watch session is cut off making no read
    and no round trip, the read not made and the reason its watch
    session's; a read on a session zk2py cannot read the interests timeout
    of saying its completeness rests on zenoh's default."""
    import zenoh

    from . import live
    from . import presence as pr

    tok = "zk2/lab/o/@zk/instance/" + "a" * 16
    up = [object()]
    out: list[tuple[str, Any, Any]] = []
    label = "core 0.42 §8.1, zk2py's wire keys"
    sel = "zk2/*/*/@zk/instance/*"
    out += [
        (f"{label}: a selector on the wire keys, the namespace in front, and its round trip's key so too",
         [live.wire_selector("ns", sel), live.settle_key(live.wire_selector("ns", sel)), live.wire_selector(None, sel)],
         ["ns/" + sel, "ns/zk2/*/*/@zk/settle", sel]),
        (f"{label}: a namespace spelled with a slash at either end reads as its chunks, on the wire and stripped "
         "alike (review-bot on PR #879)",
         [live.wire_selector("ns/", sel), live.wire_selector("/a/b/", sel), live.strip_namespace("ns/", "ns/" + tok),
          live.strip_namespace("/a/b/", "a/b/" + tok), live.wire_selector("/", sel)],
         ["ns/" + sel, "a/b/" + sel, tok, tok, sel]),
        (f"{label}: every key heard stripped of the namespace; one outside it is not the deployment's",
         [live.strip_namespace("ns", "ns/" + tok), live.strip_namespace("ns", "other/" + tok),
          live.strip_namespace("ns", "nsx/" + tok), live.strip_namespace("ns", "ns/"), live.strip_namespace("ns", tok),
          live.strip_namespace("a/b", "a/b/" + tok), live.strip_namespace(None, tok)],
         [tok, None, None, None, None, tok, tok]),
    ]

    class _Sample:
        def __init__(self, key: str, kind: Any = None) -> None:
            self.key_expr, self.kind = key, kind if kind is not None else zenoh.SampleKind.PUT

    got: list[tuple[str, Any]] = []
    st = live.Stripped("ns", zenoh.handlers.Callback(lambda smp: got.append((str(smp.key_expr), smp.kind))))
    for k in ("ns/" + tok, "other/" + tok, "ns/" + tok.replace("lab", "far")):
        st(_Sample(k, zenoh.SampleKind.DELETE))
    out.append((f"{label}: Stripped hands on each sample with its key stripped and its kind kept, and counts one "
                "outside the namespace, never handing it on",
                [got, st.outside], [[(tok, zenoh.SampleKind.DELETE), (tok.replace("lab", "far"), zenoh.SampleKind.DELETE)],
                                    1]))
    # watch_or_beside: a namespaced reading session, its watch session in none
    reader, watch = _FakeSession([], [up]), _FakeSession([], [up])
    live._OPENED[id(reader)] = (reader, "tcp/127.0.0.1:1", None)
    live._WATCHES[id(reader)] = (reader, watch)
    live.register(reader, namespace="ns", interests_timeout_s=120.0)
    live.register(watch, interests_timeout_s=120.0)
    heard: list[str] = []
    _, placed = live.watch_or_beside(reader, "zk2/lab/*/@zk/instance/*", lambda smp: heard.append(str(smp.key_expr)))
    sel_d, handler, hist = watch.subscribers[0]
    handler.callback(_Sample("ns/" + tok))
    handler.callback(_Sample("elsewhere/" + tok))
    out.append((f"{label}: watch_or_beside in a namespace: the subscriber on the wire key, without history, its round "
                "trip on the wire key, keys handed on stripped, one outside counted; none on the reading session",
                [sel_d, hist, watch.asked, heard, placed.outside, placed.namespace, reader.subscribers, reader.asked],
                ["ns/zk2/lab/*/@zk/instance/*", False, ["ns/zk2/lab/*/@zk/settle"], [tok], 1, "ns", [], []]))
    live.forget(reader)
    live.forget(watch)
    # a caller's Callback's drop hook and mode pass through the stripping
    reader, watch = _FakeSession([], [up]), _FakeSession([], [up])
    live._OPENED[id(reader)] = (reader, "tcp/127.0.0.1:1", None)
    live._WATCHES[id(reader)] = (reader, watch)
    live.register(reader, namespace="ns", interests_timeout_s=120.0)
    live.register(watch, interests_timeout_s=120.0)
    dropped: list[bool] = []
    live.watch_or_beside(reader, sel, zenoh.handlers.Callback(lambda smp: None, lambda: dropped.append(True),
                                                              indirect=False))
    h2 = watch.subscribers[0][1]
    h2.drop()
    out.append((f"{label}: watch_or_beside in a namespace keeps the caller's drop hook and callback mode "
                "(review-bot on PR #879)", [dropped, h2.indirect], [[True], False]))
    live.forget(reader)
    live.forget(watch)

    label = "core 0.42 §8.1, zk2py's follower"

    def follower(reading_links: list[Any], watch_links: list[Any], *, ns: str | None = None,
                 watch_ns: str | None = None) -> tuple[Any, Any, Any]:
        r, w = _FakeSession([tok], reading_links), _FakeSession([], watch_links)
        live.register(r, namespace=ns, interests_timeout_s=120.0)
        live.register(w, namespace=watch_ns, interests_timeout_s=120.0)
        f = pr.PresenceFollower(r, w, sel, retry_s=60.0, max_retry_s=60.0)
        f.close()
        return f, r, w

    f, r, w = follower([up], [[]])
    out.append((f"{label}: its watch session cut off, its reading session linked: no round trip and no read asked "
                "on either, the read not made, its reason the watch session's (F-227, F-228)",
                [w.asked, r.asked, f.folds, f.reads[0].not_made, f.round_trips],
                [[], [], [(live.NOT_MADE, False)], live.WATCH_CUT_OFF, []]))
    out.append((f"{label}: and what the read would establish is unobservable, never down",
                [f.address("lab", "o"), f.address("lab", "p")[0]],
                [(pr.UNOBSERVABLE, pr.READ_NOT_MADE), pr.UNOBSERVABLE]))
    f, r, w = follower([up], [up])
    out.append((f"{label}: its watch session linked: the round trip on the watch session, then the read on the "
                "reading session, ordered", [w.asked, r.asked, f.folds, f.address("lab", "o"), f.address("lab", "p")],
                [["zk2/*/*/@zk/settle"], [sel], [(live.COMPLETE, True)], (pr.UP, ""), (pr.DOWN, "")]))
    # the links read by the attempt, before and after the round trip, and at the read's start, then gone
    f, r, w = follower([up], [up, up, up, up, []])
    out.append((f"{label}: the watch session's links gone at the read's end: the read unordered",
                [f.folds, f.address("lab", "p")], [[(live.COMPLETE, False)], (pr.UNOBSERVABLE, pr.NO_ROUND_TRIP)]))
    f, r, w = follower([up], [up], ns="ns")
    sub_sel, handler, hist = w.subscribers[0]
    handler.callback(_Sample("ns/" + tok, zenoh.SampleKind.DELETE))
    handler.callback(_Sample("far/" + tok.replace("lab", "far"), zenoh.SampleKind.PUT))
    out.append((f"{label}: in a namespace, its watch session in none: without history, on the wire key, the round "
                "trip on its wire key, each key heard stripped, one outside counted and never folded",
                [sub_sel, hist, w.asked, r.asked, f.address("lab", "o"), f.outside, f.view.address("far", "o")[0]],
                ["ns/" + sel, False, ["ns/zk2/*/*/@zk/settle"], [sel], (pr.UNOBSERVABLE, pr.WITHDRAWN), 1,
                 pr.DOWN]))
    f, r, w = follower([up], [up], ns="ns", watch_ns="ns")
    out.append((f"{label}: handed a watch session in the namespace, it keeps 0.41's history, base-relative",
                [w.subscribers[0][0], w.subscribers[0][2], w.asked], [sel, True, ["zk2/*/*/@zk/settle"]]))
    for x in (r, w):
        live.forget(x)

    label = "core 0.42 §8.1, an interests timeout zk2py cannot read"
    f = _FakeSession([tok], [up, up])
    p = live.list_presence(f)  # type: ignore[arg-type]
    rt = live.settle(_FakeSession([], [up, up]), selector=sel)  # type: ignore[arg-type]
    g = _FakeSession([tok], [up, up])
    live.register(g, interests_timeout_s=120.0)
    q = live.list_presence(g)  # type: ignore[arg-type]
    live.forget(g)
    out += [
        (f"{label}: a read on a session zk2py did not open is taken at zenoh's 10 s, and says its completeness rests "
         "on it", [p.kind, p.interests_timeout_s, p.interests_timeout_assumed, live.INTERESTS_TIMEOUT_ASSUMED in p.reading],
         [live.COMPLETE, 10.0, True, True]),
        (f"{label}: so does a round trip", [rt.stands, rt.interests_timeout_assumed], [True, True]),
        (f"{label}: a session whose value zk2py set says nothing of it",
         [q.interests_timeout_assumed, live.INTERESTS_TIMEOUT_ASSUMED in q.reading], [False, False]),
    ]
    return out


def _one_at_a_time_readings() -> list[tuple[str, Any, Any]]:
    """Core 0.44 §8.1 in :mod:`zk2py.live` and :mod:`zk2py.presence`, on fake
    sessions whose GETs end when the case opens their gate (#609, round 69;
    event-driven on review-bot's word on PR #885, so a slow runner can make
    a case wait, never fail it): one round trip, and one read, at a time,
    the next that needs one taking up the one in flight; a round trip asked
    before a move standing for nothing after it; one taken up after it
    ended standing only through a session that still holds a link; the
    round trip after a move waited for; and a caller's wait for the next
    change that drops nothing."""
    import zenoh

    from . import live
    from . import presence as pr

    tok = "zk2/lab/o/@zk/instance/" + "a" * 16
    sel = "zk2/*/*/@zk/instance/*"
    key = live.settle_key(sel)
    up = [object()]
    out: list[tuple[str, Any, Any]] = []

    def until(cond: Any, limit: float = 30.0) -> bool:
        end = time.monotonic() + limit
        while time.monotonic() < end:
            if cond():
                return True
            time.sleep(0.01)
        return bool(cond())

    label = "core 0.44 §8.1, zk2py's barrier"
    gate = threading.Event()
    w = _FakeSession([], [up], gate=gate)
    live.register(w, interests_timeout_s=60.0)
    b = live.Barrier(w, sel)
    three = [b.ask(0.0) for _ in range(3)]
    gate.set()
    fourth, fl4 = b.ask(30.0)
    out += [
        (f"{label}: its round trip asked four times in a row, the first three given up on at once and the fourth "
         "waited for: one GET, the three not standing, their wait ended, and the fourth taking the same one up and "
         "standing (presence.md §10 step 10)",
         [w.asked, [r.why == live.WAIT_ENDED and r.made and not r.stands for r, _ in three], fourth.stands,
          b.asked, b.taken_up, all(f is fl4 for _, f in three)],
         [[key], [True, True, True], True, 1, 3, True]),
        (f"{label}: a flight's own GET timeout is above its session's interests timeout, never below the caller's",
         [live.flight_timeout(w), live.flight_timeout(w, 90.0), live.flight_timeout(_FakeSession([], [up]))],
         [61.0, 90.0, live.ZENOH_INTERESTS_TIMEOUT_S + 1.0]),
    ]
    live.forget(w)
    # a move: the one in flight was asked before it, so the next asks anew,
    # and the first stands for nothing after the move
    gate = threading.Event()
    w = _FakeSession([], [up], gate=gate)
    live.register(w, interests_timeout_s=60.0)
    b = live.Barrier(w, sel)
    first, _ = b.ask(0.0)
    until(lambda: w.asked == [key])
    b.moved()
    second, _ = b.ask(0.0)
    until(lambda: w.asked == [key, key])
    gate.set()
    until(lambda: not b.in_flight)
    # the first's answer, had a caller still waited on it, against the move
    stale, _ = b.ask(30.0)
    out.append((f"{label}: a move while one is in flight: the next asks another (\"since its watch session's "
                "transport last moved\"), and the one asked after the move is the one taken up and stands",
                [w.asked, b.asked, first.why, second.why, stale.stands, b.taken_up],
                [[key, key], 2, live.WAIT_ENDED, live.WAIT_ENDED, True, 1]))
    gate = threading.Event()
    w = _FakeSession([], [up], gate=gate)
    live.register(w, interests_timeout_s=60.0)
    b = live.Barrier(w, sel)
    got: list[Any] = []
    t = threading.Thread(target=lambda: got.append(b.round_trip(30.0)))
    t.start()
    until(lambda: w.asked == [key])
    b.moved()
    gate.set()
    t.join(30.0)
    out.append((f"{label}: a round trip that ended standing, its caller waiting through a move: it stands for nothing "
                "after the move", [got[0].stands if got else None, got[0].why if got else None],
                [False, live.MOVED_DURING]))
    live.forget(w)
    # one that ended is not taken up once its answer was handed back
    w = _FakeSession([], [up])
    live.register(w, interests_timeout_s=60.0)
    b = live.Barrier(w, sel)
    a1, a2 = b.round_trip(30.0), b.round_trip(30.0)
    out.append((f"{label}: one whose answer was handed back is in flight no more: the next asks its own",
                [w.asked, a1.stands, a2.stands, b.taken_up], [[key, key], True, True, 0]))
    live.forget(w)
    # 0.45 (F-242): a wait that ended first takes up the round trip in
    # flight, "asking another only once that one has ended without
    # standing": its links gone at its end (cut off during it), then a new one
    # the links the fake hands out, in the order the barrier reads them: the
    # first GET's before (up) and after (gone: cut off during it), then the
    # second's before and after (up); a round trip taken up not standing
    # reads none (review-bot on PR #887)
    gate = threading.Event()
    w = _FakeSession([], [up, [], up, up], gate=gate)
    live.register(w, interests_timeout_s=60.0)
    b = live.Barrier(w, sel)
    gave_up = b.round_trip(0.0)
    asked = until(lambda: w.asked == [key])
    gate.set()
    ended = until(lambda: not b.in_flight)
    label242 = "core 0.45 §8.1 (F-242), zk2py's barrier"
    if not (asked and ended):
        # a fake GET that was never asked, or never ended, within 30 s
        out.append((f"{label242}: setup, the first GET asked and then ended within 30 s",
                    {"asked": asked, "ended": ended}, {"asked": True, "ended": True}))
    else:
        links_left_after_first = len(w.links)
        taken = b.round_trip(30.0)
        asked_then = list(w.asked)
        links_left_after_take_up = len(w.links)
        again = b.round_trip(30.0)
        out.append((f"{label242}: a wait that ended first, then the round trip in flight taken up and ended without "
                    "standing: no second GET until then, and another asked only after it, which stands",
                    {"first": gave_up.why, "taken up": (taken.stands, taken.why), "GETs then": asked_then,
                     "another stands": again.stands, "GETs in all": w.asked, "asked": b.asked,
                     "taken up count": b.taken_up},
                    {"first": live.WAIT_ENDED, "taken up": (False, live.CUT_OFF_DURING), "GETs then": [key],
                     "another stands": True, "GETs in all": [key, key], "asked": 2, "taken up count": 1}))
        out.append((f"{label242}: the fake's links read in the barrier's order: two by the first GET, none by the "
                    "take-up of one not standing, two by the second (its last one kept)",
                    [links_left_after_first, links_left_after_take_up, w.links], [2, 2, [up]]))
    live.forget(w)
    # one taken up after it ended: it stands only through a session that
    # still holds a link (a cut its listener has not told is in no count)
    for links_now, want in (([up, up, up], (True, "")), ([up, up, []], (False, live.CUT_OFF_SINCE))):
        gate = threading.Event()
        w = _FakeSession([], links_now, gate=gate)
        live.register(w, interests_timeout_s=60.0)
        b = live.Barrier(w, sel)
        b.ask(0.0)
        gate.set()
        until(lambda: not b.in_flight)
        r = b.round_trip(30.0)
        out.append((f"{label}: taken up after it ended standing, the watch session's links "
                    f"{'held' if want[0] else 'gone'} when it is taken up: "
                    f"{'it stands' if want[0] else 'it does not stand, cut off since'}", (r.stands, r.why), want))
        live.forget(w)
    # one counter: a barrier that reads its caller's count refuses a move told
    count = [0]
    b = live.Barrier(_FakeSession([], [up]), sel, gen=lambda: count[0])
    try:
        b.moved()
        refused = False
    except RuntimeError:
        refused = True
    count[0] += 1
    out.append((f"{label}: a barrier that reads its caller's move count follows it, and refuses to count a move "
                "itself (review-bot on PR #885: one counter)", [b.gen, refused], [1, True]))

    label = "core 0.44 §8.1, zk2py's follower"
    # its reading session's GET gated: the read's wait ends first, the read
    # runs on, and the next read takes it up, asking no second GET
    gate = threading.Event()
    r, w = _FakeSession([tok], [up], gate=gate), _FakeSession([], [up])
    live.register(r, interests_timeout_s=120.0)
    live.register(w, interests_timeout_s=120.0)
    f = pr.PresenceFollower(r, w, sel, timeout=0.05, retry_s=60.0, max_retry_s=60.0)
    after_start = [list(r.asked), f.read_waits_ended, list(f.folds), f.idle]
    gate.set()
    f.timeout = 30.0
    p = f.read()
    taken = [list(r.asked), f.reads_taken_up, list(f.folds), p.kind if p is not None else None, f.address("lab", "o")]
    f.read()
    again = list(r.asked)
    f.close()
    out += [
        (f"{label}: its read's wait ends first: no answer yet, the read in flight, not idle",
         after_start, [[sel], 1, [], False]),
        (f"{label}: the next read takes it up: one GET, folded once, ordered",
         taken, [[sel], 1, [(live.COMPLETE, True)], live.COMPLETE, (pr.UP, "")]),
        (f"{label}: one ended and folded is in flight no more: the next read asks its own", again, [sel, sel]),
    ]
    live.forget(r)
    live.forget(w)
    # a read that raised: every caller that takes it up gets the error
    f2 = pr.PresenceFollower(_FakeSession([tok], [up]), _FakeSession([], [up]), sel, retry_s=60.0, max_retry_s=60.0)
    boom = live.Flight(f2.gen, {"asked": 0, "ordered": True})
    f2._read_flight = boom
    f2.timeout = 30.0
    res: list[str] = []

    def taker() -> None:
        try:
            f2.read()
            res.append("no error")
        except RuntimeError as e:
            res.append(str(e))

    t = threading.Thread(target=taker)
    t.start()
    until(lambda: boom.taken_up == 1)
    # another caller had its answer first: the error, already consumed
    with f2.lock:
        boom.error, boom.consumed = RuntimeError("boom"), True
    boom.done.set()
    t.join(30.0)
    handed = res[0] if res else "no answer"
    f2.close()
    out.append((f"{label}: a read that raised, already taken up by another caller: the error is handed on, never "
                "None (review-bot on PR #885)", handed, "boom"))
    # after a move, its round trip waits until it ends: the wait at its start
    # (round_trip_timeout) ends first, the one after the move does not
    gate = threading.Event()
    r, w = _FakeSession([tok], [up]), _FakeSession([], [up], gate=gate)
    live.register(r, interests_timeout_s=120.0)
    live.register(w, interests_timeout_s=120.0)
    f = pr.PresenceFollower(r, w, sel, round_trip_timeout=0.05, retry_s=60.0, max_retry_s=60.0)
    start = [f.rt_waits_ended, list(f.folds[:1]), len(w.listeners)]
    gate.set()
    until(lambda: not f.barrier.in_flight)
    gate.clear()
    n_rt = len(f.round_trips)

    class _Event:
        kind = zenoh.SampleKind.PUT

    w.listeners[0].callback(_Event())
    # the round trip after the move is asked; it ends only once the gate is
    # set, well past round_trip_timeout: a wait of 0.05 s would have ended
    until(lambda: len(w.asked) == 2)
    time.sleep(0.3)
    gate.set()
    ok = until(lambda: f.folds[-1:] == [(live.COMPLETE, True)] and len(f.folds) >= 2)
    out += [
        (f"{label}: at its start the round trip's wait ends first, and its read is unordered",
         start, [1, [(live.COMPLETE, False)], 1]),
        (f"{label}: after a move it waits for the round trip (\"A reader SHOULD wait for the round trip after a "
         "move\"): no wait ended, the round trip stands, then an ordered read",
         [ok, f.rt_waits_ended, [bool(x) for x in f.round_trips[n_rt:]], len(w.asked)], [True, 1, [True], 2]),
    ]
    # its caller's wait for the next change: none, then one, never missed
    sub = w.subscribers[0][1]
    nothing = f.next_move(0.02)
    v = f.version
    sub.callback(_Sample(tok.replace("lab/o", "lab/q")))
    moved = f.next_move(30.0, since=v)
    f.close()
    out.append((f"{label}: its caller's wait for the next change ends with nothing when nothing moved, and drops "
                "nothing; a token heard since the version it had is a change, never missed",
                [nothing, moved is not None and moved > v, f.address("lab", "q")], [None, True, (pr.UP, "")]))
    live.forget(r)
    live.forget(w)
    return out


class _Sample:
    """A liveliness sample for a fake subscriber: its key and kind."""

    def __init__(self, key: str, kind: Any = None) -> None:
        import zenoh

        self.key_expr, self.kind = key, kind if kind is not None else zenoh.SampleKind.PUT


def family_readings(root: Path) -> list[Result]:
    """zk2py's own cases (:data:`READINGS`, and the window's and the
    scope's)."""
    from . import alarms

    out = [_check(f"{fid}: {note}", alarms.check_payload(payload), want) for fid, note, payload, want in READINGS]
    computed = _window_cases() + _scope_cases() + _reporter_transitions(root) + _reporter_conclusions(root) \
        + _reporter_loss() + _reporter_first_answers() + _reporter_gone() + _presence_dating_cases() \
        + _left_out_cases() + _first_neither_cases() + _telemetry_adopters(root) + _telemetry_copies() \
        + _telemetry_state_order() + _telemetry_nulls() + _telemetry_edges() + _telemetry_deep() \
        + _telemetry_verdicts() + _telemetry_tag_chains() + _presence_fold() + _desired_adopters(root) \
        + _desired_readings() + _presence_view() + _desired_encoding() + _round_trip_readings() \
        + _watch_wire_readings() + _one_at_a_time_readings() + _admin_records_readings() + _admin_read_readings() \
        + _timing_readings(root) + _arbitration_readings(root) + _unbounded_readings(root) \
        + _observer_readings(root) + _observer_02_readings(root) + _observer_04_readings(root) \
        + _observer_05_readings(root) + _observer_06_readings(root) + _observer_07_readings(root) \
        + _round88_readings(root) + _round89_readings(root) + _round90_readings(root) \
        + _round91_readings(root) + _round92_readings(root) + _mend93_readings(root)
    return out + [_check(note, got, want) for note, got, want in computed]


def _mend93_readings(root: Path) -> list[tuple[str, Any, Any]]:
    """Core 0.70 and ``observer.md`` 0.12, round 92's answers (#609, the mend
    after #926 part 5). Each text at its version or later, F-406 to F-413
    answered as they read and the forms they replaced gone; then what zk2py
    implements of them:

    - **F-406**, observer §7.3: a payload whose declared type the tool does
      not hold is sniffed, and the tool says which case it is, naming where
      it stopped resolving the key (:func:`zk2py.decoding.sniff`,
      :func:`zk2py.decoding.unheld`, each row of :mod:`zk2py.echo_tool`);
    - **F-407**, observer §4.1: the bound counts each time it forgets a key,
      and its note says forgettings (:func:`zk2py.seed.bound_note`);
    - **F-409**, observer §7.3: a profile's reader reads as its profile says,
      ``telemetry.v1``'s on its resource's ``encoding`` and ``alarms.v1``'s
      strict JSON, whatever the sample's ``Encoding``;
    - **F-412**, the decode order: a held type on the wire its sample's
      ``Encoding`` names (:func:`zk2py.decoding.wire_of`), which
      ``config.v1``'s reader now follows, its profile saying nothing of it;
    - **F-408**, ``state.md`` §7 step 2's premise, judged by
      :func:`zk2py.checks.premise`.

    No session is opened: a fake one stands in."""
    import re

    from . import alarms as al
    from . import checks as ck
    from . import config as cf
    from . import decoding as dc
    from . import echo_tool
    from . import freshness as fr
    from . import seed as sd
    from . import telemetry as tm

    spec = root.parent
    flat = lambda rel: " ".join((spec / rel).read_text(encoding="utf-8").split())  # noqa: E731

    def missing(text: str, *quotes: str) -> list[str]:
        return [q for q in quotes if q not in text]

    def present(text: str, *quotes: str) -> list[str]:
        return [q for q in quotes if q in text]

    def entry(rel: str, version: str, words: str) -> list[str]:
        log = (spec / rel).read_text(encoding="utf-8").splitlines()
        ok = any(line.startswith(f"## {version} ") and words in line for line in log)
        return [] if ok else [f"{rel} has no {version} entry saying {words!r}"]

    core, obs, rules, state = flat("core.md"), flat("observer.md"), flat("RULES.md"), flat("scenarios/state.md")
    m = re.search(r"\*\*Version (\d+)\.(\d+)\*\*", core)
    core_v = (int(m.group(1)), int(m.group(2))) if m else (0, 0)
    m = re.search(r"\| \*\*Text version\*\* \| (\d+)\.(\d+) \(", obs)
    obs_v = (int(m.group(1)), int(m.group(2))) if m else (0, 0)
    m = re.search(r"\| \*\*Written against\*\* \| core (\d+)\.(\d+) ", obs)
    against = (int(m.group(1)), int(m.group(2))) if m else (0, 0)
    out: list[tuple[str, Any, Any]] = [
        ("#926 part 5: core 0.70 or later, observer.md 0.12 or later written against core 0.70 or later, each "
         "with its entry",
         [core_v >= (0, 70), obs_v >= (0, 12), against >= (0, 70)]
         + entry("CHANGELOG.md", "0.70", "round 92's answers")
         + entry("observer-CHANGELOG.md", "0.12", "F-406"),
         [True, True, True]),
        ("F-406, F-407, F-409, F-412: observer §7.3's sniffing of a type not held and what the tool says, the "
         "decode order, a profile's reader, §4.1's count of each forgetting, §8.1's pointer; the old forms gone",
         missing(obs, "A payload whose declared type the tool does not hold is sniffed (§8.1). That is a foreign "
                      "key's, or a zk2 key's whose contract is not in hand or is unavailable (§2). The tool says "
                      "which, naming where it stopped resolving the key.",
                 "a payload whose declared type the tool holds is decoded by that type, on the wire its sample's "
                 "`Encoding` names (below). Any other payload is sniffed.",
                 "A profile that says how its reader reads a sample's bytes decides for that reader.",
                 "counts each time the bound forgets a key, and reports the count.",
                 "(§7.3's sniffing)")
         + present(obs, "A payload with no declared type, a foreign key's, is sniffed",
                   "counts the keys the bound forgot", "**Decode order** for a tool is:", "§7.3's last rung"), []),
        ("F-408, F-410, F-411, F-413: state.md §7 step 2's premise in both bullets, §4.3's bullet inside a seed, "
         "\"In both runs\", RULES.md's header; the old forms gone",
         [state.count("Rev 11 and rev 12 come within 5 s of rev 10.")]
         + missing(core, "An old epoch's value can arrive after the new epoch's for as long as the GET is open, not "
                         "only in transit.",
                   "It holds it until the key's next value.")
         + missing(state, "In both runs it then holds rev 10, until rev 12, which it accepts.")
         + missing(rules, "A verb the sentence leaves out is shown in brackets. They sit where it is left out.")
         + present(core, "the risk lasts the GET", "whatever the two times")
         + present(state, "It holds rev 10 both times")
         + present(rules, "So does a verb the sentence omits"), [2]),
    ]

    # -- F-406: a payload whose declared type the tool does not hold ----------
    data = "zk2/g/svc/zk2py_guard.v1/state/level"
    told = [dc.unheld(k) for k in (data, "rmw/robot1/cmd_vel", "zk2/g/svc/@zk/instance/0123456789abcdef")]
    out += [
        ("observer §7.3 (0.12, F-406): what a tool says of a payload whose type it does not hold: which case, and "
         "where it stopped resolving the key (decoding.unheld)",
         [(t["held"], t["which"], t["stopped"]) for t in told]
         + [(dc.unheld(data, unavailable=True)["which"], dc.unheld(data, unavailable=True)["stopped"])],
         [(False, "contract_not_in_hand", "contract"), (False, "foreign", "grammar"),
          (False, "no_contract_type", "form"), ("contract_unavailable", "contract")]),
        ("observer §7.3 (0.12, F-406): the zk2 key's words name its interface and resource, the contract not in "
         "hand, and never \"no contract\"; the unavailable one says unavailable (observer §2)",
         ["zk2py_guard.v1's state/level is not in hand" in told[0]["says"], "no contract of" in told[0]["says"],
          "is unavailable" in dc.unheld(data, unavailable=True)["says"], told[0]["iface"]],
         [True, False, True, "zk2py_guard.v1"]),
    ]
    rows = [echo_tool.row_of(data, False, 1, "text/plain", b"x"),
            echo_tool.row_of("rmw/robot1/cmd_vel", False, 7, "application/json", b'{"x":1}'),
            echo_tool.row_of(data, True, 0, "zenoh/bytes", b""),
            echo_tool.row_of(data, False, 1, "zenoh/bytes")]
    out.append(("observer §7.3 (0.12), echo_tool: each row says which case and where it stopped, and sniffs a put's "
                "payload; a delete has none; a row handed no payload sniffs nothing",
                [(r["zk2"], r["declared_type"]["which"], r["sniffed"]) for r in rows],
                [(True, "contract_not_in_hand", {"as": "text", "value": "x"}),
                 (False, "foreign", {"as": "json", "value": {"x": 1}}),
                 (True, "contract_not_in_hand", None), (True, "contract_not_in_hand", None)]))
    out.append(("observer §7.3 (0.12), echo_tool: its report's notes say every payload is sniffed and why",
                ["sniffed" in echo_tool.SNIFFED, "declared_type" in echo_tool.SNIFFED,
                 "SNIFFED" in __import__("inspect").getsource(echo_tool.main)], [True, True, True]))
    long = bytes(range(256)) * 2
    out.append(("observer §7.3, §8.1 (0.12): sniffing, a structural rendering: nothing, strict JSON, a CBOR map, a "
                "CBOR scalar only on CBOR's Encoding, printable text, else hexadecimal, at most 256 bytes shown",
                [dc.sniff(b""), dc.sniff(b' {"a": [1, 2]} '), dc.sniff(b"\xa1\x61a\x42\x01\x02", "zenoh/bytes"),
                 dc.sniff(b"\x01", "application/cbor"), dc.sniff(b"\x01"), dc.sniff(b"hello\n"),
                 dc.sniff(b'{"a":1,"a":2}'), {k: (len(v) if k == "value" else v) for k, v in dc.sniff(long).items()},
                 # review-bot on PR #994: text cut at a character, shown counting the bytes it shows
                 {k: (len(v) if k == "value" else v) for k, v in dc.sniff(b"a" + "é".encode() * 200).items()}],
                [{"as": "empty"}, {"as": "json", "value": {"a": [1, 2]}}, {"as": "cbor", "value": {"a": "h'0102'"}},
                 {"as": "cbor", "value": 1}, {"as": "hex", "value": "01"}, {"as": "text", "value": "hello\n"},
                 {"as": "text", "value": '{"a":1,"a":2}'}, {"as": "hex", "value": 512, "shown": 256},
                 {"as": "text", "value": 128, "shown": 255}]))

    # -- F-412: the decode order, a held type on its Encoding's wire ----------
    def undecodable(payload: bytes, encoding: str, declared: str = "json") -> str:
        try:
            dc.decode_json_type(payload, encoding, declared)
        except dc.Undecodable as e:
            return str(e).split(":", 1)[0]
        return "decoded"

    out += [
        ("observer §7.3 (0.12, F-412): a held JSON Schema type's wire is its sample's Encoding's, JSON or CBOR, and "
         "its contract's encoding when the Encoding names neither",
         [dc.wire_of("application/json", "cbor"), dc.wire_of("text/json", "cbor"), dc.wire_of("application/cbor"),
          dc.wire_of("zenoh/bytes", "cbor"), dc.wire_of("zenoh/bytes"), dc.wire_of(""),
          dc.wire_of("image/jpeg;x", "cbor")],
         ["json", "json", "cbor", "cbor", "json", "json", "cbor"]),
        ("observer §7.3 (0.12, F-412): decoded on that wire, never sniffed; one that does not decode says the wire "
         "it was read from (types.md §2: `{` on CBOR's Encoding or zenoh's default, read as CBOR)",
         [dc.decode_json_type(b'{"a":1}', "application/json", "cbor"),
          dc.decode_json_type(b"\xa1\x61a\x01", "application/cbor"),
          undecodable(b"{", "application/cbor"), undecodable(b"{", "zenoh/bytes", "cbor"),
          undecodable(b"{", "application/json", "cbor")],
         [{"a": 1}, {"a": 1}, "read as CBOR", "read as CBOR", "read as JSON"]),
    ]

    class Fake:
        """A session that declares nothing and whose GETs end at once with no
        reply."""

        def declare_subscriber(self, *a: Any, **k: Any) -> Any:
            return None

        def get(self, selector: str, handler: Any, **kw: Any) -> None:
            handler.drop()

        def liveliness(self) -> "Fake":
            return self

    def config_reading(payload: bytes, encoding: str, key: str = "state/view") -> Any:
        sub = fr.Subscriber(Fake(), "zk2/lab/rf0/**")
        sub._record(f"zk2/lab/rf0/config.v1/{key}",
                    fr.Received("put", sub.declared_ns, 0, payload, encoding, None, None, None))
        return cf.read(Fake(), "lab/rf0", subscriber=sub, presence=False, lists=True)

    def config_view(payload: bytes, encoding: str) -> list[Any]:
        r = config_reading(payload, encoding)
        return [r.view, [": ".join(u.split(": ")[1:3]) for u in r.undecodable]]

    out.append(("observer §7.3 (0.12, F-412): config.v1's reader, its profile silent on how it reads bytes, reads "
                "its view on the wire the sample's Encoding names: CBOR on CBOR's, JSON on JSON's, the contract's "
                "JSON on zenoh's default, and an undecodable one is no view, named with its wire",
                [config_view(b"\xa1\x68revision\x03", "application/cbor"),
                 config_view(b'{"revision": 3}', "application/json"), config_view(b'{"revision": 3}', "zenoh/bytes"),
                 config_view(b"\xa1\x68revision\x03", "zenoh/bytes")],
                [[{"revision": 3}, []], [{"revision": 3}, []], [{"revision": 3}, []],
                 [None, ["read as JSON: not UTF-8"]]]))
    # Review-bot on PR #994: a document that decodes to anything but an
    # object is undecodable too, and every answer that found no document
    # names what did not decode, never reading it as merely absent.
    bad_view = config_reading(b"[1, 2]", "application/json")
    bad_event = config_reading(b"7", "application/json", "events/change/01jc0000000000000000000000")
    answers = [cf.answer_pending(bad_view), cf.answer_writable(bad_view, "radio"),
               cf.answer_survives(bad_view, "radio", "frequency_hz"), cf.answer_converged(bad_view, 1),
               cf.answer_landed(bad_view, "t1", None), cf.answer_undo_clean(bad_event, "t1")]
    out.append(("config.v1's reader (review-bot on PR #994): a view or an event that decodes to a list or a number "
                "is no document, named undecodable with its wire; each §5 answer that found none says so, "
                "unobservable, never as if it were absent; an answer with nothing undecodable says nothing of it",
                [config_view(b"[1, 2]", "application/json"), bad_event.events,
                 [": ".join(u.split(": ")[1:3]) for u in bad_event.undecodable],
                 [(a[0], "undecodable, not absent" in a[1]) for a in answers],
                 "undecodable" in cf.answer_pending(config_reading(b'{"revision": 3}', "zenoh/bytes",
                                                                   "state/x"))[1]],
                [[None, ["read as JSON: an array, not an object"]], [], ["read as JSON: a number, not an object"],
                 [(cf.UNOBSERVABLE, True)] * 6, False]))

    # -- F-409: a profile's reader decides --------------------------------------
    import inspect

    doc = json.loads((spec / "profiles/alarms/conformance/documents.json").read_text())["cases"][0]["document"]

    def telemetry_read(payload: bytes, encoding: str) -> str:
        try:
            return f"decoded {tm.decode(payload, 'json:Point', encoding)}"
        except tm.Undecodable:
            return "undecodable"

    out.append(("observer §7.3 (0.12, F-409): a profile's reader reads as its profile says, whatever the sample's "
                "Encoding: telemetry.v1's on its resource's encoding (JSON bytes on a CBOR resource undecodable), "
                "alarms.v1's strict JSON, its decode taking no Encoding at all",
                [telemetry_read(b"1", "json"), telemetry_read(b'{"value": 1}', "cbor"),
                 telemetry_read(b"\x01", "cbor"), list(inspect.signature(al.decode).parameters),
                 al.decode(json.dumps(doc).encode()) is not None],
                ["decoded 1", "undecodable", "decoded 1", ["payload"], True]))

    # -- F-407: each forgetting counted -----------------------------------------
    lj = sd.LateJoiner(Fake(), "zk2/g/s/zk2py_plans.v1/state/plans/*", consumer=False, max_keys=1)
    sub = fr.Subscriber(Fake(), "zk2/g/s/x.v1/state/*", max_keys=1)
    for key in ("a", "b", "a", "b"):
        lj._merge(f"zk2/g/s/zk2py_plans.v1/state/plans/{key}", "live", False, key.encode(), "", "200/a1")
        sub._record(key, fr.Received("put", 0, 0, key.encode(), "", None, None, None))
    out.append(("observer §4.1 (0.12, F-407): the bound counts each time it forgets a key, two keys forgotten three "
                "times counting three, in a tool's late joiner and in the readers' recording; the note says "
                "forgettings",
                [lj.forgotten, sub.forgotten, lj.bound_note().split(" (", 1)[0], sd.bound_note(1, 4).split(" (")[0]],
                [3, 3, "3 forgettings by the merge's bound of 1 keys, each time it forgot a key counted",
                 "1 forgetting by the merge's bound of 4 keys, each time it forgot a key counted"]))

    # -- F-408: state.md §7 step 2's premise ------------------------------------
    S = sd.Stamp
    t10, behind, late = S(200 << 32, "a1"), S(196 << 32, "b2"), S(201 << 32, "b2")
    # Review-bot on PR #994: judged on the runner's clock for any offset,
    # never read from the stamps, which hold it only at an offset of 5 s.
    out.append(("state.md §7 step 2 (core 0.70, F-408): its premise, rev 11 and rev 12 within 5 s of rev 10, "
                "judged on the runner's clock whatever the offset and whatever the stamps say; a value not heard "
                "fails it",
                [ck.premise(-5.0, t10, behind, behind, 4.9)[0], ck.premise(-5.0, t10, behind, behind, 20.5)[0],
                 ck.premise(-5.0, t10, behind, late, 4.9)[0], ck.premise(-2.0, t10, late, late, 1.0)[0],
                 ck.premise(-10.0, t10, behind, behind, 7.0)[0], ck.premise(0.0, t10, late, late, 1.2)[0],
                 ck.premise(0.0, t10, late, late, 5.1)[0], ck.premise(-5.0, t10, behind, None, 1.0)[0],
                 "within 5 s" in ck.premise(-5.0, t10, behind, behind, 4.9)[1]],
                [True, False, True, True, False, True, False, False, True]))
    return out


def _round91_readings(root: Path) -> list[tuple[str, Any, Any]]:
    """Core 0.68 and ``observer.md`` 0.10, round 90's answers (#609, round 91;
    #926 part 3). Each text at its version or later, the rules F-379 to
    F-392 changed as they read, and the forms they replaced gone. Then the
    late joiner 0.68 asks for, which round 91 implements:
    - **a consumer** orders by S7 from its first value, its seed's replies
      included (core §4.2, §4.3): inside the seed, a value under another id
      is kept whichever path brought it, so the consumer holds the id whose
      value arrived last;
    - **a tool** keeps §4.2's merge for as long as it follows, and a tool
      that bounds what it holds counts what the bound forgot and treats a
      forgotten key as holding nothing (observer §4.1): zk2py's tool
      ``LateJoiner`` (:class:`zk2py.seed.Merge`), and the recording the
      alarms, health and config readers merge from
      (:class:`zk2py.freshness.Subscriber`), each bounded and counted;
    - **the readers' merge** (:func:`zk2py.seed.merged`): by time, then by
      id as ``Latest`` compares, across two ids too.
    Read with no session: a fake one stands in."""
    import re

    from . import freshness as fr
    from . import seed as sd

    spec = root.parent
    flat = lambda rel: " ".join((spec / rel).read_text(encoding="utf-8").split())  # noqa: E731

    def missing(text: str, *quotes: str) -> list[str]:
        return [q for q in quotes if q not in text]

    def present(text: str, *quotes: str) -> list[str]:
        return [q for q in quotes if q in text]

    def entry(rel: str, version: str, words: str) -> list[str]:
        log = (spec / rel).read_text(encoding="utf-8").splitlines()
        ok = any(line.startswith(f"## {version} ") and words in line for line in log)
        return [] if ok else [f"{rel} has no {version} entry saying {words!r}"]

    core, obs, rules = flat("core.md"), flat("observer.md"), flat("RULES.md")
    m = re.search(r"\*\*Version (\d+)\.(\d+)\*\*", core)
    core_v = (int(m.group(1)), int(m.group(2))) if m else (0, 0)
    m = re.search(r"\| \*\*Text version\*\* \| (\d+)\.(\d+) \(", obs)
    obs_v = (int(m.group(1)), int(m.group(2))) if m else (0, 0)
    m = re.search(r"\| \*\*Written against\*\* \| core (\d+)\.(\d+) ", obs)
    against = (int(m.group(1)), int(m.group(2))) if m else (0, 0)
    out: list[tuple[str, Any, Any]] = [
        ("#926 part 3: core 0.68 or later, observer.md 0.10 or later written against core 0.68 or later, each "
         "with its entry",
         [core_v >= (0, 68), obs_v >= (0, 10), against >= (0, 68)]
         + entry("CHANGELOG.md", "0.68", "round 90's answers")
         + entry("observer-CHANGELOG.md", "0.10", "F-379"),
         [True, True, True]),
        ("F-382 and F-383: a consumer by S7 from its first value, its seed's replies included, an id other than "
         "the held value's accepted, a tool's merge lasting and bounded; 0.67's sentences gone",
         missing(core,
                 "A consumer MUST order each key's values by time within one timestamp id, from the first value it "
                 "receives, a late joiner's seed replies included (§4.2).",
                 "It MUST accept a value under an id other than the one its held value carries, and order by time "
                 "within that id from then on.",
                 "A profile MAY order a consumer's values more strictly.",
                 # Re-read under core 0.69 and observer 0.11 (round 92): F-401
                 # split the merge's MUST, F-403 reworded "no end to state" and
                 # F-398 the bound, each the same rule as 0.68 stated it.
                 "Across two timestamp ids, a consumer MUST instead keep what §4.3's consumer rule accepts.",
                 "So a consumer has no instant at which its merge ends")
         + missing(obs, "**A tool's merge lasts.** A tool keeps core §4.2's merge for as long as it follows, "
                   "across two timestamp ids too.",
                   # Re-read under core 0.70 and observer 0.12 (the mend after
                   # #926 part 5): F-407 says the bound counts each forgetting.
                   "A tool that bounds what it holds counts each time the bound forgets a key, and reports the "
                   "count. A forgotten key holds nothing")
         + present(core, "merges as §4.2 says, and the consumer rule", "It MUST accept the first value under a new id",
                   "A consumer MUST order an owner's values"), []),
        ("F-379, F-380, F-381 and F-387: §7.3 by each payload's declared type, §2.4's opt-in, §2.5's places, and "
         "§8.4's two MUSTs naming a caller or tool; the old forms gone",
         missing(obs, "**What a tool that shows or judges a payload decodes** (core §7.1). Such a tool MUST decode "
                      "each payload by its declared type:",
                 "A tool that shows a payload it cannot decode MUST render it as its declared type and size")
         + missing(core, "An owner that opts into it (§0) sets it. One that does not, or whose binding offers no such "
                         "API, MAY publish reliable, zenoh's default.",
                   "so `history` is one of the places where a participant opts into an unstable API (§0).",
                   "A caller or tool MUST set consolidation `None` on both attempts.",
                   "A caller or tool MUST NOT depend on the encoding: the hash is the check.")
         + present(core, "One that cannot set it", "`history` is the one place", "A caller MUST set consolidation",
                   "so none moves by it")
         + present(obs, "What a generic tool decodes", "A tool MUST render a kind it cannot decode"), []),
        ("F-384 and F-386 in RULES.md: S2's \"It [the owner]\", §2.4's MAY after *Except:*",
         missing(rules, "It [the owner] MUST answer a selector over them [its interfaces' `state/**` and "
                        "`@state/**`]",
                 "*Except:* One [an owner] that does not [opt into `reliability`], or whose binding offers no such "
                 "API, MAY publish reliable, zenoh's default."), []),
    ]

    st = lambda t, i: sd.Stamp(t, i)  # noqa: E731

    class Fake:
        def declare_subscriber(self, *a: Any, **k: Any) -> Any:
            return None

    k = "zk2/g/s/zk2py_plans.v1/state/plans/r"
    sel = "zk2/g/s/zk2py_plans.v1/state/plans/*"

    def inside(consumer: bool, live_first: bool) -> list[Any]:
        """Two ids inside one seed: the reply under the old epoch's id, the
        live sample under the new one's, its clock behind."""
        lj = sd.LateJoiner(Fake(), sel, consumer=consumer)
        old = (k, "reply", False, b"rev10", "application/json", "200/a1")
        new = (k, "live", False, b"rev11", "application/json", "150/b2")
        for x in ((new, old) if live_first else (old, new)):
            lj._merge(*x)
        lj._ended()
        return [[(a.outcome, a.rule) for a in lj.applied], lj.view()[k].payload.decode()]

    out.append(("core §4.2, §4.3 (0.68): inside a seed, a consumer keeps the id whose value arrived last, its seed's "
                "reply included (rev 11 live after rev 10's reply; rev 10's reply after rev 11 live); a tool keeps "
                "the later time either way (observer §4.1)",
                [inside(True, False), inside(True, True), inside(False, False), inside(False, True)],
                [[[("kept", "S7"), ("kept", "S7")], "rev11"], [[("kept", "S7"), ("kept", "S7")], "rev10"],
                 [[("kept", "merge"), ("dropped", "merge")], "rev10"],
                 [[("kept", "merge"), ("kept", "merge")], "rev10"]]))
    out.append(("core §4.3 (0.68): within one id a consumer's order is the stamp merge, equal stamps one mutation; "
                "an unstamped value under no id never over a stamped one (seed.consumer_kept)",
                [sd.consumer_kept([st(101, "a1"), st(104, "a1")]), sd.consumer_kept([st(104, "a1"), st(101, "a1")]),
                 sd.consumer_kept([st(103, "a1"), st(103, "a1")]), sd.consumer_kept([st(101, "a1"), None]),
                 sd.consumer_kept([None, st(101, "a1")])],
                [[True, True], [True, False], [True, False], [True, False], [True, True]]))

    # observer §4.1 (0.10): a tool that bounds what it holds counts what the
    # bound forgot, and treats a forgotten key as holding nothing.
    def bounded() -> list[Any]:
        lj = sd.LateJoiner(Fake(), sel, consumer=False, max_keys=2)
        for key, ts in (("a", "200/a1"), ("b", "200/a1"), ("c", "200/a1"), ("a", "150/a1")):
            lj._merge(f"zk2/g/s/zk2py_plans.v1/state/plans/{key}", "live", False, key.encode(), "", ts)
        return [lj.forgotten, sorted(x.rsplit("/", 1)[1] for x in lj.view()), [a.outcome for a in lj.applied],
                lj.merge.last_forgotten.rsplit("/", 1)[1]]

    out.append(("observer §4.1 (0.10): a tool's LateJoiner bounded at two keys forgets the least recent, counts each "
                "forgetting, and keeps the next value of a forgotten key, its stamp older, as on a key holding "
                "nothing; a tool's is bounded by default, a consumer's not",
                [bounded(), sd.LateJoiner(Fake(), sel, consumer=False).merge.max_keys,
                 sd.LateJoiner(Fake(), sel, consumer=True).merge.max_keys],
                [[2, ["a", "c"], ["kept", "kept", "kept", "kept"], "b"], sd.MAX_KEYS, None]))

    def recording() -> list[Any]:
        sub = fr.Subscriber(Fake(), "zk2/g/s/x.v1/state/*", max_keys=2)
        for key in ("a", "b", "a", "c"):
            sub._record(key, fr.Received("put", 0, 0, key.encode(), "", None, None, None))
        return [sub.forgotten, sub.keys(), len(sub.of("a")), sub.of("b")]

    out.append(("observer §4.1 (0.10): freshness.Subscriber, the recording the alarms, health and config readers "
                "merge from, bounded at two keys: the least recently arrived forgotten whole, counted, and holding "
                "nothing; bounded by default",
                [recording(), fr.Subscriber(Fake(), "zk2/g/s/x.v1/state/*").max_keys],
                [[1, ["a", "c"], 2, []], sd.MAX_KEYS]))
    # Review-bot on PR #971: a consumer refuses a bound, and a reader's
    # observation of a key it holds nothing of is dated from the last
    # forgetting, never from the declaration.
    def refused(make: Any) -> str:
        try:
            make()
        except ValueError as e:
            return "refused" if "a tool's" in str(e) else str(e)
        return "accepted"

    def forgotten_view() -> list[Any]:
        sub = fr.Subscriber(Fake(), "zk2/g/s/x.v1/state/*", max_keys=2)
        d = sub.declared_ns
        s_ = fr.NS
        for key, at in (("a", 1), ("b", 2), ("c", 3)):
            sub._record(key, fr.Received("put", d + at * s_, 0, key.encode(), "", None, None, None))
        now = d + 10 * s_
        h = fr.Horizon("within", 5)
        return [(sub.forgot_ns - d) // s_, sub.observation("a", now).listened_ns // s_,
                sub.observation("z", now).listened_ns // s_, sub.observation("c", now).listened_ns // s_,
                fr.judge_subscription(h, sub.observation("a", d + 7 * s_)).reason,
                fr.judge_subscription(h, sub.observation("a", now)).reason]

    out.append(("observer §4.1 (0.10), review-bot on PR #971: a consumer's late joiner and S7's merge refuse a bound, "
                "a tool's take one",
                [refused(lambda: sd.LateJoiner(Fake(), sel, consumer=True, max_keys=2)),
                 refused(lambda: sd.Merge("S7", 2)), refused(lambda: sd.LateJoiner(Fake(), sel, consumer=False,
                                                                                    max_keys=2)),
                 refused(lambda: sd.Merge("merge", 2))],
                ["refused", "refused", "accepted", "accepted"]))
    out.append(("observer §4.1 (0.10), F-398, review-bot on PR #971: a Subscriber dates its listening to a key it "
                "holds nothing of, forgotten or never heard, from its last forgetting: a's silence before it is never "
                "read as heard (ttl 5 s: listened_too_short 4 s after it, no_delivery only 7 s after it); a held key "
                "from the declaration",
                forgotten_view(), [3, 7, 7, 10, "listened_too_short", "no_delivery"]))
    out.append(("core §4.2 and observer §4.1 (0.10): the readers' merge (seed.merged), across two ids the later "
                "time, at one time the id Latest ranks higher (2 over 100), a delete kept by its stamp, an "
                "unstamped value never over a stamped one",
                [sd.merged([(st(200, "a1"), False, b"old"), (st(150, "b2"), False, b"new")])[2],
                 sd.merged([(st(5, "2"), False, b"x"), (st(5, "100"), False, b"y")])[2],
                 sd.merged([(st(5, "a1"), False, b"x"), (st(6, "a1"), True, b"")])[1],
                 sd.merged([(st(5, "a1"), False, b"x"), (None, False, b"u")])[2], sd.merged([])],
                [b"old", b"x", True, b"x", None]))
    return out


def _round92_readings(root: Path) -> list[tuple[str, Any, Any]]:
    """Core 0.69 and ``observer.md`` 0.11, round 91's answers (#609, round 92;
    #926 part 4). What zk2py implements of them: observer §4.1's bound, "A
    tool that bounds what it holds counts the keys the bound forgot, and
    reports the count" (F-398). Round 91 counted and showed nothing. Now
    every reading of a bounded merge or recording carries the count:
    :func:`zk2py.seed.bound_note`, a tool's :meth:`zk2py.seed.LateJoiner.bound_note`,
    :meth:`zk2py.freshness.Subscriber.note`, and ``forgotten`` on the
    alarms, health and config readings and on
    :func:`zk2py.freshness.read_service`'s answer. Read with no session: a
    fake one stands in, whose GETs end at once with no reply.

    Before it, each text at its version or later, round 91's F-393 to F-405
    answered as they read, and the forms they replaced gone; and
    ``state.md`` §7 step 2's new step inside the seed, as 0.69 writes it
    (F-393), on the fake session: the consumer holds rev 10 until rev 12; the
    tool keeps the later time."""
    import re

    from . import alarms as al
    from . import config as cf
    from . import freshness as fr
    from . import health as hl
    from . import seed as sd

    spec = root.parent
    flat = lambda rel: " ".join((spec / rel).read_text(encoding="utf-8").split())  # noqa: E731

    def missing(text: str, *quotes: str) -> list[str]:
        return [q for q in quotes if q not in text]

    def present(text: str, *quotes: str) -> list[str]:
        return [q for q in quotes if q in text]

    def entry(rel: str, version: str, words: str) -> list[str]:
        log = (spec / rel).read_text(encoding="utf-8").splitlines()
        ok = any(line.startswith(f"## {version} ") and words in line for line in log)
        return [] if ok else [f"{rel} has no {version} entry saying {words!r}"]

    core, obs, rules = flat("core.md"), flat("observer.md"), flat("RULES.md")
    readme, state, types = flat("README.md"), flat("scenarios/state.md"), flat("scenarios/types.md")
    m = re.search(r"\*\*Version (\d+)\.(\d+)\*\*", core)
    core_v = (int(m.group(1)), int(m.group(2))) if m else (0, 0)
    m = re.search(r"\| \*\*Text version\*\* \| (\d+)\.(\d+) \(", obs)
    obs_v = (int(m.group(1)), int(m.group(2))) if m else (0, 0)
    m = re.search(r"\| \*\*Written against\*\* \| core (\d+)\.(\d+) ", obs)
    against = (int(m.group(1)), int(m.group(2))) if m else (0, 0)
    out: list[tuple[str, Any, Any]] = [
        ("#926 part 4: core 0.69 or later, observer.md 0.11 or later written against core 0.69 or later, each "
         "with its entry",
         [core_v >= (0, 69), obs_v >= (0, 11), against >= (0, 69)]
         + entry("CHANGELOG.md", "0.69", "round 91's answers")
         + entry("observer-CHANGELOG.md", "0.11", "F-397"),
         [True, True, True]),
        ("F-393: inside a seed the residual risk lasts the GET, the consumer holding the answering instance's id "
         "until the key's next value, and state.md §7 step 2's step inside the seed",
         # Re-read under core 0.70 (the mend after #926 part 5): F-410
         # reworded §4.3's bullet and F-411 state.md's "both times", each the
         # same rule as 0.69 stated it.
         missing(core, "**Inside a seed.** An old epoch's value can arrive after the new epoch's for as long as the "
                       "GET is open, not only in transit.",
                 "So a consumer holds the reply's value, whether its time or the live samples' is later. It holds it "
                 "until the key's next value.",
                 "Where two instances answer, it receives only the reply `Latest` kept, the one with the greater "
                 "stamp.")
         + missing(state, "**Inside the seed.**",
                   "The consumer accepts rev 11, then the reply, which carries the old id. In both runs it then holds "
                   "rev 10, until rev 12, which it accepts.",
                   "The tool keeps the later time: rev 10 with the clock behind, and rev 11, then rev 12, on the "
                   "host's clock."), []),
        ("F-395, F-396, F-401 to F-404: §0's static rules, §8.4's retriever, §4.2's merge in three sentences, "
         "\"order differently\", \"no instant\", \"holds a value under the id\"; the old forms gone",
         missing(core, "They are the rules of §1 and §9 that name no role",
                 "A holder on the retrieving session answers too, so a caller or tool can get its own reply",
                 "so that a caller or tool following the steps would report the contract unavailable",
                 "or at the timeout the caller or tool gave it (§8.1).",
                 "It MUST keep the newer by the owner's stamp (S1, S2). Across two timestamp ids, a consumer MUST "
                 "instead keep what §4.3's consumer rule accepts.",
                 "Across them a consumer and a tool order differently (below).",
                 "So a consumer has no instant at which its merge ends",
                 "Across two ids, a consumer holds a value under the id of the last value it received")
         + present(core, "They are §1's, §9's", "A holder on the caller's own session", "at the caller's timeout",
                   "and keep the newer by the owner's stamp (S1, S2), or, for a consumer",
                   "there a consumer and a tool part", "has no end to state", "keeps the one whose value arrived last"),
         []),
        ("F-397 and F-398: observer §7.3's decode order by the declared type, the Encoding picking its wire, "
         "sniffing with no declared type; §4.1's bound counting the keys and reporting the count; types.md §2's "
         "Encoding; the old forms gone",
         # Re-read under observer 0.12 (the mend after #926 part 5): F-412
         # reworded the decode order, F-406 placed a zk2 key whose contract is
         # not in hand, and F-407 the bound's count.
         missing(obs, "**Decode order** for a tool: a payload whose declared type the tool holds is decoded by that "
                      "type, on the wire its sample's `Encoding` names (below). Any other payload is sniffed.",
                 "A payload whose declared type the tool does not hold is sniffed (§8.1).",
                 "So a state declared `cbor` whose sample's `Encoding` is JSON is read as JSON.",
                 "A tool that bounds what it holds counts each time the bound forgets a key, and reports the count.")
         + missing(types, "The second carries CBOR's `Encoding` or zenoh's default. Its reason says it was read as "
                          "CBOR.")
         + present(obs, "the sample's `Encoding`, then the contract's type, then sniffing",
                   "counts what the bound forgot, and treats"), []),
        ("F-394, F-399, F-400, F-405: S7's *Except:*, the glossary's Epoch and Static rule, the index's header; "
         "the old forms gone",
         missing(rules, "*Except:* A profile MAY order a consumer's values more strictly.",
                 "It is listed beside a rule it excepts. A row's MAY is in the rule ids.",
                 # Re-read under core 0.70 (the mend after #926 part 5): F-413's rewrite.
                 "A verb the sentence leaves out is shown in brackets. They sit where it is left out.",
                 "A rule of those sections, §1 and §9 included, that names a role is listed under that role.")
         + missing(readme, "With neither, §4.3 requires a new instance whose session has a fresh, unpinned zid",
                   "A rule of any of those sections, §1 and §9 included, that names a role is that role's")
         + present(rules, "It is listed only beside a rule it excepts.", "So is one of §1 or §9.")
         + present(readme, "With no record of its last stamp, §4.3 requires a fresh zid"), []),
    ]

    class Fake:
        """A session that declares nothing and whose GETs, liveliness ones
        included, end at once with no reply."""

        def declare_subscriber(self, *a: Any, **k: Any) -> Any:
            return None

        def get(self, selector: str, handler: Any, **kw: Any) -> None:
            handler.drop()

        def liveliness(self) -> "Fake":
            return self

    sel = "zk2/g/s/zk2py_plans.v1/state/plans/*"
    k = "zk2/g/s/zk2py_plans.v1/state/plans/r"

    def inside(consumer: bool, behind: bool) -> list[Any]:
        """state.md §7 step 2 inside the seed (0.69): rev 11 live under the new
        id, the reply of rev 10 at the GET's end, then rev 12 under the new id;
        the new clock 5 s behind or on the host's clock, rev 12 written within
        the offset of rev 10."""
        lj = sd.LateJoiner(Fake(), sel, consumer=consumer)
        t10, t11, t12 = (200, 150, 151) if behind else (200, 201, 202)
        for via, rev, t, i in (("live", b"rev11", t11, "b2"), ("reply", b"rev10", t10, "a1"),
                               ("live", b"rev12", t12, "b2")):
            lj._merge(k, via, False, rev, "application/json", f"{t}/{i}")
        lj._ended()
        return [[a.outcome for a in lj.applied], lj.view()[k].payload.decode()]

    out.append(("state.md §7 step 2 inside the seed (core 0.69, F-393): the consumer accepts rev 11, then the reply "
                "under the old id, and holds rev 10 until rev 12; the tool keeps the later time, rev 10 with the "
                "clock behind, rev 11 then rev 12 on the host's clock",
                [inside(True, True), inside(True, False), inside(False, True), inside(False, False)],
                [[["kept", "kept", "kept"], "rev12"], [["kept", "kept", "kept"], "rev12"],
                 [["kept", "kept", "dropped"], "rev10"], [["kept", "dropped", "kept"], "rev12"]]))

    # Review-bot on PR #973: the held GET's queries are dropped once, at
    # release or at once after it. Round 92 also read the step's clock-behind
    # outcome past the offset as inconclusive (F-408); core 0.70 states the
    # premise, which :func:`zk2py.checks.premise` judges since the mend after
    # #926 part 5 (its readings are that mend's).
    from . import checks as ck

    class Q:
        def __init__(self) -> None:
            self.dropped = 0

        def drop(self) -> None:
            self.dropped += 1

    hq, q1, q2 = ck.HeldQueries(limit=60.0), Q(), Q()
    hq.take(q1)
    held_before = q1.dropped
    hq.release()
    hq.release()
    hq.take(q2)
    out.append(("state.md §7 (0.69), review-bot on PR #973: a held query dropped once, at release, and one taken "
                "after it at once",
                [held_before, q1.dropped, q2.dropped, hq.done], [0, 1, 1, True]))

    def joiner() -> list[Any]:
        lj = sd.LateJoiner(Fake(), sel, consumer=False, max_keys=2)
        for key in ("a", "b", "c", "d", "a"):
            lj._merge(f"zk2/g/s/zk2py_plans.v1/state/plans/{key}", "live", False, key.encode(), "", "200/a1")
        return [lj.forgotten, lj.bound_note()]

    out.append(("observer §4.1 (0.11): a tool's late joiner reports the count of keys its bound forgot (bounded at "
                "two, five arrivals on four keys, a forgotten again after it came back: three forgettings), and what "
                "the count costs; a consumer's says it has no bound",
                [joiner(), sd.LateJoiner(Fake(), sel, consumer=True).bound_note(),
                 sd.bound_note(0, sd.MAX_KEYS)],
                # The note counts forgettings since observer 0.12's F-407 (the
                # mend after #926 part 5); round 92's said "3 keys forgotten".
                [[3, "3 forgettings by the merge's bound of 2 keys, each time it forgot a key counted (observer "
                     "§4.1): a forgotten key's next value was kept whatever its stamp, so the merge did not order "
                     "every key"],
                 "the merge holds every key it heard: it has no bound (observer §4.1)",
                 f"0 forgettings by the merge's bound of {sd.MAX_KEYS} keys, each time it forgot a key counted "
                 f"(observer §4.1)"]))

    def recorded(address: str, keys: list[str]) -> fr.Subscriber:
        sub = fr.Subscriber(Fake(), f"zk2/{address}/**", max_keys=1)
        for i, key in enumerate(keys):
            sub._record(key, fr.Received("put", sub.declared_ns + i, 0, b"{}", "application/json", None, None,
                                         None))
        return sub

    hk = "zk2/lab/svc/health.v1/state/status"
    h = hl.read_across(recorded("lab/svc", ["zk2/lab/svc/health.v1/state/checks/x", hk]), "lab/svc", True)
    ck = "zk2/lab/rf0/config.v1/state/view"
    c = cf.read(Fake(), "lab/rf0", subscriber=recorded("lab/rf0", ["zk2/lab/rf0/config.v1/state/status", ck]),
                presence=False, lists=True)
    ak = f"zk2/lab/al/{al.PROFILE}/{al.TOKEN}/{al.PREFIX}/k1"
    a = al.read_owner(Fake(), "lab/al", subscriber=recorded("lab/al", [ak[:-1] + "0", ak]), get=False)
    srv = fr.read_service(Fake(), "lab/svc", [], 0.0, fr.ClockTrust(word=True))
    out.append(("observer §4.1 (0.11): each reader's reading reports the count its recording's bound forgot "
                "(health.v1's §2.11 reader, config.v1's §5 reader, alarms.v1's reader, freshness.v1's tool), "
                "beside what it holds: the key heard last",
                [[h.forgotten, h.level is not None], [c.forgotten, c.bound.startswith("1 forgetting by"),
                                                      c.view is not None],
                 [a.forgotten, a.bound.startswith("1 forgetting by"), sorted(a.readings) == [ak]],
                 [isinstance(srv, dict), srv.forgotten, srv.note.startswith("0 forgettings by")]],
                [[1, True], [1, True, True], [1, True, True], [True, 0, True]]))
    return out


def _observer_07_readings(root: Path) -> list[tuple[str, Any, Any]]:
    """``observer.md`` text 0.7, core 0.63, and ``alarms.v1`` 0.9,
    ``telemetry.v1`` 0.6 and ``desired.v1`` 0.9 without history (#609, round
    87; #935's fourth part). The texts' answers to round 86's F-324, F-325 and
    F-326, as they read. Then the desired.v1 sentences 0.9 split whose rules
    zk2py's target implements, each read by :class:`zk2py.desired.TargetRole`
    as the new sentence states it. No session is opened."""
    import re

    from . import alarms
    from . import desired as ds

    spec = root.parent
    flat = lambda rel: " ".join((spec / rel).read_text(encoding="utf-8").split())  # noqa: E731
    ns = 1_000_000_000
    observer, core, alarms_text = flat("observer.md"), flat("core.md"), flat("profiles/alarms/v1.md")

    def missing(text: str, *quotes: str) -> list[str]:
        """The quotes the text does not hold, so a failing reading names them
        (review-bot on PR #961)."""
        return [q for q in quotes if q not in text]

    def alarms_from_09() -> list[str]:
        """``alarms.v1`` at text 0.9 or later, and its CHANGELOG's 0.9 entry
        naming the watch after a window: what is missing. #935's next part
        moves the version on, which this reading allows."""
        gaps: list[str] = []
        m = re.search(r"\| \*\*Text version\*\* \| (\d+)\.(\d+) \(", alarms_text)
        if m is None:
            gaps.append("alarms.v1's header names no text version")
        elif (int(m.group(1)), int(m.group(2))) < (0, 9):
            gaps.append(f"alarms.v1's text version {m.group(1)}.{m.group(2)}, below 0.9")
        log = (spec / "profiles/alarms/CHANGELOG.md").read_text(encoding="utf-8").splitlines()
        if not any(line.startswith("## 0.9 ") and "the watch after a window" in line for line in log):
            gaps.append("alarms.v1's CHANGELOG has no 0.9 entry naming the watch after a window")
        return gaps

    out: list[tuple[str, Any, Any]] = [
        ("observer 0.7 §5.2 (F-324): the wait_before_close half says why a path that stays up tells a withdrawal "
         "late, and that it is a bound reasoned, not measured",
         missing(observer, "A path that stays up while its queue is full keeps its lease.",
                 "**A bound reasoned, not measured.**",
                 "No run has shown one later than a lease (core Appendix B, \"Not measured\")."), []),
        ("observer 0.7 §5.2: the watch's length is the one 0.6 settled, word for word",
         missing(observer, "a tool watches the owner's instance tokens for the longer of one lease and the routers' "
                 "`wait_before_close`, plus its reply timeout."), []),
        ("core 0.63 Appendix B (F-324): the line that says what was not measured",
         missing(core, "**Not measured: a withdrawal later than one lease, under a `wait_before_close` longer than "
                 "the lease.**"), []),
        ("observer 0.7 §1 (F-325): the second spelling's sentence names NotAsked's bullet",
         missing(observer, "What NotAsked's bullet says above (nobody put the question, the tool has learned "
                 "nothing, and it renders no answer) does not hold of that spelling."), []),
        ("alarms.v1 0.9 (F-326): §2.15's watch after a window is held by a version of its own, 0.9 or later, and "
         "still cites observer §5.2",
         alarms_from_09() + missing(alarms_text, "The owner is watched past the end as observer §5.2 says: the "
                                    "longer of one lease and the routers' `wait_before_close`, plus the reply "
                                    "timeout."), []),
        ("alarms.v1 0.9 §2.15 in AlarmsReader: a reader judging at 100 s still ends its window at 89 s under "
         "zenoh's defaults, and at 94 s beside a 1 s lease",
         [alarms.window_end_ns(100 * ns) / ns, alarms.window_end_ns(100 * ns, 1.0, 5.0, 1.0) / ns], [89.0, 94.0]),
    ]

    def put(t: int, zid: int, mode: str, origin: str = ds.OWNER) -> ds.Sample:
        doc = {"revision": t, "mode": mode, "rate_hz": 10}
        return ds.Sample(origin, False, ds.Stamp(t, zid), ds.Payload(value=doc), t)

    def acted(role: ds.TargetRole, s: ds.Sample) -> str:
        word = role.receive(s)
        if word in (ds.ACT, ds.REVERT):
            role.finish(ds.Finish(ds.DONE))
        return word

    # §2.6 step 4, now three sentences: "Earlier is **older**, and a target
    # MUST NOT act on it. Equal is **the same version**, never acted on again
    # (§2.7). Later is admitted." A remembered id, another one held.
    def step4() -> list[str]:
        words = []
        for probe in (put(9, 0xA, "idle"), put(10, 0xA, "halt"), put(11, 0xA, "halt")):
            role = ds.TargetRole("k")
            acted(role, put(10, 0xA, "run"))
            acted(role, put(5, 0xB, "idle"))
            words.append(acted(role, probe))
        return words

    out.append(("desired.v1 0.9 §2.6 step 4: of an id remembered while another is held, earlier is older, equal "
                "the same version, later admitted", step4(), [ds.OLDER, ds.SAME_VERSION, ds.ACT]))

    # §2.6 step 3: "Only a version from an archive is confirmed: any other
    # delivery of a version held waiting, the author's own version included,
    # is the same version". Held, then held waiting while an act runs.
    def step3() -> list[str]:
        words = []
        for held_from, again_from in ((ds.ARCHIVE, ds.OWNER), (ds.OWNER, ds.OWNER), (ds.ARCHIVE, ds.ARCHIVE)):
            role = ds.TargetRole("k")
            acted(role, put(10, 0xA, "run", held_from))
            words.append(role.receive(put(10, 0xA, "run", again_from)))
        for held_from, again_from in ((ds.ARCHIVE, ds.OWNER), (ds.OWNER, ds.OWNER)):
            role = ds.TargetRole("k")
            role.receive(put(10, 0xA, "run"))
            role.receive(put(20, 0xA, "halt", held_from))
            words.append(role.receive(put(20, 0xA, "halt", again_from)))
        return words

    out.append(("desired.v1 0.9 §2.6 step 3: only a version from an archive is confirmed, held or held waiting; "
                "any other equal delivery is the same version", step3(),
                [ds.CONFIRMED, ds.SAME_VERSION, ds.SAME_VERSION, ds.CONFIRMED, ds.SAME_VERSION]))

    # §2.7, now three sentences: "A restart begins a new run (§0), which
    # holds nothing of the previous run's acts. It MUST act on the first
    # version it reads, the one the previous run acted on included, and
    # adopts nothing before it." Against a re-put of the same document, which
    # the run before would have adopted.
    def restart() -> list[str]:
        words = []
        for again in (False, True):
            role = ds.TargetRole("k")
            acted(role, put(10, 0xA, "run"))
            if again:
                role.restart()
            words.append(role.receive(ds.Sample(ds.OWNER, False, ds.Stamp(11, 0xB),
                                                ds.Payload(value={"revision": 10, "mode": "run", "rate_hz": 10}),
                                                10)))
        return words

    out.append(("desired.v1 0.9 §2.7: the same document under a new stamp is adopted within a run, and acted on "
                "after a restart", restart(), [ds.ADOPTED, ds.ACT]))
    return out


def _round88_readings(root: Path) -> list[tuple[str, Any, Any]]:
    """``redundancy.v1`` 0.6, ``timing.v1`` 0.6, ``arbitration.v1`` 0.4 and
    the core's scenarios without history (core 0.64), and round 87's answers:
    ``desired.v1`` 0.10, ``config.v1`` 0.8, ``alarms.v1`` 0.10 and
    ``telemetry.v1`` 0.7 (#609, round 88; #935's fifth part). Each text at
    its version or later; F-336's one rule changed, as the texts state it;
    F-337 to F-346's rewrites as they read; §12's two contracts, lint-clean
    with the reference CLI's fingerprints; the core's half of a ``config.v1``
    owner fed through ``desired.v1`` (:meth:`zk2py.config.Core.revert`, how
    an act ends); a re-mint that is a restart acting on the same version
    again (:class:`zk2py.desired.TargetRole`); and the reference's figures
    the 0.4 and 0.6 rewrites moved into sentences of their own, held by
    zk2py's runtimes. No session is opened."""
    import inspect
    import json as _json
    import re

    from . import arbitration_live as al
    from . import config as cf
    from . import desired as ds
    from . import presence as pr
    from .contract import load_contract

    spec = root.parent
    flat = lambda rel: " ".join((spec / rel).read_text(encoding="utf-8").split())  # noqa: E731

    def missing(text: str, *quotes: str) -> list[str]:
        return [q for q in quotes if q not in text]

    def at_least(rel: str, want: tuple[int, int]) -> list[str]:
        m = re.search(r"\| \*\*Text version\*\* \| (\d+)\.(\d+) \(", flat(rel))
        if m is None:
            return [f"{rel} names no text version"]
        got = (int(m.group(1)), int(m.group(2)))
        return [] if got >= want else [f"{rel} at {got[0]}.{got[1]}, below {want[0]}.{want[1]}"]

    def entry(rel: str, version: str, words: str) -> list[str]:
        log = (spec / rel).read_text(encoding="utf-8").splitlines()
        ok = any(line.startswith(f"## {version} ") and words in line for line in log)
        return [] if ok else [f"{rel} has no {version} entry saying {words!r}"]

    desired_text, config_text = flat("profiles/desired/v1.md"), flat("profiles/config/v1.md")
    out: list[tuple[str, Any, Any]] = [
        ("#935 part 5: redundancy.v1 0.6, timing.v1 0.6 and arbitration.v1 0.4 or later, each 0.N entry saying no "
         "rule changed",
         at_least("profiles/redundancy/v1.md", (0, 6)) + at_least("profiles/timing/v1.md", (0, 6))
         + at_least("profiles/arbitration/v1.md", (0, 4))
         + entry("profiles/redundancy/CHANGELOG.md", "0.6", "no rule changed")
         + entry("profiles/timing/CHANGELOG.md", "0.6", "no rule changed")
         + entry("profiles/arbitration/CHANGELOG.md", "0.4", "no rule changed")
         + entry("CHANGELOG.md", "0.64", "no rule changed"), []),
        ("round 87's answers: desired.v1 0.10, config.v1 0.8, alarms.v1 0.10 and telemetry.v1 0.7 or later",
         at_least("profiles/desired/v1.md", (0, 10)) + at_least("profiles/config/v1.md", (0, 8))
         + at_least("profiles/alarms/v1.md", (0, 10)) + at_least("profiles/telemetry/v1.md", (0, 7)), []),
        ("desired.v1 0.10 §2.16 (F-336): a config.v1 owner's re-mint is a restart, a new run that MUST act again",
         missing(desired_text,
                 "**A re-mint is a restart** for a `config.v1` owner (`config.v1` §2.12). It returns its device to "
                 "its startup values and begins a new run, so §2.7's restart holds of it: it MUST act on the first "
                 "version it reads, the one it acted on before the re-mint included. `[Sc: §12]`",
                 "For a `config.v1` owner a run and an instance are the same thing, so this is `config.v1` §2.18's "
                 "once in an instance."), []),
        ("desired.v1 0.10 §0 and §2.7: the general rule stays, each with its one exception",
         missing(desired_text, "a target's instance changes at a re-mint, its run does not. The one exception is a "
                 "`config.v1` owner, whose re-mint is a restart (§2.16).",
                 "keeps what it acted on, the ids it remembers and its report. The one exception is a `config.v1` "
                 "owner, whose re-mint is a restart (§2.16)."), []),
        ("config.v1 0.8 §2.18 (F-336): its instance is desired.v1's run, and a re-mint begins a new one",
         missing(config_text, "Its instance and desired.v1's run are the same thing. A re-mint is a restart "
                 "(§2.12), which begins a new run, as desired.v1 §2.16 says."), []),
        ("desired.v1 0.10's scenarios: §12, a config.v1 owner's re-mint is a restart",
         missing(flat("profiles/desired/scenarios.md"),
                 "## §12 A `config.v1` owner's re-mint is a restart (§2.7, §2.16)",
                 "A new instance whose `St` shows `desired` converged on revision 1 while its device holds 10000 "
                 "fails this step."), []),
        ("round 87's F-337 to F-340, as alarms.v1 0.10 and telemetry.v1 0.7 read",
         missing(flat("profiles/alarms/v1.md"), "Without this rule, a key whose resolve and delete a loss took",
                 "A GET that answers it heals it, as step 4's first sentence says.",
                 "An answer of neither, unobservable or not asked, splits by its reason, in two closed lists:")
         + missing(flat("profiles/telemetry/v1.md"), "It folds the tokens' moves and the samples into one sequence, "
                   "in the order it received them. So the presence it holds between two arrivals is the presence it "
                   "received between them.")
         + missing(flat("profiles/telemetry/scenarios.md"), "complete read, one of two:"), []),
        ("round 87's F-341 to F-346, as desired.v1 0.10 reads",
         missing(desired_text,
                 "and is reported alone: each of the others reads the binding of one slice, and a role on two "
                 "documents has no one slice.",
                 "A target that read the archive on a silent GET would act on the first again, and on the second "
                 "as last-known.",
                 "One equal delivery is not:",
                 "A version the target holds, or holds waiting, that came from its author is never confirmed:",
                 "The target acts on that value as another id, then on the new epoch's next value.",
                 "each parameter named by a document it acted on in this run goes back to its startup value",
                 "A revision that declares a desired resource, even only one, settles nothing."), []),
    ]

    # §12's contracts, written from its setup: no finding at all, and the
    # reference CLI's fingerprints (zk2 contract lint, round 88).
    fps = {"configs.v1.toml": "sha256:daee2c9d7434152c15adef1c3fae78dbf5697c8db28e1e39fb0aa9f4e17166eb",
           "zk2py_configured.v1.toml": "sha256:89872483fa68bcd829b1e814d120dd799ef6a7b0145561d6e814b0cda0e87890"}
    here = Path(__file__).resolve().parents[1] / "interop" / "desired"
    got = {}
    for name in fps:
        c = load_contract(here / name, spec_dir=spec, check_file_name=True)
        got[name] = (c.valid, list(c.codes), c.fingerprint)
    out.append(("desired.v1 scenarios §12: configs.v1 and zk2py_configured.v1 load with no finding, with the "
                "reference CLI's fingerprints", got, {n: (True, [], fp) for n, fp in fps.items()}))

    # The core's half of a config.v1 owner fed through desired.v1: an act's
    # end said (converged, then a delete's revert), the revert one set back
    # to the startup value and the view with no `desired` after it.
    rf = _json.loads((spec / "profiles/config/conformance/lifecycle.json").read_text())["schemas"]["rf"]
    startup = {"duty": {"fraction_ppm": {"value": 10000, "source": "file"}},
               "radio": {"frequency_hz": {"value": 868100000, "source": "file"}}}
    core = cf.Core(service="lab/rf0", instance="00000000000000a1", schema=rf, startup=startup, floor_s=10,
                   apply_after_ns=10**9, persist=True)
    t = 1_800_000_000 * 10**9
    core.desire({"revision": 1, "values": {"duty": {"fraction_ppm": 20000}}}, t)
    first = (core.desired_end, core.running[("duty", "fraction_ppm")].value, core.view().get("desired"))
    core.desired_end = None
    step = core.revert({"duty": {"fraction_ppm": 10000}}, t + 10**9)
    second = (core.desired_end, core.running[("duty", "fraction_ppm")].value, "desired" in core.view(),
              [e["outcome"] for e in step.events])
    out.append(("desired.v1 0.10 §2.16 in zk2py's config.v1 core: an act's end said, and a delete's revert one set "
                "back to the startup value, the view with no desired after it",
                [first, second],
                [(("converged", None), 20000, {"observed_revision": 1, "outcome": "converged"}),
                 (("reverted", None), 10000, False, ["applied"])]))

    # §2.16 through ConfigOwner's feed, with no session (review-bot on PR
    # #963): a refused document leaves the role free for the next version,
    # and so does an act that raises, failed with code apply and counted.
    # The owner's event publisher is a list here, and the owner is never
    # started, so its device holds nothing until an apply.
    owner = cf.ConfigOwner("lab", "rf0", schema=rf, startup=startup, fed=("k", "lab/author"))
    published: list[str] = []
    owner.owner.occur = lambda member, payload: published.append(_json.loads(payload)["outcome"])  # type: ignore

    def fed(t: int, doc: dict[str, Any]) -> tuple[Any, ...]:
        owner._fed_receive(False, ds.Stamp(t, 0xA).text, cf.encode_json(doc))
        rep = owner.role.report()
        return (owner._fed_running or owner.role.running, rep.get("outcome"),
                (rep.get("refusal") or {}).get("code"), owner.device.values.get("duty", {}).get("fraction_ppm"))

    def boom(act: Any) -> Any:
        raise RuntimeError("the device fell off")

    rows = [fed(10, {"revision": 2, "values": {"radio": {"frequency_hz": 869525000}}}),
            fed(11, {"revision": 3, "values": {"duty": {"fraction_ppm": 20000}}})]
    device, owner.core.device = owner.core.device, boom
    rows.append(fed(12, {"revision": 4, "values": {"duty": {"fraction_ppm": 30000}}}))
    owner.core.device = device
    rows.append(fed(13, {"revision": 5, "values": {"duty": {"fraction_ppm": 40000}}}))
    out.append(("desired.v1 §2.16 in zk2py's fed config.v1 owner: a refused document, and an act that raises, each "
                "leave the role free for the next version, which is acted on",
                [rows, owner.fed_errors, published],
                [[(False, "refused", "reach", None), (False, "converged", None, 20000),
                  (False, "failed", "apply", 20000), (False, "converged", None, 40000)], 1, ["applied", "applied"]]))

    # §2.16 (0.10) through §2.7's restart: the version acted on, delivered
    # again under its own stamp, is the same version within a run, and acted
    # on in the run a re-mint that is a restart begins.
    def again(restart: bool) -> str:
        role = ds.TargetRole("k")
        s = ds.Sample(ds.OWNER, False, ds.Stamp(10, 0xA), ds.Payload(value={"revision": 1, "values": {}}), 1)
        role.receive(s)
        role.finish(ds.Finish(ds.DONE))
        if restart:
            role.restart()
        return role.receive(s)

    out.append(("desired.v1 0.10 §2.16: the bound version delivered again is the same version in a run, and acted "
                "on after a config.v1 owner's re-mint begins a new one", [again(False), again(True)],
                [ds.SAME_VERSION, ds.ACT]))

    # The figures 0.4 and 0.6 gave sentences of their own, kept by zk2py.
    arb, arb_sc = flat("profiles/arbitration/v1.md"), flat("profiles/arbitration/scenarios.md")
    actuator = inspect.signature(al.Actuator.__init__).parameters["reread_delay_s"].default
    commander = inspect.signature(al.Commander.__init__).parameters["lead_s"].default
    out.append(("arbitration.v1 0.4 §2.7 and the scenarios' commanders: the reference's read again 0.1 s after a "
                "withdrawal and its first put 500 ms after the tokens, each a sentence of its own, are zk2py's "
                "actuator's and commander's",
                [missing(arb, "The reference makes one read 0.1 s after the first withdrawal of a burst."),
                 missing(arb_sc, "first put is **500 ms after its tokens**"), actuator, commander, pr.REREAD_DELAY_S],
                [[], [], 0.1, 0.5, 0.1]))
    return out


def _round89_readings(root: Path) -> list[tuple[str, Any, Any]]:
    """Core 0.66's conformance classes, ``observer.md`` 0.8, ``spec/RULES.md``
    and the glossary, and round 88's answers (#609, round 89; #926, #935
    part 6). Each text at its version or later; F-347 to F-357's answers as
    they read; the nineteen core and three observer sentences 0.66 and 0.8
    made name their role, as they read, the old forms gone; ``RULES.md``'s
    own counts against its entries, as zk2py parses them; zk2py's walk of it
    (:mod:`zk2py.classes`) in step with it, and its counts; the glossary's 36
    terms; and the owner rules the walk found zk2py's owner not keeping,
    kept now, read with no session (a fake one records what is declared):
    core §2.4's QoS, every field and an event's; §3.3's stated cardinality
    never raised; O6's one summary; §5.2's detail; §5.1's "Beside
    replicas"; and observer §7.4's check at the echo tool's start."""
    import inspect
    import re

    from . import classes, echo_tool
    from . import owner as ow
    from .contract import load_contract

    spec = root.parent
    flat = lambda rel: " ".join((spec / rel).read_text(encoding="utf-8").split())  # noqa: E731

    def missing(text: str, *quotes: str) -> list[str]:
        return [q for q in quotes if q not in text]

    def present(text: str, *quotes: str) -> list[str]:
        return [q for q in quotes if q in text]

    def at_least(rel: str, want: tuple[int, int]) -> list[str]:
        m = re.search(r"\| \*\*Text version\*\* \| (\d+)\.(\d+) \(", flat(rel))
        if m is None:
            return [f"{rel} names no text version"]
        got = (int(m.group(1)), int(m.group(2)))
        return [] if got >= want else [f"{rel} at {got[0]}.{got[1]}, below {want[0]}.{want[1]}"]

    def entry(rel: str, version: str, words: str) -> list[str]:
        log = (spec / rel).read_text(encoding="utf-8").splitlines()
        ok = any(line.startswith(f"## {version} ") and words in line for line in log)
        return [] if ok else [f"{rel} has no {version} entry saying {words!r}"]

    core, obs = flat("core.md"), flat("observer.md")
    m = re.search(r"\*\*Version (\d+)\.(\d+)\*\*", core)
    core_v = (int(m.group(1)), int(m.group(2))) if m else None
    out: list[tuple[str, Any, Any]] = [
        ("#926: core 0.66 or later, observer.md 0.8 or later written against core 0.66 or later, each with its "
         "entry", [core_v is not None and core_v >= (0, 66)] + at_least("observer.md", (0, 8))
         + entry("CHANGELOG.md", "0.66", "conformance classes by role")
         + entry("observer-CHANGELOG.md", "0.8", "no rule changed"), [True]),
        ("#935 part 6: desired.v1 0.11, timing.v1 0.7, arbitration.v1 0.5 and redundancy.v1 0.7 or later, each "
         "with its entry",
         at_least("profiles/desired/v1.md", (0, 11)) + at_least("profiles/timing/v1.md", (0, 7))
         + at_least("profiles/arbitration/v1.md", (0, 5)) + at_least("profiles/redundancy/v1.md", (0, 7))
         + entry("profiles/desired/CHANGELOG.md", "0.11", "seeds (F-347)")
         + entry("profiles/timing/CHANGELOG.md", "0.7", "no rule changed")
         + entry("profiles/arbitration/CHANGELOG.md", "0.5", "no rule changed")
         + entry("profiles/redundancy/CHANGELOG.md", "0.7", "no rule changed"), []),
        ("F-347, resolved by desired.v1 0.11 §2.5: a target seeds at a re-mint that is a restart",
         missing(flat("profiles/desired/v1.md"),
                 "A target MUST seed at its start, at a re-mint that is a restart (§2.16), and again each time "
                 "its author's instance token appears. It appears for a new instance, or for one coming back after "
                 "an absence (core §8.1). `[Sc: §1, §5, §12]`"), []),
        ("F-348, F-350 and F-351, resolved by timing.v1 0.7's rewrites",
         missing(flat("profiles/timing/v1.md"),
                 "So a reader can handle a new instance's first samples before its descriptor, and read them "
                 "`unattributed` or `foreign_stamp`.",
                 "else **not asked**: nobody put the question of any of them. That is never unobservable, the answer "
                 "of a window that could not answer it.",
                 "The window's start is the first instant both stand: - its presence read, made after its watch "
                 "session's round trip (core §8.1); - the descriptor of every instance that read shows, which it "
                 "holds."), []),
        ("F-349, resolved by arbitration.v1 0.5's list",
         missing(flat("profiles/arbitration/v1.md"),
                 "**As one change:** each address ends the move one of three ways: - withdrawn, when what was kept "
                 "leaves it with no token known held, and what was kept holds a withdrawal of it. It forgets its "
                 "arrivals then; - re-admitted, when it was withdrawn before the move, and what was kept leaves a "
                 "token known held there; - otherwise, as it was."), []),
        ("F-352 and F-353, resolved by redundancy.v1 0.7's scenarios",
         missing(flat("profiles/redundancy/scenarios.md"),
                 "the test does it one of two ways, and §1 and §10 run both: - it makes the instance's links silent "
                 "first, through a proxy between it and its router that stops forwarding and closes nothing.",
                 "- or it kills the process as it runs. The host's kernel closes its links, and the crash is seen "
                 "at once.",
                 "S's GET at A's instance token answers A's descriptor marked standby. The one exception is a "
                 "takeover before that GET was answered, which S tells by A's interface token arriving within a "
                 "round trip of the answer: the token and the answer ride two priorities"), []),
        ("F-354 to F-356, resolved by core 0.65's presence.md §10, and F-357 by the profiles' index",
         missing(flat("scenarios/presence.md"),
                 "It owes a read at once, and makes none. While its watch session is cut off it asks no round trip "
                 "and makes no read, though its reading session holds its link: a read not made. The owner is "
                 "unobservable, never down.",
                 "has no call for it to drop. Its own waits, each given up after 20 ms, stand in for the dropped "
                 "calls.",
                 "has run out under the 300 ms interests timeout. How many that is depends on the reader: two for "
                 "one that asks a round trip at its start and another before its seed, and one for one whose start's "
                 "round trip serves its seed too.")
         + missing(flat("profiles/README.md"),
                   "A stamp's id that is never 0 needed none either. That rule is `desired.v1`'s own, in its §2.4;"),
         []),
    ]

    # The sentences 0.66 and 0.8 made name their role: each new form in
    # place, each old form gone (round 89 compared them pair by pair).
    core_pairs = [
        ("The owner SHOULD do so in a new session", "- It SHOULD do so in a new session"),
        ("An owner's counter that crosses a face where neither is visible SHOULD",
         "- A counter that crosses a face where neither is visible"),
        ("**A contract's state and events** SHOULD keep `reliable` and `block`.",
         "- **State and events** SHOULD keep"),
        ("| R7 | **Consumer:** a binding MUST NOT require presence.", "| R7 | A binding MUST NOT"),
        ("A consumer's runtime MUST resolve a provider whose system position is `self.system`",
         "A runtime MUST resolve a provider"),
        ("The owner's descriptor MUST NOT raise it.", "contract's. It MUST NOT raise it."),
        ("**Size.** An owner's descriptor SHOULD stay within 1 KB.", "**Size.** A descriptor SHOULD"),
        ("- The owner SHOULD detect drift against a reference", "- It SHOULD detect drift"),
        ("When a detection shows it beyond the delta, the owner MUST stop writing state",
         "the delta, it MUST stop"),
        ("**An archive's backend MUST NOT accept a put older than a delete it holds.**", "**Its backend MUST NOT"),
        ("A request type SHOULD NOT repeat a template parameter (W102).", "repeat a template parameter. `"),
        ("**The `Encoding` of the owner's reply** MUST say which:", "**The reply's `Encoding`**"),
        ("or a value in a protobuf one, MUST NOT be sent by an owner.", "protobuf one, MUST NOT be sent. "),
        ("The name MUST be unique across the listed files (E024).", "unique across the listed files. |"),
        ("**A deployment's batch SHOULD take at most a third of the lease to cross the face.**",
         "**A batch SHOULD take"),
        ("**A deployment SHOULD deny `@stream` keys across the face**", "**`@stream` keys SHOULD be denied"),
        ("So a deployment MUST grant the far side's principal `declare_queryable`", "MUST be granted"),
        ("For a deployment under `allow`, an operation that must not run for everyone MUST be",
         "for a principal never granted it; only its answers are denied. Under `allow`"),
        ("| Descriptor (§3.3) | An owner's descriptor MUST fit one fragment", "| Descriptor (§3.3) | MUST fit"),
    ]
    obs_pairs = [
        ("Every liveliness subscriber the caller or tool holds on that session MUST be callback-driven",
         "Every liveliness subscriber on that session MUST"),
        ("**The subscribers of a reader's watch session SHOULD have no history**", "when their reader seeds"),
        ("A tool's grace period SHOULD exceed the longest re-mint overlap", "tool setting. It SHOULD exceed"),
    ]
    out.append(("core 0.66's nineteen and observer.md 0.8's three sentences that now name their role, as they read, "
                "and none of the old forms",
                [missing(core, *[n for n, _ in core_pairs]), present(core, *[o for _, o in core_pairs]),
                 missing(obs, *[n for n, _ in obs_pairs]), present(obs, *[o for _, o in obs_pairs])],
                [[], [], [], []]))
    out.append(("core §12 0.66: five classes, the static rules' sections, constrained a modifier, profiles one "
                "class each; observer.md §0's classes",
                missing(core, "## 12. Conformance classes",
                        # the static rules' sections, as F-390's rewrite keeps them (#926 part 3 rewords the
                        # sentence around them)
                        "§2.2, §2.4, §2.5, §2.6, §3.1, §7.1 and §10", "**Constrained** is a modifier on a class.",
                        "**Profiles** stay opt-in, one class each (§10).")
                + missing(obs, "**Its classes.** Conformance is claimed by role (core §12)."), []))

    # RULES.md, as zk2py parses it: each figure its table states against its
    # own entries, where zk2py reads both. Reported, never failed: the index
    # is regenerated by a script zk2py does not read, and its agreement with
    # itself is a finding about it (F-367's kind), not a case of zk2py's.
    rules = (spec / "RULES.md").read_text(encoding="utf-8")
    entries = classes._rules_md(spec)
    stated, counted = {}, {}
    for name, n in re.findall(r"^\| \[([^\]]+)\]\(#[^)]+\) \| (\d+)", rules, re.M):
        if name.strip() in classes.HEADINGS:
            stated[name] = int(n)
            counted[name] = sum(1 for e in entries if e[0] == classes.HEADINGS[name.strip()])
    m = re.search(r"(\d+) of them a reader's \|", rules)
    marked = len(re.findall(r"^\| \[[^\]]+\]\([^)]+\), \*reader\* \|", rules, re.M))
    if m and marked:
        stated["readers"], counted["readers"] = int(m.group(1)), marked
    m = re.search(r"^\| Every rule, once \| (\d+) \|", rules, re.M)
    if m:
        stated["once"], counted["once"] = int(m.group(1)), len({(w, x) for _, w, _, x in entries})
    m = re.search(r"^\| \[Rule ids\]\(#[^)]+\) \| (\d+)", rules, re.M)
    ids_part = rules.split("\n## Rule ids", 1)[1] if "\n## Rule ids" in rules else ""
    if m and ids_part:
        stated["ids"] = int(m.group(1))
        counted["ids"] = len(re.findall(r"^\| \[[^\]]+\]\([^)]+\) \|", ids_part, re.M))
    off = {k: (stated[k], counted[k]) for k in stated if stated[k] != counted[k]}
    NOTES.append(("readings", "RULES.md: its table " + (
        "agrees with its entries: " + ", ".join(f"{k} {v}" for k, v in counted.items()) if not off else
        "disagrees with its entries (stated, counted): " + ", ".join(f"{k} {v}" for k, v in off.items()))))

    # zk2py's walk of it (classes.py), keyed to the rules: the one failure is
    # a verdict whose rule its text no longer states. A RULES.md row of a
    # claimed class with no verdict is reported, counted and named, never
    # failed: the next round gives it its verdict.
    out.append(("core §12: each verdict of zk2py's walk names a rule its text still states (zk2py.classes.check); "
                "RULES.md's rows with no verdict are reported below the totals", classes.check(spec), []))
    out.append(("core §12: zk2py claims the owner, consumer, caller and observer classes at the full level, and "
                "its walk holds of itself: a claimed class or the static rules, a section, a status with what "
                "shows it, one verdict per key (zk2py.classes.structure)",
                [classes.CLAIMED, classes.structure()],
                [{"owner": "full", "consumer": "full", "caller": "full", "observer": "full"}, []]))
    NOTES.extend(("readings", line) for line in classes.report(spec))

    # The glossary: each term linked to a section, the count reported.
    readme = (spec / "README.md").read_text(encoding="utf-8")
    gloss = readme.split("## Glossary", 1)[1].split("\n## ", 1)[0] if "## Glossary" in readme else ""
    terms = re.findall(r"^- \*\*([^*]+)\*\*:", gloss, re.M)
    linked = [t for t in re.split(r"\n(?=- \*\*)", gloss) if t.startswith("- **") and "](" in t]
    out.append(("spec/README.md's glossary: each term linked to its section", [bool(terms), len(linked)],
                [True, len(terms)]))
    NOTES.append(("readings", f"glossary: {len(terms)} terms"))

    # -- the owner rules zk2py's owner keeps since this round ----------------------------
    probe = load_contract(Path(__file__).resolve().parents[1] / "interop" / "zk2py_probe.v1.toml", spec_dir=spec)
    replica = load_contract(Path(__file__).resolve().parents[1] / "interop" / "classes" / "zk2py_replica.v1.toml",
                            spec_dir=spec, check_file_name=True)
    out.append(("round 89's zk2py_replica.v1 loads with no finding, with the reference CLI's fingerprint",
                (replica.valid, list(replica.codes), replica.fingerprint),
                (True, [], "sha256:3ab773bde71104aa5a1f61c74893b521daeede49634f1a0fa8fc10ddd1a4cf68")))

    class Entity:
        def undeclare(self) -> None:
            pass

        def put(self, payload: Any, **kw: Any) -> None:
            pass

    class Fake:
        """A session that records what is declared and put on it; with
        ``fail``, every declaration and put raises."""

        def __init__(self, fail: bool = False) -> None:
            self.publishers: list[tuple[str, dict[str, Any]]] = []
            self.puts: list[tuple[str, dict[str, Any]]] = []
            self.queryables: list[str] = []
            self.fail = fail

        def declare_publisher(self, key: Any, **kw: Any) -> Entity:
            if self.fail:
                raise RuntimeError("the session refused the declaration")
            self.publishers.append((str(key), kw))
            return Entity()

        def put(self, key: Any, payload: Any, **kw: Any) -> None:
            if self.fail:
                raise RuntimeError("the session refused the put")
            self.puts.append((str(key), kw))

        def declare_queryable(self, key: Any, *a: Any, **kw: Any) -> Entity:
            self.queryables.append(str(key))
            return Entity()

    def names(kw: dict[str, Any]) -> tuple[str, ...]:
        return tuple(str(kw[f]).rsplit(".", 1)[-1] if f in kw else "zenoh's"
                     for f in ("reliability", "congestion_control", "priority", "express"))

    res = {f"{r['token']}/{r['template']}": r for r in probe.canonical["resources"]}
    ev = "zk2/lab/probe/zk2py_probe.v1/events/changed"
    o = ow.Owner("lab", "probe", [probe], budget=False)
    o.session = Fake()  # type: ignore[assignment]
    o._declare_member("zk2/lab/probe/zk2py_probe.v1/@stream/lanes/a")
    o.occur(ev, b"{}")
    o.occur(ev, b"{}")
    # the same event declared best_effort, on a copy of the contract
    probe_be = load_contract(Path(__file__).resolve().parents[1] / "interop" / "zk2py_probe.v1.toml", spec_dir=spec)
    next(r for r in probe_be.canonical["resources"] if r["template"] == "changed")["reliability"] = "best_effort"
    be = ow.Owner("lab", "probe", [probe_be])
    be.session = Fake()  # type: ignore[assignment]
    be.occur(ev, b"{}")
    # a session that refuses: nothing is put, so nothing is counted, and the
    # member's rate ("low", one an hour) refuses no second attempt (§2.7)
    failing = ow.Owner("lab", "probe", [probe])
    failing.session = Fake(fail=True)  # type: ignore[assignment]
    raised = []
    for _ in range(2):
        try:
            failing.occur(ev, b"{}")
        except RuntimeError:
            raised.append(True)
    out.append(("core §2.4 \"An owner MUST publish with it\": zk2py's owner sets every QoS field, reliability "
                "through zenoh's unstable API, on a templated member's publisher; a reliable event goes on "
                "Session.put with the three stable fields and zenoh's own reliability, no publisher declared, a "
                "best_effort one on a publisher of its own; an occurrence counted for its rate only once it is put",
                [names(ow.qos_of(res["state/status"])), [(k, names(kw)) for k, kw in o.session.publishers],
                 [names(kw) for _, kw in o.session.puts], [names(kw) for _, kw in be.session.publishers],
                 be.session.puts, len(o._occurrences.get(ev, [])), raised, failing._occurrences.get(ev, [])],
                [("RELIABLE", "BLOCK", "DATA", "False"),
                 [("zk2/lab/probe/zk2py_probe.v1/@stream/lanes/a", ("BEST_EFFORT", "DROP", "DATA_LOW", "False"))],
                 [("zenoh's", "BLOCK", "DATA", "False")] * 2, [("BEST_EFFORT", "BLOCK", "DATA", "False")], [], 2,
                 [True, True], []]))

    def refused(**kw: Any) -> str:
        try:
            ow.Owner("lab", "probe", [probe], **kw)._plan()
        except ow.OwnerRefused as e:
            return "refused" if "(§3.3, D0" in str(e) else str(e)
        return "starts"

    out.append(("core §3.3 \"The owner's descriptor MUST NOT raise it\": a stated bound above the contract's, "
                "below 1, of no templated resource, or not an integer (\"3\"), refused before anything is declared; "
                "a lower one starts",
                [refused(cardinality={"zk2py_probe.v1": {"@stream/lanes/{lane}": n}}) for n in (5, 0, 4, 1, "3")]
                + [refused(cardinality={"zk2py_probe.v1": {"state/status": 1}})],
                ["refused", "refused", "starts", "starts", "refused", "refused"]))

    class Query:
        payload = attachment = None

        def __init__(self) -> None:
            self.sent: list[str] = []

        def reply(self, key: str, payload: Any, **kw: Any) -> None:
            self.sent.append("summary" if kw.get("attachment") == b"summary" else "value")

        def reply_err(self, payload: Any, **kw: Any) -> None:
            self.sent.append("envelope")

    rres = {r["template"]: r for r in replica.canonical["resources"]}
    base = "zk2/lab/r/zk2py_replica.v1"

    def call(template: str, *steps: tuple[str, Any]) -> tuple[list[str], list[str]]:
        q = Query()
        c = ow.OpCall(q, base, rres[template], "application/json", {}, f"{base}/@op/{template}")
        said = []
        for what, arg in steps:
            try:
                getattr(c, what)(**arg) if isinstance(arg, dict) else getattr(c, what)(arg)
                said.append("sent")
            except (RuntimeError, ValueError):
                said.append("refused")
        return said, q.sent

    def handled(template: str, *steps: tuple[str, Any]) -> list[str]:
        """What the caller receives when a handler takes these steps and then
        raises: :meth:`OpCall.finish` after a handler that failed, as
        ``Owner._op_handler`` runs it."""
        q = Query()
        c = ow.OpCall(q, base, rres[template], "application/json", {}, f"{base}/@op/{template}")
        try:
            for what, arg in steps:
                getattr(c, what)(**arg) if isinstance(arg, dict) else getattr(c, what)(arg)
            raise RuntimeError("the handler's own bug")
        except (RuntimeError, ValueError) as e:
            c.failed = str(e)
        c.finish()
        return q.sent

    out.append(("core O6 \"each replier MUST end with exactly one summary reply\": a second summary, and a value "
                "after it, refused to the handler; a summary for an operation that declares none refused",
                [call("scan", ("reply", b"a"), ("summary", b"1"), ("summary", b"2"), ("reply", b"b")),
                 call("sync", ("summary", b"1"))],
                [(["sent", "sent", "refused", "refused"], ["value", "summary"]), (["refused"], [])]))
    out.append(("core O3 and §5.1, a call left without its reply gets internal, never silence: a handler that "
                "raises at a refused step gets its caller an envelope, but after its summary, which ended the "
                "replier, nothing more",
                [handled("sync", ("summary", b"1")), handled("sync", ("refuse", {"code": "app", "message": "no",
                                                                                 "detail": "x"})),
                 handled("scan", ("reply", b"a")), handled("scan", ("reply", b"a"), ("summary", b"1"), ("reply", b"b")),
                 handled("sync", ("reply", b"a"))],
                [["envelope"], ["envelope"], ["value", "envelope"], ["value", "summary"], ["value"]]))
    out.append(("core §5.2 \"an owner MUST NOT put a detail in it\": an operation that declares no error type "
                "refuses a detail to its handler, nothing sent; a refusal without one is sent",
                [call("sync", ("refuse", {"code": "app", "message": "no", "detail": "x"})),
                 call("sync", ("refuse", {"code": "app", "message": "no"}))],
                [(["refused"], []), (["sent"], ["envelope"])]))

    def queryables(unavailable: dict[str, str]) -> list[str]:
        o = ow.Owner("lab", "r", [replica], unavailable=unavailable)
        o.session = Fake()  # type: ignore[assignment]
        plan = o._plan()
        o._declare_resources(replica, plan[replica.interface])
        return sorted(k.rsplit("/", 1)[1] for k in o.session.queryables if "/@op/" in k)

    out.append(("core §5.1 \"Beside replicas\": an instance exposing only replicated operations of an interface "
                "declares no unavailable queryable on an exclusive one; beside an exclusive one exposed it does",
                [ow.replica_only([(rres["sync"], "exposed"), (rres["admin"], "listed")]),
                 queryables({"@op/admin": "config", "@op/scan": "config"}), queryables({"@op/admin": "config"})],
                [True, ["sync"], ["admin", "scan", "sync"]]))
    out.append(("observer §7.4 \"A tool SHOULD check the limit at start\": zk2py's three tool processes say it on "
                "their standard error before they listen",
                ["memlock_said" in inspect.getsource(echo_tool.main)],
                [True]))
    return out


def _round90_readings(root: Path) -> list[tuple[str, Any, Any]]:
    """Core 0.67 and ``observer.md`` 0.9, round 89's answers (#609, round 90;
    #926 part 2). Each text at its version or later; F-358 to F-378's
    answers as they read, and the forms they replaced gone; and S7's
    consumer rule after a late joiner's seed, which 0.67 states and zk2py's
    :class:`zk2py.seed.LateJoiner` keeps since this round: its seed by §4.2's
    merge, then each value a consumer receives by time within one id, the
    first under a new id kept (:func:`zk2py.seed.follows`), a tool's by the
    merge still. Read with no session: a fake one stands in."""
    import re

    from . import seed as sd

    spec = root.parent
    flat = lambda rel: " ".join((spec / rel).read_text(encoding="utf-8").split())  # noqa: E731

    def missing(text: str, *quotes: str) -> list[str]:
        return [q for q in quotes if q not in text]

    def present(text: str, *quotes: str) -> list[str]:
        return [q for q in quotes if q in text]

    def entry(rel: str, version: str, words: str) -> list[str]:
        log = (spec / rel).read_text(encoding="utf-8").splitlines()
        ok = any(line.startswith(f"## {version} ") and words in line for line in log)
        return [] if ok else [f"{rel} has no {version} entry saying {words!r}"]

    core, obs, rules, readme = flat("core.md"), flat("observer.md"), flat("RULES.md"), flat("README.md")
    # A section's flat text, for a phrase that must stand there (§0's, §12's).
    section = lambda text, start, end: text.split(start, 1)[-1].split(end, 1)[0]  # noqa: E731
    core0 = section(core, "## 0. Conventions", "## 1. Identity and grammar")
    core12 = section(core, "## 12. Conformance classes", "## 13.")
    m = re.search(r"\*\*Version (\d+)\.(\d+)\*\*", core)
    core_v = (int(m.group(1)), int(m.group(2))) if m else (0, 0)
    m = re.search(r"\| \*\*Text version\*\* \| (\d+)\.(\d+) \(", obs)
    obs_v = (int(m.group(1)), int(m.group(2))) if m else (0, 0)
    m = re.search(r"\| \*\*Written against\*\* \| core (\d+)\.(\d+) ", obs)
    against = (int(m.group(1)), int(m.group(2))) if m else (0, 0)
    out: list[tuple[str, Any, Any]] = [
        ("#926 part 2: core 0.67 or later, observer.md 0.9 or later written against core 0.67 or later, each "
         "with its entry",
         [core_v >= (0, 67), obs_v >= (0, 9), against >= (0, 67)]
         + entry("CHANGELOG.md", "0.67", "round 89's answers")
         + entry("observer-CHANGELOG.md", "0.9", "F-370 and F-376"),
         [True, True, True]),
        # The sentences round 90's findings ask #926 part 3 to reword (F-389, F-390) are read by a phrase their
        # fixes keep, in their own section, never word for word.
        ("F-358, F-359, F-360, F-362, F-377 and F-378, resolved by core 0.67's §12 and §0",
         missing(core,
                 "**Full** is the level of a class claimed without the constrained modifier.",
                 "**How a static rule binds.** A static rule binds an implementation where it does what the rule "
                 "governs.",
                 "§9.8's publication rules bind whoever publishes a revision: its CI, and the person who accepts it.",
                 "A class also holds the rules its texts state without one, such as `observer.md` §1, §2, §5 and §8.",
                 "Of the static rules, it meets those that govern what it does (above).",
                 "A profile's rules name the roles they bind, in core §0's words, as each profile's §0 says.")
         + missing(core12, "that names a role is that role's", "descriptor checker")
         + missing(core0, "whichever of the roles above it plays", "where it does what")
         + present(core, "**Full** is every class, their union", "as this text's do (its §0)", "full included,"),
         []),
        ("F-363, restated by core 0.67: §2.2's MUST NOT (E010, which §9.2's table makes \"a rest parameter that is "
         "not last\") and §11.2's SHOULD, the old forms gone",
         missing(core, "A rest parameter MUST NOT appear except as the last segment (E010)",
                 "A deployment SHOULD run `deny` where the router can afford the enumeration.",
                 "or a rest parameter that is not last |")
         + present(core, "It MAY appear only as the last segment", "This is RECOMMENDED where"), []),
        # F-368's S7 sub-bullet and F-369's opt-in are sentences round 90's F-380, F-382, F-383 and F-391 ask
        # #926 part 3 to reword: F-369 is read by the phrases its fixes keep, and F-368 by zk2py's late joiner
        # below, never word for word.
        ("F-369 and F-374, two of the three rules core 0.67 changed, as they read; F-368's is read by the late "
         "joiner below",
         missing(core,
                 "`congestion`, `priority` and `express`",
                 "MAY publish reliable",
                 "zenoh-python 1.10.1 marks `Reliability` unstable too, and `Session.put` takes none.",
                 "or any selector without a verbatim chunk) never delivers")
         + present(core, "The fields map one-to-one onto Zenoh's stable QoS", "without a verbatim chunk at position 5"),
         []),
        # observer §7.3's sentence is F-379's, which #926 part 3 rewords: read by the subject its fix keeps.
        ("F-370 and F-376, observer.md 0.9's two sentences, and §0's rules without a keyword (F-362)",
         missing(obs.lower(), "tool that shows or judges a payload")
         + missing(obs,
                 "**The subscribers of a reader's watch session SHOULD have no history** when the reader seeds by a "
                 "read on its other session.",
                 "lists both texts' rules stated with a keyword, by role. This text states many without one, in §1, "
                 "§2, §5 and §8. A class holds those too.")
         + present(obs, "A generic tool MUST decode", "SHOULD have no history** when it seeds"), []),
        ("F-361, F-364 and F-366 in RULES.md: S7 binds owner and consumer, a late joiner's MUSTs under observer, §9.4's "
         "file part E032, a pronoun's noun and a list's length shown",
         missing(rules, "| [S7](core.md#42-rules-for-owners-and-consumers) | owner, consumer |",
                 "| [E032](core.md#94-schema-artifacts-and-type-references) | Otherwise it resolves",
                 "Every instance MUST hold one [an instance token]",
                 "An owner MUST bring itself up in this order, so that alive ⇒ callable: (4 items)",
                 "A tag that closes a list cites each rule of it that cites none."),
         []),
        ("F-371 to F-375, spec/README.md's glossary as 0.67 corrected it",
         missing(readme, "- **Reader**: any participant that reads presence, state or the admin space",
                 "with no error reply, before its session's interests timeout",
                 "- **Presence**: the liveliness tokens that say which instances run",
                 "- **Late joiner**: a consumer or a tool",
                 "- **Ambient selector**: a selector with no verbatim chunk, such as `zk2/**`. It reaches no"),
         []),
    ]

    # S7 after the seed: the pure rule, then a late joiner over a fake session.
    st = lambda t, i: sd.Stamp(t, i)  # noqa: E731
    out.append(("core §4.3 S7's consumer rule (seed.follows): within one id by time; a new id's first value kept "
                "though its time be lower; equal stamps not kept twice; an unstamped sample over a stamped one not "
                "kept; a value back under the old id kept, other than the held one's (F-383's reading (a))",
                [sd.follows(st(150, "b2"), st(200, "a1")), sd.follows(st(199, "a1"), st(200, "a1")),
                 sd.follows(st(201, "a1"), st(200, "a1")), sd.follows(st(200, "a1"), st(200, "a1")),
                 sd.follows(None, st(200, "a1")), sd.follows(st(1, "a1"), None),
                 sd.follows(st(210, "a1"), st(160, "b2"))],
                [True, False, True, False, False, True, True]))
    # Round 91: 0.68 corrected 0.67's S7 sentence (F-382). A consumer orders
    # by S7 from its first value on, its seed's replies included, so the
    # seed-then-S7 order round 90 read here (seed.ordered) is gone.
    out.append(("core §4.3 (0.68): a consumer by S7 from its first value, its seed's included (merge.json's two "
                "ids, the new one's time behind, kept), and a tool by the merge (not kept)",
                [sd.consumer_kept([st(200, "a1"), st(150, "b2")]),
                 sd.consumer_kept([st(200, "a1"), st(150, "b2"), st(149, "b2"), st(151, "b2")]),
                 sd.kept([(st(200, "a1"), "put"), (st(150, "b2"), "put")])],
                [[True, True], [True, True, False, True], [True, False]]))
    class Fake:
        pass

    def joiner(consumer: bool) -> list[tuple[str, str, str]]:
        k = "zk2/g/s/zk2py_plans.v1/state/plans/r"
        lj = sd.LateJoiner(Fake(), "zk2/g/s/zk2py_plans.v1/state/plans/*", consumer=consumer)
        lj._merge(k, "reply", False, b"rev10", "application/json", "200/a1")
        lj._merge(k, "live", False, b"in-seed", "application/json", "150/b2")
        lj._ended()
        lj._merge(k, "live", False, b"rev11", "application/json", "160/b2")
        lj._merge(k, "live", False, b"late", "application/json", "155/b2")
        return [(a.outcome, a.rule, str(a.stamp)) for a in lj.applied] + [("held", lj.view()[k].payload.decode(), "")]

    out.append(("seed.LateJoiner (0.68): a consumer's by S7 from its first value, its seed's included, so a new "
                "id's value inside the seed is kept (state.md §7 step 2's rev 11 after it too); a tool's by the merge "
                "throughout (F-382)",
                [joiner(True), joiner(False)],
                [[("kept", "S7", "200/a1"), ("kept", "S7", "150/b2"), ("kept", "S7", "160/b2"),
                  ("dropped", "S7", "155/b2"), ("held", "rev11", "")],
                 [("kept", "merge", "200/a1"), ("dropped", "merge", "150/b2"), ("dropped", "merge", "160/b2"),
                  ("dropped", "merge", "155/b2"), ("held", "rev10", "")]]))

    # Round 90 decided the switch from the merge to S7 by the order of
    # arrival into one queue (review-bot on PR #969). Since 0.68 there is no
    # switch: a sample and the GET's final reply, interleaved both ways, in
    # one thread and in two, leave a consumer's by S7 and a tool's by the
    # merge alike. The queue still orders a key's values as they arrived.
    import threading

    def interleaved(sample_first: bool, threads: bool, consumer: bool = True) -> list[Any]:
        k = "zk2/g/s/zk2py_plans.v1/state/plans/r"
        lj = sd.LateJoiner(Fake(), "zk2/g/s/zk2py_plans.v1/state/plans/*", consumer=consumer)
        lj._merge(k, "reply", False, b"rev10", "application/json", "200/a1")
        sample = (k, "live", False, b"rev11", "application/json", "150/b2")
        if not threads:
            if sample_first:
                lj._merge(*sample, drain=False)  # arrived; its thread is handled after the end's
                lj._ended()                      # the end's thread applies both, in arrival order
                lj._drain()                      # the sample's thread: nothing left
            else:
                lj._ended(drain=False)           # the final reply arrived; its thread waits
                lj._merge(*sample)               # the sample's thread applies both
                lj._drain()
        else:
            first, second = threading.Event(), threading.Event()

            def early() -> None:  # arrives first, handled last
                if sample_first:
                    lj._merge(*sample, drain=False)
                else:
                    lj._ended(drain=False)
                first.set()
                second.wait(5.0)
                lj._drain()

            def late() -> None:   # arrives second, handled at once
                first.wait(5.0)
                if sample_first:
                    lj._ended()
                else:
                    lj._merge(*sample)
                second.set()

            ts = [threading.Thread(target=early), threading.Thread(target=late)]
            for t in ts:
                t.start()
            for t in ts:
                t.join(10.0)
        return [[(a.outcome, a.rule) for a in lj.applied], lj.view()[k].payload.decode(), lj.seed.ended,
                lj._done.is_set()]

    consumer = [[("kept", "S7"), ("kept", "S7")], "rev11", True, True]
    tool = [[("kept", "merge"), ("dropped", "merge")], "rev10", True, True]
    out.append(("seed.LateJoiner (0.68): no switch at the GET's final reply: a new id's sample before or after it "
                "is kept by a consumer (S7 from its first value) and dropped by a tool (the merge for as long as it "
                "follows); in one thread and in two",
                [interleaved(True, False), interleaved(False, False), interleaved(True, True),
                 interleaved(False, True), interleaved(True, False, False), interleaved(False, True, False)],
                [consumer, consumer, consumer, consumer, tool, tool]))
    return out


def _observer_06_readings(root: Path) -> list[tuple[str, Any, Any]]:
    """``observer.md`` text 0.6 (#609, round 86): §5.2's watch after a
    window, settled as round 85's reading A of F-320, "the longer of one
    lease and the routers' `wait_before_close`, plus its reply timeout";
    and ``alarms.v1`` §2.15, which restated 0.5's shorter length and now
    cites §5.2 (its changelog's "0.8, read against observer.md 0.6"). Until
    round 86 zk2py's alarms reader ended its window one lease and the reply
    timeout before the instant it judged, as §2.15 said. It now ends it by
    :func:`zk2py.alarms.window_end_ns`, the budget's length. No session is
    opened."""
    from . import alarms
    from . import budget as bg

    spec = root.parent
    flat = lambda rel: " ".join((spec / rel).read_text(encoding="utf-8").split())  # noqa: E731
    ns = 1_000_000_000
    label = "observer 0.6 §5.2"
    out: list[tuple[str, Any, Any]] = [
        (f"{label}: the text states the watch after a window as the longer of one lease and wait_before_close, "
         f"plus the reply timeout",
         "a tool watches the owner's instance tokens for the longer of one lease and the routers' "
         "`wait_before_close`, plus its reply timeout." in flat("observer.md"), True),
        ("alarms.v1 §2.15 (read against observer 0.6): the owner is watched past the end as observer §5.2 says",
         "The owner is watched past the end as observer §5.2 says: the longer of one lease and the routers' "
         "`wait_before_close`, plus the reply timeout." in flat("profiles/alarms/v1.md"), True),
        ("alarms.v1 scenarios §10 step 2 (read against observer 0.6): its owners are watched as observer §5.2 says",
         "Its owners are watched past it as observer §5.2 says" in flat("profiles/alarms/scenarios.md"), True),
        # (lease, wait_before_close, reply timeout) -> the window's end for a
        # reader judging at 100 s, in seconds
        (f"{label} in alarms.v1 §2.15: a reader judging at 100 s ends its window at 89 s under zenoh's defaults "
         f"(10 s, 5 s, 1 s), at 94 s beside a 1 s lease, at 79 s beside a 20 s lease, at 83 s beside a 15 s "
         f"wait_before_close and a 2 s reply timeout",
         [alarms.window_end_ns(100 * ns) / ns, alarms.window_end_ns(100 * ns, 1.0, 5.0, 1.0) / ns,
          alarms.window_end_ns(100 * ns, 20.0, 5.0, 1.0) / ns, alarms.window_end_ns(100 * ns, 10.0, 15.0, 2.0) / ns],
         [89.0, 94.0, 79.0, 83.0]),
        (f"{label}: the alarms reader's watch is the budget windows' watch, whatever the three lengths",
         [100 * ns - alarms.window_end_ns(100 * ns, ls, ws, rs) == int(bg.watch_after_s(ls, ws, rs) * ns)
          for ls, ws, rs in ((10.0, 5.0, 1.0), (1.0, 5.0, 1.0), (0.0, 5.0, 0.5), (30.0, 5.0, 1.0), (5.0, 5.0, 1.0))],
         [True] * 5),
        (f"{label} in AlarmsReader.window: declared at 0 s, judged at 100 s, nothing lost, the owner held "
         f"throughout: 88 s listened under zenoh's defaults, 93 s beside a 1 s lease (0.8's old words gave "
         f"97 s), 78 s beside a 20 s lease",
         [_alarms_reader_window(alarms, watch, 100 * ns) for watch in
          ((10.0, 5.0, 1.0), (1.0, 5.0, 1.0), (20.0, 5.0, 1.0))], [88.0, 93.0, 78.0]),
        (f"{label} in AlarmsReader.window: an offline reader carries every input window() reads: a loss at 50 s "
         f"and no GET since is not lossless; a withdrawal at 50 s is not present throughout; a complete GET asked "
         f"at 60 s heals both, the window running from it to 89 s; an incomplete one heals nothing",
         [_offline_window(alarms, ns, losses=[50 * ns]), _offline_window(alarms, ns, events=[(50 * ns, "delete")]),
          _offline_window(alarms, ns, losses=[50 * ns], events=[(50 * ns, "delete")], gets=[(60 * ns, True)]),
          _offline_window(alarms, ns, losses=[50 * ns], gets=[(60 * ns, False)])],
         [(88.0, False, True), (88.0, True, False), (29.0, True, True), (88.0, False, True)]),
    ]
    return out


def _offline_window(alarms: Any, ns: int, *, losses: list[int] = (), events: list[tuple[int, str]] = (),
                    gets: list[tuple[int, bool]] = ()) -> tuple[float, bool, bool]:
    """An offline reader's window for ``lab/a`` at 100 s under zenoh's
    defaults, declared at 0 and its owner held from the start: (listened
    seconds, lossless, present throughout)."""
    r = alarms.AlarmsReader.offline(0, {"lab/a": "1"}, losses=losses, token_events={"lab/a": list(events)},
                                    seeds={"lab/a": [alarms.SeedGet(t, ok) for t, ok in gets]})
    w = r.window("lab/a", at_ns=100 * ns)
    return (w.listened_s, w.lossless, w.present_throughout)


def _alarms_reader_window(alarms: Any, watch: tuple[float, float, float], at_ns: int) -> float:
    """:meth:`zk2py.alarms.AlarmsReader.window` on a reader with no session
    (:meth:`zk2py.alarms.AlarmsReader.offline`): declared at 0, its owner
    ``lab/a`` held from the start and never withdrawn, no loss, no GET. Its
    listened seconds."""
    lease_s, wait_before_close_s, reply_timeout_s = watch
    r = alarms.AlarmsReader.offline(0, {"lab/a": "1"}, lease_s=lease_s, reply_timeout_s=reply_timeout_s,
                                    wait_before_close_s=wait_before_close_s)
    return r.window("lab/a", at_ns=at_ns).listened_s


def _observer_05_readings(root: Path) -> list[tuple[str, Any, Any]]:
    """``observer.md`` text 0.5 (#609, round 85): §1's NotAsked that spells
    not this profile's keeps its reason in every medium (F-295), over every
    such answer of the three fixtures the rule cites, and in zk2py's
    freshness tool's member field; and §5.2's watch after a window, the
    longer of the lease and ``wait_before_close`` (F-320). No session is
    opened."""
    from . import budget as bg
    from . import freshness as fr
    from . import judgement as jd

    label = "observer 0.5 §1"
    spec = root.parent
    out: list[tuple[str, Any, Any]] = []
    # "That NotAsked keeps its reason in every medium. A table cell, a
    # report field and a note each say not this profile's, or name the
    # reason, and never show a bare not asked."
    cited = {"freshness.v1": "profiles/freshness/conformance/judgements.json",
             "telemetry.v1": "profiles/telemetry/conformance/samples.json",
             "desired.v1": "profiles/desired/conformance/conclusions.json"}
    out.append((f"{label}: the rule cites the three fixtures §1 names for the NotAsked spelling",
                cited, {p: jd.FIXTURES[p] for p in jd.SPELLINGS}))
    spelled: list[jd.Judgement] = []
    for profile, rel in cited.items():
        doc = json.loads((spec / rel).read_text())
        spelled += [j for j in (jd.read(profile, a, rs) for a, rs in jd.answers(doc))
                    if j.shape == jd.TOKEN and j.spelled == jd.NOT_ASKED]
    bare = ("not asked", "not_asked", "")

    def says(text: str, j: jd.Judgement) -> bool:
        return "not this profile's" in text or any(r in text for r in j.reasons)

    # The 15 (6, 1 and 8) is pinned on purpose, a tripwire, as round 84's
    # counts are: a fixture that gains or loses a NotAsked of the second
    # spelling fails here, so the rule is read against it again rather than
    # passing on whatever the fixtures now hold (review-bot on PR #957)
    out += [
        (f"{label}: the three fixtures give 15 NotAsked answers that spell not this profile's",
         len(spelled), 15),
        (f"{label}: no cell of them is a bare not asked, and each says not this profile's or names its reason",
         [jd.cell(j) for j in spelled if jd.cell(j).strip().lower() in bare or not says(jd.cell(j), j)], []),
        (f"{label}: each field keeps the profile's not_asked with its reasons, never an empty list",
         [jd.field(j) for j in spelled if not jd.field(j)["reasons"] or jd.field(j)["answer"] != "not_asked"], []),
        (f"{label}: each note says not this profile's and names the reason",
         [jd.note(j) for j in spelled if not (says(jd.note(j), j) and "not this profile's" in jd.note(j))], []),
    ]
    # zk2py's freshness tool (scenarios §6): a member's field is its token
    # and its reason, so a deleted member reads not asked, deleted
    members = {"m/1": fr.Verdict(fr.FRESH, "within_horizon"), "m/2": fr.Verdict(fr.NOT_ASKED, "deleted"),
               "m/3": fr.Verdict(fr.UNOBSERVABLE, "silent")}
    fields = fr.member_fields(members)
    out += [
        (f"{label}: zk2py's freshness tool keeps each member's reason beside its token", fields,
         {"m/1": {"verdict": "fresh", "reason": "within_horizon"}, "m/2": {"verdict": "not_asked", "reason": "deleted"},
          "m/3": {"verdict": "unobservable", "reason": "silent"}}),
        (f"{label}: its deleted member's cell is not this profile's, the reason named",
         jd.cell(jd.read(fr.PROFILE, fields["m/2"]["verdict"], fields["m/2"]["reason"])),
         "not this profile's (deleted), spelled not asked"),
    ]

    # The member field's boundary, typed both ways (review-bot on PR #957):
    # a caller still reading round 84's bare token fails, never reads a
    # character of it
    def refused(f: Any, *a: Any) -> str:
        try:
            f(*a)
        except TypeError:
            return "refused"
        return "read"

    got = {"state/status": (fr.Verdict(fr.FRESH, "within_horizon"), fields), "stream/level": (
        fr.Verdict(fr.UNOBSERVABLE, "no_observation"), {})}
    out += [
        (f"{label}: service_members reads read_service's members as their verdicts, each field's shape checked",
         fr.service_members(got), {"state/status": {"m/1": "fresh", "m/2": "not_asked", "m/3": "unobservable"},
                                   "stream/level": {}}),
        (f"{label}: member_verdicts refuses round 84's bare token, a field without its reason, one with more, and "
         f"a verdict that is not §5's",
         [refused(fr.member_verdicts, {"m": "not_asked"}), refused(fr.member_verdicts, {"m": {"verdict": "fresh"}}),
          refused(fr.member_verdicts, {"m": {"verdict": "fresh", "reason": "x", "detail": ""}}),
          refused(fr.member_verdicts, {"m": {"verdict": "gone", "reason": "x"}}),
          refused(fr.service_members, {"r": (fr.Verdict(fr.FRESH, "x"), {"m": "fresh"})})],
         ["refused"] * 5),
        (f"{label}: member_fields refuses a member that is not a Verdict (a token string)",
         refused(fr.member_fields, {"m": "deleted"}), "refused"),
    ]
    # §5.2: "for one lease plus its reply timeout … A deployment that sets a
    # longer lease, or a wait_before_close longer than its lease, lengthens
    # it" (F-320: zk2py reads the longer of the two)
    out.append(("observer 0.5 §5.2: the watch after a window is one lease plus the reply timeout, the longer of the "
                "lease and wait_before_close standing for the lease",
                [bg.watch_after_s(), bg.watch_after_s(20.0, 5.0, 1.0), bg.watch_after_s(10.0, 15.0, 2.0)],
                [11.0, 21.0, 17.0]))
    # A caller's own lease is read the same way (review-bot on PR #957): a
    # lease of 0 still watches wait_before_close and the reply timeout
    out.append(("observer 0.5 §5.2: a caller's lease of 0 s or 1 s still watches zenoh's 5 s wait_before_close and "
                "the reply timeout", [bg.watch_after_s(0.0), bg.watch_after_s(1.0)], [6.0, 6.0]))
    return out


def _observer_04_readings(root: Path) -> list[tuple[str, Any, Any]]:
    """``observer.md`` text 0.3, as text 0.4 states it (#609, round 84):
    §1's two spellings of not this profile's, read over every answer the six
    profiles' fixtures §1 cites give (``zk2py.judgement``), each kept apart
    in the three media; and §8.2's medium, a stream of rows stating its
    scope on standard error (``zk2py.echo_tool``'s NDJSON). No session is
    opened."""
    from . import echo_tool
    from . import judgement as jd

    out: list[tuple[str, Any, Any]] = []
    label = "observer 0.4 §1"
    spec = root.parent
    every: dict[str, list[jd.Judgement]] = {}
    for profile, rel in jd.FIXTURES.items():
        doc = json.loads((spec / rel).read_text())
        every[profile] = [jd.read(profile, a, rs) for a, rs in jd.answers(doc)]
    # What each fixture holds, by shape, and by spelling for not this
    # profile's: the counts the fixtures give today (round 84). They are
    # pinned on purpose, a tripwire: a fixture that gains or loses a case
    # of not this profile's fails here, so the reading is checked again
    # against §1 rather than passing on whatever the fixture now holds
    # (review-bot on PR #955)
    want = {
        "timing.v1": ("token", 11), "arbitration.v1": ("token", 9), "redundancy.v1": ("token", 5),
        "freshness.v1": ("not_asked", 6), "telemetry.v1": ("not_asked", 1), "desired.v1": ("not_asked", 8),
    }
    for profile, (spelled, n) in want.items():
        js = every[profile]
        mine = [j for j in js if j.shape == jd.TOKEN]
        out.append((f"{label}: {profile}'s fixture gives not this profile's {n} times, spelled {spelled}, and no "
                    f"answer of another spelling reads as it",
                    (len(mine), sorted({j.spelled for j in mine})), (n, [spelled])))
        out.append((f"{label}: {profile}: not this profile's never read as Unobservable, nor as Established",
                    [j.shape for j in mine if j.answer == jd.UNOBSERVABLE or j.shape != jd.TOKEN], []))
    # Every other not_asked is NotAsked, and each profile's other reasons
    # never read as not this profile's
    others = {p: sorted({r for j in js if j.shape == jd.NOT_ASKED for r in j.reasons}) for p, js in every.items()}
    out.append((f"{label}: every other not_asked the six fixtures give reads as NotAsked, its reason kept",
                others, {"arbitration.v1": ["no_descriptor", "not_listened"],
                         "desired.v1": ["author_absent", "not_acted", "target_absent", "unbound"],
                         "freshness.v1": ["last_known", "no_horizon"], "redundancy.v1": [
                             "absent", "no_candidate", "not_a_candidate"],
                         "telemetry.v1": ["ignored"],
                         "timing.v1": ["ended", "no_deadline", "no_lifespan", "no_period", "not_heard"]}))
    # A premise reason binds only the profile that names it: timing.v1
    # spells it as the token, so a not_asked "kind" there is NotAsked
    for profile, answer, reasons, shape, why in (
            ("timing.v1", "not_asked", "kind", jd.NOT_ASKED, "a token profile's not_asked, whatever its reason"),
            ("freshness.v1", "not_asked", "no_horizon", jd.NOT_ASKED, "freshness.v1's other reason"),
            ("freshness.v1", "not_asked", "deleted", jd.TOKEN, "freshness.v1's deleted"),
            ("telemetry.v1", "not_asked", ["not_this_profiles"], jd.TOKEN, "telemetry.v1's reason, a list"),
            ("desired.v1", "not_asked", "not_a_target", jd.TOKEN, "desired.v1's not_a_target"),
            ("x.v1", "not_this_profiles", None, jd.TOKEN, "§1's token, from a profile §1 does not name"),
            ("x.v1", "not_asked", "kind", jd.NOT_ASKED, "a not_asked from a profile §1 does not name"),
            ("timing.v1", "unobservable", "not_this_profiles", jd.UNOBSERVABLE,
             "an Unobservable is one whatever its reason says"),
            ("desired.v1", "not_yet", None, jd.ESTABLISHED, "desired.v1's not_yet, an answer the bus gave")):
        out.append((f"{label}: {why} reads as {shape}", jd.read(profile, answer, reasons).shape, shape))
    # The three media keep it apart, whichever spelling: its cell, field and
    # note never say Unobservable or a negative pole, never read as a
    # NotAsked's, and a NotAsked that spells it keeps its reason in the cell
    cells_na = {jd.cell(j) for js in every.values() for j in js if j.shape == jd.NOT_ASKED}
    cells_un = {jd.cell(j) for js in every.values() for j in js if j.shape == jd.UNOBSERVABLE}
    nt = [j for js in every.values() for j in js if j.shape == jd.TOKEN]
    out += [
        (f"{label}: every not this profile's cell says so, and none is a NotAsked's or an Unobservable's cell",
         sorted({jd.cell(j) for j in nt} & (cells_na | cells_un)) + [jd.cell(j) for j in nt
                                                                   if "not this profile's" not in jd.cell(j)], []),
        (f"{label}: every not this profile's field keeps the profile's token and reasons beside shape "
         f"not_this_profiles",
         sorted({(f["answer"], f["shape"], f["established"], f["spelled"]) for f in map(jd.field, nt)}),
         [("not_asked", "not_this_profiles", False, "not_asked"),
          ("not_this_profiles", "not_this_profiles", False, "token")]),
        (f"{label}: every not this profile's note says never no and never unobservable",
         all("never no and never unobservable" in jd.note(j) for j in nt), True),
        (f"{label}: a NotAsked that spells it keeps its reason in the cell (freshness.v1's kind)",
         jd.cell(jd.read("freshness.v1", "not_asked", "kind")), "not this profile's (kind), spelled not asked"),
        (f"{label}: a NotAsked nobody put, its cell and note",
         [jd.cell(jd.read("freshness.v1", "not_asked", "no_horizon")),
          "asking cures it" in jd.note(jd.read("freshness.v1", "not_asked", "no_horizon"))],
         ["not asked: no_horizon", True]),
        (f"{label}: Established poles are the profile's own, and an Unestablished cell never one",
         [jd.cell(jd.read("telemetry.v1", "does_not", ["unit_invalid"])), jd.cell(jd.read("timing.v1", "no")),
          sorted(c for c in cells_na | cells_un if c in ("no", "does not", "stale"))],
         ["does not", "no", []]),
    ]
    # §1's two lists cover the six fixtures §1 cites, each profile once
    out.append((f"{label}: the token-spelled and the NotAsked-spelled profiles are the six §1 cites, none twice",
                (sorted(jd.TOKEN_SPELLED | set(jd.SPELLINGS)), jd.TOKEN_SPELLED & set(jd.SPELLINGS)),
                (sorted(jd.FIXTURES), set())))
    # The NDJSON rows' hand-off (review-bot on PR #955): zenoh's callback
    # only queues a row, and the main thread writes it, so a reader slow to
    # take standard output never stalls the session's delivery; a write
    # that fails is the stream cut, never a clean end
    import io
    import threading
    import time as _time

    rows = echo_tool.Rows(count=3)
    gate, handed = threading.Event(), []

    def blocked(row: dict[str, Any]) -> None:
        gate.wait(5.0)
        handed.append(row["key"])

    pumping = threading.Thread(target=rows.pump, args=(blocked, _time.monotonic() + 5.0), daemon=True)
    pumping.start()
    t0 = _time.monotonic()
    for i in range(5):
        rows.offer(echo_tool.row_of(f"zk2/g/svc/zk2py_guard.v1/stream/cmd{i}", False, 1, "zenoh/bytes"))
    offered_s = _time.monotonic() - t0
    gate.set()
    pumping.join(5.0)
    out += [
        ("observer 0.4 §8.2: the callback's half returns at once while the writer is blocked, and keeps the first "
         "--count rows",
         [offered_s < 0.5, [r["key"][-4:] for r in rows.rows()]], [True, ["cmd0", "cmd1", "cmd2"]]),
        ("observer 0.4 §8.2: the main thread's half hands them on in arrival order, then ends, the count reached",
         [handed, pumping.is_alive()],
         [[f"zk2/g/svc/zk2py_guard.v1/stream/cmd{i}" for i in range(3)], False]),
    ]

    class Refusing(io.StringIO):
        def __init__(self, exc: OSError, on: str) -> None:
            super().__init__()
            self.exc, self.on = exc, on

        def write(self, text: str) -> int:
            if self.on == "write":
                raise self.exc
            return super().write(text)

        def flush(self) -> None:
            if self.on == "flush":
                raise self.exc

    def cut(out: io.StringIO) -> str:
        r = echo_tool.Rows()
        r.offer(echo_tool.row_of("zk2/g/svc/zk2py_guard.v1/stream/cmd", False, 1, "zenoh/bytes"))
        try:
            r.rest(echo_tool.writer(out))
        except echo_tool.StreamCut as e:
            return f"cut: {e}"
        return "written"

    ok = io.StringIO()
    out += [
        ("observer 0.4 §8.2: a row written is one NDJSON line", [cut(ok), ok.getvalue().count("\n")], ["written", 1]),
        ("observer 0.4 §8.2: a closed pipe on write, or any OSError on flush, is the stream cut, never a clean end",
         [cut(Refusing(BrokenPipeError(32, "Broken pipe"), "write")).startswith("cut: "),
          cut(Refusing(OSError(5, "Input/output error"), "flush")).startswith("cut: ")], [True, True]),
    ]
    # §8.2's medium: a stream of rows has no place for the scope, so
    # standard error states it; the rows are the samples alone
    sc = echo_tool.scope_of("zk2/g/**")
    row = echo_tool.row_of("zk2/g/svc/zk2py_guard.v1/stream/cmd", False, 4, "application/json")
    out += [
        # A sample's row holds its payload's declared type, not held, and its
        # sniffing since observer 0.12 (the mend after #926 part 5), still
        # no scope
        ("observer 0.4 §8.2: zk2py's echo tool's NDJSON row is a sample's alone, no scope in it",
         sorted(row), ["bytes", "declared_type", "delete", "encoding", "key", "kind", "sniffed", "zk2"]),
        ("observer 0.4 §8.2: its standard error line states what it watched and every kind it excludes, "
         "whatever the format",
         [all(k in echo_tool.scope_line(sc) for k in ("@stream", "@state", "@op", "@adv", "@zk")),
          "watching stream, state, events" in echo_tool.scope_line(sc)], [True, True]),
        ("observer 0.4 §8.2: a key that is not zk2 is a row too, its kind null (§8.1)",
         echo_tool.row_of("v1/a/b", False, 1, "zenoh/bytes")["kind"], None),
    ]
    return out


def _observer_02_readings(root: Path) -> list[tuple[str, Any, Any]]:
    """``observer.md`` text 0.2 and core 0.59 (#609, round 83): §3.2's
    transient half over three reads a grace apart (operations.md §8 step 6),
    §7.2's two halves in the words operations.md §5 step 2 expects, §7.4's
    report of a limit, §8.2's scope as zk2py's echo tool states it
    (grammar.md §1 step 5), and F-291's reading: the three ways the profiles'
    fixtures give "not this profile's". No session is opened."""
    from . import echo_tool, live

    out: list[tuple[str, Any, Any]] = []
    label = "observer 0.2"
    shape = ("zk2/h4/tc/@zk/instance/0123456789abcdef", "no token", "zk2py_tc.v1")
    for reads, want, why in (
            ([{shape}, set(), set()], (set(), {shape}), "in the first of three reads alone, the token a second "
                                                        "after the descriptor (operations.md §8 step 6): no fault"),
            ([set(), {shape}, {shape}], ({shape}, set()), "in the last two: a fault"),
            ([{shape}, {shape}, set()], ({shape}, set()), "in the first two: a fault, gone after"),
            ([{shape}, set(), {shape}], (set(), {shape}), "in the first and the last, not between: it did not hold "
                                                          "between two reads a grace apart, no fault"),
            ([{shape}] * 3, ({shape}, set()), "in every read, the step's control: a fault"),
            ([set()] * 3, (set(), set()), "in none")):
        out.append((f"{label} §3.2: a shape {why}", live.shape_faults(reads), want))

    def refused(seen: list[set]) -> str:
        try:
            live.shape_faults(seen)
        except ValueError as e:
            return f"refused: {e}"
        return "read"

    out.append((f"{label} §3.2: one read, or none, is no pair a grace apart, and shape_faults refuses it, as "
                f"presence_faults refuses reads < 2 (review-bot on PR #952)",
                [refused([{shape}]), refused([])],
                ["refused: observer §3.2: a fault needs two reads a grace apart"] * 2))
    silent = live.CallResult(replies=[live.CallReply("transport", None, "", b"")])
    told = live.o3_verdict(silent, True, True)
    untold = live.o3_verdict(silent, None, True)
    out += [
        (f"{label} §7.2: told, a present owner's silence is the O3 finding, an access-control refusal said to be "
         f"silent too beside it (operations.md §5 step 2)",
         [told[0], "the O3 finding" in told[1], "an access-control refusal is silent too" in told[1]],
         ["finding", True, True]),
        (f"{label} §7.2: not told, the silence is unobservable, never the finding, and the premise it lacked "
         f"named: grants that let it call, which no tool can observe (core §11.3)",
         [untold[0], "never the finding" in untold[1], "grants that let it call, which no tool can observe" in
          untold[1]], ["unobservable", True, True]),
        (f"{label} §7.2: told, but the owner's tokens not read: unobservable, absent rather than unanswered",
         live.o3_verdict(silent, True, False)[0], "unobservable"),
        (f"{label} §7.2: a silence is never no such operation, told or not",
         any("no such" in v[1] for v in (told, untold)), False),
    ]
    mib = 1 << 20
    out += [
        (f"{label} §7.4: an unlimited limit admits any pool", "admits any SHM pool" in live.memlock_note(-1), True),
        (f"{label} §7.4: below 2 MiB the limit admits none", "it admits no SHM pool" in live.memlock_note(mib), True),
        (f"{label} §7.4: the limit less 2 MiB, the largest pool it covers",
         f"at most {14 * mib} bytes" in live.memlock_note(16 * mib), True),
        (f"{label} §7.4: a limit the tool cannot read leaves the check unobservable",
         live.memlock_said(lambda: (_ for _ in ()).throw(PermissionError("no"))).startswith(
             "the memlock check is unobservable"), True),
    ]
    every = ["@stream", "@state", "@op", "@adv", "@zk"]
    for sel, watched, excluded in (
            ("zk2/g/**", ["stream", "state", "events"], every),
            ("**", ["stream", "state", "events"], every),
            ("zk2/g/*/*/@stream/**", ["@stream"], [k for k in every if k != "@stream"]),
            ("zk2/*/*/*/@state/**", ["@state"], [k for k in every if k != "@state"]),
            ("zk2/*/*/*/state/**", ["state"], every),
            ("zk2/g/svc/zk2py_guard.v1/events/**", ["events"], every),
            ("zk2/g/*/@zk/**", ["@zk"], [k for k in every if k != "@zk"])):
        sc = echo_tool.scope_of(sel)
        out.append((f"{label} §8.2: zk2py's echo tool over {sel} watches {watched} and states {excluded} excluded "
                    f"(grammar.md §1 step 5)", [sc["watched"], sc["excluded"], all(k in sc["says"] for k in excluded)],
                    [watched, excluded, True]))
    # The steps' two contracts: no finding, W107 included, and the
    # fingerprint the reference's CLI computes (round 83, `zk2 contract
    # fingerprint`, run as a black box)
    from .contract import load_contract

    interop = root.parent.parent / "impl" / "python" / "interop"
    for rel, fp in (("grammar/zk2py_guard.v1.toml",
                     "sha256:a02ba26600adf38f98091e13ae7427bc1bb3f2df44c8985ed3cf7d293297e285"),
                    ("operations/zk2py_frozen.v1.toml",
                     "sha256:2d12b9d627171afa8c161afd2b9a7ea0ec928a0b4b93ce1a5bb577b3db87ef79")):
        c = load_contract(interop / rel, check_file_name=True)
        out.append((f"{label}: {rel}, a step's contract, loads with no finding and the reference CLI's fingerprint",
                    (c.codes, c.fingerprint), ([], fp)))
    # F-291 (resolved by text 0.3, which names both spellings; round 84's
    # _observer_04_readings reads them): text 0.2 kept not this profile's
    # apart from NotAsked in every medium, "The profiles' fixtures give it as
    # the token `not_this_profiles`"; three profiles' fixtures give it as not
    # asked.
    spec = root.parent

    def cases(path: str) -> list[dict[str, Any]]:
        return json.loads((spec / path).read_text())["cases"]

    out += [
        ("F-291: timing.v1's conclusions give not this profile's as a token of its own",
         any(c.get("expect", {}).get("answer") == "not_this_profiles"
             for c in cases("profiles/timing/conformance/conclusions.json")), True),
        ("F-291: freshness.v1's judgements give its not this profile's (an event, an operation) as not asked, "
         "reason kind",
         sorted({json.dumps(c["expect"], sort_keys=True) for c in cases("profiles/freshness/conformance/judgements.json")
                 if "not this profile's" in (c.get("note") or "")}),
         ['{"reason": "kind", "verdict": "not_asked"}']),
        ("F-291: telemetry.v1's samples give its not this profile's (a composite state) as not asked, reason "
         "not_this_profiles",
         [(c["expect"]["conformance"], c["expect"]["reasons"]) for c in cases("profiles/telemetry/conformance/samples.json")
          if "not this profile's" in (c.get("note") or "")], [("not_asked", ["not_this_profiles"])]),
        ("F-291: desired.v1 §2.17's not this profile's is not asked, not_a_target, in its slices",
         any(c.get("expect", {}).get("report", {}).get("found") == "not_a_target"
             for c in cases("profiles/desired/conformance/slices.json")), True),
    ]
    return out


def _observer_readings(root: Path) -> list[tuple[str, Any, Any]]:
    """``observer.md`` text 0.1 where zk2py meets what it asks beyond the
    rules the core moved (#609, round 82): a wildcard scope's verbatim kinds
    (§8.2), the timed streams a tool's selector misses, and the memlock
    limit a tool checks at start (§7.4). No session is opened."""
    from . import live
    from . import timing_tool as tt
    from .contract import load_contract

    out: list[tuple[str, Any, Any]] = []
    label = "observer 0.1"
    every = ["@stream", "@state", "@op", "@adv", "@zk"]

    def but(*reached: str) -> list[str]:
        return [k for k in every if k not in reached]

    # §8.2 over live.VERBATIM_REACH, the key forms of core §1.1 and §2.5:
    # for each kind a selector that reaches it, one shorter and one longer
    # than its form, which do not, and selectors too short for any kind.
    for sel, want, why in (
            ("zk2/**", every, "an ambient selector"), ("**", every, "the bus root"),
            ("zk2/lab/pace/**", every, "one owner's ambient selector"),
            ("zk2/a/**", every, "a system's ambient selector"),
            ("zk2", every, "too short for any form"), ("zk2/a/b", every, "too short for any form"),
            ("zk2/*/*/*/*", every, "position 5's `*` matches no kind token"),
            ("zk2/*/*/*/*/**", every, "position 5's `*` matches no kind token, at any length"),
            # @stream, @state, @op: core §1.1's data form, the token at position 5
            ("zk2/lab/pace/pace.v1/@stream/**", but("@stream"), "an explicit stream's subtree"),
            ("zk2/a/b/i/@stream/x/y/z", but("@stream"), "longer than the form's one resource chunk"),
            ("zk2/a/b/@stream/**", every, "@stream at position 4, where no form puts it"),
            ("zk2/*/*/*/@state/**", but("@state"), "every explicit state"),
            ("zk2/a/b/i/@state", every, "the kind token with no resource chunk: no key"),
            ("zk2/a/b/i/@op/set", but("@op"), "one operation"),
            ("zk2/a/b/i/@op", every, "the kind token with no resource chunk: no key"),
            ("zk2/a/b/i/@op/interfaces/x/set", but("@op"), "a templated operation, longer"),
            # @adv: core §2.5, under a plain stream's or state's data key
            ("zk2/a/b/i/stream/x/@adv/**", but("@adv"), "a plain stream's sidecar"),
            ("zk2/a/b/i/stream/x/@adv/y", but("@adv"), "one key under it"),
            ("zk2/a/b/i/state/x/@adv/**", but("@adv"), "a plain state's sidecar"),
            # the reach pattern's `**` before `@adv` spans chunks: fails if
            # zenoh's intersection did not let it
            ("zk2/a/b/i/stream/x/y/z/@adv/**", but("@adv"), "three resource chunks before @adv"),
            # a selector's `**` never crosses `@adv`: fails if it did
            ("zk2/a/b/i/stream/**", every, "a plain stream's subtree, whose ** crosses no @adv"),
            ("zk2/a/b/i/stream/@adv/y", every, "no resource chunk before @adv: no data key"),
            ("zk2/a/b/i/events/x/@adv/y", every, "an event has no advanced publication (core §2.5)"),
            ("zk2/a/b/i/@stream/x/@adv/y", every, "@adv under a verbatim chunk, no form's"),
            # @zk: core §1.1's four control forms
            ("zk2/*/*/@zk/**", but("@zk"), "every service's control keys"),
            ("zk2/a/b/@zk/instance/0123456789abcdef", but("@zk"), "an instance token"),
            ("zk2/a/b/@zk/alive/**", but("@zk"), "the interface tokens"),
            ("zk2/a/b/@zk/instance", every, "shorter than the instance form"),
            ("zk2/a/b/@zk/instance/0123456789abcdef/x", every, "longer than the instance form"),
            ("zk2/@zk/contract/**", but("@zk"), "the contract bundles"),
            ("zk2/@zk", every, "shorter than the contract form"),
            ("zk2/@zk/contract/nav.v2/ab/cd", every, "longer than the contract form")):
        out.append((f"{label} §8.2: {sel} ({why}) never reaches {', '.join(want) or 'nothing'}",
                    live.blind_spots(sel), want))
    # A timed explicit stream beside a plain one: the ambient selector
    # misses the first, and the tool names it unreached; listed from the
    # contract, so a selector naming another address misses both.
    text = ('[interface]\nname = "beat"\nmajor = 1\nminor = 0\nuses = ["timing.v1"]\n\n'
            '[resources.cmd]\nkind = "stream"\ntype = { raw = "text/plain" }\n'
            'annotations = { "timing.period_ms" = 100, "timing.deadline_ms" = 500 }\n\n'
            '[resources.frame]\nkind = "stream"\nexplicit = true\ntype = { raw = "image/jpeg" }\n'
            'annotations = { "timing.period_ms" = 40, "timing.deadline_ms" = 200 }\n\n'
            '[resources.note]\nkind = "stream"\ntype = { raw = "text/plain" }\n')
    c = load_contract(root / "contracts" / "beat.v1.toml", text=text, spec_dir=root.parent).canonical
    out.append((f"{label} §8.2: every timed stream the contract declares at the address, in its canonical order, read from it alone, an untimed stream left out",
                tt.timed_streams(c, "lab/beat"),
                [("@stream/frame", "zk2/lab/beat/beat.v1/@stream/frame"),
                 ("stream/cmd", "zk2/lab/beat/beat.v1/stream/cmd")]))
    for sel, reached, missed in (("zk2/lab/beat/**", ["stream/cmd"], ["@stream/frame"]),
                                 ("zk2/lab/beat/*/@stream/**", ["@stream/frame"], ["stream/cmd"]),
                                 ("zk2/lab/beat/beat.v1/stream/cmd", ["stream/cmd"], ["@stream/frame"]),
                                 ("zk2/lab/other/**", [], ["@stream/frame", "stream/cmd"]),
                                 ("zk2/lab/other/*/@stream/**", [], ["@stream/frame", "stream/cmd"]),
                                 ("zk2/lab/beat/beat.v1/*/**", ["stream/cmd"], ["@stream/frame"])):
        out.append((f"{label} §8.2: a timing tool at lab/beat over {sel} watches {reached} and names {missed} "
                    f"unreached", (tt.timed_rids(c, sel, "lab/beat"), tt.unreached_rids(c, sel, "lab/beat")),
                    (reached, missed)))
    # §7.4: the largest pool a soft limit admits, the pool plus 2 MiB, and
    # core §7.4's words below a pool's need.
    mib = 1 << 20
    for limit, want in ((-1, "unlimited"), (8 * mib, f"at most {6 * mib} bytes"), (2 * mib, "at most 0 bytes"),
                        (64 * 1024, "below 2 MiB"), (8 * mib, live.BELOW_NEED), (64 * 1024, live.BELOW_NEED)):
        note = live.memlock_note(limit)
        out.append((f"{label} §7.4: RLIMIT_MEMLOCK {limit}: the note says {want!r}", want in note, True))
    out.append((f"{label} §7.4: this host's limit reads, and the note cites core §7.4",
                "(core §7.4" in live.memlock_said(), True))

    def unreadable() -> str:
        raise OSError("getrlimit refused")

    out.append((f"{label} §7.4: a limit that cannot be read leaves the check unobservable, said so, and the tool "
                f"goes on (text 0.2, F-290)",
                live.memlock_said(unreadable),
                "the memlock check is unobservable: the limit could not be read: OSError: getrlimit refused"))
    out.append((f"{label} §7.4: any failure, not only the ones expected",
                live.memlock_said(lambda: live.memlock_note("8 MiB")).startswith(
                    "the memlock check is unobservable: the limit could not be read: TypeError: "), True))
    return out


def _unbounded_readings(root: Path) -> list[tuple[str, Any, Any]]:
    """Core 0.57's ``cardinality = "unbounded"`` beyond its two fixtures
    (#609, round 82): "4294967295 from resolution on" (§9.1), so every
    reader of the resolved resource, the bound (§2.7), D007 (§3.3) and the
    compatibility check (§9.8), reads the name as the number. No session is
    opened."""
    from . import budget, compat
    from .contract import NO_CEILING, load_contract
    from .descriptor import _check_exposure

    out: list[tuple[str, Any, Any]] = []
    label = "core 0.57 §2.2"
    d = root / "contracts"

    def load(name: str, body: str):
        text = '[interface]\nname = "seed"\nmajor = 1\nminor = 0\n\n' + body
        return load_contract(d / f"{name}.toml", text=text, spec_dir=root.parent)

    def card(c, rid: str):
        return next(r["cardinality"] for r in c.canonical["resources"] if f"{r['token']}/{r['template']}" == rid)

    stream = load("s", '[resources."a/{x}"]\nkind = "stream"\ntype = { raw = "text/plain" }\n'
                       'params = { x = "string" }\ncardinality = "unbounded"\n')
    number = load("s", '[resources."a/{x}"]\nkind = "stream"\ntype = { raw = "text/plain" }\n'
                       'params = { x = "string" }\ncardinality = 4294967295\n')
    out.append((f"{label}: a templated stream writing \"unbounded\" lints clean, resolved to 4294967295",
                ([x.code for x in stream.diagnostics], card(stream, "stream/a/{x}")), ([], NO_CEILING)))
    out.append((f"{label}: \"unbounded\" and 4294967295 on a stream have one fingerprint",
                stream.fingerprint == number.fingerprint and stream.fingerprint is not None, True))
    op = load("o", '[resources."set/{k}"]\nkind = "operation"\nrequest = { raw = "text/plain" }\n'
                   'response = { raw = "text/plain" }\nparams = { k = "string" }\ncardinality = "unbounded"\n')
    out.append((f"{label}: an operation's template writing \"unbounded\" lints clean, resolved to 4294967295",
                ([x.code for x in op.diagnostics], card(op, "@op/set/{k}")), ([], NO_CEILING)))
    plain = load("p", '[resources.a]\nkind = "stream"\ntype = { raw = "text/plain" }\ncardinality = "unbounded"\n')
    out.append((f"{label}: \"unbounded\" on a template without parameters is E014, as a number there is",
                [x.code for x in plain.diagnostics], ["E014"]))
    for spelled in ("UNBOUNDED", "4294967295", "", "unbounded "):
        c = load("e", '[resources."a/{x}"]\nkind = "stream"\ntype = { raw = "text/plain" }\n'
                      f'params = {{ x = "string" }}\ncardinality = "{spelled}"\n')
        out.append((f"{label}: cardinality = {spelled!r}, any other string, is E000 (§9.1)",
                    [x.code for x in c.diagnostics], ["E000"]))
    # §2.7 "The bound": "the no-ceiling value, 4294967295, is no bound, from
    # either source"; an instance that knows its population MAY lower it.
    b = card(stream, "stream/a/{x}")
    out.append((f"{label}: an \"unbounded\" contract's bound is no ceiling (§2.7)", budget.bound(b, [None]),
                "no_ceiling"))
    out.append((f"{label}: an \"unbounded\" contract lowered to 64 by an instance is 64 (§2.7, §3.3)",
                budget.bound(b, [64]), 64))
    # §3.3, D007: a stated value "from 1 to the contract's".
    out.append((f"{label}: an instance stating 4294967295 of an \"unbounded\" template is within it (no D007)",
                _check_exposure({"cardinality": {"stream/a/{x}": NO_CEILING}}, stream, set()), []))
    out.append((f"{label}: an instance stating 4294967296 of an \"unbounded\" template raises it, D007",
                _check_exposure({"cardinality": {"stream/a/{x}": NO_CEILING + 1}}, stream, set()), ["D007"]))
    # §9.8: one canonical form, one revision.
    rev = lambda c: compat.Revision(c.canonical, {a.id: a.data for a in c.schemas.artifacts()})  # noqa: E731
    out.append((f"{label}: \"unbounded\" against 4294967295 is the same revision (§9.8)",
                compat.identical(rev(number), rev(stream)), True))
    return out


def _arbitration_readings(root: Path) -> list[tuple[str, Any, Any]]:
    """arbitration.v1 0.1 in :mod:`zk2py.arbitration` (#609, round 77): the
    walkthrough's arbitrated role and its binding, the scenarios' contracts,
    the linter, and the arbiter fed F-268's two orders. No session is
    opened."""
    import tomllib

    from . import arbitration as ab
    from .contract import load_contract

    out: list[tuple[str, Any, Any]] = []
    label = "arbitration.v1 0.1"
    repo = root.parent.parent
    walk = repo / "examples/zk2/walkthrough"
    thruster, twist = load_contract(walk / "thruster.v1.toml"), load_contract(walk / "twist_cmd.v1.toml")
    want = {"declaration": "arbitrated", "policy": "priority", "resource": "stream/cmd", "deadline_ms": 100,
            "period_ms": 20}
    out.append((f"{label} Appendix B: thruster.v1's cmd over twist_cmd.v1, as the walkthrough writes them",
                ab.declaration(ab.role_of(thruster.canonical, "cmd"), ab.provider_of(twist.canonical)).expect(), want))
    out.append((f"{label} §2.2: thruster.v1 draws no warning of the profile's linter, twist_cmd.v1 held",
                ab.lint(thruster.canonical, {"twist_cmd.v1": twist.canonical}), []))
    enr = tomllib.loads((repo / "examples/zk2/acl/walkthrough.enrollment.toml").read_text())
    binds = [s["bindings"]["cmd"]["providers"] for s in enr.get("service", []) if "cmd" in s.get("bindings", {})]
    out.append((f"{label} §2.4: the walkthrough enrollment's two thrusters bind cmd in priority order, exact entries",
                [ab.ranking(b, []) for b in binds],
                [{"ranked": ["vehicle-01/safety", "vehicle-01/teleop", "vehicle-01/autopilot"], "unbound": [],
                  "wildcards": [], "shadowed": []}] * 2))
    idir = repo / "impl/python/interop/arbitration"
    rudder, steer = load_contract(idir / "rudder.v1.toml"), load_contract(idir / "steer.v1.toml")
    out.append((f"{label}: the scenarios' rudder.v1 over steer.v1, a 300 ms deadline and a 50 ms period",
                ab.declaration(ab.role_of(rudder.canonical, "cmd"), ab.provider_of(steer.canonical)).expect(),
                {**want, "deadline_ms": 300, "period_ms": 50}))
    bad = {"uses": ["arbitration.v1"], "requires": {"cmd": {"interface": "steer.v1", "resources": ["cmd"],
                                                            "cardinality": "one",
                                                            "annotations": {"arbitration.policy": "freshest"}}}}
    out.append((f"{label} §2.2: the linter's checks 1 to 5 without the required interface, 6 and 7 with it",
                (ab.lint(bad), ab.lint({**bad, "requires": {"cmd": {**bad["requires"]["cmd"], "cardinality": "many",
                                                                    "annotations": {"arbitration.policy": "priority"},
                                                                    "resources": ["nope"]}}},
                                      {"steer.v1": steer.canonical})),
                ([("cmd", "policy_unsupported"), ("cmd", "cardinality")], [("cmd", "resource")])))
    # F-268: the close's withdrawals heard before the move's event, as
    # zenoh-python 1.10.1 tells them, against the move heard first.
    tok = "zk2/lab/{}/@zk/alive/steer.v1/00000000000000a{}/6f2b0c1d9e8a7b65"
    seq = [{"at": 0, "event": "start"}, {"at": 1, "event": "declared", "token": tok.format("safety", 1)},
           {"at": 2, "event": "declared", "token": tok.format("teleop", 2)},
           {"at": 10, "event": "sample", "key": "zk2/lab/safety/steer.v1/stream/cmd"},
           {"at": 15, "event": "sample", "key": "zk2/lab/teleop/steer.v1/stream/cmd"}]
    close = [{"at": 20, "event": "withdrawn", "token": tok.format("safety", 1)},
             {"at": 20, "event": "withdrawn", "token": tok.format("teleop", 2)}]
    reopen = [{"at": 40, "event": "moved"}, {"at": 41, "event": "declared", "token": tok.format("safety", 1)},
              {"at": 41, "event": "declared", "token": tok.format("teleop", 2)}, {"at": 50, "event": "stood"}]
    for name, events, want_sw in (
            ("the move's event after its withdrawals: withdrawn, to the dead-man", seq + close
             + [{"at": 20, "event": "moved"}] + reopen,
             [("lab/safety", "lab/teleop", "withdrawn"), ("lab/teleop", None, "withdrawn")]),
            ("the move begun at the first withdrawal heard with no link: nothing", seq
             + [{"at": 20, "event": "moved"}] + close + [{"at": 20, "event": "moved"}] + reopen, [])):
        a = ab.Arbiter("steer.v1", "stream", "cmd", {}, 300, ["lab/safety", "lab/teleop"])
        got = [s for e in events[len(seq):] for s in a.event(e).switches] if all(a.event(e) for e in seq) else []
        out.append((f"{label} §2.8 (F-268): {name}", [(s.from_, s.to, s.cause) for s in got], want_sw))
    # text 0.2 (round 78): the counts line in rank order, every provider
    # known (F-271); scenarios §11's governed revision.
    label2 = "arbitration.v1 0.2"
    a = ab.Arbiter("steer.v1", "stream", "cmd", {}, 300, ["lab/safety", "lab/*"])
    for e in ({"at": 0, "event": "start"}, {"at": 1, "event": "sample", "key": "zk2/lab/teleop/steer.v1/stream/cmd"},
              {"at": 2, "event": "sample", "key": "zk2/lab/autopilot/steer.v1/stream/cmd", "delete": True},
              {"at": 3, "event": "sample", "key": "zk2/lab/safety/steer.v1/stream/cmd", "incumbent": False}):
        a.event(e)
    line = a.counts_line()
    out.append((f"{label2} scenarios' conventions (F-271): the counts line lists every provider known in rank order, "
                f"five counts each: the exact entry first, then the wildcard's heard by system and service",
                [(k, v["delivered"], v["deleted"], v["not_incumbent"], len(v)) for k, v in line["providers"].items()],
                [("lab/safety", 0, 0, 1, 5), ("lab/autopilot", 0, 1, 0, 5), ("lab/teleop", 1, 0, 0, 5)]))
    gov = load_contract(idir / "steer-governed.v1.toml")
    out.append((f"{label2} scenarios §11: steer.v1 minor 1, governed (claim on [defaults]), loads with no finding, "
                f"the reference CLI's fingerprint, and rudder.v1 over it is arbitrated as over minor 0",
                (gov.valid, gov.codes, gov.fingerprint,
                 ab.declaration(ab.role_of(rudder.canonical, "cmd"), ab.provider_of(gov.canonical)).expect()),
                (True, [], "sha256:26b19c8cc407214e345624e20424da3be3b6ce761c6e56ceecb1773934d04f9c",
                 {**want, "deadline_ms": 300, "period_ms": 50})))
    out += _arbitration_review_readings(root)
    out += _arbitration_tool_readings(root)
    return out


def _arbitration_tool_readings(root: Path) -> list[tuple[str, Any, Any]]:
    """arbitration.v1 0.3's tool (#609, round 79): what a view reads that no
    descriptor carries, and when it has listened (§2.14). No session is
    opened."""
    from . import arbitration as ab
    from .contract import load_contract

    label = "arbitration.v1 0.3"
    idir = root.parent.parent / "impl/python/interop/arbitration"
    steer, gov = (load_contract(idir / f).canonical for f in ("steer.v1.toml", "steer-governed.v1.toml"))
    out: list[tuple[str, Any, Any]] = []
    out.append((f"{label} §2.14: a view reads outcome 6 only when every revision held lists redundancy.v1: none, "
                f"minor 0, minor 1, both",
                [ab.view_governed(r) for r in ([], [steer], [gov], [steer, gov])], [False, False, True, False]))
    out.append((f"{label} §2.14: listening counts from the last subscription to the role's stream",
                (ab.listened_ns([], 10), ab.listened_ns([4_000, 4_100], 4_400)), (0, 300)))
    decl = ab.declaration(ab.role_of({"uses": ["arbitration.v1"], "requires": {"cmd": {
        "interface": "steer.v1", "resources": ["cmd"], "cardinality": "many",
        "annotations": {"arbitration.policy": "priority"}}}}, "cmd"), ab.provider_of(steer))
    # a watch's round trip of 4 s after a reopen beside a large population,
    # its subscriptions declared after it, the view asked 100 ms later
    t_watch, t_subs, now = 0, 4_000 * 1_000_000, 4_100 * 1_000_000
    out.append((f"{label} §2.14: a view in the dead-man 100 ms after its subscriptions, its watch's 4 s round trip "
                f"before them: listening, never dead_man as a count from the watch would read it",
                (ab.conclude(decl, ["lab/safety"], ab.View(ab.listened_ns([t_subs], now), False, None))["selection"]
                 .reason, ab.conclude(decl, ["lab/safety"], ab.View(now - t_watch, False, None))["selection"].reason),
                ("listening", "dead_man")))
    return out


def _arbitration_review_readings(root: Path) -> list[tuple[str, Any, Any]]:
    """Review-bot on PR #912 (#609, round 77): the actuator's timer never
    spins, its round trip after a move is never given up, and a binding
    entry that is not one is refused. No session is opened: the round trip
    runs on stand-ins for ``live.settle`` and ``live.links_held``."""
    import copy
    import threading

    from . import arbitration as ab
    from . import live
    from .arbitration_live import Actuator

    out: list[tuple[str, Any, Any]] = []
    label = "arbitration.v1 0.1 (review-bot on PR #912)"
    # 1. A tick at a past due always moves the state: over every state
    # arbitrations.json reaches, a tick 1 ns after the due changes the
    # active provider or its due.
    path = root.parent / "profiles" / "arbitration" / "conformance" / "arbitrations.json"
    states = moved = 0
    for c in _load_json(path)["cases"]:
        a = ab.arbiter_of(c)
        for e in c["events"]:
            a.event(e)
            due = a.due()
            if due is None:
                continue
            states += 1
            b = copy.deepcopy(a)
            before = (b.active, due)
            b.tick(due + 1)
            moved += (b.active, b.due()) != before
    out.append((f"{label}: the timer: a tick just past the due moves the state, in every state the fixture reaches "
                f"with a due ({states})", moved, states))

    # 2. The round trip after a move: asked until one stands.
    class Fake:
        pass

    def fake(gen_now: int) -> Any:
        f = Fake()
        f.lock = threading.RLock()
        f.log, f.round_trips, f.warned = [], [], []
        f.gen, f.closed, f.t0, f.retry_s = gen_now, False, 0, 0.0
        f.selectors = ["zk2/lab/safety/@zk/alive/steer.v1/**"]
        f.watch = object()
        f.warn = f.warned.append
        f.arb = ab.Arbiter("steer.v1", "stream", "cmd", {}, 300, ["lab/safety", "lab/teleop"])
        f.arb.start(0)
        f.arb.moved(1)
        f.handled = []
        f._handle = f.handled.append
        return f

    saved = live.settle, live.links_held
    try:
        answers = [False, False, True]
        live.settle = lambda *a, **k: live.RoundTrip(answers.pop(0), True, "k", "" if not answers else "x")
        live.links_held = lambda s: True
        f = fake(1)
        Actuator._round_trip(f, 1)  # type: ignore[arg-type]
        out.append((f"{label}: two round trips that do not stand, then one that does: asked three times, the kept "
                    f"events applied once, each failure said",
                    ([r["stands"] for r in f.round_trips], f.arb.pending, len(f.handled),
                     sum(1 for _, w, _ in f.log if w.startswith("pending:")), len(f.warned)),
                    ([False, False, True], False, 1, 2, 2)))
        answers = [False, False, False]
        f = fake(1)
        calls = []

        def settle_then_move(*a: Any, **k: Any) -> Any:
            calls.append(1)
            if len(calls) == 2:
                f.gen = 2  # a later move: it asks its own
            return live.RoundTrip(False, True, "k", "not yet")

        live.settle = settle_then_move
        Actuator._round_trip(f, 1)  # type: ignore[arg-type]
        out.append((f"{label}: a later move during the retries ends this move's, applying nothing",
                    (len(calls), f.arb.pending, len(f.handled)), (2, True, 0)))
        cut = iter([False, False, True])
        live.links_held = lambda s: next(cut, True)
        live.settle = lambda *a, **k: live.RoundTrip(True, True, "k", "")
        f = fake(1)
        Actuator._round_trip(f, 1)  # type: ignore[arg-type]
        out.append((f"{label}: cut off, it waits for a link, saying so, then asks and applies",
                    (sum(1 for _, w, _ in f.log if w.startswith("pending: cut off")), f.arb.pending),
                    (2, False)))
    finally:
        live.settle, live.links_held = saved

    # 3. A binding entry that is not <chunk>/<chunk> is refused, never left
    # silently unbound.
    out.append((f"{label}: entries checked: exact, wildcard, and three that are not",
                [ab.entry_valid(e) for e in ("lab/safety", "*/teleop", "lab", "lab/a/b", "Lab/x")],
                [True, True, False, False, False]))
    try:
        ab.Arbiter("steer.v1", "stream", "cmd", {}, 300, ["lab"])
        refused = False
    except ValueError:
        refused = True
    out.append((f"{label}: the arbiter refuses an entry without a /", refused, True))
    return out


def _timing_readings(root: Path) -> list[tuple[str, Any, Any]]:
    """timing.v1 0.1 in :mod:`zk2py.timing` (#609, round 76): the adopters'
    contracts, the profile's linter, and zk2py's readings of F-261, F-262
    and F-264. No session is opened."""
    from . import timing as tm
    from .contract import load_contract

    out: list[tuple[str, Any, Any]] = []
    label = "timing.v1 0.1"
    repo = root.parent.parent
    # Appendix A's two adopters, and core's ok-kinds (the text's §0 sources).
    adopters = [(repo / "examples/zk2/walkthrough/twist_cmd.v1.toml", "stream/cmd",
                 {"declaration": "declared", "period_ms": 20, "deadline_ms": 100, "lifespan_ms": 100,
                  "below_period": False}),
                (repo / "examples/zk2/walkthrough/nav.v2.toml", "stream/position",
                 {"declaration": "declared", "period_ms": 100, "deadline_ms": None, "lifespan_ms": 500,
                  "below_period": False}),
                (repo / "impl/python/interop/timing/pace.v1.toml", "stream/cmd",
                 {"declaration": "declared", "period_ms": 100, "deadline_ms": 500, "lifespan_ms": 300,
                  "below_period": False}),
                (repo / "impl/python/interop/timing/pace.v1.toml", "stream/axis/{axis}",
                 {"declaration": "declared", "period_ms": 100, "deadline_ms": 500, "lifespan_ms": None,
                  "below_period": False})]
    for path, rid, want in adopters:
        c = load_contract(path)
        r = next(x for x in c.canonical["resources"] if f"{x['token']}/{x['template']}" == rid)
        out.append((f"{label}: {path.name}'s {rid} declares as the text reads it",
                    tm.declaration(r["kind"], r["annotations"]).expect(), want))
        out.append((f"{label}: {path.name} draws no warning of the profile's linter", tm.lint(c.canonical), []))
    # The linter's four SHOULDs (§2.2, §2.3, §2.5), over a canonical form.
    canon = {"uses": ["timing.v1"], "requires": {"src": {"annotations": {"timing.deadline_ms": 50}},
                                                 "plain": {"annotations": {}}},
             "resources": [
                 {"token": "state", "template": "s", "kind": "state", "annotations": {"timing.period_ms": 20}},
                 {"token": "stream", "template": "a", "kind": "stream", "annotations": {"timing.deadline_ms": 0}},
                 {"token": "stream", "template": "b", "kind": "stream",
                  "annotations": {"timing.deadline_ms": 50, "timing.period_ms": 100}},
                 {"token": "stream", "template": "c", "kind": "stream",
                  "annotations": {"timing.deadline_ms": 100, "timing.period_ms": 100}}]}
    out.append((f"{label}: the linter warns of a key on a state, an invalid value, a deadline below the period and "
                f"a key on a requirement, and of nothing else",
                [(w.what, w.where) for w in tm.lint(canon)],
                [("ignored_kind", "state/s"), ("invalid_value", "stream/a"), ("below_period", "stream/b"),
                 ("ignored_requirement", "requires.src")]))
    # F-264: §5's roll-up over a resource, by each question's finding. A
    # roll-up whose reason is read takes (key, Answer) pairs (review-bot on
    # PR #941): keyed here a, b, c… in the order listed, so byte order is it.
    expired, live = tm.Answer(tm.YES, "expired"), tm.Answer(tm.NO, "live")
    unobs = tm.Answer(tm.UNOBSERVABLE, "no_bound")

    def keyed(answers: list[Any]) -> list[tuple[str, Any]]:
        return [(chr(ord("a") + i), a) for i, a in enumerate(answers)]

    out.append((f"{label} §5 (F-264): the lifespan over a resource with an expired member beside a live one is the "
                f"finding, yes", tm.rollup("lifespan", keyed([live, expired])).pair(), {"answer": "yes", "reason": "expired"}))
    out.append((f"{label} §5 (F-264): the sentence as written answers no there, hiding the finding",
                tm.rollup_literal([live, expired]).pair(), {"answer": "no", "reason": "live"}))
    out.append((f"{label} §5 (F-264): an expired member beside an unobservable one is the finding",
                tm.rollup("lifespan", keyed([unobs, expired])).pair(), {"answer": "yes", "reason": "expired"}))
    out.append((f"{label} §5: the deadline over a resource, one member missed, is no",
                tm.rollup("deadline", keyed([tm.Answer(tm.YES, "met"), tm.Answer(tm.NO, "missed")])).pair(),
                {"answer": "no", "reason": "missed"}))
    out.append((f"{label} §5: a member not asked, or not this profile's, set aside: every other yes is yes",
                tm.rollup("period", keyed([tm.Answer(tm.NOT_ASKED, "not_heard"), tm.Answer(tm.YES, "kept"),
                                           tm.Answer("not_this_profiles", "kind")])).pair(),
                {"answer": "yes", "reason": "kept"}))
    # F-262: a missed member never heard stays missed when its owner is
    # present again: one missed, then met at the arrival.
    t = tm.Tracker(100, 1, ["cmd"])
    for e in ({"at_ms": 0, "event": "owner", "state": "present"}, {"at_ms": 0, "event": "start"},
              {"at_ms": 101, "event": "tick"}, {"at_ms": 150, "event": "owner", "state": "unconfirmed"},
              {"at_ms": 160, "event": "owner", "state": "absent"}, {"at_ms": 200, "event": "owner", "state": "present"},
              {"at_ms": 400, "event": "tick"}, {"at_ms": 450, "event": "arrival", "member": "cmd"}):
        t.event(e)
    out.append((f"{label} §2.5 (F-262): a missed member, its owner absent then present again, is not waited for "
                f"again: one missed, then met at its arrival",
                [(r.event, tm.ns_to_ms(r.at_ns)) for r in t.reports], [("missed", 101), ("met", 450)]))
    # F-261, answered by text 0.2: a known member of an epoch template is
    # waited for from its first member token seen, whichever came first, the
    # reader's read or the owner's declaration.
    label2 = "timing.v1 0.2"
    t = tm.Tracker(100, 4, ["axis/x"], epoch=True)
    for e in ({"at_ms": 0, "event": "start"}, {"at_ms": 5, "event": "member_gone", "member": "axis/x"},
              {"at_ms": 50, "event": "owner", "state": "present"}, {"at_ms": 1000, "event": "tick"},
              {"at_ms": 1200, "event": "member_seen", "member": "axis/x"}, {"at_ms": 1301, "event": "tick"}):
        t.event(e)
    out.append((f"{label2} §2.5 (F-261): a read before the owner declared a known epoch member ends nothing, its "
                f"owner's presence waits for nothing, and its member token seen starts the wait",
                [(r.event, tm.ns_to_ms(r.at_ns), r.due_ns and tm.ns_to_ms(r.due_ns)) for r in t.reports],
                [("missed", 1301, 1300)]))
    t = tm.Tracker(100, 4, ["axis/x"], epoch=True)
    for e in ({"at_ms": 0, "event": "start"}, {"at_ms": 50, "event": "owner", "state": "present"},
              {"at_ms": 60, "event": "member_seen", "member": "axis/x"}, {"at_ms": 161, "event": "tick"}):
        t.event(e)
    out.append((f"{label2} §2.5 (F-261): the same member, its token seen at the reader's first read, is missed a "
                f"deadline after it",
                [(r.event, tm.ns_to_ms(r.due_ns)) for r in t.reports], [("missed", 160)]))
    # 0.1's reading, kept to show what 0.2 changed: without the epoch rule a
    # read before the owner declared the member ended it for good.
    t = tm.Tracker(100, 4, ["axis/x"])
    for e in ({"at_ms": 0, "event": "start"}, {"at_ms": 5, "event": "member_gone", "member": "axis/x"},
              {"at_ms": 50, "event": "owner", "state": "present"}, {"at_ms": 1000, "event": "tick"}):
        t.event(e)
    out.append((f"{label} §2.5 (F-261, as 0.1 read): with no epoch rule, that read ended the member for good",
                ([r.expect() for r in t.reports], t.next_due()), ([], None)))
    # F-260: beyond_bound once, while the member stays past the bound; held
    # later, it is a member first heard, and past the bound again reported
    # again.
    t = tm.Tracker(100, 1, [], epoch=True)
    for e in ({"at_ms": 0, "event": "start"}, {"at_ms": 0, "event": "member_seen", "member": "a"},
              {"at_ms": 0, "event": "arrival", "member": "a"}, {"at_ms": 10, "event": "arrival", "member": "b"},
              {"at_ms": 20, "event": "arrival", "member": "b"}, {"at_ms": 30, "event": "member_gone", "member": "a"},
              {"at_ms": 40, "event": "arrival", "member": "b"}, {"at_ms": 50, "event": "arrival", "member": "a"},
              {"at_ms": 60, "event": "arrival", "member": "a"}):
        t.event(e)
    out.append((f"{label2} §2.5 (F-260): beyond_bound once per stay past the bound",
                [(r.event, r.member, tm.ns_to_ms(r.at_ns)) for r in t.reports],
                [("beyond_bound", "b", 10), ("ended", "a", 30), ("beyond_bound", "a", 50)]))
    # F-259: no tracker under an invalid declaration, a deadline below the
    # period, a state, or no deadline.
    out.append((f"{label2} §2.2, §2.5 (F-259): a tracker only where a deadline is judged",
                [tm.tracker_for(tm.declaration(k, a)) is not None for k, a in (
                    ("stream", {"timing.deadline_ms": 100, "timing.lifespan_ms": "100"}),
                    ("stream", {"timing.deadline_ms": 50, "timing.period_ms": 100}),
                    ("state", {"timing.deadline_ms": 100}),
                    ("stream", {"timing.lifespan_ms": 100}),
                    ("@stream", {"timing.deadline_ms": 100, "timing.period_ms": 100}))],
                [False, False, False, False, True]))
    # F-275's reading (round 78) is deadlines.json's case since text 0.3.
    # §2.6: a zid of 33 digits states none; one of 32 is one.
    out.append((f"{label} §2.6: a meta.zid of 33 hexadecimal digits states none, one of 32 does",
                (tm.zid_value("1" * 33), tm.zid_value("f" * 32) == 2**128 - 1), (None, True)))
    out += _timing_tool_readings()
    return out


def _timing_tool_readings() -> list[tuple[str, Any, Any]]:
    """timing.v1 0.3's tool (#609, round 79): §2.7's known members and its
    loss without an owner, and §2.8's ``meta.zid`` on the deployment's word.
    No session is opened."""
    from . import timing as tm

    label = "timing.v1 0.3"
    out: list[tuple[str, Any, Any]] = []
    out.append((f"{label} §2.7: a tool knows a member only when its template has no parameter, pace.v1's cmd and "
                f"not axis/{{axis}}, a selector naming axis/x binding nothing",
                [tm.tool_known(t) for t in ("cmd", "axis/{axis}", "a/b", "rest/{tail...}")],
                [True, False, True, False]))
    out.append((f"{label} §2.7: a loss the tool can attribute is its member's; one it cannot, a queue's refusal or a "
                f"stall, every member's",
                ([tm.lost_for(m, ["cmd"], False) for m in ("cmd", "axis/x")],
                 [tm.lost_for(m, [], True) for m in ("cmd", "axis/x")]), ([True, False], [True, True])))
    dec = tm.declaration("stream", {"timing.period_ms": 20, "timing.deadline_ms": 100, "timing.lifespan_ms": 100})
    stamps = [tm.Stamp(5 * tm.MS, "1f"), tm.Stamp(250 * tm.MS, "1f")]
    out.append((f"{label} §2.8: across a face a descriptor that did reach the reader states nothing there; the "
                f"deployment's word alone: none, every sample unattributed; given, judged as anywhere",
                [[v.reason for v in tm.judge_lifespans(dec, 100, tm.owners_held(True, ["1f"], zids), stamps)]
                 for zids in ([], ["1F"])], [["unattributed", "unattributed"], ["live", "expired"]]))
    out.append((f"{label} §2.6: away from a face the descriptors are the word, the deployment's set aside",
                tm.owners_held(False, ["1f", "1f", "2a"], ["99"]), ["1f", "2a"]))
    # F-277 (round 79), resolved by text 0.4: §5's roll-up whose every
    # member is set aside is not this profile's when every one is, else not
    # asked; no member at all stays unobservable. Since text 0.5 (F-279) the
    # reason is the first member's giving the answer, and over no member
    # there is none (conclusions.json #43 to #49 hold the rule itself).
    na, nt = tm.Answer(tm.NOT_ASKED, "no_lifespan"), tm.Answer("not_this_profiles", "kind")
    out.append(("timing.v1 0.5 §5 (F-277, F-279): a roll-up whose every member is set aside: not asked with the "
                "first not-asked member's reason, not this profile's when every one is, unobservable with no reason "
                "over no member, and a set-aside member beside one pole changing nothing",
                [tm.rollup("lifespan", [(f"m{i}", x) for i, x in enumerate(a)]).pair()
                 for a in ([na, na], [nt, na], [nt, nt], [], [nt, tm.Answer(tm.NO, "live")],
                           [na, tm.Answer(tm.UNOBSERVABLE, "no_bound")])],
                [{"answer": "not_asked", "reason": "no_lifespan"}, {"answer": "not_asked", "reason": "no_lifespan"},
                 {"answer": "not_this_profiles", "reason": "kind"}, {"answer": "unobservable", "reason": None},
                 {"answer": "no", "reason": "live"}, {"answer": "unobservable", "reason": "no_bound"}]))
    # Text 0.5's held time beyond deadlines.json's cases (round 81). F-278's
    # and F-280's readings went: deadlines.json #46 to #51, periods.json #21
    # and conclusions.json #50 hold them.
    label5 = "timing.v1 0.5 §2.7"
    dl = tm.declaration("stream", {"timing.deadline_ms": 500})

    def run(t: tm.Tracker, events: list[dict[str, Any]]) -> tm.Tracker:
        for e in events:
            t.event(e)
        return t

    def held(t: tm.Tracker) -> dict[str, Any]:
        return {k: tm.ns_to_ms(v) for k, v in t.held_ns().items()}  # type: ignore[union-attr]

    def deadline_of(t: tm.Tracker, m: str) -> dict[str, Any]:
        reps = [r for r in t.reports if r.member == m]
        acct = tm.DeadlineAccount(t.held_ns(m), sum(1 for r in reps if r.event == "missed" and r.owner == tm.PRESENT),
                                  sum(1 for r in reps if r.event == "missed" and r.owner != tm.PRESENT),
                                  t.first_end(m) is not None)
        return tm.conclude(dl, tm.Reading(acct, tm.NOT_HEARD, (), False, False, False))["deadline"].pair()

    # A known member never heard, missed while its owner was present, then
    # its owner gone: a missed member is held while it stays missed (the
    # CHANGELOG's "deliberately not changed"; deadlines.json's "heard, missed
    # or waited for"), so the owner's absence after the miss is no pause.
    t = run(tm.Tracker(500, 1, ["cmd"]), [
        {"at_ms": 0, "event": "owner", "state": "present"}, {"at_ms": 0, "event": "start"},
        {"at_ms": 501, "event": "tick"}, {"at_ms": 1000, "event": "owner", "state": "unconfirmed"},
        {"at_ms": 1100, "event": "owner", "state": "absent"}, {"at_ms": 3000, "event": "end"}])
    out.append((f"{label5}: a known member never heard, missed with its owner present, its owner then absent: held "
                f"to the window's end while it stays missed, and its deadline no (missed)",
                (held(t), deadline_of(t, "cmd")), ({"cmd": 3000}, {"answer": "no", "reason": "missed"})))
    # A known member waited for, its owner absent at 300 (a pause), then
    # heard from 1000 every 100 ms with its owner still absent (a sample a
    # publisher went on putting after its tokens went): heard, it is held
    # from that arrival whatever presence says, a new stretch of 2,000 ms,
    # and no miss: met. Read with presence alone restarting a stretch, as
    # v1.md's "its presence again starts a new one" alone would, it would be
    # held 300 ms: window_too_short.
    t = run(tm.Tracker(500, 1, ["cmd"]), [
        {"at_ms": 0, "event": "owner", "state": "present"}, {"at_ms": 0, "event": "start"},
        {"at_ms": 300, "event": "owner", "state": "absent"},
        *({"at_ms": 1000 + 100 * i, "event": "arrival", "member": "cmd"} for i in range(20)),
        {"at_ms": 3000, "event": "end"}])
    out.append((f"{label5}: a known member paused by its owner's absence, then heard while its owner is absent: a new "
                f"stretch from that arrival, 2,000 ms, never the 300 ms before the pause; met",
                (held(t), deadline_of(t, "cmd")), ({"cmd": 2000}, {"answer": "yes", "reason": "met"})))
    # A member not known, heard 100 and 200, ended at 300 (its token gone
    # by a complete read), not heard again: no longer held at the window's
    # end, and held in it, 200 ms, ended.
    t = run(tm.Tracker(500, 4, [], epoch=True), [
        {"at_ms": 0, "event": "start"}, {"at_ms": 0, "event": "member_seen", "member": "axis/x"},
        {"at_ms": 100, "event": "arrival", "member": "axis/x"}, {"at_ms": 200, "event": "arrival", "member": "axis/x"},
        {"at_ms": 300, "event": "member_gone", "member": "axis/x"}, {"at_ms": 1000, "event": "end"}])
    out.append((f"{label5}: a member ended in the window and not heard again: held 200 ms though no longer held at its "
                f"end, its first end at 300; its deadline not asked (ended)",
                (t.held(), held(t), tm.ns_to_ms(t.first_end("axis/x") or 0), deadline_of(t, "axis/x")),
                ([], {"axis/x": 200}, 300, {"answer": "not_asked", "reason": "ended"})))
    # A roll-up's reason over members given out of byte order, through the
    # pairs zk2py's tool hands it: the run's first member in byte order.
    out.append(("timing.v1 0.5 §5 (F-279): a run's roll-up over two resources' members, handed in row order, reads "
                "its reason from the first in byte order",
                tm.rollup("deadline", [("zk2/lab/pace/pace.v1/stream/cmd", tm.Answer(tm.UNOBSERVABLE, "window_too_short")),
                                       ("zk2/lab/pace/pace.v1/stream/axis/x",
                                        tm.Answer(tm.UNOBSERVABLE, "owner_not_present"))]).pair(),
                {"answer": "unobservable", "reason": "owner_not_present"}))
    # review-bot on PR #941: the roll-up's two forms are explicit, and a
    # tool's window closes once. rollup takes (key, Answer) pairs alone,
    # rollup_answer answer tokens alone; each refuses the other's, and a
    # mix. A second end judges nothing and adds no report.
    def refused(f: Any, arg: Any) -> bool:
        try:
            f("deadline", arg)
        except TypeError:
            return True
        return False

    met, missed = tm.Answer(tm.YES, "met"), tm.Answer(tm.NO, "missed")
    out.append(("timing.v1 0.5 §5 (F-279): rollup refuses bare answers and a mix of pairs and answers; rollup_answer "
                "refuses pairs and Answers; each reads its own form",
                ([refused(tm.rollup, a) for a in ([met], [("k", met), missed], [("k", "yes")])],
                 [refused(tm.rollup_answer, a) for a in ([("k", met)], [met], ["yes", met])],
                 tm.rollup("deadline", [("b", met), ("a", missed)]).pair(), tm.rollup_answer("deadline", ["yes", "no"])),
                ([True, True, True], [True, True, True], {"answer": "no", "reason": "missed"}, "no")))
    t = run(tm.Tracker(500, 1, ["cmd"]), [
        {"at_ms": 0, "event": "owner", "state": "present"}, {"at_ms": 0, "event": "start"},
        {"at_ms": 1000, "event": "end"}])
    n = len(t.reports)
    again = t.end(2 * 1000 * tm.MS)
    out.append((f"{label5}: a tool's window closes once: a second end, after a miss would be due, judges nothing, "
                f"adds no report, and leaves the window's end and held time as the first end left them",
                (n, again, len(t.reports), tm.ns_to_ms(t.window_end or 0), held(t)),
                (1, [], 1, 1000, {"cmd": 1000})))
    # review-bot on PR #939: a tool's held time where it holds no tracker (a
    # resource with no deadline, which no answer reads): the end that counts
    # is the first at or after the instant the member was first held. An
    # earlier one (100 ms, the member first held at 200 ms) ends nothing;
    # one after (1.5 s) ends it there.
    ms = tm.MS
    out.append(("timing.v1 0.4 §2.7: with no tracker, a member's end before the instant it was first held in the "
                "window ends nothing; the first end at or after it does; a member never held was held 0",
                [tm.window_held(0, 3000 * ms, 200 * ms, e) for e in ([100 * ms], [100 * ms, 1500 * ms, 2500 * ms], [])]
                + [tm.window_held(0, 3000 * ms, None, [])],
                [(2800 * ms, None), (1300 * ms, 1500 * ms), (2800 * ms, None), (0, None)]))
    return out


def _admin_read_readings() -> list[tuple[str, Any, Any]]:
    """Core 0.50 §11.1 and §11.2 in :mod:`zk2py.acl` (#609, round 74): the
    admin read as the tools' three reads, the generator's MUST NOT (under
    ``deny``, on an ingress ``query``, 0.51, round 75), the block check
    (:func:`zk2py.acl.block_hazards`) and inclusion, by zenoh 1.10.1's own
    key expressions. No session is opened."""
    import zenoh

    from . import acl, live

    label = "core 0.50 §11.1, the admin read"
    out: list[tuple[str, Any, Any]] = []
    for key in acl.ADMIN_READ:
        out.append((f"{label}: {key} reaches no node's declaration records", live.admin_records_reached(key), []))
    out.append((f"{label}: 0.15's @/*/router/** reached every family of them",
                len(live.admin_records_reached("@/*/router/**")), len(live.ADMIN_RECORDS)))
    # §11.2 (0.50): admitted by inclusion, never by intersection.
    status = "@/*/router/status/plugins/storage_manager/storages/**"
    for allowed, asked, includes in (("@/*/router", "@/*/*", False), (status, "@/*/router/**", False),
                                     ("@/*/router/**", "@/*/*", False), ("@/*/*", "@/*/router", True)):
        a, q = zenoh.KeyExpr(allowed), zenoh.KeyExpr(asked)
        out.append((f"core 0.50 §11.2: {allowed} intersects {asked}, and "
                    f"{'includes' if includes else 'does not include'} it", (a.intersects(q), a.includes(q)),
                    (True, includes)))
    tool = acl.Principal("tool", "tool", inspects=["*/*"], admin_read=True)
    svc = acl.Principal("svc", "svc", service="h1/tc")
    deny = acl.generate([tool, svc], "deny")
    out.append((f"{label}: the generator grants the tool query on the three reads and their reply, never "
                f"namespaced",
                (sorted(k for k in deny.grants[("tool", "query", acl.INGRESS)] if k.startswith("@")),
                 sorted(k for k in deny.grants[("tool", "reply", acl.EGRESS)] if k.startswith("@"))),
                (sorted(acl.ADMIN_READ), sorted(acl.ADMIN_READ))))
    out.append((f"{label}: the generated deny block allows no hazardous query", acl.block_hazards(deny.block), []))
    try:
        acl.generate([tool, svc], "deny", admin_read=acl.ADMIN_READ_015)
        refused = None
    except acl.HazardousGrant as e:
        refused = "@/*/router/**" in str(e)
    out.append((f"{label}: the generator refuses 0.15's admin read (MUST NOT, 0.50)", refused, True))
    old = acl.generate([tool, svc], "deny", admin_read=acl.ADMIN_READ_015, hazard_ok=True).block
    out.append((f"{label}: the block check finds 0.15's grant, once, on the tool's ingress query",
                [(h.kind, h.policy, h.flow, h.key) for h in acl.block_hazards(old)],
                [("allowed_query", "tool-policy", acl.INGRESS, "@/*/router/**")]))
    allow = acl.generate([tool, svc], "allow").block
    out.append((f"{label}: under allow, every policy is open to a GET of the records (0.51: a router there is "
                f"not protected)", sorted(h.policy for h in acl.block_hazards(allow) if h.kind == "allow_posture"),
                ["svc-policy", "tool-policy"]))
    out.append((f"{label}: under allow, the generator compiles no allow rule, the admin read's included (0.51)",
                [r["id"] for r in allow["rules"] if r["permission"] == "allow"], []))

    def block(rules: list[dict], posture: str = "deny", listed: list[str] | None = None, **extra: Any) -> dict:
        return {"enabled": True, "default_permission": posture, "rules": rules,
                "subjects": [{"id": "s", "usernames": ["u"]}],
                "policies": [{"id": "p", "rules": [r["id"] for r in rules] if listed is None else listed,
                              "subjects": ["s"]}], **extra}

    def rule(rid: str, keys: list[str], messages: tuple = ("query",), flows: list[str] | None = None,
             permission: str = "allow") -> dict:
        r = {"id": rid, "messages": list(messages), "permission": permission, "key_exprs": keys}
        if flows is not None:
            r["flows"] = flows
        return r

    cases = [
        ("a peer's token records (any node's, F-253)", block([rule("r", ["@/*/peer/token/**"], flows=["ingress"])]),
         [("allowed_query", "ingress", "@/*/peer/token/**")]),
        ("a rule with no flows applies to both (zenoh's default), so to an ingress query",
         block([rule("r", ["@/**"])]), [("allowed_query", "ingress", "@/**")]),
        ("a rule on both flows is found once, on ingress", block([rule("r", ["@/**"], flows=["egress", "ingress"])]),
         [("allowed_query", "ingress", "@/**")]),
        ("an egress query alone is not one: the freezing GET is the asker's, on ingress (0.51, F-256)",
         block([rule("r", ["@/**"], flows=["egress"])]), []),
        ("a rule no policy lists is inert", block([rule("r", ["@/**"], flows=["ingress"])], listed=[]), []),
        ("a reply alone is not a query", block([rule("r", ["@/**"], messages=("reply",), flows=["egress"])]), []),
        ("a verbatim chunk past the records' prefix reaches nothing (§1.3)",
         block([rule("r", ["@/*/router/token/zk2/*/*/@zk/**"], flows=["ingress"])]), []),
        ("access control not enabled admits everything", {**block([]), "enabled": False}, [("disabled", None, None)]),
        ("under allow, a deny of query on @/** keeps a policy off",
         block([rule("d", ["@/**"], flows=["ingress"], permission="deny")], posture="allow"), []),
        ("under allow, a deny of query narrower than @/** does not",
         block([rule("d", ["@/*/router/**"], flows=["ingress"], permission="deny")], posture="allow"),
         [("allow_posture", None, None)]),
        ("under allow, a deny of query on @/*/router alone does not",
         block([rule("d", ["@/*/router"], flows=["ingress"], permission="deny")], posture="allow"),
         [("allow_posture", None, None)]),
        # zenoh's inclusion, not a string or glob test: `**` crosses no
        # verbatim chunk (§1.3), so it includes no `@/…` key, and `@/*/**`
        # needs a chunk after `@` that `@/**` does not
        ("under allow, a deny of query on ** does not: ** includes no @/… key",
         block([rule("d", ["**"], flows=["ingress"], permission="deny")], posture="allow"),
         [("allow_posture", None, None)]),
        ("under allow, a deny of query on @/*/** does not: it does not include @/**",
         block([rule("d", ["@/*/**"], flows=["ingress"], permission="deny")], posture="allow"),
         [("allow_posture", None, None)]),
        ("under allow, a deny of reply on @/** does not: it is no query",
         block([rule("d", ["@/**"], messages=("reply",), flows=["ingress"], permission="deny")], posture="allow"),
         [("allow_posture", None, None)]),
    ]
    for a, b, want in (("@/**", "@/*/router", True), ("**", "@/**", False), ("@/*/**", "@/**", False),
                       ("@/*/router", "@/**", False), ("@/**", "@/**", True)):
        out.append((f"{label}, the block check: zenoh's inclusion, {a} {'includes' if want else 'does not include'} "
                    f"{b}", acl._includes(a, b), want))
    for note, b, want in cases:
        got = [(h.kind, h.flow, h.key) for h in acl.block_hazards(b)]
        out.append((f"{label}, the block check: {note}", got, want))
    text = """// a block as a generator writes it, at a router configuration's top level
access_control: {
  enabled: true,
  default_permission: "deny",  // §11.2
  rules: [ { id: "a", permission: "allow", flows: ["ingress"], messages: ["query"],
             key_exprs: ["@/*/router/**"] }, ],
  subjects: [ { id: "s", usernames: ["u"] } ],
  policies: [ { id: "p", rules: ["a"], subjects: ["s"] } ],
},
"""
    read = acl.read_block_json5(text)
    out.append((f"{label}, the block check: a JSON5 block with comments, read by zenoh's own loader",
                [(h.kind, h.policy, h.rule, h.flow, h.key) for h in acl.block_hazards(read)],
                [("allowed_query", "p", "a", "ingress", "@/*/router/**")]))
    return out


def _admin_records_readings() -> list[tuple[str, Any, Any]]:
    """Core 0.48 §4.2 "What a tool does not ask", in :mod:`zk2py.live`
    (#609, round 73): which selectors reach a node's declaration records,
    its ``linkstate/*`` or its ``route/successor/**``, by zenoh 1.10.1's own
    intersection; S4's two selectors reach none and 0.11's reached them; and
    :func:`zk2py.live._answers` refuses one before it is asked, no session
    touched."""
    from . import live

    label = "core 0.48 §4.2, a selector reaching a node's declaration records"
    kinds = {p.split("/")[3]: p for p in live.ADMIN_RECORDS}
    cases = [
        (live.S4_ROUTERS, []),
        (live.S4_STORAGES, []),
        (live.S4_STORAGES_011, ["subscriber", "publisher", "queryable", "querier", "token", "route"]),
        ("@/*/router/queryable/**", ["queryable"]),
        ("@/*/router/subscriber/load/**", ["subscriber"]),
        ("@/*/router/**", list(kinds)),
        ("@/**", list(kinds)),
        ("@/*/router/linkstate/routers", ["linkstate"]),
        ("@/*/router/route/successor/src/a1/dst/b2", ["route"]),
        ("@/*/peer/token/**", ["token"]),  # Appendix B: a node's, not a router's alone
        ("@/*/router/metrics", []),
        ("@/*/*", []),  # every node's root document, as zenctl admin graph reads it
        ("@/*/router/status/**", []),
        ("@/*/router?_time=[now(-1s)..]", []),
        # §1.3: `**` crosses no verbatim chunk, so a record of a zk2 token,
        # `@/<zid>/router/token/zk2/…/@zk/…`, is reached by no `**` and is
        # asked only by a selector the admin space never hears
        ("@/*/router/token/zk2/*/*/@zk/**", []),
        ("zk2/**", []),
        ("**", []),
    ]
    out: list[tuple[str, Any, Any]] = []
    for sel, want in cases:
        got = sorted(k for k, p in kinds.items() if p in live.admin_records_reached(sel))
        out.append((f"{label}: {sel} reaches {want or 'none'}", got, sorted(want)))
    import zenoh

    class Asked:
        """A session that records each GET and parses its selector as
        ``Session.get`` does (``zenoh.Selector``, measured to raise the
        same ``ZError``), then ends it with no reply."""

        def __init__(self) -> None:
            self.asked: list[str] = []

        def get(self, selector: str, handler: Any, **_: Any) -> None:
            zenoh.Selector(selector)
            self.asked.append(selector)
            handler.drop()

    def call(sel: str, **kw: Any) -> tuple[str, list[str]]:
        """How a call of ``_answers`` ends: refused when called, a GET made
        at the first ``next()``, or zenoh's error there."""
        sess = Asked()
        try:
            gen = live._answers(sess, sel, zenoh.QueryTarget.ALL, 1.0, **kw)  # type: ignore[arg-type]
        except live.AdminRecordsRefused:
            return "refused when called", sess.asked
        if sess.asked:
            return "asked when called", sess.asked
        try:
            list(gen)
        except zenoh.ZError:
            return "ZError at the first next()", sess.asked
        return "asked at the first next()", sess.asked

    for sel in (live.S4_STORAGES_011, "@/*/router/queryable/**", "@/**"):
        out.append((f"{label}: zk2py's GET refuses {sel} when called, before anything is asked",
                    call(sel), ("refused when called", [])))
    out.append((f"{label}: {live.S4_STORAGES} is asked, at the first next(), as before the guard",
                call(live.S4_STORAGES), ("asked at the first next()", [live.S4_STORAGES])))
    out.append((f"{label}: a read the user asked for passes the guard (user_asked)",
                call("@/*/router/queryable/**", user_asked=True),
                ("asked at the first next()", ["@/*/router/queryable/**"])))
    # A malformed selector fails as it did before the guard: zenoh's own
    # error, at the GET, not a refusal, the hazardous-looking one included.
    for sel in ("a//b", "@/*/router/queryable/**/**"):
        out.append((f"{label}: malformed {sel!r} reaches nothing here, and fails at the GET with zenoh's ZError",
                    (live.admin_records_reached(sel), call(sel)), ([], ("ZError at the first next()", []))))
    return out


def _desired_encoding() -> list[tuple[str, Any, Any]]:
    """desired.v1 0.3, zk2py's author: §2.11's deterministic encoding with
    every number written by its exact value (§2.7, 0.2), and §2.4's revision
    of a document read exactly (0.3, F-181)."""
    from . import desired as ds

    label = "desired.v1 0.3 §2.11, zk2py's author writes each number by its exact value"
    out: list[tuple[str, Any, Any]] = []
    for text, want in (("1.5", "1.5"), ("0.10000000000000000555", "0.10000000000000000555"), ("10.0", "10"),
                       ("1e1", "10"), ("-0.0", "0"), ("2.5e-30", "0.0000000000000000000000000000025"),
                       ("1e-50", "1e-50"), ("18446744073709551616", "18446744073709551616"),
                       ("1e-99999999999999999999999", "1e-99999999999999999999999")):
        v = ds.strict_json(text.encode(), exact=True)
        got = ds.canonical_json(v).decode()
        out.append((f"{label}: {text} is written {want}, and reads back the same value",
                    (got, ds.jcs_equal(v, ds.strict_json(got.encode(), exact=True))), (want, True)))
    out.append((f"{label}: members sorted, no whitespace",
                ds.canonical_json(ds.strict_json(b'{ "b": [1, 2.50], "a": {"y": null, "x": true} }', exact=True)),
                b'{"a":{"x":true,"y":null},"b":[1,2.5]}'))
    try:
        ds.canonical_json(float("inf"))
        inf = "written"
    except ValueError:
        inf = "refused"
    out.append((f"{label}: a non-finite number is refused, never written", inf, "refused"))
    lab = "desired.v1 0.3 §2.4, a document's revision read by its exact value"
    for text, want in (('{"revision": 3.0}', 3), ('{"revision": 30e-1}', 3), ('{"revision": 1.5}', None),
                       ('{"revision": 18446744073709551615}', 2**64 - 1), ('{"revision": 18446744073709551616}', None),
                       ('{"revision": -1}', None), ('{"revision": true}', None), ('[3]', None), ('{"revision":', None)):
        out.append((f"{lab}: {text}", ds.revision_of(text.encode()), want))
    return out


BUDGET_FIXTURES = {"bounds.json": _budget_bounds, "population.json": _budget_population, "rate.json": _budget_rate,
                   "lossless.json": _budget_lossless}


def family_budget(root: Path) -> list[Result]:
    """``budget/`` (core §2.7 and observer §5, 0.24): every file, and an unknown one fails."""
    out: list[Result] = []
    for path in sorted((root / "budget").iterdir()):
        run = BUDGET_FIXTURES.get(path.name)
        if run is None:
            out.append((f"budget/{path.name}", False, "an unknown fixture file: the runner does not read it"))
            continue
        out += run(path)
    return out


def family_seed(root: Path) -> list[Result]:
    """``seed/`` (core §4.2, 0.27): a late joiner's merge, every file, an
    unknown one failing. ``merge.json``: two stamps -> their order, and one
    key's arrivals -> which are kept."""
    from . import seed

    out: list[Result] = []
    for path in sorted((root / "seed").iterdir()):
        if path.name != "merge.json":
            out.append((f"seed/{path.name}", False, "an unknown fixture file: the runner does not read it"))
            continue
        data = _load_json(path)

        def stamp(x: Any):
            return None if x is None else seed.Stamp(int(x["time"]), x["id"])

        for c in data["order"]:
            out.append(_check(f"seed: order: {c['note']}", seed.compare(stamp(c["a"]), stamp(c["b"])), c["expect"]))
        for c in data["cases"]:
            got = seed.kept([(stamp(a["stamp"]), a["kind"]) for a in c["arrivals"]])
            out.append(_check(f"seed: merge: {c['note']}", {"kept": got}, c["expect"]))
    return out


#: Every entry of spec/conformance/ that a family reads. Appendix E lists
#: the fixture formats; an entry no family reads fails ``layout``, so a new
#: family cannot land unrun.
KNOWN_ENTRIES = {"README.md", "keys.json", "slugs.json", "templates.json", "contracts", "sets", "bundles",
                 "history", "descriptors", "errors", "compat", "budget", "seed"}


def family_layout(root: Path) -> list[Result]:
    """Every entry of ``spec/conformance/`` is read by a family, and every
    profile with a ``conformance/`` directory has a family: a new profile's
    fixtures fail here until zk2py reads them (profiles/README.md)."""
    out = [(f"spec/conformance/{p.name}", p.name in KNOWN_ENTRIES,
            "" if p.name in KNOWN_ENTRIES else "an entry no family reads")
           for p in sorted(root.iterdir())]
    profiles = root.parent / "profiles"
    if profiles.is_dir():
        out += [(f"spec/profiles/{p.name}/conformance", p.name in PROFILE_FIXTURES,
                 "" if p.name in PROFILE_FIXTURES else "a profile's fixtures no family reads")
                for p in sorted(profiles.iterdir()) if (p / "conformance").is_dir()]
    return out


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
    "seed": family_seed,
    "examples": family_examples,
    # zk2py's own readings of open findings: not a fixture family.
    "readings": family_readings,
    # The profiles, one family each (profiles/README.md, "How the harnesses
    # take profiles in").
    **{name: _profile_family(name) for name in PROFILE_FIXTURES},
}


#: What a family reports beside its cases, printed after the totals: never a
#: failure (round 89: zk2py's walk of spec/RULES.md, its rows with no verdict).
NOTES: list[tuple[str, str]] = []


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
    for fam, note in NOTES:
        print(f"NOTE {fam}: {note}")
    for fam, case, detail in failures:
        print(f"FAIL {fam}: {case}: {detail}")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())

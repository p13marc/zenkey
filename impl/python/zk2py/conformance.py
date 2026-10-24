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
        got = ds.bind(decl, r["template"], bool(r.get("explicit")), c["iface"], c["target"], c["requirement"])
        out.append(_check(f"bindings #{i}: {c['note']}", got.expect(), c["expect"]))
    return out


def _desired_order(path: Path) -> list[Result]:
    """desired.v1 §2.6 to §2.10: one role's steps -> what each does, and the
    report after the last. Each report the machine builds must also read
    with no finding (§2.10: the findings are "each one the state machine
    never produces")."""
    from . import desired as ds

    out: list[Result] = []
    for i, c in enumerate(_load_json(path)["cases"]):
        name = f"order #{i}: {c['note']}"
        if len(c["steps"]) != len(c["expect"]["steps"]):
            out.append((name, False, "the case's steps and expected words differ in number"))
            continue
        words, report = ds.run_steps(c["key"], c["steps"])
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


def _desired_conclusions(path: Path) -> list[Result]:
    """desired.v1 §2.17, §5: one reading of one target's role -> §5's four
    questions, each an answer and a reason."""
    from . import desired as ds

    out = []
    for i, c in enumerate(_load_json(path)["cases"]):
        got = {q: a.pair() for q, a in ds.conclude(ds.reading_of(c["reading"])).items()}
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
                "conclusions.json": _desired_conclusions},
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
        got = budget.rate(c["rate"], members, seconds_to_ns(r["listened_s"]), r["lossless"], r["present"])
        out.append(_check(f"rate: {c['note']}", got.pair(), c["expect"]))
    return out


def _budget_lossless(path: Path) -> list[Result]:
    """Core §2.7 (0.25): the declared QoS, the reader's own loss and miss
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
        (f"{lab} (F-181): a revision 18446744073709551615.0 is read as its double, 2^64: revision_range",
         ds.read_report_bytes(('{"key":"%s","source":"desired","observed":{"stamp":"1/1",'
                               '"revision":18446744073709551615.0},"in_force":{"stamp":"1/1",'
                               '"revision":18446744073709551615},"outcome":"converged"}' % K).encode()).expect(),
         {"readable": False, "errors": ["revision_range"]}),
        (f"{lab} (F-181): in_force's 9007199254740993 and observed's 9007199254740993.0, its double 2^53: "
         "not observed",
         ds.read_report_bytes(('{"key":"%s","source":"desired","observed":{"stamp":"1/1",'
                               '"revision":9007199254740993.0},"in_force":{"stamp":"1/1",'
                               '"revision":9007199254740993},"outcome":"converged"}' % K).encode()).expect(),
         {"readable": True, "errors": ["in_force"]}),
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
    ]


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
        + _desired_readings() + _presence_view()
    return out + [_check(note, got, want) for note, got, want in computed]


BUDGET_FIXTURES = {"bounds.json": _budget_bounds, "population.json": _budget_population, "rate.json": _budget_rate,
                   "lossless.json": _budget_lossless}


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

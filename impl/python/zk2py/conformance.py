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
        if q == "reading":
            r = alarms.Reading(presence=c["presence"], descriptor=c.get("descriptor"), face=c.get("face"),
                               observations=[_freshness_observation(o) for o in c["observations"]],
                               latest=c["latest"], previous_instance=bool(c["previous_instance"]),
                               held_whole=bool(c["held_whole"]))
            got: Any = {"firing": alarms.judge(r, alarms.Q_FIRING).pair(),
                        "stale_firing": alarms.judge(r, alarms.Q_STALE).pair()}
        elif q == "owner_set":
            got = alarms.owner_set(c["presence"], c.get("descriptor"), c.get("face"), _alarms_seed(c["seed"]),
                                   _alarms_window(c["window"])).expect()
        elif q == "any_firing":
            owners = [alarms.OwnerRead(o["set"]["verdict"],
                                       tuple(alarms.AlarmRead(a["answer"], a["severity"]) for a in o["alarms"]))
                      for o in c["owners"]]
            got = alarms.any_firing(c["at_least"], owners)
        elif q == "rederives":
            payload = alarms.DELETE if c["payload"] == "delete" else c["payload"]
            got = alarms.rederives(c["alert_key"], payload).pair()
        else:
            out.append((name, False, f"an unknown question {q!r}"))
            continue
        out.append(_check(name, got, c["expect"]))
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

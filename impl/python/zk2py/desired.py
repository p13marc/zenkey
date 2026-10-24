"""desired.v1, text 0.1 (``spec/profiles/desired/v1.md``), with core 0.36.

A desired document is state of its **author**, keyed by its **target**: the
author writes it under its own address, and the target binds its own slice of
the author's template (core R2), converges, and says what it did in a
``desired.v1.Report`` of its own. Written from the text alone (#609, round
52):

- the declaration (§2.2): ``desired.target_param`` read into its target
  parameters, or the reasons it is invalid;
- the binding (§2.3): a requirement as a descriptor lists it, read into the
  slice's key, or its findings;
- versions (§2.4): a stamp in zenoh's form, and §2.6's order of them;
- the target (§2.6–§2.10): a pure state machine that says what to act on,
  adopts a version whose document it already acted on, holds the latest
  while an act runs, and is told how the act ended (two phases, §2.7); its
  report after each step;
- the reader of a report (§2.10): strict JSON, the schema, the stamp's form
  and the revision's range, then the eight consistency findings;
- §2.17's conclusions, §5's four questions.

Where the text left a choice, the choice is commented with its finding
(``SPEC-FINDINGS.md``, "At desired.v1 0.1").
"""

from __future__ import annotations

import json
import math
import re
from dataclasses import dataclass
from functools import lru_cache
from pathlib import Path
from typing import Any

import rfc8785

from . import alarms, jcs, keys, slug, templates
from .lexical import is_plain_chunk
from . import freshness as fr

PROFILE = "desired.v1"
#: §4: the vocabulary, which core 0.36's Appendix D publishes. Its interim
#: ``desired.target`` is retired, and W105's (contract.py).
KEY = "desired.target_param"
REPO = Path(__file__).resolve().parents[3]
SCHEMA_PATH = REPO / "spec" / "profiles" / "desired" / "desired.v1.json"

REPORT, VERSION, REFUSAL = "desired.v1.Report", "desired.v1.Version", "desired.v1.Refusal"
#: §3's table, oldest first: (artifact id, the text that published it).
PUBLISHED: tuple[tuple[str, str], ...] = (
    ("sha256:8c5ffce35fadeb6196233a46e163ee3b15cbd2bd8339a05de2cf8fe22b52e9ef", "0.1"),
)
PUBLISHED_IDS = tuple(i for i, _ in PUBLISHED)
CURRENT = PUBLISHED[-1][0]
#: §3: the three types, each with the first revision that defines it.
TYPES = {REPORT: "0.1", VERSION: "0.1", REFUSAL: "0.1"}

U64_MAX = 2**64 - 1
#: §2.10: "its id at most 128 bits", 32 hexadecimal digits.
ID_DIGITS = 32

SELF_SYSTEM, SELF_SERVICE = "self.system", "self.service"
EITHER = "either"


# -- §3: the standard schema ------------------------------------------------------------------

def artifact_id(document: Any) -> str:
    """Core §7.1, §9.4: the hash of the document's JCS bytes."""
    return jcs.jcs_id(document)


def is_published(artifact: str | None) -> bool:
    """§3: "Its types are the profile's in any artifact whose id is one of
    these, and in no other, whatever its types are named"."""
    return artifact in PUBLISHED_IDS


def is_profile_type(t: Any, name: str = REPORT) -> bool:
    """§2.10 "A tool finds a target's reports by type identity, never by
    name": a ``jsonschema`` type of that name from a published artifact
    whose revision defines it."""
    if not isinstance(t, dict) or t.get("kind") != "jsonschema" or t.get("name") != name:
        return False
    sid = t.get("schema")
    if sid not in PUBLISHED_IDS or name not in TYPES:
        return False
    since = [r for _, r in PUBLISHED].index(TYPES[name])
    return PUBLISHED_IDS.index(sid) >= since


@lru_cache(maxsize=1)
def load_schema() -> dict[str, Any]:
    """The profile's file, the latest revision (§3), as published."""
    return jcs.loads(SCHEMA_PATH.read_bytes())


# -- §2.2: the declaration -----------------------------------------------------------------------

DECLARED, INVALID, NOT_THIS = "declared", "invalid", "not_this_profiles"
#: §2.2's reasons, "checked in this order, each alone, a check that needs a
#: value an earlier one found wrong skipped".
DECLARATION_REASONS = ("not_state", "value", "repeated", "not_a_param", "rest_param", "param_not_target")


@dataclass(frozen=True)
class Target:
    """One target parameter, and the identity its binding gives it (§2.2,
    "Which binds to which"): ``either`` for a lone one, else
    ``self.system`` then ``self.service``."""

    param: str
    binds: str


@dataclass(frozen=True)
class Declaration:
    declaration: str
    reasons: tuple[str, ...] = ()
    targets: tuple[Target, ...] = ()
    #: freshness.v1's reading of the resource's horizon (§2.2 "Its horizon"),
    #: informative: a horizon, whatever its value, makes no declaration
    #: invalid.
    horizon: fr.Horizon | None = None

    def expect(self) -> dict[str, Any]:
        """declarations.json's ``expect``."""
        if self.declaration == NOT_THIS:
            return {"declaration": NOT_THIS}
        if self.declaration == INVALID:
            return {"declaration": INVALID, "reasons": list(self.reasons)}
        return {"declaration": DECLARED, "targets": [{"param": t.param, "binds": t.binds} for t in self.targets]}


def _is_state(kind: str) -> bool:
    """§2.2 ``not_state``: "the resource is not ``state`` or ``@state``". An
    authoring kind, or a kind token."""
    return kind in ("state", "@state")


def _names(value: Any) -> list[str] | None:
    """§2.2 "Its value is a string, or an array of one or two strings … A
    string is the same as a one-element array." None for anything else
    (``value``)."""
    if isinstance(value, str):
        return [value]
    if isinstance(value, list) and 1 <= len(value) <= 2 and all(isinstance(x, str) for x in value):
        return list(value)
    return None


def declare(kind: str, template: str, annotations: dict[str, Any]) -> Declaration:
    """§2.2 over a resolved resource: its authoring kind (or kind token), its
    template, and its resolved annotations (core §9.3). "A resource without
    the key is not this profile's, whatever else it declares"."""
    if KEY not in annotations:
        return Declaration(NOT_THIS)
    horizon = fr.horizon("state", annotations) if fr.KEY in annotations else None
    t = templates.parse_template(template)
    single = {s.text for s in t.segments if s.kind == templates.PARAM}
    rest = {s.text for s in t.segments if s.kind == templates.REST}
    reasons: list[str] = []
    if not _is_state(kind):
        reasons.append("not_state")
    names = _names(annotations[KEY])
    if names is None:
        reasons.append("value")
    else:
        if len(set(names)) != len(names):
            reasons.append("repeated")
        if any(n not in single and n not in rest for n in names):
            reasons.append("not_a_param")
        if any(n in rest for n in names):
            reasons.append("rest_param")
        if any(p not in names for p in t.param_names):
            reasons.append("param_not_target")
    if reasons:
        return Declaration(INVALID, tuple(reasons), horizon=horizon)
    assert names is not None
    if len(names) == 1:
        targets = (Target(names[0], EITHER),)
    else:
        targets = (Target(names[0], SELF_SYSTEM), Target(names[1], SELF_SERVICE))
    return Declaration(DECLARED, (), targets, horizon)


# -- §2.3: the binding ---------------------------------------------------------------------------

BOUND, UNBOUND = "bound", "unbound"
#: §2.3's findings, in their order.
BINDING_REASONS = ("required_unbound", "not_one_author", "target_unbound", "not_self", "wrong_self")
#: zk2py's own refusal, outside §2.3's list: a target address that is not
#: ``<system>/<service>``.
TARGET_ADDRESS = "target_address"


@dataclass(frozen=True)
class Binding:
    binding: str
    reasons: tuple[str, ...] = ()
    key: str | None = None

    def expect(self) -> dict[str, Any]:
        """bindings.json's ``expect``."""
        if self.binding == BOUND:
            return {"binding": BOUND, "key": self.key}
        if self.binding == UNBOUND:
            return {"binding": UNBOUND}
        return {"binding": INVALID, "reasons": list(self.reasons)}


def slice_key(author: str, iface: str, explicit: bool, template: str, identities: dict[str, str]) -> str:
    """§2.3 "The slice's key is the author's address, the interface, the
    resource's kind token and its template, each target parameter filled
    with the target's own system or service as its binding says, slugged as
    any parameter value is (core §1.4)"."""
    t = templates.parse_template(template)
    chunks = [s.text if s.kind == templates.LITERAL else slug.slug(identities[s.text]) for s in t.segments]
    return "/".join([keys.GRAMMAR, author, iface, "@state" if explicit else "state", *chunks])


def bind(decl: Declaration, template: str, explicit: bool, iface: str, target: str,
         requirement: dict[str, Any]) -> Binding:
    """§2.3 over a declared resource and a requirement as the target's
    descriptor lists it (core §3.3: ``bindings``, the provider addresses;
    ``params``, the parameter bindings as configured; ``optional``)."""
    if decl.declaration != DECLARED:
        raise ValueError("§2.3 binds a declared resource only")
    # The target's own address is the premise of every check: one that is
    # not <system>/<service>, each a plain chunk (core §1.1, §1.5), fills no
    # slice. zk2py's own refusal, checked alone, outside §2.3's list.
    tparts = target.split("/") if isinstance(target, str) else []
    if len(tparts) != 2 or not all(is_plain_chunk(x) for x in tparts):
        return Binding(INVALID, (TARGET_ADDRESS,))
    providers = list(requirement.get("bindings") or [])
    params = dict(requirement.get("params") or {})
    if not providers:
        # §2.3: required_unbound "checked alone"; "Unbound ("bindings": [])
        # and optional is the kill switch (§2.12), not a finding."
        if requirement.get("optional"):
            return Binding(UNBOUND)
        return Binding(INVALID, ("required_unbound",))
    reasons: list[str] = []
    parts = providers[0].split("/")
    # "more than one binding, or one with * at either position". A provider
    # that is not <system>/<service> is not "one exact address" either: the
    # descriptor's own D codes would refuse it first.
    if len(providers) != 1 or len(parts) != 2 or "*" in parts:
        reasons.append("not_one_author")
    if any(t.param not in params for t in decl.targets):
        reasons.append("target_unbound")
    if any(t.param in params and params[t.param] not in (SELF_SYSTEM, SELF_SERVICE) for t in decl.targets):
        reasons.append("not_self")
    # wrong_self: "of two target parameters, the first not bound to
    # self.system, or the second not to self.service". zk2py reads it as the
    # fixture's description does, "one bound to a self other than its own":
    # a literal or an unbound one is not_self's or target_unbound's alone
    # (F-161).
    if len(decl.targets) == 2 and any(params.get(t.param) in (SELF_SYSTEM, SELF_SERVICE)
                                      and params[t.param] != t.binds for t in decl.targets):
        reasons.append("wrong_self")
    if reasons:
        return Binding(INVALID, tuple(reasons))
    system, service = tparts
    identities = {t.param: system if params[t.param] == SELF_SYSTEM else service for t in decl.targets}
    return Binding(BOUND, (), slice_key(providers[0], iface, explicit, template, identities))


# -- §2.4: versions ------------------------------------------------------------------------------

#: §2.4: "its NTP64 time in decimal, without leading zeros, then /, then its
#: id in lowercase hexadecimal without leading zeros". The time 0 is "0";
#: the id 0 is refused, as reports.json does (F-164).
_STAMP = re.compile(r"(0|[1-9][0-9]*)/([1-9a-f][0-9a-f]*)")


@dataclass(frozen=True)
class Stamp:
    """A state mutation's stamp: its time, an NTP64 value, and its id, the
    issuing session's zid, both compared by value (§2.6)."""

    time: int
    id: int

    @property
    def text(self) -> str:
        """§2.4: "as zenoh writes a timestamp"."""
        return f"{self.time}/{self.id:x}"

    @classmethod
    def parse(cls, text: Any) -> Stamp | None:
        """§2.10: "a stamp in the form of §2.4, its time at most 2^64−1 and
        its id at most 128 bits", or None."""
        if not isinstance(text, str):
            return None
        m = _STAMP.fullmatch(text)
        if m is None or len(m.group(2)) > ID_DIGITS:
            return None
        t = int(m.group(1))
        return cls(t, int(m.group(2), 16)) if t <= U64_MAX else None

    @classmethod
    def of(cls, x: dict[str, Any]) -> Stamp:
        """A stamp as order.json and conclusions.json write one, ``{time,
        id}``, the id lowercase hexadecimal (core's seed/merge.json)."""
        return cls(int(x["time"]), int(x["id"], 16))


# -- §2.7: equal documents -----------------------------------------------------------------------

@dataclass(frozen=True)
class Payload:
    """A put's payload as §2.7 compares it: ``value`` for a payload encoded
    as JSON that reads as strict JSON, else its ``data``, byte for byte."""

    value: Any = None
    data: bytes | None = None

    @classmethod
    def of_bytes(cls, data: bytes, json_encoded: bool) -> Payload:
        """§2.7: "Equal is JCS-equal (RFC 8785) for a payload encoded as JSON
        (a JSON Schema type whose encoding is json, core §7.2): the two read
        as strict JSON … Any other payload, CBOR, protobuf or raw, and JSON
        that does not read, is equal byte for byte"."""
        if json_encoded:
            try:
                return cls(value=strict_json(data))
            except NotStrict:
                pass
        return cls(data=bytes(data))


def _in_strict_domain(v: Any) -> bool:
    """Whether ``v`` is a value strict JSON reading can yield (§2.10, as
    :func:`strict_json` reads): null, a boolean, an integer, a finite float,
    a string with a UTF-8 form, and arrays and objects of them, every key a
    string with a UTF-8 form."""
    if v is None or isinstance(v, (bool, int)):
        return True
    if isinstance(v, float):
        return math.isfinite(v)
    if isinstance(v, str):
        return _utf8(v)
    if isinstance(v, list):
        return all(_in_strict_domain(x) for x in v)
    if isinstance(v, dict):
        return all(isinstance(k, str) and _utf8(k) and _in_strict_domain(x) for k, x in v.items())
    return False


def _exactly_equal(a: Any, b: Any) -> bool:
    """JSON values equal by value, a number's exactly (``10`` is ``10.0``),
    a boolean never a number."""
    num = (int, float)
    if isinstance(a, num) and isinstance(b, num) and not isinstance(a, bool) and not isinstance(b, bool):
        return a == b
    if type(a) is not type(b):
        return False
    if isinstance(a, dict):
        return a.keys() == b.keys() and all(_exactly_equal(a[k], b[k]) for k in a)
    if isinstance(a, list):
        return len(a) == len(b) and all(_exactly_equal(x, y) for x, y in zip(a, b))
    return bool(a == b)


def jcs_equal(a: Any, b: Any) -> bool:
    """§2.7's JCS-equal. RFC 8785 is defined over I-JSON, whose numbers are
    doubles: where both documents are in its domain, their JCS bytes are
    compared. A document with an integer beyond ±(2^53−1), which RFC 8785
    does not canonicalize (and a double could not tell from its neighbour),
    is compared by exact value instead, so that two documents whose
    revisions differ above 2^53 are two documents, never one (F-166).

    That fallback is for ``IntegerDomainError`` alone, and only over two
    values strict JSON reading can yield. Any other value RFC 8785 refuses
    (a non-finite float, a string with no UTF-8 form, a key that is not a
    string, anything that is not JSON) is no document read as strict JSON,
    so it equals nothing, itself included, and costs an act. Both ways keep
    §2.7's "conservative" direction: an act may be spent, never skipped."""
    try:
        return rfc8785.dumps(a) == rfc8785.dumps(b)
    except rfc8785.IntegerDomainError:
        return _in_strict_domain(a) and _in_strict_domain(b) and _exactly_equal(a, b)
    except rfc8785.CanonicalizationError:
        return False


def same_document(a: Payload | None, b: Payload | None) -> bool:
    """§2.7: "a delete is equal to a delete" (None is a delete's payload);
    a put is never equal to a delete; JSON by JCS, anything else byte for
    byte. A payload that reads and one that does not are never equal."""
    if a is None or b is None:
        return a is None and b is None
    if a.data is not None or b.data is not None:
        return a.data is not None and a.data == b.data
    return jcs_equal(a.value, b.value)


# -- §2.6–§2.10: the target ----------------------------------------------------------------------

OWNER, ARCHIVE = "owner", "archive"
#: The words a step answers with (order.json's ``expect.steps``).
UNSTAMPED, SAME_VERSION, CONFIRMED, OLDER, HELD = "unstamped", "same_version", "confirmed", "older", "held"
ADOPTED, ACT, REVERT, DONE, CALLED, RESTARTED = "adopted", "act", "revert", "done", "called", "restarted"
#: §2.10's sources and outcomes.
BASELINE, DESIRED, CALL = "baseline", "desired", "call"
CONVERGED, REFUSED, FAILED, REVERTED = "converged", "refused", "failed", "reverted"
#: §2.7, §2.10: the profile's own refusal codes, and its failure's.
REFUSAL_CODES = ("undecodable", "reach", "sensitive", "invalid")
FAILURE_CODE = "apply"


@dataclass(frozen=True)
class Sample:
    """One sample or reply of the slice, from either path (§2.5): from the
    author (``owner``) or an archive's last-known (``archive``, §2.9); a put
    with its payload, or a delete (a delete or a ``reply_del``); its stamp,
    or None for an unstamped sample; the document type's own revision when
    it carries one (§2.4)."""

    origin: str
    delete: bool
    stamp: Stamp | None
    payload: Payload | None = None
    revision: int | None = None


@dataclass
class _Version:
    stamp: Stamp
    delete: bool
    payload: Payload | None
    revision: int | None
    #: Read from an archive and not confirmed by the author since (§2.9).
    last_known: bool

    def written(self) -> dict[str, Any]:
        """§2.10's ``observed``/``in_force``: the stamp, and the revision
        for a put whose type carries one. "A delete has no document, so its
        version has a stamp and no revision" (§2.4)."""
        out: dict[str, Any] = {"stamp": self.stamp.text}
        if self.revision is not None and not self.delete:
            out["revision"] = self.revision
        return out


@dataclass
class Finish:
    """How the running act ended (§2.7 "Two phases"): ``done`` (a put
    converged, a delete reverted), ``refused`` or ``failed``, with the
    refusal (``{code, pointer?, detail?}``)."""

    how: str
    refusal: dict[str, Any] | None = None


class TargetRole:
    """One target's role on a desired resource, as §2.6 to §2.10 state it:
    a pure state machine. It holds the version it last acted on or adopted
    (the running act's while one runs, §2.7: "records it as acted on when
    the act starts"), and while an act runs, the latest version it admits,
    waiting. It never decodes a document: the act is the target's, and is
    told how it ended (:meth:`finish`). ``key`` is the slice's (§2.3), None
    for a role left unbound (§2.12)."""

    def __init__(self, key: str | None) -> None:
        self.key = key
        self._fresh()

    def _fresh(self) -> None:
        self.current: _Version | None = None
        self.running = False
        self.waiting: _Version | None = None
        self.source = BASELINE
        self.in_force: dict[str, Any] | None = None
        self.observed: dict[str, Any] | None = None
        self.outcome: str | None = None
        self.refusal: dict[str, Any] | None = None
        self.last_known = False
        #: How many times the report moved (§2.10 "and after each change").
        self.report_puts = 1

    # The order of versions (§2.6), one held version against another.
    @staticmethod
    def _judge(v: _Version, held: _Version) -> str | None:
        """§2.6 steps 3 and 4 against one held version: the same version,
        older, or None for admitted (a later time of its id, or another
        id). Only the held version is compared, as step 4 and order.json
        say, so an old id's version older than one acted on before a new
        id is admitted, which "Never an older version" forbids (F-163)."""
        if v.stamp.id == held.stamp.id:
            if v.stamp.time == held.stamp.time:
                return SAME_VERSION
            if v.stamp.time < held.stamp.time:
                return OLDER
        return None

    def _same(self, v: _Version, held: _Version, origin: str) -> str:
        """§2.6 step 3 "equal": the same version, or, the author's own sample
        of a version held from an archive, confirmed (§2.9)."""
        if origin == OWNER and held.last_known:
            held.last_known = False
            if held is self.current and not self.running:
                self.last_known = False
                self.report_puts += 1
            return CONFIRMED
        return SAME_VERSION

    def receive(self, s: Sample) -> str:
        """§2.6: judge one sample or reply, in the order received. While an
        act runs (§2.7, "One act at a time"), "judged against the running
        act's and then against the held one", an admitted version held
        waiting, the later replacing the earlier (F-162)."""
        if s.stamp is None:
            return UNSTAMPED  # step 1: nothing it holds changes
        v = _Version(s.stamp, s.delete, None if s.delete else s.payload, None if s.delete else s.revision,
                     s.origin == ARCHIVE)
        for held in (self.current, self.waiting if self.running else None):
            if held is None:
                continue
            word = self._judge(v, held)
            if word == SAME_VERSION:
                return self._same(v, held, s.origin)
            if word == OLDER:
                return OLDER
        if self.running:
            self.waiting = v
            return HELD
        return self._start(v)

    def _start(self, v: _Version) -> str:
        """An admitted version, nothing running: adopted when its document
        equals that of the last version acted on or adopted in this
        instance (§2.6 step 5, §2.7), else acted on."""
        prev = self.current
        if prev is not None and same_document(prev.payload, v.payload):
            # §2.7 "The report moves": observed to the adopted version, and
            # in_force with it "when it named the previous one"; outcome,
            # refusal and source stay; last_known says where it came from.
            if self.in_force is not None and self.in_force == prev.written():
                self.in_force = v.written()
            self.current = v
            self.observed = v.written()
            self.last_known = v.last_known
            self.report_puts += 1
            return ADOPTED
        self.current = v
        self.running = True
        return REVERT if v.delete else ACT

    def finish(self, f: Finish) -> str:
        """§2.7, §2.8, §2.10: the running act ended. The report moves, then
        the version held waiting, if any, is acted on or adopted."""
        if not self.running or self.current is None:
            raise ValueError("no act runs")
        v = self.current
        self.running = False
        how = f.how
        if how == REFUSED and v.delete:
            how = FAILED  # §2.8: "A revert is never refused"; read as a failure
        if how == DONE:
            self.outcome, self.refusal = (REVERTED if v.delete else CONVERGED), None
            self.source = BASELINE if v.delete else DESIRED
            self.in_force = None if v.delete else v.written()
        elif how == REFUSED:
            self.outcome, self.refusal = REFUSED, dict(f.refusal or {})
        elif how == FAILED:
            self.outcome, self.refusal = FAILED, dict(f.refusal or {})
            self.source = BASELINE if v.delete else DESIRED
            self.in_force = None
        else:
            raise ValueError(f"an unknown end {f.how!r}")
        self.observed = v.written()
        self.last_known = v.last_known
        self.report_puts += 1
        if self.waiting is not None:
            w, self.waiting = self.waiting, None
            return self._start(w)
        return DONE

    def call(self) -> str:
        """§2.7 "Not a lock": the target's own operation changed what is in
        force. ``source`` ``call``, no ``in_force``; the last act's
        ``observed`` and ``outcome`` stay."""
        self.source, self.in_force = CALL, None
        self.report_puts += 1
        return CALLED

    def restart(self) -> str:
        """§2.7 "A restart acts again": a new instance holds nothing."""
        self._fresh()
        return RESTARTED

    def report(self) -> dict[str, Any]:
        """§2.10: the Report after the last step."""
        out: dict[str, Any] = {}
        if self.key is not None:
            out["key"] = self.key
        out["source"] = self.source
        if self.in_force is not None:
            out["in_force"] = dict(self.in_force)
        if self.observed is not None:
            out["observed"] = dict(self.observed)
            out["outcome"] = self.outcome
        if self.refusal is not None:
            out["refusal"] = dict(self.refusal)
        if self.last_known:
            out["last_known"] = True
        return out


# -- §2.10: reading a report ---------------------------------------------------------------------

#: §2.10 "How a reader reads one": unreadable, the first only, in order.
UNREADABLE = ("json", "schema", "stamp", "revision_range")
#: §2.10's eight consistency findings, in order.
FINDINGS = ("keyless", "key_form", "observed_outcome", "refusal", "source", "in_force", "revision_on_revert",
            "last_known")


class NotStrict(ValueError):
    """Bytes that are not strict JSON (§2.10, as alarms.v1 §2.4 reads)."""


def _utf8(v: Any) -> bool:
    if isinstance(v, str):
        try:
            v.encode("utf-8")
        except UnicodeEncodeError:
            return False
        return True
    if isinstance(v, dict):
        return all(_utf8(k) and _utf8(x) for k, x in v.items())
    if isinstance(v, list):
        return all(_utf8(x) for x in v)
    return True


def _finite(v: Any) -> bool:
    """"every number finite". An integer's text is read exactly, and is
    finite when its value, rounded to the nearest double, is not an infinity:
    telemetry.v1 §2.6's reading (0.3, F-150), which neither alarms.v1 §2.4
    nor this text spells out (F-167)."""
    if isinstance(v, bool):
        return True
    if isinstance(v, float):
        return math.isfinite(v)
    if isinstance(v, int):
        try:
            float(v)
        except OverflowError:
            return False
        return True
    if isinstance(v, dict):
        return all(_finite(x) for x in v.values())
    if isinstance(v, list):
        return all(_finite(x) for x in v)
    return True


def _no_duplicates(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    out: dict[str, Any] = {}
    for k, v in pairs:
        if k in out:
            raise NotStrict(f"duplicate member {k!r}")
        out[k] = v
    return out


def _no_constant(name: str) -> Any:
    raise NotStrict(f"{name} is not JSON")


def strict_json(payload: bytes) -> Any:
    """§2.10, "As alarms.v1 §2.4 reads a document: strict JSON first (UTF-8,
    RFC 8259, nothing after the value but whitespace, no duplicate member at
    any depth, every number finite, every string with a UTF-8 form)". The
    whitespace is RFC 8259's ``ws`` (alarms.v1 0.3). A value nested past
    Python's recursion limit is not read, never a crash. Raises
    :class:`NotStrict`."""
    try:
        text = payload.decode("utf-8")
    except UnicodeDecodeError as e:
        raise NotStrict(f"not UTF-8: {e}") from e
    try:
        value = json.loads(text.strip(" \t\n\r"), object_pairs_hook=_no_duplicates, parse_constant=_no_constant)
    except NotStrict:
        raise
    except RecursionError as e:
        raise NotStrict("nested past this reader's depth") from e
    except ValueError as e:
        raise NotStrict(str(e)) from e
    if not _utf8(value):
        raise NotStrict("a string with no UTF-8 form")
    if not _finite(value):
        raise NotStrict("a number past the double's finite range")
    return value


def _integral(v: Any) -> bool:
    return isinstance(v, (int, float)) and not isinstance(v, bool) and (isinstance(v, int) or v.is_integer())


def _version_of(x: dict[str, Any]) -> dict[str, Any]:
    """A Version read: "an integral number read as an integer (3.0 is 3)", a
    member the schema does not name dropped."""
    out: dict[str, Any] = {"stamp": x["stamp"]}
    if "revision" in x:
        out["revision"] = int(x["revision"])
    return out


@dataclass(frozen=True)
class ReportReading:
    """A report as a reader reads it (§2.10): whether it reads, its findings
    in order (one, the first, when it does not read), and the report read,
    its integral revisions as integers and its unnamed members dropped."""

    readable: bool
    errors: tuple[str, ...]
    report: dict[str, Any] | None = None

    @property
    def valid(self) -> bool:
        """"A report that fails one of these is unreadable, and one that reads
        with a finding above is malformed: a tool answers nothing from
        either (§2.17)"."""
        return self.readable and not self.errors

    def expect(self) -> dict[str, Any]:
        """reports.json's ``expect``."""
        return {"readable": self.readable, "errors": list(self.errors)}


def read_report_bytes(payload: bytes) -> ReportReading:
    """§2.10 over a report's bytes: strict JSON first."""
    try:
        value = strict_json(payload)
    except NotStrict:
        return ReportReading(False, ("json",))
    return read_report(value)


def read_report(value: Any) -> ReportReading:
    """§2.10 over a decoded report: the schema, the stamps' form, the
    revisions' range, then the consistency findings."""
    if not alarms.validate(value, {"$ref": f"#/$defs/{REPORT}"}, load_schema()):
        return ReportReading(False, ("schema",))
    versions = [value[m] for m in ("in_force", "observed") if m in value]
    if any(Stamp.parse(x["stamp"]) is None for x in versions):
        return ReportReading(False, ("stamp",))
    if any("revision" in x and int(x["revision"]) > U64_MAX for x in versions):
        return ReportReading(False, ("revision_range",))
    r: dict[str, Any] = {"source": value["source"]}
    for m in ("key", "outcome", "last_known"):
        if m in value:
            r[m] = value[m]
    for m in ("in_force", "observed"):
        if m in value:
            r[m] = _version_of(value[m])
    if "refusal" in value:
        r["refusal"] = {k: value["refusal"][k] for k in ("code", "pointer", "detail") if k in value["refusal"]}
    return ReportReading(True, tuple(consistency(r)), r)


def _is_slice_key(key: str) -> bool:
    """§2.10 ``key_form``: "a concrete zk2 data key of a state or @state
    resource (core §1.1)"."""
    p = keys.parse(key)
    return p is not None and p["form"] == "data" and p["token"] in ("state", "@state")


def consistency(r: dict[str, Any]) -> list[str]:
    """§2.10 "A target's report MUST be consistent": every finding that
    applies, in order."""
    out: list[str] = []
    source, outcome = r["source"], r.get("outcome")
    observed, in_force = r.get("observed"), r.get("in_force")
    if "key" not in r and (source != BASELINE or observed is not None):
        out.append("keyless")
    if "key" in r and not _is_slice_key(r["key"]):
        out.append("key_form")
    if (observed is None) != (outcome is None):
        out.append("observed_outcome")
    if ("refusal" in r and outcome not in (REFUSED, FAILED)) or (outcome in (REFUSED, FAILED) and "refusal" not in r):
        out.append("refusal")
    if (source == DESIRED and observed is None) or (outcome == CONVERGED and source == BASELINE) \
            or (outcome == REVERTED and source == DESIRED):
        out.append("source")
    # in_force: "or, with converged and desired, an in_force that is not
    # observed": an absent in_force is not observed either (reports.json
    # #62, F-165).
    if (in_force is not None and (source != DESIRED or outcome == FAILED)) \
            or (outcome == CONVERGED and source == DESIRED and in_force != observed):
        out.append("in_force")
    if outcome == REVERTED and observed is not None and "revision" in observed:
        out.append("revision_on_revert")
    # last_known: "last_known with no observed", read as the fixture's
    # "last_known true": false written out is absent.
    if r.get("last_known") is True and observed is None:
        out.append("last_known")
    return out


# -- §2.17, §5: what a tool concludes of one target ----------------------------------------------

YES, NO, NOT_YET, UNOBSERVABLE, NOT_ASKED = "yes", "no", "not_yet", "unobservable", "not_asked"
#: §5's four questions.
Q_CONVERGED, Q_REFUSED, Q_LAST_KNOWN, Q_IN_FORCE = "converged", "refused", "last_known", "in_force"
QUESTIONS = (Q_CONVERGED, Q_REFUSED, Q_LAST_KNOWN, Q_IN_FORCE)
PRESENT, ABSENT, INCOMPLETE, ACROSS_FACE = "present", "absent", "incomplete", "across_face"


@dataclass(frozen=True)
class Answer:
    answer: str
    reason: str

    def pair(self) -> dict[str, str]:
        return {"answer": self.answer, "reason": self.reason}


@dataclass(frozen=True)
class Reading:
    """One reading of one target's role against its author's document
    (conclusions.json's ``reading``):
    - ``target_presence``: present, absent, incomplete or across_face;
    - ``descriptor``: None when not read, else whether its ``requires`` list
      the role and whether its bindings are not empty;
    - ``report``: None when none was read, else its reading (§2.10);
    - ``report_freshness``: freshness.v1's verdict on it (fresh, stale,
      unobservable), ``not_asked`` with no horizon declared;
    - ``author_presence``, as the target's;
    - ``author_current``: None for no GET made, ``incomplete``, ``silent``,
      or ``(kind, Stamp)`` its latest put or delete by S4's GET."""

    target_presence: str
    descriptor: tuple[bool, bool] | None
    report: ReportReading | None
    report_freshness: str
    author_presence: str
    author_current: Any


def _all(answer: str, reason: str) -> dict[str, Answer]:
    return {q: Answer(answer, reason) for q in QUESTIONS}


def conclude(r: Reading) -> dict[str, Answer]:
    """§2.17: "by the first of these that answers". Steps 1 to 4 answer
    every question alike, but ``in_force`` for an unbound role; then step 5,
    and ``converged`` by steps 6 and 7."""
    # 1. The target's presence.
    if r.target_presence == ABSENT:
        return _all(NOT_ASKED, "target_absent")
    if r.target_presence == INCOMPLETE:
        return _all(UNOBSERVABLE, "presence_incomplete")
    across = r.target_presence == ACROSS_FACE
    # 2. Its descriptor; across a face, "the deployment's word stands for the
    # descriptor, and step 3": step 2 is not read there (F-168).
    if not across:
        if r.descriptor is None:
            return _all(UNOBSERVABLE, "no_descriptor")
        lists, bound = r.descriptor
        if not lists:
            return _all(NOT_ASKED, "not_a_target")
        if not bound:
            out = _all(NOT_ASKED, "unbound")
            out[Q_IN_FORCE] = Answer(NO, "unbound")
            return out
    # 3. Its report of the role's slice.
    if r.report is None:
        return _all(UNOBSERVABLE, "nothing_crossed" if across else "no_report")
    if not r.report.valid:
        return _all(UNOBSERVABLE, "report_invalid")
    # 4. Its age.
    if r.report_freshness == "stale":
        return _all(UNOBSERVABLE, "report_stale")
    if r.report_freshness == "unobservable":
        return _all(UNOBSERVABLE, "report_unobservable")
    rep = r.report.report or {}
    observed, outcome = rep.get("observed"), rep.get("outcome")
    out: dict[str, Answer] = {}
    # 5. The questions.
    if observed is None:
        out[Q_REFUSED] = Answer(NOT_ASKED, "not_acted")
        out[Q_LAST_KNOWN] = Answer(NOT_ASKED, "not_acted")
    else:
        out[Q_REFUSED] = Answer(YES if outcome in (REFUSED, FAILED) else NO, outcome)
        out[Q_LAST_KNOWN] = Answer(YES, "last_known") if rep.get("last_known") else Answer(NO, "current")
    source = rep["source"]
    if source == DESIRED:
        out[Q_IN_FORCE] = Answer(YES, "desired" if "in_force" in rep else "partial")
    else:
        out[Q_IN_FORCE] = Answer(NO, source)
    out[Q_CONVERGED] = _converged(r, observed, outcome)
    return {q: out[q] for q in QUESTIONS}


def _converged(r: Reading, observed: dict[str, Any] | None, outcome: str | None) -> Answer:
    """§2.17 steps 6 and 7."""
    # 6. The author.
    if r.author_presence == ABSENT:
        return Answer(NOT_ASKED, "author_absent")
    if r.author_presence == INCOMPLETE:
        return Answer(UNOBSERVABLE, "author_presence_incomplete")
    cur = r.author_current
    if cur is None:
        return Answer(UNOBSERVABLE, "author_not_read")
    if cur == "incomplete":
        return Answer(UNOBSERVABLE, "author_read_incomplete")
    if cur == "silent":
        return Answer(UNOBSERVABLE, "author_silent")
    _, author = cur
    # 7. The comparison, "observed compared with the author's stamp by value".
    if observed is None:
        return Answer(NOT_YET, "not_acted")
    mine = Stamp.parse(observed["stamp"])
    assert mine is not None  # a valid report's stamps read
    if mine.id != author.id:
        return Answer(NOT_YET, "other_epoch")
    if mine.time == author.time:
        return Answer(YES if outcome in (CONVERGED, REVERTED) else NO, outcome or "")
    if mine.time < author.time:
        return Answer(NOT_YET, "behind")
    return Answer(UNOBSERVABLE, "ahead")


def reading_of(x: dict[str, Any]) -> Reading:
    """conclusions.json's ``reading``, read."""
    t, a = x["target"], x["author"]
    d = t["descriptor"]
    rep = t["report"]
    if rep is None:
        report = None
    elif rep == "undecodable":
        report = ReportReading(False, ("json",))
    else:
        report = read_report(rep["document"])
    cur = a["current"]
    if isinstance(cur, dict):
        (kind, st), = cur.items()
        cur = (kind, Stamp.of(st))
    return Reading(t["presence"], None if d is None else (bool(d["lists"]), bool(d["bound"])), report,
                   t["report_freshness"], a["presence"], cur)


# -- order.json's steps --------------------------------------------------------------------------

def sample_of(x: dict[str, Any]) -> Sample:
    """order.json's ``sample``: a put's ``document`` (a payload encoded as
    JSON, decoded) or ``bytes`` (hex, compared byte for byte)."""
    stamp = None if x["stamp"] is None else Stamp.of(x["stamp"])
    if x["kind"] == "delete":
        return Sample(x["from"], True, stamp)
    if "document" in x:
        payload = Payload(value=x["document"])
    else:
        payload = Payload(data=bytes.fromhex(x["bytes"]))
    rev = x.get("revision")
    return Sample(x["from"], False, stamp, payload, None if rev is None else int(rev))


def run_steps(key: str | None, steps: list[dict[str, Any]]) -> tuple[list[str], dict[str, Any]]:
    """order.json: one role's steps -> each step's word, and the report after
    the last."""
    t = TargetRole(key)
    words: list[str] = []
    for s in steps:
        if "sample" in s:
            words.append(t.receive(sample_of(s["sample"])))
        elif "finish" in s:
            f = s["finish"]
            if f == "done":
                words.append(t.finish(Finish(DONE)))
            else:
                (how, refusal), = f.items()
                words.append(t.finish(Finish(how, refusal)))
        elif s.get("call"):
            words.append(t.call())
        elif s.get("restart"):
            words.append(t.restart())
        else:
            raise ValueError(f"an unknown step {s!r}")
    return words, t.report()


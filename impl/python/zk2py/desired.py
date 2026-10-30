"""desired.v1, text 0.2 (``spec/profiles/desired/v1.md``), with core 0.36.
Read again against text 0.9 (#609, round 87; #935): its inline history
taken out and its 25 long sentences split, no rule changed, and none of
this module's rules moved. Where 0.9 reads "in a run", this module's older
comments still say "in this instance": the unit is the run (§0), as since
0.4 (F-192).

A desired document is state of its **author**, keyed by its **target**: the
author writes it under its own address, and the target binds its own slice of
the author's template (core R2), converges, and says what it did in a
``desired.v1.Report`` of its own. Written from the text alone (#609, round
52; text 0.2 in round 53):

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
(``SPEC-FINDINGS.md``, "At desired.v1 0.1" and "At desired.v1 0.2").
"""

from __future__ import annotations

import json
import math
import re
from collections import OrderedDict
from dataclasses import dataclass
from decimal import Decimal
from functools import lru_cache
from pathlib import Path
from typing import Any, Iterable

from . import alarms, jcs, keys, slug, telemetry, templates
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
    # 0.2 (F-173): the descriptions of the Report, its observed and its
    # outcome say "acted on or adopted"; every keyword unchanged.
    ("sha256:43c952598f454ec903dba63bad36419f5dcff3312db727bd4e58a88dc1ca55ae", "0.2"),
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
#: value an earlier one found wrong skipped"; ``not_string`` since 0.2
#: (F-170).
DECLARATION_REASONS = ("not_state", "value", "repeated", "not_a_param", "rest_param", "param_not_target",
                       "not_string")


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


def declare(kind: str, template: str, annotations: dict[str, Any], params: dict[str, str]) -> Declaration:
    """§2.2 over a resolved resource: its authoring kind (or kind token), its
    template, its resolved annotations (core §9.3) and its parameters' types
    as its contract declares them (core §2.2: ``string``, ``uint``,
    ``path``). "A resource without the key is not this profile's, whatever
    else it declares"."""
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
        # 0.2 (F-170): "a target parameter whose type (core §2.2) is not
        # string … It reads the type of a name that is a parameter and not
        # a rest one". A type the contract leaves out is not string either
        # (core's E011 refuses such a contract first).
        if any(n in single and params.get(n) != "string" for n in names):
            reasons.append("not_string")
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
#: §2.3's findings, in their order; ``two_documents`` since 0.6 (F-218).
BINDING_REASONS = ("required_unbound", "two_documents", "not_one_author", "target_unbound", "not_self", "wrong_self")
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
         requirement: dict[str, Any], also: Iterable[Any] = ()) -> Binding:
    """§2.3 over a declared resource and a requirement as the target's
    descriptor lists it (core §3.3: ``bindings``, the provider addresses;
    ``params``, the parameter bindings as configured; ``optional``).
    ``also`` (0.6, F-218) is the other desired resources of the author's
    declaration the requirement names: any is ``two_documents``."""
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
    # 0.6 (F-218): "after required_unbound, which reads only whether a
    # binding exists … and before every other problem, alone: each of them
    # binds the one slice there is not"
    if list(also):
        return Binding(INVALID, ("two_documents",))
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
        that does not read, is equal byte for byte". Read as strict JSON as
        §2.10 reads (finite by rounding, at most 256 deep), every number by
        its exact value (0.2): an integer as itself, any other number as an
        :class:`Exact`, the value its text denotes."""
        if json_encoded:
            try:
                return cls(value=strict_json(data, exact=True))
            except NotStrict:
                pass
        return cls(data=bytes(data))


def _in_strict_domain(v: Any) -> bool:
    """Whether ``v`` is a value strict JSON reading can yield (§2.10, as
    :func:`strict_json` reads): null, a boolean, an integer, a finite float
    or :class:`Exact`, a string with a UTF-8 form, and arrays and objects of them,
    every key a string with a UTF-8 form, at most 256 deep."""
    def walk(x: Any) -> bool:
        if x is None or isinstance(x, (bool, int)):
            return True
        if isinstance(x, float):
            return math.isfinite(x)
        if isinstance(x, Exact):
            return math.isfinite(x.double)
        if isinstance(x, str):
            return _utf8(x)
        if isinstance(x, list):
            return all(walk(y) for y in x)
        if isinstance(x, dict):
            return all(isinstance(k, str) and _utf8(k) and walk(y) for k, y in x.items())
        return False

    try:
        return telemetry.depth(v) <= MAX_DEPTH and walk(v)
    except RecursionError:
        return False


#: An exponent whose text, its leading zeros stripped, is longer than this
#: is kept as text (``number_form``): beyond about 10^400 either way a
#: value's double is 0 or an infinity, so no such exponent is arithmetic
#: any reader needs, and comparing it as text keeps the cost to its length.
_EXPONENT_DIGITS = 25

_JSON_NUMBER = re.compile(r"(-?)(0|[1-9][0-9]*)(?:\.([0-9]+))?(?:[eE]([+-]?)([0-9]+))?")


def _form(sign: int, digits: str, exponent: Any) -> tuple[int, str, Any]:
    """(sign, significant digits, exponent), the value sign × digits ×
    10^exponent, with no leading or trailing zero in ``digits``; zero is
    ``(0, "", 0)``, whatever its sign. ``exponent`` is an int, or, kept as
    text, ``(sign, text, adjustment)``."""
    d = digits.lstrip("0")
    if not d:
        return (0, "", 0)
    t = len(d) - len(d.rstrip("0"))
    d = d[:len(d) - t] if t else d
    if isinstance(exponent, tuple):
        es, et, adj = exponent
        return (sign, d, (es, et, adj + t))
    return (sign, d, exponent + t)


@dataclass(frozen=True)
class Exact:
    """A JSON number that is not an integer, read from its text for §2.7's
    equality (0.2, F-166): its exact value as a normalised form, computed
    from the text alone (so its cost is the text's length, never its
    exponent's value), and its double, for §2.10's finiteness by rounding."""

    form: tuple[int, str, Any]
    double: float

    @classmethod
    def of_text(cls, text: str) -> Exact:
        m = _JSON_NUMBER.fullmatch(text)
        if m is None:  # the JSON parser hands only its own grammar here
            raise NotStrict(f"no JSON number: {text[:40]!r}")
        neg, whole, frac, esign, edigits = m.groups()
        frac = frac or ""
        sign = 1 if neg else 0
        etext = (edigits or "0").lstrip("0") or "0"
        if len(etext) > _EXPONENT_DIGITS:
            exponent: Any = (esign == "-", etext, -len(frac))
        else:
            exponent = (-int(etext) if esign == "-" else int(etext)) - len(frac)
        # float() of a number's text rounds to the nearest double, ties to
        # even, in time linear in the text: an overflow is an infinity.
        return cls(_form(sign, whole + frac, exponent), float(text))


class NonInteger(float):
    """A number that is no integer, carried as its double for a schema's
    ``type``, ``minimum`` and ``maximum`` (§2.10, 0.3): its double may be
    integral (``1e-400`` reads as 0.0), so ``is_integer`` says what its
    exact value is."""

    def is_integer(self) -> bool:  # type: ignore[override]
        return False


def exact_value(x: Exact) -> int | NonInteger:
    """§2.10 (0.3, F-181): "every number read by its exact value … a number
    with a zero fraction is that integer, so 3.0 is 3 and
    18446744073709551615.0 is 2^64−1 … and 1.5 is no integer". A finite
    number's exponent is at most about 308, so the integer is cheap: one
    past the finite range, which :func:`strict_json` refuses before any
    caller sees it, is refused here too (``NotStrict``), never computed."""
    sign, digits, exponent = x.form
    if not math.isfinite(x.double):
        raise NotStrict("a number past the double's finite range")
    if not digits:
        return 0
    if isinstance(exponent, tuple) or exponent < 0:
        return NonInteger(x.double)
    value = int(digits) * 10 ** exponent
    return -value if sign else value


def exact_json(value: Any) -> Any:
    """A value :func:`strict_json` read with ``exact``, every :class:`Exact`
    replaced by :func:`exact_value`."""
    if isinstance(value, Exact):
        return exact_value(value)
    if isinstance(value, list):
        return [exact_json(v) for v in value]
    if isinstance(value, dict):
        return {k: exact_json(v) for k, v in value.items()}
    return value


def _number_text(x: Exact) -> str:
    """An :class:`Exact` written back, its exact value kept: its significant
    digits and its exponent (``10000000000000000555e-20``), an integer as
    its digits."""
    sign, digits, exponent = x.form
    if not digits:
        return "0"
    neg = "-" if sign else ""
    if isinstance(exponent, tuple):
        eneg, etext, adj = exponent
        if adj >= 0:
            mant = digits + "0" * adj
        elif len(digits) > -adj:
            mant = digits[:adj] + "." + digits[adj:]
        else:
            mant = "0." + "0" * (-adj - len(digits)) + digits
        return f"{neg}{mant}e{'-' if eneg else ''}{etext}"
    if exponent >= 0:
        return neg + digits + "0" * exponent if exponent <= 400 else f"{neg}{digits}e{exponent}"
    if -exponent <= 40:
        k = -exponent
        whole = digits[:-k] if len(digits) > k else "0"
        frac = digits[-k:] if len(digits) > k else "0" * (k - len(digits)) + digits
        return f"{neg}{whole}.{frac}"
    return f"{neg}{digits}e{exponent}"


def canonical_json(value: Any) -> bytes:
    """§2.11: "encode each document deterministically (JSON's members in one
    order …)", with every number written by its exact value (§2.7, 0.2): a
    value :func:`strict_json` read with ``exact``, or a decoded one. Members
    sorted by their code points, no whitespace. Raises ``ValueError`` for a
    value strict JSON cannot carry (a non-finite number)."""
    out: list[str] = []

    def w(v: Any) -> None:
        if v is None or isinstance(v, bool):
            out.append("null" if v is None else "true" if v else "false")
        elif isinstance(v, Exact):
            if not math.isfinite(v.double):
                raise ValueError("a number past the double's finite range")
            out.append(_number_text(v))
        elif isinstance(v, int):
            out.append(str(v))
        elif isinstance(v, float):
            if not math.isfinite(v):
                raise ValueError("a non-finite number")
            out.append(repr(v))
        elif isinstance(v, str):
            out.append(json.dumps(v, ensure_ascii=False))
        elif isinstance(v, list):
            out.append("[")
            for i, x in enumerate(v):
                if i:
                    out.append(",")
                w(x)
            out.append("]")
        elif isinstance(v, dict):
            out.append("{")
            for i, k in enumerate(sorted(v)):
                if i:
                    out.append(",")
                out.append(json.dumps(k, ensure_ascii=False))
                out.append(":")
                w(v[k])
            out.append("}")
        else:
            raise ValueError(f"not JSON: {type(v).__name__}")

    w(value)
    return "".join(out).encode("utf-8")


def revision_of(payload: bytes) -> int | None:
    """§2.4 (0.3): a document's own revision, "a top-level member named
    revision … an integer from 0 to 2^64−1, read by its exact value as a
    report's is: 3.0 is 3, 1.5 is no revision". None for a document that
    does not read, or carries none."""
    try:
        doc = exact_json(strict_json(payload, exact=True))
    except NotStrict:
        return None
    r = doc.get("revision") if isinstance(doc, dict) else None
    if isinstance(r, int) and not isinstance(r, bool) and 0 <= r <= U64_MAX:
        return r
    return None


def number_form(x: Any) -> tuple[int, str, Any]:
    """The exact value of a JSON number as ``_form`` writes it: an integer's
    from its decimal digits (at most 4300, Python's own bound, and at most
    309 for a finite one), a float's from its double's exact decimal (at
    most about 1100 digits, the double's exponent being bounded), an
    :class:`Exact`'s from its text."""
    if isinstance(x, Exact):
        return x.form
    if isinstance(x, int):
        return _form(1 if x < 0 else 0, str(abs(x)), 0)
    t = Decimal(x).as_tuple()  # a finite double's exact value
    return _form(t.sign, "".join(map(str, t.digits)), t.exponent)


def _is_number(v: Any) -> bool:
    return isinstance(v, (int, float, Exact)) and not isinstance(v, bool)


def _exactly_equal(a: Any, b: Any) -> bool:
    """JSON values equal by value, a number's exactly (``10`` is ``10.0`` is
    ``1e1``; a float is the exact value of its double, an :class:`Exact`
    the value of its text), compared as normalised forms, so the cost is
    bounded by the numbers' texts; a boolean never a number; members in any
    order. Two exponents kept as text are equal only when their texts and
    adjustments are: a value written two ways past 10^25 in its exponent
    reads as two, which costs an act, never skips one (§2.7)."""
    if _is_number(a) and _is_number(b):
        return number_form(a) == number_form(b)
    if type(a) is not type(b):
        return False
    if isinstance(a, dict):
        return a.keys() == b.keys() and all(_exactly_equal(a[k], b[k]) for k in a)
    if isinstance(a, list):
        return len(a) == len(b) and all(_exactly_equal(x, y) for x, y in zip(a, b))
    return bool(a == b)


def jcs_equal(a: Any, b: Any) -> bool:
    """§2.7 (0.2, F-166): "JCS-equal … **Every number compares by its exact
    value**, the value its text denotes, never the double RFC 8785 reads it
    as: 10, 10.0 and 1e1 are one value, and two documents that differ in an
    integer beyond 2^53, such as 2^53 and 2^53 + 1, are two documents". JCS's
    other rules (members in any order, strings by their code points) are a
    structural comparison. A decoded document's float is its double; a
    document read from its text (:meth:`Payload.of_bytes`) holds
    :class:`Exact` numbers.

    A value strict JSON reading cannot yield (a non-finite number, a string
    with no UTF-8 form, a key that is not a string, a value that is not
    JSON, a nesting past 256) is no document read as strict JSON: it equals
    nothing, itself included, and costs an act, never skips one."""
    try:
        return _in_strict_domain(a) and _in_strict_domain(b) and _exactly_equal(a, b)
    except RecursionError:
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
#: §2.6 "What it remembers" (0.2, F-163): "at least the 16 most recent ids
#: it acted on or adopted a version of". zk2py remembers exactly that many.
REMEMBERED_IDS = 16
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

    def __init__(self, key: str | None, memory: int = REMEMBERED_IDS) -> None:
        self.key = key
        self.memory = memory
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
        #: §2.6 (0.2): per id, "the latest time it acted on or adopted a
        #: version of that id in this instance", the most recent last.
        self.remembered: OrderedDict[int, int] = OrderedDict()

    def _remember(self, v: _Version) -> None:
        """§2.6 "What it remembers": an act (recorded when it starts, §2.7)
        or an adoption moves its id to the most recent, at its latest time;
        an id beyond the 16 most recent is forgotten, and then read as new."""
        m = self.remembered
        m[v.stamp.id] = max(m.get(v.stamp.id, v.stamp.time), v.stamp.time)
        m.move_to_end(v.stamp.id)
        while len(m) > self.memory:
            m.popitem(last=False)

    # The order of versions (§2.6), one held version against another.
    @staticmethod
    def _judge(v: _Version, held: _Version) -> str | None:
        """§2.6 steps 3 and 4 against one held version: the same version,
        older, or None for admitted (a later time of its id, or another
        id). The ids it remembers are judged beside it (0.2, F-163)."""
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
        """§2.6: judge one sample or reply, in the order received: against the
        version last acted on or adopted (the running act's while one runs)
        and every id it remembers (0.2, F-163), then, while an act runs,
        against the one held waiting (0.2, F-162). It is admitted when each
        admits it, and held waiting while an act runs."""
        if s.stamp is None:
            return UNSTAMPED  # step 1: nothing it holds changes
        v = _Version(s.stamp, s.delete, None if s.delete else s.payload, None if s.delete else s.revision,
                     s.origin == ARCHIVE)
        if self.current is not None:
            word = self._judge(v, self.current)
            if word == SAME_VERSION:
                return self._same(v, self.current, s.origin)
            if word == OLDER:
                return OLDER
        # Step 4 (0.2): "never an older version of an id it remembers …
        # whichever id is held, and from whichever path it came: earlier is
        # older …; equal is the same version …; later is admitted."
        latest = self.remembered.get(v.stamp.id)
        if latest is not None:
            if v.stamp.time < latest:
                return OLDER
            if v.stamp.time == latest:
                return SAME_VERSION
        if self.running and self.waiting is not None:
            word = self._judge(v, self.waiting)
            if word == SAME_VERSION:
                return self._same(v, self.waiting, s.origin)
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
            self._remember(v)
            self.observed = v.written()
            self.last_known = v.last_known
            self.report_puts += 1
            return ADOPTED
        self.current = v
        self._remember(v)
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
    """§2.10 (0.2, F-167): "Finite is by rounding, as telemetry.v1 §2.6
    reads it: a number's text, an integer's included, is finite when its
    exact value, rounded to the nearest double …, is not an infinity".
    Python's ``float()`` rounds so, ties to even: an integer past the edge
    raises ``OverflowError``, and a number's text past it (an
    :class:`Exact`'s, or a float's as ``json`` reads it) is an infinity, in
    time linear in the text."""
    if isinstance(v, bool):
        return True
    if isinstance(v, float):
        return math.isfinite(v)
    if isinstance(v, Exact):
        return math.isfinite(v.double)
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


#: §2.10 "Depth" (0.2): "a document nests at most 256 arrays and objects
#: deep, counted as telemetry.v1 §2.6 counts it".
MAX_DEPTH = 256


def strict_json(payload: bytes, exact: bool = False) -> Any:
    """§2.10, "As alarms.v1 §2.4 reads a document: strict JSON first (UTF-8,
    RFC 8259, nothing after the value but whitespace, no duplicate member at
    any depth, every number finite, every string with a UTF-8 form)". The
    whitespace is RFC 8259's ``ws`` (alarms.v1 0.3). "Finite (0.2) is by
    rounding", and "Depth (0.2): a document nests at most 256 arrays and
    objects deep": one at depth 257 is not strict JSON. A value nested past
    Python's recursion limit is not read either, never a crash.

    ``exact`` reads a number that is not an integer as an :class:`Exact`,
    its text's value: for §2.7's equality (0.2, F-166), and for a report and
    a document's revision (0.3, F-181, through :func:`exact_json`);
    otherwise as its double. An integer is exact either way. Raises
    :class:`NotStrict`."""
    try:
        text = payload.decode("utf-8")
    except UnicodeDecodeError as e:
        raise NotStrict(f"not UTF-8: {e}") from e
    try:
        value = json.loads(text.strip(" \t\n\r"), object_pairs_hook=_no_duplicates, parse_constant=_no_constant,
                           **({"parse_float": Exact.of_text} if exact else {}))
        if not _utf8(value):
            raise NotStrict("a string with no UTF-8 form")
        if not _finite(value):
            raise NotStrict("a number past the double's finite range")
        if telemetry.depth(value) > MAX_DEPTH:
            raise NotStrict(f"nested deeper than {MAX_DEPTH} arrays and objects")
    except NotStrict:
        raise
    except RecursionError as e:
        raise NotStrict("nested past this reader's depth") from e
    except ValueError as e:
        raise NotStrict(str(e)) from e
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
    """§2.10 over a report's bytes: strict JSON first, every number then read
    by its exact value (0.3, F-181)."""
    try:
        value = exact_json(strict_json(payload, exact=True))
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
    - ``deployment``: across a face, the deployment's word for what the
      descriptor would say, as ``descriptor`` reads, None when the tool has
      none (0.2, F-168); ignored elsewhere;
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
    deployment: tuple[bool, bool] | None = None


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
    # 2. Its descriptor, "or across a constrained face the deployment's word
    # for what it would say, listed and bound (0.2): not read, or no word
    # had, unobservable, no_descriptor". A kill switch is answered here
    # across a face too, before any report is read (F-168).
    said = r.deployment if across else r.descriptor
    if said is None:
        return _all(UNOBSERVABLE, "no_descriptor")
    lists, bound = said
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
    d, w = t["descriptor"], t.get("deployment")
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
                   t["report_freshness"], a["presence"], cur,
                   None if w is None else (bool(w["lists"]), bool(w["bound"])))


# -- §2.10, §2.17: the slice a tool reads, and which reply is the report (0.5) -------------------

#: §2.17 (0.5) "What a tool reads to answer": where a slice came from.
AUTHOR_CONTRACT, OFFLINE_CONTRACT, REQUIREMENT = "author_contract", "offline_contract", "requirement"
#: §2.10 "Which reply is the report": found one, unreadable, none, or no slice.
FOUND_ONE, FOUND_UNREADABLE, FOUND_NONE, NO_SLICE = "one", "unreadable", "none", "no_slice"
#: 0.6 (F-217): a revision in hand declaring no desired resource the
#: requirement names: the role is no target's
NOT_A_TARGET = "not_a_target"
#: 0.8 (F-225): a requirement whose declaring contract the tool does not
#: hold, or holds in a revision declaring no such role, is unread
REQUIREMENT_UNREAD = "requirement_unread"
TWO_DIFFER, UNREADABLE_REPLY = "two_differ", "unreadable_reply"


@dataclass(frozen=True)
class SliceTaken:
    """The slices a tool reads one role through, before its report is
    found: one from a contract, the candidates from the target alone, none
    when §2.3 finds the binding invalid or §2.2 the declaration."""

    candidates: tuple[str, ...]
    source: str | None
    binding: tuple[str, ...] = ()
    declaration: tuple[str, ...] = ()
    #: False when a revision of the author's interface in hand, named or
    #: held, declares no desired resource the requirement names: the role
    #: is not this profile's (§2.1, 0.6, F-217)
    desired: bool = True
    #: True when the requirement is unread (0.8, F-225): no slice, no
    #: binding problem, beside requirement_unread
    unread: bool = False


@dataclass(frozen=True)
class ReportFound:
    """§2.10's choice among the replies of a target's report resources."""

    found: str
    claims: str | None = None
    why: str | None = None
    reading: ReportReading | None = None
    #: the key of the reply found, the report resource's member
    reply: str | None = None

    def expect(self) -> dict[str, Any]:
        if self.found == FOUND_ONE:
            return {"found": FOUND_ONE, "claims": self.claims}
        if self.found == FOUND_UNREADABLE:
            return {"found": FOUND_UNREADABLE, "why": self.why}
        return {"found": self.found}

    def as_reading(self) -> ReportReading | None:
        """What §2.17 step 3 reads: the report found, an unreadable one, or
        none (``no_report``)."""
        if self.found == FOUND_ONE:
            return self.reading
        if self.found == FOUND_UNREADABLE:
            return ReportReading(False, (self.why or UNREADABLE_REPLY,))
        return None


def _names_resource(requirement: dict[str, Any], template: str) -> bool:
    """Core §3.1: a requirement names the resources it consumes, every one
    when it lists none."""
    names = requirement.get("resources")
    return names is None or template in names


def _kind_of(r: dict[str, Any]) -> tuple[str, bool]:
    """A resource as declarations.json gives one: its kind, read as its kind
    token (``@state`` for an explicit state), and whether its slice is under
    ``@state``. ``explicit`` on a kind that is not state changes neither: such
    a resource is no document's (§2.2 ``not_state``), and gives no slice."""
    kind = str(r.get("kind"))
    explicit = kind == "@state" or (kind == "state" and bool(r.get("explicit")))
    return ("@state" if explicit else kind), explicit


def desired_named(resources: Iterable[dict[str, Any]], requirement: dict[str, Any]) -> list[dict[str, Any]]:
    """A revision's desired resources the requirement names: each resource
    "whose resolved annotations hold desired.target_param" (§0), whatever
    its declaration then says (§2.2). §2.1 (0.6): "A requirement on desired
    resources names exactly one of them"."""
    return [r for r in resources
            if KEY in (r.get("annotations") or {}) and _names_resource(requirement, str(r.get("template")))]


def _from_contract(found: list[dict[str, Any]], iface: str, target: str, requirement: dict[str, Any],
                   source: str) -> SliceTaken:
    """§2.3 over the author's declaration: "The kind token and the target
    parameters are the author's declaration". Two or more desired
    resources named are ``two_documents`` (0.6, F-218), after the binding's
    absence."""
    res = found[0]
    kind, explicit = _kind_of(res)
    decl = declare(kind, str(res["template"]), dict(res.get("annotations") or {}), dict(res.get("params") or {}))
    if decl.declaration != DECLARED and len(found) == 1:
        return SliceTaken((), None, (), decl.reasons)
    if decl.declaration != DECLARED:
        # Two documents named, whatever the first's declaration says: §2.3's
        # order reads the binding's absence first, then two_documents
        # alone. This stand-in declaration is only for bind, which with a
        # non-empty also returns before it reads any target parameter, so
        # the stand-in's targets are never read (asserted below).
        decl = Declaration(DECLARED, (), tuple(Target(n, EITHER) for n in
                                               templates.parse_template(str(res["template"])).param_names))
        b = bind(decl, str(res["template"]), explicit, iface, target, requirement, also=found[1:])
        assert b.binding != BOUND, "two_documents never binds the stand-in's slice"
        return SliceTaken((), None, b.reasons)
    b = bind(decl, str(res["template"]), explicit, iface, target, requirement, also=found[1:])
    if b.binding != BOUND:
        return SliceTaken((), None, b.reasons)
    assert b.key is not None
    return SliceTaken((b.key,), source)


def _readable(rev: Any) -> bool:
    """A revision as take_slice reads one: a dict whose ``resources`` is a list."""
    return isinstance(rev, dict) and isinstance(rev.get("resources"), list)


def take_slice(target: str, iface: str, requirement: dict[str, Any], named: dict[str, Any] | None,
               held: Iterable[dict[str, Any]] = ()) -> SliceTaken:
    """§2.17 (0.5) "What a tool reads to answer", the slice:
    - from the author's contract, the revision its descriptor names, in
      hand (``named``: its resources);
    - else, with no revision its descriptor names in hand, from a revision
      of its interface held offline that declares a desired resource the
      requirement names (F-213): "any such revision gives the same slice,
      and the slice so taken is the slice";
    - else from the target alone (F-210): "each template its requirement
      names … each parameter filled as its binding says, under either kind
      token, state or @state, is a candidate", each in the requirement's
      order, ``state`` first.
    "A binding §2.3 finds invalid, or a declaration §2.2 does, gives no
    slice".

    With ``requirement["unread"]`` (0.8, F-225), "a requirement whose
    declaring contract the descriptor names and the tool does not hold …
    is unread: no binding problem is read from it … never every resource".
    "A revision of the author's in hand, named or held, that declares no
    desired resource at all still shows the role no target's … one that
    declares any is no help, one desired resource included". The binding's
    absence is the descriptor's word, read first: unbound and optional is
    the kill switch, unbound and required ``required_unbound``."""
    # A revision whose resources do not read (no list) shows nothing: it is
    # not in hand, and can prove neither a slice nor that the role is no
    # target's. The tool's revisions come from loaded or verified contracts,
    # whose resources are always a list; another caller may pass less.
    if named is not None and not _readable(named):
        named = None
    held = [rev for rev in held if _readable(rev)]
    if requirement.get("unread"):
        if not requirement.get("bindings"):
            return SliceTaken((), None, () if requirement.get("optional") else ("required_unbound",))
        revisions = [named] if named is not None else held
        if revisions and all(not any(KEY in (r.get("annotations") or {}) for r in rev["resources"])
                             for rev in revisions):
            return SliceTaken((), None, desired=False)
        return SliceTaken((), None, unread=True)
    if named is not None:
        found = desired_named(named.get("resources") or [], requirement)
        if found:
            return _from_contract(found, iface, target, requirement, AUTHOR_CONTRACT)
        # The revision its descriptor names decides, whatever held revisions
        # declare: one in hand that declares no desired resource the
        # requirement names makes the role not this profile's, not_a_target
        # (0.6, F-217).
        return SliceTaken((), None, desired=False)
    for rev in held:
        found = desired_named(rev.get("resources") or [], requirement)
        if found:
            return _from_contract(found, iface, target, requirement, OFFLINE_CONTRACT)
    if held:
        # 0.6 (F-217): "the target alone is read only with no revision of the
        # interface in hand at all"
        return SliceTaken((), None, desired=False)
    # From the target alone. Every parameter of a desired template is a
    # target parameter (§2.2); which identity each takes is its binding's,
    # the declaration's order unknown, so wrong_self cannot be found here.
    params = dict(requirement.get("params") or {})
    candidates: list[str] = []
    reasons: set[str] = set()
    for template in requirement.get("resources") or []:
        t = templates.parse_template(template)
        targets = tuple(Target(n, params[n] if params.get(n) in (SELF_SYSTEM, SELF_SERVICE) else EITHER)
                        for n in t.param_names)
        decl = Declaration(DECLARED, (), targets)
        for explicit in (False, True):
            b = bind(decl, template, explicit, iface, target, requirement)
            if b.binding == BOUND:
                assert b.key is not None
                if b.key not in candidates:
                    candidates.append(b.key)
            else:
                reasons |= set(b.reasons)
    if reasons:
        order = (*BINDING_REASONS, TARGET_ADDRESS)
        return SliceTaken((), None, tuple(x for x in order if x in reasons))
    if not requirement.get("bindings"):
        return SliceTaken((), None)  # unbound: the kill switch, §2.17 step 2's
    return SliceTaken(tuple(candidates), REQUIREMENT if candidates else None)


def find_report(taken: SliceTaken, replies: Iterable[tuple[str, bytes]]) -> ReportFound:
    """The report of a role, from what its slice taking found: no target's
    role (0.6, F-217), else :func:`which_report` among its candidates."""
    if not taken.desired:
        return ReportFound(NOT_A_TARGET)
    if taken.unread:
        return ReportFound(REQUIREMENT_UNREAD)
    return which_report(replies, taken.candidates)


def which_report(replies: Iterable[tuple[str, bytes]], candidates: Iterable[str]) -> ReportFound:
    """§2.10 (0.5) "A tool finds a target's reports", among the replies of
    every report resource of the target to S4's GET (each its key and its
    payload), each read as §2.10 reads a report:
    - the readable one whose ``key`` is a candidate is the report, two
      such that are equal being one; two readable ones that claim
      candidates and differ are unreadable (F-212, and F-210's "§2.10's
      two"). One key is one reply: S4's GET answers the later (core §4.1);
    - with none readable claiming one, one that does not read is
      unreadable too: it may be the slice's;
    - otherwise none was read.
    A report that reads with a §2.10 finding is found, and §2.17 step 3 then
    answers ``report_invalid``."""
    wanted = list(candidates)
    if not wanted:
        return ReportFound(NO_SLICE)
    claiming: list[tuple[str, ReportReading]] = []
    unreadable = False
    for rkey, payload in replies:
        r = read_report_bytes(payload)
        if not r.readable:
            unreadable = True
            continue
        if (r.report or {}).get("key") in wanted and all(x.report != r.report for _, x in claiming):
            claiming.append((rkey, r))
    if len(claiming) == 1:
        rkey, r = claiming[0]
        return ReportFound(FOUND_ONE, str((r.report or {})["key"]), reading=r, reply=rkey)
    if claiming:
        return ReportFound(FOUND_UNREADABLE, why=TWO_DIFFER)
    if unreadable:
        return ReportFound(FOUND_UNREADABLE, why=UNREADABLE_REPLY)
    return ReportFound(FOUND_NONE)


# -- §2.7 where a refusal points (0.4, F-193) -------------------------------------------------------

def _escape(name: str) -> str:
    return name.replace("~", "~0").replace("/", "~1")


def refusal_pointer(value: Any, schema: Any, root: dict[str, Any], at: str = "") -> str | None:
    """§2.7 (0.4, F-193): where a JSON Schema (core §7.3) refuses a decoded
    document, "as JSON Schema's output places an error, "" being the
    document itself", or None when it satisfies it:
    - "a keyword of a member's own schema that refuses the member (its
      type, its enum, its bounds) points at the member";
    - "a required member that is missing points at the object that holds
      it";
    - "a value of the wrong type points at that value".

    "A document the schema refuses in more than one place is refused at one
    of them, the target's choice": the first member refused, in the
    document's order, then the object itself. Numbers are read by their
    exact values (:func:`exact_json`) before."""
    from . import alarms

    if alarms.validate(value, schema, root):
        return None
    if not isinstance(schema, dict):
        return at
    if "$ref" in schema:
        inner = refusal_pointer(value, alarms._resolve(root, schema["$ref"]), root, at)
        if inner is not None:
            return inner
        schema = {k: v for k, v in schema.items() if k != "$ref"}
        if alarms.validate(value, schema, root):
            return at
    if "type" in schema:
        types = [schema["type"]] if isinstance(schema["type"], str) else schema["type"]
        if not any(alarms._is_type(value, t) for t in types):
            return at
    if isinstance(value, dict):
        props = schema.get("properties", {})
        for k, x in value.items():
            sub = props[k] if k in props else schema.get("additionalProperties", True)
            inner = refusal_pointer(x, sub, root, f"{at}/{_escape(k)}")
            if inner is not None:
                return inner
    elif isinstance(value, list):
        prefix = schema.get("prefixItems", [])
        for i, x in enumerate(value):
            sub = prefix[i] if i < len(prefix) else schema.get("items", True)
            inner = refusal_pointer(x, sub, root, f"{at}/{i}")
            if inner is not None:
                return inner
    # a required member missing, an enum, a bound or a count: at the value
    return at


def check_finish(v: Any, schema: dict[str, Any]) -> Finish:
    """order.json's ``{"finish": "check"}`` (0.4, F-193): the running act's
    put ends as its type's JSON Schema, ``$defs.Task`` of ``schema``, judges
    its document: done when it satisfies it, else refused whole, ``invalid``
    at :func:`refusal_pointer`'s pointer, with no detail. A payload that does
    not decode is ``undecodable`` (§2.7)."""
    if v is None or v.delete or v.payload is None:
        return Finish(DONE)
    if v.payload.value is None and v.payload.data is not None:
        return Finish(REFUSED, {"code": "undecodable"})
    doc = exact_json(v.payload.value)
    ptr = refusal_pointer(doc, schema["$defs"]["Task"], schema)
    return Finish(DONE) if ptr is None else Finish(REFUSED, {"code": "invalid", "pointer": ptr})


# -- order.json's steps --------------------------------------------------------------------------

def sample_of(x: dict[str, Any]) -> Sample:
    """order.json's ``sample``: a put's ``document`` (a payload encoded as
    JSON, decoded) or ``bytes`` (hex, compared byte for byte)."""
    stamp = None if x["stamp"] is None else Stamp.of(x["stamp"])
    if x["kind"] == "delete":
        return Sample(x["from"], True, stamp)
    if "document" in x:
        payload = Payload(value=x["document"])
    elif "text" in x:
        # 0.2: "its text, read as strict JSON as reports.json reads a report
        # … or byte for byte when it does not read".
        # surrogatepass keeps a lone surrogate in the text as its three bytes,
        # which are no UTF-8, so strict JSON refuses them on purpose and the
        # payload compares byte for byte, as §2.7 says of JSON that does not
        # read.
        payload = Payload.of_bytes(x["text"].encode("utf-8", "surrogatepass"), True)
    else:
        payload = Payload(data=bytes.fromhex(x["bytes"]))
    rev = x.get("revision")
    return Sample(x["from"], False, stamp, payload, None if rev is None else int(rev))


def run_steps(key: str | None, steps: list[dict[str, Any]],
              schema: dict[str, Any] | None = None) -> tuple[list[str], dict[str, Any]]:
    """order.json: one role's steps -> each step's word, and the report after
    the last. ``schema`` is the file's top-level one, which a
    ``{"finish": "check"}`` judges by (0.4)."""
    t = TargetRole(key)
    words: list[str] = []
    for s in steps:
        if "sample" in s:
            words.append(t.receive(sample_of(s["sample"])))
        elif "finish" in s:
            f = s["finish"]
            if f == "done":
                words.append(t.finish(Finish(DONE)))
            elif f == "check":
                if schema is None:
                    raise ValueError("a check step with no schema in the file")
                words.append(t.finish(check_finish(t.current, schema)))
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


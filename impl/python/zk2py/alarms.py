"""``alarms.v1``: the active alarms of one service (profile text 0.1, draft,
written against core 0.34; ``spec/profiles/alarms/v1.md``).

The half a reader and a tool need, from the text alone, with no owner and
no session:
- **the key** (§2.2, §2.3): :func:`derive` runs the recipe through the
  sanitizing step, which changes nothing in framed input, and reports
  whether the strict recipe accepts the input as it is (:func:`is_framed`);
  :func:`is_host_scoped`, :func:`fnv1a64`; :func:`key_of` and
  :func:`parse_key` are §2.12's structured reference and its key;
- **the document** (§2.4): :func:`read` is the reader's (a payload outside
  the 1.0 schema, or with a time out of range, does not read), and
  :func:`check` adds the owner's MUSTs and SHOULDs, as
  ``conformance/documents.json`` reports them. The schema is read from
  ``spec/profiles/alarms/schemas/alarms.v1.json`` at run time and checked
  by :func:`validate`, the core §7.3 subset;
- **transitions** (§2.6): :class:`Transitions`, one key's samples in the
  owner's stamp order, each against the document held before it;
- **the conclusions** (§2.14, §2.15, §5): :func:`judge` answers "is this
  alarm firing?" and "is this firing alarm past its horizon, its owner
  present?" from one :class:`Reading`; :func:`owner_set` says whether an
  owner's alarms are established; :func:`any_firing` is "is anything firing
  at or above *S*?"; :func:`rederives` is "does this key re-derive from its
  document?".

Freshness is ``freshness.v1``'s (§2.7): :func:`judge` hands the key's
observations to :func:`zk2py.freshness.judge` at the contract's 900 s.
"""

from __future__ import annotations

import json
import re
from dataclasses import dataclass, field
from functools import lru_cache
from pathlib import Path
from typing import Any, Iterable

from . import freshness as fr
from . import jcs, keys, slug
from .lexical import is_plain_chunk

PROFILE = "alarms.v1"
#: §3: the one resource, ``state/alarms/{alert_key}``.
TOKEN, TEMPLATE, PREFIX = "state", "alarms/{alert_key}", "alarms"
#: §3: `freshness.ttl_s = 900`; §2.7: confirmed at least every 450 s.
TTL_S = 900
HORIZON = fr.horizon("state", {fr.KEY: TTL_S})
CONFIRM_S = TTL_S // 2
#: §2.8: the contract's cardinality.
CARDINALITY = 256
#: §2.4: "`info`, `warning` or `critical`, in that order".
SEVERITIES = ("info", "warning", "critical")
FIRING, RESOLVED = "firing", "resolved"
#: §2.4: "both times from 0 to 2^53−1".
TIME_MAX = 2**53 - 1

#: §2.2 step 3: FNV-1a, 64 bits.
FNV_OFFSET, FNV_PRIME, MASK64 = 0xCBF29CE484222325, 0x100000001B3, 2**64 - 1
NEWLINE = "\n"  # §2.2 step 2: "a newline (0x0a)"

#: §2.4: "A name SHOULD be one or more segments `[a-z][a-z0-9_]*` joined by `.`".
NAME_FORM = re.compile(r"[a-z][a-z0-9_]*(?:\.[a-z][a-z0-9_]*)*")

SCHEMA_PATH = Path(__file__).resolve().parents[3] / "spec" / "profiles" / "alarms" / "schemas" / "alarms.v1.json"
TYPE_NAME = "alarms.v1.Alarm"

# §5's answers, and §2.15's verdicts on an owner's alarms.
YES, NO, UNOBSERVABLE, NOT_ASKED = "yes", "no", "unobservable", "not_asked"
ESTABLISHED = "established"
# §5's first and last questions, as conclusions.json names them.
Q_FIRING, Q_STALE = "firing", "stale_firing"

# §2.6's transitions, and transitions.json's `unreadable`.
RAISED, ESCALATED, REFRESHED, CONFIRMED, RETIRED, UNREADABLE = (
    "raised", "escalated", "refreshed", "confirmed", "retired", "unreadable")
DELETE = "delete"


# -- §2.2, §2.3 the key ------------------------------------------------------------

def is_host_scoped(name: str) -> bool:
    """§2.2 step 1: "the label named `host`, and every label whose name
    starts with `host.`. The comparison is exact, without trimming or case
    folding"."""
    return name == "host" or name.startswith("host.")


def discriminating(labels: dict[str, str] | None) -> dict[str, str]:
    """§0: "Every other label is **discriminating**"."""
    return {k: v for k, v in (labels or {}).items() if not is_host_scoped(k)}


def is_framed(rule: str, labels: dict[str, str] | None) -> bool:
    """§2.3 "Framed input": the rule not empty and without a newline; no
    discriminating label's name empty or holding a newline or `=`; no
    discriminating label's value holding a newline. Host-scoped labels are
    removed first, "so nothing in one breaks the framing"."""
    if rule == "" or NEWLINE in rule:
        return False
    return all(n != "" and NEWLINE not in n and "=" not in n and NEWLINE not in v
               for n, v in discriminating(labels).items())


def sanitize(rule: str, labels: dict[str, str] | None) -> tuple[str, list[tuple[str, str]]]:
    """§2.3's step 1 to 4: host-scoped labels removed; each newline of the
    rule and of each value `_`; each newline and `=` of each name `_`; then
    an empty rule `unnamed` and an empty name `_`. Names that sanitize to
    one stay two terms, which §2.2's sort orders by value."""
    r = rule.replace(NEWLINE, "_")
    pairs = [(n.replace(NEWLINE, "_").replace("=", "_"), v.replace(NEWLINE, "_"))
             for n, v in discriminating(labels).items()]
    if r == "":
        r = "unnamed"
    return r, [("_" if n == "" else n, v) for n, v in pairs]


def recipe_input(rule: str, pairs: Iterable[tuple[str, str]]) -> str:
    """§2.2 step 2: the rule, then for each label a newline, its name, `=`
    and its value, "sorted ascending by name, then by value, each compared
    as UTF-8 bytes (memcmp)"."""
    ordered = sorted(pairs, key=lambda p: (p[0].encode("utf-8"), p[1].encode("utf-8")))
    return rule + "".join(f"{NEWLINE}{n}={v}" for n, v in ordered)


def fnv1a64(data: bytes) -> int:
    """§2.2 step 3: "for each byte, the hash is XORed with it, then
    multiplied by the prime modulo 2^64"."""
    h = FNV_OFFSET
    for b in data:
        h = ((h ^ b) * FNV_PRIME) & MASK64
    return h


def alert_key_of(text: str) -> str:
    """§2.2 step 4: "16 lowercase hexadecimal digits, zero-padded"."""
    return f"{fnv1a64(text.encode('utf-8')):016x}"


@dataclass(frozen=True)
class Derivation:
    """keys.json's `derive` expectation: the alert key a reader derives
    (through §2.3's step), the string hashed, and whether the strict recipe
    (§2.2) accepts the input as it is."""

    key: str
    input: str
    framed: bool

    def expect(self) -> dict[str, Any]:
        return {"framed": self.framed, "input": self.input, "key": self.key}


def derive(rule: str, labels: dict[str, str] | None) -> Derivation:
    """The reader's derivation (§2.3: "A reader derives every key through
    this step"). On framed input it is §2.2's recipe, byte for byte."""
    r, pairs = sanitize(rule, labels)
    text = recipe_input(r, pairs)
    return Derivation(alert_key_of(text), text, is_framed(rule, labels))


def strict_key(rule: str, labels: dict[str, str] | None) -> str | None:
    """§2.2's recipe alone: None when the input is not framed."""
    if not is_framed(rule, labels):
        return None
    return alert_key_of(recipe_input(rule, discriminating(labels).items()))


# -- §2.12 the structured reference --------------------------------------------------

def key_of(system: str, service: str, alert_key: str) -> str | None:
    """§2.12: `zk2/<system>/<service>/alarms.v1/state/alarms/<alert_key>`,
    "the alert key slugged as any parameter value is (core §1.4)", or None
    when the system or the service is no name (core §1.2: a plain chunk).
    The service is carried whole, dots and all."""
    if not (is_plain_chunk(system) and is_plain_chunk(service)):
        return None
    key = f"zk2/{system}/{service}/{PROFILE}/{TOKEN}/{PREFIX}/{slug.slug(alert_key)}"
    return key if keys.parse(key) is not None else None


def parse_key(key: str) -> dict[str, str] | None:
    """The reference a key names, the alert key unslugged, or None when the
    key is not one of alarms.v1's one template: another major, another
    token (`@state` included), another template, a chunk no slug produces."""
    p = keys.parse(key)
    if p is None or p["form"] != "data" or p["iface"] != PROFILE or p["token"] != TOKEN:
        return None
    if len(p["resource"]) != 2 or p["resource"][0] != PREFIX:
        return None
    value = slug.unslug(p["resource"][1])
    if value is None:
        return None
    return {"system": p["system"], "service": p["service"], "alert_key": value}


# -- §2.4 the document: the 1.0 schema, read by core §7.3's subset -------------------

_ANNOTATIONS = {"$schema", "$id", "$defs", "$comment", "title", "description", "default", "examples", "format",
                "deprecated", "readOnly", "writeOnly"}
_MEANING = {"type", "properties", "required", "additionalProperties", "items", "prefixItems", "enum", "const",
            "minimum", "maximum", "exclusiveMinimum", "exclusiveMaximum", "minLength", "maxLength", "minItems",
            "maxItems", "$ref", "oneOf", "anyOf"}


def _number(v: Any) -> bool:
    return isinstance(v, (int, float)) and not isinstance(v, bool)


def _integral(v: Any) -> bool:
    """§2.4: "It reads an integral number as an integer (`1000.0` is
    `1000`), as JSON Schema 2020-12 does"."""
    return _number(v) and (isinstance(v, int) or v.is_integer())


def _json_equal(a: Any, b: Any) -> bool:
    """JSON Schema's equality: numbers by value (`1` is `1.0`), a boolean
    never a number."""
    if _number(a) and _number(b):
        return a == b
    if type(a) is not type(b):
        return False
    if isinstance(a, dict):
        return a.keys() == b.keys() and all(_json_equal(a[k], b[k]) for k in a)
    if isinstance(a, list):
        return len(a) == len(b) and all(_json_equal(x, y) for x, y in zip(a, b))
    return bool(a == b)


def _is_type(v: Any, t: str) -> bool:
    return {"object": isinstance(v, dict), "array": isinstance(v, list), "string": isinstance(v, str),
            "boolean": isinstance(v, bool), "null": v is None, "number": _number(v),
            "integer": _integral(v)}[t]


def _resolve(root: dict[str, Any], ref: str) -> Any:
    if not ref.startswith("#/"):
        raise NotImplementedError(f"$ref {ref!r}: the alarms schema refers within itself only")
    node: Any = root
    for part in ref[2:].split("/"):
        node = node[part.replace("~1", "/").replace("~0", "~")]
    return node


def validate(value: Any, schema: Any, root: dict[str, Any]) -> bool:
    """Whether ``value`` satisfies ``schema``: core §7.3's keywords that carry
    meaning, its annotations ignored. A keyword outside the subset raises,
    since no loaded contract holds one (E037)."""
    if schema is True:
        return True
    if schema is False:
        return False
    for k in schema:
        if k not in _MEANING and k not in _ANNOTATIONS:
            raise NotImplementedError(f"keyword {k!r} is outside core §7.3's subset")
    if "$ref" in schema and not validate(value, _resolve(root, schema["$ref"]), root):
        return False
    if "type" in schema:
        types = [schema["type"]] if isinstance(schema["type"], str) else schema["type"]
        if not any(_is_type(value, t) for t in types):
            return False
    if "enum" in schema and not any(_json_equal(value, e) for e in schema["enum"]):
        return False
    if "const" in schema and not _json_equal(value, schema["const"]):
        return False
    if "anyOf" in schema and not any(validate(value, s, root) for s in schema["anyOf"]):
        return False
    if "oneOf" in schema and sum(validate(value, s, root) for s in schema["oneOf"]) != 1:
        return False
    if _number(value):
        if "minimum" in schema and value < schema["minimum"]:
            return False
        if "maximum" in schema and value > schema["maximum"]:
            return False
        if "exclusiveMinimum" in schema and value <= schema["exclusiveMinimum"]:
            return False
        if "exclusiveMaximum" in schema and value >= schema["exclusiveMaximum"]:
            return False
    if isinstance(value, str):
        if "minLength" in schema and len(value) < schema["minLength"]:
            return False
        if "maxLength" in schema and len(value) > schema["maxLength"]:
            return False
    if isinstance(value, dict):
        props = schema.get("properties", {})
        if any(r not in value for r in schema.get("required", [])):
            return False
        for k, x in value.items():
            if k in props:
                if not validate(x, props[k], root):
                    return False
            elif "additionalProperties" in schema and not validate(x, schema["additionalProperties"], root):
                return False
    if isinstance(value, list):
        prefix = schema.get("prefixItems", [])
        for x, s in zip(value, prefix):
            if not validate(x, s, root):
                return False
        if "items" in schema and any(not validate(x, schema["items"], root) for x in value[len(prefix):]):
            return False
        if "minItems" in schema and len(value) < schema["minItems"]:
            return False
        if "maxItems" in schema and len(value) > schema["maxItems"]:
            return False
    return True


@lru_cache(maxsize=1)
def load_schema() -> dict[str, Any]:
    """The 1.0 schema (§3), read from ``spec/`` as published."""
    return json.loads(SCHEMA_PATH.read_text(encoding="utf-8"))


def in_schema(value: Any) -> bool:
    """§2.4: whether ``value`` satisfies the 1.0 schema's `alarms.v1.Alarm`.
    "Members the schema does not name" are ignored, since the type admits
    them (§3)."""
    root = load_schema()
    return validate(value, {"$ref": f"#/$defs/{TYPE_NAME}"}, root)


@dataclass(frozen=True)
class Alarm:
    """A document that reads (§2.4), its integral numbers as integers. A
    member left out is None; `labels` absent is None, never `{}`, since
    "Equal is equal in every member this text names" (§2.6; SPEC-FINDINGS
    F-127)."""

    timestamp: int
    rule: str
    severity: str
    state: str
    summary: str
    observed_at_ms: int | None = None
    category: str | None = None
    labels: dict[str, str] | None = None

    @property
    def firing(self) -> bool:
        return self.state == FIRING

    def derive(self) -> Derivation:
        return derive(self.rule, self.labels)


def _in_range(t: int | None) -> bool:
    return t is None or 0 <= t <= TIME_MAX


def read(value: Any) -> Alarm | None:
    """§2.4 "The reader's": None for a payload outside the schema, or with a
    time out of 0 to 2^53−1, "never at a severity or in a state it
    guesses". A document that breaks the rules of `observed_at_ms` reads as
    written."""
    if not in_schema(value):
        return None
    obs = value.get("observed_at_ms")
    a = Alarm(timestamp=int(value["timestamp"]), rule=value["rule"], severity=value["severity"],
              state=value["state"], summary=value["summary"],
              observed_at_ms=None if obs is None else int(obs), category=value.get("category"),
              labels=None if "labels" not in value else dict(value["labels"]))
    return a if _in_range(a.timestamp) and _in_range(a.observed_at_ms) else None


def _utf8(value: Any) -> bool:
    if isinstance(value, str):
        try:
            value.encode("utf-8")
        except UnicodeEncodeError:
            return False
        return True
    if isinstance(value, dict):
        return all(_utf8(k) and _utf8(v) for k, v in value.items())
    if isinstance(value, list):
        return all(_utf8(v) for v in value)
    return True


def decode(payload: bytes) -> Alarm | None:
    """A sample's payload, JSON (the contract's encoding, core §7.2), read as
    §2.4 says. zk2py reads strict JSON: a duplicate member, NaN, or a
    string with no UTF-8 form (a lone surrogate escape) does not read
    (SPEC-FINDINGS F-126)."""
    try:
        value = jcs.loads(payload)
    except jcs.JsonError:
        return None
    return read(value) if _utf8(value) else None


def check(value: Any) -> dict[str, Any]:
    """documents.json's reading: `readable`, the owner's `errors` (MUSTs
    broken) and its `warnings` (SHOULDs broken), in the order checked.
    Unreadable reports `schema`, else `time_range`, and nothing else."""
    if not in_schema(value):
        return {"readable": False, "errors": ["schema"], "warnings": []}
    a = read(value)
    if a is None:
        return {"readable": False, "errors": ["time_range"], "warnings": []}
    errors: list[str] = []
    if a.observed_at_ms is not None and a.observed_at_ms < a.timestamp:
        errors.append("observed_before_timestamp")
    if a.observed_at_ms is not None and a.state == RESOLVED:
        errors.append("observed_on_resolved")
    warnings: list[str] = []
    if not is_framed(a.rule, a.labels):
        warnings.append("unframed")
    # §2.4's form is a SHOULD on every name, a host-scoped one included.
    for name in sorted(a.labels or {}, key=lambda n: n.encode("utf-8")):
        if NAME_FORM.fullmatch(name) is None:
            warnings.append("label_name_form")
    return {"readable": True, "errors": errors, "warnings": warnings}


def at_or_above(severity: str | None, floor: str) -> bool:
    """§2.4: "A severity is **at or above** another when it is the same, or
    comes later". An unknown severity is not."""
    return severity in SEVERITIES and SEVERITIES.index(severity) >= SEVERITIES.index(floor)


# -- §2.6 reading transitions --------------------------------------------------------

class Transitions:
    """One key's samples, "in the order of the owner's stamps", each read
    "against the document it held before it". The reader starts holding
    nothing; a delete (or a `reply_del`) leaves it holding nothing, and so
    does a put that does not read."""

    def __init__(self) -> None:
        self.held: Alarm | None = None

    def step(self, sample: Any) -> str:
        """``sample`` is :data:`DELETE`, or a put's decoded payload (an
        :class:`Alarm`, or the JSON value)."""
        held = self.held
        if sample == DELETE:
            self.held = None
            return RESOLVED if held is not None and held.firing else RETIRED
        doc = sample if isinstance(sample, Alarm) else read(sample)
        self.held = doc
        if doc is None:
            return UNREADABLE
        if held is None:
            return RAISED if doc.firing else RESOLVED
        if held.firing:
            if not doc.firing:
                return RESOLVED
            if doc.severity != held.severity:
                return ESCALATED
            if doc.timestamp != held.timestamp:
                return RAISED
            return CONFIRMED if doc == held else REFRESHED
        if doc.firing:
            return RAISED
        if doc.timestamp != held.timestamp:
            return RESOLVED
        return CONFIRMED if doc == held else REFRESHED


def transitions(samples: Iterable[Any]) -> list[str]:
    t = Transitions()
    return [t.step(s) for s in samples]


# -- §2.14 what a reader concludes of one alarm -------------------------------------

@dataclass(frozen=True)
class Answer:
    answer: str
    reason: str
    detail: str = ""

    def pair(self) -> dict[str, str]:
        return {"answer": self.answer, "reason": self.reason}


@dataclass
class Reading:
    """§0: "what one reader observed of one alarm key at one instant", as
    conclusions.json gives it:
    - ``presence``: present, absent, incomplete, or across_face;
    - ``descriptor``: None (not read), or ``{"lists": bool, "token": bool}``;
      the token plays no part (§2.13);
    - ``face``: ``{"reachable": bool}`` across a face: whether the
      deployment lets this reader reach the alarms (§2.11);
    - ``observations``: freshness.v1's, of this key;
    - ``latest``: the latest sample by the owner's stamp: firing, resolved,
      deleted, undecodable, or None when none was read;
    - ``previous_instance``: that latest put stamped by an earlier instance
      of the owner (its stamp's id not the present descriptor's
      ``meta.zid``; SPEC-FINDINGS F-129);
    - ``held_whole``: the owner's alarms established in this reading
      (§2.15)."""

    presence: str
    descriptor: dict[str, Any] | None = None
    face: dict[str, Any] | None = None
    observations: list[Any] = field(default_factory=list)
    latest: str | None = None
    previous_instance: bool = False
    held_whole: bool = False


def judge(r: Reading, question: str = Q_FIRING) -> Answer:
    """§2.14: "the first of these steps that answers". ``question`` is
    :data:`Q_FIRING` ("is this alarm firing?") or :data:`Q_STALE` ("is this
    firing alarm past its horizon, its owner present?"), which "reads the
    same steps, with other answers in three places"."""
    stale_q = question == Q_STALE
    across = r.presence == "across_face"
    obs = list(r.observations)
    # 1. Presence (core §8.1).
    if r.presence == "absent":
        return Answer(NOT_ASKED, "absent", "a complete read without its instance token; an archive's is last-known")
    if r.presence == "incomplete":
        return Answer(UNOBSERVABLE, "presence_incomplete", "its token not seen, in a read possibly incomplete")
    if across:
        if not (r.face or {}).get("reachable"):
            return Answer(UNOBSERVABLE, "face_closed", "the face closes the alarms to this reader (§2.11)")
        if stale_q:
            return Answer(UNOBSERVABLE, "across_face", "presence does not cross")
    elif r.presence == "present":
        # 2. The descriptor (§2.13).
        if r.descriptor is None:
            return Answer(UNOBSERVABLE, "no_descriptor", "the descriptor was not read")
        if not r.descriptor.get("lists"):
            return Answer(NOT_ASKED, "not_listed", "its descriptor does not list alarms.v1")
    else:
        raise ValueError(f"presence {r.presence!r}")
    # 3. An archive alone (freshness.v1 §2.8).
    if obs and all(isinstance(o, fr.Archive) for o in obs):
        return Answer(NOT_ASKED, "last_known", "only an archive's answer was read (core S6)")
    # 4. A loss, with no GET that answered the key since (core §4.2): a
    # value or a reply_del for it, zk2py's reading (SPEC-FINDINGS F-128).
    lost = any(isinstance(o, fr.Subscription) and not o.complete for o in obs)
    reseeded = any(isinstance(o, fr.Get) and o.reply is not None for o in obs)
    if lost and not reseeded:
        return Answer(UNOBSERVABLE, "incomplete", "a delivery was lost, and no GET answered the key since")
    # 5. The latest sample.
    not_firing = Answer(NOT_ASKED, "not_firing", "the latest is not a firing alarm")
    if r.latest == "deleted":
        return not_firing if stale_q else Answer(NO, "deleted", "a delete or a reply_del")
    if r.latest == RESOLVED:
        return not_firing if stale_q else Answer(NO, "resolved", "a resolved document, whatever its age")
    if r.latest is None:
        if r.held_whole:
            return not_firing if stale_q else Answer(NO, "not_held", "the owner's established alarms hold no such key")
        if across:
            return Answer(UNOBSERVABLE, "nothing_crossed", "nothing of the key crossed the face")
        return Answer(UNOBSERVABLE, "not_read", "nothing of the key was read: silence is no verdict (core O5)")
    if r.previous_instance:
        return Answer(NOT_ASKED, "previous_instance", "an earlier instance's last word, shown as last-known")
    # 6. Its freshness, at 900 s (§2.7).
    v = fr.judge("state", HORIZON, obs)
    if stale_q:
        if r.latest != FIRING:
            return Answer(UNOBSERVABLE, "undecodable", "the latest put does not read")
        if v.verdict == fr.STALE:
            return Answer(YES, "stale", v.detail)
        if v.verdict == fr.FRESH:
            return Answer(NO, "fresh", v.detail)
        return Answer(UNOBSERVABLE, v.reason, v.detail)
    if v.verdict == fr.FRESH:
        if r.latest == FIRING:
            return Answer(YES, "firing", v.detail)
        return Answer(UNOBSERVABLE, "undecodable", "fresh, and the latest put does not read")
    if v.verdict == fr.STALE:
        if across and v.reason == "no_delivery":
            return Answer(UNOBSERVABLE, "nothing_crossed", "no push crosses the face at 1.0 (§2.11)")
        return Answer(UNOBSERVABLE, "stale", v.detail)
    # Unobservable (freshness.v1's reason). A not-asked freshness here is a
    # reading whose observations contradict its latest put (its last
    # delivery a delete, say): unobservable too, never a guess.
    return Answer(UNOBSERVABLE, v.reason, v.detail)


# -- §2.15 what a reader concludes of an owner's alarms -----------------------------

@dataclass(frozen=True)
class Seed:
    """The reader's S4 GET of ``…/alarms.v1/state/alarms/*``: complete (ran to
    its final reply with no error reply), and the keys it answered with a
    value."""

    complete: bool
    members: int = 0


@dataclass(frozen=True)
class Window:
    """The reader's subscription, declared before the GET (core §4.2)."""

    listened_s: float
    lossless: bool
    present_throughout: bool


@dataclass(frozen=True)
class Verdict:
    verdict: str
    reason: str

    def expect(self) -> dict[str, str]:
        return {"reason": self.reason, "verdict": self.verdict}


def owner_set(presence: str, descriptor: dict[str, Any] | None, face: dict[str, Any] | None,
              seed: Seed | None, window: Window | None) -> Verdict:
    """§2.15 "Established", else unobservable, else not asked, checked in
    this order (zk2py's, SPEC-FINDINGS F-130): §2.14's steps 1 and 2 with
    their reasons; no GET; a GET not complete; a window that lost a
    delivery; an owner not present throughout; a GET that answered a key
    (`answered`); across a face, an empty GET; no window; a window of 900 s
    or less; else `window`."""
    if presence == "absent":
        return Verdict(NOT_ASKED, "absent")
    if presence == "incomplete":
        return Verdict(UNOBSERVABLE, "presence_incomplete")
    across = presence == "across_face"
    if across:
        if not (face or {}).get("reachable"):
            return Verdict(UNOBSERVABLE, "face_closed")
    elif presence == "present":
        if descriptor is None:
            return Verdict(UNOBSERVABLE, "no_descriptor")
        if not descriptor.get("lists"):
            return Verdict(NOT_ASKED, "not_listed")
    else:
        raise ValueError(f"presence {presence!r}")
    if seed is None:
        return Verdict(UNOBSERVABLE, "no_seed")
    if not seed.complete:
        return Verdict(UNOBSERVABLE, "seed_incomplete")
    if window is not None and not window.lossless:
        return Verdict(UNOBSERVABLE, "window_lost")
    if window is not None and not window.present_throughout:
        return Verdict(UNOBSERVABLE, "owner_gap")
    if seed.members > 0:
        return Verdict(ESTABLISHED, "answered")
    if across:
        return Verdict(UNOBSERVABLE, "across_face")
    if window is None:
        return Verdict(UNOBSERVABLE, "no_member")
    if window.listened_s <= TTL_S:
        return Verdict(UNOBSERVABLE, "window_short")
    return Verdict(ESTABLISHED, "window")


@dataclass(frozen=True)
class AlarmRead:
    """One alarm of an owner in scope: its first question's answer, and the
    latest document's severity (None when none reads)."""

    answer: str
    severity: str | None


@dataclass(frozen=True)
class OwnerRead:
    verdict: str  # owner_set's: established, unobservable, not_asked
    alarms: tuple[AlarmRead, ...] = ()


def any_firing(at_least: str, owners: Iterable[OwnerRead]) -> dict[str, Any]:
    """§2.15 "Anything firing at or above a severity *S*", its five steps in
    order, with the count of alarms reading yes at *S* or above, and the
    owners counted by verdict. Which owners are in scope is the caller's: an
    absent one is counted only if the caller knows of it (SPEC-FINDINGS
    F-131)."""
    owners = list(owners)
    counts = {ESTABLISHED: 0, NOT_ASKED: 0, UNOBSERVABLE: 0}
    for o in owners:
        counts[o.verdict] += 1
    firing = sum(1 for o in owners for a in o.alarms if a.answer == YES and at_or_above(a.severity, at_least))

    def out(answer: str, reason: str) -> dict[str, Any]:
        return {"answer": answer, "firing": firing, "owners": counts, "reason": reason}

    if firing:
        return out(YES, "any_firing")
    if any(o.verdict == UNOBSERVABLE for o in owners):
        return out(UNOBSERVABLE, "owner_unobservable")
    for o in owners:
        if o.verdict != ESTABLISHED:
            continue
        for a in o.alarms:
            unknown = a.severity not in SEVERITIES
            if (a.answer == UNOBSERVABLE and (unknown or at_or_above(a.severity, at_least))) \
                    or (a.answer == YES and unknown):
                return out(UNOBSERVABLE, "alarm_unobservable")
    if counts[ESTABLISHED] == 0:
        return out(NOT_ASKED, "no_owner")
    return out(NO, "none_firing")


# -- §5 the third question -----------------------------------------------------------

def rederives(alert_key: str, payload: Any) -> Answer:
    """§5 "Does this key re-derive from its document?": `strict` when the
    recipe gives the key, `sanitized` when only §2.3's step does, else
    `mismatch`. A delete carries no document; a payload that does not read
    is unobservable. The comparison is exact. The reasons of a delete
    (`no_document`) and of an undecodable payload are the fixture's
    (SPEC-FINDINGS F-132)."""
    if payload == DELETE:
        return Answer(NOT_ASKED, "no_document", "a delete carries no document")
    doc = payload if isinstance(payload, Alarm) else read(payload)
    if doc is None:
        return Answer(UNOBSERVABLE, "undecodable", "the payload does not read (§2.4)")
    d = doc.derive()
    if d.key != alert_key:
        return Answer(NO, "mismatch", f"the document derives {d.key}")
    return Answer(YES, "strict") if d.framed else Answer(YES, "sanitized", "keyed through §2.3's step")

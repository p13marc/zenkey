"""``alarms.v1``: the active alarms of one service (profile text 0.1, draft,
written against core 0.34; ``spec/profiles/alarms/v1.md``). Read again
against text 0.9 (#609, round 87; #935): its inline history taken out and
its six long sentences split, no rule changed but §2.15's watch after a
window, now under a version of its own (F-326), and none of this module's
rules moved.

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
import threading
import time
from dataclasses import dataclass, field
from functools import lru_cache
from pathlib import Path
from typing import Any, Iterable

from . import budget as bg
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
    member left out is None, but `labels` absent is `{}`: §2.6 (0.2, F-127),
    "`labels` absent is equal to `labels: {}` … Any other member absent is
    equal only to itself absent, so `category: ""` is not an absent
    `category`"."""

    timestamp: int
    rule: str
    severity: str
    state: str
    summary: str
    observed_at_ms: int | None = None
    category: str | None = None
    labels: dict[str, str] = field(default_factory=dict)

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
              labels=dict(value.get("labels", {})))
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


class NotStrict(ValueError):
    """§2.4 "Strict JSON first" (0.2): bytes that are not strict JSON."""


#: RFC 8259 §2's ``ws``: space, horizontal tab, line feed, carriage return.
#: §2.4 (0.3, F-134): "Whitespace is RFC 8259's ws, before the value and
#: after it … Any other character after the value, a form feed or a
#: no-break space included, is text after it." A byte-order mark before the
#: value is refused too: zk2py's reading, RFC 8259's ``JSON-text`` having
#: none.
JSON_WS = b" \t\n\r"


def strict_json(payload: bytes) -> Any:
    """§2.4 (0.2, F-126): "UTF-8, JSON (RFC 8259) with nothing after its
    value, no duplicate member at any depth, every number finite, and every
    string with a UTF-8 form, so no lone surrogate escape". Raises
    :class:`NotStrict`."""
    # The whitespace is stripped here, explicitly, so the reading does not
    # rest on the JSON library's own (§2.4, 0.3).
    try:
        value = jcs.loads(payload.strip(JSON_WS))
    except jcs.JsonError as e:
        raise NotStrict(str(e)) from e
    if not _utf8(value):
        raise NotStrict("a string with no UTF-8 form")
    return value


def decode(payload: bytes) -> Alarm | None:
    """A sample's payload, JSON (the contract's encoding, core §7.2), read as
    §2.4 says: strict JSON first, then the schema and the time range."""
    try:
        return read(strict_json(payload))
    except NotStrict:
        return None


def check_payload(payload: bytes) -> dict[str, Any]:
    """documents.json's reading of a case given as its `text` (0.2): `json`
    when the bytes are not strict JSON, else :func:`check`."""
    try:
        value = strict_json(payload)
    except NotStrict:
        return {"readable": False, "errors": ["json"], "warnings": []}
    return check(value)


def check(value: Any) -> dict[str, Any]:
    """documents.json's reading of a decoded value: `readable`, the owner's
    `errors` (MUSTs broken) and its `warnings` (SHOULDs broken), in the
    order checked. Unreadable reports `schema`, else `time_range`, and
    nothing else."""
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
    for name in sorted(a.labels, key=lambda n: n.encode("utf-8")):
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
      (§2.15);
    - ``left_out`` (0.5): the GET that established them "left the key out:
      it answered the key with neither a value nor a `reply_del`, and
      nothing of the key arrived after that GET was asked" (§2.14 step 5),
      which heals step 4 and makes the held sample none."""

    presence: str
    descriptor: dict[str, Any] | None = None
    face: dict[str, Any] | None = None
    observations: list[Any] = field(default_factory=list)
    latest: str | None = None
    previous_instance: bool = False
    held_whole: bool = False
    left_out: bool = False


def judge(r: Reading, question: str = Q_FIRING, horizon: fr.Horizon = HORIZON) -> Answer:
    """§2.14: "the first of these steps that answers". ``question`` is
    :data:`Q_FIRING` ("is this alarm firing?") or :data:`Q_STALE` ("is this
    firing alarm past its horizon, its owner present?"), which "reads the
    same steps, with other answers in three places". ``horizon`` is the
    contract's: 900 s for the standard one, another for a short-ttl copy
    (scenarios.md's conventions, 0.2)."""
    stale_q = question == Q_STALE
    across = r.presence == "across_face"
    obs = list(r.observations)
    # 1. Presence (observer §3).
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
    # 4. A loss, "and since then no GET has answered this key, with a value
    # or a reply_del, nor has the GET that established its owner's alarms
    # (§2.15) left it out (step 5)" (0.5, F-136).
    left_out = r.left_out and r.held_whole
    lost = any(isinstance(o, fr.Subscription) and not o.complete for o in obs)
    reseeded = any(isinstance(o, fr.Get) and o.reply is not None for o in obs)
    if lost and not reseeded and not left_out:
        return Answer(UNOBSERVABLE, "incomplete", "a delivery was lost, and no GET answered the key since")
    # 5. The latest sample. "A sample the reader holds, a delete included,
    # is none … when the GET that established its owner's alarms, either
    # way (answered or window), left the key out" (0.3, 0.5).
    latest = None if left_out else r.latest
    not_firing = Answer(NOT_ASKED, "not_firing", "the latest is not a firing alarm")
    if latest == "deleted":
        return not_firing if stale_q else Answer(NO, "deleted", "a delete or a reply_del")
    if latest == RESOLVED:
        return not_firing if stale_q else Answer(NO, "resolved", "a resolved document, whatever its age")
    if latest is None:
        if r.held_whole:
            return not_firing if stale_q else Answer(NO, "not_held", "the owner's established alarms hold no such key")
        if across:
            return Answer(UNOBSERVABLE, "nothing_crossed", "nothing of the key crossed the face")
        return Answer(UNOBSERVABLE, "not_read", "nothing of the key was read: silence is no verdict (core O5)")
    if r.previous_instance:
        return Answer(NOT_ASKED, "previous_instance", "an earlier instance's last word, shown as last-known")
    # 6. Its freshness, at 900 s (§2.7).
    v = fr.judge("state", horizon, obs)
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
              seed: Seed | None, window: Window | None, ttl_s: float = TTL_S) -> Verdict:
    """§2.15 "The order" (0.2, F-130): §2.14's steps 1 and 2 with their
    reasons; the seed (`no_seed`, `seed_incomplete`); the window
    (`window_lost`, then `owner_gap`, which "across a face … is not asked
    there"); `answered`; across a face, an empty GET (`across_face`);
    `no_member` and `window_short`; then `window`. ``window`` is over §2.15's
    span (from its declaration, or after a loss from the first complete GET
    that followed it), as the caller measured it."""
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
    if window is not None and not across and not window.present_throughout:
        return Verdict(UNOBSERVABLE, "owner_gap")
    if seed.members > 0:
        return Verdict(ESTABLISHED, "answered")
    if across:
        return Verdict(UNOBSERVABLE, "across_face")
    if window is None:
        return Verdict(UNOBSERVABLE, "no_member")
    if window.listened_s <= ttl_s:
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


def any_firing(at_least: str, owners: Iterable[OwnerRead], scope_whole: bool = True) -> dict[str, Any]:
    """§2.15 "Anything firing at or above a severity *S*", its six steps in
    order (0.5), with the count of alarms reading yes at *S* or above, and
    the owners counted by verdict. Which owners are in scope is the
    caller's, and ``scope_whole`` whether the reads that found them are
    whole (§2.15 "Whole", 0.5): the latest presence read complete, and no
    token move lost since; an archive's read plays no part in it."""
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
    if not scope_whole:
        # "an owner they missed is in no other step, and may fire"
        return out(UNOBSERVABLE, "scope_incomplete")
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


# -- §2.16 reporting changes (0.4) -------------------------------------------------------

@dataclass(frozen=True)
class Report:
    """One report of a tool that reports each alarm's changes (§2.16): the
    key (None for the scope's loss), §2.14's answer after the change and its
    reason, the document's severity while yes, and the change that moved it
    (§2.6's transition, or the reader's own), which is never reported
    alone."""

    key: str | None
    answer: str
    reason: str
    severity: str | None = None
    change: str | None = None


#: §2.16 (0.7, 0.8), "A key's first answer": neither "splits by its reason,
#: in two closed lists". The reader's own sight, "which time or the next
#: read settles", is these ten, `silent` among them since 0.8 (F-141);
#: "any other reason … MAY be reported". zk2py's reporter reports no first
#: neither, so it needs neither list; the readings family checks that every
#: neither reason its judge gives is on exactly one.
READER_SIGHT = frozenset({"incomplete", "presence_incomplete", "no_descriptor", "not_read", "nothing_crossed",
                          "silent", "clock_untrusted", "listened_too_short", "near_horizon", "no_observation"})


def first_neither_list(reason: str) -> str:
    """Which of §2.16's two closed lists a first neither's reason is on."""
    return "reader_sight" if reason in READER_SIGHT else "other"


def _side(answer: str) -> str:
    """§2.16: "the answer moving between yes, no and neither"."""
    return answer if answer in (YES, NO) else "neither"


class ChangeReporter:
    """§2.16 (0.4, 0.6), "Reporting changes", over one reader's readings.
    Each :meth:`update` takes keys' first-question answers now, the latest
    document's severity, and the change that led there, and returns what to
    report. Every answer is compared with the key's **last report**, the
    one a reader of the reports holds, never with an answer read and not
    reported:
    - **the answer after the change**, never the change's name alone;
    - **a change** is the answer moving from the last report between yes,
      no and neither, or, while yes, the severity moving, either way. A
      refresh, a confirmation, and a raise of an alarm already reported
      firing at its severity are none;
    - **a key's first answer** (0.6), at the first reading or after it,
      "moves from no report": yes is reported (the SHOULD); no is not, there
      being no report to clear; neither is not, whether the reader's own
      sight makes it so, which "is no answer to report", or the key's own,
      which "MAY be reported" and which zk2py does not (the other branch of
      the MAY). A key with no report keeps none until it reads yes;
    - **a loss** (a key reading §2.14 step 4's ``incomplete``) is reported
      once for the scope, the MAY taken: one report when the first key goes
      lossy, none for a second loss while any key is unhealed, and one,
      "observable again", when the last lossy key heals. No key the loss
      made unobservable is reported; "each key then keeps its last report
      through the loss", and its answer once healed "is a change to report
      only if it moved from that last report" (0.6).

    Scope-level reports carry no key: ``unobservable``/``incomplete`` for a
    loss, ``observable``/``healed`` when it heals. With ``partial`` the
    readings are some of the scope's keys (one owner's, say), and keys left
    out of them keep their state. Without it they are the whole scope, and a
    key whose last report was yes or no that is not among them is never
    dropped silently: it is reported once, ``unobservable``/``not_read``,
    then forgotten."""

    def __init__(self) -> None:
        #: each key's last report: (yes | no | neither, severity while yes)
        self.reported: dict[str, tuple[str, str | None]] = {}
        #: every key read, reported or not
        self.known: set[str] = set()
        self.lossy: set[str] = set()
        self.loss_reported = False

    def update(self, readings: dict[str, tuple[Answer, str | None, str | None]],
               partial: bool = False) -> list[Report]:
        out: list[Report] = []
        for key, (a, _, _) in readings.items():
            self.known.add(key)
            if a.reason == "incomplete":
                self.lossy.add(key)
            else:
                self.lossy.discard(key)
        gone = [] if partial else sorted(self.known - set(readings))
        for key in gone:
            self.lossy.discard(key)
            self.known.discard(key)
        if self.lossy and not self.loss_reported:
            out.append(Report(None, UNOBSERVABLE, "incomplete", None, "loss"))
            self.loss_reported = True
        for key in gone:
            # §2.16: a key with no report has nothing to clear, and is
            # forgotten: a reappearance is a first answer, which "moves from
            # no report". A key with a report keeps it, so a reappearance is
            # compared with it: one last reported yes or no is reported
            # gone (neither) and keeps that report, so a later no clears it.
            last = self.reported.get(key)
            if last is not None and last[0] != "neither":
                self.reported[key] = ("neither", None)
                out.append(Report(key, UNOBSERVABLE, "not_read", None, "gone"))
        for key in sorted(readings):
            a, severity, change = readings[key]
            if key in self.lossy:
                continue  # the loss is reported once; the key keeps its last report
            side = _side(a.answer)
            now = (side, severity if side == YES else None)
            last = self.reported.get(key)
            if last is None:
                if side == YES:
                    self.reported[key] = now
                    out.append(Report(key, a.answer, a.reason, severity, change or "first answer"))
                continue
            if now != last:
                self.reported[key] = now
                out.append(Report(key, a.answer, a.reason, now[1], change))
        if self.loss_reported and not self.lossy:
            out.append(Report(None, "observable", "healed", None, "healed"))
            self.loss_reported = False
        return out


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


# == the runtime half: an owner, and a reader on a bus =================================

REPO = Path(__file__).resolve().parents[3]
CONTRACT = REPO / "spec" / "profiles" / "alarms" / "alarms.v1.toml"


def load(ttl_s: int | None = None) -> Any:
    """The standard contract (§3), or a copy of it with ``freshness.ttl_s =
    ttl_s``, read in memory at the standard contract's path so its schema
    resolves, and never published: scenarios.md's conventions (0.2) for a
    run that cannot wait 900 s. The copy is another revision, with a
    fingerprint of its own, which an owner serves like any contract."""
    from .contract import load_contract

    if ttl_s is None:
        c = load_contract(CONTRACT)
    else:
        text = CONTRACT.read_text(encoding="utf-8")
        old = f'"freshness.ttl_s" = {TTL_S}'
        if text.count(old) != 1:
            raise ValueError(f"{CONTRACT}: {old} not found once")
        c = load_contract(CONTRACT, text=text.replace(old, f'"freshness.ttl_s" = {int(ttl_s)}'))
    if not c.valid:
        raise ValueError(f"{CONTRACT}: {c.codes}")
    return c


def horizon_of(contract: Any) -> fr.Horizon:
    """The horizon of the contract's one resource."""
    r = next(r for r in contract.canonical["resources"] if r["template"] == TEMPLATE)
    return fr.horizon("state", r["annotations"])


def document_bytes(doc: dict[str, Any]) -> bytes:
    """An owner's put, as strict JSON (§2.4, 0.2): UTF-8, members sorted."""
    return json.dumps(doc, separators=(",", ":"), sort_keys=True, ensure_ascii=False).encode("utf-8")


class Record:
    """§2.9 "What the record holds" (0.2, a SHOULD): each alarm's latest
    document as put, by the key it was put on, a resolved one included until
    its delete. In memory, or a JSON file (``path``) saved after every put
    and delete: ``{"profile": "alarms.v1", "alarms": {<key>: <document>}}``.
    The format is zk2py's own, as the text leaves it."""

    def __init__(self, path: Path | None = None) -> None:
        self.path = path
        self.docs: dict[str, bytes] = {}
        if path is not None and path.is_file():
            data = json.loads(path.read_text(encoding="utf-8"))
            self.docs = {k: document_bytes(v) if isinstance(v, dict) else v.encode("utf-8")
                         for k, v in data.get("alarms", {}).items()}

    def put(self, key: str, payload: bytes) -> None:
        self.docs[key] = payload
        self._save()

    def delete(self, key: str) -> None:
        self.docs.pop(key, None)
        self._save()

    def items(self) -> list[tuple[str, bytes]]:
        return sorted(self.docs.items())

    def _save(self) -> None:
        if self.path is None:
            return
        out: dict[str, Any] = {}
        for k, v in self.docs.items():
            try:
                out[k] = json.loads(v)
            except ValueError:
                out[k] = v.decode("utf-8", "replace")
        self.path.write_text(json.dumps({"profile": PROFILE, "alarms": out}, sort_keys=True), encoding="utf-8")


class AlarmsOwner:
    """A service's ``alarms.v1`` (§2.5, §2.7 to §2.9), over
    :class:`zk2py.owner.Owner`: ``owner`` is the service, built with the
    alarms contract (the standard one, or ``contract``) and any others.
    - Each act puts the whole document on its one key (§2.5). A new
      `timestamp` is the clock's instant, or the previous one plus 1 where
      the clock has not passed it. A duplicate observation puts nothing.
    - Confirmations are the owner's freshness re-puts (§2.7). An alarm whose
      condition it no longer evaluates is not confirmed
      (:meth:`stop_evaluating`), until an act on it.
    - The bound is the owner's (§2.8): a raise beyond it raises
      :class:`zk2py.owner.BudgetExceeded`, and resolves or deletes nothing.
    - At start, it adopts from ``record`` (§2.9) before the tokens, in 0.2's
      order: resolved, deleted; undecodable or not re-deriving, deleted;
      firing under a rule outside ``rules``, resolved; then the rest
      adopted, up to the bound, unchanged.
    - A clean close leaves its alarms (§2.9)."""

    def __init__(self, system: str, service: str, *, contract: Any = None, contracts: list[Any] | None = None,
                 record: Record | None = None, rules: set[str] | None = None, now_ms: Any = None,
                 **owner_kw: Any) -> None:
        from .owner import Owner

        self.contract = contract or load()
        self.record = record if record is not None else Record()
        self.rules = rules
        self.now_ms = now_ms or (lambda: time.time_ns() // 1_000_000)
        self.held: dict[str, Alarm] = {}
        #: the latest transition's `timestamp` on each key, as far as it knows
        self.last_ts: dict[str, int] = {}
        #: §2.9: what the start did with each document of the record
        self.adoption: list[tuple[str, str]] = []
        self.owner = Owner(system, service, [self.contract, *(contracts or [])],
                           before_tokens=self._adopt, **owner_kw)

    # keys
    def key(self, alert_key: str) -> str:
        k = key_of(self.owner.system, self.owner.service, alert_key)
        if k is None:
            raise ValueError(f"{self.owner.system}/{self.owner.service}: no name (core §1.2)")
        return k

    def key_for(self, rule: str, labels: dict[str, str] | None = None) -> str:
        return self.key(derive(rule, labels).key)

    def start(self) -> None:
        self.owner.start()

    def close(self) -> None:
        """§2.9 "A clean close leaves them": the tokens go, the alarms stay."""
        self.owner.close()

    # puts
    def _put(self, key: str, doc: dict[str, Any]) -> Alarm:
        payload = document_bytes({k: v for k, v in doc.items() if v is not None})
        a = decode(payload)
        assert a is not None, doc
        self.owner.set_state(key, payload)
        self.held[key] = a
        self.last_ts[key] = max(self.last_ts.get(key, a.timestamp), a.timestamp)
        self.record.put(key, payload)
        return a

    def _delete(self, key: str) -> None:
        self.owner.delete_state(key)
        self.held.pop(key, None)
        self.record.delete(key)

    def _new_ts(self, key: str) -> int:
        now, last = self.now_ms(), self.last_ts.get(key)
        return now if last is None or now > last else last + 1

    @staticmethod
    def _doc(a: Alarm, **change: Any) -> dict[str, Any]:
        doc: dict[str, Any] = {"timestamp": a.timestamp, "rule": a.rule, "severity": a.severity,
                               "state": a.state, "summary": a.summary}
        if a.observed_at_ms is not None:
            doc["observed_at_ms"] = a.observed_at_ms
        if a.category is not None:
            doc["category"] = a.category
        if a.labels:
            doc["labels"] = dict(a.labels)
        doc.update(change)
        return {k: v for k, v in doc.items() if v is not None}

    # §2.5's acts
    def raise_alarm(self, rule: str, labels: dict[str, str] | None, severity: str, summary: str,
                    category: str | None = None) -> str:
        """The condition found: a raise, an escalation, a refresh, or, for a
        duplicate observation, ``none`` (0.2: "the owner puts nothing").
        Returns the act."""
        if severity not in SEVERITIES:
            raise ValueError(f"severity {severity!r}")
        key = self.key_for(rule, labels)
        held = self.held.get(key)
        base = {"rule": rule, "severity": severity, "state": FIRING, "summary": summary,
                "category": category, "labels": dict(labels) if labels else None}
        if held is None or not held.firing:
            self._put(key, {**base, "timestamp": self._new_ts(key)})
            return RAISED
        if held.severity != severity:
            self._put(key, {**base, "timestamp": self._new_ts(key)})
            return ESCALATED
        if (held.summary, held.category, held.labels) == (summary, category, dict(labels or {})):
            # §2.5 (0.2): no act; §2.7 (0.2): "An evaluation that finds the
            # condition again confirms it again", so confirmations stopped
            # by stop_evaluating resume.
            self.owner.reopen_writer(key)
            return "none"
        observed = max(self.now_ms(), held.timestamp)
        self._put(key, {**base, "timestamp": held.timestamp, "observed_at_ms": observed})
        return REFRESHED

    def resolve(self, rule: str, labels: dict[str, str] | None = None) -> bool:
        """§2.5: a resolved put, a new `timestamp` and no `observed_at_ms`,
        then the delete, at once. False when nothing firing is held."""
        key = self.key_for(rule, labels)
        held = self.held.get(key)
        if held is None or not held.firing:
            return False
        self._resolve_key(key, held)
        return True

    def _resolve_key(self, key: str, held: Alarm) -> None:
        self._put(key, self._doc(held, state=RESOLVED, observed_at_ms=None, timestamp=self._new_ts(key)))
        self.owner.close_writer(key)  # §2.7: a resolved alarm is never confirmed
        self._delete(key)

    def stop_evaluating(self, rule: str, labels: dict[str, str] | None = None) -> None:
        """§2.7: "MUST NOT confirm an alarm whose condition it has stopped
        evaluating": the owner's re-puts of it stop; it keeps its tokens and
        answers its GETs."""
        self.owner.close_writer(self.key_for(rule, labels))

    def confirm(self, rule: str, labels: dict[str, str] | None = None) -> None:
        """§2.7 (0.2): "An evaluation that finds the condition again confirms
        it again": the document put again unchanged."""
        key = self.key_for(rule, labels)
        held = self.held[key]
        payload = self.record.docs.get(key) or document_bytes(self._doc(held))
        self.owner.set_state(key, payload)

    # §2.9
    def _adopt(self, owner: Any) -> None:
        from .owner import BudgetExceeded

        adopt: list[tuple[str, bytes, Alarm]] = []
        for key, payload in self.record.items():
            ref, a = parse_key(key), decode(payload)
            if a is not None and not a.firing:
                self.adoption.append((key, "deleted: resolved, its delete lost"))
                self._delete(key)
            elif a is None or ref is None or rederives(ref["alert_key"], a).answer != YES:
                self.adoption.append((key, "deleted: undecodable, or its key does not re-derive"))
                self._delete(key)
            elif self.rules is not None and a.rule not in self.rules:
                self.adoption.append((key, "resolved: its rule no longer evaluated"))
                self.held[key], self.last_ts[key] = a, a.timestamp
                self._resolve_key(key, a)
            else:
                adopt.append((key, payload, a))
        for key, payload, a in adopt:
            try:
                owner.set_state(key, payload)
            except BudgetExceeded:
                # 0.2: "adopts up to the bound, and neither resolves nor
                # deletes the rest".
                self.adoption.append((key, "not adopted: past the bound"))
                continue
            self.held[key], self.last_ts[key] = a, a.timestamp
            self.adoption.append((key, "adopted"))


# -- the reader on a bus (§2.13 to §2.15) ----------------------------------------------

@dataclass
class SeedGet:
    """One S4 GET of an owner's ``alarms.v1/state/alarms/*`` (§2.15): when it
    was asked (the reader's monotonic clock), whether it ran to its final
    reply with no error reply, and its replies by key."""

    asked_ns: int
    complete: bool
    replies: dict[str, Any] = field(default_factory=dict)

    @property
    def seed(self) -> Seed:
        return Seed(self.complete, sum(1 for r in self.replies.values() if not r.deleted))


def is_left_out(seed: SeedGet | None, established: bool, key: str, arrivals: Iterable[int]) -> bool:
    """§2.14 step 5 (0.5): "the GET that established its owner's alarms,
    either way (`answered` or `window`), left the key out: it answered the
    key with neither a value nor a `reply_del`, and nothing of the key
    arrived after that GET was asked". ``seed.replies`` holds every reply,
    a ``reply_del`` included, so a key answered by one is not left out (it
    reads ``deleted``). ``arrivals``: the key's deliveries, on the reader's
    monotonic clock."""
    return seed is not None and seed.complete and established and key not in seed.replies \
        and not any(t >= seed.asked_ns for t in arrivals)


def seed_get(tool: Any, address: str, timeout: float = 1.0) -> SeedGet:
    """S4's GET (target ``All``, consolidation ``Latest``) of the owner's
    alarms, recorded with the instant it was asked."""
    from . import live

    prefix = f"zk2/{address}/{PROFILE}/{TOKEN}/{PREFIX}/"
    asked = time.monotonic_ns()
    st = live.get_state(tool, prefix + "*", timeout=timeout)
    return SeedGet(asked, not st.errors, {r.key: r for r in st.replies if r.key.startswith(prefix)})


def window_span(start_ns: int, end_ns: int, losses: Iterable[int], gaps: Iterable[int],
                complete_gets: Iterable[int], lossless_path: bool = True) -> Window:
    """§2.15's window (0.3) from what a reader recorded, every instant on
    its monotonic clock: the window's first instant (the declaration plus
    the reply timeout), its end (the watch after it before the instant
    judged: observer §5.2's longer of one lease and ``wait_before_close``,
    plus the reply timeout), the instants each loss and each gap was seen, and the
    instants its complete GETs of the owner were asked. A loss or a gap
    "breaks the window", which "runs again from the first complete GET that
    followed" it: a GET asked strictly after the instant the break was
    seen. A stall is seen by the tick that ends it, so a GET asked inside
    it heals nothing. ``lossless_path``: the QoS and the reader's own queue
    (observer §5.2)."""
    gets = sorted(complete_gets)
    start, lossless, present = start_ns, lossless_path, True
    for kind, instants in (("loss", losses), ("gap", gaps)):
        for t in instants:
            after = next((g for g in gets if g > t), None)
            if after is None:
                if kind == "loss":
                    lossless = False
                else:
                    present = False
            else:
                start = max(start, after)
    return Window(max(0, end_ns - start) / fr.NS, lossless, present)


def window_end_ns(at_ns: int, lease_s: float = 10.0, wait_before_close_s: float = 5.0,
                  reply_timeout_s: float = 1.0) -> int:
    """Where §2.15's window ends for a reader judging at ``at_ns``: "The
    owner is watched past the end as observer §5.2 says: the longer of one
    lease and the routers' `wait_before_close`, plus the reply timeout. So a
    reader judging at an instant ends its window that long before it" (§2.15
    as read against observer.md 0.6, and text 0.9's own since round 87). The
    length is
    :func:`zk2py.budget.watch_after_s`. Under zenoh's defaults, a 10 s lease
    and a 5 s ``wait_before_close``, it is 11 s, as before."""
    return at_ns - int(bg.watch_after_s(lease_s, wait_before_close_s, reply_timeout_s) * fr.NS)


class _FixedStall:
    """A receive-path watch with no session (:meth:`AlarmsReader.offline`):
    the losses it was handed, and nothing to close."""

    def __init__(self, losses: Iterable[int] = ()) -> None:
        self._losses = sorted(losses)

    def losses(self) -> list[int]:
        return list(self._losses)

    def close(self) -> None:
        pass


class _FixedSubscription:
    """A subscription with no session (:meth:`AlarmsReader.offline`): the
    instant it was declared, and whether it lost nothing."""

    def __init__(self, declared_ns: int, complete: bool = True) -> None:
        self.declared_ns, self.complete = declared_ns, complete

    def close(self) -> None:
        pass


class AlarmsReader:
    """A live reader of ``alarms.v1`` across owners (§2.13 to §2.15, text
    0.3): one subscription; its own receive path watched; each owner's
    instance tokens watched; its S4 GETs recorded. :meth:`read` gives one
    owner's reading at an instant, its window as §2.15 counts one:
    - **its span** counts as observer §5.2 counts a window's, "from the
      declaration plus the reply timeout, or from that GET, to its end. The
      owner is watched past the end as observer §5.2 says: the longer of one
      lease and the routers' `wait_before_close`, plus the reply timeout. So
      a reader judging at an instant ends its window that long before it"
      (§2.15 as read against observer.md 0.6, #935; text 0.9's own since
      round 87, F-326). That length is
      :func:`zk2py.budget.watch_after_s`, the one zk2py's budget windows
      watch (F-320, reading A). Before round 86 this reader watched one
      lease and the reply timeout, as 0.8's §2.15 then said, whatever
      ``wait_before_close`` was;
    - **a loss** is a stall of the reader's receive path, or a transport of
      its session closing (observer §5.2, 0.27), "a loss at the instant the
      reader sees it" (:class:`zk2py.budget.StallWatch`);
    - **a gap in the owner's presence** is "its instance token withdrawn,
      or a new instance's appearing", an owner joining included;
    - each "breaks the window", which "runs again from the first complete
      GET that followed" it. Until one, a loss is ``window_lost``, a gap
      ``owner_gap``. zk2py dates a GET by the instant it was asked
      (SPEC-FINDINGS, "At alarms.v1 0.3").
    - **lossless** also needs the resource's QoS, ``reliable`` with
      ``block`` (observer §5.2, 0.25), the standard contract's.
    The owners in scope are those :meth:`add_owner` was given: found by
    presence, or known before (§2.15, 0.2)."""

    def __init__(self, session: Any, selector: str, addresses: Iterable[str] = (), *,
                 trust: fr.ClockTrust | None = None, lease_s: float = 10.0, reply_timeout_s: float = 1.0,
                 wait_before_close_s: float = 5.0, qos: tuple[str, str] = ("reliable", "block"),
                 timeout: float = 1.0, stall: Any = None, subscriber: Any = None) -> None:
        """``stall`` and ``subscriber``, when given, stand in for the
        receive-path watch and the subscription built on ``session``
        (:meth:`offline`)."""
        self.session, self.qos, self.timeout = session, qos, timeout
        self.trust = trust or fr.ClockTrust(word=True)
        self.reply_timeout_ns = int(reply_timeout_s * fr.NS)
        #: how long the owner is watched past a window's end (§2.15,
        #: observer §5.2): the longer of the lease and wait_before_close,
        #: plus the reply timeout (:func:`window_end_ns`)
        self.watch = (lease_s, wait_before_close_s, reply_timeout_s)
        self.watch_after_ns = int(bg.watch_after_s(*self.watch) * fr.NS)
        self.stall = stall if stall is not None else \
            bg.StallWatch(session, bg.stall_bound(int(wait_before_close_s * 1000), int(lease_s * 1000)))
        self.subscriber = subscriber if subscriber is not None else fr.Subscriber(session, selector, self.trust)
        self.lock = threading.Lock()
        #: per owner: [(instant, "delete" | instance id put)]
        self.token_events: dict[str, list[tuple[int, str]]] = {}
        self.held_at_start: dict[str, str | None] = {}
        #: owners whose presence at the start a complete read did not show:
        #: the instant that read ended
        self.unknown_start: dict[str, int] = {}
        self.subs: list[Any] = []
        self.seeds: dict[str, list[SeedGet]] = {}
        #: losses a test notes by hand (``note_loss``): the reader's own
        #: queue overflowing, which a zenoh-python callback never does
        self.noted: list[int] = []
        for a in addresses:
            self.add_owner(a)

    @classmethod
    def offline(cls, declared_ns: int, held_at_start: dict[str, str | None], *,
                token_events: dict[str, list[tuple[int, str]]] | None = None,
                seeds: dict[str, list[SeedGet]] | None = None, losses: Iterable[int] = (),
                complete: bool = True, lease_s: float = 10.0, reply_timeout_s: float = 1.0,
                wait_before_close_s: float = 5.0,
                qos: tuple[str, str] = ("reliable", "block")) -> "AlarmsReader":
        """A reader with no session, holding what a run would have recorded,
        for a reading of :meth:`window` (``zk2py.conformance``). Every
        attribute is :meth:`__init__`'s, so one added there is here too.
        :meth:`window` reads, beside the watch's lengths and ``qos``:
        - the subscription's declaration and whether it lost nothing
          (``declared_ns``, ``complete``);
        - the losses its receive path saw (``losses``);
        - each owner's instance held at the window's start and its token
          moves since (``held_at_start``, ``token_events``);
        - the S4 GETs asked of each owner (``seeds``).

        It opens nothing, so :meth:`add_owner`, :meth:`seed`, :meth:`read`
        and the scope's reads, which need a session, are not for it."""
        r = cls(None, "", (), lease_s=lease_s, reply_timeout_s=reply_timeout_s,
                wait_before_close_s=wait_before_close_s, qos=qos, stall=_FixedStall(losses),
                subscriber=_FixedSubscription(declared_ns, complete))
        r.held_at_start = dict(held_at_start)
        r.token_events = {a: list(e) for a, e in (token_events or {}).items()}
        r.seeds = {a: list(g) for a, g in (seeds or {}).items()}
        return r

    @property
    def declared_ns(self) -> int:
        return self.subscriber.declared_ns

    def add_owner(self, address: str) -> None:
        import zenoh

        from . import live

        with self.lock:
            if address in self.token_events:
                return
            self.token_events[address] = []

        def got(smp: Any, a: str = address) -> None:
            k = str(smp.key_expr)
            ev = "delete" if smp.kind == zenoh.SampleKind.DELETE else k.rsplit("/", 1)[1]
            with self.lock:
                self.token_events[a].append((time.monotonic_ns(), ev))

        # Without history: a token held before the subscription delivers no
        # put, so only a token that appears after it is an event. Then the
        # read of what is held, recorded under the lock gaps() reads it by.
        # Observer §3.7 (0.37): the subscriber on the reader's watch session, one
        # round trip there, and the read on its own session.
        sub, placed = live.watch_or_beside(self.session, f"zk2/{address}/@zk/instance/*",
                                           zenoh.handlers.Callback(got))
        self.subs.append(sub)
        pres = live.list_presence(self.session, f"zk2/{address}/@zk/instance/*")
        with self.lock:
            self.held_at_start[address] = pres.instances[0]["instance"] if pres.instances else None
            # A round trip on the watch session that did not come back leaves
            # the read unordered against the subscriber: not shown complete.
            if not pres.complete or placed.settled is False:
                # §2.15: present throughout is "held its instance token from
                # the window's start". A read possibly incomplete does not
                # show it: owner_gap until a complete GET after this read.
                self.unknown_start[address] = time.monotonic_ns()

    def note_loss(self) -> None:
        with self.lock:
            self.noted.append(time.monotonic_ns())

    def seed(self, address: str) -> SeedGet:
        g = seed_get(self.session, address, self.timeout)
        self.seeds.setdefault(address, []).append(g)
        return g

    def gaps(self, address: str) -> list[int]:
        """The instants of a gap: a withdrawn instance token, or an instance
        appearing that is not the one held (an owner joining included); and
        the end of a start-of-window presence read that did not show the
        owner held (incomplete), or of an :meth:`add_owner` not finished."""
        with self.lock:
            events = list(self.token_events.get(address, []))
            if address not in self.held_at_start:
                return [time.monotonic_ns()]
            held = self.held_at_start[address]
            unknown = self.unknown_start.get(address)
        out = [] if unknown is None else [unknown]
        for t, ev in events:
            if ev == "delete":
                held = None
                out.append(t)
            elif ev != held:
                # a new instance, or an owner joining while the window
                # listened: "a new instance or a gap" (observer §5.2)
                out.append(t)
                held = ev
        return out

    def losses(self) -> list[int]:
        with self.lock:
            noted = list(self.noted)
        return sorted(self.stall.losses() + noted)

    def window(self, address: str, at_ns: int | None = None) -> Window:
        """§2.15's window for ``address``, judged at ``at_ns`` (now)."""
        now = time.monotonic_ns() if at_ns is None else at_ns
        lossless_qos, _ = bg.lossless(*self.qos)
        return window_span(self.declared_ns + self.reply_timeout_ns, window_end_ns(now, *self.watch),
                           [t for t in self.losses() if t <= now], [t for t in self.gaps(address) if t <= now],
                           [g.asked_ns for g in self.seeds.get(address, []) if g.complete and g.asked_ns <= now],
                           lossless_qos and self.subscriber.complete)

    def read(self, address: str, *, get: bool = True, horizon: fr.Horizon = HORIZON,
             keys_known: Iterable[str] = ()) -> OwnerReading:
        """One reading of ``address`` now: a new S4 GET first when ``get``
        (the reader re-seeds at each re-read), else its latest one."""
        self.add_owner(address)
        if get:
            self.seed(address)
        seeds = self.seeds.get(address) or []
        losses = self.losses()
        return read_owner(self.session, address, subscriber=self.subscriber, window=self.window(address),
                          seed=seeds[-1] if seeds else None, trust=self.trust, get=False, horizon=horizon,
                          keys_known=keys_known, timeout=self.timeout, lost_at=losses[-1] if losses else None)

    def scope_read(self, systems: str = "*", services: str = "*") -> ScopeRead:
        """§2.15 "Whole" (0.5, 0.6): a presence read of the scope now, dated
        by the instant it was asked. Whether it is whole is
        :meth:`scope_whole`'s, at the instant the second question is asked."""
        sr = owners_by_presence(self.session, systems, services)
        for a in sr.owners:
            self.add_owner(a)
        return sr

    def scope_whole(self, sr: ScopeRead) -> bool:
        """§2.15 "Whole": the presence read ran to its final reply, and "a
        loss heard at or after that instant counts against it, wherever its
        replies arrive" (0.6)."""
        return presence_read_whole(sr.whole, sr.asked_ns, self.losses())

    def close(self) -> None:
        self.stall.close()
        self.subscriber.close()
        for x in self.subs:
            try:
                x.undeclare()
            except Exception:  # noqa: BLE001 - closing anyway
                pass


def archive_form(archive: str, selector: str) -> str:
    """Core §4.4: "A pattern over its keys is formed from an origin selector
    the same way, chunk by chunk: the leading zk2 is dropped, * and ** stay
    as written, and every other chunk is slugged", under the archive's
    ``archive.v1/@state``."""
    chunks = selector.split("/")
    if chunks[0] != "zk2":
        raise ValueError(f"{selector}: not a zk2 selector")
    body = [c if c in ("*", "**") else slug.slug(c) for c in chunks[1:]]
    return f"zk2/{archive}/archive.v1/@state/" + "/".join(body)


@dataclass
class ScopeRead:
    """The owners a tool found for a scope (§2.15, "the owners in scope"),
    and whether the reads that found them are **whole** (0.5): "the
    reader's latest presence read of them ran to its final reply … and, for
    a reader that follows instance tokens since, it lost none of their
    moves". "An archive's read plays no part in it": an archive's GET short
    of its final reply, or a key that does not read back (``skipped``), can
    leave out only an absent owner, not asked, so it only sets
    ``archive_short``, which a tool says."""

    owners: list[str]
    whole: bool = True
    archive_short: bool = False
    skipped: int = 0
    #: the presence read's ask, on the reader's monotonic clock (0.6: "a
    #: presence read is dated as a GET is, by the instant it was asked")
    asked_ns: int | None = None

    def union(self, other: "ScopeRead") -> "ScopeRead":
        asks = [x for x in (self.asked_ns, other.asked_ns) if x is not None]
        return ScopeRead(sorted(set(self.owners) | set(other.owners)), self.whole and other.whole,
                         self.archive_short or other.archive_short, self.skipped + other.skipped,
                         min(asks) if asks else None)


def presence_read_whole(complete: bool, asked_ns: int | None, losses: Iterable[int]) -> bool:
    """§2.15 "Whole" (0.6): a presence read that ran to its final reply,
    "dated as a GET is, by the instant it was asked: a loss heard at or
    after that instant counts against it, wherever its replies arrive". A
    read with no ask on record cannot be dated, so it is not whole."""
    return complete and asked_ns is not None and not any(t >= asked_ns for t in losses)


def owners_by_presence(tool: Any, systems: str = "*", services: str = "*") -> ScopeRead:
    """§2.13: the owners a presence read finds, by their instance tokens;
    whether ``alarms.v1`` is theirs is each descriptor's (§2.14 step 2)."""
    from . import live

    asked = time.monotonic_ns()
    pres = live.list_presence(tool, f"zk2/{systems}/{services}/@zk/**")
    return ScopeRead(sorted({f"{p['system']}/{p['service']}" for p in pres.instances}), whole=pres.complete,
                     asked_ns=asked)


def owners_from_archive(tool: Any, archive: str, systems: str = "*", services: str = "*",
                        timeout: float = 1.0) -> ScopeRead:
    """§2.15 (0.2, 0.3): the owners a tool "knew of before … by an
    archive's keys (core S6)": a GET of the archive's form of
    ``zk2/<systems>/<services>/alarms.v1/state/alarms/*``, each key read
    back (every chunk unslugged) into the alarm key it archives, and that
    key's address. A tool whose readings end with its process knows of an
    absent owner this way (scenarios.md §8, 0.3). A GET that did not run to
    its final reply, or a key that does not read back, sets
    ``archive_short``, and leaves the scope whole (0.5)."""
    from . import live

    pattern = archive_form(archive, f"zk2/{systems}/{services}/{PROFILE}/{TOKEN}/{PREFIX}/*")
    head = f"zk2/{archive}/archive.v1/@state/"
    st = live.get_state(tool, pattern, timeout=timeout)
    out: set[str] = set()
    skipped = 0
    for r in st.replies:
        values = [slug.unslug(c) for c in r.key[len(head):].split("/")] if r.key.startswith(head) else [None]
        chunks = [v for v in values if v is not None]
        ref = parse_key("zk2/" + "/".join(chunks)) if len(chunks) == len(values) else None
        if ref is None:
            skipped += 1
            continue
        out.add(f"{ref['system']}/{ref['service']}")
    return ScopeRead(sorted(out), whole=True, archive_short=bool(st.errors) or skipped > 0, skipped=skipped)


def _zid(text: Any) -> int | None:
    """Observer §4.4 (0.11): a zid compares by value."""
    try:
        return int(str(text), 16)
    except (TypeError, ValueError):
        return None


@dataclass
class OwnerReading:
    """One reading of one owner's alarms (§2.14, §2.15): its presence and
    descriptor, the S4 GET, the window, the verdict on its alarms, and one
    :class:`Reading` per alarm key the reader knows of, with its latest
    document (None for a delete, or a put that does not read)."""

    address: str
    presence: str
    descriptor: dict[str, Any] | None
    meta_zid: str | None
    seed: Seed | None
    window: Window | None
    verdict: Verdict
    readings: dict[str, Reading] = field(default_factory=dict)
    documents: dict[str, Alarm | None] = field(default_factory=dict)
    stamps: dict[str, str | None] = field(default_factory=dict)
    #: §2.14 step 5 (0.2): GET replies whose stamp's id is not ``meta.zid``,
    #: by value: observer §4.4's foreign stamp, "a finding a tool reports
    #: apart". After §2.9's re-put, no reply of the present instance carries
    #: an earlier one's stamp.
    foreign: list[str] = field(default_factory=list)
    #: observer §4.1 (0.11): "A tool that bounds what it holds counts the
    #: keys the bound forgot, and reports the count": the keys the
    #: subscription's recording forgot, and its note (round 92). zk2py marks
    #: neither a forgotten key nor its next value.
    forgotten: int = 0
    bound: str = ""

    def answers(self, horizon: fr.Horizon = HORIZON) -> dict[str, tuple[Answer, Answer]]:
        return {k: (judge(r, Q_FIRING, horizon), judge(r, Q_STALE, horizon)) for k, r in self.readings.items()}

    def owner_read(self, horizon: fr.Horizon = HORIZON) -> OwnerRead:
        """For :func:`any_firing`: the verdict, and each alarm's first
        answer with its latest document's severity."""
        return OwnerRead(self.verdict.verdict, tuple(
            AlarmRead(judge(r, Q_FIRING, horizon).answer,
                      None if self.documents.get(k) is None else self.documents[k].severity)
            for k, r in self.readings.items()))


def read_owner(tool: Any, address: str, *, subscriber: fr.Subscriber | None = None,
               window: Window | None = None, seed: SeedGet | None = None, trust: fr.ClockTrust | None = None,
               get: bool = True, horizon: fr.Horizon = HORIZON, keys_known: Iterable[str] = (),
               timeout: float = 1.0, lost_at: int | None = None) -> OwnerReading:
    """One reading of ``address`` (``<system>/<service>``):
    - presence (observer §3) by a liveliness GET of its instance tokens, the
      descriptor through it (§2.13: never the interface token);
    - the S4 GET of ``…/alarms.v1/state/alarms/*``, aged against ``trust``;
    - ``seed``, a GET already made (:class:`AlarmsReader`'s), or, with
      ``get``, one made now;
    - the subscription ``subscriber`` made before it, for the key's
      observations, and its ``window`` (:class:`AlarmsReader`'s), for §2.15;
    - §2.14 step 4: with ``lost_at``, the last instant the reader saw its
      subscription lose, each key's subscription observation is a lossy
      one, and the GET is among the observations, the re-seed that heals
      a key it answers, only when it was asked after that instant;
    - §2.14 step 5 (0.3): a key the reader holds a sample of, which the
      complete GET that established the owner's alarms did not answer, with
      nothing of it arriving after that GET was asked, holds none;
    - each key the subscription or the GET knows, and ``keys_known`` (an
      earlier reading's): its observations, and its latest sample merged by
      the owner's stamp (core §4.2), `previous_instance` when that put's
      stamp id is not ``meta.zid`` by value (§2.14 step 5, 0.2)."""
    from . import live
    from . import seed as sd

    pres = live.list_presence(tool, f"zk2/{address}/@zk/**")
    presence = "present" if pres.instances else "absent" if pres.complete else "incomplete"
    descriptor, meta_zid = None, None
    if presence == "present":
        inst = pres.instances[0]["instance"]
        d = live.get_descriptor(tool, f"zk2/{address}/@zk/instance/{inst}", timeout=timeout)
        if len(d) == 1 and d[0].ok:
            doc = json.loads(d[0].payload)
            meta_zid = (doc.get("meta") or {}).get("zid")
            entry = next((e for e in doc.get("interfaces") or [] if e.get("iface") == PROFILE), None)
            descriptor = {"lists": False} if entry is None else {"lists": True, "token": entry.get("token", True)}
    prefix = f"zk2/{address}/{PROFILE}/{TOKEN}/{PREFIX}/"
    if seed is None and get:
        seed = seed_get(tool, address, timeout)
    replies = {} if seed is None else seed.replies
    verdict = owner_set(presence, descriptor, None, None if seed is None else seed.seed, window,
                        ttl_s=horizon.ttl_s or TTL_S)
    out = OwnerReading(address, presence, descriptor, meta_zid, None if seed is None else seed.seed, window,
                       verdict)
    if meta_zid is not None:
        out.foreign = sorted(k for k, r in replies.items() if _zid(r.stamp_id) != _zid(meta_zid))
    known = set(keys_known) | set(replies)
    if subscriber is not None:
        with subscriber.lock:
            known |= {k for k in subscriber.deliveries if k.startswith(prefix)}
        # observer §4.1 (0.11): the bound's count, reported with what it holds
        out.forgotten, out.bound = subscriber.forgotten, subscriber.note()
    trust = trust or fr.ClockTrust(word=True)
    for key in sorted(known):
        obs: list[Any] = []
        # (stamp, kind, payload, stamp id), merged by core §4.2 through
        # seed.newest: by time, then by id as Latest compares (round 91;
        # round 90 compared ids as integers)
        cands: list[tuple[Any, str, bytes, str | None]] = []
        if subscriber is not None:
            o = subscriber.observation(key)
            obs.append(o if lost_at is None else fr.Subscription(o.listened_ns, False, o.last))
            for x in subscriber.of(key):
                if x.stamp is not None:
                    cands.append((sd.Stamp.of(x.stamp), x.kind, x.payload, x.stamp_id))
        if seed is not None:
            r = replies.get(key)
            if r is None:
                g = fr.GetReading(None)
            else:
                g = fr.GetReading("delete" if r.deleted else "put", r.stamp_ns, r.stamp_id, r.stamp, r.payload)
                if r.stamp is not None:
                    cands.append((sd.Stamp.of(r.stamp), "delete" if r.deleted else "put", r.payload, r.stamp_id))
            if lost_at is None or seed.asked_ns >= lost_at:
                obs.append(g.observation(trust))
        latest, prev, doc, sid = None, False, None, None
        # §2.14 step 5 (0.3, 0.5): the establishing GET "left the key out: it
        # answered the key with neither a value nor a reply_del, and nothing
        # of the key arrived after that GET was asked". The judge reads it
        # (Reading.left_out), healing step 4 and making the sample none.
        left_out = is_left_out(seed, verdict.verdict == ESTABLISHED, key,
                               [] if subscriber is None else [x.arrival_ns for x in subscriber.of(key)])
        if cands:
            _, kind, payload, sid = sd.merged(cands)
            if kind == "delete":
                latest = "deleted"
            else:
                doc = decode(payload)
                latest = "undecodable" if doc is None else doc.state
                prev = meta_zid is not None and _zid(sid) != _zid(meta_zid)
        out.readings[key] = Reading(presence, descriptor, None, obs, latest, prev,
                                    verdict.verdict == ESTABLISHED, left_out)
        out.documents[key], out.stamps[key] = doc, sid
    return out

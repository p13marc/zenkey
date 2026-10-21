"""``telemetry.v1`` (``spec/profiles/telemetry/v1.md``, text 0.3, core 0.35):
what a metric resource carries, and what a reader may conclude from it.

From the text alone:
- **the standard schema** (§2.3, §3): its published artifact ids, and a type
  the profile's by its kind, its name and its artifact's id
  (:func:`is_profile_type`), never by its name alone;
- **a resource's declaration** (§2.1, §2.2, §2.4, §5's first question):
  :func:`declare`, ``declared`` with its carrier, kind, unit, buckets,
  semantic and where an in-band epoch rides, ``invalid`` with its reasons in
  the text's order, ``ignored`` (an event or an operation that writes a key)
  or ``not_this_profiles``;
- **a sample's conformance** (§2.5, §2.6): :func:`conform`, every reason in
  order, each check made alone, and the member's kind and unit as the sample
  gives them;
- **a pair** (§2.9 to §2.11): :func:`epoch_of`, :func:`continuity` and
  :func:`rate`, the rate by the text's IEEE formula;
- **a contract's resources** (:func:`contract_declarations`), a bundle's
  included (:func:`bundle_declarations`), each type's root read with its
  ``$ref``s followed as core §9.8 follows them.

The owner's half (:class:`MeterOwner`) and the reader on a bus
(:class:`Reader`) are below, for ``scenarios.md``; they run in
:mod:`zk2py.telemetry_scenarios`.

0.2 answered round 45's findings, F-142 to F-149 (``SPEC-FINDINGS.md``,
"At telemetry.v1 0.2"), and each answer is followed here:
- a JSON sample is strict JSON, every number finite, so ``1e400`` and
  10^400 written out are undecodable (F-143);
- a CBOR bignum, tag 2 or 3 around a byte string, is the integer it denotes;
  every other tag is its content (F-144);
- ``counts_length`` is checked whenever the value is a Histogram (F-145);
- ``incomparable`` is no reason for a missing rate: such a pair is not of
  two counters, ``kind`` (F-146);
- an Epoch attachment that does not decode is ``epoch_invalid`` (F-147);
- a state member's pairs follow a late joiner's merge (F-148);
- a member written ``null`` is present (F-149);
- a stamp's nanoseconds are NTP64's ``as_nanos``, the fraction rounded up,
  each stamp alone (F-142, :func:`ntp64_ns`).

0.3 answered round 46's two, as zk2py read them: "finite" is by rounding
(F-150), and an Epoch attachment is read as a sample of its encoding is,
strict JSON or CBOR, depth included (F-151). It adds a data item's depth,
at most 256 arrays and maps (:data:`MAX_DEPTH`), and §5's "over many pairs"
(:func:`verdict`).
"""

from __future__ import annotations

import base64
import json
import math
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Iterable

from . import jcs

PROFILE = "telemetry.v1"
REPO = Path(__file__).resolve().parents[3]
SCHEMA_PATH = REPO / "spec" / "profiles" / "telemetry" / "telemetry.v1.json"

#: §2.2: the six kinds, "compared byte for byte".
KINDS = ("counter", "gauge", "histogram", "text", "boolean", "binary")
#: §2.2: the closed eight.
SEMANTICS = ("temperature", "power", "bytes", "duration", "ratio", "count", "identity", "state")
#: §4: the vocabulary, which core 0.35's Appendix D publishes.
KIND, UNIT, BUCKETS, SEMANTIC = "telemetry.kind", "telemetry.unit", "telemetry.buckets", "telemetry.semantic"
KEYS = (KIND, UNIT, BUCKETS, SEMANTIC)

#: §2.2, §2.6: a counter's range, and a Point's timestamp's (ZenSight's i64).
U64_MAX = 2**64 - 1
I64_MIN, I64_MAX = -(2**63), 2**63 - 1

POINT, VALUE, HISTOGRAM, EPOCH = "telemetry.v1.Point", "telemetry.v1.Value", "telemetry.v1.Histogram", \
    "telemetry.v1.Epoch"
#: §3's table, oldest first: (artifact id, the revision that published it).
PUBLISHED: tuple[tuple[str, str], ...] = (
    ("sha256:3d6276dd302873ca5938013a44d460267971123fd25500f189e30be718f9dc1e", "zensight"),
    ("sha256:715ce537b2150beb3863f0e5cd356281357f7ec56d93625ee64e14f0045629e9", "0.1"),
)
PUBLISHED_IDS = tuple(i for i, _ in PUBLISHED)
CURRENT = PUBLISHED[-1][0]
#: §2.3: the four types, each with the first revision that defines it ("the
#: Epoch exists only from 0.1's", §3).
TYPES = {POINT: "zensight", VALUE: "zensight", HISTOGRAM: "zensight", EPOCH: "0.1"}

DECLARED, INVALID, IGNORED, NOT_THIS = "declared", "invalid", "ignored", "not_this_profiles"
CONFORMS, DOES_NOT, UNOBSERVABLE, NOT_ASKED = "conforms", "does_not", "unobservable", "not_asked"
#: §2.2's reasons, in the order a reader checks them.
DECLARATION_REASONS = ("kind_value", "unit_value", "buckets_value", "semantic_value", "kind_missing",
                       "buckets_without_histogram", "histogram_without_buckets", "not_a_carrier")
#: §2.6's reasons, in order.
SAMPLE_REASONS = ("not_a_point", "epoch_invalid", "unit_invalid", "tag_disagrees", "unit_disagrees",
                  "wrong_type", "out_of_range", "not_finite", "bounds_invalid", "bounds_differ",
                  "counts_length", "count_sum")

_MISSING = object()


# -- the standard schema (§2.3, §3) ------------------------------------------------------

def artifact_id(document: Any) -> str:
    """Core §7.1, §9.4: "the hash of the document's JCS bytes"."""
    return jcs.jcs_id(document)


def is_published(artifact: str | None) -> bool:
    """§3: an id this profile publishes; every revision's stays the
    profile's."""
    return artifact in PUBLISHED_IDS


def is_profile_type(t: Any, name: str) -> bool:
    """§2.3: "A type is one of the profile's when its kind is ``jsonschema``,
    its name is one of the four, and its artifact's id … is one this profile
    publishes", and the revision of that id defines it (§3)."""
    if not isinstance(t, dict) or t.get("kind") != "jsonschema" or t.get("name") != name:
        return False
    sid = t.get("schema")
    if sid not in PUBLISHED_IDS or name not in TYPES:
        return False
    since = [r for _, r in PUBLISHED].index(TYPES[name])
    return PUBLISHED_IDS.index(sid) >= since


def load_schema() -> Any:
    """The profile's file, the latest revision (§3), as published."""
    return jcs.loads(SCHEMA_PATH.read_bytes())


def _profile_root(name: str) -> Any:
    """A profile type's root in the latest file, its top-level ``$ref``
    followed (§2.4); every keyword a carrier reads is unchanged across the
    revisions (§3)."""
    doc = load_schema()
    node = doc["$defs"].get(name)
    seen = set()
    while isinstance(node, dict) and isinstance(node.get("$ref"), str) and node["$ref"] not in seen:
        seen.add(node["$ref"])
        ref = node["$ref"]
        if not ref.startswith("#/$defs/"):
            break
        target = doc["$defs"].get(ref.removeprefix("#/$defs/"))
        node = {**(target or {}), **{k: v for k, v in node.items() if k != "$ref"}}
    return node


# -- numbers and data items (§2.6 "How a value is read") ---------------------------------

def is_number(v: Any) -> bool:
    """A number of JSON's or CBOR's data model: an integer or a float, a
    non-finite float included (§2.6). A boolean is not one."""
    return isinstance(v, (int, float)) and not isinstance(v, bool)


def as_integer(v: Any) -> int | None:
    """§2.6: "An integer is read exactly … An integral double is the integer
    it equals, as JSON Schema's ``integer`` … reads it": ``5.0`` is 5, ``-0.0``
    is 0, and ``1e300`` is an integer. NaN and the infinities are none."""
    if isinstance(v, bool):
        return None
    if isinstance(v, int):
        return v
    if isinstance(v, float) and math.isfinite(v) and v.is_integer():
        return int(v)
    return None


def to_double(v: Any) -> float:
    """§2.2: "the IEEE 754 double nearest its value". An integer beyond the
    double range rounds, as IEEE 754's round-to-nearest does, to an
    infinity: a CBOR bignum can be one (a strict JSON sample cannot, 0.2)."""
    if isinstance(v, float):
        return v
    try:
        return float(v)
    except OverflowError:
        return math.inf if v > 0 else -math.inf


def is_finite(v: Any) -> bool:
    return is_number(v) and math.isfinite(to_double(v))


def is_unit(u: Any) -> bool:
    """§2.2: "a string of 1 to 64 characters, each a printable ASCII
    character from 0x21 to 0x7E, so no space and no control"."""
    return isinstance(u, str) and 1 <= len(u) <= 64 and all(0x21 <= ord(c) <= 0x7E for c in u)


def is_epoch(e: Any) -> bool:
    """§2.6 check 2: "a string of 1 to 64 characters, counted as Unicode code
    points, as JSON Schema counts them"."""
    return isinstance(e, str) and 1 <= len(e) <= 64


def bounds_valid(b: Any) -> bool:
    """§2.2: "a non-empty array of numbers, strictly ascending, each finite",
    compared as doubles: ``-0.0`` equals ``0``."""
    if not isinstance(b, list) or not b or not all(is_number(x) for x in b):
        return False
    d = [to_double(x) for x in b]
    return all(math.isfinite(x) for x in d) and all(a < c for a, c in zip(d, d[1:]))


def bounds_equal(a: Iterable[Any], b: Iterable[Any]) -> bool:
    """§2.2: "two bounds are compared as doubles"; another length differs."""
    x, y = [to_double(v) for v in a], [to_double(v) for v in b]
    return len(x) == len(y) and all(p == q for p, q in zip(x, y))


def from_fixture(x: Any) -> Any:
    """A data item as the fixtures write it: ``{"$int": "<decimal>"}``,
    ``{"$float": "NaN" | "Infinity" | "-Infinity"}``, ``{"$bytes":
    "<base64>"}``, each an object of one member, the rest JSON read by its
    text (an integer exactly, any other number as its double)."""
    if isinstance(x, dict):
        if len(x) == 1:
            (k, v), = x.items()
            if k == "$int" and isinstance(v, str):
                return int(v)
            if k == "$float" and v in ("NaN", "Infinity", "-Infinity"):
                return {"NaN": math.nan, "Infinity": math.inf, "-Infinity": -math.inf}[v]
            if k == "$bytes" and isinstance(v, str):
                return base64.b64decode(v, validate=True)
        return {k: from_fixture(v) for k, v in x.items()}
    if isinstance(x, list):
        return [from_fixture(v) for v in x]
    return x


class Undecodable(ValueError):
    """Bytes the core could not decode as its JSON or its CBOR (§2.6)."""


def _no_duplicates(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    out: dict[str, Any] = {}
    for k, v in pairs:
        if k in out:
            raise Undecodable(f"duplicate member {k!r}")
        out[k] = v
    return out


def _no_constant(name: str) -> Any:
    raise Undecodable(f"{name} is not JSON")


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


#: §2.6 "Depth" (0.3): "A data item nests at most 256 arrays and maps deep".
MAX_DEPTH = 256


def depth(v: Any) -> int:
    """§2.6 "Depth" (0.3): "an array or a map at the top is at depth 1, and
    an array or a map inside another is one deeper. A scalar adds no
    depth", and a CBOR tag, read as its content, adds none either. Counted
    without recursion."""
    deepest, stack = 0, [(v, 1)]
    while stack:
        node, d = stack.pop()
        if isinstance(node, (list, dict)):
            deepest = max(deepest, d)
            stack.extend((x, d + 1) for x in (node.values() if isinstance(node, dict) else node))
    return deepest


def _finite(v: Any) -> bool:
    """§2.6 "Finite" (0.3, F-150): "a number's text, an integer's included,
    is finite when its exact value, rounded to the nearest double (IEEE 754
    binary64, round to nearest, ties to even), is not an infinity". Python's
    ``float()`` rounds so, for a float's text as for an integer."""
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


def decode_json(payload: bytes) -> Any:
    """§2.6 "Strict JSON" (0.2, F-143): "UTF-8, RFC 8259's grammar with
    nothing after the value but whitespace, no duplicate member at any
    depth, every string with a UTF-8 form (so no lone surrogate escape),
    and every number finite", ``1e400`` and 10^400 written out included; an
    integer within the double's range "is read exactly, however long its
    text". Raises :class:`Undecodable`.

    §2.6 "Depth" (0.3): an array or a map at depth 257 is undecodable. A
    value nested past Python's recursion limit is too, never a crash."""
    try:
        text = payload.decode("utf-8")
    except UnicodeDecodeError as e:
        raise Undecodable(f"not UTF-8: {e}") from e
    try:
        value = json.loads(text.strip(" \t\r\n"), object_pairs_hook=_no_duplicates, parse_constant=_no_constant)
        if not _utf8(value):
            raise Undecodable("a string with no UTF-8 form")
        if not _finite(value):
            raise Undecodable("a number past the double's finite range")
        if depth(value) > MAX_DEPTH:
            raise Undecodable(f"nested deeper than {MAX_DEPTH} arrays and maps")
    except Undecodable:
        raise
    except RecursionError as e:
        raise Undecodable("nested past this reader's depth") from e
    except ValueError as e:
        raise Undecodable(str(e)) from e
    return value


def decode_cbor(payload: bytes) -> Any:
    """§2.6 "CBOR" (0.2, F-144): an integer is major type 0 or 1, or a
    bignum, tag 2 or 3 around a byte string of any length, definite or not,
    the integer it denotes; every other tag, and a tag 2 or 3 around
    anything but a byte string, is its content; a map's keys are text, none
    twice; ``undefined`` is ``null``; anything else does not decode. A
    non-finite float is kept as one. Raises :class:`Undecodable`."""
    from . import cbor

    def textual(v: Any) -> bool:
        if isinstance(v, dict):
            return all(isinstance(k, str) and textual(x) for k, x in v.items())
        if isinstance(v, list):
            return all(textual(x) for x in v)
        return True

    try:
        value = cbor.loads(payload, bignums=True)
        if not textual(value):
            raise Undecodable("a map key that is not text")
        if depth(value) > MAX_DEPTH:
            raise Undecodable(f"nested deeper than {MAX_DEPTH} arrays and maps")
    except Undecodable:
        raise
    except cbor.CborError as e:
        raise Undecodable(str(e)) from e
    except RecursionError as e:
        # As decode_json: nested past this reader's depth.
        raise Undecodable("nested past this reader's depth") from e
    return value


def decode_attachment(payload: bytes, encoding: str | None = "json") -> Any:
    """§2.6 check 2 (0.3, F-151): "An Epoch attachment is read as this
    section reads a sample of its encoding: strict JSON, or CBOR, depth
    included". Raises :class:`Undecodable`; the caller reads that as an
    epoch present and not a string."""
    return decode_cbor(payload) if encoding == "cbor" else decode_json(payload)


def decode(payload: bytes, type_: Any, encoding: str | None = "json") -> Any:
    """A sample's payload as its resource's type decodes it: bytes for a raw
    type, else JSON or CBOR by the resource's ``encoding`` (core §7.2)."""
    if isinstance(type_, dict) and type_.get("kind") == "raw":
        return bytes(payload)
    return decode_cbor(payload) if encoding == "cbor" else decode_json(payload)


# -- the declaration (§2.1, §2.2, §2.4) --------------------------------------------------

@dataclass(frozen=True)
class Declaration:
    """§5's first question: one of ``declared``, ``invalid``, ``ignored``
    and ``not_this_profiles``."""

    declaration: str
    reasons: tuple[str, ...] = ()
    carrier: str | None = None        # "point" or "bare"
    kind: str | None = None
    unit: str | None = None
    buckets: tuple[float, ...] | None = None
    semantic: str | None = None
    epoch: str | None = None          # "point", "attachment", or None

    def expect(self) -> dict[str, Any]:
        """The fixture's shape (``declarations.json``)."""
        if self.declaration == DECLARED:
            return {"declaration": DECLARED, "carrier": self.carrier, "kind": self.kind, "unit": self.unit,
                    "buckets": None if self.buckets is None else list(self.buckets),
                    "semantic": self.semantic, "epoch": self.epoch}
        if self.declaration == INVALID:
            return {"declaration": INVALID, "reasons": list(self.reasons)}
        return {"declaration": self.declaration}


def resolve(layers: dict[str, Any]) -> dict[str, Any]:
    """Core §9.3: "key by key, resource over kind_defaults over defaults"."""
    return {**(layers.get("defaults") or {}), **(layers.get("kind_defaults") or {}),
            **(layers.get("resource") or {})}


def type_names(root: Any) -> set[str] | None:
    """Core §7.3: a root's ``type`` "read as a set of names", or None when it
    has none."""
    if not isinstance(root, dict) or "type" not in root:
        return None
    t = root["type"]
    if isinstance(t, str):
        return {t}
    if isinstance(t, list) and all(isinstance(x, str) for x in t):
        return set(t)
    return None


def root_of(t: Any) -> Any:
    """A JSON Schema type's root: the fixture's (or a contract reader's)
    ``root``, else, for a profile type, the profile file's own."""
    if not isinstance(t, dict):
        return None
    if t.get("root") is not None:
        return t["root"]
    name = t.get("name")
    if isinstance(name, str) and is_profile_type(t, name):
        return _profile_root(name)
    return None


def carries(kind: str, t: Any) -> bool:
    """§2.4: whether a resource of type ``t`` can carry ``kind``.
    - The Point carries every kind; the profile's Histogram carries
      ``histogram`` alone, by its artifact's id;
    - a raw type carries ``binary`` alone, and no JSON Schema type does;
    - any other JSON Schema type carries by its root's ``type`` as a set:
      ``counter`` exactly {integer}, ``gauge`` a non-empty subset of
      {integer, number}, ``boolean`` exactly {boolean}, ``text`` exactly
      {string}. A root with no ``type`` (a ``oneOf``, the nullable form)
      carries nothing, and a ``type`` holding ``null`` matches no row: "a
      type that admits it carries none";
    - a protobuf message carries nothing at 1.x."""
    if is_profile_type(t, POINT):
        return True
    if kind == "histogram":
        return is_profile_type(t, HISTOGRAM)
    if not isinstance(t, dict):
        return False
    if t.get("kind") == "raw":
        return kind == "binary"
    if t.get("kind") != "jsonschema" or kind == "binary" or is_profile_type(t, HISTOGRAM):
        return False
    names = type_names(root_of(t))
    if names is None:
        return False
    if kind == "counter":
        return names == {"integer"}
    if kind == "gauge":
        return bool(names) and names <= {"integer", "number"}
    if kind == "boolean":
        return names == {"boolean"}
    if kind == "text":
        return names == {"string"}
    return False


def declare(kind: str, annotations: dict[str, Any], type_: Any, attachment: Any = None) -> Declaration:
    """§2.1, §2.2 and §2.4 over a resolved resource: its authoring ``kind``
    (``@stream`` and ``@state`` read as ``stream`` and ``state``), its
    resolved ``annotations`` (core §9.3), its payload type and its
    attachment's type."""
    kind = {"@stream": "stream", "@state": "state"}.get(kind, kind)
    writes = any(k in annotations for k in KEYS)
    if kind not in ("stream", "state"):
        # §2.1 "Which kinds", §2.2 "On an event or an operation, ignored":
        # whatever the values (a requirement too, §2.1).
        return Declaration(IGNORED if writes else NOT_THIS)
    point = is_profile_type(type_, POINT)
    if not writes and not point:
        return Declaration(NOT_THIS)
    k = annotations.get(KIND, _MISSING)
    u = annotations.get(UNIT, _MISSING)
    b = annotations.get(BUCKETS, _MISSING)
    s = annotations.get(SEMANTIC, _MISSING)
    k_ok = k is _MISSING or (isinstance(k, str) and k in KINDS)
    b_ok = b is _MISSING or bounds_valid(b)
    reasons: list[str] = []
    # 1. each key alone
    if not k_ok:
        reasons.append("kind_value")
    if u is not _MISSING and not is_unit(u):
        reasons.append("unit_value")
    if not b_ok:
        reasons.append("buckets_value")
    if s is not _MISSING and not (isinstance(s, str) and s in SEMANTICS):
        reasons.append("semantic_value")
    # 2. "nothing says what the annotations describe"
    if k is _MISSING and not point:
        reasons.append("kind_missing")
    # 3. and 4. need a valid kind (or none) and valid buckets
    if b is not _MISSING and b_ok and k_ok and k != "histogram":
        reasons.append("buckets_without_histogram")
    if k == "histogram" and b is _MISSING:
        reasons.append("histogram_without_buckets")
    # 5. needs a valid kind
    if k is not _MISSING and k_ok and not carries(k, type_):
        reasons.append("not_a_carrier")
    if reasons:
        return Declaration(INVALID, tuple(reasons))
    declared_kind = None if k is _MISSING else k
    if point:
        epoch = "point"
    elif declared_kind in ("counter", "histogram") and is_profile_type(attachment, EPOCH):
        epoch = "attachment"
    else:
        epoch = None
    return Declaration(DECLARED, (), "point" if point else "bare", declared_kind,
                       None if u is _MISSING else u,
                       None if b is _MISSING else tuple(to_double(x) for x in b),
                       None if s is _MISSING else s, epoch)


# -- a sample's conformance (§2.5, §2.6) -------------------------------------------------

@dataclass(frozen=True)
class Sample:
    """One sample as a reader holds it: its decoded data item and its decoded
    attachment (``_MISSING`` when it has none), or undecodable. An
    attachment present whose bytes do not decode as its encoding is
    ``attachment_undecodable``, with ``attachment`` left ``_MISSING``."""

    value: Any = None
    attachment: Any = _MISSING
    undecodable: bool = False
    attachment_undecodable: bool = False


NO_ATTACHMENT = _MISSING


@dataclass(frozen=True)
class Conformance:
    """§5's second and third questions over one sample, and what a pair
    reads of it when it conforms (§2.10)."""

    conformance: str
    reasons: tuple[str, ...]
    kind: str | None
    kind_from: str | None
    unit: str | None
    unit_from: str | None
    #: what a pair reads (conforming samples only): the value of its kind,
    #: its in-band epoch, and a Point's timestamp
    value: Any = None
    epoch: str | None = None
    timestamp_ms: int | None = None

    def expect(self) -> dict[str, Any]:
        return {"conformance": self.conformance, "reasons": list(self.reasons), "kind": self.kind,
                "kind_from": self.kind_from, "unit": self.unit, "unit_from": self.unit_from}


def _point_shape(p: Any) -> bool:
    """§2.6 check 1, "a Point carrier whose value is not one"; members the
    schema does not name are tolerated (core §9.8)."""
    if not isinstance(p, dict):
        return False
    ts = as_integer(p.get("timestamp")) if "timestamp" in p else None
    if ts is None or not I64_MIN <= ts <= I64_MAX:
        return False
    v = p.get("value", _MISSING)
    if not isinstance(v, dict) or "value" not in v or not (isinstance(v.get("type"), str) and v["type"] in KINDS):
        return False
    if "labels" in p and not (isinstance(p["labels"], dict) and all(isinstance(x, str) for x in p["labels"].values())):
        return False
    if "unit" in p and not isinstance(p["unit"], str):
        return False
    return True


def _value_reasons(kind: str, v: Any, point: bool, declared_buckets: tuple[float, ...] | None) -> list[str]:
    """§2.6 checks 6 to 12 for one value of ``kind``."""
    if kind == "counter":
        i = as_integer(v)
        if i is None:
            return ["wrong_type"]
        return [] if 0 <= i <= U64_MAX else ["out_of_range"]
    if kind == "gauge":
        if not is_number(v):
            return ["wrong_type"]
        # An integer is read exactly (§2.6), so it is never NaN or infinite,
        # however large (a CBOR bignum, 0.2); only a float can be.
        return ["not_finite"] if isinstance(v, float) and not math.isfinite(v) else []
    if kind == "boolean":
        return [] if isinstance(v, bool) else ["wrong_type"]
    if kind == "text":
        return [] if isinstance(v, str) else ["wrong_type"]
    if kind == "binary":
        if point:
            ok = isinstance(v, list) and all(as_integer(x) is not None and 0 <= as_integer(x) <= 255 for x in v)
        else:
            ok = isinstance(v, (bytes, bytearray))
        return [] if ok else ["wrong_type"]
    # histogram
    if not (isinstance(v, dict) and isinstance(v.get("buckets"), list) and all(is_number(x) for x in v["buckets"])
            and isinstance(v.get("counts"), list) and all(as_integer(x) is not None for x in v["counts"])
            and as_integer(v.get("count")) is not None and is_number(v.get("sum"))):
        return ["wrong_type"]
    out: list[str] = []
    count, counts = as_integer(v["count"]), [as_integer(x) for x in v["counts"]]
    bounds = v["buckets"]
    in_range = 0 <= count <= U64_MAX and all(0 <= c <= U64_MAX for c in counts)  # type: ignore[operator]
    if not in_range:
        out.append("out_of_range")
    # A bound is read as its nearest double (§2.2), so a huge integer bound
    # is an infinity; the sum is a number, read exactly when an integer.
    finite = all(math.isfinite(to_double(x)) for x in bounds)
    sum_finite = not (isinstance(v["sum"], float) and not math.isfinite(v["sum"]))
    if not sum_finite or not finite:
        out.append("not_finite")
    valid = finite and bounds_valid(bounds)
    if finite and not valid:
        out.append("bounds_invalid")
    if valid and declared_buckets is not None and not bounds_equal(bounds, declared_buckets):
        out.append("bounds_differ")
    # §2.6 check 11 (0.2, F-145): "checked whenever the value is a Histogram
    # … it needs the two lengths alone".
    if len(counts) != len(bounds) + 1:
        out.append("counts_length")
    if in_range and count != sum(counts):  # type: ignore[arg-type]
        out.append("count_sum")
    return out


def conform(decl: Declaration, sample: Sample) -> Conformance:
    """§2.6 over one sample of a member whose declaration is ``decl``, with
    §2.5's kind and unit: "the declared kind and unit always, unless the
    declaration is invalid or not asked; the Point's tag and its unit only
    when the sample conforms" (§2.6: "A sample that does not conform is not
    read")."""
    if decl.declaration in (NOT_THIS, IGNORED):
        return Conformance(NOT_ASKED, (decl.declaration,), None, None, None, None)
    if decl.declaration == INVALID:
        return Conformance(UNOBSERVABLE, ("declaration_invalid",), None, None, None, None)
    dk = (decl.kind, "declared") if decl.kind is not None else (None, None)
    du = (decl.unit, "declared") if decl.unit is not None else (None, None)
    if sample.undecodable:
        return Conformance(UNOBSERVABLE, ("undecodable",), *dk, *du)
    reasons: list[str] = []
    v = sample.value
    epoch: str | None = None
    timestamp: int | None = None
    if decl.carrier == "point":
        if not _point_shape(v):
            return Conformance(DOES_NOT, ("not_a_point",), *dk, *du)
        if "epoch" in v:
            if is_epoch(v["epoch"]):
                epoch = v["epoch"]
            else:
                reasons.append("epoch_invalid")
        pu = v.get("unit", _MISSING)
        if pu is not _MISSING and not is_unit(pu):
            reasons.append("unit_invalid")
        tag = v["value"]["type"]
        if decl.kind is not None and tag != decl.kind:
            reasons.append("tag_disagrees")
        if pu is not _MISSING and is_unit(pu) and decl.unit is not None and pu != decl.unit:
            reasons.append("unit_disagrees")
        kind, inner = tag, v["value"]["value"]
        timestamp = as_integer(v["timestamp"])
        reasons += _value_reasons(kind, inner, True, decl.buckets)
    else:
        if decl.epoch == "attachment" and sample.attachment_undecodable:
            # §2.6 check 2 (0.2, F-147): "An Epoch attachment whose bytes do
            # not decode as its encoding is an epoch present and not a
            # string, zero bytes included; the sample is still read".
            reasons.append("epoch_invalid")
        elif decl.epoch == "attachment" and sample.attachment is not _MISSING:
            if is_epoch(sample.attachment):
                epoch = sample.attachment
            else:
                reasons.append("epoch_invalid")
        kind, inner = decl.kind, v  # type: ignore[assignment]
        reasons += _value_reasons(kind, inner, False, decl.buckets)  # type: ignore[arg-type]
    if reasons:
        return Conformance(DOES_NOT, tuple(reasons), *dk, *du)
    k = dk if decl.kind is not None else (kind, "point")
    if decl.unit is not None:
        u = du
    elif decl.carrier == "point" and isinstance(v.get("unit"), str):
        u = (v["unit"], "point")
    else:
        u = (None, None)
    return Conformance(CONFORMS, (), *k, *u, value=_read_value(kind, inner), epoch=epoch,
                       timestamp_ms=timestamp)


def _read_value(kind: str, v: Any) -> Any:
    """A conforming value as a pair reads it: a counter's integer exactly, a
    histogram's integers."""
    if kind == "counter":
        return as_integer(v)
    if kind == "histogram":
        return {"buckets": list(v["buckets"]), "counts": [as_integer(c) for c in v["counts"]],
                "count": as_integer(v["count"]), "sum": v["sum"]}
    return v


# -- a pair (§2.9 to §2.11) --------------------------------------------------------------

@dataclass(frozen=True)
class Read:
    """One conforming sample as a pair reads it (``continuity.json``'s
    sample): its kind, value, in-band epoch, a Point's timestamp, a state's
    stamp in nanoseconds, and its arrival on the reader's monotonic clock."""

    kind: str
    value: Any
    epoch: str | None = None
    timestamp_ms: int | None = None
    stamp_ns: int | None = None
    arrival_ns: int = 0


@dataclass(frozen=True)
class Presence:
    """§2.9 point 2, what the reader saw of the owner's instance tokens (or a
    member's): one alone and without a loss across the pair, and the only
    one present at each arrival (None for none or several)."""

    alone_throughout: bool
    earlier: str | None
    later: str | None


SAME, DIFFERENT = "same", "different"


def epoch_of(a: Read, b: Read, presence: Presence | None) -> tuple[str | None, str | None]:
    """§2.9: (``same`` | ``different`` | None for unobservable, ``in_band`` |
    ``presence`` | None), the first that answers."""
    if a.epoch is not None and b.epoch is not None:
        return (SAME if a.epoch == b.epoch else DIFFERENT), "in_band"
    if presence is not None:
        if presence.alone_throughout:
            return SAME, "presence"
        if presence.earlier is not None and presence.later is not None and presence.earlier != presence.later:
            return DIFFERENT, "presence"
    return None, None


def _decrease(a: Read, b: Read) -> bool:
    """§2.10: "a later counter below the earlier one, or a later histogram
    with its ``count``, or any bucket's count, below the earlier one's"."""
    if a.kind == "counter":
        return b.value < a.value
    return b.value["count"] < a.value["count"] or any(y < x for x, y in zip(a.value["counts"], b.value["counts"]))


def elapsed_ns(a: Read, b: Read) -> int:
    """§2.11 "The time base is the first that both samples have", and the
    elapsed time "an integer of nanoseconds"."""
    if a.timestamp_ms is not None and b.timestamp_ms is not None:
        return (b.timestamp_ms - a.timestamp_ms) * 10**6
    if a.stamp_ns is not None and b.stamp_ns is not None:
        return b.stamp_ns - a.stamp_ns
    return b.arrival_ns - a.arrival_ns


def ntp64_ns(raw: int) -> int:
    """§2.11 (0.2, F-142): a stamp's time in nanoseconds "as zenoh reads
    it: its NTP64 seconds, the high 32 bits, times 10^9, plus its 32-bit
    fraction times 10^9 divided by 2^32 and rounded up (uhlc's
    ``NTP64::as_nanos``)", each stamp converted alone."""
    secs, frac = raw >> 32, raw & 0xFFFFFFFF
    return secs * 10**9 + -(-(frac * 10**9) // 2**32)


def rate_of(d: int, e: int) -> float:
    """§2.11: ``rate = double(d) / (double(e) / 1e9)`` in IEEE 754 binary64,
    round to nearest, ties to even. Python's ``float(int)`` is the double
    nearest the integer, and its ``/`` is IEEE division."""
    return float(d) / (float(e) / 1e9)


@dataclass(frozen=True)
class Pair:
    """§5's fourth and fifth questions over one pair."""

    continuity: str
    epoch: str | None       # established, assumed, or None
    via: str | None         # in_band, presence, or None
    rate: float | None
    rate_unit: str | None
    no_rate: str | None

    def expect(self) -> dict[str, Any]:
        return {"continuity": self.continuity, "epoch": self.epoch, "via": self.via, "rate": self.rate,
                "rate_unit": self.rate_unit, "no_rate": self.no_rate}


def continuity(a: Read, b: Read, presence: Presence | None, unit: str | None) -> Pair:
    """§2.10's answer, the first that holds, then §2.11's rate."""
    pairing = ("counter", "histogram")
    if a.kind not in pairing and b.kind not in pairing:
        cont, est, via = NOT_ASKED, None, None
    elif a.kind != b.kind or (a.kind == "histogram" and (
            not bounds_equal(a.value["buckets"], b.value["buckets"])
            or len(a.value["counts"]) != len(b.value["counts"]))):
        cont, est, via = UNOBSERVABLE, None, None
    else:
        same, via = epoch_of(a, b, presence)
        dec = _decrease(a, b)
        if same == SAME:
            cont, est = ("decreased" if dec else "same_epoch"), "established"
        elif same == DIFFERENT:
            cont, est = "reset", "established"
        else:
            # §2.10 point 4: "A reader MUST read a decrease it cannot place
            # in one epoch as a reset".
            cont, est = ("reset" if dec else "same_epoch"), "assumed"
    # §2.11 and §5 (0.2, F-146): no rate "of an incomparable pair, which is
    # never of two counters (kind)".
    if not (a.kind == "counter" and b.kind == "counter") or cont == UNOBSERVABLE:
        return Pair(cont, est, via, None, None, "kind")
    if cont != "same_epoch":
        return Pair(cont, est, via, None, None, cont)
    e = elapsed_ns(a, b)
    if e <= 0:
        return Pair(cont, est, via, None, None, "elapsed")
    return Pair(cont, est, via, rate_of(b.value - a.value, e), None if unit is None else f"{unit}/s", None)


FINDING, NO, COULD_NOT_TELL = "finding", "no", "could_not_tell"


def verdict(conformances: Iterable[Conformance], pairs: Iterable[Pair]) -> str:
    """A tool's answer over a window's samples and pairs (§5, with "Over many
    pairs", 0.3):
    - ``finding``: a sample that does not conform, or a pair ``decreased``;
    - ``could_not_tell``: nothing conforms (silence is no verdict), or
      counters or histograms were read and no pair of theirs had its epoch
      established: "over pairs of counters or histograms whose every epoch
      was assumed SHOULD say it could not tell … no decrease could have been
      found". zk2py reads "every" as holding of no pair too, so a counter
      read with no pair at all is here as well (F-152);
    - ``no`` otherwise, a window of gauges, text, booleans or bytes alone
      included (the continuity question not asked)."""
    cs, ps = list(conformances), list(pairs)
    if any(c.conformance == DOES_NOT for c in cs) or any(p.continuity == "decreased" for p in ps):
        return FINDING
    if not any(c.conformance == CONFORMS for c in cs):
        return COULD_NOT_TELL
    pairing = [c for c in cs if c.conformance == CONFORMS and c.kind in ("counter", "histogram")]
    if pairing and not any(p.epoch == "established" for p in ps if p.continuity != NOT_ASKED):
        return COULD_NOT_TELL
    return NO


def kept_reads(arrivals: list[tuple[Read, Any]]) -> list[Read]:
    """§2.10 (0.2, F-148): "For state, it is in the order a late joiner's
    merge keeps them (core §4.2) … a sample the merge does not keep (a
    GET's reply older than a live put already held) is no sample of any
    pair". Each arrival is (its read, its :class:`zk2py.seed.Stamp` or
    None), in arrival order."""
    from . import seed

    flags = seed.kept([(stamp, "put") for _, stamp in arrivals])
    return [r for (r, _), k in zip(arrivals, flags) if k]


def read_of(c: Conformance, *, stamp_ns: int | None = None, arrival_ns: int = 0) -> Read:
    """A conforming sample as a pair reads it."""
    assert c.conformance == CONFORMS and c.kind is not None
    return Read(c.kind, c.value, c.epoch, c.timestamp_ms, stamp_ns, arrival_ns)


# -- a contract's resources --------------------------------------------------------------

def _world_root(docs: dict[str, Any], ids: dict[str, str], t: dict[str, Any]) -> Any:
    """§2.4: "the named definition, its ``$ref``s followed as core §9.8
    follows them (a keyword beside a ``$ref`` added to its target, an outer
    one taking the place of the target's own)"."""
    from .compat import JsonWorld

    where = ids.get(t.get("schema"))  # type: ignore[arg-type]
    doc = docs.get(where) if where is not None else None
    defs = doc.get("$defs") if isinstance(doc, dict) else None
    if not isinstance(defs, dict) or t.get("name") not in defs:
        return None
    return JsonWorld(docs).resolve(where, defs[t["name"]])


def _typed(t: Any, docs: dict[str, Any], ids: dict[str, str]) -> Any:
    if not isinstance(t, dict) or t.get("kind") != "jsonschema":
        return t
    return {**t, "root": _world_root(docs, ids, t)}


def declarations_of(canonical: dict[str, Any], docs: dict[str, Any], ids: dict[str, str]) -> dict[str, Declaration]:
    """Every resource of a canonical contract (its annotations resolved,
    core §9.3), by ``<token>/<template>``. ``docs`` are its JSON Schema
    artifacts by stem, ``ids`` an artifact id's stem."""
    out = {}
    for r in canonical["resources"]:
        out[f"{r['token']}/{r['template']}"] = declare(
            r["kind"], r.get("annotations") or {}, _typed(r.get("type"), docs, ids),
            _typed(r.get("attachment"), docs, ids))
    return out


def contract_declarations(c: Any) -> dict[str, Declaration]:
    """A loaded zk2py contract's declarations."""
    docs = {a.name: a.data for a in c.schemas.json_files}
    ids = {a.id: a.name for a in c.schemas.json_files}
    return declarations_of(c.canonical, docs, ids)


def bundle_declarations(data: bytes) -> tuple[dict[str, Any], dict[str, Declaration]]:
    """A bundle's contract (core §9.6) and its declarations: what "a reader
    of an old bundle" reads (§2.3)."""
    b = jcs.loads(data)
    names = {s["id"]: s.get("name") for s in b["contract"]["schemas"]}
    docs = {names[i]: s["data"] for i, s in b["schemas"].items() if s.get("kind") == "jsonschema"}
    ids = {i: names[i] for i in b["schemas"]}
    return b["contract"], declarations_of(b["contract"], docs, ids)


def bundle_artifact(data: bytes, name: str = PROFILE) -> Any:
    """The JSON Schema artifact a bundle's contract lists under ``name``
    (its stem, core §9.4), as the bundle carries it."""
    b = jcs.loads(data)
    for s in b["contract"]["schemas"]:
        if s.get("name") == name:
            return b["schemas"][s["id"]]["data"]
    return None

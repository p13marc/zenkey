"""The ``packed`` type kind (core §7.6, 0.75, #979 B1): a packed file's
shape and its lints (§9.2's E038 to E045, cascade 7), each type's layout,
layout form and tag, and its codec, a record written from its JSON reading
and read back to it.

**The file** (§7.6 "The schema file") is strict JSON: ``types``, each a
record of ``fields``, each item a field or the extension marker ``"..."``.
:func:`lint` reads one document into its types and its findings.

**Widths.** A field's width is ⌈log₂ c⌉ for its c codes: an ``int``'s range
plus its reserved codes, a ``fixed``'s ``(maximum − minimum) / resolution +
1`` plus its reserved codes, an ``enum``'s values (one more with
``escape``, or its own ``width``), a ``bool``'s 1, a ``counter``'s ``width``,
a ``time``'s 32 or 42 with ``ms``.

**Exact arithmetic** (§7.6's MUST): every decimal is a
:class:`fractions.Fraction`, read from its string; a code is rounded with a
tie to the even integer, and a value read back is ``minimum + code ×
resolution`` in its shortest form. No binary float takes part, but for a
writer handed a JSON number, which zk2py's strict JSON reader holds as a
binary64: that number is read by its shortest decimal text (``repr``), as
``packed/vectors.json``'s "a JSON number for a fixed-point value is read by
its shortest decimal text" says (round 94's F-426 asks whether that is the
text's reading).

**The layout** (§7.6): the fields in order, most significant bit first, an
optional field's presence bit before it, a marker one bit (1 when a field
after it is present), then a pad of fewer than 8 zero bits. The **tag** is
the low four bits of the last byte of the sha256 of the layout form's JCS
bytes: the root's fields before its first marker, without names, then
``"..."`` where the root ends at a marker.

**The reader** refuses, in §7.6's order: ``tag``; then field by field
``length`` and ``code``; ``extension``; ``length`` (a byte after the pad);
``pad``. A marker read as 1 at the end of the reader's layout ends the
reading: the rest is a later revision's, and holds at least one bit.

**The writer** refuses with ``packed/vectors.json``'s tokens, which the text
does not name (it says only what a writer MUST NOT write): ``unknown`` (a
member the type does not have), ``missing`` (a field not given), ``value``
(a value of the wrong JSON type, a name not declared, a decimal that does not
read), ``range`` (a value with no code) and ``epoch`` (a time field and no
epoch in hand).
"""

from __future__ import annotations

import hashlib
import math
import re
from dataclasses import dataclass, field
from fractions import Fraction
from typing import Any

from . import jcs

MARKER = "..."
FIELD_TYPES = ("int", "fixed", "enum", "bool", "counter", "time")
#: §7.6 "Reserved codes": they take the top codes, from all ones down, "in
#: that order".
RESERVED = ("not_available", "error", "out_of_range")

TYPE_NAME = re.compile(r"[A-Z][A-Za-z0-9]*")
FIELD_NAME = re.compile(r"[a-z][a-z0-9_]*")
VALUE_NAME = re.compile(r"[A-Za-z][A-Za-z0-9_]*")
#: §7.6 "A decimal string".
DECIMAL = re.compile(r"-?(0|[1-9][0-9]*)(\.[0-9]+)?")
#: What a writer's fixed-point string is read as: a JSON number's text, an
#: exponent included (``packed/vectors.json``'s "an exponent in a writer's
#: decimal"; round 94's F-425). ``43,5`` is no decimal. Groups: the sign,
#: the integer digits, the fraction's, the exponent's sign and digits.
WRITER_DECIMAL = re.compile(r"(-?)(0|[1-9][0-9]*)(?:\.([0-9]+))?(?:[eE]([+-]?)([0-9]+))?")

_COMMON_MEMBERS = {"name", "type", "optional", "description"}
#: Each field type's members: (needed, taken).
_MEMBERS = {
    "int": ({"minimum", "maximum"}, {"reserved"}),
    "fixed": ({"minimum", "maximum", "resolution"}, {"reserved"}),
    "enum": ({"values"}, {"escape", "width"}),
    "bool": (set(), set()),
    "counter": ({"width"}, set()),
    "time": (set(), {"ms"}),
}


class Refused(ValueError):
    """A writer's or a reader's refusal, by its token."""

    def __init__(self, token: str, message: str = "") -> None:
        super().__init__(f"{token}: {message}" if message else token)
        self.token = token


# -- decimals ------------------------------------------------------------------

def read_decimal(text: str) -> Fraction | None:
    """§7.6's decimal string, "read as the exact decimal it writes", or None."""
    return Fraction(text) if isinstance(text, str) and DECIMAL.fullmatch(text) else None


def shortest(q: Fraction) -> str:
    """§7.6 "Its shortest form": no zero at the end of a fraction, no point
    with no digit after it, ``0`` for zero. ``q`` is a terminating decimal."""
    sign = "-" if q < 0 else ""
    q = abs(q)
    k = 0
    while (10 ** k) % q.denominator:
        k += 1
    digits = str(q.numerator * (10 ** k // q.denominator)).rjust(k + 1, "0")
    out = digits if k == 0 else f"{digits[:-k]}.{digits[-k:]}"
    return "0" if out == "0" else sign + out


def round_half_even(q: Fraction) -> int:
    """round(q), a tie to the even integer (§7.6 "fixed")."""
    f = math.floor(q)
    r = q - f
    if r != Fraction(1, 2):
        return f + 1 if r > Fraction(1, 2) else f
    return f if f % 2 == 0 else f + 1


def bits_for(codes: int) -> int:
    """⌈log₂ codes⌉, the fewest bits that hold that many codes."""
    return (codes - 1).bit_length() if codes > 0 else 0


# -- the declarations ----------------------------------------------------------

@dataclass
class Field:
    """One field of a record, as §7.6 declares it."""

    name: str
    type: str
    optional: bool = False
    #: int: the bounds; fixed: the bounds and the resolution, exact.
    minimum: Any = None
    maximum: Any = None
    resolution: Fraction | None = None
    #: The declared reserved names, in §7.6's order.
    reserved: tuple[str, ...] = ()
    values: tuple[str, ...] = ()
    escape: bool = False
    enum_width: int | None = None
    counter_width: int | None = None
    ms: bool = False

    @property
    def values_count(self) -> int:
        """An int's or a fixed's value codes, from 0."""
        if self.type == "int":
            return self.maximum - self.minimum + 1
        assert self.type == "fixed" and self.resolution is not None
        return int((self.maximum - self.minimum) / self.resolution) + 1

    @property
    def width(self) -> int:
        k = len(self.reserved)
        if self.type in ("int", "fixed"):
            return bits_for(self.values_count + k)
        if self.type == "enum":
            if self.enum_width is not None:
                return self.enum_width
            return bits_for(len(self.values) + (1 if self.escape else 0))
        if self.type == "bool":
            return 1
        if self.type == "counter":
            assert self.counter_width is not None
            return self.counter_width
        return 42 if self.ms else 32

    def reserved_codes(self) -> dict[str, int]:
        """Each declared reserved name's code: the top codes of the width,
        from all ones down, in §7.6's order."""
        top = (1 << self.width) - 1
        return {name: top - i for i, name in enumerate(self.reserved)}

    def form(self) -> dict[str, Any]:
        """The field's layout form (§7.6 "The tag")."""
        out: dict[str, Any] = {"type": self.type, "width": self.width, "optional": self.optional}
        if self.type == "int":
            out.update(minimum=self.minimum, maximum=self.maximum, reserved=list(self.reserved))
        elif self.type == "fixed":
            assert self.resolution is not None
            out.update(minimum=shortest(self.minimum), maximum=shortest(self.maximum),
                       resolution=shortest(self.resolution), reserved=list(self.reserved))
        elif self.type == "enum":
            out["escape"] = self.escape
            if not self.escape:
                out["values"] = len(self.values)
        elif self.type == "time":
            out["ms"] = self.ms
        return out


Item = Field | str  # a field, or MARKER


def _as_jcs_numbers(form: list[Any]) -> list[Any]:
    """A layout form with each integer outside JCS's exact domain made the
    binary64 RFC 8785 writes for it (``packed/vectors.json``'s ``Wide64``)."""
    def num(v: Any) -> Any:
        return float(v) if jcs.is_json_integer(v) and abs(v) > jcs.INT_MAX else v
    return [{k: num(v) for k, v in it.items()} if isinstance(it, dict) else it for it in form]


@dataclass
class Record:
    """One packed type: its items in order."""

    name: str
    items: list[Item]

    # -- the layout --------------------------------------------------------

    def layout_form(self) -> list[Any]:
        out: list[Any] = []
        for it in self.items:
            if it == MARKER:
                out.append(MARKER)
                break
            assert isinstance(it, Field)
            out.append(it.form())
        return out

    def layout_sha256(self) -> str:
        """The sha256 of the layout form's JCS bytes. An ``int``'s bound
        past ±(2^53 − 1) is written as RFC 8785 writes a double (core 0.76's
        ``Wide64``: such a type is in no contract, E028, but has a tag)."""
        return hashlib.sha256(jcs.dumps(_as_jcs_numbers(self.layout_form()))).hexdigest()

    @property
    def tag(self) -> int:
        return bytes.fromhex(self.layout_sha256())[-1] & 0x0F

    def item_layout(self) -> list[dict[str, Any]]:
        """Each item's width and start: its offset, None once an optional
        field or a marker before it makes it vary."""
        out: list[dict[str, Any]] = []
        offset: int | None = 0
        for it in self.items:
            if it == MARKER:
                out.append({"marker": True, "offset": offset})
                offset = None
                continue
            assert isinstance(it, Field)
            out.append({"name": it.name, "width": it.width, "optional": it.optional, "offset": offset})
            if offset is not None:
                offset = None if it.optional else offset + it.width
        return out

    def bit_bounds(self) -> tuple[int, int]:
        """The shortest and the longest record this revision writes, in bits
        before the pad."""
        shortest_bits = 0
        for it in self.items:
            if it == MARKER:
                shortest_bits += 1
                break
            assert isinstance(it, Field)
            shortest_bits += 1 if it.optional else it.width
        longest = 0
        for i, it in enumerate(self.items):
            if it == MARKER:
                longest += 1
                if not any(isinstance(x, Field) for x in self.items[i + 1:]):
                    break
                continue
            assert isinstance(it, Field)
            longest += (1 if it.optional else 0) + it.width
        return shortest_bits, longest

    def field(self, name: str) -> Field | None:
        return next((f for f in self.items if isinstance(f, Field) and f.name == name), None)

    # -- writing -----------------------------------------------------------

    def write_bits(self, value: Any, epoch: int | None = None) -> str:
        """The record's bits before the pad, for its JSON reading ``value``
        (§7.6 "Writing", "The JSON reading": a fixed-point value as a string
        or a number). Raises :class:`Refused`."""
        if not isinstance(value, dict):
            raise Refused("value", "a record's value is a JSON object")
        fields = [f for f in self.items if isinstance(f, Field)]
        names = {f.name for f in fields}
        for k in value:
            if k not in names:
                raise Refused("unknown", f"{k!r} is no field of {self.name}")
        # Every field not optional, over the whole layout, before any bit: a
        # marker written 0 ends the record, and E045 keeps a clean type from
        # holding a field after a marker that is not optional, so none could
        # be skipped there; this holds of a Record built without the lints too.
        for f in fields:
            if not f.optional and f.name not in value:
                raise Refused("missing", f"{f.name!r} is not given")
        bits: list[str] = []
        for i, it in enumerate(self.items):
            if it == MARKER:
                later = any(isinstance(x, Field) and x.name in value for x in self.items[i + 1:])
                bits.append("1" if later else "0")
                if not later:
                    break
                continue
            assert isinstance(it, Field)
            if it.name not in value:
                bits.append("0")  # an optional field absent: its presence bit
                continue
            code = encode(it, value[it.name], epoch)
            if it.optional:
                bits.append("1")
            bits.append(format(code, f"0{it.width}b"))
        return "".join(bits)

    def write(self, value: Any, epoch: int | None = None) -> bytes:
        """The record's bytes: its bits, then the pad (§7.6 "Writing")."""
        bits = self.write_bits(value, epoch)
        pad = -len(bits) % 8
        return int(bits + "0" * pad, 2).to_bytes((len(bits) + pad) // 8, "big") if bits else b""

    # -- reading -----------------------------------------------------------

    def read(self, tag: int, record: bytes, epoch: int | None = None) -> dict[str, Any]:
        """The JSON reading of a record that came under CH's ``tag``, or
        :class:`Refused` with the first refusal in §7.6's order."""
        if tag != self.tag:
            raise Refused("tag", f"CH's tag {tag} is not {self.name}'s {self.tag}")
        total = len(record) * 8
        whole = int.from_bytes(record, "big") if record else 0
        pos = 0

        def take(n: int) -> int:
            nonlocal pos
            if pos + n > total:
                raise Refused("length", f"the record ends at bit {total}, before bit {pos + n}")
            v = (whole >> (total - pos - n)) & ((1 << n) - 1) if n else 0
            pos += n
            return v

        out: dict[str, Any] = {}
        open_markers: list[int] = []  # markers read as 1, not yet followed by a present field
        later_revision = False
        for i, it in enumerate(self.items):
            if it == MARKER:
                if take(1) == 0:
                    break
                if i == len(self.items) - 1:
                    # §7.6 "A later revision's fields": "That rest MUST hold
                    # at least one bit (length)", and the extension, length
                    # and pad checks are not made.
                    if pos >= total:
                        raise Refused("length", "a marker read as 1 at the layout's end, and no bit after it")
                    later_revision = True
                    break
                open_markers.append(i)
                continue
            assert isinstance(it, Field)
            if it.optional and take(1) == 0:
                continue
            out[it.name] = decode(it, take(it.width), epoch)
            open_markers.clear()
        if later_revision:
            return out
        if open_markers:
            raise Refused("extension", "a marker read as 1 is followed by no present field")
        if total - pos >= 8:
            raise Refused("length", f"{(total - pos) // 8} byte(s) after the pad")
        if total - pos and whole & ((1 << (total - pos)) - 1):
            raise Refused("pad", "a pad bit is 1")
        return out


def _show(v: Any) -> str:
    """A value for a refusal's message, bounded: an integer of thousands of
    digits is not written out (Python refuses to past 4300), nor a long
    string."""
    if isinstance(v, int) and not isinstance(v, bool) and v.bit_length() > 128:
        return f"an integer of {v.bit_length()} bits"
    text = repr(v)
    return text if len(text) <= 60 else text[:57] + "…"


def encode(f: Field, v: Any, epoch: int | None) -> int:
    """One field's code for its JSON value (§7.6 "Codes")."""
    if f.type in ("int", "fixed") and isinstance(v, str) and not (f.type == "fixed" and _decimal_like(v)):
        if v in f.reserved:
            return f.reserved_codes()[v]
        raise Refused("value", f"{f.name}: {_show(v)} is no value and no reserved code it declares")
    if f.type == "int":
        _integer_input(f, v, "an int's value is an integer")
        if not f.minimum <= v <= f.maximum:
            raise Refused("range", f"{f.name}: {_show(v)} is outside {f.minimum} to {f.maximum}")
        return v - f.minimum
    if f.type == "fixed":
        x = writer_decimal(f, v)
        assert f.resolution is not None
        code = round_half_even((x - f.minimum) / f.resolution)
        if not 0 <= code < f.values_count:
            raise Refused("range", f"{f.name}: {_show(v)} has no code")
        return code
    if f.type == "enum":
        if v is None:
            if not f.escape:
                raise Refused("value", f"{f.name}: a closed enum has no escape")
            return (1 << f.width) - 1
        if not isinstance(v, str) or v not in f.values:
            raise Refused("value", f"{f.name}: {_show(v)} is not a declared value")
        return f.values.index(v)
    if f.type == "bool":
        if not isinstance(v, bool):
            raise Refused("value", f"{f.name}: a bool's value is true or false")
        return int(v)
    if f.type == "counter":
        _integer_input(f, v, "a counter's value is an integer")
        if not 0 <= v < 1 << 64:
            raise Refused("range", f"{f.name}: a count from 0 to 2^64 − 1")
        return v % (1 << f.width)
    # time
    _integer_input(f, v, "a time is an integer of Unix milliseconds")
    if epoch is None:
        raise Refused("epoch", f"{f.name}: a time field and no deployment epoch")
    # ⌊(t − 1000E) / 1000⌋ from 0 to 2^32 − 1 is t from 1000E to 1000(E +
    # 2^32) − 1: compared before any arithmetic on t.
    if not 1000 * epoch <= v < 1000 * (epoch + (1 << 32)):
        raise Refused("range", f"{f.name}: {_show(v)} is outside the epoch's 2^32 seconds")
    since = v - 1000 * epoch
    seconds = since // 1000
    return (seconds << 10) | (since % 1000) if f.ms else seconds


#: core 0.76 §7.6 "A number of magnitude 2^63 or more is outside every
#: range, however it is written."
_BEYOND = 1 << 63


def _integer_input(f: Field, v: Any, why: str) -> None:
    """An ``int``, ``counter`` or ``time`` value (core 0.76 §7.6 "What a
    writer takes"): a JSON number written with no fraction and no exponent,
    so ``5.0`` is ``value``; one written so whose magnitude is 2^63 or more
    is ``range``, as ``packed/vectors.json`` reads the text ("one with a
    fraction or an exponent is `value`, unless its magnitude is 2^63 or
    more"). Integer text is read by its exact value, so a 64-bit counter
    takes 2^64 − 1 (the vectors' write 21; F-434 asks the text to say so)."""
    if isinstance(v, float) and abs(v) >= _BEYOND:
        raise Refused("range", f"{f.name}: a number of magnitude 2^63 or more")
    if not jcs.is_json_integer(v):
        raise Refused("value", f"{f.name}: {why}")


def decode(f: Field, code: int, epoch: int | None) -> Any:
    """One field's JSON reading for its code, or ``Refused("code")`` for an
    unused one (§7.6 "Unused codes", "The JSON reading")."""
    if f.type in ("int", "fixed"):
        if code < f.values_count:
            if f.type == "int":
                return f.minimum + code
            assert f.resolution is not None
            return shortest(f.minimum + code * f.resolution)
        for name, c in f.reserved_codes().items():
            if c == code:
                return name
        raise Refused("code", f"{f.name}: code {code} is unused")
    if f.type == "enum":
        if code < len(f.values):
            return f.values[code]
        if f.escape:
            return None
        raise Refused("code", f"{f.name}: code {code} is past a closed enum's values")
    if f.type == "bool":
        return code == 1
    if f.type == "counter":
        return code
    seconds, ms = (code >> 10, code & 0x3FF) if f.ms else (code, 0)
    if ms >= 1000:
        raise Refused("code", f"{f.name}: a millisecond code of {ms}")
    if epoch is None:
        raise Refused("epoch", f"{f.name}: a time field and no deployment epoch")
    return 1000 * (epoch + seconds) + ms


def _decimal_like(v: str) -> bool:
    return WRITER_DECIMAL.fullmatch(v) is not None


def _fraction_digits(q: Fraction) -> int:
    """The fewest fractional digits that write the terminating decimal q."""
    k = 0
    while (10 ** k) % q.denominator:
        k += 1
    return k


def _bounds(f: Field) -> tuple[int, int]:
    """For a fixed-point field, (h, P): every value with a code is below
    10^h in magnitude, and every tie point minimum + (k + 1/2) × resolution
    is written with at most P fractional digits."""
    assert f.resolution is not None
    reach = max(abs(f.minimum), abs(f.maximum)) + f.resolution
    h = len(str(math.ceil(reach)))
    p = max(_fraction_digits(f.minimum), _fraction_digits(f.maximum), _fraction_digits(f.resolution)) + 1
    return h, p


#: An exponent's digits past this many are clamped: 10^(10^18) is beyond
#: every bound either way, and int() of a long digit string is refused by
#: Python's int_max_str_digits.
_EXPONENT_DIGITS = 18


def writer_decimal(f: Field, v: Any) -> Fraction:
    """A writer's fixed-point value for field ``f``, exact where it matters:
    a string read as a JSON number's text, an integer, or a binary64 by its
    shortest decimal text (F-426). Raises ``Refused("value")`` for no
    decimal and ``Refused("range")`` for a magnitude no code reaches.

    The work is bounded by the declaration, not by the value: a value of
    10^h or more is refused before any arithmetic, and a value written with
    more than P + 1 fractional digits is truncated toward zero at P + 1 and
    given one more nonzero digit (a sticky digit). Every tie point has at
    most P fractional digits, so the substitute lies between the same two
    tie points as the value, and rounds to the same code: exact, never a
    float."""
    if isinstance(v, bool):
        raise Refused("value", f"{f.name}: {_show(v)} is not a decimal")
    h, p = _bounds(f)
    if isinstance(v, int):
        if abs(v) >= 10 ** h:
            raise Refused("range", f"{f.name}: an integer of 10^{h} or more has no code")
        return Fraction(v)
    if isinstance(v, float):
        v = repr(v)
    m = WRITER_DECIMAL.fullmatch(v) if isinstance(v, str) else None
    if m is None:
        raise Refused("value", f"{f.name}: {_show(v)} is not a decimal")
    sign, whole, frac, esign, edigits = m.groups()
    frac = frac or ""
    exp = 0
    if edigits:
        e = edigits.lstrip("0") or "0"
        exp = 10 ** _EXPONENT_DIGITS if len(e) > _EXPONENT_DIGITS else int(e)
        exp = -exp if esign == "-" else exp
    digits = whole + frac
    point = len(whole) + exp  # digits of `digits` before the decimal point
    stripped = digits.lstrip("0")
    point -= len(digits) - len(stripped)
    if not stripped:
        return Fraction(0)
    if point - 1 >= h:  # its leading digit is at 10^(point - 1)
        raise Refused("range", f"{f.name}: a decimal of 10^{h} or more has no code")
    keep = point + p + 1  # digits through the (P + 1)th after the point
    if keep <= 0:
        kept, sticky = 0, True
    else:
        head, rest = stripped[:keep], stripped[keep:]
        kept = int(head + "0" * (keep - len(head)))
        sticky = rest.strip("0") != ""
    q = Fraction(10 * kept + (1 if sticky else 0), 10 ** (p + 2))
    return -q if sign == "-" else q


# -- the file and its lints (§9.2 E038–E045, cascade 7) -------------------------

@dataclass
class Linted:
    """One packed document read: the names of the types it defines, each
    lint-clean type as a :class:`Record`, and the findings as (code, where)
    in the order met."""

    names: set[str] = field(default_factory=set)
    types: dict[str, Record] = field(default_factory=dict)
    findings: list[tuple[str, str]] = field(default_factory=list)

    def err(self, code: str, where: str) -> None:
        self.findings.append((code, where))


def _shape_problem(item: dict[str, Any]) -> str | None:
    """Why a field is outside §7.6's shape (E038), or None. Its ``type`` is
    one of §7.6's six."""
    t = item["type"]
    needed, taken = _MEMBERS[t]
    for m in item:
        if m not in _COMMON_MEMBERS | needed | taken:
            return f"holds {m!r}, which a {t} does not take"
    for m in ("name", *sorted(needed)):
        if m not in item:
            return f"no {m!r}"
    if not isinstance(item["name"], str) or FIELD_NAME.fullmatch(item["name"]) is None:
        return f"name {item['name']!r} is not a field name"
    if "optional" in item and not isinstance(item["optional"], bool):
        return "optional is not a boolean"
    if "description" in item and not isinstance(item["description"], str):
        return "description is not a string"
    if t == "int" and not all(jcs.is_json_integer(item[m]) for m in ("minimum", "maximum")):
        return "an int's bounds are integers"
    if t == "fixed" and not all(isinstance(item[m], str) for m in ("minimum", "maximum", "resolution")):
        return "a fixed point's bounds and resolution are strings"
    if "reserved" in item:
        r = item["reserved"]
        if not isinstance(r, list) or not all(isinstance(x, str) and x in RESERVED for x in r):
            return "reserved is not a list of not_available, error and out_of_range"
    if t == "enum":
        vs = item["values"]
        if not isinstance(vs, list) or not all(isinstance(x, str) and VALUE_NAME.fullmatch(x) for x in vs):
            return "values is not a list of value names"
        if "escape" in item and not isinstance(item["escape"], bool):
            return "escape is not a boolean"
        if "width" in item and not jcs.is_json_integer(item["width"]):
            return "width is not an integer"
    if t == "counter" and not jcs.is_json_integer(item["width"]):
        return "width is not an integer"
    if t == "time" and "ms" in item and not isinstance(item["ms"], bool):
        return "ms is not a boolean"
    return None


def _repeats(names: list[str]) -> int:
    return len(names) - len(set(names))


def _field_of(item: dict[str, Any]) -> Field:
    t = item["type"]
    f = Field(item["name"], t, optional=item.get("optional", False))
    if t in ("int", "fixed"):
        declared = set(item.get("reserved", []))
        f.reserved = tuple(r for r in RESERVED if r in declared)
    if t == "int":
        f.minimum, f.maximum = item["minimum"], item["maximum"]
    elif t == "fixed":
        f.minimum, f.maximum = read_decimal(item["minimum"]), read_decimal(item["maximum"])
        f.resolution = read_decimal(item["resolution"])
    elif t == "enum":
        f.values = tuple(item["values"])
        f.escape = item.get("escape", False)
        f.enum_width = item.get("width")
    elif t == "counter":
        f.counter_width = item["width"]
    elif t == "time":
        f.ms = item.get("ms", False)
    return f


def _declaration_problem(item: dict[str, Any]) -> tuple[str, str] | None:
    """Cascade 7's one of E039, E040 and E041, the first met: a decimal
    string that does not read (E040), an empty range (E039), a resolution
    not above 0 or not dividing (E040), the width (E041)."""
    t = item["type"]
    if t == "fixed":
        lo, hi, res = (read_decimal(item[m]) for m in ("minimum", "maximum", "resolution"))
        if lo is None or hi is None or res is None:
            return "E040", "a bound or the resolution is not a decimal string"
        if lo > hi:
            return "E039", "minimum above maximum"
        if res <= 0 or ((hi - lo) / res).denominator != 1:
            return "E040", "the resolution is not above 0, or does not divide maximum − minimum"
    if t == "int" and item["minimum"] > item["maximum"]:
        return "E039", "minimum above maximum"
    if t == "enum":
        n = len(item["values"])
        if n == 0:
            return "E039", "an enum with no value"
        need = n + (1 if item.get("escape", False) else 0)
        if "width" in item:
            w = item["width"]
            if not 1 <= w <= 64 or (1 << w) < need:
                return "E041", f"width {w} does not hold {need} codes, or is not from 1 to 64"
            return None
    w = _field_of(item).width
    if not 1 <= w <= 64:
        return "E041", f"a width of {w} bits"
    return None


def lint(doc: Any, where: str = "") -> Linted:
    """Read one packed document (§7.6 "The schema file"): its types, and
    §9.2's E038 to E045 under cascade 7."""
    out = Linted()
    if not isinstance(doc, dict):
        out.err("E038", f"{where}: the top level is not an object")
        return out
    types = doc.get("types")
    if (set(doc) - {"types", "description"} or not isinstance(types, dict)
            or not isinstance(doc.get("description", ""), str)):
        out.err("E038", f"{where}: the top level is outside §7.6's shape")
    if not isinstance(types, dict):
        # Cascade 7: "A file whose types is missing, or is not an object,
        # defines none."
        return out
    for tname, rec in types.items():
        here = f"{where}: {tname}"
        if TYPE_NAME.fullmatch(tname) is None:
            # Cascade 7: "a type whose name is not a type name is E038, and
            # defines no type."
            out.err("E038", f"{here}: not a type name")
            continue
        out.names.add(tname)
        before = len(out.findings)
        if not isinstance(rec, dict):
            out.err("E038", f"{here}: a record is an object")
            continue
        items = rec.get("fields")
        if (set(rec) - {"fields", "description"} or not isinstance(items, list)
                or not isinstance(rec.get("description", ""), str)):
            out.err("E038", f"{here}: the record is outside §7.6's shape")
        if not isinstance(items, list):
            continue
        # Cascade 7: "E042 and E045 read the fields of a record that have
        # neither E038 nor E043", each here as (name, optional), or MARKER.
        read: list[tuple[str, bool] | str] = []
        fields: list[Item] = []
        for idx, item in enumerate(items):
            at = f"{here}: item {idx}"
            if item == MARKER:
                read.append(MARKER)
                fields.append(MARKER)
                continue
            if not isinstance(item, dict):
                out.err("E038", f"{at}: neither a field nor the marker")
                continue
            t = item.get("type")
            # Cascade 7: "A field gets at most one of E038 and E043, and then
            # nothing more. E043 is checked first, on a type that is a string."
            if isinstance(t, str) and t not in FIELD_TYPES:
                out.err("E043", f"{at}: type {t!r} is no field type")
                continue
            why = "no type, or a type that is not a string" if not isinstance(t, str) else _shape_problem(item)
            if why is not None:
                out.err("E038", f"{at}: {why}")
                continue
            # "A repeat inside one field's values or reserved is E042 too, on
            # a field without E038."
            for _ in range(_repeats(list(item.get("values", [])))):
                out.err("E042", f"{at}: an enum value repeated")
            for _ in range(_repeats(list(item.get("reserved", [])))):
                out.err("E042", f"{at}: a reserved code repeated")
            problem = _declaration_problem(item)
            if problem is not None:
                out.err(problem[0], f"{at}: {problem[1]}")
            else:
                fields.append(_field_of(item))
            read.append((item["name"], item.get("optional", False)))
        if not any(isinstance(x, dict) for x in items):
            out.err("E039", f"{here}: a record with no field")
        for _ in range(_repeats([x[0] for x in read if x != MARKER])):
            out.err("E042", f"{here}: a field name repeated")
        after_marker = False
        for i, x in enumerate(read):
            if x == MARKER:
                if i and read[i - 1] == MARKER:
                    out.err("E045", f"{here}: a marker right after another")
                after_marker = True
            elif after_marker and not x[1]:
                out.err("E045", f"{here}: field {x[0]!r} after a marker is not optional")
        if len(out.findings) == before:
            out.types[tname] = Record(tname, fields)
    return out


def record(doc: Any, name: str) -> Record:
    """A lint-clean document's type ``name``, for the codec. Raises
    ``ValueError`` when the type has a finding."""
    linted = lint(doc)
    if linted.findings or name not in linted.types:
        raise ValueError(f"{name}: not a clean packed type: {linted.findings}")
    return linted.types[name]


# -- in the envelope (§7.5, §7.6) ----------------------------------------------

def seal(rec: Record, value: Any, run: int, seq: int, width: int = 16, epoch: int | None = None) -> bytes:
    """A put's payload carrying ``value`` as ``rec``'s record: §7.5's
    mutation, CH's tag bits the type's tag ("An implementation that writes a
    record MUST write the type's tag in CH's tag bits")."""
    from . import zk2c

    return zk2c.write("mutation", width, run, seq, rec.tag, rec.write(value, epoch))


#: Core 0.76 §7.6 "The deployment epoch": "One that states none has E = 0,
#: the Unix epoch"; observer 0.16 §7.3: a tool "takes the one its deployment
#: states, and 0, the Unix epoch, where it states none". zk2py's owner and
#: readers hand the codec this where no epoch is given (F-433); the codec
#: itself, handed none, still refuses ``epoch`` as ``vectors.json`` asks
#: (F-435).
DEPLOYMENT_EPOCH = 0


def deployment_epoch(stated: int | None) -> int:
    """The epoch a writer or reader holds: the one its deployment states,
    else 0 (core 0.76 §7.6)."""
    return DEPLOYMENT_EPOCH if stated is None else stated


@dataclass(frozen=True)
class Reading:
    """A packed sample read as observer §7.3 (0.16) reads one: ``value``,
    its JSON reading, or ``refusal``, the token naming why it is
    undecodable, and ``why``."""

    value: dict[str, Any] | None
    refusal: str | None = None
    why: str = ""

    @property
    def undecodable(self) -> bool:
        return self.refusal is not None

    def says(self, name: str) -> str:
        return f"undecodable as packed:{name}: {self.refusal} ({self.why})"


def read_sample(rec: Record, read: Any, epoch: int | None = None) -> Reading:
    """A put's :class:`zk2py.zk2c.Read` read as ``rec``'s record, observer
    §7.3 (0.16): "A packed record is read through its type's layout, under
    the tag its envelope's CH carried (core §7.6). One its layout refuses is
    undecodable, and the tool names the refusal. One that came in no
    envelope has no tag, so it is undecodable too." A refused envelope is
    undecodable by its own token. ``time`` fields under ``epoch``, the
    deployment's, 0 where none is stated."""
    if not read.enveloped:
        return Reading(None, "no_envelope", "a packed record travels in core §7.5's envelope, which carries its tag")
    if read.refused:
        return Reading(None, read.refusal, f"a refused zk2c envelope: {read.why}")
    try:
        return Reading(rec.read(read.tag, read.record or b"", deployment_epoch(epoch)))
    except Refused as e:
        return Reading(None, e.token, str(e).split(": ", 1)[-1])


def open_put(rec: Record, payload: bytes, epoch: int | None = None) -> tuple[Any, dict[str, Any]]:
    """A put's payload read as ``rec``'s record: §7.5's parse, which reports
    the tag and checks it against no type, then §7.6's reader, whose first
    check is that tag. Returns (the envelope, the JSON reading); raises
    :class:`zk2c.Refused` or :class:`Refused`."""
    from . import zk2c

    env = zk2c.parse(payload, "put")
    return env, rec.read(env.tag, env.record or b"", epoch)

"""Core §7.7, frames (core 0.83, #981 K1): several records of one packed type
(§7.6) under one envelope and one time. Session-free; built on
:mod:`zk2py.packed`'s fields and codes. K1's static rules only: the
declaration, the tag, the body written and read. The owner's and the
reader's rules over a key's frames are K2's and K3's, and zk2py writes and
reads no frame on a bus yet.

**The declaration** (:class:`Declaration`, :func:`declaration`): ``records``
(1 by default), ``step_ms`` (exactly when ``records`` is above 1),
``jitter_ms`` (0 with a step, none without), ``keyframe_every`` (1) and
``deadband``, a field's name -> ``{delta, max_records}``. The span is
``records`` × ``step_ms``.

**The tag** (:class:`Layout`): the frame form, ``{"frame": {records, step_ms,
jitter_ms, deadband}, "record": <layout form>}``, its deadband a list in
field order, null or ``{delta, max_records}`` with ``delta`` in its shortest
form; the tag is the low four bits of the sha256 of its JCS bytes.
``keyframe_every`` and the fields' names are left out.

**The body**::

    body       time · count · record · … · record · pad
    time       42 bits: record 0's measured time; all ones when unknown
    count      the records less 1, in ⌈log₂ records⌉ bits
    record     full · when · offset · fields
    when       record 1 and later: 1 when scheduled
    offset     when `when` is 0: ms after record 0, in ⌈log₂ span⌉ bits
    fields     full: every field as §7.6 writes it; delta: a bit per field,
               then each field whose bit is 1, as §7.6 writes it

A value is the fixtures' (``conformance/frame/bodies.json``): ``{"time":
{"s", "ms"} | None, "records": [{"full", "at"?, "values", "absent"}]}``,
``at`` (record 1 and later) ``"scheduled"`` or an offset in ms.

**Reading** refuses, for the first that applies: ``tag``; then, as the
fields are read, ``length``, ``code`` and ``order``; then ``length`` for a
byte after the pad, and ``pad`` (:class:`Refused`). A record lies "before
the record before it" when its latest time is below that record's earliest;
a scheduled record lies from ``jitter_ms`` before its instant to
``jitter_ms`` after it.
"""

from __future__ import annotations

import hashlib
from dataclasses import dataclass, field
from fractions import Fraction
from typing import Any

from . import jcs, packed

UNKNOWN_S, UNKNOWN_MS = 2**32 - 1, 1023


class Refused(ValueError):
    """A frame the writer or the reader refuses, with its token."""

    def __init__(self, token: str, why: str = ""):
        super().__init__(f"{token}: {why}" if why else token)
        self.token = token


@dataclass(frozen=True)
class Deadband:
    delta: Fraction
    max_records: int


@dataclass(frozen=True)
class Declaration:
    """A ``frame`` table, resolved (§7.7 "The declaration")."""

    records: int = 1
    step_ms: int | None = None
    jitter_ms: int | None = None
    keyframe_every: int = 1
    deadband: dict[str, Deadband] = field(default_factory=dict)

    @property
    def span_ms(self) -> int | None:
        return None if self.step_ms is None else self.records * self.step_ms

    def canonical(self) -> dict[str, Any]:
        """The canonical form's ``frame`` member (core §9.5, 0.83)."""
        return {"records": self.records, "step_ms": self.step_ms, "jitter_ms": self.jitter_ms,
                "keyframe_every": self.keyframe_every,
                "deadband": {n: {"delta": packed.shortest(d.delta), "max_records": d.max_records}
                             for n, d in sorted(self.deadband.items())}}


def declaration(table: dict[str, Any]) -> Declaration:
    """A ``frame`` table as a contract writes it, its omitted keys at §7.7's
    defaults. The table is assumed lint-clean (§9.2's E048)."""
    step = table.get("step_ms")
    jitter = table.get("jitter_ms", 0 if step is not None else None)
    db = {n: Deadband(packed.read_decimal(v["delta"]), v["max_records"])
          for n, v in (table.get("deadband") or {}).items()}
    return Declaration(table.get("records", 1), step, jitter, table.get("keyframe_every", 1), db)


@dataclass
class Layout:
    """A frame resource's record type and declaration."""

    record: packed.Record
    decl: Declaration

    @property
    def fields(self) -> list[packed.Field]:
        return [f for f in self.record.items if isinstance(f, packed.Field)]

    def form(self) -> dict[str, Any]:
        deadband = []
        for f in self.fields:
            d = self.decl.deadband.get(f.name)
            deadband.append(None if d is None else {"delta": packed.shortest(d.delta), "max_records": d.max_records})
        return {"frame": {"records": self.decl.records, "step_ms": self.decl.step_ms,
                          "jitter_ms": self.decl.jitter_ms, "deadband": deadband},
                "record": self.record.layout_form()}

    def sha256(self) -> str:
        f = self.form()
        f["record"] = packed._as_jcs_numbers(f["record"])
        return hashlib.sha256(jcs.dumps(f)).hexdigest()

    @property
    def tag(self) -> int:
        """"An implementation that writes a frame MUST write its tag in CH's
        tag bits (§7.5)"."""
        return bytes.fromhex(self.sha256())[-1] & 0x0F

    @property
    def count_bits(self) -> int:
        return packed.bits_for(self.decl.records)

    @property
    def offset_bits(self) -> int | None:
        span = self.decl.span_ms
        return None if span is None else packed.bits_for(span)

    def expect(self) -> dict[str, Any]:
        """``layouts.json``'s ``expect``."""
        return {"form": self.form(), "sha256": self.sha256(), "tag": self.tag, "count_bits": self.count_bits,
                "offset_bits": self.offset_bits, "span_ms": self.decl.span_ms}

    # -- times ----------------------------------------------------------------------------

    def bounds(self, i: int, at: Any) -> tuple[int, int]:
        """Record ``i``'s earliest and latest time, in ms after record 0."""
        if i == 0:
            return 0, 0
        if at == "scheduled":
            assert self.decl.step_ms is not None
            mid, j = self.decl.step_ms * i, self.decl.jitter_ms or 0
            return mid - j, mid + j
        return at, at

    # -- writing --------------------------------------------------------------------------

    def write_bits(self, value: dict[str, Any], epoch: int | None = None) -> str:
        """The body's bits before the pad, or :class:`Refused`: ``records``,
        ``unknown``, ``missing``, ``value``, ``range`` (an offset at or past
        the span, a time outside its codes, and §7.6's) and ``order``."""
        recs = value.get("records", [])
        if not 1 <= len(recs) <= self.decl.records:
            raise Refused("records", f"{len(recs)} records, where a frame holds 1 to {self.decl.records}")
        out: list[str] = []
        t = value.get("time")
        if t is None:
            out.append(format(UNKNOWN_S, "032b") + format(UNKNOWN_MS, "010b"))
        else:
            if not (type(t.get("s")) is int and 0 <= t["s"] <= UNKNOWN_S and type(t.get("ms")) is int
                    and 0 <= t["ms"] <= 999):
                raise Refused("range", f"the time {t} has no code")
            out.append(format(t["s"], "032b") + format(t["ms"], "010b"))
        if self.count_bits:
            out.append(format(len(recs) - 1, f"0{self.count_bits}b"))
        names = [f.name for f in self.fields]
        prev: tuple[int, int] | None = None
        for i, r in enumerate(recs):
            values, absent = dict(r.get("values", {})), list(r.get("absent", []))
            for n in [*values, *absent]:
                if n not in names:
                    raise Refused("unknown", f"{n!r} is no field of {self.record.name}")
            out.append("1" if r.get("full") else "0")
            at: Any = None
            if i >= 1:
                at = r.get("at", "scheduled")
                if at == "scheduled":
                    out.append("1")
                else:
                    span = self.decl.span_ms
                    if type(at) is not int or span is None or not 0 <= at < span:
                        raise Refused("range", f"record {i}: an offset of {at}, where the span is {span}")
                    out.append("0" + format(at, f"0{self.offset_bits}b"))
            lo, hi = self.bounds(i, at)
            if prev is not None and hi < prev[0]:
                raise Refused("order", f"record {i} lies before the record before it")
            prev = (lo, hi)
            if not r.get("full"):
                out.append("".join("1" if f.name in values or f.name in absent else "0" for f in self.fields))
            for f in self.fields:
                included = r.get("full") or f.name in values or f.name in absent
                if not included:
                    continue
                if f.name in values:
                    code = packed.encode(f, values[f.name], epoch)  # packed.Refused: value, range
                    out.append(("1" if f.optional else "") + format(code, f"0{f.width}b"))
                elif not f.optional:
                    raise Refused("missing", f"record {i}: {f.name} is not optional")
                else:
                    out.append("0")
        return "".join(out)

    def write(self, value: dict[str, Any], epoch: int | None = None) -> bytes:
        bits = self.write_bits(value, epoch)
        bits += "0" * (-len(bits) % 8)
        return int(bits, 2).to_bytes(len(bits) // 8, "big") if bits else b""

    # -- reading --------------------------------------------------------------------------

    def read(self, tag: int, body: bytes, epoch: int | None = None) -> dict[str, Any]:
        """A frame's value, or :class:`Refused` for the first refusal in
        §7.7's order."""
        if tag != self.tag:
            raise Refused("tag", f"CH's tag {tag} is not the frame's {self.tag}")
        data = bytes(body)
        total, pos = len(data) * 8, 0
        whole = int.from_bytes(data, "big") if data else 0

        def take(n: int) -> int:
            nonlocal pos
            if pos + n > total:
                raise Refused("length", f"the body ends at bit {total}, before bit {pos + n}")
            v = (whole >> (total - pos - n)) & ((1 << n) - 1) if n else 0
            pos += n
            return v

        s, ms = take(32), take(10)
        if ms >= 1000 and not (s == UNKNOWN_S and ms == UNKNOWN_MS):
            raise Refused("code", f"a millisecond code of {ms}")
        count = take(self.count_bits)
        if count >= self.decl.records:
            raise Refused("code", f"a count of {count}, where a frame holds {self.decl.records}")
        recs: list[dict[str, Any]] = []
        prev: tuple[int, int] | None = None
        for i in range(count + 1):
            full = take(1) == 1
            rec: dict[str, Any] = {"full": full}
            at: Any = None
            if i >= 1:
                if take(1) == 1:
                    at = "scheduled"
                else:
                    assert self.offset_bits is not None and self.decl.span_ms is not None
                    at = take(self.offset_bits)
                    if at >= self.decl.span_ms:
                        raise Refused("code", f"record {i}: an offset of {at}, at or past the span")
                rec["at"] = at
            lo, hi = self.bounds(i, at)
            if prev is not None and hi < prev[0]:
                raise Refused("order", f"record {i} lies before the record before it")
            prev = (lo, hi)
            included = [True] * len(self.fields) if full else [take(1) == 1 for _ in self.fields]
            values: dict[str, Any] = {}
            absent: list[str] = []
            for f, inc in zip(self.fields, included):
                if not inc:
                    continue
                if f.optional and take(1) == 0:
                    absent.append(f.name)
                    continue
                try:
                    values[f.name] = packed.decode(f, take(f.width), epoch)
                except packed.Refused as e:
                    raise Refused(e.token, str(e)) from None
            rec["values"], rec["absent"] = values, absent
            recs.append(rec)
        used = (pos + 7) // 8
        if len(data) > used:
            raise Refused("length", f"{len(data) - used} byte(s) after the pad")
        if pos % 8 and whole & ((1 << (8 - pos % 8)) - 1):
            raise Refused("pad", "a pad bit is 1")
        return {"time": None if ms >= 1000 else {"s": s, "ms": ms}, "records": recs}

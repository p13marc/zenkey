"""Core §7.7, frames (core 0.83, #981 K1; the reader, observer 0.21, K2; the
owner, core 0.87, K3): several records of one packed type (§7.6) under one
envelope and one time. Session-free; built on :mod:`zk2py.packed`'s fields
and codes: the declaration, the tag, the body written and read
(:class:`Layout`); the reader's field states over one key's frames
(:class:`Reader`, round 97); and the owner's frames, gathered and written
(:class:`Framer`, round 97). :mod:`zk2py.frame_live` and
:meth:`zk2py.owner.Owner.record_frame` put them on a bus.

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


# -- the reader's fields (core §7.7, 0.83; observer §7.6, 0.21; #981 K2) ---------------------

SEQ_MOD = 2**24
#: why a field is unknown: the text's table, by its case (apply.json's ``why``)
WHY = ("first", "run", "gap", "rewind", "deadband", "no_value")


def follows(held_run: str | None, expected: int | None, run: str, seq: int) -> tuple[str, int | None]:
    """What a frame at ``run``/``seq`` follows, for a reader holding
    ``held_run`` and expecting ``expected`` (core §7.7 "The frames it
    expects"): ``first`` (no run held), ``run`` (another run, its width
    included), ``continues`` (the seq after the last record read), ``gap``
    with the records lost (a newer seq by §4.3's serial order: ahead by 1 to
    2^23 − 1), or ``rewind`` (any other seq, 2^23 ahead included)."""
    if held_run is None or expected is None:
        return "first", None
    if run != held_run:
        return "run", None
    d = (seq - expected) % SEQ_MOD
    if d == 0:
        return "continues", None
    if d < SEQ_MOD // 2:
        return "gap", d
    return "rewind", None


@dataclass
class FieldState:
    """One field's state (core §7.7's table): ``known`` with its value,
    ``within`` with the last value included (and its deadband's delta),
    ``unknown`` with why, or ``absent``. ``since`` counts the records after
    the last inclusion of a value, for a deadband; ``valued`` says that
    inclusion was a value, not a reserved code."""

    state: str = "unknown"
    value: Any = None
    why: str | None = "first"
    since: int = 0
    valued: bool = False

    def shown(self, delta: Fraction | None) -> dict[str, Any]:
        """The field as ``apply.json`` writes it, and as observer §7.6 has a
        tool show it: ``known`` its value, ``within`` the last value and its
        delta, ``unknown`` and ``absent`` their state's name and no value
        (core §7.7: "MUST NOT show or use an unknown or absent field as a
        value")."""
        if self.state == "known":
            return {"state": "known", "value": self.value}
        if self.state == "within":
            assert delta is not None
            return {"state": "within", "value": self.value, "delta": packed.shortest(delta)}
        if self.state == "absent":
            return {"state": "absent"}
        return {"state": "unknown", "why": self.why}


class Reader:
    """Core §7.7's **reader** of one frame key (a consumer or a tool): "It
    keeps one state for each field of the key, in the run it holds". Each
    frame is read in the order it arrives (:meth:`read`, a frame's run, its
    seq and its body's value; :meth:`read_put`, a put's envelope read first).

    - **What resets** (MUST): every field ``unknown`` at a gap, a rewind, and
      a frame of another run, the first included; then the frame is read.
    - **A record**, in order: an included field is ``known``, or ``absent``
      where its presence bit is 0. An omitted field with no deadband holds
      what it held in the record before, absent included (the owner includes
      it whenever its code or presence differs: ``apply.json``'s reading of
      §7.7's owner rule). An omitted field with a deadband is ``within``
      fewer than ``max_records`` records after an inclusion of a value, and
      ``unknown`` at the ``max_records``-th (``deadband``, MUST), or after an
      inclusion that was absent or a reserved code (``no_value``).
    - **Times**: record 0's is the frame's ``time``, ``unknown`` when all 42
      bits are set; a later record's is ``scheduled`` (``step_ms`` × i, its
      ``jitter_ms``, an instant and a bound, never a measurement) or
      ``measured`` (its offset), known when record 0's is not."""

    def __init__(self, layout: Layout) -> None:
        self.layout = layout
        self.run: str | None = None
        self.expected: int | None = None
        self.fields: dict[str, FieldState] = {f.name: FieldState() for f in layout.fields}
        #: frames read, and those counted undecodable with why
        self.read_count = 0
        self.undecodable: list[str] = []

    def _reset(self, why: str) -> None:
        for s in self.fields.values():
            s.state, s.value, s.why, s.since, s.valued = "unknown", None, why, 0, False

    def _record(self, r: dict[str, Any]) -> None:
        values, absent, full = r.get("values", {}), set(r.get("absent", [])), bool(r.get("full"))
        for f in self.layout.fields:
            s = self.fields[f.name]
            if full or f.name in values or f.name in absent:
                if f.name in values:
                    v = values[f.name]
                    s.state, s.value, s.why, s.since = "known", v, None, 0
                    s.valued = not (isinstance(v, str) and v in f.reserved)
                else:
                    s.state, s.value, s.why, s.since, s.valued = "absent", None, None, 0, False
                continue
            db = self.layout.decl.deadband.get(f.name)
            if db is None:
                continue  # holds what it held in the record before
            if s.state in ("known", "within") and s.valued:
                s.since += 1
                if s.since >= db.max_records:
                    s.state, s.why = "unknown", "deadband"
                    s.value = None
                else:
                    s.state = "within"
            elif s.state in ("known", "absent"):
                s.state, s.value, s.why = "unknown", None, "no_value"

    def _time(self, t: dict[str, int] | None, i: int, at: Any) -> dict[str, Any]:
        out: dict[str, Any] = {}
        if i == 0:
            out["time"] = "unknown" if t is None else "measured"
            after = 0
        elif at == "scheduled":
            assert self.layout.decl.step_ms is not None
            after = self.layout.decl.step_ms * i
            out.update(time="scheduled", after_ms=after, jitter_ms=self.layout.decl.jitter_ms or 0)
        else:
            after = at
            out.update(time="measured", after_ms=after)
        if t is not None:
            total = t["s"] * 1000 + t["ms"] + after
            out["at"] = {"s": total // 1000, "ms": total % 1000}
        return out

    def read(self, run: str, seq: int, value: dict[str, Any]) -> dict[str, Any]:
        """One frame, its body's value read (:meth:`Layout.read`): what it
        follows (``lost`` with a gap), then each record's seq, time and
        field states, as ``apply.json``'s ``expect`` writes them."""
        how, lost = follows(self.run, self.expected, run, seq)
        if how != "continues":
            self._reset(how)
        out: dict[str, Any] = {"run": run, "seq": seq, "follows": how}
        if lost is not None:
            out["lost"] = lost
        recs = value["records"]
        shown = []
        for i, r in enumerate(recs):
            self._record(r)
            row = {"seq": (seq + i) % SEQ_MOD, **self._time(value.get("time"), i, r.get("at"))}
            row["fields"] = {f.name: self.fields[f.name].shown(
                self.layout.decl.deadband[f.name].delta if f.name in self.layout.decl.deadband else None)
                for f in self.layout.fields}
            shown.append(row)
        out["records"] = shown
        self.run, self.expected = run, (seq + len(recs)) % SEQ_MOD
        self.read_count += 1
        return out

    def read_put(self, read: Any, epoch: int | None = None) -> dict[str, Any]:
        """A put on the frame key, its envelope read first (observer §7.3,
        a :class:`zk2py.zk2c.Read`): a frame is read with the layout; a
        mutation, a heartbeat, a refused envelope, no envelope, or a body
        the layout refuses has no reading and is counted undecodable, naming
        why (core §7.7 "No reading"; observer §7.6). Its seqs are not
        counted, so the next frame reads as following a gap."""
        if not read.enveloped:
            why = "no_envelope: a frame travels in core §7.5's envelope"
        elif read.refused:
            why = f"{read.refusal}: a refused zk2c envelope ({read.why})"
        elif read.form != "frame":
            why = f"no_reading: a {read.form} on a frame resource's key has no reading (core §7.7)"
        else:
            try:
                value = self.layout.read(read.tag, read.record or b"", epoch)
            except (Refused, packed.Refused) as e:
                why = f"{e.token}: {str(e).split(': ', 1)[-1]}"
            else:
                return self.read(read.stamp.run_hex(), read.stamp.seq, value)
        self.undecodable.append(why)
        return {"undecodable": why, "envelope": None if read.stamp is None else {
            "run": read.stamp.run_hex(), "seq": read.stamp.seq}}


def layout_of(contract: Any, resource: dict[str, Any]) -> Layout | None:
    """A loaded contract's frame resource as a :class:`Layout`: its record
    type and its canonical ``frame`` member (core §9.5), or None for a
    resource that is no frame resource or whose type does not resolve."""
    if resource.get("frame") is None or resource.get("type", {}).get("kind") != "packed":
        return None
    rec = contract.schemas.packed_record(resource["type"])
    return None if rec is None else Layout(rec, declaration(resource["frame"]))


# -- the owner's frames (core §7.7 "The owner's frames", 0.87; #981 K3) ----------------------

@dataclass
class _Measured:
    """One record as its owner measured it: every field's value, an absent
    optional field left out (§7.6's JSON reading), and its instant on the
    owner's monotonic clock."""

    values: dict[str, Any]
    mono_ns: int


@dataclass
class Ready:
    """A frame to put: its records as measured, and record 0's wall time
    (``{s, ms}`` from the deployment epoch, or None while the owner's clock
    is unknown)."""

    records: list[_Measured]
    time: dict[str, int] | None


class Framer:
    """One frame key's writer, session-free: it gathers an owner's records
    into frames (:meth:`record`, :meth:`end`) and writes each frame's value
    when it is put (:meth:`seal`), under the run it is put in.

    - **Frames** hold 1 to ``records`` records in the order measured. A frame
      is ready when it holds ``records``; "A record that would lie at or past
      its frame's span ends the frame, and starts the next one as its record
      0"; and "An owner MAY end a frame before it holds records records, and
      puts it then" (:meth:`end`, 0.87).
    - **Full records**: the first frame of a key in a run starts with one,
      and one comes at least once every ``keyframe_every`` records.
    - **Delta records**, against the record before: every field with no
      deadband whose code or presence differs is included. A field with a
      deadband is omitted only while it is present and holds a value, not a
      reserved code, so did the last record that included it, the two differ
      by at most ``delta``, and fewer than ``max_records`` records have
      passed since that inclusion: "An owner MUST include the field in any
      other case, and in at least one of every max_records records".
    - **Times**: record 0's from the wall clock; a later record's offset on
      the monotonic clock from record 0, in whole milliseconds, ``scheduled``
      only when within ``jitter_ms`` of ``step_ms`` × i."""

    def __init__(self, layout: Layout, epoch: int = 0) -> None:
        self.layout = layout
        self.epoch = epoch
        self.open: list[_Measured] = []
        self.open_time: dict[str, int] | None = None
        #: the run the state below belongs to; the record before, by field
        #: (its code, or None for absent); records since the last full one;
        #: each deadband field's last inclusion (its value or None for no
        #: value) and the records since it
        self.run: Any = None
        self.prev: dict[str, int | None] | None = None
        self.since_full = 0
        self.included: dict[str, tuple[Any, int]] = {}

    def _offset(self, m: _Measured) -> int:
        return (m.mono_ns - self.open[0].mono_ns) // 1_000_000

    def record(self, values: dict[str, Any], mono_ns: int, wall: dict[str, int] | None) -> list[Ready]:
        """One record measured at ``mono_ns`` (``wall``, the wall clock then,
        as a frame's time, or None while unknown) -> the frames now ready, in
        order: the open frame its span ends, and this record's own frame when
        it is full."""
        fields = {f.name: f for f in self.layout.fields}
        for n in values:
            if n not in fields:
                raise Refused("unknown", f"{n!r} is no field of {self.layout.record.name}")
        for f in self.layout.fields:
            if not f.optional and f.name not in values:
                raise Refused("missing", f"{f.name} is not optional")
            if f.name in values:
                packed.encode(f, values[f.name], self.epoch)  # packed.Refused: value, range
        out: list[Ready] = []
        span = self.layout.decl.span_ms
        m = _Measured(dict(values), mono_ns)
        if self.open and (span is None or self._offset(m) >= span):
            out.append(self.end_open())
        if not self.open:
            self.open_time = wall
        self.open.append(m)
        if len(self.open) >= self.layout.decl.records:
            out.append(self.end_open())
        return out

    def end_open(self) -> Ready:
        r = Ready(self.open, self.open_time)
        self.open, self.open_time = [], None
        return r

    def end(self) -> Ready | None:
        """§7.7's MAY: the open frame ended before it holds ``records``
        records, to be put now; None with none open."""
        return self.end_open() if self.open else None

    def _code(self, name: str, values: dict[str, Any]) -> int | None:
        f = next(x for x in self.layout.fields if x.name == name)
        return None if name not in values else packed.encode(f, values[name], self.epoch)

    def seal(self, ready: Ready, run: Any) -> dict[str, Any]:
        """The frame's value (``bodies.json``'s spelling, for
        :meth:`Layout.write`) as its owner writes it, put under ``run``: the
        first frame of a run starts with a full record."""
        if run != self.run:
            self.run, self.prev, self.since_full, self.included = run, None, 0, {}
        decl, recs = self.layout.decl, []
        for i, m in enumerate(ready.records):
            full = self.prev is None or self.since_full + 1 >= decl.keyframe_every
            codes = {f.name: self._code(f.name, m.values) for f in self.layout.fields}
            include: set[str] = set()
            for f in self.layout.fields:
                if full:
                    include.add(f.name)
                    continue
                db = decl.deadband.get(f.name)
                if db is None:
                    if codes[f.name] != self.prev[f.name]:
                        include.add(f.name)
                    continue
                v = m.values.get(f.name)
                last = self.included.get(f.name)
                valued = f.name in m.values and not (isinstance(v, str) and v in f.reserved)
                if not (valued and last is not None and last[0] is not None
                        and abs(packed.read_decimal(str(v)) - packed.read_decimal(str(last[0]))) <= db.delta
                        and last[1] + 1 < db.max_records):
                    include.add(f.name)
            for f in self.layout.fields:
                if f.name not in decl.deadband:
                    continue
                if f.name in include:
                    v = m.values.get(f.name)
                    valued = f.name in m.values and not (isinstance(v, str) and v in f.reserved)
                    self.included[f.name] = (v if valued else None, 0)
                elif f.name in self.included:
                    v, n = self.included[f.name]
                    self.included[f.name] = (v, n + 1)
            self.since_full = 0 if full else self.since_full + 1
            self.prev = codes
            rec: dict[str, Any] = {"full": full,
                                   "values": {n: m.values[n] for n in sorted(include) if n in m.values},
                                   "absent": [f.name for f in self.layout.fields
                                              if f.name in include and f.name not in m.values]}
            if i >= 1:
                off = (m.mono_ns - ready.records[0].mono_ns) // 1_000_000
                step, jitter = decl.step_ms, decl.jitter_ms or 0
                rec["at"] = "scheduled" if step is not None and abs(off - step * i) <= jitter else off
            recs.append(rec)
        return {"time": ready.time, "records": recs}

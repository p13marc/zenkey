"""``telemetry.v2`` (``spec/profiles/telemetry/v2.md``, text 0.1, core 0.87;
#609, round 98; #983): a frame's fields as metric series. Session-free.

``telemetry.v2`` restates ``telemetry.v1`` 0.7's §2.1 to §2.15 by reference on
every resource that is no frame resource (:mod:`zk2py.telemetry` reads
those), and adds §2.16 to §2.21 on a frame resource (core §7.7), which this
module reads:

- **the declaration** (§2.16, §5's first question): :func:`declare_frame`,
  ``telemetry.series`` read alone, v1's four keys meaning nothing on a frame
  resource; ``declared`` with each series' field, kind, unit, semantic,
  mirror and width in field order, ``invalid`` with every entry's reasons in
  byte order of its key, each check made alone and a check that needs a
  wrong value skipped, ``ignored`` or ``not_this_profiles``. A mirror lends
  the kind, unit and semantic of its v1 declaration, and is a label, never
  the series' identity (§2.17);
- **a record's reading** (§2.18, §5's second question): :func:`reading`, over
  :class:`zk2py.frame.Reader`'s field states: a ``sample`` where the record
  included the field with a code that is a value; ``held`` where it omitted
  it and the field holds a value; ``absent``, ``owner_error``,
  ``beyond_range``, ``unnamed`` or ``unknown`` (with why) for no value, never
  0 and never a finding;
- **a pair** (§2.19 to §2.21, §5's third and fourth questions):
  :func:`pair`, two consecutive samples of one counter series, of one epoch
  when no frame between them was read as another run's or as a rewind, the
  epoch always established in band; a narrow counter read in its serial
  window, ``wrap_or_decrease`` at or past 2^(w-1), v1's ``decreased`` at 64
  bits; the time base the records' offsets within one frame, their instants
  across two, else the arrivals, its basis named ``measured``, ``scheduled``
  (with the scheduled records' ``jitter_ms`` summed, its bound) or
  ``arrival``; v1 §2.11's formula;
- **a key's frames** (:class:`SeriesReader`): one frame reader per frame key,
  each frame's records read, and the pairs each counter series forms;
- **over many pairs** (§5): :func:`series_answer`;
- **the owner** (§2.19, §2.20): :class:`Meter`, which refuses a counter's
  count below one it wrote before on that key within one run, the count
  before the reduction, and leaves the reduction modulo 2^w to the field's
  writer (core §7.6, :func:`zk2py.packed.encode`).
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any, Iterable

from . import frame, packed, telemetry as tm

PROFILE = "telemetry.v2"
SERIES = "telemetry.series"
#: §2.16's inline series: "a table of kind, unit and semantic"
INLINE = ("kind", "unit", "semantic")
#: §2.16 "The carriers": a field's packed type carries one kind
CARRIERS = {"counter": ("counter",), "gauge": ("int", "fixed"), "boolean": ("bool",), "text": ("enum",)}
#: §2.16's reasons, in the order each entry's checks are made
REASONS = ("series_field", "series_value", "kind_value", "unit_value", "semantic_value", "mirror_missing",
           "mirror_invalid", "kind_missing", "not_a_carrier")
#: §2.18's table: what a no-value code reads as
NO_VALUE = {"not_available": "absent", "error": "owner_error", "out_of_range": "beyond_range"}
SAMPLE, HELD = "sample", "held"

_MISSING = object()


def uses_v2(canonical: dict[str, Any]) -> bool:
    return PROFILE in canonical.get("uses", [])


# -- the declaration (§2.16) -------------------------------------------------------------

@dataclass(frozen=True)
class Other:
    """A resource of the interface as a mirror may name it: its kind token,
    its template, whether it is a frame resource, and its ``telemetry.v1``
    declaration (None for a frame resource)."""

    token: str
    template: str
    frame: bool
    declaration: tm.Declaration | None


@dataclass(frozen=True)
class Series:
    """One series field's entry, resolved (§2.16, §2.17)."""

    field: str
    kind: str
    unit: str | None
    semantic: str | None
    mirror: str | None
    width: int | None
    type: str = "counter"

    def expect(self) -> dict[str, Any]:
        return {"field": self.field, "kind": self.kind, "unit": self.unit, "semantic": self.semantic,
                "mirror": self.mirror, "width": self.width}

    def label(self, resource: str) -> str:
        """§2.17 "The mirror is a label": "`sdu/dropped_total` (`frames/link`)"
        for a mirrored series, the field and its resource otherwise."""
        return f"{self.mirror} ({resource})" if self.mirror is not None else f"{self.field} ({resource})"


@dataclass(frozen=True)
class FrameDeclaration:
    """§5's first question on a frame resource."""

    declaration: str
    series: tuple[Series, ...] = ()
    reasons: tuple[tuple[str | None, str], ...] = ()

    def expect(self) -> dict[str, Any]:
        if self.declaration == tm.DECLARED:
            return {"declaration": tm.DECLARED, "series": [s.expect() for s in self.series]}
        if self.declaration == tm.INVALID:
            return {"declaration": tm.INVALID, "reasons": [{"entry": e, "reason": r} for e, r in self.reasons]}
        return {"declaration": self.declaration}

    def of(self, name: str) -> Series | None:
        return next((s for s in self.series if s.field == name), None)


def _semantic_ok(s: Any) -> bool:
    return isinstance(s, str) and s in tm.SEMANTICS


def declare_frame(fields: list[packed.Field], annotations: dict[str, Any], others: Iterable[Other]) -> FrameDeclaration:
    """§2.16 over a frame resource: its record type's fields, its resolved
    annotations (core §9.3: v1's four keys, wherever they resolve from,
    mean nothing here), and the interface's other resources, which a mirror
    names by template, byte for byte."""
    if SERIES not in annotations:
        # "A frame resource with no telemetry.series is not this profile's.
        # It is ignored when it resolves one of v1's four keys, and
        # not_this_profiles otherwise."
        return FrameDeclaration(tm.IGNORED if any(k in annotations for k in tm.KEYS) else tm.NOT_THIS)
    table = annotations[SERIES]
    if not isinstance(table, dict) or not table:
        # "The annotation whole is checked first … Nothing else is then checked."
        return FrameDeclaration(tm.INVALID, reasons=((None, "series_value"),))
    by_name = {f.name: f for f in fields}
    others = list(others)
    reasons: list[tuple[str | None, str]] = []
    resolved: dict[str, Series] = {}
    for key in sorted(table, key=lambda k: k.encode("utf-8")):
        value, f, rs = table[key], by_name.get(key), []
        if f is None:
            rs.append("series_field")
        kind: Any = None
        unit = semantic = mirror = None
        if isinstance(value, str):
            mirror = value
            named = [o for o in others if o.template == value]
            if (len(named) != 1 or named[0].frame or "{" in value or named[0].declaration is None
                    or named[0].declaration.declaration in (tm.NOT_THIS, tm.IGNORED)):
                rs.append("mirror_missing")
            elif named[0].declaration.declaration == tm.INVALID:
                rs.append("mirror_invalid")
            elif named[0].declaration.kind is None:
                # "a mirror that declares no kind, a Point with no telemetry.kind"
                rs.append("kind_missing")
            else:
                d = named[0].declaration
                kind, unit, semantic = d.kind, d.unit, d.semantic
        elif isinstance(value, dict):
            if set(value) - set(INLINE):
                rs.append("series_value")
            k, u, s = (value.get(m, _MISSING) for m in INLINE)
            kind_ok = k is _MISSING or (isinstance(k, str) and k in tm.KINDS)
            if not kind_ok:
                rs.append("kind_value")
            if u is not _MISSING and not tm.is_unit(u):
                rs.append("unit_value")
            if s is not _MISSING and not _semantic_ok(s):
                rs.append("semantic_value")
            if k is _MISSING:
                rs.append("kind_missing")
            elif kind_ok:
                kind = k
            unit = None if u is _MISSING else u
            semantic = None if s is _MISSING else s
        else:
            # "A value that is neither skips checks 3 to 7."
            rs.append("series_value")
        if kind is not None and f is not None and f.type not in CARRIERS.get(kind, ()):
            rs.append("not_a_carrier")
        if rs:
            reasons += [(key, r) for r in rs]
        else:
            assert f is not None
            resolved[key] = Series(key, kind, unit, semantic, mirror,
                                   f.counter_width if f.type == "counter" else None, f.type)
    if reasons:
        return FrameDeclaration(tm.INVALID, reasons=tuple(reasons))
    return FrameDeclaration(tm.DECLARED, tuple(resolved[f.name] for f in fields if f.name in resolved))


def others_of(resources: Iterable[dict[str, Any]]) -> list[Other]:
    """The fixtures' ``resources`` (``declarations.json``) as :class:`Other`."""
    out = []
    for r in resources:
        is_frame = bool(r.get("frame"))
        out.append(Other(r.get("token", r["kind"]), r["template"], is_frame, None if is_frame else
                         tm.declare(r["kind"], r.get("annotations") or {}, r.get("type"), r.get("attachment"))))
    return out


def contract_declarations(c: Any) -> dict[str, FrameDeclaration | tm.Declaration]:
    """Every resource of a loaded contract by template: a frame resource's
    §2.16 declaration (when the contract uses ``telemetry.v2``; else v1's
    reading, which reads no frame resource as a metric resource), any
    other's ``telemetry.v1`` declaration."""
    v1 = tm.contract_declarations(c)
    others = [Other(r["token"], r["template"], r.get("frame") is not None,
                    None if r.get("frame") is not None else v1[f"{r['token']}/{r['template']}"])
              for r in c.canonical["resources"]]
    out: dict[str, FrameDeclaration | tm.Declaration] = {}
    for r in c.canonical["resources"]:
        lo = frame.layout_of(c, r)
        if r.get("frame") is not None and lo is not None and uses_v2(c.canonical):
            out[r["template"]] = declare_frame(lo.fields, r.get("annotations") or {}, others)
        else:
            out[r["template"]] = v1[f"{r['token']}/{r['template']}"]
    return out


def lint(c: Any) -> list[str]:
    """§2.16 "A contract linter SHOULD warn of one": a warning, no core code
    (§5), per frame resource whose ``telemetry.series`` is invalid, naming
    each entry's reasons."""
    if not uses_v2(c.canonical):
        return []
    out = []
    for template, d in contract_declarations(c).items():
        if isinstance(d, FrameDeclaration) and d.declaration == tm.INVALID:
            out.append(f"{template}: telemetry.series is invalid (telemetry.v2 §2.16): "
                       + ", ".join(f"{e or '(the annotation)'} {r}" for e, r in d.reasons))
    return out


# -- a record's reading (§2.18) -------------------------------------------------------------

def no_value(f: packed.Field, v: Any) -> str | None:
    """§2.18's table for a field's JSON reading: the token of a no-value
    code (``not_available``, ``error``, ``out_of_range``, an enum's escape),
    or None for a value."""
    if isinstance(v, str) and v in f.reserved:
        return NO_VALUE[v]
    if f.type == "enum" and v is None:
        return "unnamed"
    return None


def reading(f: packed.Field, s: Series, state: dict[str, Any], included: bool) -> dict[str, Any]:
    """What one record says of one series (§2.18, ``readings.json``): the
    field's state after the record (:meth:`zk2py.frame.FieldState.shown`)
    and whether the record included it."""
    st = state["state"]
    if st == "absent":
        return {"reading": "absent"}
    if st == "unknown":
        return {"reading": "unknown", "why": state["why"]}
    nv = no_value(f, state["value"])
    if nv is not None:
        return {"reading": nv}
    if st == "known" and included:
        return {"reading": SAMPLE, "kind": s.kind, "value": state["value"]}
    return {"reading": HELD, "state": st, "value": state["value"]}


# -- a pair (§2.19 to §2.21) ------------------------------------------------------------------

@dataclass(frozen=True)
class Sample:
    """One sample of a counter series as a pair reads it: where it is (the
    frame's index among those read, the record's in its frame), its count
    as the frame carries it, its width, the epoch it was read under (a
    count of the frames read as another run's or as a rewind), and its time:
    the frame's instant in ms (None when unknown), the record's offset,
    whether it is scheduled and its ``jitter_ms``, and the frame's
    arrival."""

    frame: int
    record: int
    value: int
    width: int
    epoch: int
    frame_ms: int | None
    after_ms: int
    scheduled: bool
    jitter_ms: int
    arrival_ns: int

    @property
    def instant_ms(self) -> int | None:
        return None if self.frame_ms is None else self.frame_ms + self.after_ms


@dataclass(frozen=True)
class Pair:
    """§5's third and fourth questions over one pair (``continuity.json``)."""

    field: str
    earlier: tuple[int, int]
    later: tuple[int, int]
    continuity: str
    epoch: str | None
    via: str | None
    why: str | None
    rate: float | None
    rate_unit: str | None
    elapsed_ns: int | None
    time_basis: str | None
    jitter_ms: int | None
    no_rate: str | None

    def expect(self) -> dict[str, Any]:
        return {"field": self.field, "earlier": {"frame": self.earlier[0], "record": self.earlier[1]},
                "later": {"frame": self.later[0], "record": self.later[1]}, "continuity": self.continuity,
                "epoch": self.epoch, "via": self.via, "why": self.why, "rate": self.rate,
                "rate_unit": self.rate_unit, "elapsed_ns": self.elapsed_ns, "time_basis": self.time_basis,
                "jitter_ms": self.jitter_ms, "no_rate": self.no_rate}


def elapsed(a: Sample, b: Sample) -> tuple[int, str, int | None]:
    """§2.21 "The time base", the first that applies, as (nanoseconds, the
    basis, the bound): in one frame, the offsets; in two frames with both
    instants known, the instants; else the arrivals. ``scheduled`` when
    either record is, its bound the scheduled records' ``jitter_ms`` summed."""
    if a.frame == b.frame:
        e = (b.after_ms - a.after_ms) * 10**6
    elif a.instant_ms is not None and b.instant_ms is not None:
        e = (b.instant_ms - a.instant_ms) * 10**6
    else:
        return b.arrival_ns - a.arrival_ns, "arrival", None
    if a.scheduled or b.scheduled:
        return e, "scheduled", (a.jitter_ms if a.scheduled else 0) + (b.jitter_ms if b.scheduled else 0)
    return e, "measured", None


def pair(name: str, a: Sample, b: Sample, unit: str | None) -> Pair:
    """§2.19 to §2.21 over two consecutive samples of one counter series."""
    where = ((a.frame, a.record), (b.frame, b.record))
    if a.width != b.width:
        # §2.20: "Two counter samples of two widths are incomparable, v1
        # §2.10's unobservable, with no rate (kind)."
        return Pair(name, *where, tm.UNOBSERVABLE, None, None, "incomparable", None, None, None, None, None, "kind")
    if a.epoch != b.epoch:
        # §2.19: "A pair under two epochs is a reset, whatever its values".
        return Pair(name, *where, "reset", "established", "in_band", None, None, None, None, None, None, "reset")
    if a.width < 64:
        d = (b.value - a.value) % (1 << a.width)
        if d >= 1 << (a.width - 1):
            return Pair(name, *where, tm.UNOBSERVABLE, "established", "in_band", "wrap_or_decrease", None, None,
                        None, None, None, "wrap_or_decrease")
    else:
        if b.value < a.value:
            return Pair(name, *where, "decreased", "established", "in_band", None, None, None, None, None, None,
                        "decreased")
        d = b.value - a.value
    e, basis, bound = elapsed(a, b)
    if e <= 0:
        return Pair(name, *where, "same_epoch", "established", "in_band", None, None, None, None, None, None,
                    "elapsed")
    return Pair(name, *where, "same_epoch", "established", "in_band", None, tm.rate_of(d, e),
                None if unit is None else f"{unit}/s", e, basis, bound, None)


# -- one key's frames ------------------------------------------------------------------------

class SeriesReader:
    """One frame key's reader (``scenarios-v2.md``'s R): a
    :class:`zk2py.frame.Reader` for its field states, each record read by
    §2.18, and each counter series' pairs, two consecutive samples
    whatever was omitted, unknown or lost between them. The epoch moves at
    each frame read as another run's or as a rewind (core §7.7's
    ``follows``); a gap keeps it."""

    def __init__(self, layout: frame.Layout, decl: FrameDeclaration) -> None:
        self.layout, self.decl = layout, decl
        self.frames = frame.Reader(layout)
        self.fields = {f.name: f for f in layout.fields}
        self.epoch = 0
        self.count = 0
        self.last: dict[str, Sample] = {}
        self.pairs: list[Pair] = []
        #: every sample, by field: (frame index, record index, value)
        self.samples: dict[str, list[tuple[int, int, Any]]] = {s.field: [] for s in decl.series}
        self.undecodable: list[str] = []

    def read(self, run: str, seq: int, value: dict[str, Any], arrival_ns: int = 0) -> dict[str, Any]:
        """One frame, its body's value read: ``readings.json``'s ``expect``
        (``follows``, ``lost`` with a gap, and each record's ``seq`` and
        series fields), the pairs it closed in ``pairs``."""
        got = self.frames.read(run, seq, value)
        if got["follows"] in ("run", "rewind"):
            self.epoch += 1
        index, self.count = self.count, self.count + 1
        t = value.get("time")
        frame_ms = None if t is None else t["s"] * 1000 + t["ms"]
        closed: list[Pair] = []
        records = []
        for i, (r, row) in enumerate(zip(value["records"], got["records"])):
            full, values, absent = bool(r.get("full")), r.get("values", {}), set(r.get("absent", []))
            fields = {}
            for s in self.decl.series:
                f = self.fields[s.field]
                included = full or s.field in values or s.field in absent
                rd = reading(f, s, row["fields"][s.field], included)
                fields[s.field] = rd
                if rd["reading"] != SAMPLE:
                    continue
                self.samples[s.field].append((index, i, rd["value"]))
                if s.kind != "counter":
                    continue
                smp = Sample(index, i, rd["value"], s.width or 64, self.epoch, frame_ms, row.get("after_ms", 0),
                             row.get("time") == "scheduled", row.get("jitter_ms") or 0, arrival_ns)
                before = self.last.get(s.field)
                if before is not None:
                    p = pair(s.field, before, smp, s.unit)
                    closed.append(p)
                    self.pairs.append(p)
                self.last[s.field] = smp
            records.append({"seq": row["seq"], "fields": fields})
        out: dict[str, Any] = {"follows": got["follows"]}
        if "lost" in got:
            out["lost"] = got["lost"]
        out["records"] = records
        out["pairs"] = [p.expect() for p in closed]
        return out

    def read_put(self, read: Any, arrival_ns: int = 0, epoch: int | None = None) -> dict[str, Any]:
        """A put on the frame key, its envelope read first (observer §7.3):
        a frame is read with the layout; anything else has no reading and
        is counted undecodable, its seqs uncounted (core §7.7)."""
        why = None
        if not read.enveloped:
            why = "no_envelope: a frame travels in core §7.5's envelope"
        elif read.refused:
            why = f"{read.refusal}: a refused zk2c envelope ({read.why})"
        elif read.form != "frame":
            why = f"no_reading: a {read.form} on a frame resource's key has no reading (core §7.7)"
        else:
            try:
                value = self.layout.read(read.tag, read.record or b"", epoch)
            except (frame.Refused, packed.Refused) as e:
                why = f"{e.token}: {str(e).split(': ', 1)[-1]}"
            else:
                return self.read(read.stamp.run_hex(), read.stamp.seq, value, arrival_ns)
        self.undecodable.append(why)
        return {"undecodable": why}


# -- over many pairs (§5) --------------------------------------------------------------------

def series_answer(kind: str, samples: int, pairs: Iterable[Pair]) -> str:
    """§5 "Over many pairs", read for one series: ``not_asked`` for no
    counter; ``established`` (the finding) on a pair ``decreased``;
    ``could_not_tell`` when it formed no pair (fewer than two samples, or
    unknown throughout), or when one of its pairs is ``wrap_or_decrease``
    and none is ``decreased``; ``not_established`` otherwise."""
    ps = list(pairs)
    if kind != "counter":
        return tm.NOT_ASKED
    if any(p.continuity == "decreased" for p in ps):
        return tm.DECREASED_ESTABLISHED
    if samples < 2 or not ps or any(p.why == "wrap_or_decrease" for p in ps):
        return tm.COULD_NOT_TELL
    return tm.NOT_ESTABLISHED


# -- the owner (§2.19, §2.20) -----------------------------------------------------------------

class Decreased(frame.Refused):
    """§2.19: "Within one run, an owner MUST NOT write a counter field's
    count below one it wrote before on that key.\""""

    def __init__(self, field_: str, before: int, after: int):
        super().__init__("decreased", f"field={field_} before={before} after={after}: a counter's count below one "
                                      "written before on this key within one run (telemetry.v2 §2.19)")
        self.field, self.before, self.after = field_, before, after


@dataclass
class Meter:
    """The owner's half of one frame key (§2.19): the counts it wrote in the
    run it writes under, by counter series field. :meth:`check` refuses a
    record whose count is below one written before in that run, before
    any reduction; :meth:`accept` records the counts of a record put. A new
    run starts the counts again: "An owner MUST write a counter field's
    samples under a new run from the moment the counter restarts"."""

    decl: FrameDeclaration
    run: Any = None
    counts: dict[str, int] = field(default_factory=dict)
    #: §2.20's SHOULD broken: "An owner SHOULD declare each counter field
    #: wide enough that its increase between two samples of its series stays
    #: below 2^(w-1)", each increase at or past it named (the owner writes it
    #: all the same: a SHOULD on its declaration, not a refusal)
    warnings: list[str] = field(default_factory=list)

    def check(self, run: Any, values: dict[str, Any]) -> None:
        if run != self.run:
            self.run, self.counts = run, {}
        for s in self.decl.series:
            if s.kind != "counter" or s.field not in values:
                continue
            v = values[s.field]
            if isinstance(v, int) and not isinstance(v, bool) and s.field in self.counts and v < self.counts[s.field]:
                raise Decreased(s.field, self.counts[s.field], v)

    def accept(self, run: Any, values: dict[str, Any]) -> None:
        if run != self.run:
            self.run, self.counts = run, {}
        for s in self.decl.series:
            v = values.get(s.field)
            if s.kind == "counter" and isinstance(v, int) and not isinstance(v, bool):
                last = self.counts.get(s.field)
                w = s.width or 64
                if last is not None and w < 64 and v - last >= 1 << (w - 1):
                    self.warnings.append(f"telemetry.v2 §2.20: {s.field}'s increase {v - last} since its last "
                                         f"record is at or past 2^{w - 1}: a reader reads wrap_or_decrease, with "
                                         "no rate; declare the field wider")
                self.counts[s.field] = max(v, last if last is not None else v)

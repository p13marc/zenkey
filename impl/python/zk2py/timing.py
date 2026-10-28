"""``timing.v1``: a stream's cadence, its deadline and its lifespan (profile
text 0.2, draft, written against core 0.55; ``spec/profiles/timing/v1.md``).

Read cold from the text, its fixtures and its scenarios alone (#609, round
76; text 0.2 in round 78). The session-free half:
- :func:`declaration` reads a resolved resource's three keys (§2.2, §2.3),
  as ``conformance/declarations.json`` does, and :func:`requirement` a
  ``[requires.<role>]``'s (§2.3: ignored at 1.0);
- :class:`Tracker` is one reader's deadline tracker for one provider and
  resource (§2.5): edge-triggered ``missed`` and ``met``, ``ended`` (gone
  or forgotten) and ``beyond_bound`` (once, text 0.2's MUST), its due and
  its detection bound, an epoch template's known member waited for from
  its first member token seen (text 0.2, F-261), as
  ``conformance/deadlines.json`` drives it; :func:`tracker_for` builds one
  only for a declaration that judges a deadline (text 0.2, F-259);
- :class:`LifespanReading` judges each sample at its arrival (§2.6), on
  the deployment's ε only, against the owner's own stamp, with a clock
  falsified for the rest of the reading once a stamp of it is more than ε
  ahead, as ``conformance/lifespans.json`` does;
- :func:`period` is a tool's period over a window (§2.7), as
  ``conformance/periods.json`` does;
- :func:`conclude` is §5's three questions of one member over one window,
  as ``conformance/conclusions.json`` does, and :func:`rollup` the answer
  over a resource or a run, by each question's finding (F-264);
- :func:`lint` is the profile's contract linter: the SHOULDs of §2.2, §2.3
  and §2.5, which have no core code.

Times are integers of nanoseconds on the clock each rule names: the
fixtures give milliseconds "exact to the nanosecond", read here through
their shortest decimal spelling (:func:`ms_to_ns`).

The session half (a consumer's deadline events and its lifespan discard,
a tool's window, an owner's member tokens) is :mod:`zk2py.timing_live`.
"""

from __future__ import annotations

import math
import re
from dataclasses import dataclass, field
from decimal import ROUND_HALF_EVEN, Decimal
from typing import Any, Iterable

PROFILE = "timing.v1"
PERIOD, DEADLINE, LIFESPAN = "timing.period_ms", "timing.deadline_ms", "timing.lifespan_ms"
#: §4: the vocabulary, in §2.2's order of checks.
KEYS = (PERIOD, DEADLINE, LIFESPAN)
#: §2.2: "each that fails is a reason: period_value, deadline_value, lifespan_value".
REASONS = {PERIOD: "period_value", DEADLINE: "deadline_value", LIFESPAN: "lifespan_value"}
#: §2.2: "from 1 to 2^53−1".
MAX_MS = 2**53 - 1
MS = 1_000_000
#: §2.5 "Ended … it is forgotten: … its last arrival was an hour or more
#: before" (core §2.7's liveness span of a stream member).
FORGET_NS = 3_600_000 * MS

# Declarations (declarations.json's expect.declaration).
NOT_THIS_PROFILES, IGNORED, INVALID, DECLARED = "not_this_profiles", "ignored", "invalid", "declared"
# The owner's token as the reader observes it (v1.md §0); "unseen" before any
# state is known (deadlines.json: "unseen when no state was given").
UNSEEN, PRESENT, UNCONFIRMED, ABSENT = "unseen", "present", "unconfirmed", "absent"
# §5's answers.
YES, NO, UNOBSERVABLE, NOT_ASKED = "yes", "no", "unobservable", "not_asked"
# Lifespan verdicts (lifespans.json).
LIVE, EXPIRED = "live", "expired"
# The period over a window (periods.json).
KEPT, NOT_KEPT, TOO_SHORT, NOT_HEARD = "kept", "not_kept", "too_short", "not_heard"
#: §5 "Finding on": which answer of each question is the finding.
FINDING = {"deadline": NO, "period": NO, "lifespan": YES}


# -- milliseconds, exactly --------------------------------------------------------------

def ms_to_ns(value: int | float | Decimal) -> int:
    """Milliseconds as a fixture writes them, to nanoseconds, exactly: a float
    is read through its shortest decimal spelling, so ``110.000001`` is
    110,000,001 ns and ``-1e-06`` is −1 ns."""
    if isinstance(value, bool):
        raise TypeError("a boolean is not a number of milliseconds")
    if isinstance(value, int):
        return value * MS
    d = value if isinstance(value, Decimal) else Decimal(repr(float(value)))
    return int((d * MS).to_integral_value(rounding=ROUND_HALF_EVEN))


def ns_to_ms(ns: int) -> int | float:
    """Nanoseconds back to milliseconds as a fixture writes them: an integer
    when whole, else the double nearest the exact value."""
    if ns % MS == 0:
        return ns // MS
    return float(Decimal(ns) / MS)


# -- §2.2, §2.3 the declaration ---------------------------------------------------------

def value_ms(value: Any) -> int | None:
    """§2.2: the value in milliseconds, or None when it is no value. "Each is
    a whole number of milliseconds, from 1 to 2^53−1. An integral float is
    that integer"; "Not a value: 0, -0.0, a negative number, a fraction, a
    number above 2^53−1, a string, a boolean, an array or a table"."""
    if isinstance(value, bool):
        return None
    if isinstance(value, int):
        return value if 1 <= value <= MAX_MS else None
    if isinstance(value, float):
        if not math.isfinite(value) or value != math.floor(value) or not 1 <= value <= MAX_MS:
            return None
        return int(value)
    return None


def kind_of(kind: str) -> str:
    """An authoring kind or a kind token, as an authoring kind: ``@stream``
    is a stream (§2.3, "plain or explicit")."""
    return {"@stream": "stream", "@state": "state", "@op": "operation"}.get(kind, kind)


@dataclass(frozen=True)
class Declaration:
    """A resolved resource's declaration (declarations.json's ``expect``)."""

    declaration: str
    kind: str = "stream"
    period_ms: int | None = None
    deadline_ms: int | None = None
    lifespan_ms: int | None = None
    reasons: tuple[str, ...] = ()

    @property
    def below_period(self) -> bool:
        """§2.5: "When both are declared and the deadline is below the
        period … A deadline equal to the period is one"."""
        return self.period_ms is not None and self.deadline_ms is not None and self.deadline_ms < self.period_ms

    @property
    def is_stream(self) -> bool:
        return self.kind == "stream"

    @property
    def judged_deadline_ms(self) -> int | None:
        """The deadline a reader judges: none for an invalid declaration
        (§2.2), and none below the period (§2.5)."""
        if self.declaration != DECLARED or self.below_period:
            return None
        return self.deadline_ms

    @property
    def judged_lifespan_ms(self) -> int | None:
        return self.lifespan_ms if self.declaration == DECLARED else None

    def expect(self) -> dict[str, Any]:
        if self.declaration == DECLARED:
            return {"declaration": DECLARED, "period_ms": self.period_ms, "deadline_ms": self.deadline_ms,
                    "lifespan_ms": self.lifespan_ms, "below_period": self.below_period}
        if self.declaration == INVALID:
            return {"declaration": INVALID, "reasons": list(self.reasons)}
        return {"declaration": self.declaration}


def merge(layers: dict[str, dict[str, Any]]) -> dict[str, Any]:
    """Core §9.3: "Annotations merge key by key", the resource's over
    ``[defaults.<kind>]``'s over ``[defaults]``'s (declarations.json's
    ``layers``)."""
    return {**layers.get("defaults", {}), **layers.get("kind_defaults", {}), **layers.get("resource", {})}


def declaration(kind: str, annotations: dict[str, Any]) -> Declaration:
    """§2.2 and §2.3 over a resolved resource, in the fixture's order: no key
    of the vocabulary (another ``timing.<key>`` is W105's and no key of it);
    then the kind (a state, an event or an operation ignores the keys
    "whatever their values"); then each key alone, in §2.2's order."""
    k = kind_of(kind)
    if not any(key in annotations for key in KEYS):
        return Declaration(NOT_THIS_PROFILES, k)
    if k != "stream":
        return Declaration(IGNORED, k)
    reasons = tuple(REASONS[key] for key in KEYS if key in annotations and value_ms(annotations[key]) is None)
    if reasons:
        return Declaration(INVALID, k, reasons=reasons)
    vals = {key: (value_ms(annotations[key]) if key in annotations else None) for key in KEYS}
    return Declaration(DECLARED, k, vals[PERIOD], vals[DEADLINE], vals[LIFESPAN])


def requirement(annotations: dict[str, Any]) -> Declaration:
    """§2.3: "On a requirement, nothing at this version": a key of the
    vocabulary there is ignored, whatever its value."""
    if any(key in annotations for key in KEYS):
        return Declaration(IGNORED, "requirement")
    return Declaration(NOT_THIS_PROFILES, "requirement")


def declaration_of(case: dict[str, Any]) -> Declaration:
    """A fixture's ``resource`` (``annotations`` resolved, or ``layers``) or
    ``requirement``."""
    if "requirement" in case:
        return requirement(case["requirement"].get("annotations", {}))
    r = case["resource"] if "resource" in case else case
    ann = r["annotations"] if "annotations" in r else merge(r["layers"])
    return declaration(r["kind"], ann)


def _decl(kind_or_decl: str | Declaration, annotations: dict[str, Any] | None = None) -> Declaration:
    if isinstance(kind_or_decl, Declaration):
        return kind_or_decl
    return declaration(kind_or_decl, annotations or {})


# -- §2.5 the deadline ------------------------------------------------------------------

def detection_bound_ns(deadline_ms: int) -> int:
    """§2.5: "at most max(deadline/50, 1 ms) after it is due". deadline/50 in
    ns is deadline·20,000, exact for a whole number of milliseconds."""
    return max(deadline_ms * 20_000, MS)


@dataclass(frozen=True)
class Report:
    """One report of the tracker: ``missed`` (its due, its detection bound's
    end and the owner's state at the report), ``met``, ``ended`` (its reason,
    ``gone`` or ``forgotten``) or ``beyond_bound``, at ``at_ns``."""

    at_ns: int
    event: str
    member: str
    due_ns: int | None = None
    by_ns: int | None = None
    owner: str | None = None
    reason: str | None = None

    def expect(self) -> dict[str, Any]:
        out: dict[str, Any] = {"at_ms": ns_to_ms(self.at_ns), "event": self.event, "member": self.member}
        if self.event == "missed":
            assert self.due_ns is not None and self.by_ns is not None
            out.update(due_ms=ns_to_ms(self.due_ns), by_ms=ns_to_ms(self.by_ns), owner=self.owner)
        if self.event == "ended":
            out["reason"] = self.reason
        return out


@dataclass
class _Member:
    """One member the tracker holds. ``last``: its last arrival (heard);
    ``wait``: the instant its wait began (a known member never heard);
    ``missed``: a miss reported and no arrival since; ``gone``: a known
    member whose member tokens are gone, held and waited for by nothing
    until its next arrival or, on an epoch template, a token of it seen
    again."""

    known: bool
    last: int | None = None
    wait: int | None = None
    missed: bool = False
    gone: bool = False


class Tracker:
    """One reader's deadline tracker for one provider and resource (§2.5),
    as deadlines.json states it. Every event first judges every member it
    holds at its instant, in member order (byte order of the names): the
    misses due, then the members forgotten; then the event applies.

    Text 0.2:
    - **A missed member stays missed** when its owner is present again
      ("every known member never heard, not missed and not ended", F-262).
    - **An epoch template** (``epoch``, F-261): a known member is waited for
      from the later of the start and the instant a member token of it was
      first seen, never from its owner's presence; ``member_gone`` ends
      nothing before a token of the member was seen since it last ended.
    - **``beyond_bound`` once** (F-260), at a member's first arrival past
      the bound, and not again while it stays past it.

    Read beyond the fixture:
    - **At the start, an unconfirmed owner** counts as present: "while its
      owner is unconfirmed, the wait goes on as it was".
    - **A member not held** whose token is seen, then shown gone, forgets
      the sighting: its tokens are gone, so a later read ends nothing until
      one is seen again.
    """

    def __init__(self, deadline_ms: int, bound: int | None = None, known: Iterable[str] = (),
                 forget_ns: int = FORGET_NS, epoch: bool = False) -> None:
        self.deadline_ms = deadline_ms
        self.deadline_ns = deadline_ms * MS
        self.bound_ns = detection_bound_ns(deadline_ms)
        self.bound = bound
        self.forget_ns = forget_ns
        self.epoch = epoch
        self.started_at: int | None = None
        self.owner_state = UNSEEN
        self.present_since: int | None = None
        self.members: dict[str, _Member] = {k: _Member(True) for k in known}
        #: epoch only: the instant a member token of each member was first
        #: seen since the member last ended (§2.5, F-261)
        self.seen: dict[str, int] = {}
        #: the members reported ``beyond_bound`` and not held since (F-260)
        self.beyond: set[str] = set()
        self.reports: list[Report] = []

    # -- what it holds --------------------------------------------------------------

    def held(self) -> list[str]:
        return sorted(self.members, key=lambda s: s.encode())

    def _base(self, m: _Member) -> int | None:
        if m.missed or m.gone:
            return None
        return m.last if m.last is not None else m.wait

    def next_due(self) -> int | None:
        """The earliest due among held members heard or waited for, and not
        missed: a missed one is due nothing more until its next arrival."""
        dues = [b + self.deadline_ns for m in self.members.values() if (b := self._base(m)) is not None]
        return min(dues, default=None)

    def next_instant(self) -> int | None:
        """The next instant the reader must judge at itself: just after the
        earliest due ("more than the deadline"), or a member's forgetting."""
        due = self.next_due()
        out = [] if due is None else [due + 1]
        out += [m.last + self.forget_ns for m in self.members.values()
                if not m.known and m.missed and m.last is not None]
        return min(out, default=None)

    # -- judging --------------------------------------------------------------------

    def _judge(self, t: int) -> list[Report]:
        out: list[Report] = []
        for name in self.held():
            m = self.members[name]
            b = self._base(m)
            if b is not None and t - b > self.deadline_ns:
                m.missed = True
                due = b + self.deadline_ns
                out.append(Report(t, "missed", name, due, due + self.bound_ns, self.owner_state))
        for name in self.held():
            m = self.members[name]
            if not m.known and m.missed and m.last is not None and t - m.last >= self.forget_ns:
                out.append(Report(t, "ended", name, reason="forgotten"))
                del self.members[name]
                self.seen.pop(name, None)
        return out

    def _done(self, out: list[Report]) -> list[Report]:
        self.reports += out
        return out

    # -- events ---------------------------------------------------------------------

    def start(self, t: int) -> list[Report]:
        """The subscription declared: each known member is waited for from
        the later of the start and the instant the owner became present, if
        it is present; on an epoch template, from the later of the start and
        the instant a member token of it was first seen, if one was."""
        out = self._judge(t)
        self.started_at = t
        if self.epoch:
            for name, m in self.members.items():
                if m.known and m.last is None and not m.missed and name in self.seen:
                    m.wait, m.gone = max(t, self.seen[name]), False
        elif self.owner_state in (PRESENT, UNCONFIRMED) and self.present_since is not None:
            for m in self.members.values():
                if m.known and m.last is None and not m.gone and not m.missed:
                    m.wait = max(t, self.present_since)
        return self._done(out)

    def arrival(self, t: int, member: str, expired: bool = False) -> list[Report]:
        """A sample of ``member``; an expired one is no arrival (§2.6)."""
        out = self._judge(t)
        if self.started_at is None or expired:
            return self._done(out)
        m = self.members.get(member)
        if m is not None:
            if m.missed:
                out.append(Report(t, "met", member))
            m.last, m.wait, m.missed, m.gone = t, None, False, False
        elif self.bound is None or len(self.members) < self.bound:
            self.members[member] = _Member(False, last=t)
            self.beyond.discard(member)
        elif member not in self.beyond:
            self.beyond.add(member)
            out.append(Report(t, "beyond_bound", member))
        return self._done(out)

    def owner(self, t: int, state: str) -> list[Report]:
        """The owner's token as the reader observes it: present, unconfirmed
        or absent (v1.md §0)."""
        if state not in (PRESENT, UNCONFIRMED, ABSENT):
            raise ValueError(f"{state!r}: not an owner's state")
        out = self._judge(t)
        prev, self.owner_state = self.owner_state, state
        if state == PRESENT and prev in (UNSEEN, ABSENT):
            self.present_since = t
            if self.started_at is not None and not self.epoch:
                for m in self.members.values():
                    if m.known and m.last is None and not m.gone and not m.missed:
                        m.wait = t
        elif state == ABSENT and not self.epoch:
            for m in self.members.values():
                m.wait = None
        return self._done(out)

    def member_seen(self, t: int, member: str) -> list[Report]:
        """A member token of ``member`` seen: heard declared or withdrawn, or
        shown by a read (§2.5, F-261). Epoch templates only."""
        out = self._judge(t)
        if not self.epoch or member in self.seen:
            return self._done(out)
        self.seen[member] = t
        m = self.members.get(member)
        if m is not None and m.known and m.last is None and not m.missed and self.started_at is not None:
            m.wait, m.gone = t, False
        return self._done(out)

    def member_gone(self, t: int, member: str) -> list[Report]:
        """A complete read shows no member token of ``member`` (§2.5); on an
        epoch template, only after a token of it was seen since it last
        ended (F-261)."""
        out = self._judge(t)
        if self.epoch:
            if member not in self.seen:
                return self._done(out)
            del self.seen[member]
        m = self.members.get(member)
        if m is None:
            return self._done(out)
        if m.last is not None or m.missed:
            out.append(Report(t, "ended", member, reason="gone"))
        if m.known:
            m.last, m.wait, m.missed, m.gone = None, None, False, True
        else:
            del self.members[member]
        return self._done(out)

    def tick(self, t: int) -> list[Report]:
        """An instant the reader judges at."""
        return self._done(self._judge(t))

    def event(self, e: dict[str, Any]) -> list[Report]:
        """One of deadlines.json's events."""
        t = ms_to_ns(e["at_ms"])
        what = e["event"]
        if what == "start":
            return self.start(t)
        if what == "arrival":
            return self.arrival(t, e["member"], bool(e.get("expired", False)))
        if what == "owner":
            return self.owner(t, e["state"])
        if what == "member_seen":
            return self.member_seen(t, e["member"])
        if what == "member_gone":
            return self.member_gone(t, e["member"])
        if what == "tick":
            return self.tick(t)
        raise ValueError(f"{what!r}: not an event")


def tracker_for(decl: Declaration, bound: int | None = None, known: Iterable[str] = (),
                epoch: bool = False) -> Tracker | None:
    """A reader's tracker of a resource, or None: "A consumer holds no
    tracker of it, and reports no deadline event" under an invalid
    declaration (§2.2), a deadline below the period (§2.5), a resource that
    is not a stream (§2.3) or one with no deadline (text 0.2, F-259)."""
    if not decl.is_stream or decl.judged_deadline_ms is None:
        return None
    return Tracker(decl.judged_deadline_ms, bound, known, epoch=epoch)


# -- §2.6 the lifespan ------------------------------------------------------------------

_ZID = re.compile(r"[0-9a-fA-F]{1,32}")


def zid_value(text: Any) -> int | None:
    """A zid's value, compared by value (core §3.3): 1 to 32 hexadecimal
    digits (Appendix B), case and leading zeros ignored; None otherwise."""
    if not isinstance(text, str) or _ZID.fullmatch(text) is None:
        return None
    return int(text, 16)


@dataclass(frozen=True)
class Verdict:
    verdict: str
    reason: str
    detail: str = ""

    def pair(self) -> dict[str, str]:
        return {"verdict": self.verdict, "reason": self.reason}


@dataclass(frozen=True)
class Stamp:
    """A sample's stamp as the reader takes it at the arrival: its age (the
    reader's wall clock then minus the stamp's time, negative when ahead) and
    its id."""

    age_ns: int
    id: str


@dataclass
class LifespanReading:
    """§2.6 over one reading (a subscription's lifetime, or a tool's
    window): each sample judged at its arrival by the first of the ten
    steps that holds. ``epsilon_ms`` is the deployment's ε (None: none
    stated), ``owners`` the ``meta.zid`` of each instance of the provider's
    address whose descriptor the reader holds."""

    decl: Declaration
    epsilon_ms: int | float | None
    owners: list[str] = field(default_factory=list)
    #: the clocks a stamp more than ε ahead falsified for the reading, by value
    falsified: set[int] = field(default_factory=set)

    def add_owner(self, zid: str) -> None:
        if zid not in self.owners:
            self.owners.append(zid)

    def judge(self, stamp: Stamp | None) -> Verdict:
        d = self.decl
        if not d.is_stream:
            return Verdict("not_this_profiles", "kind", "not a stream (§2.3)")
        if d.declaration == INVALID:
            return Verdict(UNOBSERVABLE, "invalid", f"the declaration is invalid: {', '.join(d.reasons)}")
        if d.judged_lifespan_ms is None:
            return Verdict(NOT_ASKED, "no_lifespan", "no timing.lifespan_ms")
        if self.epsilon_ms is None:
            return Verdict(UNOBSERVABLE, "no_bound", "the deployment states no ε")
        if stamp is None:
            return Verdict(UNOBSERVABLE, "unstamped", "the sample carries no stamp")
        owners = {v for z in self.owners if (v := zid_value(z)) is not None}
        if not owners:
            return Verdict(UNOBSERVABLE, "unattributed", "no meta.zid of an instance of the provider is held")
        sid = zid_value(stamp.id)
        if sid not in owners:
            return Verdict(UNOBSERVABLE, "foreign_stamp", f"stamped by {stamp.id}, not the owner's meta.zid")
        eps = ms_to_ns(self.epsilon_ms)
        if stamp.age_ns < -eps:
            self.falsified.add(sid)
        if sid in self.falsified:
            return Verdict(UNOBSERVABLE, "clock_disagrees",
                           f"a stamp of {stamp.id} was more than ε ahead in this reading")
        if stamp.age_ns - eps > d.judged_lifespan_ms * MS:
            return Verdict(EXPIRED, "expired", f"aged {stamp.age_ns / MS:.3f} ms, less ε above the lifespan")
        return Verdict(LIVE, "live", f"aged {stamp.age_ns / MS:.3f} ms")


def judge_lifespans(decl: Declaration, epsilon_ms: int | float | None, owners: list[str],
                    stamps: Iterable[Stamp | None]) -> list[Verdict]:
    r = LifespanReading(decl, epsilon_ms, list(owners))
    return [r.judge(s) for s in stamps]


# -- §2.7 the period over a window ------------------------------------------------------

@dataclass(frozen=True)
class Period:
    verdict: str
    arrivals: int
    span_ns: int | None

    def expect(self) -> dict[str, Any]:
        return {"verdict": self.verdict, "arrivals": self.arrivals,
                "span_ms": None if self.span_ns is None else ns_to_ms(self.span_ns)}


def period(period_ms: int, from_ns: int, to_ns: int, known: bool, ended_ns: int | None,
           arrivals_ns: Iterable[int]) -> Period:
    """§2.7 "The period over the window": the span from the window's start
    for a known member, else from its first arrival in the window, to the
    window's end or the member's; its arrivals within it, both ends
    included; not heard, too short under three periods, kept when
    (arrivals + 2) × period is at least the span."""
    arr = sorted(arrivals_ns)
    within = [t for t in arr if from_ns <= t <= to_ns]
    if not known and not within:
        return Period(NOT_HEARD, 0, None)
    start = from_ns if known else within[0]
    end = to_ns if ended_ns is None else ended_ns
    span = end - start
    n = sum(1 for t in arr if start <= t <= end)
    p = period_ms * MS
    if span < 3 * p:
        return Period(TOO_SHORT, n, span)
    return Period(KEPT if (n + 2) * p >= span else NOT_KEPT, n, span)


# -- §5 what a tool may conclude --------------------------------------------------------

@dataclass(frozen=True)
class Answer:
    answer: str
    reason: str
    detail: str = ""

    def pair(self) -> dict[str, str]:
        return {"answer": self.answer, "reason": self.reason}


@dataclass(frozen=True)
class DeadlineAccount:
    """The tracker's account of one member over a window (§2.7)."""

    held_ns: int
    missed_present: int = 0
    missed_not_present: int = 0
    ended: bool = False


@dataclass(frozen=True)
class Reading:
    """A tool's reading of one member over one window (conclusions.json)."""

    deadline: DeadlineAccount
    period: str
    lifespan: tuple[Verdict, ...] = ()
    across_face: bool = False
    lost: bool = False
    owner_present: bool = True

    @classmethod
    def of(cls, r: dict[str, Any]) -> Reading:
        d = r["deadline"]
        return cls(DeadlineAccount(ms_to_ns(d["held_ms"]), d["missed_present"], d["missed_not_present"],
                                   bool(d["ended"])),
                   r["period"], tuple(Verdict(x["verdict"], x["reason"]) for x in r["lifespan"]),
                   bool(r["across_face"]), bool(r["lost"]), bool(r["owner_present"]))


def conclude(decl: Declaration, r: Reading) -> dict[str, Answer]:
    """§5's three questions, each read "the kind first, then the
    declaration, then its key, then the window" (conclusions.json's
    steps)."""
    return {"deadline": _deadline(decl, r), "period": _period(decl, r), "lifespan": _lifespan(decl, r)}


def _premises(decl: Declaration) -> Answer | None:
    if not decl.is_stream:
        return Answer("not_this_profiles", "kind", "not a stream (§2.3)")
    if decl.declaration == INVALID:
        return Answer(UNOBSERVABLE, "invalid", f"the declaration is invalid: {', '.join(decl.reasons)} (§2.2)")
    return None


def _deadline(decl: Declaration, r: Reading) -> Answer:
    if (a := _premises(decl)) is not None:
        return a
    if decl.deadline_ms is None:
        return Answer(NOT_ASKED, "no_deadline", "no timing.deadline_ms")
    if decl.below_period:
        return Answer(UNOBSERVABLE, "below_period", "the deadline is below the period: no deadline (§2.5)")
    if r.across_face:
        return Answer(UNOBSERVABLE, "across_face", "presence does not cross a constrained face (§2.8)")
    d = r.deadline
    if d.missed_present > 0:
        if r.lost:
            return Answer(UNOBSERVABLE, "lost", "a miss while the owner was present, and a delivery lost")
        return Answer(NO, "missed", f"{d.missed_present} miss(es) while the owner was present")
    if d.missed_not_present > 0:
        return Answer(UNOBSERVABLE, "owner_not_present", "a miss only while the owner was not present")
    if d.ended:
        return Answer(NOT_ASKED, "ended", "the member ended in the window without a miss")
    if d.held_ns == 0:
        return Answer(NOT_ASKED, "not_heard", "never held in the window")
    if d.held_ns <= decl.deadline_ms * MS:
        return Answer(UNOBSERVABLE, "window_too_short", "held for the deadline or less")
    return Answer(YES, "met", "no miss reported, held longer than the deadline")


def _period(decl: Declaration, r: Reading) -> Answer:
    if (a := _premises(decl)) is not None:
        return a
    if decl.period_ms is None:
        return Answer(NOT_ASKED, "no_period", "no timing.period_ms")
    if r.across_face:
        return Answer(UNOBSERVABLE, "across_face", "a face's rate is not the owner's (§2.8)")
    if r.period == NOT_HEARD:
        return Answer(NOT_ASKED, "not_heard", "no member to ask about")
    if r.period == TOO_SHORT:
        return Answer(UNOBSERVABLE, "window_too_short", "a span under three periods")
    if r.period == KEPT:
        return Answer(YES, "kept", "(arrivals + 2) × period at least the span")
    if r.lost:
        return Answer(UNOBSERVABLE, "lost", "not kept, and a delivery lost")
    if not r.owner_present:
        return Answer(UNOBSERVABLE, "owner_not_present", "not kept, and the owner not present throughout")
    return Answer(NO, "not_kept", "not kept, the owner present throughout and nothing lost")


def _lifespan(decl: Declaration, r: Reading) -> Answer:
    if (a := _premises(decl)) is not None:
        return a
    if decl.lifespan_ms is None:
        return Answer(NOT_ASKED, "no_lifespan", "no timing.lifespan_ms")
    vs = list(r.lifespan)
    if any(v.verdict == EXPIRED for v in vs):
        return Answer(YES, "expired", "a sample arrived expired")
    unobs = [v for v in vs if v.verdict == UNOBSERVABLE]
    if unobs:
        return Answer(UNOBSERVABLE, unobs[0].reason, "a sample unobservable, none expired")
    if any(v.verdict == LIVE for v in vs):
        return Answer(NO, "live", "every sample judged live")
    return Answer(UNOBSERVABLE, "no_sample", "no sample of the member")


def rollup(question: str, answers: Iterable[Answer]) -> Answer:
    """§5 "Over a resource, or a run", read by each question's finding
    (F-264): the finding when any member's answer is the finding; the other
    pole when every member's is, and there is at least one; unobservable
    otherwise. A member not asked, or not this profile's, is set aside."""
    finding = FINDING[question]
    other = YES if finding == NO else NO
    asked = [a for a in answers if a.answer not in (NOT_ASKED, "not_this_profiles")]
    hit = [a for a in asked if a.answer == finding]
    if hit:
        return Answer(finding, hit[0].reason, f"{len(hit)} member(s) the finding")
    if asked and all(a.answer == other for a in asked):
        return Answer(other, asked[0].reason, f"{len(asked)} member(s)")
    unobs = [a for a in asked if a.answer == UNOBSERVABLE]
    return Answer(UNOBSERVABLE, unobs[0].reason if unobs else "no_member", "no finding, and not every member clean")


def rollup_literal(answers: Iterable[Answer]) -> Answer:
    """§5's sentence as written, for every question alike: "no when any
    member's answer is no; yes when every member's is yes, and there is at
    least one; unobservable otherwise. A member not asked is set aside."
    Kept to show F-264: over the lifespan question it answers "no" for a
    resource with an expired member beside a live one."""
    asked = [a for a in answers if a.answer != NOT_ASKED]
    if any(a.answer == NO for a in asked):
        return Answer(NO, next(a.reason for a in asked if a.answer == NO))
    if asked and all(a.answer == YES for a in asked):
        return Answer(YES, asked[0].reason)
    return Answer(UNOBSERVABLE, "mixed")


# -- the profile's contract linter (§2.2, §2.3, §2.5: SHOULDs, no core code) ------------

@dataclass(frozen=True)
class Warning:
    what: str
    where: str
    message: str


def lint(canonical: dict[str, Any]) -> list[Warning]:
    """The warnings a contract linter SHOULD give, over a canonical form (its
    annotations resolved, core §9.3): an invalid value (§2.2), a key on a
    state, an event or an operation (§2.3, ``[defaults]`` included, since
    the canonical form carries what reached the resource), a key on a
    requirement (§2.3), a deadline below the period (§2.5). The core has no
    code for any of them, and the reference CLI's ``contract lint`` gives
    none (round 76)."""
    out: list[Warning] = []
    if PROFILE not in canonical.get("uses", []):
        return out
    for r in canonical.get("resources", []):
        where = f"{r['token']}/{r['template']}"
        d = declaration(r["kind"], r.get("annotations", {}))
        if d.declaration == IGNORED:
            out.append(Warning("ignored_kind", where, f"timing keys on a {d.kind} mean nothing (§2.3)"))
        elif d.declaration == INVALID:
            out.append(Warning("invalid_value", where, f"not a value: {', '.join(d.reasons)} (§2.2)"))
        elif d.declaration == DECLARED and d.below_period:
            out.append(Warning("below_period", where,
                               f"deadline {d.deadline_ms} ms below period {d.period_ms} ms: no deadline (§2.5)"))
    for role, req in canonical.get("requires", {}).items():
        if requirement(req.get("annotations", {})).declaration == IGNORED:
            out.append(Warning("ignored_requirement", f"requires.{role}",
                               "timing keys on a requirement tighten nothing at 1.0 (§2.3)"))
    return out


__all__ = ["PROFILE", "KEYS", "Declaration", "declaration", "requirement", "declaration_of", "merge", "value_ms",
           "Tracker", "tracker_for", "Report", "detection_bound_ns", "LifespanReading", "Stamp", "Verdict", "judge_lifespans",
           "zid_value", "period", "Period", "conclude", "Reading", "DeadlineAccount", "Answer", "rollup",
           "rollup_literal", "lint", "ms_to_ns", "ns_to_ms"]

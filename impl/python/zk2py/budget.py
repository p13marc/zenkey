"""Core §2.7, the population budget (0.24): a templated resource's
``cardinality`` is a budget the owner keeps and a tool can see broken.

The session-free half, as ``spec/conformance/budget/`` reads it:
- :func:`bound` (``bounds.json``): the descriptors' stated bounds and the
  contract's, the greatest across instances;
- :func:`population` (``population.json``): one reading, an S4 GET of a
  state or a subscription window over a stream or an event, against the
  bound;
- :func:`rate` (``rate.json``): an event's occurrences against its rate, per
  member.

Instants and spans are integers of nanoseconds, compared exactly.

The bus half, holding a session:
- :func:`get_population`: a complete S4 GET of a templated state (§2.7,
  "What a tool reads");
- :class:`Window`: a subscription window over a stream's or an event's
  members, with its owner's presence watched from its start to its end.
"""

from __future__ import annotations

import re
import threading
import time
from dataclasses import dataclass, field
from typing import Any

#: §2.2: "the no-ceiling value, 4294967295".
NO_CEILING = 4294967295
NS = 1_000_000_000
HOUR_NS, MINUTE_NS = 3600 * NS, 60 * NS
EXCEEDS, WITHIN, UNOBSERVABLE, NOT_ASKED = "exceeds", "within", "unobservable", "not_asked"


@dataclass(frozen=True)
class Verdict:
    verdict: str
    reason: str
    detail: str = ""

    def pair(self) -> dict[str, str]:
        return {"verdict": self.verdict, "reason": self.reason}


# -- the bound ------------------------------------------------------------------------

def bound(contract: int | None, stated: list[int | None]) -> int | str:
    """§2.7 "The bound": per instance, the cardinality its descriptor states
    when it is from 1 to the contract's (else D007's, and the contract's
    applies); across the instances exposing the resource, the greatest. The
    no-ceiling value is no bound, from either source. ``contract`` None is
    a template without parameters."""
    if contract is None:
        return "untemplated"
    per = [s if s is not None and 1 <= s <= contract else contract for s in stated] or [contract]
    if any(b == NO_CEILING for b in per):
        return "no_ceiling"
    return max(per)


# -- readings ---------------------------------------------------------------------------

@dataclass(frozen=True)
class GetReading:
    """An owner's S4 GET of a state resource: the keys it answered with a
    value, and whether it ran to its final reply with no error reply."""

    members: int
    complete: bool


@dataclass(frozen=True)
class WindowReading:
    """A subscription window: per member, the instants it was heard at, on
    one clock; how long it listened; whether it lost no delivery; whether
    the owner was present, its instance token held, from its start to its
    end."""

    heard: dict[str, list[int]]
    listened_ns: int
    lossless: bool = True
    present: bool = True


def span_ns(kind: str, retention_ns: int | None) -> int:
    """§2.7 "A member is live": a stream's members for an hour after their
    last publication, an event's for the retention after its last
    occurrence."""
    if kind == "event":
        if retention_ns is None:
            raise ValueError("an event declares a retention (§2.6)")
        return retention_ns
    return HOUR_NS


def most_within(heard: dict[str, list[int]], span: int) -> int:
    """The most members heard within any span: within [t, t + span), since
    "two instants exactly one span apart are not within one span"."""
    events = sorted((t, m) for m, ts in heard.items() for t in ts)
    best = 0
    j = 0
    count: dict[str, int] = {}
    for i, (t, _) in enumerate(events):
        while j < len(events) and events[j][0] < t + span:
            count[events[j][1]] = count.get(events[j][1], 0) + 1
            j += 1
        best = max(best, len(count))
        m = events[i][1]
        count[m] -= 1
        if not count[m]:
            del count[m]
    return best


def _kind(kind: str) -> str:
    return {"@stream": "stream", "@state": "state", "@op": "operation"}.get(kind, kind)


def population(kind: str, bnd: int | str, retention_ns: int | None,
               reading: GetReading | WindowReading) -> Verdict:
    """§2.7 "What a tool concludes", for a population, with the reasons in
    population.json's order: not asked (an operation; the no-ceiling bound;
    a template without parameters); a reading that does not count the kind;
    the finding, in any reading; an incomplete reading (a GET short of its
    final reply; a window lossy, then too short, then with the owner
    absent); no member (O5); clean."""
    kind = _kind(kind)
    if kind == "operation":
        return Verdict(NOT_ASKED, "kind", "an operation's members are the values its callers name")
    if bnd == "no_ceiling":
        return Verdict(NOT_ASKED, "no_ceiling", "4294967295 is no bound (§2.2)")
    if bnd == "untemplated":
        return Verdict(NOT_ASKED, "untemplated", "a template without parameters")
    assert isinstance(bnd, int)
    get = isinstance(reading, GetReading)
    if (kind == "state") != get:
        return Verdict(UNOBSERVABLE, "reading_kind",
                       "a window shows no state population" if kind == "state" else "S4 is state's")
    if get:
        n = reading.members
    else:
        span = span_ns(kind, retention_ns)
        n = most_within(reading.heard, span)
    if n > bnd:
        return Verdict(EXCEEDS, "over_bound", f"{n} members, the bound {bnd}")
    if get:
        if not reading.complete:
            return Verdict(UNOBSERVABLE, "incomplete", f"{n} members in a GET short of its final reply")
    else:
        if not reading.lossless:
            return Verdict(UNOBSERVABLE, "lossy", "the window lost a delivery")
        if reading.listened_ns < span:
            return Verdict(UNOBSERVABLE, "window_too_short", "the window is shorter than one liveness span")
        if not reading.present:
            return Verdict(UNOBSERVABLE, "owner_absent", "the owner was not present throughout")
    if n == 0:
        return Verdict(UNOBSERVABLE, "empty", "no member (O5)")
    return Verdict(WITHIN, "complete", f"{n} members, the bound {bnd}")


# -- rate -----------------------------------------------------------------------------

_BURST = re.compile(r"burst\(([1-9][0-9]*)/h\)")


def parse_rate(text: str) -> tuple[int, int]:
    """§2.6: (n, period in ns): ``rare`` 1 per hour, ``low`` 1 per minute,
    ``burst(<n>/h)`` n per hour."""
    if text == "rare":
        return 1, HOUR_NS
    if text == "low":
        return 1, MINUTE_NS
    m = _BURST.fullmatch(text)
    if m is None:
        raise ValueError(f"rate {text!r}")
    return int(m.group(1)), HOUR_NS


def rate(text: str | None, members: dict[str, list[int]], listened_ns: int, complete: bool) -> Verdict:
    """§2.7: "<n> + 1 occurrences of one member less than the period apart"
    are the finding, in a window of any length; then lost deliveries, a
    window shorter than one period, no occurrence (O5); else clean."""
    if text is None:
        return Verdict(NOT_ASKED, "no_rate", "a resource that declares no rate")
    n, period = parse_rate(text)
    for m, ts in members.items():
        ts = sorted(ts)
        for i in range(len(ts) - n):
            if ts[i + n] - ts[i] < period:
                return Verdict(EXCEEDS, "rate_exceeded", f"{n + 1} occurrences of {m} less than the period apart")
    if not complete:
        return Verdict(UNOBSERVABLE, "lossy", "deliveries lost")
    if listened_ns < period:
        return Verdict(UNOBSERVABLE, "window_too_short", "the window is shorter than one period")
    if not any(members.values()):
        return Verdict(UNOBSERVABLE, "empty", "no occurrence heard (O5)")
    return Verdict(WITHIN, "rate_kept", "none beyond the rate")


# -- on a bus --------------------------------------------------------------------------

def get_population(session: Any, selector: str, timeout: float = 1.0) -> tuple[GetReading, list[str]]:
    """§2.7: "A complete reading of a state is an S4 GET to its owner …
    that ran to its final reply, with no error reply … Its members are the
    keys answered with a value. A reply_del is not a member.\""""
    from . import live

    st = live.get_state(session, selector, timeout=timeout)
    members = sorted({r.key for r in st.replies if not r.deleted})
    return GetReading(len(members), not st.errors), members


def event_member(key: str) -> str:
    """§2.7: an event's member is "its key without the ULID chunk"."""
    return key.rsplit("/", 1)[0]


class Window:
    """A subscription window over a templated stream's or event's members
    (§2.7, "What a tool reads"), on the tool's receive clock. The owner's
    instance token is watched from the window's start to its end: present
    at the start (a liveliness GET), no delete of it while it listens, and
    present at the end. zenoh-python's callback handler drops nothing, so
    the window knows of no lost delivery (``lossless``): whether it lost
    none is not something a plain subscription can prove (SPEC-FINDINGS
    F-105)."""

    def __init__(self, session: Any, selector: str, owner_address: str, event: bool = False):
        import zenoh

        from . import live

        self.event = event
        self.lock = threading.Lock()
        self.heard: dict[str, list[int]] = {}
        self.stamps: dict[str, list[tuple[str | None, int | None]]] = {}
        self.token_events: list[str] = []
        self.start_ns = time.monotonic_ns()
        prefix = f"zk2/{owner_address}/@zk/instance/"
        self.lv = session.liveliness().declare_subscriber(f"{prefix}*", zenoh.handlers.Callback(
            lambda s: self.token_events.append(str(s.kind))))
        pres = live.list_presence(session, f"{prefix}*")
        self.present_at_start = bool(pres.instances) and pres.complete
        self.session, self.prefix = session, prefix
        self.sub = session.declare_subscriber(selector, zenoh.handlers.Callback(self._received))

    def _received(self, sample: Any) -> None:
        at = time.monotonic_ns()
        key = str(sample.key_expr)
        if "*" in key:
            return  # R6
        member = event_member(key) if self.event else key
        ts = sample.timestamp
        text = None if ts is None else str(ts)
        with self.lock:
            self.heard.setdefault(member, []).append(at - self.start_ns)
            self.stamps.setdefault(member, []).append(
                (None if text is None else text.split("/", 1)[1],
                 None if ts is None else ts.get_time_as_ntp64().as_nanos()))

    def reading(self) -> WindowReading:
        from . import live

        listened = time.monotonic_ns() - self.start_ns
        pres = live.list_presence(self.session, f"{self.prefix}*")
        gone = any("DELETE" in k for k in self.token_events)
        with self.lock:
            heard = {m: list(v) for m, v in self.heard.items()}
        return WindowReading(heard, listened, True, self.present_at_start and bool(pres.instances) and not gone)

    def rate_members(self) -> dict[str, list[int]]:
        """§2.7 "Spans": a rate's spans between the occurrences' stamps when
        they carry one clock id, else on the receive clock."""
        out: dict[str, list[int]] = {}
        with self.lock:
            for m, recv in self.heard.items():
                st = self.stamps.get(m, [])
                ids = {i for i, _ in st}
                out[m] = [t for _, t in st] if len(ids) == 1 and None not in ids else list(recv)  # type: ignore[misc]
        return out

    def close(self) -> None:
        for e in (self.sub, self.lv):
            try:
                e.undeclare()
            except Exception:  # noqa: BLE001 - closing anyway
                pass

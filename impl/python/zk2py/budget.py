"""Core §2.7, the population budget (0.24, 0.25): a templated resource's
``cardinality`` is a budget the owner keeps and a tool can see broken.

The session-free half, as ``spec/conformance/budget/`` reads it:
- :func:`bound` (``bounds.json``): the descriptors' stated bounds and the
  contract's, the greatest across instances;
- :func:`population` (``population.json``): one reading, an S4 GET of a
  state or a subscription window over a stream or an event, against the
  bound;
- :func:`rate` (``rate.json``): an event's occurrences against its rate, per
  member;
- :func:`lossless` (``lossless.json``, 0.25): whether a window is shown to
  have lost no delivery, from the resource's QoS, its reader's own loss and
  miss detection.

Instants and spans are integers of nanoseconds, compared exactly.

The bus half, holding a session:
- :func:`get_population`: a complete S4 GET of a templated state (§2.7,
  "What a tool reads");
- :class:`Window`: a subscription window over a stream's or an event's
  members, its span counted from its declaration plus the reply timeout,
  and its owner's presence watched until one lease after its end (0.25).
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
            return Verdict(UNOBSERVABLE, "lossy", "the window is not shown to have lost no delivery")
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


#: zenoh's defaults (core §2.7, 0.27): wait_before_close 5 s, a 10 s lease,
#: a keep-alive every quarter of it.
WAIT_BEFORE_CLOSE_MS, LEASE_MS, KEEP_ALIVE = 5000, 10000, 4


def stall_bound(wait_before_close_ms: int = WAIT_BEFORE_CLOSE_MS, lease_ms: int = LEASE_MS,
                keep_alive: int = KEEP_ALIVE) -> int:
    """§2.7 (0.27), the longest stall tolerated, as lossless.json's
    ``stall_bounds`` derive it: "the transport's wait_before_close, or its
    lease less one keep-alive interval when that is shorter". The interval
    is lease ÷ keep_alive rounded up to a whole millisecond, a keep-alive
    count of 0 taken as 1."""
    ka = max(1, keep_alive)
    interval = -(-lease_ms // ka)
    return min(wait_before_close_ms, lease_ms - interval)


def lossless(reliability: str, congestion: str, reader_lost: int = 0,
             miss_declared: bool = False, miss_used: bool = False, misses: int = 0,
             stall_gap_ms: int = 0, stall_bound_ms: int | None = None,
             transport_closed: bool = False) -> tuple[bool, str]:
    """§2.7 (0.25, 0.27): "Lost no delivery is established, never assumed".
    Both must hold: the reader lost none, and the path could not drop one
    silently, the resource declaring ``reliable`` with ``block``. The reader
    lost none only when, over the window and the watch after it, its queue
    lost nothing, no transport of its session closed, and its ticker's
    greatest gap is no longer than the bound (exactly the bound is no
    stall). A miss detected (where the resource declares miss detection
    and the subscription uses it) is a loss; none detected establishes
    nothing. Reasons in lossless.json's order: reader_lost, reader_stalled,
    misses_detected, best_effort, congestion_drop, else reliable_block."""
    bound = stall_bound() if stall_bound_ms is None else stall_bound_ms
    if reader_lost > 0:
        return False, "reader_lost"
    if transport_closed or stall_gap_ms > bound:
        return False, "reader_stalled"
    if miss_declared and miss_used and misses > 0:
        return False, "misses_detected"
    if reliability != "reliable":
        return False, "best_effort"
    if congestion != "block":
        return False, "congestion_drop"
    return True, "reliable_block"


def rate(text: str | None, members: dict[str, list[int]], listened_ns: int, lossless: bool,
         present: bool = True) -> Verdict:
    """§2.7: "<n> + 1 occurrences of one member less than the period apart"
    are the finding, in a window of any length; then a window not shown
    lossless, shorter than one period, the owner not present throughout
    (0.25), no occurrence (O5); else clean."""
    if text is None:
        return Verdict(NOT_ASKED, "no_rate", "a resource that declares no rate")
    n, period = parse_rate(text)
    for m, ts in members.items():
        ts = sorted(ts)
        for i in range(len(ts) - n):
            if ts[i + n] - ts[i] < period:
                return Verdict(EXCEEDS, "rate_exceeded", f"{n + 1} occurrences of {m} less than the period apart")
    if not lossless:
        return Verdict(UNOBSERVABLE, "lossy", "the window is not shown to have lost no delivery")
    if listened_ns < period:
        return Verdict(UNOBSERVABLE, "window_too_short", "the window is shorter than one period")
    if not present:
        return Verdict(UNOBSERVABLE, "owner_absent", "the owner was not present until one lease after the end")
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


class StallWatch:
    """§2.7 (0.27), "How a tool establishes it": a tool watches its own
    receive path from before the window opens until the watch after it
    ends:
    - its transports, from zenoh's transport events. zenoh-python 1.10.1
      exposes them (``Session.info.declare_transport_events_listener``,
      unstable): a transport of its session that closed is a stall;
    - a monotonic ticker on a thread of its own: the greatest gap between
      two ticks, the one still open when it is read included, longer than
      the bound is a stall."""

    def __init__(self, session: Any, bound_ms: int | None = None, tick_s: float = 0.05):
        import zenoh

        self.bound_ms = stall_bound() if bound_ms is None else bound_ms
        self.closes: list[int] = []
        self.opens: list[int] = []
        self.lock = threading.Lock()

        def event(e: Any) -> None:
            (self.closes if e.kind == zenoh.SampleKind.DELETE else self.opens).append(time.monotonic_ns())

        info = session.info() if callable(getattr(session, "info", None)) else session.info
        self.listener = info.declare_transport_events_listener(zenoh.handlers.Callback(event), history=False)
        self.last = time.monotonic_ns()
        self.gap_ns = 0
        #: the instants (monotonic ns) a tick found the gap past the bound:
        #: ``alarms.v1`` §2.15 (0.3) reads "a stall is a loss at the instant
        #: the reader sees it"
        self.stalls: list[int] = []
        self._stop = threading.Event()
        self.tick_s = tick_s
        self.thread = threading.Thread(target=self._tick, daemon=True)
        self.thread.start()

    def _tick(self) -> None:
        while not self._stop.wait(self.tick_s):
            now = time.monotonic_ns()
            with self.lock:
                gap = now - self.last
                self.gap_ns = max(self.gap_ns, gap)
                if gap > self.bound_ms * 1_000_000:
                    self.stalls.append(now)
                self.last = now

    def reading(self) -> tuple[int, bool]:
        """(the greatest gap in ms, the one still open included; whether a
        transport of the session closed)."""
        now = time.monotonic_ns()
        with self.lock:
            gap = max(self.gap_ns, now - self.last)
        return gap // 1_000_000, bool(self.closes)

    def losses(self) -> list[int]:
        """Every instant its path was seen to lose: a stall seen by a tick
        (the tick that ends its gap), a transport of the session closed,
        and a gap still open past the bound when it is read, "the one still
        open when it is read included", dated now."""
        now = time.monotonic_ns()
        with self.lock:
            open_gap = [now] if now - self.last > self.bound_ms * 1_000_000 else []
            return sorted(self.stalls + self.closes + open_gap)

    def close(self) -> None:
        self._stop.set()
        try:
            self.listener.undeclare()
        except Exception:  # noqa: BLE001 - closing anyway
            pass


def event_member(key: str) -> str:
    """§2.7: an event's member is "its key without the ULID chunk"."""
    return key.rsplit("/", 1)[0]


class Window:
    """A subscription window over a templated stream's or event's members
    (§2.7, "What a tool reads"), on the tool's receive clock.
    - **Its span** counts "from its subscription's declaration plus its
      reply timeout" (0.25), a convention the text states as one.
    - **Lost no delivery** is established, never assumed (0.25), by
      :func:`lossless`: the resource's declared QoS, and the reader's own
      loss. zenoh-python's callback handler has no queue to overflow or
      fall behind, and this window keeps nothing out of its count, so its
      reader loses no counted delivery. Its own receive path is watched
      (0.27): a :class:`StallWatch` on its session from before the window
      opens until the watch after it ends, the bound from ``lease_s``
      (zenoh's 4 keep-alives per lease) and ``wait_before_close_s``. It does
      not use miss detection.
    - **The owner's presence** is watched from before the window starts
      "until one lease after its end", plus the reply timeout for the
      withdrawal's transit: its instance token held at the start, none of
      its tokens withdrawn, and held at the last instant.
    ``qos`` is the resource's (``reliability``, ``congestion``), as its
    canonical form states it."""

    def __init__(self, session: Any, selector: str, owner_address: str, *, qos: tuple[str, str],
                 event: bool = False, reply_timeout_s: float = 1.0, lease_s: float = 10.0,
                 wait_before_close_s: float = 5.0):
        import zenoh

        from . import live

        self.event, self.qos = event, qos
        self.lease_s = lease_s
        self.stall = StallWatch(session, stall_bound(int(wait_before_close_s * 1000), int(lease_s * 1000)))
        self.reply_timeout_ns = int(reply_timeout_s * NS)
        self.lock = threading.Lock()
        self.heard: dict[str, list[int]] = {}
        self.stamps: dict[str, list[tuple[str | None, int | None, int]]] = {}
        self.token_events: list[tuple[str, int]] = []
        prefix = f"zk2/{owner_address}/@zk/instance/"
        # Core §8.1 (0.37): the subscriber on the reader's watch session, one
        # round trip there, and the reads on its own session.
        self.lv, _ = live.watch_or_beside(session, f"{prefix}*", zenoh.handlers.Callback(
            lambda s: self.token_events.append((str(s.kind), time.monotonic_ns()))))
        pres = live.list_presence(session, f"{prefix}*")
        self.present_at_start = bool(pres.instances) and pres.complete
        self.session, self.prefix = session, prefix
        self.declared_ns = time.monotonic_ns()
        #: "counted from when its subscription reached the owner": the
        #: declaration plus the reply timeout
        self.start_ns = self.declared_ns + self.reply_timeout_ns
        self.sub = session.declare_subscriber(selector, zenoh.handlers.Callback(self._received))
        self.end_ns: int | None = None

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
                 None if ts is None else ts.get_time_as_ntp64().as_nanos(), at - self.start_ns))

    def end(self) -> None:
        """The window's end: what it heard after this does not count."""
        self.end_ns = time.monotonic_ns()

    def reading(self, lease_s: float | None = None) -> WindowReading:
        """The window's reading, once its owner, and its own receive path,
        have been watched for one lease after its end, and the reply timeout
        more. Ends the window now if :meth:`end` was not called."""
        from . import live

        if lease_s is None:
            lease_s = self.lease_s
        if self.end_ns is None:
            self.end()
        assert self.end_ns is not None
        watch_until = self.end_ns + int(lease_s * NS) + self.reply_timeout_ns
        while time.monotonic_ns() < watch_until:
            time.sleep(min(0.1, (watch_until - time.monotonic_ns()) / NS))
        pres = live.list_presence(self.session, f"{self.prefix}*")
        gone = any("DELETE" in k for k, _ in self.token_events)
        cut = self.end_ns - self.start_ns
        with self.lock:
            heard = {m: [x for x in v if x <= cut] for m, v in self.heard.items()}
        heard = {m: v for m, v in heard.items() if v}
        ok, _ = self._lossless()
        return WindowReading(heard, max(0, cut), ok, self.present_at_start and bool(pres.instances) and not gone)

    def _lossless(self) -> tuple[bool, str]:
        gap, closed = self.stall.reading()
        return lossless(*self.qos, stall_gap_ms=gap, stall_bound_ms=self.stall.bound_ms, transport_closed=closed)

    def lossless_reason(self) -> str:
        return self._lossless()[1]

    def rate_members(self) -> dict[str, list[int]]:
        """§2.7 "Spans": a rate's spans between the occurrences' stamps when
        they carry one clock id, else on the receive clock; what was heard
        after the window's end does not count."""
        out: dict[str, list[int]] = {}
        cut = None if self.end_ns is None else self.end_ns - self.start_ns
        with self.lock:
            for m, st in self.stamps.items():
                st = [x for x in st if cut is None or x[2] <= cut]
                if not st:
                    continue
                ids = {i for i, _, _ in st}
                out[m] = ([s for _, s, _ in st] if len(ids) == 1 and None not in ids  # type: ignore[misc]
                          else [r for _, _, r in st])
        return out

    def close(self) -> None:
        self.stall.close()
        for e in (self.sub, self.lv):
            try:
                e.undeclare()
            except Exception:  # noqa: BLE001 - closing anyway
                pass

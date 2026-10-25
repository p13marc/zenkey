"""What a complete presence read is (core §8.1, 0.37), for a reader that
follows presence live.

- :class:`PresenceView` is the session-free half: tokens heard live and
  reads folded in the order the reader had them, each holder (an address's
  instance tokens, an interface's tokens, a member's epochs) up, down or
  unobservable, and whether a read is owed.
- :class:`PresenceFollower` is the live half: its liveliness subscriber on
  a **watch session**, one round trip there before the seed, its reads on
  the reading session, a re-read soon after a withdrawal, one for a burst,
  and none for a make-before-break re-mint.

Written from core 0.37's §8.1 alone (#609, round 54):
- "An owner or a token is absent when a complete read (above) shows none,
  and only then."
- "A withdrawal heard live is a reason to read again, never absence by
  itself … Until then what was withdrawn is **unconfirmed**: an address
  unobservable, never down, and a withdrawn token not yet gone."
- "A make-before-break re-mint (above) leaves nothing with nothing held,
  so it owes no read: another instance holds throughout."
- "A read not made (above) leaves the withdrawal unconfirmed until the
  reader's session reaches a node again and a read is made."

Core 0.41 (#609, round 66) adds, for the watch session:
- "The watch session's subscribers SHOULD have no history … when their
  reader seeds by a read on its other session … Except in a namespace":
  the follower's subscriber has none, unless its watch session is in one.
- "A round trip stands until the watch session's transport moves … A reader
  that follows presence live SHOULD make the round trip again before its
  next read, and read again, when its watch session's transport closes or
  opens, as after a withdrawal. A seed made while no round trip stands is
  possibly incomplete: an address it does not show is unobservable, never
  down." The follower hears its watch session's transport events, and a
  read made while no round trip stands, or with a move while it ran, is
  folded **unordered**: what it shows is up, what it does not is
  unobservable (:data:`NO_ROUND_TRIP`), whatever came before. A move also
  leaves every holder the reader had established, up or down, unobservable
  (:data:`MOVED`) until a read: a subscriber without history hears no
  withdrawal at a link's drop of a token it was never told of, nor a token
  declared while its transport was down (F-229).
"""

from __future__ import annotations

import threading
import time
from dataclasses import dataclass, field
from typing import Any, Iterable

from . import keys

UP, DOWN, UNOBSERVABLE = "up", "down", "unobservable"
#: Why a holder is unobservable.
WITHDRAWN = "withdrawn, unconfirmed until a read"
NEVER_READ = "no complete read yet"
READ_INCOMPLETE = "the last read was possibly incomplete"
READ_NOT_MADE = "the last read was not made"
#: core §8.1 (0.41): "A seed made while no round trip stands is possibly
#: incomplete: an address it does not show is unobservable, never down."
NO_ROUND_TRIP = "the last read was made while no round trip stood on the watch session (core §8.1, 0.41)"
#: core §8.1 (0.41): "A round trip stands until the watch session's transport
#: moves": a holder established before the move is not followed since (F-229).
MOVED = "the watch session's transport moved since it was last read: no round trip stands (core §8.1, 0.41)"

#: The reference "reads 0.1 s after the first withdrawal of a burst" (core
#: CHANGELOG 0.37); §8.1 says "soon".
REREAD_DELAY_S = 0.1


def holder_of(key: str) -> tuple[str, ...] | None:
    """§8.1 (0.37): what a token holds for, "an address its instance tokens,
    an interface its tokens, a member its epochs": ``("instance", system,
    service)``, ``("alive", system, service, iface)`` or ``("member",
    system, service, iface, member)``; None for a key that is no zk2 token."""
    p = keys.parse(key)
    if p is None:
        return None
    if p["form"] == "instance":
        return ("instance", p["system"], p["service"])
    if p["form"] == "alive":
        return ("alive", p["system"], p["service"], p["iface"])
    if p["form"] == "member":
        return ("member", p["system"], p["service"], p["iface"], p["member"])
    return None


def _covers(selector: str, key: str) -> bool:
    """Whether a selector (a key expression of whole-chunk ``*`` and ``**``,
    as zk2py's reads write them) matches a key."""
    return _match(selector.split("/"), key.split("/"))


def _match(sel: list[str], k: list[str]) -> bool:
    if not sel:
        return not k
    head, rest = sel[0], sel[1:]
    if head == "**":
        return any(_match(rest, k[i:]) for i in range(len(k) + 1))
    if not k:
        return False
    if head == "*" or head == k[0]:
        return _match(rest, k[1:])
    return False


@dataclass
class Holder:
    """One holder's view: the token keys it holds as the reader saw them,
    whether a complete read last established it, and why not."""

    held: set[str] = field(default_factory=set)
    #: True when a complete read covering it is the last word on it, or a
    #: put heard since; False after a withdrawal left it with nothing held
    confirmed: bool = False
    why: str = NEVER_READ


@dataclass
class Read:
    """A presence read as the view folds it: when it was asked (the reader's
    monotonic ns), its selector, its kind (``live.COMPLETE``,
    ``live.POSSIBLY_INCOMPLETE`` or ``live.NOT_MADE``), and the token keys it
    showed."""

    asked_ns: int
    selector: str
    kind: str
    keys: tuple[str, ...] = ()
    #: False for a read made while no round trip stood on the watch session,
    #: or across a move of its transport (core §8.1, 0.41): folded as a seed
    #: possibly incomplete, what it shows up and what it does not unobservable
    ordered: bool = True


class PresenceView:
    """Tokens heard live and reads, folded in the order the reader had them
    (core §8.1, 0.37). ``selector`` is what the reader follows."""

    def __init__(self, selector: str = "zk2/*/*/@zk/**") -> None:
        self.selector = selector
        self.holders: dict[tuple[str, ...], Holder] = {}
        #: (ns, key, "put" | "delete"), as heard
        self.events: list[tuple[int, str, str]] = []
        #: the instant of the first withdrawal no read has been asked since
        #: that left something with nothing held: a read is owed
        self.owed_since: int | None = None
        self.reads: list[Read] = []
        #: the instants the watch session's transport moved (core §8.1, 0.41)
        self.moves: list[int] = []

    def _holder(self, h: tuple[str, ...]) -> Holder:
        return self.holders.setdefault(h, Holder())

    def heard(self, key: str, kind: str, at_ns: int | None = None) -> None:
        """A token heard live: ``put`` or ``delete``."""
        at = time.monotonic_ns() if at_ns is None else at_ns
        self.events.append((at, key, kind))
        h = holder_of(key)
        if h is None:
            return
        x = self._holder(h)
        if kind == "put":
            # A token heard is positive evidence (§8.1: "present when it
            # shows one"), whatever came before.
            x.held.add(key)
            x.confirmed, x.why = True, ""
            # the reason to read again is gone when nothing is left
            # unconfirmed by a withdrawal
            if self.owed_since is not None and not any(
                    y.why in (WITHDRAWN, MOVED) and not y.held for y in self.holders.values()):
                self.owed_since = None
        else:
            x.held.discard(key)
            if not x.held:
                # "A withdrawal that leaves something with nothing held":
                # unconfirmed, and a reason to read again. A re-mint, whose
                # new token was heard first, leaves something held, and owes
                # nothing.
                x.confirmed, x.why = False, WITHDRAWN
                if self.owed_since is None:
                    self.owed_since = at

    def asked(self, at_ns: int | None = None) -> int:
        """A read is asked now: it answers every withdrawal heard before it,
        "one read for a burst of them"."""
        at = time.monotonic_ns() if at_ns is None else at_ns
        if self.owed_since is not None and self.owed_since <= at:
            self.owed_since = None
        return at

    def moved(self, at_ns: int | None = None) -> None:
        """The watch session's transport closed or opened (core §8.1, 0.41):
        no round trip stands, and a read is owed, "as after a withdrawal".
        Every holder the reader established, up or down, is unconfirmed
        until that read (:data:`MOVED`): "a subscriber without history hears
        no withdrawal, at its link's drop, of a token it was never told of",
        so a token the seed showed is not heard going at a close, and one
        declared meanwhile is not heard coming (F-229). One already
        unconfirmed keeps its reason."""
        at = time.monotonic_ns() if at_ns is None else at_ns
        self.moves.append(at)
        for x in self.holders.values():
            if x.held or x.confirmed:
                x.held, x.confirmed, x.why = set(), False, MOVED
        if self.owed_since is None:
            self.owed_since = at

    def answered(self, read: Read) -> None:
        """A read's answer, folded as of when it was asked: the moves heard
        after it was asked keep their word over it, so a reply sent before
        a withdrawal never brings the token back.
        - **Complete:** each holder under its selector holds what it showed,
          and what was heard put since; one left with nothing is **down**
          ("absent when it is complete and shows none"), unless a withdrawal
          heard since leaves it unconfirmed still.
        - **Unordered** (0.41): complete by its replies, made while no round
          trip stood: each holder holds what it showed, and what was heard
          put since, as for a complete one; one left with nothing is
          **unobservable**, never down ("an address it does not show is
          unobservable, never down"); a withdrawal it was asked for keeps
          its reason, and a read stays owed, an ordered one, whatever it
          showed.
        - **Possibly incomplete, or not made:** a token it shows is
          positive evidence ("present when it shows one"); it removes
          nothing, and a withdrawal it was asked for stays unconfirmed, the
          read still owed ("until … a read is made")."""
        self.reads.append(read)
        whole = read.kind == "complete" and read.ordered
        unordered = read.kind == "complete" and not read.ordered
        later = [(k, kind) for t, k, kind in self.events if t > read.asked_ns]
        moved = {k for k, _ in later}
        # each moved key's latest move since the ask decides it (put, delete
        # and put again is held; put and delete is withdrawn)
        last: dict[str, str] = {}
        for k, kind in later:
            last[k] = kind
        withdrawn_since = {holder_of(k) for k, kind in last.items() if kind == "delete"}
        shown = {k for k in read.keys if holder_of(k) is not None}
        scope = {h for h in self.holders if self._scoped(read.selector, h)}
        scope |= {holder_of(k) for k in shown}  # type: ignore[misc]
        for h in scope:
            x = self._holder(h)
            mine = {k for k in shown if holder_of(k) == h and k not in moved}
            if whole:
                x.held = {k for k in moved if holder_of(k) == h and last[k] == "put"} | mine
                if x.held or h not in withdrawn_since:
                    x.confirmed, x.why = True, ""
            elif unordered:
                x.held = {k for k in moved if holder_of(k) == h and last[k] == "put"} | mine
                if x.held:
                    x.confirmed, x.why = True, ""
                elif h in withdrawn_since or x.why == WITHDRAWN:
                    x.confirmed, x.why = False, WITHDRAWN
                else:
                    x.confirmed, x.why = False, NO_ROUND_TRIP
            elif mine:
                x.held |= mine
                x.confirmed, x.why = True, ""
            elif not x.held and not x.confirmed and x.why not in (WITHDRAWN, MOVED):
                x.why = READ_NOT_MADE if read.kind == "not made" else READ_INCOMPLETE
        if not whole and self.owed_since is None and any(
                x.why in (WITHDRAWN, MOVED) and not x.held for x in self.holders.values()):
            self.owed_since = read.asked_ns
        # An unordered read owes an ordered one, whatever it showed: "It reads
        # again, making the round trip first, and once one stands a read is
        # complete again" (presence.md §10 Expected 6; review of PR #876).
        if unordered and self.owed_since is None:
            self.owed_since = read.asked_ns

    def _scoped(self, selector: str, h: tuple[str, ...]) -> bool:
        """Whether a holder lies under a read's selector, by a key of its
        form (an instance's ``…/@zk/instance/<16 hex>``)."""
        probe = {"instance": "zk2/{1}/{2}/@zk/instance/0000000000000000",
                 "alive": "zk2/{1}/{2}/@zk/alive/{3}/0000000000000000/0000000000000000",
                 "member": "zk2/{1}/{2}/@zk/member/{3}/{4}/0000000000000000"}[h[0]]
        return _covers(selector, probe.format(*h))

    @property
    def owes_read(self) -> bool:
        return self.owed_since is not None

    def state(self, h: tuple[str, ...]) -> tuple[str, str]:
        """A holder's state and its reason: up (something held), down (a
        complete read established none), unobservable (unconfirmed, or no
        complete read yet)."""
        x = self.holders.get(h)
        if x is None:
            # never heard of: the latest read under whose selector it lies
            for r in reversed(self.reads):
                if self._scoped(r.selector, h):
                    if r.kind == "complete" and not r.ordered:
                        return UNOBSERVABLE, NO_ROUND_TRIP
                    if r.kind == "complete":
                        if any(m > r.asked_ns for m in self.moves):
                            return UNOBSERVABLE, MOVED
                        return DOWN, ""
                    return UNOBSERVABLE, READ_NOT_MADE if r.kind == "not made" else READ_INCOMPLETE
            return UNOBSERVABLE, NEVER_READ
        if x.held:
            return UP, ""
        if x.confirmed:
            return DOWN, ""
        return UNOBSERVABLE, x.why

    def address(self, system: str, service: str) -> tuple[str, str]:
        """An address's state, by its instance tokens."""
        return self.state(("instance", system, service))


class PresenceFollower:
    """A reader that follows presence live (core §8.1, 0.37, 0.41): its
    liveliness subscriber on ``watch``, a session it holds for no read; a
    round trip there (:func:`zk2py.live.settle`) before each read while none
    stands; every read on ``session``, which holds no liveliness subscriber;
    and, soon after a withdrawal that leaves something with nothing held,
    one read for the burst.

    0.41:
    - the subscriber has no history (``history`` None), unless the watch
      session is in a namespace (:func:`zk2py.live.namespace_of`): "A watch
      session in a namespace keeps history";
    - the round trip asks :func:`zk2py.live.settle_key` of the selector, and
      stands only as :class:`zk2py.live.RoundTrip` says;
    - "A round trip stands until the watch session's transport moves": the
      follower hears its watch session's transport events
      (``Session.info.declare_transport_events_listener``); a move ends the
      round trip, leaves what it established unobservable (F-229), and
      asks a read soon, made after a new round trip;
    - a read made while no round trip stands, or with a move while it ran,
      is folded unordered (:class:`Read` ``ordered``), and made again.

    A read that does not settle what is owed (not made, possibly
    incomplete, unordered, or one that raised) is made again after
    ``retry_s``, doubling each time up to ``max_retry_s``, and back to
    ``retry_s`` after a complete one or a move: a session that stays cut off
    is not asked five times a second forever. While the watch session holds
    no link, no round trip can stand: after the read its move asked, the
    reading session is read again only for a token move heard, and
    otherwise at the transport's opening (:meth:`_wait_for_watch`), so a
    long watch-link outage costs a healthy reading session no reads."""

    def __init__(self, session: Any, watch: Any, selector: str = "zk2/*/*/@zk/**", *,
                 reread_delay_s: float = REREAD_DELAY_S, retry_s: float = 0.2, max_retry_s: float = 5.0,
                 timeout: float = 1.0, round_trip: bool = True, history: bool | None = None,
                 round_trip_timeout: float | None = None, follow_moves: bool = True) -> None:
        """``round_trip`` False makes the reader core §8.1 warns of, a
        scenario's control (presence.md §9 step 5, §10 step 2): no round trip
        on the watch session, and every read taken as ordered.
        ``follow_moves`` False hears no transport event: a round trip, once
        made, is taken to stand for good, as before 0.41 (a control)."""
        import zenoh

        from . import live

        self.session, self.watch, self.selector = session, watch, selector
        self.delay, self.timeout = reread_delay_s, timeout
        self.rt_timeout = timeout if round_trip_timeout is None else round_trip_timeout
        self.retry_min, self.retry_max = retry_s, max_retry_s
        self.backoff = retry_s
        self.round_trip = round_trip
        self.history = (live.namespace_of(watch) is not None) if history is None else history
        self.view = PresenceView(selector)
        self.lock = threading.Lock()
        self._attempting = threading.Lock()
        self._timer: threading.Timer | None = None
        self._due: float | None = None
        self._closed = False
        self.reads: list[Any] = []
        #: (kind, ordered) of each read as folded, beside :attr:`reads`
        self.folds: list[tuple[str, bool]] = []
        #: reads that raised, with the exception
        self.failures: list[BaseException] = []
        #: every round trip made, or found not makeable, in order
        self.round_trips: list[Any] = []
        #: the watch session's transport moves heard: (monotonic ns, "open" | "close")
        self.moves: list[tuple[int, str]] = []
        #: how many moves were heard; the round trip stands for the count it was made at
        self.gen = 0
        self._stood: int | None = None
        #: the move count, and how many token moves were heard, when the last
        #: read was asked: while the watch session holds no link, a read is
        #: made only for a move or a withdrawal heard since (review of PR #876)
        self._read_at: tuple[int, int] | None = None
        #: attempts that made no read, the watch session cut off
        self.waited = 0

        def got(sample: Any) -> None:
            kind = "delete" if sample.kind == zenoh.SampleKind.DELETE else "put"
            with self.lock:
                self.view.heard(str(sample.key_expr), kind)
                owed = self.view.owes_read
            if owed:
                self._schedule(self.delay)

        def moved(event: Any) -> None:
            kind = "close" if event.kind == zenoh.SampleKind.DELETE else "open"
            with self.lock:
                self.gen += 1
                self.moves.append((time.monotonic_ns(), kind))
                self.view.moved()
                self.backoff = self.retry_min
            self._schedule(self.delay)

        # The subscriber first, then the listener, then the first attempt
        # (review of PR #876): close() finds each declared or None, and a
        # move heard before the first round trip only asks a read that waits
        # on the constructor's (``_attempting``). A move before the listener
        # needs no hearing: no round trip stands yet.
        self.sub: Any = None
        self.listener: Any = None
        self.sub = live.declare_liveliness_subscriber(watch, selector, got, history=self.history)
        if follow_moves:
            try:
                self.listener = live._info(watch).declare_transport_events_listener(
                    zenoh.handlers.Callback(moved), history=False)
            except Exception:  # noqa: BLE001 - a binding without transport events
                self.listener = None
        self._attempt()

    @property
    def standing(self) -> bool:
        """Whether a round trip stands: it stood, and the watch session's
        transport has not moved since (core §8.1, 0.41)."""
        with self.lock:
            return self._stands()

    def _stands(self) -> bool:
        if not self.round_trip:
            return True
        if self.listener is None:
            # no transport event can say a move came: a round trip stands
            # for the read right after it alone
            return False
        return self._stood is not None and self._stood == self.gen

    @property
    def settled(self) -> bool:
        """Whether the first round trip stood (0.37's word)."""
        return bool(self.round_trips) and bool(self.round_trips[0])

    def _schedule(self, after_s: float) -> None:
        with self.lock:
            if self._closed:
                return
            due = time.monotonic() + after_s
            if self._timer is not None:
                if self._due is not None and self._due <= due:
                    return
                self._timer.cancel()
            self._timer = threading.Timer(after_s, self._fire)
            self._timer.daemon = True
            self._due = due
            self._timer.start()

    @property
    def idle(self) -> bool:
        """No read scheduled, and none owed."""
        with self.lock:
            return self._timer is None and not self.view.owes_read

    def _make_round_trip(self) -> None:
        """One round trip on the watch session unless one stands; settle()
        makes none through a session cut off (it would be answered at once)."""
        from . import live

        with self.lock:
            if not self.round_trip or (self.listener is not None and self._stands()):
                return
            gen = self.gen
        rt = live.settle(self.watch, self.rt_timeout, selector=self.selector)
        with self.lock:
            self.round_trips.append(rt)
            if rt.stands and self.gen == gen:
                self._stood = gen

    def _attempt(self) -> None:
        """A round trip unless one stands, one read, and the next one
        scheduled when it settled nothing."""
        from . import live

        with self._attempting:
            if self._wait_for_watch():
                return
            ok = False
            try:
                self._make_round_trip()
                p = self.read(_after_round_trip=True)
                ok = p.kind == live.COMPLETE and self.folds[-1][1]
            except Exception as e:  # noqa: BLE001 - a read that raised is retried, never fatal
                with self.lock:
                    self.failures.append(e)
            with self.lock:
                owed = self.view.owes_read
                if ok:
                    self.backoff = self.retry_min
                    again = self.retry_min if owed else None
                else:
                    again = self.backoff
                    self.backoff = min(self.backoff * 2, self.retry_max)
        if again is not None:
            self._schedule(again)

    def _wait_for_watch(self) -> bool:
        """Core §8.1 (0.41): the reader reads again "when its watch session's
        transport closes or opens", and soon after a withdrawal. While the
        watch session holds no link no round trip can stand, so every read
        is unordered and shows no more than the last one did: once one has
        been made since the last move and the last token heard, the next
        waits for the transport to open (its event asks it), checked every
        ``max_retry_s`` without a read in case that event is lost. Never
        for a follower without transport events, or without the round trip
        (a control): they retry as before."""
        from . import live

        with self.lock:
            quiet = (self.round_trip and self.listener is not None and self._closed is False
                     and self._read_at == (self.gen, len(self.view.events)))
        if not quiet or live.links_held(self.watch) is not False:
            return False
        with self.lock:
            self.waited += 1
        self._schedule(self.retry_max)
        return True

    def _fire(self) -> None:
        with self.lock:
            self._timer, self._due = None, None
            if self._closed:
                return
        self._attempt()

    def read(self, unordered: bool = False, _after_round_trip: bool = False) -> Any:
        """One read on the reading session, folded as of when it was asked;
        unordered (possibly incomplete, what it does not show unobservable)
        when ``unordered``, when no round trip stands, or when the watch
        session's transport moved while it ran (core §8.1, 0.41)."""
        from . import live

        with self.lock:
            asked = self.view.asked()
            gen = self.gen
            self._read_at = (gen, len(self.view.events))
            ordered = not unordered and self._stands_for_read(_after_round_trip)
        p = live.list_presence(self.session, self.selector, self.timeout)
        found = tuple(f"zk2/{x['system']}/{x['service']}/@zk/instance/{x['instance']}" for x in p.instances) + \
            tuple(f"zk2/{x['system']}/{x['service']}/@zk/alive/{x['iface']}/{x['instance']}/{x['fp16']}"
                  for x in p.alive) + \
            tuple(f"zk2/{x['system']}/{x['service']}/@zk/member/{x['iface']}/{x['member']}/{x['epoch']}"
                  for x in p.members)
        with self.lock:
            ordered = ordered and self.gen == gen
            self.view.answered(Read(asked, self.selector, p.kind, found, ordered=ordered))
            self.reads.append(p)
            self.folds.append((p.kind, ordered))
        return p

    def _stands_for_read(self, after_round_trip: bool) -> bool:
        if self._stands():
            return True
        # without transport events, the round trip just made stands for the
        # read that follows it
        return (self.round_trip and self.listener is None and after_round_trip and bool(self.round_trips)
                and bool(self.round_trips[-1]))

    def address(self, system: str, service: str) -> tuple[str, str]:
        with self.lock:
            return self.view.address(system, service)

    def close(self) -> None:
        with self.lock:
            self._closed = True
            if self._timer is not None:
                self._timer.cancel()
        for x in (getattr(self, "sub", None), getattr(self, "listener", None)):
            try:
                if x is not None:
                    x.undeclare()
            except Exception:  # noqa: BLE001 - closing anyway
                pass


def fold(events: Iterable[tuple[str, Any]], selector: str = "zk2/*/*/@zk/**") -> PresenceView:
    """The view after a sequence, for readings: ``("put", key)``,
    ``("delete", key)``, ``("ask", None)`` (a read asked), ``("read",
    (kind, keys))`` or ``("read", (kind, keys, ordered))`` (the last asked
    read's answer; ``ordered`` False for one made while no round trip stood,
    0.41) and ``("move", None)`` (the watch session's transport moved), one
    nanosecond apart."""
    v = PresenceView(selector)
    t = 0
    asked: int | None = None
    for what, x in events:
        t += 1
        if what in ("put", "delete"):
            v.heard(x, what, t)
        elif what == "ask":
            asked = v.asked(t)
        elif what == "move":
            v.moved(t)
        elif what == "read":
            kind, ks, *rest = x
            v.answered(Read(asked if asked is not None else t, selector, kind, tuple(ks),
                            ordered=rest[0] if rest else True))
            asked = None
    return v


__all__ = ["PresenceView", "PresenceFollower", "Read", "holder_of", "fold", "UP", "DOWN", "UNOBSERVABLE",
           "WITHDRAWN", "NEVER_READ", "READ_INCOMPLETE", "READ_NOT_MADE", "NO_ROUND_TRIP", "MOVED", "REREAD_DELAY_S"]

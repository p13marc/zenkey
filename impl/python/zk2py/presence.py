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
                    y.why == WITHDRAWN and not y.held for y in self.holders.values()):
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

    def answered(self, read: Read) -> None:
        """A read's answer, folded as of when it was asked: the moves heard
        after it was asked keep their word over it, so a reply sent before
        a withdrawal never brings the token back.
        - **Complete:** each holder under its selector holds what it showed,
          and what was heard put since; one left with nothing is **down**
          ("absent when it is complete and shows none"), unless a withdrawal
          heard since leaves it unconfirmed still.
        - **Possibly incomplete, or not made:** a token it shows is
          positive evidence ("present when it shows one"); it removes
          nothing, and a withdrawal it was asked for stays unconfirmed, the
          read still owed ("until … a read is made")."""
        self.reads.append(read)
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
            if read.kind == "complete":
                x.held = {k for k in moved if holder_of(k) == h and last[k] == "put"} | mine
                if x.held or h not in withdrawn_since:
                    x.confirmed, x.why = True, ""
            elif mine:
                x.held |= mine
                x.confirmed, x.why = True, ""
            elif not x.held and not x.confirmed and x.why != WITHDRAWN:
                x.why = READ_NOT_MADE if read.kind == "not made" else READ_INCOMPLETE
        if read.kind != "complete" and self.owed_since is None and any(
                x.why == WITHDRAWN and not x.held for x in self.holders.values()):
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
                    if r.kind == "complete":
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
    """A reader that follows presence live (core §8.1, 0.37): its liveliness
    subscriber, with history, on ``watch``, a session it holds for no read;
    one round trip there (:func:`zk2py.live.settle`) before the seed; every
    read on ``session``, which holds no liveliness subscriber; and, soon
    after a withdrawal that leaves something with nothing held, one read for
    the burst.

    A read that does not settle what is owed (not made, possibly
    incomplete, or one that raised) is made again after ``retry_s``,
    doubling each time up to ``max_retry_s``, and back to ``retry_s`` after
    a complete read: a session that stays cut off is not asked five times a
    second forever. A seed whose round trip did not come back cannot be
    shown ordered against the subscriber, so it is folded as possibly
    incomplete, and retried as one."""

    def __init__(self, session: Any, watch: Any, selector: str = "zk2/*/*/@zk/**", *,
                 reread_delay_s: float = REREAD_DELAY_S, retry_s: float = 0.2, max_retry_s: float = 5.0,
                 timeout: float = 1.0) -> None:
        import zenoh

        from . import live

        self.session, self.watch, self.selector = session, watch, selector
        self.delay, self.timeout = reread_delay_s, timeout
        self.retry_min, self.retry_max = retry_s, max_retry_s
        self.backoff = retry_s
        self.view = PresenceView(selector)
        self.lock = threading.Lock()
        self._timer: threading.Timer | None = None
        self._closed = False
        self.reads: list[Any] = []
        #: reads that raised, with the exception
        self.failures: list[BaseException] = []

        def got(sample: Any) -> None:
            kind = "delete" if sample.kind == zenoh.SampleKind.DELETE else "put"
            with self.lock:
                self.view.heard(str(sample.key_expr), kind)
                owed = self.view.owes_read
            if owed:
                self._schedule(self.delay)

        self.sub = live.declare_liveliness_subscriber(watch, selector, got, history=True)
        self.settled = any(live.settle(watch, timeout) for _ in range(live.SETTLE_TRIES))
        self._attempt(seed=True)

    def _schedule(self, after_s: float) -> None:
        with self.lock:
            if self._timer is not None or self._closed:
                return
            self._timer = threading.Timer(after_s, self._fire)
            self._timer.daemon = True
            self._timer.start()

    @property
    def idle(self) -> bool:
        """No read scheduled, and none owed."""
        with self.lock:
            return self._timer is None and not self.view.owes_read

    def _attempt(self, seed: bool = False) -> None:
        """One read, and the next one scheduled when it settled nothing."""
        try:
            p = self.read(unordered=seed and not self.settled)
            ok = p.kind == "complete" and not (seed and not self.settled)
        except Exception as e:  # noqa: BLE001 - a read that raised is retried, never fatal
            with self.lock:
                self.failures.append(e)
            ok = False
        with self.lock:
            owed = self.view.owes_read
            if ok:
                self.backoff = self.retry_min
                again = None if not owed else self.retry_min
            else:
                again = self.backoff
                self.backoff = min(self.backoff * 2, self.retry_max)
        if again is not None and (not ok or owed):
            self._schedule(again)

    def _fire(self) -> None:
        with self.lock:
            self._timer = None
        self._attempt()

    def read(self, unordered: bool = False) -> Any:
        """One read on the reading session, folded as of when it was asked;
        ``unordered`` folds it as possibly incomplete."""
        from . import live

        with self.lock:
            asked = self.view.asked()
        p = live.list_presence(self.session, self.selector, self.timeout)
        found = tuple(f"zk2/{x['system']}/{x['service']}/@zk/instance/{x['instance']}" for x in p.instances) + \
            tuple(f"zk2/{x['system']}/{x['service']}/@zk/alive/{x['iface']}/{x['instance']}/{x['fp16']}"
                  for x in p.alive) + \
            tuple(f"zk2/{x['system']}/{x['service']}/@zk/member/{x['iface']}/{x['member']}/{x['epoch']}"
                  for x in p.members)
        kind = live.POSSIBLY_INCOMPLETE if unordered and p.kind == live.COMPLETE else p.kind
        with self.lock:
            self.view.answered(Read(asked, self.selector, kind, found))
            self.reads.append(p)
        return p

    def address(self, system: str, service: str) -> tuple[str, str]:
        with self.lock:
            return self.view.address(system, service)

    def close(self) -> None:
        with self.lock:
            self._closed = True
            if self._timer is not None:
                self._timer.cancel()
        try:
            self.sub.undeclare()
        except Exception:  # noqa: BLE001 - closing anyway
            pass


def fold(events: Iterable[tuple[str, Any]], selector: str = "zk2/*/*/@zk/**") -> PresenceView:
    """The view after a sequence, for readings: ``("put", key)``,
    ``("delete", key)``, ``("ask", None)`` (a read asked) and ``("read",
    (kind, keys))`` (the last asked read's answer), one nanosecond apart."""
    v = PresenceView(selector)
    t = 0
    asked: int | None = None
    for what, x in events:
        t += 1
        if what in ("put", "delete"):
            v.heard(x, what, t)
        elif what == "ask":
            asked = v.asked(t)
        elif what == "read":
            kind, ks = x
            v.answered(Read(asked if asked is not None else t, selector, kind, tuple(ks)))
            asked = None
    return v


__all__ = ["PresenceView", "PresenceFollower", "Read", "holder_of", "fold", "UP", "DOWN", "UNOBSERVABLE",
           "WITHDRAWN", "NEVER_READ", "READ_INCOMPLETE", "READ_NOT_MADE", "REREAD_DELAY_S"]

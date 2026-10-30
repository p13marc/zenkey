"""Core §4.2, "A late joiner's seed" (0.26, 0.27): a consumer or a tool that wants
an owner's current state and every change after it seeds from S4's GET and
follows a subscription.

- **The order:** "A late joiner MUST declare its subscription first, then
  make S4's GET." :class:`LateJoiner` does, unless told ``order="get-first"``
  (state.md §10's control).
- **The merge:** per key, by the owner's stamp, whichever path brought each
  one. A newer live put or delete replaces a reply, a newer reply replaces a
  live sample, a ``reply_del`` is a deletion with its stamp, and two samples
  with equal stamps are one mutation, kept once. Stamps compare as
  ``Latest`` compares replies (§4.1), which 0.27 spells out: by time, then
  by id, each id's 128-bit value written as 16 bytes little-endian and
  compared from the first byte. An unstamped sample ranks below every
  stamped one, and two unstamped samples have no order but their arrival
  (0.27). :func:`compare` and :func:`kept` are the pure rule
  (``conformance/seed/merge.json``).
- **R6** binds a consumer, whose seed discards a key that is not concrete
  on either path; a tool's seed keeps it under the key it arrived on,
  merged with that key alone (0.27).
- **What a seed cannot know:** a GET that ran to its timeout (it ends with
  an error reply, Appendix B) seeds possibly incompletely; an owner absent
  at the GET seeds nothing, which is silence; a seed is current state, never
  last-known.
"""

from __future__ import annotations

import threading
import time
from dataclasses import dataclass, field
from typing import Any

GET_TIMEOUT_S = 1.0


@dataclass(frozen=True)
class Stamp:
    """A zenoh timestamp as ``Latest`` orders replies (§4.1, §4.2, 0.27):
    its NTP64 time, then its id, "each id's 128-bit value written as 16
    bytes little-endian, least significant byte first, and the two compared
    byte by byte from the first"."""

    time: int        # the NTP64 value, 2^-32 s since the UNIX epoch
    id: str          # the HLC's id, hex

    @classmethod
    def of(cls, ts: Any) -> "Stamp | None":
        if ts is None:
            return None
        text = str(ts)
        raw, _, ident = text.partition("/")
        return cls(int(raw), ident.lower().lstrip("0"))

    def key(self) -> tuple[int, bytes]:
        return self.time, int(self.id or "0", 16).to_bytes(16, "little")

    def __str__(self) -> str:
        return f"{self.time}/{self.id}"


@dataclass
class Entry:
    """A key's merged value: a put (``payload``) or a deletion, with the
    stamp that decided it and the path that brought it."""

    deleted: bool
    payload: bytes
    encoding: str
    stamp: Stamp | None
    via: str            # "live" or "reply"


@dataclass
class Applied:
    """One arrival at the merge: what it was, and what the merge did with
    it: ``kept`` (it replaced the held value), ``same`` (an equal stamp, one
    mutation kept once), or ``dropped`` (older than the held value)."""

    key: str
    via: str
    deleted: bool
    stamp: Stamp | None
    outcome: str


@dataclass
class Seed:
    """What a seed's GET came to (observer §4.1, "What a seed cannot know")."""

    complete: bool = False          # ran to its final reply with no error reply
    errors: list[str] = field(default_factory=list)
    replies: int = 0
    ended: bool = False

    @property
    def silent(self) -> bool:
        """No reply at all: O5's silence, never "no state"."""
        return self.ended and self.replies == 0 and not self.errors


def compare(a: Stamp | None, b: Stamp | None) -> str:
    """``less``, ``equal`` or ``greater`` for ``a`` against ``b`` (merge.json's
    ``order``). An unstamped one ranks below every stamped one."""
    if a is None or b is None:
        if a is None and b is None:
            return "equal"
        return "less" if a is None else "greater"
    ka, kb = a.key(), b.key()
    return "less" if ka < kb else "greater" if ka > kb else "equal"


def kept(arrivals: list[tuple[Stamp | None, str]]) -> list[bool]:
    """One key's arrivals, in arrival order, each (stamp or None, kind):
    whether each replaces what the key holds (merge.json's ``cases``). The
    kind never decides."""
    held: Stamp | None = None
    holding = False
    out = []
    for stamp, _kind in arrivals:
        k = (not holding) or newer(stamp, held)
        out.append(k)
        if k:
            held, holding = stamp, True
    return out


def newer(a: Stamp | None, b: Stamp | None) -> bool:
    """Whether ``a`` replaces a held ``b`` (§4.2, 0.27): "An unstamped sample
    ranks below every stamped one … It is kept on a key that holds nothing,
    or only unstamped samples, and never over a stamped one. Two unstamped
    samples of a key have no order but their arrival, so the later replaces
    the earlier." """
    if a is None:
        return b is None
    if b is None:
        return True
    return a.key() > b.key()


class LateJoiner:
    """A late joiner over ``selector`` (§4.2, 0.26). ``consumer`` discards
    a sample or a reply on a key that is not concrete (R6); a tool keeps it,
    as the text leaves to the tool."""

    def __init__(self, session: Any, selector: str, *, timeout: float = GET_TIMEOUT_S, consumer: bool = True,
                 order: str = "subscribe-first"):
        self.session, self.selector, self.timeout = session, selector, timeout
        self.consumer = consumer
        self.order = order
        self.lock = threading.Lock()
        self.state: dict[str, Entry] = {}
        self.applied: list[Applied] = []
        self.seed = Seed()
        self.sub: Any = None
        self._done = threading.Event()

    # -- the two paths ------------------------------------------------------

    def _merge(self, key: str, via: str, deleted: bool, payload: bytes, encoding: str, ts: Any) -> None:
        if self.consumer and "*" in key:
            return  # R6
        stamp = Stamp.of(ts)
        with self.lock:
            held = self.state.get(key)
            if held is None or newer(stamp, held.stamp):
                outcome = "kept"
                self.state[key] = Entry(deleted, payload, encoding, stamp, via)
            elif stamp is not None and held.stamp is not None and stamp.key() == held.stamp.key():
                outcome = "same"
            else:
                outcome = "dropped"
            self.applied.append(Applied(key, via, deleted, stamp, outcome))

    def _sample(self, sample: Any) -> None:
        import zenoh

        self._merge(str(sample.key_expr), "live", sample.kind == zenoh.SampleKind.DELETE,
                    sample.payload.to_bytes(), str(sample.encoding), sample.timestamp)

    def _reply(self, reply: Any) -> None:
        import zenoh

        if reply.ok is None:
            with self.lock:
                self.seed.errors.append(f"{reply.err.encoding}: {reply.err.payload.to_bytes()!r}")
            return
        s = reply.ok
        with self.lock:
            self.seed.replies += 1
        self._merge(str(s.key_expr), "reply", s.kind == zenoh.SampleKind.DELETE, s.payload.to_bytes(),
                    str(s.encoding), s.timestamp)

    def _ended(self) -> None:
        with self.lock:
            self.seed.ended = True
            self.seed.complete = not self.seed.errors
        self._done.set()

    # -- the seed -----------------------------------------------------------

    def subscribe(self) -> None:
        import zenoh

        self.sub = self.session.declare_subscriber(self.selector, zenoh.handlers.Callback(self._sample))

    def get(self) -> None:
        """S4's GET: target ``All``, consolidation ``Latest``, set
        explicitly."""
        import zenoh

        self.session.get(self.selector, zenoh.handlers.Callback(self._reply, self._ended),
                         target=zenoh.QueryTarget.ALL, consolidation=zenoh.ConsolidationMode.LATEST,
                         timeout=self.timeout)

    def start(self) -> None:
        if self.order == "subscribe-first":
            self.subscribe()
            self.get()
        else:
            self.get()

    def wait(self, timeout: float | None = None) -> bool:
        return self._done.wait(self.timeout + 5.0 if timeout is None else timeout)

    def view(self) -> dict[str, Entry]:
        with self.lock:
            return dict(self.state)

    def close(self) -> None:
        if self.sub is not None:
            try:
                self.sub.undeclare()
            except Exception:  # noqa: BLE001 - closing anyway
                pass


def seed(session: Any, selector: str, timeout: float = GET_TIMEOUT_S, settle: float = 0.0, **kw: Any) -> LateJoiner:
    """Seed and follow ``selector``: subscribe, GET, and wait for the GET's
    end (and ``settle`` seconds more)."""
    lj = LateJoiner(session, selector, timeout=timeout, **kw)
    lj.start()
    lj.wait()
    if settle:
        time.sleep(settle)
    return lj

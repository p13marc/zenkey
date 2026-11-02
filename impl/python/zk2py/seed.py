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
- **Across two timestamp ids** (core §4.2, §4.3, 0.68, F-382, F-383): a
  consumer orders by S7's consumer rule "from the first value it receives,
  a late joiner's seed replies included" (:func:`follows`): by time within
  one timestamp id, and "a value under an id other than the one its held
  value carries" accepted, ordered by time within that id from then on.
  Within one id that is the stamp merge. A tool "keeps core §4.2's merge
  for as long as it follows, across two timestamp ids too" (observer §4.1,
  0.10; :func:`newer`). Round 90's late joiner turned a consumer's to S7 at
  its GET's final reply, 0.67's reading; 0.68 corrected that sentence, so a
  consumer's has no switch to make (round 91).
- **A tool's bound** (observer §4.1, 0.12): "A tool that bounds what it
  holds counts each time the bound forgets a key, and reports the count. A
  forgotten key holds nothing, so the merge keeps the key's next value
  whatever its stamp." :class:`Merge` holds at most ``max_keys`` keys, the
  least recently arrived forgotten first, each forgetting counted
  (``forgotten``); the next arrival on a forgotten key is kept, as on a key
  that holds nothing. What reports it is :func:`bound_note`, which every
  reading of a bounded merge or recording carries (round 92). A
  tool's :class:`LateJoiner` is bounded at :data:`MAX_KEYS` unless told
  otherwise. A consumer's is never bounded, and refuses a bound: observer
  §4.1's is a tool's, and S7 orders a consumer's values from the first it
  receives, so a key it forgot would accept any stamp again (round 91,
  review-bot on PR #971).
- **R6** binds a consumer, whose seed discards a key that is not concrete
  on either path; a tool's seed keeps it under the key it arrived on,
  merged with that key alone (0.27).
- **What a seed cannot know:** a GET that ran to its timeout (it ends with
  an error reply, Appendix B) seeds possibly incompletely; an owner absent
  at the GET seeds nothing, which is silence; a seed is current state, never
  last-known.

**Runs and seqs (core 0.73, observer 0.15, #976).** Since core 0.73 an
owner's values are ordered by the in-band stamp its envelope carries
(§4.3, §7.5): "It MUST keep the newer by its in-band stamp: within one run,
by seq (§4.3). Across two runs, a consumer MUST instead keep what §4.3's
consumer rule accepts." Every arrival is read envelope first
(:func:`zk2py.zk2c.read_sample`, observer §7.3), and merged by its
:class:`Mark`, its in-band stamp and its Zenoh timestamp
(:func:`relation`, ``conformance/seed/inband.json``):
- **within one run** by seq: plain for a state or event key, serial over 24
  bits for a stream key (unordered at exactly 2^23);
- **across two runs** unordered: "nothing orders two runs", so a tool keeps
  the later arrival (observer §4.1) and a consumer accepts a value under a
  run other than its held value's (§4.3's consumer rule);
- **a sample with no envelope** is ordered among such samples by its Zenoh
  timestamp, as above (``merge.json``), and beside an in-band stamp it is
  unordered; one with neither stamp ranks below every stamped one;
- **a refused envelope** is undecodable, its refusal named, and never
  merged (``undecodable``): its bytes are never read as a payload.

What every reader merges is the envelope's record, never its bytes.
"""

from __future__ import annotations

import queue
import threading
import time
from collections import OrderedDict
from dataclasses import dataclass, field
from typing import Any, Iterable

from . import zk2c

GET_TIMEOUT_S = 1.0

#: The keys a tool's merge holds at most unless told otherwise (observer
#: §4.1, 0.11: "A tool that bounds what it holds counts the keys the bound
#: forgot, and reports the count"). The text sets no number; this one is
#: zk2py's, far above any scenario's population.
MAX_KEYS = 65536


def bound_note(forgotten: int, max_keys: int | None) -> str:
    """What a tool reports of its merge's bound (observer §4.1, 0.12): "A
    tool that bounds what it holds counts each time the bound forgets a
    key, and reports the count." zk2py marks neither a forgotten key nor
    its next value, so the count is the one place a person learns that the
    merge did not order every key ("That is the bound's cost"). A merge with
    no bound says so. ``forgotten`` counts each forgetting: a key forgotten,
    heard again and forgotten again counts twice (round 92's F-407, reading
    (a), which 0.12 states). The note counts forgettings, not keys, so that
    it reads as the count it is (the mend after #926 part 5)."""
    if max_keys is None:
        return "the merge holds every key it heard: it has no bound (observer §4.1)"
    word = "forgetting" if forgotten == 1 else "forgettings"
    cost = "" if not forgotten else (": a forgotten key's next value was kept whatever its stamp, so the merge "
                                     "did not order every key")
    return (f"{forgotten} {word} by the merge's bound of {max_keys} keys, each time it forgot a key counted "
            f"(observer §4.1){cost}")


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


@dataclass(frozen=True)
class Mark:
    """What orders one arrival (core §4.2, 0.73): its in-band stamp
    (:class:`zk2py.zk2c.InBand`, from its envelope) and its Zenoh timestamp
    (:class:`Stamp`), either or both None."""

    inband: zk2c.InBand | None = None
    zenoh: Stamp | None = None

    @classmethod
    def of(cls, x: Any) -> "Mark":
        """A :class:`Mark` from a mark, a Zenoh :class:`Stamp`, an in-band
        stamp, or None."""
        if isinstance(x, Mark):
            return x
        if isinstance(x, zk2c.InBand):
            return cls(x, None)
        if x is None:
            return cls()
        return cls(None, x)

    @property
    def stamped(self) -> bool:
        return self.inband is not None or self.zenoh is not None

    def __str__(self) -> str:
        """An in-band stamp shown as a run and a seq, never as a time
        (observer §8.3, text 0.15)."""
        if self.inband is not None:
            return str(self.inband)
        return "no stamp" if self.zenoh is None else f"zenoh {self.zenoh}"


def serial(kind: str) -> bool:
    """Core §4.3: a stream key's seqs compare by serial arithmetic over 24
    bits; a state or event key's as integers."""
    return kind in ("stream", "@stream")


def kind_of(key: str) -> str:
    """The kind token of a key as a session hears it, or ``""`` for a key
    the grammar does not parse as a zk2 data key."""
    p = zk2c.data_key(key)
    return p.get("token", "") if p is not None and p.get("form") == "data" else ""


def inband_order(a: zk2c.InBand, b: zk2c.InBand, kind: str) -> str:
    """``less``, ``equal``, ``greater`` or ``unordered`` for ``a`` against
    ``b`` (``inband.json``'s ``order``). Two runs, or one value at two
    widths, are unordered (core §4.3: "Nothing orders two runs"). Within one
    run, plain order, or serial order for a stream: "s′ is newer when
    (s′ − s) mod 2^24 is from 1 to 2^23 − 1, and older when it is above
    2^23. At exactly 2^23 they are unordered"."""
    if not a.same_run(b):
        return "unordered"
    if not serial(kind):
        return "less" if a.seq < b.seq else "greater" if a.seq > b.seq else "equal"
    d = (b.seq - a.seq) % 2**24
    if d == 0:
        return "equal"
    if d == 2**23:
        return "unordered"
    return "less" if d < 2**23 else "greater"


def relation(a: Any, b: Any, kind: str = "state", consumer: bool = False) -> str:
    """How arrival ``a`` stands to a held ``b`` (core §4.2, §4.3, 0.73;
    observer §4.1, text 0.15): ``less``, ``equal``, ``greater`` or
    ``unordered``. ``inband.json``: "An arrival with an in-band stamp is
    ordered by it, whatever Zenoh timestamp it also carries. One with a
    Zenoh timestamp alone is ordered among those as merge.json says for a
    tool, and for a consumer by time within one id, a value under another
    id accepted. An in-band stamp and a Zenoh timestamp alone are
    unordered, either way round. … An arrival with neither stamp is kept on
    a key that holds nothing or only such arrivals, and never over a
    stamped one." """
    a, b = Mark.of(a), Mark.of(b)
    if a.inband is not None and b.inband is not None:
        return inband_order(a.inband, b.inband, kind)
    if a.inband is not None or b.inband is not None:
        other = b if a.inband is not None else a
        if other.zenoh is not None:
            return "unordered"
        return "greater" if a.inband is not None else "less"
    if a.zenoh is not None and b.zenoh is not None:
        if consumer and a.zenoh.id != b.zenoh.id:
            return "unordered"
        if consumer:
            return "less" if a.zenoh.time < b.zenoh.time else "greater" if a.zenoh.time > b.zenoh.time else "equal"
        return compare(a.zenoh, b.zenoh)
    if a.zenoh is not None or b.zenoh is not None:
        return "greater" if a.zenoh is not None else "less"
    return "unordered"  # neither stamp: "no order but their arrival, so the later replaces the earlier"


def replaces(a: Any, b: Any, kind: str = "state", consumer: bool = False) -> bool:
    """Whether arrival ``a`` replaces a held ``b``: "A tool keeps a newer or
    an unordered arrival, never an equal or an older one, and a consumer
    accepts likewise" (``inband.json``)."""
    return relation(a, b, kind, consumer) in ("greater", "unordered")


@dataclass
class Entry:
    """A key's merged value: a put (``payload``, the envelope's record) or a
    deletion, with the stamps that decided it and the path that brought
    it. ``stamp`` is its Zenoh timestamp; ``inband`` its in-band stamp
    (core §4.3, 0.73)."""

    deleted: bool
    payload: bytes
    encoding: str
    stamp: Stamp | None
    via: str            # "live" or "reply"
    inband: zk2c.InBand | None = None

    @property
    def mark(self) -> Mark:
        return Mark(self.inband, self.stamp)


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
    #: which rule decided it: §4.2's merge (a tool's, for as long as it
    #: follows, observer §4.1), or S7's consumer rule (a consumer's, from its
    #: first value on, its seed's replies included: core §4.3, 0.68)
    rule: str = "merge"
    #: its in-band stamp (core §4.3, 0.73), or None
    inband: zk2c.InBand | None = None
    #: core §7.5's token where its envelope was refused (outcome
    #: ``undecodable``): never merged, its bytes never read (observer §7.3)
    refusal: str | None = None


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


def kept(arrivals: list[tuple[Any, str]], kind: str = "state", consumer: bool = False) -> list[bool]:
    """One key's arrivals, in arrival order, each (its mark, a Zenoh
    :class:`Stamp`, an in-band stamp or None; the sample's kind): whether
    each replaces what the key holds, for a tool or, with ``consumer``, by
    §4.3's consumer rule (``merge.json``'s and ``inband.json``'s ``cases``).
    ``kind`` is the key's kind token; the sample's kind never decides."""
    held: Any = None
    holding = False
    out = []
    for stamp, _sample in arrivals:
        k = (not holding) or replaces(stamp, held, kind, consumer)
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


def follows(a: Stamp | None, b: Stamp | None) -> bool:
    """Whether ``a`` replaces a held ``b`` by S7's consumer rule (core §4.3,
    0.68): "A consumer MUST order each key's values by time within one
    timestamp id, from the first value it receives, a late joiner's seed
    replies included (§4.2). It MUST accept a value under an id other than
    the one its held value carries, and order by time within that id from
    then on." An unstamped sample, no owner's (S1), keeps §4.2's rule: it
    carries no id, so it is under no id other than the held value's."""
    if a is None or b is None:
        return newer(a, b)
    # 0.68 settles F-383 as reading (a): an id other than the held value's,
    # per key. So an old epoch's value still in transit, arriving after the
    # new epoch's first, is accepted and the key steps back until the new
    # epoch's next value: S7's residual risk, as it now says. Two live
    # writers under two ids (a re-mint's overlap, a split brain) make a key
    # flap between them, each value accepted as another id's.
    if a.id != b.id:
        return True
    return a.time > b.time


def consumer_kept(arrivals: Iterable[Stamp | None]) -> list[bool]:
    """One key's arrivals in arrival order, whichever path brought each:
    whether each replaces what a consumer holds, S7's consumer rule
    deciding every one from the first (core §4.2, §4.3, 0.68)."""
    held: Stamp | None = None
    holding = False
    out = []
    for stamp in arrivals:
        k = (not holding) or follows(stamp, held)
        out.append(k)
        if k:
            held, holding = stamp, True
    return out


def newest(arrivals: Iterable[Any], kind: str = "state") -> int | None:
    """The index of the arrival a tool's merge holds at the end, over one
    key's arrivals in arrival order (core §4.2; observer §4.1), or None for
    none. A reader that records what arrived and merges when it reads calls
    this, so the merge is §4.2's: by the in-band stamp within one run, the
    later arrival across two (0.73), and among samples with no envelope by
    time, then by id as ``Latest`` compares, an unstamped sample below
    every stamped one. Each arrival is its :class:`Mark`, its Zenoh
    :class:`Stamp`, or None."""
    at = None
    for i, k in enumerate(kept([(x, "put") for x in arrivals], kind)):
        if k:
            at = i
    return at


def merged(arrivals: Iterable[tuple[Any, ...]], kind: str = "state") -> tuple[Any, ...] | None:
    """What a tool's merge holds at the end over one key's arrivals, in
    arrival order, each a tuple whose first item is its :class:`Mark`, its
    :class:`Stamp` or None (core §4.2; observer §4.1): the arrival
    :func:`newest` names, or None for none."""
    xs = list(arrivals)
    at = newest([x[0] for x in xs], kind)
    return None if at is None else xs[at]


class Merge:
    """One late joiner's per-key merge (core §4.2), by ``rule``: ``"merge"``,
    a tool's, kept for as long as it follows (observer §4.1, 0.10), or
    ``"S7"``, a consumer's, S7's consumer rule from its first value on (core
    §4.3, 0.68).

    **Bounded** at ``max_keys`` when given: "A tool that bounds what it
    holds counts each time the bound forgets a key, and reports the count. A
    forgotten key holds nothing, so the merge keeps the key's next value
    whatever its stamp" (observer §4.1, 0.12). The key that arrived least
    recently is forgotten first, and counted in ``forgotten``; the next
    arrival on it is kept, as on a key that holds nothing, whatever its
    stamp. :meth:`note` reports the count (:func:`bound_note`)."""

    def __init__(self, rule: str = "merge", max_keys: int | None = None):
        if rule not in ("merge", "S7"):
            raise ValueError(f"no such rule: {rule!r}")
        if max_keys is not None and max_keys < 1:
            raise ValueError("a bound holds at least one key")
        if rule == "S7" and max_keys is not None:
            # observer §4.1's bound is a tool's. A consumer orders by S7 from
            # the first value it receives (core §4.3): a key it forgot would
            # take any stamp as its first again.
            raise ValueError("a consumer's merge is not bounded: observer §4.1's bound is a tool's")
        self.rule, self.max_keys = rule, max_keys
        self.held: "OrderedDict[str, Entry]" = OrderedDict()
        #: how many times the bound forgot a key (observer §4.1)
        self.forgotten = 0
        #: the key forgotten last, for a report
        self.last_forgotten: str | None = None

    def apply(self, key: str, entry: Entry) -> str:
        """``kept``, ``same`` or ``dropped`` for one arrival: by its in-band
        stamp within one run, plain or serial as the key's kind says, a
        run other than the held value's kept (a tool's later arrival, a
        consumer's other run), and a sample with no envelope by its Zenoh
        timestamp (core §4.2, §4.3, 0.73; observer §4.1, text 0.15)."""
        held = self.held.get(key)
        rel = "greater" if held is None else relation(entry.mark, held.mark, kind_of(key), self.rule == "S7")
        if rel in ("greater", "unordered"):
            self.held[key] = entry
            outcome = "kept"
        elif rel == "equal":
            outcome = "same"
        else:
            outcome = "dropped"
        self.held.move_to_end(key)
        if self.max_keys is not None:
            while len(self.held) > self.max_keys:
                gone, _ = self.held.popitem(last=False)
                self.forgotten += 1
                self.last_forgotten = gone
        return outcome

    def note(self) -> str:
        """The count, reported (observer §4.1, 0.11; :func:`bound_note`)."""
        return bound_note(self.forgotten, self.max_keys)


class LateJoiner:
    """A late joiner over ``selector`` (§4.2, 0.26). ``consumer`` discards
    a sample or a reply on a key that is not concrete (R6); a tool keeps it,
    as the text leaves to the tool. A consumer orders every value it
    receives by S7's consumer rule, its seed's replies included (core §4.2,
    §4.3, 0.68; :func:`follows`); a tool keeps §4.2's merge for as long as
    it follows, across two ids too (observer §4.1, 0.10; :func:`newer`),
    bounded at ``max_keys`` (:data:`MAX_KEYS` unless told otherwise),
    counting in ``forgotten`` what the bound forgot (:class:`Merge`).

    **Decided in order of arrival.** zenoh runs the subscriber's callback
    and the GET's on threads of their own. Each callback's first act puts
    what it was handed on one queue (:meth:`_arrive`), and the queue is
    drained in that order under one lock, the GET's end among its items. So
    the values of a key are ordered as they arrived, whichever thread runs
    first, and the error replies count toward the seed's completeness by
    the same order (round 90, review-bot on PR #969). Since 0.68 no rule
    turns at the GET's end, so the end decides only the seed's completeness
    (round 91)."""

    def __init__(self, session: Any, selector: str, *, timeout: float = GET_TIMEOUT_S, consumer: bool = True,
                 order: str = "subscribe-first", max_keys: int | None = -1, across: Any = None):
        # core §8.1 (0.91): across a constrained face (``across``, a
        # zk2py.far_side.FarSide), a seed's GET waits at least the face's
        # reply timeout.
        if across is not None:
            timeout = across.get_timeout_s(timeout)
        self.session, self.selector, self.timeout = session, selector, timeout
        self.consumer = consumer
        self.order = order
        self.lock = threading.Lock()
        if max_keys == -1:
            max_keys = None if consumer else MAX_KEYS
        elif consumer and max_keys is not None:
            raise ValueError("a consumer's late joiner is not bounded: observer §4.1's bound is a tool's")
        #: the per-key merge: S7's for a consumer, §4.2's for a tool
        self.merge = Merge("S7" if consumer else "merge", max_keys)
        self.applied: list[Applied] = []
        #: arrivals whose envelope was refused, (key, via, token): undecodable
        #: (observer §7.3, text 0.14), never merged
        self.undecodable: list[tuple[str, str, str]] = []
        self.seed = Seed()
        self.sub: Any = None
        self._done = threading.Event()
        #: every arrival, in the order the callbacks were handed them
        self._inbox: queue.SimpleQueue[tuple[str, Any]] = queue.SimpleQueue()

    @property
    def state(self) -> "OrderedDict[str, Entry]":
        return self.merge.held

    @property
    def forgotten(self) -> int:
        """How many times the bound forgot a key (observer §4.1)."""
        with self.lock:
            return self.merge.forgotten

    def bound_note(self) -> str:
        """What this late joiner reports of its bound wherever what it holds
        is shown (observer §4.1, 0.11: "reports the count"; round 92)."""
        with self.lock:
            return self.merge.note()

    # -- arrivals, one queue ------------------------------------------------

    def _arrive(self, item: tuple[str, Any], drain: bool = True) -> None:
        """Queue one arrival, then apply every queued one in order. With
        ``drain`` false it is queued alone: a callback whose thread is
        handled later, which a reading uses to interleave two arrivals."""
        self._inbox.put(item)
        if drain:
            self._drain()

    def _drain(self) -> None:
        ended = False
        with self.lock:
            while True:
                try:
                    what, arg = self._inbox.get_nowait()
                except queue.Empty:
                    break
                ended = self._apply(what, arg) or ended
        if ended:
            self._done.set()

    def _apply(self, what: str, arg: Any) -> bool:
        """One arrival, under the lock. True for the GET's end."""
        if what in ("sample", "reply"):
            import zenoh
        if what == "end":
            self.seed.ended = True
            self.seed.complete = not self.seed.errors
            return True
        if what == "sample":
            self._sample_locked(arg, "live")
        elif what == "reply":
            if arg.ok is None:
                self.seed.errors.append(f"{arg.err.encoding}: {arg.err.payload.to_bytes()!r}")
            else:
                self.seed.replies += 1
                self._sample_locked(arg.ok, "reply")
        else:  # "merge": a session-free arrival, its fields given
            self._merge_locked(*arg)
        return False

    def _sample_locked(self, s: Any, via: str) -> None:
        """A zenoh sample read envelope first (observer §7.3, 0.14): the
        record merged by its in-band stamp; a refused envelope undecodable,
        its refusal named, never merged."""
        import zenoh

        att = s.attachment
        self._merge_locked(str(s.key_expr), via, s.kind == zenoh.SampleKind.DELETE, s.payload.to_bytes(),
                           str(s.encoding), s.timestamp, None if att is None else att.to_bytes())

    def _merge_locked(self, key: str, via: str, deleted: bool, payload: bytes, encoding: str, ts: Any,
                      attachment: bytes | None = None) -> None:
        if self.consumer and "*" in key:
            return  # R6
        stamp = Stamp.of(ts)
        read = zk2c.read_delete(key, encoding, attachment) if deleted else zk2c.read_put(key, encoding, payload)
        if read.refused:
            # observer §7.3: "A refused envelope is undecodable, and the tool
            # names the refusal"; a consumer is handed nothing of it.
            self.undecodable.append((key, via, read.refusal))
            self.applied.append(Applied(key, via, deleted, stamp, "undecodable", self.merge.rule, None,
                                        read.refusal))
            return
        record = b"" if deleted else (read.record if read.record is not None else payload)
        # A consumer by §4.3's consumer rule from its first value on, its
        # seed's replies included; a tool by §4.2's merge for as long as it
        # follows (observer §4.1). Neither turns at the GET's end.
        outcome = self.merge.apply(key, Entry(deleted, record, encoding, stamp, via, read.stamp))
        self.applied.append(Applied(key, via, deleted, stamp, outcome, self.merge.rule, read.stamp))

    # -- the two paths, and the GET's end ------------------------------------

    def _merge(self, key: str, via: str, deleted: bool, payload: bytes, encoding: str, ts: Any,
               drain: bool = True, attachment: bytes | None = None) -> None:
        """A session-free arrival: a sample (``via`` ``live``) or a reply,
        its fields given, read envelope first."""
        self._arrive(("merge", (key, via, deleted, payload, encoding, ts, attachment)), drain)

    def _sample(self, sample: Any) -> None:
        self._arrive(("sample", sample))

    def _reply(self, reply: Any) -> None:
        self._arrive(("reply", reply))

    def _ended(self, drain: bool = True) -> None:
        self._arrive(("end", None), drain)

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

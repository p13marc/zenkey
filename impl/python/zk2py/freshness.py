"""``freshness.v1``: how long a value stays current (profile text 0.1,
draft, written against core 0.21; ``spec/profiles/freshness/v1.md``).
Read again against text 0.5 (#609, round 86; #935): its inline history
taken out, no rule changed, and none of this module's rules moved.

The session-free half:
- :func:`horizon` reads a resolved resource's ``freshness.ttl_s`` (§2.1,
  §2.2, §2.3), as ``conformance/horizons.json`` does;
- :func:`judge_subscription` and :func:`judge_get` judge one observation
  (§2.5, §2.6), :func:`judge_observation` asks §2.7's order of checks
  first, and :func:`judge` combines several observations of one member
  (§2.7), as ``conformance/judgements.json`` does;
- :func:`resource_verdict` is §5's second question, one verdict per
  resource.

Ages are integers of nanoseconds: the fixture compares seconds "exactly, to
the nanosecond".

The runtime half, holding a session:
- the owner's re-puts every ttl/2 (§2.4) and the clock guard (§2.10) are
  :class:`zk2py.owner.Owner`'s;
- :class:`ClockTrust` is a GET reader's trust in a stamping clock (§2.6):
  the deployment's word, or a measurement of a live put;
- :class:`Subscriber` ages the members it receives on its monotonic clock
  (§2.5), and feeds a :class:`ClockTrust` from each put it receives;
- :func:`get_observation` reads a member as core S4 says, for §2.6.
"""

from __future__ import annotations

import math
import threading
import time
from collections import OrderedDict
from dataclasses import dataclass, field
from decimal import ROUND_HALF_EVEN, Decimal
from typing import Any, Iterable

from . import zk2c

PROFILE = "freshness.v1"
#: §4: the one key of the vocabulary.
KEY = "freshness.ttl_s"
#: §2.1: "A whole number of seconds, from 0 to 2^53−1".
MAX_TTL_S = 2**53 - 1
NS = 1_000_000_000
#: "The delta: the HLC's maximum delta of the deployment's routers, 500 ms
#: unless the deployment configures another (core §4.1)."
DEFAULT_DELTA_NS = 500_000_000

# The verdict tokens (§5, judgements.json).
FRESH, STALE, UNOBSERVABLE, NOT_ASKED = "fresh", "stale", "unobservable", "not_asked"


def seconds_to_ns(value: int | float | Decimal) -> int:
    """Seconds, as a fixture writes them, to nanoseconds, exactly: a float is
    read through its shortest decimal spelling, so ``60.001`` is
    60,001,000,000 ns and never one off."""
    if isinstance(value, bool):
        raise TypeError("a boolean is not a number of seconds")
    if isinstance(value, int):
        return value * NS
    d = value if isinstance(value, Decimal) else Decimal(repr(float(value)))
    return int((d * NS).to_integral_value(rounding=ROUND_HALF_EVEN))


# -- §2.1–§2.3 the horizon ----------------------------------------------------------

@dataclass(frozen=True)
class Horizon:
    """A resolved resource's horizon (horizons.json's ``expect``):
    - ``none``: no ``freshness.ttl_s``, so freshness is not asked (§2.3);
    - ``ignored``: an event or an operation, whatever the value (§2.2);
    - ``invalid``: a value that is not a horizon (§2.1), kept in ``value``;
    - ``never``: 0, never stale (§2.3);
    - ``within``: ``ttl_s`` above 0."""

    horizon: str
    ttl_s: int | None = None
    value: Any = None

    @property
    def refresh_ms(self) -> int | None:
        """§2.4: "at intervals of at most ttl/2", in milliseconds. ttl_s·500
        is exact, 30,500 ms for 61 s."""
        return None if self.ttl_s is None or self.horizon != "within" else self.ttl_s * 500

    @property
    def ttl_ns(self) -> int | None:
        return None if self.ttl_s is None else self.ttl_s * NS

    @property
    def refresh_ns(self) -> int | None:
        return None if self.refresh_ms is None else self.refresh_ms * 1_000_000

    def expect(self) -> dict[str, Any]:
        """horizons.json's ``expect`` object."""
        if self.horizon == "within":
            return {"horizon": "within", "ttl_s": self.ttl_s, "refresh_ms": self.refresh_ms}
        return {"horizon": self.horizon}


def _kind(kind: str) -> str:
    """horizons.json: "@stream and @state are read as stream and state"."""
    return {"@stream": "stream", "@state": "state", "@op": "operation"}.get(kind, kind)


def horizon_value(value: Any) -> int | None:
    """§2.1: the horizon in seconds, or None when the value is not one. "An
    integral float is that integer: 60.0 is 60, and -0.0 is 0." A boolean,
    a string, an array or a table is no horizon."""
    if isinstance(value, bool):
        return None
    if isinstance(value, int):
        return value if 0 <= value <= MAX_TTL_S else None
    if isinstance(value, float):
        if not math.isfinite(value) or value != math.floor(value):
            return None
        if not 0 <= value <= MAX_TTL_S:
            return None
        return int(value)  # -0.0 -> 0
    return None


def horizon(kind: str, annotations: dict[str, Any]) -> Horizon:
    """§2.1–§2.3 over a resolved resource's authoring kind (or kind token)
    and its merged annotations. The order is the fixture's, and §2.7's: no
    key first (an event without one is not asked either), then the kind
    ("the kind is read before the value"), then the value."""
    if KEY not in annotations:
        return Horizon("none")
    if _kind(kind) in ("event", "operation"):
        return Horizon("ignored", value=annotations[KEY])
    ttl = horizon_value(annotations[KEY])
    if ttl is None:
        return Horizon("invalid", value=annotations[KEY])
    return Horizon("never", 0) if ttl == 0 else Horizon("within", ttl)


# -- §2.5–§2.7 the judgements -------------------------------------------------------

@dataclass(frozen=True)
class Verdict:
    verdict: str
    reason: str
    detail: str = ""

    def pair(self) -> dict[str, str]:
        return {"verdict": self.verdict, "reason": self.reason}


@dataclass(frozen=True)
class Delivery:
    """A subscriber's last delivery of a member: ``put`` or ``delete``, and
    its age on the subscriber's monotonic clock."""

    kind: str
    age_ns: int


@dataclass(frozen=True)
class Subscription:
    """§2.5: what a subscriber knows of one member at the instant it judges.
    ``listened_ns`` runs from the subscription's declaration; ``complete`` is
    False when it knows it lost deliveries."""

    listened_ns: int
    complete: bool = True
    last: Delivery | None = None


@dataclass(frozen=True)
class Reply:
    """A GET's reply for one member: ``put`` with the reader's clock minus
    the stamp's time (None without a stamp), or ``delete`` (a reply_del)."""

    kind: str
    stamp_age_ns: int | None = None


@dataclass(frozen=True)
class Get:
    """§2.6: one GET's reply for a member (None: no reply), and whether the
    reader trusts the stamping clock, to which delta."""

    reply: Reply | None
    trusted: bool = False
    delta_ns: int = DEFAULT_DELTA_NS


@dataclass(frozen=True)
class Archive:
    """An archive's answer: last-known, never current (§2.8)."""


@dataclass(frozen=True)
class Snapshot:
    """``freshness.v2`` §2.6 (text 0.7, core 0.100): what a GET reader's
    snapshot says of one member. ``complete`` is whether the reader holds the
    member's collection's snapshot complete (core §7.5); ``entry`` is None
    when the snapshot lists the member neither live nor as a tombstone,
    ``tombstone``, or ``live``; ``age_ns`` is the reader's clock minus the
    confirmation's time ("the snapshot's time for confirmation 1, and the
    entry's conf for 2"), None for confirmation 0 or an unknown time; the
    clock is the owner's, "which built the snapshot"."""

    complete: bool
    entry: str | None
    age_ns: int | None = None
    trusted: bool = False
    delta_ns: int = DEFAULT_DELTA_NS

    def as_get(self) -> Get | None:
        """v2 §2.6: "v1 §2.6, word for word, read over a snapshot's entries
        … The order is v1 §2.6's, an entry for a reply: no entry, then a
        tombstone, then no confirmation's time, then a clock not trusted,
        then the band." None for a member no entry lists: deleted or
        silent, by :func:`judge_snapshot`."""
        if self.entry is None:
            return None
        if self.entry == "tombstone":
            return Get(Reply("delete"), self.trusted, self.delta_ns)
        return Get(Reply("put", self.age_ns), self.trusted, self.delta_ns)


Observation = Subscription | Get | Archive | Snapshot


def judge_subscription(h: Horizon, obs: Subscription) -> Verdict:
    """§2.5, for a member with a horizon (``within`` or ``never``). Rules 5
    and 6 and the ttl-0 line judge a member the reader knows from another
    observation, whose silence since is the evidence (0.2): the caller
    judges a subscription with no delivery only for such a member."""
    last = obs.last
    if h.horizon == "never":
        # "At ttl 0, a member is fresh unless its last delivery is a delete."
        if last is not None and last.kind == "delete":
            return Verdict(NOT_ASKED, "deleted", "the last delivery is a delete")
        return Verdict(FRESH, "never_stale", "ttl 0: never stale (§2.3)")
    ttl = h.ttl_ns
    assert ttl is not None
    if last is not None and last.kind == "delete":
        return Verdict(NOT_ASKED, "deleted", "the last delivery is a delete")
    if last is not None and last.age_ns <= ttl:
        return Verdict(FRESH, "within_horizon", f"the last delivery is {last.age_ns / NS:.3f} s old")
    if not obs.complete:
        return Verdict(UNOBSERVABLE, "incomplete", "deliveries were lost, and one may have been a confirmation")
    if last is not None:
        return Verdict(STALE, "beyond_horizon", f"the last delivery is {last.age_ns / NS:.3f} s old")
    if obs.listened_ns > ttl:
        return Verdict(STALE, "no_delivery", f"no delivery in {obs.listened_ns / NS:.3f} s of listening")
    return Verdict(UNOBSERVABLE, "listened_too_short", f"listened {obs.listened_ns / NS:.3f} s, ttl or less")


def judge_get(h: Horizon, obs: Get) -> Verdict:
    """§2.6 "The order": no reply; a reply_del; no stamp; a clock not
    trusted; then the band. At ttl 0, any reply that carries a value is
    fresh."""
    r = obs.reply
    if r is None:
        return Verdict(UNOBSERVABLE, "silent", "no reply for the member (core O5, S6)")
    if r.kind == "delete":
        return Verdict(NOT_ASKED, "deleted", "a reply_del")
    if h.horizon == "never":
        return Verdict(FRESH, "never_stale", "ttl 0: no stamp and no clock needed (§2.3)")
    if r.stamp_age_ns is None:
        return Verdict(UNOBSERVABLE, "no_stamp", "a reply without a stamp breaks S2")
    if not obs.trusted:
        return Verdict(UNOBSERVABLE, "clock_untrusted", "the stamping clock is not trusted to the delta")
    ttl, delta, age = h.ttl_ns, obs.delta_ns, r.stamp_age_ns
    assert ttl is not None
    if age < -delta:
        return Verdict(UNOBSERVABLE, "clock_disagrees", f"the stamp is {-age / NS:.3f} s ahead, beyond the delta")
    if age <= ttl - delta:
        return Verdict(FRESH, "within_horizon", f"aged {age / NS:.3f} s, at most ttl - delta")
    if age > ttl + delta:
        return Verdict(STALE, "beyond_horizon", f"aged {age / NS:.3f} s, above ttl + delta")
    return Verdict(UNOBSERVABLE, "near_horizon", f"aged {age / NS:.3f} s, within the delta of the horizon")


def judge_snapshot(h: Horizon, obs: Snapshot) -> Verdict:
    """``freshness.v2`` §2.6: "A member a complete snapshot omits has no
    value, by core §4.2's omission rule: deleted, as a tombstone. One an
    incomplete snapshot omits is a member the read did not reach:
    unobservable." An entry is judged as v1 §2.6 judges a reply."""
    g = obs.as_get()
    if g is None:
        if obs.complete:
            return Verdict(NOT_ASKED, "deleted", "a complete snapshot omits the member (core §4.2)")
        return Verdict(UNOBSERVABLE, "silent", "an incomplete snapshot omits the member: the read did not reach it")
    return judge_get(h, g)


def judge_observation(kind: str, annotations: dict[str, Any] | Horizon, obs: Observation) -> Verdict:
    """§2.7 "The order of the checks", for one observation: no horizon; an
    event or an operation; an archive's value; a value that is no horizon;
    then the observation itself."""
    h = annotations if isinstance(annotations, Horizon) else horizon(kind, annotations)
    if h.horizon == "none":
        return Verdict(NOT_ASKED, "no_horizon", "the resource declares no horizon (§2.3)")
    if h.horizon == "ignored":
        return Verdict(NOT_ASKED, "kind", "an event or an operation is not this profile's (§2.2)")
    if isinstance(obs, Archive):
        return Verdict(NOT_ASKED, "last_known", "an archive's answer is never current (§2.8)")
    if h.horizon == "invalid":
        return Verdict(UNOBSERVABLE, "not_a_horizon", f"freshness.ttl_s = {h.value!r} is not a horizon (§2.1)")
    if isinstance(obs, Subscription):
        return judge_subscription(h, obs)
    if isinstance(obs, Snapshot):
        return judge_snapshot(h, obs)
    return judge_get(h, obs)


def combine(verdicts: Iterable[Verdict]) -> Verdict:
    """§2.7: "Fresh when any observation is judged fresh. Else stale when
    any is judged stale. Else unobservable when any is. Else not asked, or
    not this profile's, as the observations are." The reason is the first
    of the winning verdict's (judgements.json). No verdict at all is
    unobservable, ``no_observation``: :func:`judge` asks the horizon's steps
    first (§2.7, 0.2)."""
    vs = list(verdicts)
    if not vs:
        return Verdict(UNOBSERVABLE, "no_observation", "nothing observed")
    for token in (FRESH, STALE, UNOBSERVABLE, NOT_ASKED):
        for v in vs:
            if v.verdict == token:
                return v
    raise AssertionError(f"unknown verdicts {vs}")


def judge(kind: str, annotations: dict[str, Any] | Horizon, observations: Iterable[Observation]) -> Verdict:
    """One member, observed one or several ways (§2.7). "With no observation
    at all (0.2), steps 1, 2 and 4 still answer, since they need none … Only
    a member whose resource has a horizon is unobservable for want of an
    observation (§2.3)." """
    h = annotations if isinstance(annotations, Horizon) else horizon(kind, annotations)
    obs = list(observations)
    if not obs:
        if h.horizon == "none":
            return Verdict(NOT_ASKED, "no_horizon", "the resource declares no horizon (§2.3)")
        if h.horizon == "ignored":
            return Verdict(NOT_ASKED, "kind", "an event or an operation is not this profile's (§2.2)")
        if h.horizon == "invalid":
            return Verdict(UNOBSERVABLE, "not_a_horizon", f"freshness.ttl_s = {h.value!r} is not a horizon (§2.1)")
        return Verdict(UNOBSERVABLE, "no_observation", "no observation showed a value (§2.3)")
    return combine(judge_observation(kind, h, o) for o in obs)


def resource_verdict(kind: str, annotations: dict[str, Any] | Horizon,
                     members: dict[str, Verdict]) -> tuple[Verdict, list[str]]:
    """§5's second question, "Is this resource's freshness kept?", over each
    member's combined verdict. Returns the verdict and the stale members it
    names. A deleted member, or one whose value is an archive's, is set
    aside."""
    h = annotations if isinstance(annotations, Horizon) else horizon(kind, annotations)
    if h.horizon == "none":
        return Verdict(NOT_ASKED, "no_horizon", "no horizon"), []
    if h.horizon == "ignored":
        return Verdict(NOT_ASKED, "kind", "an event or an operation"), []
    if h.horizon == "invalid":
        return Verdict(UNOBSERVABLE, "not_a_horizon", f"freshness.ttl_s = {h.value!r}"), []
    valued = {k: v for k, v in members.items() if v.verdict != NOT_ASKED}
    stale = sorted(k for k, v in valued.items() if v.verdict == STALE)
    if stale:
        return Verdict(STALE, "beyond_horizon", f"stale: {', '.join(stale)}"), stale
    if not valued:
        return Verdict(UNOBSERVABLE, "no_observation", "no member read with a value"), []
    if all(v.verdict == FRESH for v in valued.values()):
        first = next(iter(valued.values()))
        return Verdict(FRESH, first.reason, f"{len(valued)} member(s) fresh"), []
    first = next(v for v in valued.values() if v.verdict == UNOBSERVABLE)
    return Verdict(UNOBSERVABLE, first.reason, "no member stale, and at least one unobservable"), []


def member_fields(members: dict[str, Verdict]) -> dict[str, dict[str, str]]:
    """The report field of each member's verdict: its token and its reason,
    never the token alone. Observer §1 (text 0.5, F-295): freshness.v1
    spells not this profile's as not asked with a reason naming the failed
    premise (``kind``, ``deleted``), and "That NotAsked keeps its reason in
    every medium … never show a bare not asked". A deleted member is one.

    The boundary is typed both ways: a member that is not a :class:`Verdict`
    is refused here, and :func:`member_verdicts` refuses a field that is not
    ``{"verdict", "reason"}``, so a caller still reading round 84's token
    string fails at once rather than reading a character of it."""
    for k, v in members.items():
        if not isinstance(v, Verdict):
            raise TypeError(f"member {k}: {v!r} is not a freshness Verdict")
    return {k: v.pair() for k, v in members.items()}


def member_verdicts(fields: dict[str, dict[str, str]]) -> dict[str, str]:
    """Each member's verdict token, read from :func:`read_service`'s member
    field, after checking its shape: a mapping of exactly ``verdict`` and
    ``reason``, both strings, the verdict one of §5's tokens. Anything else,
    a bare token string among them, is a :class:`TypeError`."""
    out: dict[str, str] = {}
    for k, f in fields.items():
        if not (isinstance(f, dict) and set(f) == {"verdict", "reason"}
                and all(isinstance(x, str) for x in f.values())
                and f["verdict"] in (FRESH, STALE, UNOBSERVABLE, NOT_ASKED)):
            raise TypeError(f"member {k}: {f!r} is not a member field {{'verdict', 'reason'}} (observer §1, F-295)")
        out[k] = f["verdict"]
    return out


def service_members(got: dict[str, tuple[Verdict, dict[str, dict[str, str]]]]) -> dict[str, dict[str, str]]:
    """:func:`read_service`'s members, per resource, as verdict tokens, each
    member field's shape checked by :func:`member_verdicts` (a
    :class:`TypeError` otherwise). The callers of :func:`read_service` read
    its members through this."""
    return {rid: member_verdicts(m) for rid, (_, m) in got.items()}


# -- the runtime half: a GET reader's clock (§2.6) ---------------------------------

@dataclass
class ClockTrust:
    """§2.6 "Only with a trusted clock", over one reading: a GET reader
    trusts its clock and a stamping clock (by the stamp's id) to agree
    within the delta, on the deployment's word (``word``), or by a
    measurement (0.2): of the offsets of the live puts it received from that
    clock (its clock at receipt minus the stamp), "the clocks are trusted
    when the offset closest to zero is within the delta", and "a stamp ahead
    of the reader's clock by more than the delta … withdraws the trust for
    the rest of the reading". A later offset above the delta withdraws
    nothing. One object is one reading: "Trust from a measurement lasts
    that reading and no longer"."""

    delta_ns: int = DEFAULT_DELTA_NS
    word: bool = False
    #: stamp id -> [(receipt minus stamp, in ns), …]
    measurements: dict[str, list[int]] = field(default_factory=dict)

    def record(self, stamp_id: str, offset_ns: int) -> None:
        self.measurements.setdefault(stamp_id, []).append(offset_ns)

    def measure(self, stamp_id: str, stamp_ns: int, receipt_ns: int) -> bool:
        self.record(stamp_id, receipt_ns - stamp_ns)
        return self.measured(stamp_id)

    def measured(self, stamp_id: str) -> bool:
        offs = self.measurements.get(stamp_id, [])
        return bool(offs) and min(abs(o) for o in offs) <= self.delta_ns \
            and not any(o < -self.delta_ns for o in offs)

    def trusted(self, stamp_id: str | None) -> bool:
        if self.word:
            return True
        return stamp_id is not None and self.measured(stamp_id)


def stamp_ns_of(ts: Any) -> int:
    """A zenoh Timestamp's time, in ns since the UNIX epoch (core §4.1's
    NTP64)."""
    return ts.get_time_as_ntp64().as_nanos()


@dataclass
class Received:
    """One delivery, as a subscriber records it."""

    kind: str               # "put" or "delete"
    arrival_ns: int         # its monotonic clock
    receipt_utc_ns: int     # its UTC clock, for a measurement
    payload: bytes          # the envelope's record (observer §7.3, 0.14), or the bytes with none
    encoding: str
    stamp_ns: int | None
    stamp_id: str | None
    stamp: str | None
    #: core §4.3 (0.73): its in-band stamp, or None
    inband: Any = None
    #: core §7.5's token where its envelope was refused (undecodable,
    #: observer §7.3): its ``payload`` empty, never read as one
    refusal: str | None = None
    #: a ``freshness.v2`` heartbeat that confirmed the member, at its
    #: arrival, its last put's record and stamps carried over: "its
    #: judgement is v1's with a heartbeat counted as a confirmation"
    #: (:meth:`Subscriber.follow`, round 97)
    confirmation: bool = False


class Subscriber:
    """§2.5: a subscriber on one or more key expressions, ageing each member
    on its own monotonic clock from its declaration. Each put it receives
    with a stamp feeds ``trust`` (§2.6, ground 2). A sample whose key is not
    concrete is discarded (core R6). zenoh-python's callback handler drops
    nothing, so a subscriber here knows of no lost delivery.

    **Bounded** (observer §4.1, 0.10; round 91): the alarms, health and
    config readers merge what it recorded with their GETs' replies, so its
    keys are a tool's merge state. It holds at most ``max_keys`` keys
    (:data:`zk2py.seed.MAX_KEYS` unless told otherwise), the least recently
    arrived forgotten first, whole, and counted in ``forgotten``. A
    forgotten key then holds nothing: no delivery, as before its first.
    Since observer 0.11 the count is reported: :meth:`note`, and each
    reader's reading carries ``forgotten`` (round 92).

    **What it then says of a key it holds nothing of** (round 91, review-bot
    on PR #971): it cannot tell a forgotten key from one never heard, so
    :meth:`observation` dates the listening of every such key from the last
    forgetting (``forgot_ns``), never from the declaration. "No delivery in
    the window" then stays true of what it held, and a silence it forgot
    never reads as one it heard: a judge can only say less, never more
    (observer §4.1, F-398)."""

    def __init__(self, session: Any, selectors: str | list[str], trust: ClockTrust | None = None,
                 max_keys: int | None = None):
        import zenoh

        from . import seed

        self.trust = trust
        self.lock = threading.Lock()
        self.deliveries: OrderedDict[str, list[Received]] = OrderedDict()
        self.max_keys = seed.MAX_KEYS if max_keys is None else max_keys
        #: how many times the bound forgot a key (observer §4.1)
        self.forgotten = 0
        #: the monotonic instant the bound last forgot a key, or None
        self.forgot_ns: int | None = None
        self.discarded: list[str] = []
        self.complete = True
        self.declared_ns = time.monotonic_ns()
        self.session = session
        self.subs = [session.declare_subscriber(s, zenoh.handlers.Callback(self._received))
                     for s in ([selectors] if isinstance(selectors, str) else selectors)]
        #: freshness.v2 (round 97): per followed owner and interface, its
        #: view (:class:`zk2py.freshness_v2.View`), and every heartbeat read,
        #: (arrival, the view's answer, with the body's ``time`` where it was
        #: read: health.v1 0.9's clock question)
        self.views: dict[tuple[str, str], Any] = {}
        self.beats: dict[tuple[str, str], list[tuple[int, dict[str, Any]]]] = {}

    def follow(self, address: str, canonical: dict[str, Any], fingerprint: str) -> None:
        """Follow ``address``'s heartbeats covering the revision given
        (``freshness.v2`` §2.5; ``health.v1`` 0.8, ``alarms.v1`` 0.11 and
        ``config.v1`` 0.9 confirm their state so): a subscriber on its
        heartbeat key, each heartbeat read by the rule's four steps over a
        view of the members this subscription hears. A member a read
        heartbeat confirms gets a delivery of its own, its last put's record
        and stamps at the heartbeat's arrival (``confirmation``), so §2.5
        judges it as v1's judgement with a heartbeat counted as a
        confirmation; while the last read heartbeat followed a gap, every
        member of the interface reads as after a lost delivery
        (:meth:`observation`). No re-read is made: the readers built on this
        subscription seed and re-read by their own S4 GETs."""
        import zenoh

        from . import freshness_v2, heartbeat

        lo = heartbeat.layout_of(canonical, fingerprint)
        k = (address, lo.interface)
        with self.lock:
            if k in self.views:
                return
            self.views[k] = freshness_v2.View(lo, [c.resource for c in lo.collections], address)
            self.beats[k] = []
        hb = heartbeat.heartbeat_key(*address.split("/"), lo.interface)
        self.subs.append(self.session.declare_subscriber(
            hb, zenoh.handlers.Callback(lambda smp, k=k: self._beat(k, smp))))

    def _view_of(self, key: str) -> Any:
        chunks = key.split("/")
        return None if len(chunks) < 4 else self.views.get(("/".join(chunks[1:3]), chunks[3]))

    def _beat(self, k: tuple[str, str], sample: Any) -> None:
        arrival, utc = time.monotonic_ns(), time.time_ns()
        try:
            env = zk2c.parse(sample.payload.to_bytes(), "put")
        except zk2c.Refused as e:
            answer: dict[str, Any] = {"refused": e.token}
            env = None
        with self.lock:
            view = self.views[k]
            if env is not None:
                answer = {"refused": "form"} if env.form != "heartbeat" else \
                    view.heartbeat(env.tag, env.stamp, env.record or b"", arrival)
                if "read" in answer:
                    # health.v1 §2.5 (0.9): the clock question reads "the
                    # time of each heartbeat", all ones while the guard
                    # holds (freshness.v2 §2.10, 0.4)
                    from . import heartbeat

                    answer = {**answer, "time": heartbeat.read(view.layout, env.record or b"")["time"]}
            self.beats[k].append((arrival, answer))
            for sec in answer.get("read", {}).get("sections", {}).values():
                if not isinstance(sec, dict):
                    continue
                for key in sec["confirmed"]:
                    got = self.deliveries.get(key)
                    last = next((x for x in reversed(got or []) if x.kind == "put"), None)
                    if last is None:
                        continue
                    self._record(key, Received("put", arrival, utc, last.payload, last.encoding, None, None, None,
                                               last.inband, last.refusal, confirmation=True))

    def read_view(self, address: str, iface: str, asked_ns: int, arrived_ns: int,
                  replies: dict[str, Any]) -> None:
        """A reader's complete S4 GET of a followed owner's collections, its
        answer a read of each (``freshness.v2`` §2.5 "The re-read": "The
        answer MUST then replace its view of the collection"). ``replies``
        are :func:`zk2py.live.get_state`'s, by key."""
        with self.lock:
            view = self.views.get((address, iface))
            if view is None:
                return
            for c in view.layout.collections:
                ok = [(k, r.inband) for k, r in replies.items() if not r.deleted and r.refusal is None]
                gone = [(k, r.inband) for k, r in replies.items() if r.deleted]
                if any(view.collection_of(k) == c for k, _ in ok + gone):
                    view.read(c.resource, asked_ns, arrived_ns, ok, gone)

    def in_step(self, address: str, iface: str, resource: str, since_ns: int) -> bool:
        """``alarms.v1`` §2.15 (0.11): whether the latest heartbeat covering
        ``resource`` heard since ``since_ns`` was in step with the view, its
        section's digest and live count the view's (``freshness.v2`` §2.5);
        False when none was heard."""
        with self.lock:
            heard = [a for t, a in self.beats.get((address, iface), []) if t >= since_ns]
        for a in reversed(heard):
            secs = a.get("read", {}).get("sections")
            if secs is None:
                continue  # not read (another revision, refused, older): not counted
            return isinstance(secs.get(resource), dict)
        return False

    def _received(self, sample: Any) -> None:
        import zenoh

        arrival, utc = time.monotonic_ns(), time.time_ns()
        key = str(sample.key_expr)
        if "*" in key:
            with self.lock:
                self.discarded.append(key)
            return
        ts = sample.timestamp
        stamp_ns = None if ts is None else stamp_ns_of(ts)
        text = None if ts is None else str(ts)
        sid = None if text is None else text.split("/", 1)[1]
        kind = "delete" if sample.kind == zenoh.SampleKind.DELETE else "put"
        read = zk2c.read_sample(sample)
        record = b"" if kind == "delete" or read.refused else (
            read.record if read.record is not None else sample.payload.to_bytes())
        rec = Received(kind, arrival, utc, record, str(sample.encoding), stamp_ns, sid, text,
                       read.stamp, read.refusal)
        with self.lock:
            self._record(key, rec)
            view = self._view_of(key)
            if view is not None and not read.refused and view.collection_of(key) is not None:
                view.mutate(key, read.stamp, arrival, kind == "put")
        if self.trust is not None and kind == "put" and stamp_ns is not None and sid is not None:
            self.trust.measure(sid, stamp_ns, utc)

    def _record(self, key: str, rec: Received) -> None:
        """One delivery, under the lock: the key moved to the recent end,
        and the least recent forgotten past the bound."""
        self.deliveries.setdefault(key, []).append(rec)
        self.deliveries.move_to_end(key)
        while len(self.deliveries) > self.max_keys:
            self.deliveries.popitem(last=False)
            self.forgotten += 1
            self.forgot_ns = rec.arrival_ns

    def keys(self) -> list[str]:
        with self.lock:
            return list(self.deliveries)

    def bound(self) -> tuple[int, str]:
        """The bound's count and its note, read under one lock (observer
        §4.1, 0.12: "counts each time the bound forgets a key, and reports
        the count"; :func:`zk2py.seed.bound_note`)."""
        from . import seed

        with self.lock:
            return self.forgotten, seed.bound_note(self.forgotten, self.max_keys)

    def note(self) -> str:
        """The bound's note alone (:meth:`bound`)."""
        return self.bound()[1]

    def of(self, key: str) -> list[Received]:
        with self.lock:
            return list(self.deliveries.get(key, []))

    def observation(self, key: str, now_ns: int | None = None) -> Subscription:
        """§2.5 at ``now_ns`` (its monotonic clock; now by default)."""
        now = time.monotonic_ns() if now_ns is None else now_ns
        with self.lock:
            got = list(self.deliveries.get(key, []))
            forgot = self.forgot_ns
        last = None if not got else Delivery(got[-1].kind, now - got[-1].arrival_ns)
        # A key it holds nothing of may be one the bound forgot: it has
        # listened for it, knowing nothing, only since the last forgetting.
        since = self.declared_ns if got or forgot is None else max(self.declared_ns, forgot)
        # freshness.v2 §2.5 (round 97): "A subscriber MUST then read every
        # member of the interface's collections as v1 §2.5 rule 3 reads a
        # lost delivery, until a heartbeat arrives in sequence."
        with self.lock:
            view = self._view_of(key)
            gap = view is not None and view.collection_of(key) is not None and view.gap
        return Subscription(now - since, self.complete and not gap, last)

    def close(self) -> None:
        for s in self.subs:
            try:
                s.undeclare()
            except Exception:  # noqa: BLE001 - closing anyway
                pass


@dataclass
class GetReading:
    """A GET's reply for one member, kept so that it can be judged at a
    later instant: §2.6 ages it "at the instant it judges"."""

    reply: str | None         # None (no reply), "put" or "delete"
    stamp_ns: int | None = None
    stamp_id: str | None = None
    stamp: str | None = None
    payload: bytes = b""
    #: the reply's ``Encoding``, which picks a held JSON Schema type's wire
    #: (observer §7.3; :func:`zk2py.decoding.wire_of`)
    encoding: str = ""
    #: core §4.3 (0.73): the reply's in-band stamp, or None
    inband: Any = None
    #: ``freshness.v2`` 0.7 §2.6 (core 0.100): where the member came as a
    #: snapshot's entry, its confirmation's instant in Unix ns ("the
    #: snapshot's time for confirmation 1, and the entry's conf for 2"),
    #: None for none or an unknown time; ``snapshot`` whether it so came,
    #: ``complete`` whether its snapshot is complete (a member it omits: no
    #: entry), and the owner's session id, whose clock built the snapshot
    snapshot: bool = False
    complete: bool = False
    confirmed_ns: int | None = None
    owner_id: str | None = None
    #: the run of the snapshot that listed it (:attr:`zk2py.live.StateReply.snapshot_run`),
    #: by which ``alarms.v1`` 0.14 attributes a GET's entry (F-503's answer)
    snapshot_run: Any = None

    def observation(self, trust: ClockTrust, now_utc_ns: int | None = None) -> Observation:
        if self.snapshot:
            # v2 §2.6: "a GET reader MUST age an entry from its confirmation
            # as v1 §2.6 ages a reply from its stamp". "The clock is the
            # owner's, which built the snapshot."
            entry = None if self.reply is None else "tombstone" if self.reply == "delete" else "live"
            now = time.time_ns() if now_utc_ns is None else now_utc_ns
            age = None if self.confirmed_ns is None else now - self.confirmed_ns
            return Snapshot(self.complete, entry, age, trust.trusted(self.owner_id), trust.delta_ns)
        if self.reply is None:
            return Get(None, trust.word, trust.delta_ns)
        if self.reply == "delete":
            return Get(Reply("delete"), trust.trusted(self.stamp_id), trust.delta_ns)
        now = time.time_ns() if now_utc_ns is None else now_utc_ns
        age = None if self.stamp_ns is None else now - self.stamp_ns
        return Get(Reply("put", age), trust.trusted(self.stamp_id), trust.delta_ns)


def reading_of(r: Any, epoch_s: int = 0, complete: bool = True, owner_id: str | None = None) -> GetReading:
    """One reply of S4's GET (:class:`zk2py.live.StateReply`) as §2.6 reads
    it: a per-key resource's reply by its Zenoh timestamp, a snapshot's
    entry (core 0.100) by its confirmation, ``freshness.v2`` 0.7 §2.6,
    counted from the deployment epoch ``epoch_s``."""
    if r is None:
        return GetReading(None)
    if getattr(r, "snapshot_key", None) is not None:
        t = r.snapshot_time if r.confirmation == "time" else r.conf if r.confirmation == "conf" else None
        at = None if t is None else ((epoch_s + t["s"]) * 1000 + t["ms"]) * 1_000_000
        return GetReading("delete" if r.deleted else "put", None, None, None, r.payload, r.encoding, r.inband,
                          True, complete, at, owner_id, getattr(r, "snapshot_run", None))
    if r.deleted:
        return GetReading("delete", r.stamp_ns, r.stamp_id, r.stamp, inband=r.inband)
    if r.refusal is not None:
        # observer §7.3 (0.14): a refused envelope is undecodable; its bytes
        # are never read, so the reading holds an empty payload.
        return GetReading("put", r.stamp_ns, r.stamp_id, r.stamp, b"", r.encoding, r.inband)
    return GetReading("put", r.stamp_ns, r.stamp_id, r.stamp, r.payload, r.encoding, r.inband)


def get_reading(session: Any, key: str, timeout: float = 1.0, epoch_s: int = 0,
                owner_id: str | None = None, contracts: list[Any] | None = None) -> GetReading:
    """A GET of one state member as core S4 says (target All, consolidation
    None, core 0.100), the member's reply kept for §2.6. Where it comes as a
    snapshot's entry (a member read: "a snapshot of that member alone, on
    the member's own key"), ``freshness.v2`` 0.7 §2.6 ages it from its
    confirmation, each time counted from the deployment epoch ``epoch_s``
    (the catalog's, else 0); a snapshot of no entry on the key is the
    owner's word, its member deleted (§2.6: "A member a complete snapshot
    omits has no value"). ``contracts``, the revisions the reader holds: the
    owner's among them is read by its descriptors before the GET, so that a
    stray ``reply_del`` is never merged as a deletion (core 0.102 §4.2,
    review-bot on #1100, 2); with none the owner's revision cannot be read,
    and every ``reply_del`` is merged, as the text says."""
    from . import live
    from . import seed_snapshot as ss

    st = live.get_state(session, key, timeout=timeout,
                        revision=None if contracts is None else ss.ByDescriptor(contracts))
    mine = [r for r in st.replies if r.key == key]
    whole = bool(st.complete_for(key))
    if not mine:
        if st.snapshots:
            return GetReading(None, snapshot=True, complete=whole, owner_id=owner_id)
        return GetReading(None)
    # review-bot on #1090: one reading of a reply, reading_of's, its
    # snapshot complete or not as this GET heard it
    return reading_of(mine[-1], epoch_s, whole, owner_id)


# -- a tool's verdict on a service's resources (§5, scenarios §6) ------------------

class ServiceRead(dict):
    """:func:`read_service`'s answer: ``{"<token>/<template>": (verdict,
    members)}``, and ``forgotten``, the count of the times its recording's
    bound forgot a key over the window, with its ``note`` (observer §4.1,
    0.12: "counts each time the bound forgets a key, and reports the
    count"; round 92)."""

    forgotten: int = 0
    note: str = ""


def read_service(session: Any, address: str, contracts: list[dict[str, Any]], window_s: float,
                 trust: ClockTrust, on_subscribed: Any = None,
                 get_timeout: float = 1.0) -> ServiceRead:
    """§5's second question for each resource of ``address`` (a
    ``<system>/<service>``), over a window, as scenarios §6 reads: "it
    subscribes to every exposed stream and state resource first, then GETs
    every state resource, and measures its clock from what it receives. It
    judges every member at the end of the window, and reports one verdict
    per resource."

    ``contracts`` are canonical forms (a bundle's, core §9.5). A member is
    one the tool knows exists, received or answered (§2.5). Returns
    ``{"<token>/<template>": (verdict, {member key: {"verdict", "reason"}})}``:
    each member's reason is kept beside its token (:func:`member_fields`,
    observer §1, text 0.5), so a deleted member reads not asked, ``deleted``,
    never a bare not asked. The answer is a :class:`ServiceRead`, which also
    reports how many keys the recording's bound forgot (observer §4.1, 0.11)."""
    from .owner import _template_key

    plain = []
    for c in contracts:
        for r in c["resources"]:
            plain.append((c["interface"], r, f"zk2/{address}/{c['interface']}/{r['token']}/{r['template']}"))
    watched = [(iface, r, key) for iface, r, key in plain if r["kind"] in ("state", "stream")]
    sub = Subscriber(session, [_template_key(key) for _, _, key in watched], trust)
    try:
        if on_subscribed is not None:
            on_subscribed()
        readings: dict[str, GetReading] = {}
        from . import live
        from . import seed_snapshot as ss

        # core 0.102 §4.2 (review-bot on #1100, 2): the owner's revision,
        # among the canonical forms the tool holds, read by its descriptors
        by = ss.ByDescriptor.of_canonical(contracts)
        for _, r, key in watched:
            if r["kind"] != "state":
                continue
            st = live.get_state(session, _template_key(key), timeout=get_timeout, revision=by)
            for rep in st.replies:
                readings[rep.key] = (GetReading("delete", rep.stamp_ns, rep.stamp_id, rep.stamp) if rep.deleted
                                     else GetReading("put", rep.stamp_ns, rep.stamp_id, rep.stamp, rep.payload))
        end = sub.declared_ns + int(window_s * NS)
        while time.monotonic_ns() < end:
            time.sleep(min(0.05, (end - time.monotonic_ns()) / NS))
        now_mono, now_utc = time.monotonic_ns(), time.time_ns()
        import zenoh

        out = ServiceRead()
        out.forgotten, out.note = sub.bound()
        for _, r, key in plain:
            rid = f"{r['token']}/{r['template']}"
            h = horizon(r["kind"], r["annotations"])
            expr = zenoh.KeyExpr(_template_key(key))
            known = sorted({k for k in sub.keys() + list(readings) if expr.intersects(zenoh.KeyExpr(k))})
            members: dict[str, Verdict] = {}
            for m in known:
                obs: list[Observation] = [sub.observation(m, now_mono)]
                if r["kind"] == "state":
                    obs.append(readings.get(m, GetReading(None)).observation(trust, now_utc))
                members[m] = judge(r["kind"], h, obs)
            out[rid] = (resource_verdict(r["kind"], h, members)[0], member_fields(members))
        return out
    finally:
        sub.close()

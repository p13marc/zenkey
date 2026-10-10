"""``freshness.v1``: how long a value stays current (profile text 0.1,
draft, written against core 0.21; ``spec/profiles/freshness/v1.md``).

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
from dataclasses import dataclass, field
from decimal import ROUND_HALF_EVEN, Decimal
from typing import Any, Iterable

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


Observation = Subscription | Get | Archive


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
    payload: bytes
    encoding: str
    stamp_ns: int | None
    stamp_id: str | None
    stamp: str | None


class Subscriber:
    """§2.5: a subscriber on one or more key expressions, ageing each member
    on its own monotonic clock from its declaration. Each put it receives
    with a stamp feeds ``trust`` (§2.6, ground 2). A sample whose key is not
    concrete is discarded (core R6). zenoh-python's callback handler drops
    nothing, so a subscriber here knows of no lost delivery."""

    def __init__(self, session: Any, selectors: str | list[str], trust: ClockTrust | None = None):
        import zenoh

        self.trust = trust
        self.lock = threading.Lock()
        self.deliveries: dict[str, list[Received]] = {}
        self.discarded: list[str] = []
        self.complete = True
        self.declared_ns = time.monotonic_ns()
        self.subs = [session.declare_subscriber(s, zenoh.handlers.Callback(self._received))
                     for s in ([selectors] if isinstance(selectors, str) else selectors)]

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
        rec = Received(kind, arrival, utc, sample.payload.to_bytes(), str(sample.encoding), stamp_ns, sid, text)
        with self.lock:
            self.deliveries.setdefault(key, []).append(rec)
        if self.trust is not None and kind == "put" and stamp_ns is not None and sid is not None:
            self.trust.measure(sid, stamp_ns, utc)

    def of(self, key: str) -> list[Received]:
        with self.lock:
            return list(self.deliveries.get(key, []))

    def observation(self, key: str, now_ns: int | None = None) -> Subscription:
        """§2.5 at ``now_ns`` (its monotonic clock; now by default)."""
        now = time.monotonic_ns() if now_ns is None else now_ns
        got = self.of(key)
        last = None if not got else Delivery(got[-1].kind, now - got[-1].arrival_ns)
        return Subscription(now - self.declared_ns, self.complete, last)

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

    def observation(self, trust: ClockTrust, now_utc_ns: int | None = None) -> Get:
        if self.reply is None:
            return Get(None, trust.word, trust.delta_ns)
        if self.reply == "delete":
            return Get(Reply("delete"), trust.trusted(self.stamp_id), trust.delta_ns)
        now = time.time_ns() if now_utc_ns is None else now_utc_ns
        age = None if self.stamp_ns is None else now - self.stamp_ns
        return Get(Reply("put", age), trust.trusted(self.stamp_id), trust.delta_ns)


def get_reading(session: Any, key: str, timeout: float = 1.0) -> GetReading:
    """A GET of one state member as core S4 says (target All, consolidation
    Latest), the member's reply kept for §2.6."""
    from . import live

    st = live.get_state(session, key, timeout=timeout)
    mine = [r for r in st.replies if r.key == key]
    if not mine:
        return GetReading(None)
    r = mine[-1]
    if r.deleted:
        return GetReading("delete", r.stamp_ns, r.stamp_id, r.stamp)
    return GetReading("put", r.stamp_ns, r.stamp_id, r.stamp, r.payload)


# -- a tool's verdict on a service's resources (§5, scenarios §6) ------------------

def read_service(session: Any, address: str, contracts: list[dict[str, Any]], window_s: float,
                 trust: ClockTrust, on_subscribed: Any = None,
                 get_timeout: float = 1.0) -> dict[str, tuple[Verdict, dict[str, str]]]:
    """§5's second question for each resource of ``address`` (a
    ``<system>/<service>``), over a window, as scenarios §6 reads: "it
    subscribes to every exposed stream and state resource first, then GETs
    every state resource, and measures its clock from what it receives. It
    judges every member at the end of the window, and reports one verdict
    per resource."

    ``contracts`` are canonical forms (a bundle's, core §9.5). A member is
    one the tool knows exists, received or answered (§2.5). Returns
    ``{"<token>/<template>": (verdict, {member key: its verdict token})}``."""
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

        for _, r, key in watched:
            if r["kind"] != "state":
                continue
            st = live.get_state(session, _template_key(key), timeout=get_timeout)
            for rep in st.replies:
                readings[rep.key] = (GetReading("delete", rep.stamp_ns, rep.stamp_id, rep.stamp) if rep.deleted
                                     else GetReading("put", rep.stamp_ns, rep.stamp_id, rep.stamp, rep.payload))
        end = sub.declared_ns + int(window_s * NS)
        while time.monotonic_ns() < end:
            time.sleep(min(0.05, (end - time.monotonic_ns()) / NS))
        now_mono, now_utc = time.monotonic_ns(), time.time_ns()
        import zenoh

        out: dict[str, tuple[Verdict, dict[str, str]]] = {}
        for _, r, key in plain:
            rid = f"{r['token']}/{r['template']}"
            h = horizon(r["kind"], r["annotations"])
            expr = zenoh.KeyExpr(_template_key(key))
            known = sorted({k for k in list(sub.deliveries) + list(readings) if expr.intersects(zenoh.KeyExpr(k))})
            members: dict[str, Verdict] = {}
            for m in known:
                obs: list[Observation] = [sub.observation(m, now_mono)]
                if r["kind"] == "state":
                    obs.append(readings.get(m, GetReading(None)).observation(trust, now_utc))
                members[m] = judge(r["kind"], h, obs)
            out[rid] = (resource_verdict(r["kind"], h, members)[0], {k: v.verdict for k, v in members.items()})
        return out
    finally:
        sub.close()

"""The far side of a constrained face (core §8.1 and §8.5 at 0.91, observer
§7.5 at 0.23, ``link.v2`` §2.20 and §2.21 at 0.4; #988 L4, #609 round 99).

A caller, a consumer or a tool across a constrained face follows its face
policy's word on two things:
- **the reply timeout** (core §8.1, "Across a constrained face"): "A
  caller, a consumer or a tool across a constrained face whose face policy
  states a reply timeout (link.v2 §2.1's reply_timeout_ms) SHOULD wait at
  least that long for each reply it asks for: a call's, a GET's and a
  seed's" (:meth:`FarSide.call_timeout_s`, :meth:`FarSide.get_timeout_s`);
- **declaring again after silence** (core §8.5, observer §7.5): "A consumer
  or a tool across a constrained face whose medium repairs no lost frame
  SHOULD declare a subscription again when it has heard nothing on it for
  two of its horizons while its session's transport stayed up"
  (:class:`Redeclaring`).

:func:`far_side` reads a face of a ``link.v2`` policy through
:func:`zk2py.link_v2.parse_policy`; its session runs the face's
``[transport]`` (``link.v2`` §2.20: "Its session runs the face's
[transport], as the far end's block writes it"), :meth:`FarSide.config`.

zk2py's choices, where the texts leave one:
- **A call's own timeout is taken at its word**, as the reference's is; with
  none, the call waits the longer of §5.1's (the operation's
  ``timeout_ms``, else 10 s) and the face's. A GET's and a seed's timeout,
  zk2py's own default or the caller's, is floored at the face's: zk2py's
  consumers and tools set one on every GET (the scenarios' 1 s), so a floor
  is the only reading under which the SHOULD binds them.
- **A transport's move** is read from zenoh 1.10.1's transport events
  (``SessionInfo.declare_transport_events_listener``), and "transport up"
  from ``SessionInfo.transports()``: a session with no open transport is cut
  off, and declares nothing.
"""

from __future__ import annotations

import threading
import time
from dataclasses import dataclass
from typing import Any, Callable

#: core §5.1 "The timeout": "with neither, 10 s, zenoh's default query timeout"
DEFAULT_CALL_TIMEOUT_S = 10.0


@dataclass(frozen=True)
class FarSide:
    """One face of a ``link.v2`` policy, as its far side reads it."""

    face: str
    #: link.v2 §2.1: "how long a reply may take to come back"
    reply_timeout_ms: int
    #: link.v2 §2.1: "Whether the medium repairs a lost frame below zenoh"
    arq: bool
    #: the policy's [transport], every key, the defaults of absent ones
    transport: dict[str, Any]

    @property
    def reply_timeout_s(self) -> float:
        return self.reply_timeout_ms / 1000

    def call_timeout_s(self, own: float | None, recommended_ms: int | None = None) -> float:
        """A call's timeout (core §5.1, §8.1): ``own``, the caller's, at its
        word; else the longer of the operation's ``timeout_ms`` (else 10 s)
        and the face's reply timeout."""
        return call_timeout_s(own, recommended_ms, self)

    def get_timeout_s(self, own: float) -> float:
        """A GET's or a seed's timeout across the face: at least the face's
        reply timeout (core §8.1)."""
        return max(own, self.reply_timeout_s)

    @property
    def redeclares(self) -> bool:
        """Core §8.5: only where "its medium repairs no lost frame";
        link.v2 §2.21: "With arq true nothing is declared again"."""
        return not self.arq

    def config(self) -> dict[str, Any]:
        """The far end's session configuration, each of the eight keys at
        zenoh's configuration key (link.v2 §2.18's block, ``wait_before_close``
        in µs), for :func:`zk2py.live.open_client`'s ``transport``."""
        out = {}
        from .link_v2 import TRANSPORT

        for k, (path, _, _) in TRANSPORT.items():
            v = self.transport[k]
            out[path] = v * 1000 if k == "wait_before_close_ms" else v
        return out


def far_side(policy_text: str, face: str) -> FarSide:
    """Face ``face`` of a ``link.v2`` policy, as its far side reads it.
    Raises ``ValueError`` for a policy with an error, a ``link.v1`` policy
    (which states no reply timeout, no ``arq`` and no transport), or a face
    it does not have."""
    from . import link_v2

    policy, findings = link_v2.parse_policy(policy_text)
    if policy is None:
        raise ValueError(f"the face policy is refused: {[f['code'] + ' ' + f['at'] for f in findings]}")
    if policy.v1 is not None or policy.transport is None:
        raise ValueError("a link.v1 policy states no reply timeout, arq or transport (link.v2 §2.1)")
    for f in policy.faces:
        if f.face.name == face:
            return FarSide(face, f.reply_timeout_ms, f.arq, dict(policy.transport))
    raise ValueError(f"the face policy has no face {face!r}")


def call_timeout_s(own: float | None, recommended_ms: int | None, across: FarSide | None) -> float:
    """Core §5.1 "The timeout": "The timeout is the caller's (§8.1). A
    caller that sets none waits the operation's timeout_ms, the contract's
    recommendation, and with neither, 10 s … Across a constrained face, it
    waits at least the face's reply timeout (§8.1)." A timeout the caller
    sets is taken at its word (zk2py's reading, the reference's)."""
    if own is not None:
        return own
    base = recommended_ms / 1000 if recommended_ms else DEFAULT_CALL_TIMEOUT_S
    return base if across is None else max(base, across.reply_timeout_s)


# -- a subscription's horizon (core §8.5) ------------------------------------------------------

def resource_horizon_s(contract: Any, r: dict[str, Any]) -> float | None:
    """Core §8.5 "A subscription's horizon is that of what it hears: its
    resource's freshness.ttl_s above 0, or a heartbeat's … A state of a
    contract that uses freshness.v2 has none of its own: its owner re-puts
    nothing, and its heartbeat speaks for it." None for no horizon."""
    from . import freshness, freshness_v2

    kind = {"stream": "stream", "@stream": "stream", "state": "state", "@state": "state",
            "events": "event"}.get(r["token"])
    if kind is None:
        return None
    if kind == "state" and freshness_v2.uses_v2(contract.canonical):
        return None
    h = freshness.horizon(kind, r["annotations"])
    return float(h.ttl_s) if h.horizon == "within" else None


def heartbeat_horizon_s(contract: Any) -> float | None:
    """A heartbeat's horizon: "the shortest horizon of the collections it
    covers" (core §8.5, freshness.v2 §2.4), the covered contract's states
    with one; None where it uses no freshness.v2 or has no collection."""
    from . import freshness, freshness_v2

    if not freshness_v2.uses_v2(contract.canonical):
        return None
    hs = [freshness.horizon("state", r["annotations"]) for r in contract.canonical["resources"]
          if r["token"] in ("state", "@state")]
    hs = [h.ttl_s for h in hs if h.horizon == "within"]
    return float(min(hs)) if hs else None


def scope_horizons(contracts: list[Any], addresses: list[str]) -> dict[str, float | None]:
    """Observer §7.5 (0.23): "Its horizon comes from the contracts in hand,
    as its scope does: the shortest horizon of the resources the scope reads
    that have one. A scope with none is never declared again." Each of
    :func:`zk2py.face.scopes`' scopes, its horizon or None; a heartbeat scope
    of ``freshness.v2``'s template reads the shortest horizon of the
    contracts it covers that are held."""
    from . import face, freshness_v2

    held = list(contracts)
    out: dict[str, float | None] = {}
    for address in addresses:
        for c in held:
            for r in c.canonical["resources"]:
                if r["token"] not in face.SUBSCRIBED:
                    continue
                key = f"zk2/{address}/{c.interface}/{face.scope(r['token'], r['template'])}"
                if c.interface == freshness_v2.PROFILE and r["template"] == "heartbeat/{iface}":
                    hs = [heartbeat_horizon_s(x) for x in held if x.interface != freshness_v2.PROFILE]
                    h = min((x for x in hs if x is not None), default=None)
                else:
                    h = resource_horizon_s(c, r)
                if key not in out or out[key] is None:
                    out[key] = h
                elif h is not None:
                    out[key] = min(out[key], h)
    return out


# -- declaring again (core §8.5) -------------------------------------------------------------

class Redeclaring:
    """One subscription across a face whose medium repairs no lost frame
    (core §8.5): declared again when it has heard nothing for two of its
    horizons while its session's transport stayed up.

    - **The silence** "counts from the latest of three instants: the
      subscription's declaration, its last sample, and its session's last
      transport opening or closing".
    - **Declaring again** "undeclares the subscription, then declares it
      anew. Beside the old one, zenoh 1.10.1 would join the new subscriber to
      it and send nothing".
    - "Through a session cut off from every router nothing is declared": no
      open transport, no declaration.
    - ``horizon_s`` None, or ``redeclare`` False (``arq = true``), declares
      once and never again.

    :attr:`redeclared` holds each declaration made again, its instant on the
    monotonic clock. The callback runs in zenoh's thread, as a plain
    subscriber's does."""

    def __init__(self, session: Any, key: str, callback: Callable[[Any], None], horizon_s: float | None,
                 *, redeclare: bool = True, clock: Callable[[], float] = time.monotonic) -> None:
        import zenoh

        self.session, self.key, self.callback, self.horizon_s = session, key, callback, horizon_s
        self.clock = clock
        self.lock = threading.Lock()
        self.last = clock()
        self.redeclared: list[float] = []
        self.moves: list[tuple[float, str]] = []
        self.stop = threading.Event()
        self.sub = session.declare_subscriber(key, zenoh.handlers.Callback(self._heard))
        self.declared_at = self.last
        self.listener = None
        self.thread: threading.Thread | None = None
        if redeclare and horizon_s is not None and horizon_s > 0:
            self.listener = session.info.declare_transport_events_listener(
                zenoh.handlers.Callback(self._moved), history=False)
            self.thread = threading.Thread(target=self._watch, name=f"redeclare {key}", daemon=True)
            self.thread.start()

    def _heard(self, sample: Any) -> None:
        with self.lock:
            self.last = self.clock()
        self.callback(sample)

    def _moved(self, event: Any) -> None:
        import zenoh

        with self.lock:
            self.last = self.clock()
            self.moves.append((self.last, "opened" if event.kind == zenoh.SampleKind.PUT else "closed"))

    def _up(self) -> bool:
        try:
            return bool(self.session.info.transports())
        except Exception:  # noqa: BLE001 - a session closing has no transport
            return False

    def _watch(self) -> None:
        import zenoh

        assert self.horizon_s is not None
        tick = min(self.horizon_s / 20, 0.1)
        while not self.stop.wait(tick):
            with self.lock:
                silent = self.clock() - self.last
            if silent < 2 * self.horizon_s or not self._up():
                continue
            # Undeclare, then declare: beside its twin a new subscriber
            # sends nothing (core Appendix B). Outside the lock, so that a
            # callback in flight never waits on it.
            old = self.sub
            try:
                old.undeclare()
            except Exception:  # noqa: BLE001 - one undeclared already declares anew all the same
                pass
            if self.stop.is_set():
                return
            new = self.session.declare_subscriber(self.key, zenoh.handlers.Callback(self._heard))
            with self.lock:
                self.sub = new
                self.last = self.declared_at = self.clock()
                self.redeclared.append(self.last)

    def undeclare(self) -> None:
        self.stop.set()
        if self.thread is not None:
            self.thread.join(timeout=2.0)
        for x in (self.sub, self.listener):
            if x is None:
                continue
            try:
                x.undeclare()
            except Exception:  # noqa: BLE001 - closing anyway
                pass


def subscribe(session: Any, key: str, callback: Callable[[Any], None], horizon_s: float | None,
              across: FarSide | None) -> Any:
    """A subscription across ``across``: a :class:`Redeclaring` where the
    face's medium repairs no lost frame and the subscription has a horizon,
    else a plain subscriber (``across`` None, ``arq = true``, or no
    horizon). Either is undeclared by ``.undeclare()``."""
    import zenoh

    if across is not None and across.redeclares and horizon_s:
        return Redeclaring(session, key, callback, horizon_s)
    return session.declare_subscriber(key, zenoh.handlers.Callback(callback))

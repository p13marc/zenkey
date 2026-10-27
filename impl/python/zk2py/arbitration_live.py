"""``arbitration.v1`` on a bus (profile text 0.1, core 0.54; #609, round 77).

- :class:`Actuator` is a consumer's runtime for one arbitrated role
  (§2.1): it refuses an invalid declaration before anything is declared
  (§2.2), then runs zk2py's :class:`zk2py.owner.Owner` for its service and
  hands :class:`zk2py.arbitration.Arbiter` every event in one order, each
  at an instant of its monotonic clock taken under one lock:
  - each sample of the role's subscription, on the service's session;
  - each interface token its **watch** hears declared or withdrawn: one
    liveliness subscriber per entry on a watch session of its own, without
    history, its round trip made after declaring them (§2.7);
  - each move of the watch session's transport, and the round trip asked
    after it once it stands (§2.8);
  - each read again, 0.1 s after a withdrawal that leaves an address with
    no token known held, on a reader session that never held a liveliness
    subscriber (§2.7);
  - its timer, armed just after the active provider's due instant (§2.9).

  It tells each switch as a JSON line (``scenarios.md``'s conventions), and
  delivers only the active provider's samples to its application, here a
  put of each on its ``angle`` state, payload unchanged.
- :class:`Commander` is an owner of the commanders' stream publishing it at
  a period, which holds and resumes, and re-mints make-before-break.
- :class:`ToolView` is a tool's own view (§2.14): the same rules over its
  own subscription, watch and clock, never the consumer's.
"""

from __future__ import annotations

import json
import sys
import threading
import time
from typing import Any, Callable

from . import arbitration as ab
from . import live
from . import timing as tm

NS = 1_000_000_000
MS = 1_000_000


class Refused(RuntimeError):
    """§2.2: "A consumer's runtime MUST refuse to start an instance holding a
    role whose declaration is invalid … no instance token appears, and
    nothing of the instance is declared or put"."""


def _expr(entry: str, interface: str, token: str, template: str, params: dict[str, str]) -> str:
    """The role's key expression at one entry: its chunks as the parameter
    bindings give them, the entry's ``*`` kept."""
    from . import templates
    from .slug import slug

    tpl = templates.parse_template(template)
    chunks = [s.text if s.kind == templates.LITERAL else slug(params[s.text]) for s in tpl.segments]
    return f"zk2/{entry}/{interface}/{token}/{'/'.join(chunks)}"


def _ms(t: int | None, t0: int) -> float | None:
    return None if t is None else round((t - t0) / MS, 3)


class Actuator:
    """One instance of ``address`` implementing ``contract``, whose role
    ``role`` is arbitrated over the revision ``held`` of its required
    interface (§2.3: the one it was built against). ``bindings`` is the
    role's binding in its configured order (core R1), ``params`` its
    parameter bindings (core R2). ``emit`` takes each line it tells.
    ``watch_connect`` is the endpoint its watch session uses (a link the
    test can cut, scenarios §7), ``connect`` the others'."""

    def __init__(self, address: str, contract: Any, held: Any, *, connect: str, bindings: list[str],
                 role: str = "cmd", params: dict[str, str] | None = None, watch_connect: str | None = None,
                 reader_connect: str | None = None, epsilon_ms: int | None = None,
                 emit: Callable[[str], None] | None = None, reread_delay_s: float = 0.1,
                 deliver: Callable[[str, bytes], None] | None = None, infer_moves: bool = True,
                 warn: Callable[[str], None] | None = None) -> None:
        import zenoh

        from .owner import Owner, resolve_providers

        self.address = address
        system, service = address.split("/", 1)
        canon, prov = contract.canonical, held.canonical
        params = dict(params or {})
        bad = [e for e in bindings if not ab.entry_valid(e)]
        if bad:
            # §0: an entry is <system>/<service>, each chunk plain or * (core
            # §1.2, D009); one that is not would match nothing, silently
            raise Refused(f"{address}: not a binding entry: {', '.join(map(repr, bad))}")
        decl = ab.declaration(ab.role_of(canon, role), ab.provider_of(prov), params)
        if not decl.valid:
            raise Refused(f"{address}: invalid: {', '.join(decl.reasons)}")
        if "redundancy.v1" in prov.get("uses", []):
            # zk2py's limitation, never the text's: §2.5's outcome 6 needs
            # redundancy.v1 §2.10's incumbent, which this runtime does not follow
            raise Refused(f"{address}: zk2py follows no redundancy.v1 incumbent (§2.5 outcome 6)")
        assert decl.resource is not None and decl.deadline_ms is not None
        self.decl = decl
        self.iface = prov["interface"]
        self.token, self.template = decl.resource.split("/", 1)
        self.params = ab.resolve_params(params, address)
        self.bindings = resolve_providers(list(bindings), system)
        self.emit = emit or (lambda line: print(line, flush=True))
        self.warn = warn or (lambda line: print(line, file=sys.stderr, flush=True))
        self.deliver = deliver
        self.reread_delay_s = reread_delay_s
        #: the round trip's retry step after a move (0.1 s more each attempt,
        #: at most 1 s) and its wait for a link, ten steps
        self.retry_s = 0.1
        #: F-268: a withdrawal heard while the watch session holds no link
        #: begins a move; False is the control, the transport event alone
        self.infer_moves = infer_moves
        self.lock = threading.RLock()
        self.cond = threading.Condition(self.lock)
        self.closed = False
        self.lines: list[dict[str, Any]] = []
        #: every read again made: (instant, address, complete, held)
        self.reads: list[dict[str, Any]] = []
        #: every round trip asked on the watch session: (instant, after move gen, stands, why)
        self.round_trips: list[dict[str, Any]] = []
        self.moves: list[tuple[int, str]] = []
        #: every presence event as the runtime handed it to the arbiter: (ms, what, key)
        self.log: list[tuple[float | None, str, str]] = []
        self.gen = 0
        self._reading: set[str] = set()
        self.arb = ab.Arbiter(self.iface, self.token, self.template, self.params, decl.deadline_ms, self.bindings)
        td = tm.declaration(self.token, next(r for r in prov["resources"]
                                             if f"{r['token']}/{r['template']}" == decl.resource)["annotations"])
        self.lifespan = tm.LifespanReading(td, epsilon_ms) if epsilon_ms is not None and td.lifespan_ms else None
        state = [r for r in canon["resources"] if r["kind"] == "state" and not r["params"]]
        self.angle_key = (f"zk2/{address}/{canon['interface']}/{state[0]['token']}/{state[0]['template']}"
                          if state else None)
        # Everything a close() may undo, so a failure part way leaves nothing
        # behind (review-bot on PR #912).
        self.owner: Any = None
        self.watch: Any = None
        self.reader: Any = None
        self.listener: Any = None
        self.watch_subs: list[Any] = []
        self.descr_subs: list[Any] = []
        self.subs: list[Any] = []
        self._timer: threading.Thread | None = None
        self.t0 = time.monotonic_ns()
        self.selectors = [f"zk2/{e}/@zk/alive/{self.iface}/**" for e in self.bindings]
        try:
            # the service: §8.2's start, the descriptor listing the binding in
            # its order (core R3)
            self.owner = Owner(system, service, [contract], connect=connect, bindings={role: list(bindings)},
                               params={role: params} if params else None)
            self.owner.start()
            self.t0 = time.monotonic_ns()
            with self.lock:
                self._handle(self.arb.start(self.t0))
            # the watch, on a session of its own, then its round trip (§2.7)
            self.watch = live.open_client(watch_connect or connect)
            for sel in self.selectors:
                self.watch_subs.append(live.declare_liveliness_subscriber(self.watch, sel, self._token,
                                                                          history=False))
            try:
                self.listener = live._info(self.watch).declare_transport_events_listener(
                    zenoh.handlers.Callback(self._moved), history=False)
            except Exception:  # noqa: BLE001 - a binding without transport events: no move is heard
                self.listener = None
            self._round_trip(0)
            # the reader, for its reads again
            self.reader = live.open_client(reader_connect or connect)
            if self.lifespan is not None:
                for e in self.bindings:
                    self.descr_subs.append(self.owner.session.declare_subscriber(
                        f"zk2/{e}/@zk/instance/*", zenoh.handlers.Callback(self._descriptor)))
            # the role's subscription, on the service's session
            for e in self.bindings:
                self.subs.append(self.owner.session.declare_subscriber(
                    _expr(e, self.iface, self.token, self.template, self.params),
                    zenoh.handlers.Callback(self._sample)))
            self._timer = threading.Thread(target=self._timer_loop, daemon=True)
            self._timer.start()
        except BaseException:
            self.close()
            raise

    # -- the events ---------------------------------------------------------------------------

    def _sample(self, sample: Any) -> None:
        import zenoh

        key = str(sample.key_expr)
        payload = sample.payload.to_bytes()
        delete = sample.kind == zenoh.SampleKind.DELETE
        wall = time.time_ns()
        ts = sample.timestamp
        with self.lock:
            if self.closed:
                return
            expired = False
            if self.lifespan is not None and not delete:
                st = None if ts is None else tm.Stamp(wall - ts.get_time_as_ntp64().as_nanos(), str(ts.get_id()))
                expired = self.lifespan.judge(st).verdict == tm.EXPIRED
            d = self.arb.sample(time.monotonic_ns(), key, delete=delete, expired=expired)
            self._handle(d)
            if d.outcome == ab.DELIVERED:
                if self.angle_key is not None:
                    self.owner.set_state(self.angle_key, payload)
                if self.deliver is not None:
                    self.deliver(key, payload)

    def _token(self, sample: Any) -> None:
        import zenoh

        key = str(sample.key_expr)
        with self.lock:
            if self.closed:
                return
            now = time.monotonic_ns()
            kind = "withdrawn" if sample.kind == zenoh.SampleKind.DELETE else "declared"
            if kind == "withdrawn" and self.infer_moves and not self.arb.pending \
                    and live.links_held(self.watch) is False:
                # F-268: zenoh-python 1.10.1 tells the close's withdrawals before
                # the transport event, and the watch session holds no link by
                # then. A withdrawal heard so is the move's: the move begins
                # here, and the event that follows is the same move.
                self.gen += 1
                self.moves.append((now, "close (inferred)"))
                self.log.append((_ms(now, self.t0), "moved (inferred: no link)", ""))
                self._handle(self.arb.moved(now))
            self.log.append((_ms(now, self.t0), kind, key))
            self._handle(self.arb.withdrawn(now, key) if kind == "withdrawn" else self.arb.declared(now, key))

    def _moved(self, event: Any) -> None:
        import zenoh

        kind = "close" if event.kind == zenoh.SampleKind.DELETE else "open"
        with self.lock:
            if self.closed:
                return
            self.gen += 1
            gen = self.gen
            now = time.monotonic_ns()
            self.moves.append((now, kind))
            self.log.append((_ms(now, self.t0), f"moved ({kind})", ""))
            self._handle(self.arb.moved(now))
        if kind == "open":
            threading.Thread(target=self._round_trip, args=(gen,), daemon=True).start()

    def _round_trip(self, gen: int) -> None:
        """The round trip asked after the last move (§2.8; core §8.1, 0.41 to
        0.45): it stands for that move only, and applies what was kept.

        At the start (``gen`` 0) nothing is kept, so it is asked once. After a
        move it is asked until one stands, or a later move asks its own, or
        the actuator closes, never given up (review-bot on PR #912): one that
        does not stand is asked again, waiting 0.1 s more each time up to
        1 s; through a session cut off it waits for a link, checked every
        second, since the next open is a move of its own anyway. While it
        does not stand, the move's events stay kept, and each attempt says
        so in the log and on ``warn``."""
        attempt = 0
        while True:
            if gen > 0 and live.links_held(self.watch) is False:
                with self.lock:
                    if self.closed or gen != self.gen:
                        return
                    self.log.append((_ms(time.monotonic_ns(), self.t0), "pending: cut off, waiting for a link", ""))
                time.sleep(self.retry_s * 10)
                continue
            rt = live.settle(self.watch, 5.0, selector=self.selectors[0] if self.selectors else None)
            with self.lock:
                now = time.monotonic_ns()
                self.round_trips.append({"at_ms": _ms(now, self.t0), "gen": gen, "stands": bool(rt.stands),
                                         "why": rt.why})
                if self.closed or gen != self.gen:
                    return  # a later move asks its own
                if gen == 0:
                    return  # the start's: nothing is kept (§2.8, "At start there is no move")
                if rt.stands:
                    self.log.append((_ms(now, self.t0), "stood", ""))
                    self._handle(self.arb.stood(now))
                    return
                attempt += 1
                why = f"pending: round trip {attempt} after move {gen} did not stand ({rt.why}); " \
                      f"{len(self.arb.kept)} events kept"
                self.log.append((_ms(now, self.t0), why, ""))
            self.warn(why)
            time.sleep(min(self.retry_s * attempt, 10 * self.retry_s))

    def _descriptor(self, sample: Any) -> None:
        try:
            doc = json.loads(sample.payload.to_bytes())
        except ValueError:
            return
        zid = (doc.get("meta") or {}).get("zid") if isinstance(doc, dict) else None
        if isinstance(zid, str) and self.lifespan is not None:
            with self.lock:
                self.lifespan.add_owner(zid)

    def _timer_loop(self) -> None:
        """The timer (§2.9), armed just after the active provider's due. Each
        pass either moves the state or waits (review-bot on PR #912): a tick
        at a due past always moves it, since the active provider is then
        ineligible and the time decides it (``readings`` checks it over every
        state ``arbitrations.json`` reaches), and a tick that left it where it
        was would wait a millisecond, logged once, never spin."""
        stuck: tuple[Any, ...] | None = None
        while True:
            with self.cond:
                if self.closed:
                    return
                due = self.arb.due()
                now = time.monotonic_ns()
                if due is None or due >= now:
                    self.cond.wait(0.5 if due is None else min((due + 1 - now) / NS, 0.5))
                    continue
                before = (self.arb.active, due)
                self._handle(self.arb.tick(now))
                if (self.arb.active, self.arb.due()) == before:
                    if stuck != before:
                        stuck = before
                        self.log.append((_ms(now, self.t0), "timer: a tick left a past due where it was", ""))
                    self.cond.wait(0.001)

    # -- what it decided ------------------------------------------------------------------------

    def _handle(self, d: ab.Decision) -> None:
        """Each switch told as a JSON line, a read again scheduled, the timer
        re-armed (the lock held)."""
        for s in d.switches:
            line = {"switch": s.expect(), "at_ms": _ms(s.at_ns, self.t0) if s.cause != ab.START else 0.0,
                    "last_ms": _ms(s.last_ns, self.t0) if s.from_ is not None else None}
            self.lines.append(line)
            self.emit(json.dumps(line))
        for address in d.reread:
            if address not in self._reading:
                self._reading.add(address)
                t = threading.Timer(self.reread_delay_s, self._read, args=(address,))
                t.daemon = True
                t.start()
        self.cond.notify_all()

    def _read(self, address: str) -> None:
        """§2.7's read again, on the reader session."""
        with self.lock:
            self._reading.discard(address)
            if self.closed:
                return
        p = live.list_presence(self.reader, f"zk2/{address}/@zk/alive/{self.iface}/**", 1.0)
        held = sorted(x["instance"] for x in p.alive
                      if f"{x['system']}/{x['service']}" == address and x["iface"] == self.iface)
        complete = p.kind == live.COMPLETE
        with self.lock:
            if self.closed:
                return
            now = time.monotonic_ns()
            self.reads.append({"at_ms": _ms(now, self.t0), "address": address, "complete": complete, "held": held})
            self._handle(self.arb.read(now, address, complete, held))

    def counts(self) -> dict[str, Any]:
        with self.lock:
            return json.loads(json.dumps(self.arb.counts))

    def close(self) -> None:
        """Undo whatever was built, a half-built actuator's included: the
        subscriptions and the watch first, then the sessions, then the
        service, its tokens first (core §8.2)."""
        with self.cond:
            self.closed = True
            self.cond.notify_all()
        for x in self.subs + self.watch_subs + self.descr_subs + ([self.listener] if self.listener else []):
            try:
                x.undeclare()
            except Exception:  # noqa: BLE001 - closing anyway
                pass
        for sess in (self.watch, self.reader):
            if sess is not None:
                live.close(sess)
        if self.owner is not None:
            try:
                self.owner.close()
            except Exception:  # noqa: BLE001
                pass


class Commander:
    """An owner of the commanders' stream at ``address``, putting ``key``
    every ``period_s`` with the payload ``<service> <n>``, ``n`` from 0 at its
    start; it **holds** (stops putting, its tokens and session kept) and
    **resumes**, and re-mints make-before-break (core §8.1)."""

    def __init__(self, address: str, contract: Any, connect: str, period_s: float = 0.05) -> None:
        from .owner import Owner

        system, self.service = address.split("/", 1)
        c = contract.canonical
        r = next(x for x in c["resources"] if x["kind"] == "stream")
        self.key = f"zk2/{address}/{c['interface']}/{r['token']}/{r['template']}"
        self.owner = Owner(system, self.service, [contract], connect=connect)
        self.period_s = period_s
        self.lock = threading.Lock()
        self.n = 0
        self.held = threading.Event()
        self.stop = threading.Event()
        self.thread: threading.Thread | None = None

    def start(self) -> str:
        self.owner.start()
        self.thread = threading.Thread(target=self._run, daemon=True)
        self.thread.start()
        return self.owner.instance

    def _run(self) -> None:
        t0 = time.monotonic()
        i = 0
        while not self.stop.is_set():
            due = t0 + i * self.period_s
            d = due - time.monotonic()
            if d > 0:
                time.sleep(d)
            i += 1
            if self.held.is_set() or self.stop.is_set():
                continue
            with self.lock:
                n = self.n
                self.n += 1
            try:
                self.owner.publish(self.key, f"{self.service} {n}".encode())
            except Exception:  # noqa: BLE001 - closed while putting
                return

    def count(self) -> int:
        """The puts made so far."""
        with self.lock:
            return self.n

    def hold(self) -> None:
        self.held.set()

    def resume(self) -> None:
        self.held.clear()

    def remint(self) -> tuple[str, str]:
        old = self.owner.new_epoch()
        return old, self.owner.instance

    def close(self) -> None:
        self.stop.set()
        if self.thread is not None:
            self.thread.join(timeout=1.0)
        self.owner.close()


class ToolView:
    """A tool's own view of one consumer's role (§2.14): the role's binding
    and parameter bindings from the consumer's descriptor, the deadline from
    the declaration, and §2.4 to §2.11 run over the tool's own subscription,
    watch and monotonic clock. Its selection is labelled its own, never the
    consumer's."""

    def __init__(self, session: Any, watch: Any, interface: str, decl: ab.Declaration, bindings: list[str],
                 params: dict[str, str]) -> None:
        import zenoh

        from .budget import StallWatch

        assert decl.resource is not None and decl.deadline_ms is not None
        self.token, self.template = decl.resource.split("/", 1)
        self.deadline_ns = decl.deadline_ms * MS
        self.lock = threading.Lock()
        self.arb = ab.Arbiter(interface, self.token, self.template, params, decl.deadline_ms, bindings)
        self.stall = StallWatch(session)
        self.t0 = time.monotonic_ns()
        self.arb.start(self.t0)
        self.watch_subs = [live.declare_liveliness_subscriber(
            watch, f"zk2/{e}/@zk/alive/{interface}/**", self._token, history=False) for e in bindings]
        if bindings:
            live.settle(watch, 5.0, selector=f"zk2/{bindings[0]}/@zk/alive/{interface}/**")
        self.subs = [session.declare_subscriber(_expr(e, interface, self.token, self.template, params),
                                                zenoh.handlers.Callback(self._sample)) for e in bindings]

    def _sample(self, sample: Any) -> None:
        import zenoh

        with self.lock:
            self.arb.sample(time.monotonic_ns(), str(sample.key_expr), delete=sample.kind == zenoh.SampleKind.DELETE)

    def _token(self, sample: Any) -> None:
        import zenoh

        with self.lock:
            now = time.monotonic_ns()
            if sample.kind == zenoh.SampleKind.DELETE:
                self.arb.withdrawn(now, str(sample.key_expr))
            else:
                self.arb.declared(now, str(sample.key_expr))

    def view(self) -> ab.View:
        """Now: how long it listened, whether its own path stalled within the
        last deadline (core §2.7's watch), and what its view selects."""
        now = time.monotonic_ns()
        with self.lock:
            self.arb.tick(now)
            active = self.arb.active
        lost = any(t >= now - self.deadline_ns for t in self.stall.losses())
        return ab.View(now - self.t0, lost, active)

    def close(self) -> None:
        for s in self.subs + self.watch_subs:
            try:
                s.undeclare()
            except Exception:  # noqa: BLE001 - closing anyway
                pass
        self.stall.close()


def emit_stdout(line: str) -> None:
    sys.stdout.write(line + "\n")
    sys.stdout.flush()


__all__ = ["Actuator", "Commander", "ToolView", "Refused", "emit_stdout"]

"""``redundancy.v1`` on a bus (#609, round 56): a candidate of a governed
interface driving :class:`zk2py.redundancy.Machine`, a consumer following
the incumbent (§2.10), and a tool's reading of an address (§2.15).

**The candidate** (§2.1 to §2.11) is an :class:`zk2py.owner.Owner` that
starts with the interface marked standby, and a worker thread that hands
the machine its events one at a time and does each step:
- ``claim``: the owner's start, core §8.2 steps 1 to 4 with nothing of the
  interface declared and its entry marked (§2.5 step 1), then the watch: a
  liveliness subscriber on ``zk2/<address>/@zk/**`` on a session of its own
  (§2.7, core §8.1 0.37), with history, and its round trip;
- ``read``: one liveliness read of ``zk2/<address>/@zk/**`` on a session of
  its own that never holds a liveliness subscriber (core §8.1, 0.38) and
  never holds the claim, after one round trip on the claim's session
  (§2.5 step 1), and the descriptors of the instances below its own; a read
  not made is ``Read(False, ())``. Not the claim's session: zenoh answers a
  session's own tokens locally, so a read there shows the claim even when
  access control refuses it, and §2.5 step 3's own-claim check could never
  see the refusal (F-204);
- ``grace`` and a ``stand_by`` with retry arm one timer, a newer one
  replacing it;
- ``take_over`` and ``step_down``: :meth:`Owner.take_over` and
  :meth:`Owner.step_down`, core §8.2's order and its reverse (0.39).

Its watch hands the machine ``appeared`` for another instance's interface
token, and ``withdrawn`` 0.1 s after a withdrawal that leaves no holder
heard (§2.8), one for a burst. Each event reaches the machine in the order
it was heard, and what is heard while a read or an act runs is posted
before that read's answer or that act's finish, so the machine keeps it
and decides it there (text 0.2, F-199). A re-mint is told the machine
before the new id's tokens are declared (F-209).

Under ``external`` (§2.11) the lock is a ``flock`` on a file, tried every
20 ms by a thread of its own until held, which the kernel releases at the
process's end; or, with no file, the lock the commands hold and lose.
"""

from __future__ import annotations

import fcntl
import json
import os
import queue
import secrets
import sys
import threading
import time
from pathlib import Path
from typing import Any

import zenoh

from . import live, redundancy as rd
from .contract import load_contract
from .owner import Owner

REPO = Path(__file__).resolve().parents[3]
CONTRACTS = REPO / "impl" / "python" / "interop" / "redundancy"


def contract_at(path: str | Path) -> Any:
    p = Path(path)
    if not p.is_file():
        p = CONTRACTS / f"{path}.toml"
    return load_contract(p, spec_dir=REPO / "spec")


def open_session(endpoint: str, peer: bool = False, lease_ms: int | None = None) -> zenoh.Session:
    """A client of ``endpoint``, or a peer whose only endpoint it is (one
    that opens while that router is down: scenarios.md §5). ``lease_ms`` is
    its lease, as its router reads it."""
    if not peer and lease_ms is None:
        return live.open_client(endpoint)
    conf = zenoh.Config()
    if lease_ms is not None:
        conf.insert_json5("transport/link/tx/lease", str(int(lease_ms)))
    if not peer:
        conf.insert_json5("mode", json.dumps("client"))
        conf.insert_json5("connect/endpoints", json.dumps([endpoint]))
        conf.insert_json5("scouting/multicast/enabled", "false")
        s = zenoh.open(conf)
        live._OPENED[id(s)] = (s, endpoint, None)
        return s
    conf.insert_json5("mode", json.dumps("peer"))
    conf.insert_json5("connect/endpoints", json.dumps([endpoint]))
    # a peer listens, so that its fellow peers, its own other sessions among
    # them, can link to it: zenoh routes no peer's tokens to another peer
    # through a router, and a peer that cannot be reached reads them never,
    # each read complete (F-206)
    conf.insert_json5("listen/endpoints", json.dumps(["tcp/127.0.0.1:0"]))
    conf.insert_json5("scouting/multicast/enabled", "false")
    conf.insert_json5("timestamping/enabled", "true")
    # zenoh retries a peer's connect at a period that grows to 4 s by
    # default (measured: served 4.5 s after R1's start with it, 0.5 s
    # without); scenarios.md §5's "the retry interval plus 2 s of R1's
    # start" leaves no room for it, so the period stays at 0.5 s (F-205)
    conf.insert_json5("connect/retry", json.dumps({"period_init_ms": 500, "period_max_ms": 500,
                                                   "period_increase_factor": 1.0}))
    return zenoh.open(conf)


# -- reading an address -----------------------------------------------------------


def descriptor_reading(session: zenoh.Session, address: str, instance: str, iface: str,
                       timeout: float = live.GET_TIMEOUT_S) -> tuple[str, dict[str, Any] | None]:
    """An instance's descriptor as a read shows it (``elections.json``):
    ``unread`` (no single reply, or one that does not read, or an entry
    malformed), ``not_listed``, ``standby`` or ``serving``; and the
    document."""
    got = [a for a in live.get_descriptor(session, f"zk2/{address}/@zk/instance/{instance}", timeout) if a.ok]
    if len(got) != 1:
        return rd.UNREAD, None
    try:
        doc = json.loads(got[0].payload)
    except ValueError:
        return rd.UNREAD, None
    if not isinstance(doc, dict):
        return rd.UNREAD, None
    for e in doc.get("interfaces") or []:
        if isinstance(e, dict) and e.get("iface") == iface:
            m, _ = rd.mark(e, "no_contract")
            return (rd.UNREAD if m is None else m), doc
    return rd.NOT_LISTED, doc


def read_address(session: zenoh.Session, address: str, iface: str, *, below: str | None = None,
                 own: tuple[str, str] | None = None,
                 timeout: float = live.PRESENCE_TIMEOUT_S) -> tuple[rd.Read, live.Presence, dict[str, Any]]:
    """§0's read: one liveliness read of ``zk2/<address>/@zk/**``, its
    instance and interface tokens at once, with the descriptors it needs:
    every instance's, or with ``below`` only those of the ids lower than
    it (§2.5 step 3: "A higher id cannot beat it"). ``own`` is (id, its
    mark): its own descriptor is not read ("own's own descriptor reading
    plays no part"). Complete is core §8.1's (0.37, 0.38): a read not made
    is ``Read(False, ())``. Returns the read, the presence read, and the
    descriptors read by id."""
    p = live.list_presence(session, f"zk2/{address}/@zk/**", timeout)
    if p.kind == live.NOT_MADE:
        return rd.Read(False, ()), p, {}
    system, service = address.split("/")
    mine = [i for i in p.instances if i["system"] == system and i["service"] == service]
    holders = {a["instance"] for a in p.alive
               if a["system"] == system and a["service"] == service and a["iface"] == iface}
    ids = {i["instance"] for i in mine} | holders
    seen, docs = [], {}
    for i in sorted(ids, key=rd.id_key):
        if own is not None and i == own[0]:
            desc = own[1]
        elif below is None or rd.id_key(i) < rd.id_key(below):
            desc, doc = descriptor_reading(session, address, i, iface)
            if doc is not None:
                docs[i] = doc
        else:
            desc = rd.UNREAD
        seen.append(rd.Seen(i, i in holders, desc))
    return rd.Read(p.kind == live.COMPLETE, tuple(seen)), p, docs


def governance(session: zenoh.Session, iface: str, docs: list[dict[str, Any]],
               offline: dict[str, dict[str, Any]] | None = None) -> str:
    """§2.15's third premise: the governance "read from the contract each
    candidate's descriptor names (§2.2)": ``claim``/``external``,
    ``not_governed``, ``no_contract`` (one not read), or ``disagree``.
    ``offline`` holds canonical contracts by fingerprint, a revision held
    offline never retrieved (core §8.4); any other is retrieved."""
    seen: set[str] = set()
    for doc in docs:
        for e in doc.get("interfaces") or []:
            if not isinstance(e, dict) or e.get("iface") != iface:
                continue
            fp = str(e.get("contract"))
            canonical = (offline or {}).get(fp)
            if canonical is None:
                r = live.retrieve_bundle(session, iface, fp)
                if not r.verified:
                    return "no_contract"
                canonical = r.verified.contract
            d = rd.declaration_of(canonical)
            seen.add(d.election if d.governed else "not_governed" if d.declaration == "not_governed"
                     else "invalid")
    if not seen:
        return "no_contract"
    if len(seen) > 1 or "invalid" in seen:
        return "disagree"
    return seen.pop()


def tool_reading(session: zenoh.Session, address: str, iface: str, grace_s: float = 2.0,
                 across_face: bool = False, contracts: list[Any] = ()) -> tuple[rd.Reading, list[live.Presence]]:
    """§2.15: two reads a grace apart, each its tokens, its completeness and
    every descriptor in it; the governance from the contracts the
    descriptors name. The presence reads ride along, so a tool reports a
    read's own kind beside the profile's answer (core §8.1, 0.38)."""
    r1, p1, d1 = read_address(session, address, iface)
    time.sleep(grace_s)
    r2, p2, d2 = read_address(session, address, iface)
    gov = governance(session, iface, list({**d1, **d2}.values()),
                     {c.fingerprint: c.canonical for c in contracts})
    return rd.Reading(gov, r1, r2, across_face), [p1, p2]


# -- the candidate -------------------------------------------------------------------


class Refused(RuntimeError):
    """The instance refused to start: no instance token appeared."""


class Candidate:
    """One instance of a governed interface at ``address`` (§2.1)."""

    def __init__(self, address: str, endpoint: str, contract: str | Path, *, instance: str | None = None,
                 name: str = "A", retry_s: float = 2.0, grace_s: float = 2.0, lock: str | None = None,
                 refuse_without_lock: bool = False, tokenless: tuple[str, ...] = (), peer: bool = False,
                 timeout: float = 1.0, hold_heal: bool = False, auth: tuple[str, str] | None = None,
                 poll_s: float | None = None, lease_ms: int | None = None,
                 reader_endpoint: str | None = None) -> None:
        self.address, self.endpoint, self.name = address, endpoint, name
        self.system, self.service = address.split("/")
        self.contract = contract_at(contract)
        self.iface = self.contract.interface
        self.tokenless = set(tokenless)
        decl = rd.declaration_of(self.contract.canonical, tokenless=self.iface in self.tokenless)
        self.declaration = decl
        self.election = decl.election or rd.CLAIM
        self.instance = instance or secrets.token_hex(8)
        self.machine = rd.Machine(self.instance, self.election)
        self.retry_s, self.grace_s, self.timeout = retry_s, grace_s, timeout
        self.lock_path, self.refuse_without_lock = lock, refuse_without_lock
        self.peer, self.auth = peer, auth
        #: §2.7 (0.2): "A poll, where no watch session can be held": the
        #: address read every period on the reader instead of a watch
        self.poll_s, self.lease_ms = poll_s, lease_ms
        #: a test's hook: the reader through an endpoint of its own, so that a
        #: test can cut it apart from the watch (a reader that reconnects late)
        self.reader_endpoint = reader_endpoint
        #: what the owner example's "redundancy reads" counts
        self.counts = {"made": 0, "complete": 0, "not_made": 0, "polls": 0, "polls_complete": 0, "last": None,
                       "reader_reads": 0, "reader_settles": 0, "watch_reads": 0, "watch_settles": 0}
        #: under external with no lock file, the lock as the commands set it
        self.held_lock = False
        self.events: queue.Queue[Any] = queue.Queue()
        self.lock = threading.RLock()
        #: (monotonic s, event, step)
        self.log: list[tuple[float, Any, dict[str, Any]]] = []
        #: every read made: (monotonic s, Read, presence kind, reason)
        self.reads: list[tuple[float, rd.Read, str, str]] = []
        self.executions = 0
        #: a failed act, after which it elects no more ("Decided here", 0.2)
        self.fault: str | None = None
        self.owner: Owner | None = None
        self.watch: zenoh.Session | None = None
        #: the session its reads go on: neither the claim's nor the watch's
        self.reader: zenoh.Session | None = None
        self._entities: list[Any] = []
        #: the interface's holders the watch hears now, by instance
        self.heard: dict[str, set[str]] = {}
        self._withdrawal_due = False
        self._timer_gen = 0
        self._stop = threading.Event()
        self._fd: int | None = None
        self._lock_thread: threading.Thread | None = None
        self._hold_heal = threading.Event()
        if not hold_heal:
            self._hold_heal.set()
        self._worker: threading.Thread | None = None

    # -- bring-up ------------------------------------------------------------
    def start(self) -> str:
        """§2.11: under external, the lock first; "Without the lock, it either
        refuses to start … or stands by". Then the claim, through the
        machine. Raises :class:`Refused` before any session opens for a
        refusal of step 2's or the lock's."""
        self.machine_lock = False
        if self.election == rd.EXTERNAL and self.lock_path is not None:
            self._fd = os.open(self.lock_path, os.O_RDWR | os.O_CREAT, 0o644)
            try:
                fcntl.flock(self._fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
                self.machine_lock = True
            except BlockingIOError:
                self.machine_lock = False
                if self.refuse_without_lock:
                    os.close(self._fd)
                    self._fd = None
                    raise Refused(f"{self.lock_path}: the lock is held, and this instance refuses (§2.11)")
        handlers = {"@op/append": self._append}
        self.owner = Owner(self.system, self.service, [self.contract], connect=self.endpoint,
                           standby={self.iface}, handlers=handlers, tokenless=self.tokenless, auth=self.auth,
                           lease_ms=self.lease_ms)
        self.owner.instance = self.instance
        try:
            self.owner._plan()  # step 2's refusals (core §8.2, 0.39): nothing is opened
        except Exception as e:
            raise Refused(str(e)) from e
        self._worker = threading.Thread(target=self._run, daemon=True)
        self._worker.start()
        if self._fd is not None and self.machine_lock:
            self.post({"lock": True})
        elif self._fd is not None:
            self._lock_thread = threading.Thread(target=self._take_lock, daemon=True)
            self._lock_thread.start()
        self.post(rd.START)
        return self.instance

    def _take_lock(self) -> None:
        """§10: "each standing by while it cannot take the lock"; the kernel
        releases a dead holder's. A non-blocking try every 20 ms until it
        holds it or the candidate closes: :meth:`close` closes the fd only
        once this thread is gone, so it never locks a reused fd."""
        fd = self._fd
        assert fd is not None
        while not self._stop.is_set():
            try:
                fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
            except BlockingIOError:
                self._stop.wait(0.02)
                continue
            except OSError:
                return
            self.post({"lock": True})
            return

    def unlock(self) -> None:
        """The lock lost (a test's hand on it): §2.11 "It steps down … when it
        loses the lock"."""
        if self._fd is not None:
            fcntl.flock(self._fd, fcntl.LOCK_UN)
        self.held_lock = False
        self.post({"lock": False})

    def relock(self) -> bool:
        """The lock asked again: a lock file taken when free, else the
        commands' lock held (the owner example's ``redundancy lock hold``)."""
        if self._fd is not None:
            try:
                fcntl.flock(self._fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
            except BlockingIOError:
                return False
        self.held_lock = True
        self.post({"lock": True})
        return True

    def post(self, event: Any) -> None:
        self.events.put(event)

    # -- the worker -------------------------------------------------------------
    def _run(self) -> None:
        while not self._stop.is_set():
            try:
                ev = self.events.get(timeout=0.2)
            except queue.Empty:
                continue
            if self._stop.is_set():
                return
            with self.lock:
                step = self.machine.step(ev)
                self.log.append((time.monotonic(), ev if not isinstance(ev, dict) or "read" not in ev
                                 else "read", step))
            try:
                self._do(step)
            except Exception as e:  # noqa: BLE001 - a failed act is logged, the worker goes on
                print(f"zk2py redundancy: {step['step']} failed: {type(e).__name__}: {e}", file=sys.stderr,
                      flush=True)
                if step["step"] in ("take_over", "step_down"):
                    # "Decided here" (text 0.2): undone, the instance stands
                    # by and elects no more, its standing saying why; a retry
                    # would ask the same session again, and flap
                    with self.lock:
                        self.fault = f"{step['step']} failed: {type(e).__name__}: {e}"
                    return

    def _do(self, step: dict[str, Any]) -> None:
        k = step["step"]
        if k == "claim":
            self._claim()
            self.post(rd.FINISH)
        elif k == "read":
            self.post({"read": self._read()})
        elif k == "grace":
            self._arm(self.grace_s, heal=True)
        elif k == "retry":
            # text 0.2 (F-199): a heal read that ended nothing, made again
            self._arm(self.retry_s, heal=True)
        elif k == "stand_by":
            if step["retry"]:
                self._arm(self.retry_s)
        elif k == "take_over":
            assert self.owner is not None
            self.owner.take_over(self.iface, puts=self._puts)
            self.post(rd.FINISH)
        elif k == "step_down":
            assert self.owner is not None
            self.owner.step_down(self.iface)
            self.post(rd.FINISH)

    def _arm(self, seconds: float, heal: bool = False) -> None:
        with self.lock:
            self._timer_gen += 1
            gen = self._timer_gen

        def fire() -> None:
            if heal:
                self._hold_heal.wait()
            with self.lock:
                if gen != self._timer_gen:
                    return
            self.post(rd.TIMER)

        t = threading.Timer(seconds, fire)
        t.daemon = True
        t.start()

    # -- the claim and the watch (§2.5 step 1, §2.7) -------------------------------
    def _claim(self) -> None:
        assert self.owner is not None
        if self.peer:
            _start_peer(self.owner, self.lease_ms)
        else:
            self.owner.start()
        if self.poll_s is None:
            self.watch = open_session(self.endpoint, self.peer, self.lease_ms)
            self._entities.append(live.declare_liveliness_subscriber(
                self.watch, f"zk2/{self.address}/@zk/**", self._heard, history=True))
            live.settle(self.watch, self.timeout)
            self.counts["watch_settles"] += 1
        self.reader = open_session(self.reader_endpoint or self.endpoint, self.peer, self.lease_ms)
        # §2.5 step 1: "When that is not the session holding the claim, the
        # instance makes one round trip on the claim's session after
        # declaring it, and before the read"
        assert self.owner.session is not None
        live.settle(self.owner.session, self.timeout)
        if self.poll_s is not None:
            self._poll_last: set[str] | None = None
            self._poll_once()  # the first poll, before its first read: "from before its first read"
            threading.Thread(target=self._poll_loop, daemon=True).start()

    def _poll_loop(self) -> None:
        assert self.poll_s is not None
        while not self._stop.wait(self.poll_s):
            try:
                self._poll_once()
            except Exception:  # noqa: BLE001 - a poll that fails is the next one's to make
                pass

    def _poll_once(self) -> None:
        """§2.7 (0.2): "each read's holders compared with the last's: a holder
        new to it is an appearance, and a complete read with no other holder,
        after one with some, a withdrawal (a read not complete establishes no
        absence)". It misses what comes and goes within one period."""
        assert self.reader is not None
        p = live.list_presence(self.reader, f"zk2/{self.address}/@zk/**", self.timeout)
        with self.lock:
            self.counts["polls"] += 1
            self.counts["polls_complete"] += p.kind == live.COMPLETE
            self.counts["reader_reads"] += 1
        holders = {a["instance"] for a in p.alive if a["system"] == self.system and a["service"] == self.service
                   and a["iface"] == self.iface}
        others = {h for h in holders if not self.machine.mine(h)}
        with self.lock:
            self.heard = {h: {h} for h in holders}
        events, self._poll_last = rd.poll_step(self._poll_last, others, p.kind == live.COMPLETE)
        for ev in events:
            self.post(ev)

    def _heard(self, sample: Any) -> None:
        parsed = live.keys.parse(str(sample.key_expr))
        if not parsed or parsed.get("form") != "alive" or parsed.get("iface") != self.iface:
            return
        inst, key = parsed["instance"], str(sample.key_expr)
        if sample.kind == zenoh.SampleKind.PUT:
            with self.lock:
                self.heard.setdefault(inst, set()).add(key)
            if inst != self.instance:
                self.post({"appeared": inst})
            return
        with self.lock:
            keys = self.heard.get(inst, set())
            keys.discard(key)
            if not keys:
                self.heard.pop(inst, None)
            if self.heard or self._withdrawal_due:
                return
            self._withdrawal_due = True

        def due() -> None:
            with self.lock:
                self._withdrawal_due = False
                if self.heard:
                    return
            self.post(rd.WITHDRAWN)

        # §2.8: "one read for a burst (the reference: 0.1 s after the first)"
        t = threading.Timer(0.1, due)
        t.daemon = True
        t.start()

    def _others_heard(self) -> set[str]:
        with self.lock:
            return {i for i in self.heard if i != self.instance and i not in self.machine.former}

    def _read(self) -> rd.Read:
        assert self.owner is not None and self.reader is not None
        own = (self.instance, rd.STANDBY if self.iface in self.owner.standing_by else rd.SERVING)
        r, p, _ = read_address(self.reader, self.address, self.iface, below=self.instance, own=own,
                               timeout=self.timeout)
        with self.lock:
            # as the owner example counts them: "made" the reads made, a read
            # not made counted apart
            k = {live.COMPLETE: "complete", live.NOT_MADE: "not_made"}.get(p.kind, "incomplete")
            if k == "not_made":
                self.counts["not_made"] += 1
            else:
                self.counts["made"] += 1
                self.counts["reader_reads"] += 1
                if k == "complete":
                    self.counts["complete"] += 1
            self.counts["last"] = k
        reason = p.not_made or ("; ".join(p.errors) if p.errors else "") or \
            (live.CUT_OFF_DURING if p.cut_off_during else "")
        self.reads.append((time.monotonic(), r, p.kind, reason))
        return r

    # -- the application (scenarios.md, "The instances") ----------------------------
    def _append(self, call: Any) -> None:
        with self.lock:
            self.executions += 1
        call.reply(self.name.encode())

    def _puts(self, owner: Owner) -> None:
        """"puts balance with its own name as the value when it takes over"
        (scenarios.md; a valve's position the same)."""
        rid = "balance" if self.iface.startswith("ledger.") else "position"
        owner.set_state(f"zk2/{self.address}/{self.iface}/state/{rid}", self.name.encode())

    def put(self, rid: str, value: str) -> str:
        assert self.owner is not None
        return str(self.owner.set_state(f"zk2/{self.address}/{self.iface}/state/{rid}", value.encode()))

    def new_epoch(self, new: str | None = None) -> str:
        """A re-mint in its own session (core §8.1), make-before-break: its
        machine told first (F-209), so that its new id's token is its own,
        never another holder's appearance; then the owner's re-mint, the new
        descriptor and tokens before the old ones go (§2.8: "A re-mint is no
        vacancy"). Returns the new id."""
        assert self.owner is not None
        new = new or secrets.token_hex(8)
        with self.lock:
            # text 0.2 (F-209): "A runtime tells its election of the new id
            # before it declares the new id's tokens, in every state"
            self.log.append((time.monotonic(), "reminted", self.machine.step({"reminted": new})))
            self.instance = new
        self.owner.new_epoch(new)
        return new

    def release_heal(self) -> None:
        self._hold_heal.set()

    @property
    def role(self) -> str:
        with self.lock:
            return self.machine.role

    def close(self) -> None:
        """A clean stop (§2.8): the owner's close undeclares its tokens
        first, then everything else; it puts and deletes nothing. The worker
        and the lock's thread are joined first, so no step in flight runs
        against an owner or a session being closed."""
        self._stop.set()
        self._hold_heal.set()
        for t in (self._worker, self._lock_thread):
            if t is not None and t is not threading.current_thread():
                t.join(timeout=10.0)
        for e in self._entities:
            try:
                e.undeclare()
            except Exception:  # noqa: BLE001
                pass
        if self.owner is not None:
            self.owner.close()
        for x in (self.watch, self.reader):
            if x is not None:
                live.close(x)
        if self._fd is not None:
            try:
                os.close(self._fd)
            except OSError:
                pass


def _start_peer(owner: Owner, lease_ms: int | None = None) -> None:
    """:meth:`Owner.start` as a peer whose only endpoint is its router's
    (scenarios.md §5 step 1: it opens while that router is down)."""
    owner.resolve_system()
    owner.resolve_bindings()
    plan = owner._plan()
    owner.session = open_session(owner.endpoint, peer=True, lease_ms=lease_ms)
    try:
        owner._bring_up(plan)
    except Exception:
        owner.close()
        raise


# -- §2.10 a consumer following the incumbent ------------------------------------------


class Follower:
    """§2.10: "accept its state, stream and event samples only from the
    incumbent: a sample whose stamp's id is the incumbent's meta.zid,
    compared by value"; the incumbent learnt "as a candidate does: a watch
    on the provider's @zk/**, a complete read, and the holder's
    descriptor". During a split it follows the lowest holder it sees; with
    no incumbent seen it accepts nothing new."""

    def __init__(self, endpoint: str, address: str, iface: str, rid: str, timeout: float = 1.0) -> None:
        self.address, self.iface, self.timeout = address, iface, timeout
        self.key = f"zk2/{address}/{iface}/state/{rid}"
        self.session = live.open_client(endpoint)
        self.watch = open_session(endpoint)
        self.lock = threading.RLock()
        self.holders: dict[str, str | None] = {}  # instance -> meta.zid
        self.value: bytes | None = None
        self.accepted: list[tuple[float, bytes, str | None]] = []
        self.discarded: list[tuple[float, bytes, str | None]] = []
        self.seed_result: str | None = None
        self._entities = [live.declare_liveliness_subscriber(
            self.watch, f"zk2/{address}/@zk/**", self._token, history=True)]
        live.settle(self.watch, timeout)
        self._entities.append(self.session.declare_subscriber(self.key, zenoh.handlers.Callback(self._sample)))
        self.reread()

    def _zid_of(self, inst: str) -> str | None:
        _, doc = descriptor_reading(self.session, self.address, inst, self.iface)
        z = (doc or {}).get("meta", {}).get("zid") if isinstance(doc, dict) else None
        return z if isinstance(z, str) else None

    def reread(self) -> None:
        """A complete read of the provider, its holders' descriptors read."""
        r, p, docs = read_address(self.session, self.address, self.iface, timeout=self.timeout)
        if not r.complete:
            return
        hs = {}
        for i in sorted(r.holders(), key=rd.id_key):
            z = (docs.get(i) or {}).get("meta", {}).get("zid")
            hs[i] = z if isinstance(z, str) else None
        with self.lock:
            self.holders = hs

    def _token(self, sample: Any) -> None:
        parsed = live.keys.parse(str(sample.key_expr))
        if not parsed or parsed.get("form") != "alive" or parsed.get("iface") != self.iface:
            return
        inst = parsed["instance"]
        if sample.kind == zenoh.SampleKind.PUT:
            z = self._zid_of(inst)
            with self.lock:
                self.holders[inst] = z
        else:
            with self.lock:
                self.holders.pop(inst, None)

    def incumbent_zid(self) -> str | None:
        """The lowest holder seen, the one the heal keeps (§2.10)."""
        with self.lock:
            low = rd.lowest(self.holders)
            return None if low is None else self.holders.get(low)

    def _accepts(self, stamp_id: str | None) -> bool:
        z = self.incumbent_zid()
        return stamp_id is not None and z is not None and live._zid_value(stamp_id) == live._zid_value(z)

    def _sample(self, sample: Any) -> None:
        ts = sample.timestamp
        sid = None if ts is None else str(ts).split("/", 1)[1]
        data = sample.payload.to_bytes()
        with self.lock:
            if self._accepts(sid):
                self.value = data
                self.accepted.append((time.monotonic(), data, sid))
            else:
                self.discarded.append((time.monotonic(), data, sid))

    def seed(self) -> str:
        """Core §4.2's seed: S4's GET; "A consumer following the lower discards
        a reply of the higher's, and holds no value for that key until the
        incumbent it follows puts it again"."""
        st = live.get_state(self.session, self.key, self.timeout)
        for r in st.replies:
            if r.key != self.key or r.deleted:
                continue
            sid = None if r.stamp is None else r.stamp.split("/", 1)[1]
            with self.lock:
                if self._accepts(sid):
                    self.value = r.payload
                    self.seed_result = "accepted"
                else:
                    self.seed_result = "discarded"
                    self.discarded.append((time.monotonic(), r.payload, sid))
            return self.seed_result
        self.seed_result = "silent"
        return "silent"

    def close(self) -> None:
        for e in self._entities:
            try:
                e.undeclare()
            except Exception:  # noqa: BLE001
                pass
        live.close(self.session)
        live.close(self.watch)

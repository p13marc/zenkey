"""``timing.v1`` on a bus (profile text 0.5, core 0.55; #609, rounds 76 and 78 to 81).

- :class:`Reader` is a consumer of a timed resource (§2.1, §2.5, §2.6) for
  one provider address: a :class:`zk2py.timing.Tracker` per resource with a
  deadline, judged at every arrival and at instants it sets itself (a
  one-shot wait at the earliest instant a miss is due, §2.5), fed the
  owner's token as :class:`zk2py.presence.PresenceFollower` observes it (a
  liveliness subscriber on a watch session, reads on a session that never
  held one, a read 100 ms after a withdrawal, core §8.1, 0.37), each
  member token of an epoch template seen (heard declared or withdrawn, or
  shown by a read: text 0.2, F-261), and each member's end at a complete
  read that shows no member token of it once one was seen; no tracker at
  all where the declaration judges no deadline (F-259); a
  :class:`zk2py.timing.LifespanReading` per resource, on the ε it is given,
  against the ``meta.zid`` of every descriptor of the address it holds. It
  subscribes to the address's instance keys, so it holds a new instance's
  descriptor before its first sample: core §3.3 puts the first descriptor
  in §8.2 step 3, before any token. An expired sample is discarded, never
  handed on and no arrival; every other sample is handed on (§2.6).
- :class:`Window` is a tool's window over it (§2.7): presence followed and
  the descriptors held first, then the subscriptions (the window's start),
  the trackers evaluated at its end, and §5's three questions per member,
  each member's held time its tracker's longest stretch and its span ended
  at its first end (text 0.5, F-278, F-280).
  It holds no binding, so only a member of a template without parameters
  is known to it (text 0.3); a loss it cannot attribute, a stall of its
  receive path, is every member's (text 0.3); across a constrained face it
  follows no presence, reads no descriptor, and holds an owner's
  ``meta.zid`` only on its deployment's word (§2.8, text 0.3).
- :func:`stamped_clock` is an owner's stamping clock offset from its host's
  wall clock (the scenarios' ``simulate_offset``).
"""

from __future__ import annotations

import json
import threading
import time
from dataclasses import dataclass, field
from typing import Any, Callable

from . import timing as tm
from .presence import DOWN, UP, PresenceFollower

NS = 1_000_000_000


def _template_expr(template: str) -> str:
    from .owner import _template_key

    return _template_key(template)


@dataclass
class Arrival:
    """One sample as the reader took it: its key, its arrival on the
    monotonic clock and the wall clock then, its stamp, its lifespan's
    judgement, and whether it was handed on."""

    rid: str
    key: str
    mono_ns: int
    wall_ns: int
    stamp_ns: int | None
    stamp_id: str | None
    verdict: tm.Verdict
    handed: bool
    payload: bytes


@dataclass
class _Resource:
    rid: str
    r: dict[str, Any]
    decl: tm.Declaration
    expr: str
    known: list[str]
    tracker: tm.Tracker | None
    lifespan: tm.LifespanReading
    epoch: str | None
    #: the instant each member first began to be held: its first arrival, or
    #: the start of its wait (a known member), for a tool's ``held``
    first_held: dict[str, int] = field(default_factory=dict)


class Reader:
    """A consumer of ``rids`` (``<kind token>/<template>``) of ``iface`` at
    ``address`` (``<system>/<service>``), its role bound to it (core R1).

    ``params`` binds template parameters (core R2): a template whose every
    parameter is bound has its member known. ``epsilon_ms`` is the
    deployment's ε, None for none stated. ``on_value`` is the application:
    called with (key, payload) for each sample handed on. ``on_report`` is
    called with (rid, :class:`zk2py.timing.Report`) for each deadline
    report. With ``seed_first`` the subscriptions are declared only once the
    follower's first read is folded and every instance it shows has its
    descriptor held (a tool's window, §2.7). With ``subscribe_first`` they
    are declared before the follower exists, as scenarios §6 lists a tool's
    steps (F-263).

    ``follow_presence`` False is a reader that follows no presence (§2.5,
    §2.8): every owner unseen, no descriptor read. ``across_face`` is a
    reader across a constrained face (§2.8): it follows no presence, and
    holds a ``meta.zid`` only from ``owner_zids``, its deployment's word
    (text 0.3, :func:`zk2py.timing.owners_held`); elsewhere ``owner_zids``
    is refused, since its descriptors are the word."""

    def __init__(self, session: Any, watch: Any, address: str, contract: Any, rids: list[str], *,
                 epsilon_ms: int | float | None = None, params: dict[str, str] | None = None,
                 on_value: Callable[[str, bytes], None] | None = None,
                 on_report: Callable[[str, tm.Report], None] | None = None,
                 seed_first: bool = False, seed_timeout_s: float = 3.0, subscribe_first: bool = False,
                 follow_presence: bool = True, across_face: bool = False,
                 owner_zids: list[str] | tuple[str, ...] = ()) -> None:
        if owner_zids and not across_face:
            raise ValueError("owner_zids is a deployment's word across a constrained face (§2.8); elsewhere the "
                             "descriptors state meta.zid")
        self.across_face = across_face
        self.follows = follow_presence and not across_face
        self.session, self.watch, self.address = session, watch, address
        self.system, self.service = address.split("/", 1)
        canonical = contract.canonical if hasattr(contract, "canonical") else contract
        self.iface = canonical["interface"]
        self.epsilon_ms = epsilon_ms
        self.params = dict(params or {})
        self.on_value, self.on_report = on_value, on_report
        self.lock = threading.RLock()
        self.cond = threading.Condition(self.lock)
        self.closed = False
        self.owner_state = tm.UNSEEN
        #: (monotonic ns, state) of each change of the owner's token as observed
        self.owner_states: list[tuple[int, str]] = []
        self.arrivals: list[Arrival] = []
        self.reports: list[tuple[str, tm.Report]] = []
        #: samples on a wildcard key (core R6) and delete samples: no arrival
        self.discarded: list[str] = []
        self.deletes: list[str] = []
        #: descriptors held, by instance
        self.descriptors: dict[str, dict[str, Any]] = {}
        #: when each instance's first descriptor reached the reader, and how
        #: (its ``put``, core §3.3, or a ``get``)
        self.descriptor_at: dict[str, tuple[int, str]] = {}
        self._fetching: set[str] = set()
        #: the instants a tick ran late past a report's detection bound
        self.late: list[tuple[str, int]] = []
        self.resources: dict[str, _Resource] = {}
        by_rid = {f"{r['token']}/{r['template']}": r for r in canonical["resources"]}
        for rid in rids:
            r = by_rid[rid]
            decl = tm.declaration(r["kind"], r.get("annotations", {}))
            base = f"zk2/{address}/{self.iface}/{r['token']}"
            known = self._known(base, r)
            bound = self._bound(r)
            # text 0.2 (F-259): no tracker under an invalid declaration, a
            # deadline below the period, or none; an epoch template's known
            # member waited for from its member token (F-261)
            tracker = tm.tracker_for(decl, bound, known, epoch=bool(r.get("epoch")))
            self.resources[rid] = _Resource(rid, r, decl, f"{base}/{_template_expr(r['template'])}", known, tracker,
                                            tm.LifespanReading(decl, epsilon_ms,
                                                               tm.owners_held(across_face, [], owner_zids)),
                                            r.get("epoch"))
        self._folds_seen = 0
        #: how far the follower's heard events and reads were scanned for
        #: member tokens seen (F-261)
        self._events_scanned = 0
        self._reads_scanned = 0
        #: each member's first member token seen, as handed to its tracker
        self.member_seen_at: dict[str, list[int]] = {}
        self.subs: list[Any] = []
        self.start_ns: int | None = None
        if subscribe_first:
            # §6's setup in its listed order: subscribed, then presence
            # followed, then the descriptor read (F-263)
            self._subscribe()
        # Presence first: the follower (its seed read made in its
        # constructor), then the descriptors' puts. A reader that follows no
        # presence holds every owner unseen (§2.5), and across a face no
        # descriptor crosses (§2.8): it reads none.
        self.follower: PresenceFollower | None = None
        self._threads = [threading.Thread(target=self._timer_loop, daemon=True)]
        if self.follows:
            self.follower = PresenceFollower(session, watch, f"zk2/{address}/@zk/**")
            import zenoh

            self.subs.append(session.declare_subscriber(f"zk2/{address}/@zk/instance/*",
                                                        zenoh.handlers.Callback(self._descriptor_put)))
            self._threads.append(threading.Thread(target=self._presence_loop, daemon=True))
        for th in self._threads:
            th.start()
        if seed_first and self.follower is not None:
            self.wait_seeded(seed_timeout_s)
        if not subscribe_first:
            self._subscribe()

    # -- setup ----------------------------------------------------------------------

    def _known(self, base: str, r: dict[str, Any]) -> list[str]:
        """v1.md §0: a member is known when its template has no parameter, or
        the reader's binding binds every one (core R2)."""
        from . import templates

        tpl = templates.parse_template(r["template"])
        names = [s.text for s in tpl.segments if s.kind != templates.LITERAL]
        if any(n not in self.params for n in names):
            return []
        from .slug import slug

        chunks = [slug(self.params[s.text]) if s.kind != templates.LITERAL else s.text for s in tpl.segments]
        return [f"{base}/{'/'.join(chunks)}"]

    def _bound(self, r: dict[str, Any], stated: int | None = None) -> int | None:
        """Core §2.7 (v1.md §2.5): the instance's stated cardinality, else the
        contract's; one for a template without parameters; none for
        4294967295."""
        if not r.get("params"):
            return 1
        b = r.get("cardinality")
        if b is None:
            return None
        if stated is not None and 1 <= stated <= b:
            b = stated
        return None if b == 4294967295 else b

    def _subscribe(self) -> None:
        import zenoh

        t0 = time.monotonic_ns()
        with self.lock:
            self.start_ns = t0
            for res in self.resources.values():
                if res.tracker is not None:
                    self._apply(res, res.tracker.start(t0))
        for res in self.resources.values():
            self.subs.append(self.session.declare_subscriber(
                res.expr, zenoh.handlers.Callback(lambda s, res=res: self._sample(res, s))))

    def wait_seeded(self, timeout_s: float) -> bool:
        """Until the follower's first read is folded and every instance it
        shows has its descriptor held."""
        if self.follower is None:
            return False
        end = time.monotonic() + timeout_s
        while time.monotonic() < end:
            with self.follower.lock:
                folded = bool(self.follower.folds)
                instances = {k.rsplit("/", 1)[1] for x in self.follower.view.holders.values() for k in x.held
                             if "/@zk/instance/" in k}
            with self.lock:
                have = set(self.descriptors)
            if folded and instances <= have:
                # the seed's word applied before the window starts (§2.7: the
                # owner present at its start)
                self._presence_changed()
                return True
            time.sleep(0.01)
        return False

    # -- descriptors (meta.zid, core §3.3) ------------------------------------------

    def _descriptor_put(self, sample: Any) -> None:
        at = time.monotonic_ns()
        try:
            doc = json.loads(sample.payload.to_bytes())
        except ValueError:
            return
        self._take_descriptor(doc, at, "put")

    def _take_descriptor(self, doc: Any, at: int | None = None, source: str = "get") -> None:
        if not isinstance(doc, dict) or not isinstance(doc.get("instance"), str):
            return
        if doc.get("service") != self.address:
            return
        meta = doc.get("meta") if isinstance(doc.get("meta"), dict) else {}
        with self.lock:
            if doc["instance"] not in self.descriptors:
                self.descriptor_at[doc["instance"]] = (time.monotonic_ns() if at is None else at, source)
            self.descriptors[doc["instance"]] = doc
            zid = meta.get("zid")
            for res in self.resources.values():
                if isinstance(zid, str):
                    res.lifespan.add_owner(zid)
                for e in doc.get("interfaces", []):
                    if isinstance(e, dict) and e.get("iface") == self.iface and res.tracker is not None:
                        stated = (e.get("cardinality") or {}).get(res.rid)
                        if isinstance(stated, int):
                            res.tracker.bound = self._bound(res.r, stated)
        self._presence_changed()

    def _fetch_descriptors(self, instances: set[str]) -> None:
        from . import live

        for inst in instances:
            with self.lock:
                if inst in self.descriptors or inst in self._fetching:
                    continue
                self._fetching.add(inst)

            def go(inst: str = inst) -> None:
                try:
                    for a in live.get_descriptor(self.session, f"zk2/{self.address}/@zk/instance/{inst}"):
                        if a.ok:
                            try:
                                self._take_descriptor(json.loads(a.payload), None, "get")
                            except ValueError:
                                pass
                            break
                finally:
                    with self.lock:
                        self._fetching.discard(inst)

            threading.Thread(target=go, daemon=True).start()

    # -- samples ----------------------------------------------------------------------

    def _sample(self, res: _Resource, sample: Any) -> None:
        import zenoh

        mono, wall = time.monotonic_ns(), time.time_ns()
        key = str(sample.key_expr)
        if "*" in key or "$" in key:
            with self.lock:
                self.discarded.append(key)  # core R6
            return
        if sample.kind == zenoh.SampleKind.DELETE:
            with self.lock:
                self.deletes.append(key)  # "a delete sample is none": a stream's samples are values
            return
        ts = sample.timestamp
        stamp_ns = None if ts is None else ts.get_time_as_ntp64().as_nanos()
        stamp_id = None if ts is None else str(ts.get_id())
        stamp = None if ts is None else tm.Stamp(wall - stamp_ns, stamp_id)
        payload = sample.payload.to_bytes()
        with self.lock:
            v = res.lifespan.judge(stamp)
            expired = v.verdict == tm.EXPIRED
            if res.tracker is not None:
                self._apply(res, res.tracker.arrival(mono, key, expired))
            if not expired and key not in res.first_held and (res.tracker is None or key in res.tracker.members):
                res.first_held[key] = mono
            self.arrivals.append(Arrival(res.rid, key, mono, wall, stamp_ns, stamp_id, v, not expired, payload))
            self.cond.notify_all()
        # §2.6: "A consumer MUST discard an expired sample: it does not hand it
        # to its application as a value … It MUST NOT discard a sample whose
        # lifespan is live, unobservable or not asked."
        if not expired and self.on_value is not None:
            self.on_value(key, payload)

    def _apply(self, res: _Resource, reports: list[tm.Report]) -> None:
        """Record a tracker's reports (the lock held), and each known
        member's wait as it begins."""
        if res.tracker is not None:
            for name in res.known:
                m = res.tracker.members.get(name)
                if m is not None and m.wait is not None and name not in res.first_held:
                    res.first_held[name] = m.wait
        for r in reports:
            self.reports.append((res.rid, r))
            if r.event == "missed" and r.at_ns > (r.by_ns or 0):
                self.late.append((r.member, r.at_ns - (r.by_ns or 0)))
            if self.on_report is not None:
                self.on_report(res.rid, r)
        self.cond.notify_all()

    # -- presence -----------------------------------------------------------------------

    def _presence_loop(self) -> None:
        # from the follower's first version: a fold before this thread first
        # waits is never missed
        v: int | None = 0
        assert self.follower is not None
        while not self.closed:
            got = self.follower.next_move(0.5, since=v)
            if self.closed:
                return
            if got is not None:
                v = got
            self._presence_changed()

    def _tokenless(self) -> bool:
        with self.lock:
            for doc in self.descriptors.values():
                for e in doc.get("interfaces", []):
                    if isinstance(e, dict) and e.get("iface") == self.iface and e.get("token") is False:
                        return True
        return False

    def _epoch_members(self, res: _Resource, chunk: str) -> list[str]:
        """The members of an epoch resource whose epoch parameter's chunk is
        ``chunk``: those held, and the one key a template whose one
        parameter is the epoch names."""
        from .slug import slug
        from . import templates

        tpl = templates.parse_template(res.r["template"])
        prefix = f"zk2/{self.address}/{self.iface}/{res.r['token']}/"
        out = []
        held = res.tracker.held() if res.tracker is not None else []
        for m in held:
            b = templates.match(tpl, m[len(prefix):].split("/")) if m.startswith(prefix) else None
            if b and res.epoch in b and slug(b[res.epoch][0]) == chunk:
                out.append(m)
        names = [x.text for x in tpl.segments if x.kind != templates.LITERAL]
        if names == [res.epoch]:
            key = prefix + "/".join(chunk if x.kind != templates.LITERAL else x.text for x in tpl.segments)
            if key not in out:
                out.append(key)
        return out

    def _sightings(self) -> list[str]:
        """The member chunks of this address's interface whose member tokens
        the follower heard declared or withdrawn, or a read showed, since
        the last scan, in the order had (the follower's lock held)."""
        from .presence import holder_of

        f = self.follower
        out: list[str] = []
        events = f.view.events[self._events_scanned:]
        self._events_scanned = len(f.view.events)
        reads = f.view.reads[self._reads_scanned:]
        self._reads_scanned = len(f.view.reads)
        for key in [k for _, k, _ in events] + [k for r in reads for k in r.keys]:
            h = holder_of(key)
            if h is not None and h[0] == "member" and h[1:4] == (self.system, self.service, self.iface):
                out.append(h[4])
        return out

    def _presence_changed(self) -> None:
        """The owner's token as observed (v1.md §0); each member token of an
        epoch template seen, heard or shown (text 0.2, F-261); and each held
        member of an epoch template whose member tokens a complete read
        shows gone, which its tracker takes only after a token of it was
        seen."""
        from .slug import slug
        from . import templates

        if self.follower is None:
            return
        tokenless = self._tokenless()
        f = self.follower
        with f.lock:
            sightings = self._sightings()
            if tokenless:
                st, _ = f.view.state(("instance", self.system, self.service))
            else:
                st, _ = f.view.state(("alive", self.system, self.service, self.iface))
            new_folds = f.folds[self._folds_seen:]
            self._folds_seen = len(f.folds)
            complete = any(kind == "complete" and ordered for kind, ordered in new_folds)
            instances = {k.rsplit("/", 1)[1] for x in f.view.holders.values() for k in x.held
                         if "/@zk/instance/" in k}
            gone: dict[str, list[str]] = {}
            if complete:
                with self.lock:
                    for res in self.resources.values():
                        if res.tracker is None or not res.epoch:
                            continue
                        tpl = templates.parse_template(res.r["template"])
                        prefix = f"zk2/{self.address}/{self.iface}/{res.r['token']}/"
                        for m in res.tracker.held():
                            b = templates.match(tpl, m[len(prefix):].split("/")) if m.startswith(prefix) else None
                            if not b or res.epoch not in b:
                                continue
                            h = ("member", self.system, self.service, self.iface, slug(b[res.epoch][0]))
                            if f.view.state(h)[0] == DOWN:
                                gone.setdefault(res.rid, []).append(m)
        if tokenless and st == UP and not self._tokenless_listed():
            st = "unobservable"
        now = time.monotonic_ns()
        with self.lock:
            if st == UP:
                state = tm.PRESENT
            elif st == DOWN:
                state = tm.ABSENT
            else:
                # a withdrawal heard, a read not made or possibly incomplete,
                # a transport's move: not present, never absent by itself
                state = tm.UNCONFIRMED if self.owner_state in (tm.PRESENT, tm.UNCONFIRMED) else self.owner_state
            if state != self.owner_state:
                self.owner_state = state
                self.owner_states.append((now, state))
                for res in self.resources.values():
                    if res.tracker is not None:
                        self._apply(res, res.tracker.owner(now, state))
            for chunk in sightings:
                for res in self.resources.values():
                    if res.tracker is None or not res.epoch:
                        continue
                    for m in self._epoch_members(res, chunk):
                        if m not in res.tracker.seen:
                            self.member_seen_at.setdefault(m, []).append(now)
                        self._apply(res, res.tracker.member_seen(now, m))
            for rid, members in gone.items():
                res = self.resources[rid]
                for m in members:
                    self._apply(res, res.tracker.member_gone(now, m))
        self._fetch_descriptors(instances)

    def _tokenless_listed(self) -> bool:
        """For a tokenless interface, "its descriptor listing the interface"."""
        with self.lock:
            return any(any(isinstance(e, dict) and e.get("iface") == self.iface for e in d.get("interfaces", []))
                       for d in self.descriptors.values())

    # -- the reader's own instants (§2.5) --------------------------------------------

    def _timer_loop(self) -> None:
        """A one-shot wait at the earliest instant a miss is due among the
        members held (or one is forgotten): "A one-shot timer set at the
        earliest instant a miss is due among them meets the bound"."""
        while True:
            with self.cond:
                if self.closed:
                    return
                nxt = min((i for res in self.resources.values() if res.tracker is not None
                           and (i := res.tracker.next_instant()) is not None), default=None)
                now = time.monotonic_ns()
                if nxt is None or nxt > now:
                    self.cond.wait(0.5 if nxt is None else min((nxt - now) / NS, 0.5))
                    continue
                for res in self.resources.values():
                    if res.tracker is not None:
                        self._apply(res, res.tracker.tick(now))

    def tick(self, at_ns: int | None = None) -> None:
        """Judge every member at an instant."""
        now = time.monotonic_ns() if at_ns is None else at_ns
        with self.lock:
            for res in self.resources.values():
                if res.tracker is not None:
                    self._apply(res, res.tracker.tick(now))

    def end_window(self, at_ns: int) -> None:
        """A tool's window ends (§2.7): every tracker judged there, as at a
        tick, and its stretches closed (text 0.5, F-278, F-280)."""
        with self.lock:
            for res in self.resources.values():
                if res.tracker is not None:
                    self._apply(res, res.tracker.end(at_ns))

    # -- reading it -----------------------------------------------------------------------

    def reports_of(self, member: str | None = None, event: str | None = None) -> list[tm.Report]:
        with self.lock:
            return [r for _, r in self.reports if (member is None or r.member == member)
                    and (event is None or r.event == event)]

    def arrivals_of(self, key: str) -> list[Arrival]:
        with self.lock:
            return [a for a in self.arrivals if a.key == key]

    def present_at(self) -> int | None:
        """The instant the reader first saw its owner present."""
        with self.lock:
            return next((t for t, s in self.owner_states if s == tm.PRESENT), None)

    def close(self) -> None:
        with self.cond:
            self.closed = True
            self.cond.notify_all()
        for s in self.subs:
            try:
                s.undeclare()
            except Exception:  # noqa: BLE001 - closing anyway
                pass
        if self.follower is not None:
            self.follower.close()


class Window:
    """A tool's window over ``address`` (§2.7): it follows presence and holds
    the descriptors first, then subscribes (the window's start, F-263), and
    at the end evaluates every tracker and answers §5's questions of every
    member it held in the window, one that ended in it included (§2.7's
    "each member it holds over a window", F-267).

    A tool holds no binding (§2.7, text 0.3): it is handed none, so only a
    member of a template without parameters is known to it, and a member of
    ``axis/{axis}`` is judged from its first arrival, as §6's ``axis/x``.
    Across a constrained face (``across_face``) it follows no presence and
    reads no descriptor (§2.8), so the deadline and the period are
    unobservable, and ``owner_zids``, the deployment's word, is every
    ``meta.zid`` it holds for the lifespan (text 0.3)."""

    def __init__(self, session: Any, watch: Any, address: str, contract: Any, rids: list[str], *,
                 epsilon_ms: int | float | None = None, across_face: bool = False,
                 owner_zids: list[str] | tuple[str, ...] = (), seed_first: bool = True) -> None:
        """``seed_first`` False opens the window (the subscriptions) before
        the first presence read lands, as §6's setup lists the steps
        (F-263): the owner is then unseen at the window's start."""
        from .budget import StallWatch

        self.across_face = across_face
        self.stall = StallWatch(session)
        self.reader = Reader(session, watch, address, contract, rids, epsilon_ms=epsilon_ms,
                             seed_first=seed_first and not across_face,
                             subscribe_first=not seed_first and not across_face, across_face=across_face,
                             owner_zids=owner_zids)
        self.start_ns = self.reader.start_ns
        self.end_ns: int | None = None
        #: each member's reading and its period span (§2.7; None with no
        #: period declared), both filled by :meth:`end` over one member set,
        #: the keys of what it returns (review-bot on PR #941)
        self.readings: dict[str, tm.Reading] = {}
        self.spans: dict[str, int | None] = {}

    def end(self) -> dict[str, dict[str, tm.Answer]]:
        """The window's end: every tracker evaluated there, then each
        member's reading and §5's three answers."""
        rd = self.reader
        self.end_ns = end = time.monotonic_ns()
        rd.end_window(end)
        gap_ms, closed = self.stall.reading()
        # §2.7 (text 0.3): a stall of the receive path names no member, so it
        # is "lost for every member it holds"; zk2py's subscriptions hand
        # each sample to its callback with no queue of its own, so no
        # refusal is known of one member alone
        unattributed = closed or gap_ms > self.stall.bound_ms
        out: dict[str, dict[str, tm.Answer]] = {}
        self.readings, self.spans = {}, {}
        start = self.start_ns
        assert start is not None
        with rd.lock:
            states = list(rd.owner_states)
            present_at_start = self._state_at(states, start) == tm.PRESENT
            present_throughout = present_at_start and all(s == tm.PRESENT for t, s in states if start < t <= end)
            for res in rd.resources.values():
                members = set(res.first_held) | set(res.known)
                if res.tracker is not None:
                    # each member the tracker held at any instant of the window
                    members |= set(res.tracker.held_in_window())
                for m in sorted(members):
                    lost = tm.lost_for(m, (), unattributed)
                    out[m] = tm.conclude(res.decl, self._reading(res, m, start, end, lost, present_throughout))
        # one member set: each member answered has its reading and its span
        # (None only where no period is declared), never one without the other
        assert set(out) == set(self.readings) == set(self.spans), (sorted(out), sorted(self.spans))
        return out

    @staticmethod
    def _state_at(states: list[tuple[int, str]], t: int) -> str:
        s = tm.UNSEEN
        for at, st in states:
            if at <= t:
                s = st
        return s

    def _reading(self, res: _Resource, m: str, start: int, end: int, lost: bool, present: bool) -> tm.Reading:
        rd = self.reader
        mine = [r for rid, r in rd.reports if rid == res.rid and r.member == m and start <= r.at_ns <= end]
        arr = [a.mono_ns for a in rd.arrivals if a.key == m and a.handed]
        if res.tracker is not None:
            # §2.7 (text 0.5): the tracker's account, its longest unbroken
            # stretch, never the sum: a known member never heard held only
            # while waited for (F-278), and a member over its first life,
            # its first end stopping its stretches and its span (F-280)
            held, ended_at = res.tracker.held_ns(m), res.tracker.first_end(m)
            assert isinstance(held, int)
        else:
            # no tracker, no deadline judged and no end reported: the held
            # time no answer reads, from the member's first holding
            held, ended_at = tm.window_held(start, end, res.first_held.get(m), [])
        missed = [r for r in mine if r.event == "missed"]
        acct = tm.DeadlineAccount(held, sum(1 for r in missed if r.owner == tm.PRESENT),
                                  sum(1 for r in missed if r.owner != tm.PRESENT), ended_at is not None)
        known = m in res.known
        # conclude() reads the kind, the declaration, the key and the face
        # before the period's verdict, so with no period this is never read
        p = (tm.period(res.decl.period_ms, start, end, known, ended_at, arr)
             if res.decl.period_ms is not None else None)
        self.spans[m] = None if p is None else p.span_ns
        # each sample's lifespan, in either life, within the window (§2.7)
        lifespans = tuple(a.verdict for a in rd.arrivals if a.key == m and start <= a.mono_ns <= end)
        reading = tm.Reading(acct, tm.NOT_HEARD if p is None else p.verdict, lifespans, self.across_face, lost, present)
        self.readings[m] = reading
        return reading

    def close(self) -> None:
        self.reader.close()
        self.stall.close()


def stamped_clock(session: Any, offset_ns: Callable[[], int] | int = 0) -> Callable[[], Any]:
    """An owner's stamping clock: its host's wall clock plus ``offset_ns``
    (a number, or a function of nothing giving one), under its session's
    zid, as the scenarios' runner sets an owner's clock (``simulate_offset``).
    Read from the wall clock, never the session's HLC, which follows the
    later stamps it routes (core §4.1, 0.53)."""
    import zenoh

    zid = session.new_timestamp().get_id()

    def clock() -> Any:
        off = offset_ns() if callable(offset_ns) else offset_ns
        n = time.time_ns() + off
        return zenoh.Timestamp(zenoh.NTP64(n // NS, n % NS), zid)

    return clock


__all__ = ["Reader", "Window", "Arrival", "stamped_clock"]

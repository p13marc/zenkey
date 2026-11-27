"""``health.v2`` on a bus (text 0.1 and 0.2, #609 rounds 120 and 123): zk2py's
owner and its reader, over :class:`zk2py.owner.Owner` and
:mod:`zk2py.freshness`'s subscriber. Round 120 ran the two against each
other; round 123 runs each against the reference's
(:mod:`zk2py.health_v2_pairings`).

**The owner** (:class:`HealthV2Owner`, §2.2, §2.3, §2.5, §2.10): the status
"before step 4, then on every change", packed, its word written from the
profile's table or its instance's own; a check "on every change of level or
detail", in §2.2's order by seq; a fault "on each occurrence, a
``clock_ahead`` once per hold"; nothing put again unchanged, the heartbeat
confirming the status, its time the ahead time while the guard holds
(``freshness.v2`` §2.10).

**The reader** (:class:`Listener`, :func:`read_near`, :func:`read_across`):
S's deliveries kept as they came, each put read under its CH's tag; a late
joiner's GET over ``health.v2/state/**`` read as two snapshots
(:func:`zk2py.health_v2.late_joiner`); §2.11's reading; the clock question
from the heartbeats read; a word shown through the instance's table, read
from the revisions its descriptor names.
"""

from __future__ import annotations

import json
import threading
import time
from typing import Any

from . import freshness as fr
from . import health as v1
from . import health_v2 as h2

IFACE = h2.IFACE


def standard() -> Any:
    from .contract import load_contract

    c = load_contract(h2.CONTRACT)
    if not c.valid:
        raise ValueError(f"{h2.CONTRACT}: {c.codes}")
    return c


class RuleError(ValueError):
    """An application's request that §2.2 forbids."""


class HealthV2Owner:
    """A service's ``health.v2``. ``contracts`` are its own, one of which
    may hold its tables (§2.10); ``liar`` lets a test owner hold a status
    better than a check (scenarios §7); ``clock_ms`` is its clock, Unix
    milliseconds, for ``since`` and ``at``."""

    def __init__(self, system: str, service: str, *, contracts: list[Any] | None = None, level: str = h2.UNKNOWN,
                 reason: str | None = "starting", clock_ms: Any = None, liar: bool = False, **owner_kw: Any):
        from .owner import Owner

        self.contract = standard()
        self.contracts = [self.contract, *(contracts or [])]
        #: §2.10 "An instance's table": the one table among its contracts
        self.tables = {k: h2.instance_table(k, [c.fingerprint for c in self.contracts],
                                            {c.fingerprint: c.canonical for c in self.contracts}) for k in h2.KINDS}
        self.clock_ms = clock_ms or (lambda: time.time_ns() // 1_000_000)
        self.liar = liar
        self.level, self.reason = level, reason
        self.since_ms = self.clock_ms()
        self.checks: dict[str, tuple[str, str | None]] = {}
        #: "The words stay in the owner's log": each word written out_of_range
        self.log: list[str] = []
        self.faults: list[dict[str, Any]] = []
        self.holds = 0
        self._lock = threading.RLock()
        #: the checks held before the tokens: name -> (level, detail, at)
        self._before: dict[str, tuple[str, str | None, int]] = {}
        self.owner = Owner(system, service, self.contracts, **owner_kw)
        self.owner.on_guard = self._guard

    def key(self, rel: str) -> str:
        return f"zk2/{self.owner.system}/{self.owner.service}/{IFACE}/{rel}"

    @property
    def status_key(self) -> str:
        return self.key("state/status")

    def check_key(self, name: str) -> str:
        from .slug import slug

        return self.key(f"state/checks/{slug(name)}")

    def start(self) -> None:
        """The status put before step 4 (§2.3), and the checks held with
        it. Their records are written here, never at construction: the
        owner's catalog is set "before start", and its epoch is theirs."""
        with self._lock:
            self.owner.initial = self.initial() | dict(self.owner.initial or {})
        self.owner.start()

    def initial(self) -> dict[str, bytes]:
        """The records held at start, by key relative to the service: the
        status, and each check of :meth:`hold_check`."""
        from .slug import slug

        out = {f"{IFACE}/state/status": self._status_bytes()}
        for name, (level, detail, at) in self._before.items():
            out[f"{IFACE}/state/checks/{slug(name)}"] = self._record(h2.CHECK, level, detail, at)[1]
        return out

    def close(self) -> None:
        self.owner.close()

    def _word(self, kind: str, word: str | None) -> Any:
        n = h2.write_word(kind, word, self.tables[kind])
        if n == h2.OUT_OF_RANGE:
            self.log.append(f"{kind}: {word}")
        return n

    @property
    def epoch(self) -> int:
        """The deployment epoch every record of this owner counts its
        ``since`` and ``at`` from: its owner's, "its catalog's, else the one
        its deployment states, else 0" (core §7.6, §8.6), the one its
        heartbeat's time counts from."""
        return self.owner.writing_epoch()

    def _record(self, name: str, level: str, word: str | None, at_ms: int | None,
                argument: Any = h2.NOT_AVAILABLE) -> tuple[dict[str, Any], bytes]:
        """One record of this owner, its value and its bytes: its word
        through the instance's table of its kind (:meth:`_word`), its time
        under :attr:`epoch`. Every record the owner writes is written here,
        the status and the checks it holds before its tokens among them."""
        value = h2.value_of(name, level, self._word(h2.KIND_OF[name], word), at_ms, argument, self.epoch)
        return value, h2.write(name, value, self.epoch)

    def _status_bytes(self) -> bytes:
        return self._record(h2.STATUS, self.level, self.reason, self.since_ms)[1]

    def hold_check(self, name: str, level: str, detail: str | None = None) -> None:
        """A check held before the tokens (a test owner's, scenarios §7):
        written at :meth:`start` as :meth:`set_check` writes one, and put
        with the status at step 4. Only before :meth:`start`."""
        with self._lock:
            self.checks[name] = (level, detail)
            self._before[name] = (level, detail, self.clock_ms())

    def _worst(self) -> str | None:
        return h2.worst_of(lv for lv, _ in self.checks.values())

    def set_status(self, level: str, reason: str | None = None) -> None:
        """Put the status on a change. "A change of level sets [since]. A
        change of reason alone keeps it." A level better than the worst
        current check is refused (§2.2); nothing is put again unchanged."""
        with self._lock:
            w = self._worst()
            if not self.liar and level in h2.LEVELS and w is not None \
                    and h2.LEVELS.index(w) > h2.LEVELS.index(level):
                raise RuleError(f"status {level} would be better than a check at {w} (§2.2)")
            if (level, reason) == (self.level, self.reason):
                return
            if level != self.level:
                self.since_ms = self.clock_ms()
            self.level, self.reason = level, reason
            self.owner.set_state(self.status_key, self._status_bytes())

    def set_check(self, name: str, level: str, detail: str | None = None) -> None:
        """Put a check on a change of level or detail. "A check that gets
        worse than the status: the status first, then the check", the
        status's reason ``check``; "A check that gets better, or is deleted:
        the check first, then the status, if it improves" (the status's
        improvement is the application's, :meth:`set_status`)."""
        with self._lock:
            if self.checks.get(name) == (level, detail):
                return
            if not self.liar and level in h2.LEVELS and self.level in h2.LEVELS \
                    and h2.LEVELS.index(level) > h2.LEVELS.index(self.level):
                self.set_status(level, "check")
            self.checks[name] = (level, detail)
            self.owner.set_state(self.check_key(name), self._record(h2.CHECK, level, detail, self.clock_ms())[1])

    def retire_check(self, name: str) -> None:
        with self._lock:
            self.checks.pop(name, None)
            self.owner.delete_state(self.check_key(name))

    def publish_fault(self, code: str | None, level: str, argument: Any = h2.NOT_AVAILABLE) -> None:
        """A fault, an occurrence (§2.3). "No Zenoh timestamp": its put
        carries none of the owner's (§2.5), so none is given here."""
        value, data = self._record(h2.FAULT, level, code, self.clock_ms(), argument)
        self.owner.publish(self.key("stream/faults"), data)
        self.faults.append(value)

    def _guard(self, held: bool, offset_ns: int) -> None:
        """§2.5 "The fault, once": "When its guard begins to hold, an owner
        MUST publish one fault with the code clock_ahead and the level
        failed", its argument "the offset it measured, in whole seconds,
        rounded up"; "the owner SHOULD NOT publish the fault again" while it
        holds; "A guard that releases and later holds again begins a new
        hold, with a fault of its own"."""
        if not held:
            return
        from fractions import Fraction

        self.holds += 1
        self.publish_fault("clock_ahead", "failed", h2.clock_ahead_argument(Fraction(max(offset_ns, 0), 10**9)))


# -- the reader ---------------------------------------------------------------------------

class Listener:
    """S: a subscriber on ``zk2/<address>/health.v2/**`` and on the
    heartbeat key covering ``health.v2``, "before the owner starts". It
    "records each delivery's arrival on its monotonic clock, with its key,
    its kind, its payload and its in-band stamp", the payload as it came,
    and "reads each heartbeat as freshness.v2 §2.5 says"
    (:meth:`zk2py.freshness.Subscriber.follow`). ``selectors`` names other
    keys to hear: a late joiner hears the heartbeat key alone (``[]``).
    ``epoch`` is the deployment epoch this reader holds, its catalog's, else
    0 (core §7.6): each record's ``since`` and ``at`` is read under it."""

    def __init__(self, session: Any, address: str, selectors: list[str] | None = None, trust: Any = None,
                 epoch: int = 0, catalogs: Any = None, named: str | None = None):
        """``catalogs``: the catalogs the reader holds, and ``named`` the id
        the owner's descriptor states; given, it follows the owner's owner
        heartbeats too (``freshness.v2`` §2.4; round 123)."""
        import zenoh

        self.address = address
        self.epoch = epoch
        self.raw: list[dict[str, Any]] = []
        self._lock = threading.Lock()
        sel = [f"zk2/{address}/{IFACE}/**"] if selectors is None else selectors
        self.sub = fr.Subscriber(session, sel, trust) if sel else fr.Subscriber(session, [], trust)
        c = standard()
        self.sub.follow(address, c.canonical, c.fingerprint)
        if catalogs is not None:
            self.sub.follow_owner_heartbeats(address, catalogs, named)
        self._subs = [session.declare_subscriber(s, zenoh.handlers.Callback(self._heard)) for s in sel]
        time.sleep(0.2)

    def _heard(self, sample: Any) -> None:
        import zenoh

        with self._lock:
            self.raw.append({"at": time.monotonic_ns(), "key": str(sample.key_expr),
                             "kind": "put" if sample.kind == zenoh.SampleKind.PUT else "delete",
                             "wire": sample.payload.to_bytes(), "encoding": str(sample.encoding),
                             "timestamp": None if sample.timestamp is None else str(sample.timestamp)})

    def of(self, rel: str, since: int = 0) -> list[dict[str, Any]]:
        key = f"zk2/{self.address}/{IFACE}/{rel}"
        with self._lock:
            return [x for x in self.raw if x["key"] == key and x["at"] >= since]

    def all(self, since: int = 0) -> list[dict[str, Any]]:
        with self._lock:
            return [x for x in self.raw if x["at"] >= since]

    def record(self, x: dict[str, Any]) -> h2.Read:
        """One delivery's record, by its key's resource."""
        from . import zk2c

        rel = x["key"].split(f"/{IFACE}/", 1)[1]
        name = h2.STATUS if rel == "state/status" else h2.CHECK if rel.startswith("state/checks/") else h2.FAULT
        return h2.read_put(name, x["wire"], self.epoch, zk2c.envelope_encoding(x["encoding"]))

    def stamp(self, x: dict[str, Any]) -> Any:
        from . import zk2c

        try:
            return zk2c.parse(x["wire"], "put" if x["kind"] == "put" else "delete").stamp
        except zk2c.Refused:
            return None

    def beats(self, since: int = 0) -> list[tuple[int, dict[str, Any]]]:
        """Each heartbeat heard on the key covering ``health.v2``, with the
        view's answer, and its ``time`` where it was read."""
        return [(t, a) for t, a in self.sub.beats.get((self.address, IFACE), []) if t >= since]

    def clock(self, presence: str = "present", interfaces: Any = (IFACE,), since: int = 0) -> tuple[str, str]:
        """§5's clock question from the heartbeats this listener read."""
        from . import heartbeat

        words = {heartbeat.INSTANT: h2.INSTANT, heartbeat.AHEAD: h2.AHEAD, heartbeat.UNKNOWN: h2.TIME_UNKNOWN}
        heard = [words[heartbeat.reading_of(a["time"])] if "read" in a else h2.NOT_READ
                 for _, a in self.beats(since)]
        return h2.clock_question(presence, interfaces, heard)

    def close(self) -> None:
        for s in self._subs:
            s.undeclare()
        self.sub.close()


def _freshness(observations: list[Any]) -> tuple[str, str]:
    v = fr.judge("state", v1.HORIZON, observations)
    return v.verdict, v.reason


def unknown_catalog(r: h2.Read | None) -> h2.Read | None:
    """Core §8.6, "Never a guess": a record whose ``since`` or ``at`` counts
    from a catalog the reader holds neither of, nor a successor of, is read
    as refused, ``unknown_catalog`` (round 123, F-610): its time cannot be
    read, and v2 §2.6 reads a refused record unknown, "never at a level read
    from its bits". A record with no time needs no epoch, and stands."""
    if r is None or r.refused or r.at_ms is None:
        return r
    return h2.Read(r.name, None, "unknown_catalog", "its time counts from a catalog this reader does not hold")


def epoch_of(named: str | None, catalogs: Any) -> tuple[int | None, str | None]:
    """The epoch a reader holding ``catalogs`` reads an owner naming the
    catalog ``named`` under (core §8.6): 0 where it names none; the held
    catalog's, or a successor's whose ``lineage`` holds it; else none,
    with the refusal ``unknown_catalog``."""
    if named is None:
        return 0, None
    for c in catalogs:
        if c.id == named or named in [x[0] if isinstance(x, (list, tuple)) else x
                                       for x in (getattr(c, "lineage", None) or [])]:
            return c.epoch, None
    return None, "unknown_catalog"


def read_near(tool: Any, address: str, *, listener: Listener | None = None, trust: Any = None, get: bool = True,
              timeout: float = 1.0, held: dict[str, Any] | None = None,
              epoch: int | None = None, catalogs: Any = None) -> tuple[h2.Reading, dict[str, Any]]:
    """One reading of ``address`` on the near side, for §2.11: presence, the
    descriptor's ``interfaces`` (never the interface token), the status as
    ``listener`` saw it and as a GET over ``health.v2/state/**`` answered
    it, in two snapshots (§2.4 "How a late joiner reads"), each record read
    under its snapshot's tag. ``held`` maps a fingerprint to a revision the
    reader holds, for the instance's tables. ``epoch`` is the deployment
    epoch the reader holds: the listener's where one is given and none is
    stated, else 0. The records' times and the snapshot's are read under it.
    ``catalogs`` (round 123): the catalogs the reader holds, the epoch then
    read from the one the descriptor names (core §8.6), and where it holds
    none of it every timed record refused ``unknown_catalog`` and no
    confirmation dated (:func:`unknown_catalog`). Returns the reading and
    what was read: ``joined`` (:class:`zk2py.health_v2.Joined`), ``tables``,
    ``descriptor``, ``catalog`` (the refusal, or None)."""
    from . import live
    from . import seed_snapshot as ss

    info: dict[str, Any] = {}
    if epoch is None:
        epoch = 0 if listener is None else listener.epoch
    elif listener is not None and listener.epoch != epoch:
        raise ValueError(f"the listener reads under the epoch {listener.epoch}, and this reading under {epoch}")
    pres = live.list_presence(tool, f"zk2/{address}/@zk/**")
    info["presence"] = pres
    presence = "present" if pres.instances else "absent" if pres.complete else "incomplete"
    interfaces = None
    named: list[str] | None = None
    if presence == "present":
        d = live.get_descriptor(tool, f"zk2/{address}/@zk/instance/{pres.instances[0]['instance']}", timeout=timeout)
        if len(d) == 1 and d[0].ok:
            doc = json.loads(d[0].payload)
            info["descriptor"] = doc
            interfaces = [e.get("iface") for e in doc.get("interfaces") or []]
            named = [e.get("contract") for e in doc.get("interfaces") or [] if e.get("contract")]
    info["tables"] = {k: h2.instance_table(k, named, held or {}) for k in h2.KINDS}
    info["catalog"] = None
    if catalogs is not None:
        e, why = epoch_of((info.get("descriptor") or {}).get("catalog"), catalogs)
        info["catalog"] = why
        epoch = 0 if e is None else e
    status_key = f"zk2/{address}/{IFACE}/state/status"
    observations: list[Any] = []
    record: h2.Read | None = None
    if listener is not None:
        observations.append(listener.sub.observation(status_key))
        puts = [x for x in listener.of("state/status") if x["kind"] == "put"]
        record = listener.record(puts[-1]) if puts else None
    checks = None
    if get:
        st = live.get_state(tool, f"zk2/{address}/{IFACE}/state/**", timeout=timeout,
                            revision=ss.ByDescriptor([standard()]))
        info["state"] = st
        pages: dict[bool, list[dict[str, Any]]] = {True: [], False: []}
        for snap in st.snapshots:
            pages[snap.key == status_key] += [snap.pages[i] for i in sorted(snap.pages)]
        joined = h2.late_joiner(h2.held(pages[True]), h2.held(pages[False]), epoch)
        if info["catalog"] is not None:
            import dataclasses

            joined = dataclasses.replace(joined, status=unknown_catalog(joined.status),
                                         checks={k: unknown_catalog(c) for k, c in joined.checks.items()})
        info["joined"] = joined
        info["snapshots"] = (h2.held(pages[True]), h2.held(pages[False]))
        mine = [x for x in st.replies if x.key == status_key]
        got = fr.reading_of(mine[-1], epoch, bool(st.complete_for(status_key))) if mine else fr.GetReading(None)
        if info["catalog"] is not None and got.snapshot:
            import dataclasses

            got = dataclasses.replace(got, confirmed_ns=None)    # no confirmation dated: no epoch to date it
        observations.append(got.observation(trust or fr.ClockTrust(word=True)))
        if joined.status is not None:
            record = joined.status
        checks = dict(joined.checks)
    return h2.Reading(presence, interfaces, None, _freshness(observations), record, checks), info


def read_across(listener: Listener, address: str, status_crosses: bool) -> h2.Reading:
    """A reading across a constrained face (§2.8): presence and descriptors
    do not cross, the deployment's word names ``health.v2`` and says whether
    the face lets ``state/status`` cross."""
    puts = [x for x in listener.of("state/status") if x["kind"] == "put"]
    obs = [listener.sub.observation(f"zk2/{address}/{IFACE}/state/status")]
    return h2.Reading("across_face", None, {"status_crosses": status_crosses, "major": h2.V2}, _freshness(obs),
                      listener.record(puts[-1]) if puts else None, None)


def shown(r: h2.Read | None, tables: dict[str, h2.Table]) -> str:
    """A record as a tool shows it (§5): its level, and its word as its
    name, its number, no word, or a word no table names."""
    if r is None:
        return "no record"
    if r.refused:
        return f"unknown (undecodable: {r.refusal})"
    kind = h2.KIND_OF[r.name]
    return f"{r.value['level']}, {h2.read_word(kind, r.word, tables.get(kind)).text()}"

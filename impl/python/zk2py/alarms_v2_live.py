"""``alarms.v2`` on a bus (text 0.1, #609 round 124): zk2py's owner and its
reader, over :class:`zk2py.owner.Owner`, a subscriber and a collection read.
Round 124 runs each against the reference's (:mod:`zk2py.alarms_v2_scenarios`).

**The owner** (:class:`AlarmsV2Owner`, §2.2, §2.4, §2.5, §2.8, §2.9): an
alarm's record on the slot §2.2 gives its identity, written by its
instance's table where the table names it, else by words, the words put
first; a new ``at`` at each transition, the one held at a refresh; a
refresh deferred within half the horizon of the last; a resolve one put, no
delete; at the bound a raise refused, nothing resolved or put over; its
record kept by slot, and a restart's adoption from it before the tokens.

**The reader** (:class:`Listener`, :func:`collection_read`,
:func:`read_owner`): S's deliveries kept as they came, each slot read by
identity (:class:`zk2py.alarms_v2.SlotReader`); a collection read of
``alarms/**`` answered by the alarms' snapshot; the owner's alarms
established by §2.15; each slot named by the instance's table, read from the
revisions its descriptor names that the reader holds; each judged by §2.14.

**The horizon** (text 0.2, round 127; F-613): "A reader ages an alarm at the
``freshness.ttl_s`` of the revision its owner's descriptor names (core
§3.3) ... Across a face, the deployment's word names the revision (§2.11)."
The reader holds the standard revision, and any it is given; it retrieves
any other by fingerprint (core §8.4). "A reader that neither holds nor can
retrieve the revision its owner names reads that owner's alarms' freshness
as unobservable, never at 900 s": zk2py's reason is freshness.v1's
``not_a_horizon`` (:func:`horizon_of`).
"""

from __future__ import annotations

import json
import queue
import threading
import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

from . import alarms as v1
from . import alarms_v2 as a2
from . import freshness as fr
from . import jcs

IFACE = a2.IFACE
NS = 1_000_000_000


def standard(ttl_s: int | None = None) -> Any:
    """The standard contract, or a copy whose ``freshness.ttl_s`` is
    ``ttl_s``, "made in memory and never published" (scenarios-v2,
    Conventions)."""
    from .contract import load_contract

    if ttl_s is None:
        c = load_contract(a2.CONTRACT)
    else:
        text = a2.CONTRACT.read_text(encoding="utf-8").replace('"freshness.ttl_s" = 900',
                                                               f'"freshness.ttl_s" = {ttl_s}')
        c = load_contract(a2.CONTRACT, text=text)
    if not c.valid:
        raise ValueError(f"{a2.CONTRACT}: {c.codes}")
    return c


class Refused(ValueError):
    """A raise §2.8 refuses, or an act on no alarm."""


@dataclass
class Alarm:
    """One slot as the owner holds it, and its record keeps it (§2.9 "What
    the record holds": "by slot, each record as put, its alarm's words and
    its alert key, a resolved one included")."""

    record: dict[str, Any]
    words: dict[str, Any]
    alert_key: str
    reading: str | None = None
    refreshed_ns: int | None = None


class AlarmsV2Owner:
    """A service's ``alarms.v2``. ``contracts`` are its own, one of which may
    hold its table (§4); ``record`` a file holding its record by slot, read
    at start and written whole after every act; ``rules`` the rules it
    evaluates (None: every rule); ``bound`` the cardinality its descriptor
    states (§2.8); ``clock_ms`` its clock, Unix milliseconds."""

    def __init__(self, system: str, service: str, *, contracts: list[Any] | None = None,
                 record: str | Path | None = None, rules: set[str] | None = None, bound: int | None = None,
                 clock_ms: Any = None, ttl_s: int | None = None, **owner_kw: Any):
        from .owner import Owner

        self.contract = standard(ttl_s)
        self.ttl_s = a2.TTL_S if ttl_s is None else ttl_s
        self.contracts = [self.contract, *(contracts or [])]
        self.table = a2.instance_table([c.fingerprint for c in self.contracts],
                                       {c.fingerprint: c.canonical for c in self.contracts})
        self.record_path = None if record is None else Path(record)
        self.rules = rules
        self.bound = a2.CARDINALITY if bound is None else bound
        self.clock_ms = clock_ms or (lambda: time.time_ns() // 1_000_000)
        self.at = a2.AtClock()
        self.slots = a2.Slots(self.bound)
        self.alarms: dict[int, Alarm] = {}
        self.adoption: dict[str, Any] = {"source": "nothing", "adopted": [], "resolved": [], "free": [],
                                         "left": []}
        self._lock = threading.RLock()
        card = {IFACE: {"state/alarms/{slot}": bound}} if bound is not None else None
        self.owner = Owner(system, service, self.contracts, cardinality=card, before_tokens=self._before_tokens,
                           **owner_kw)

    # -- keys ------------------------------------------------------------------------------

    def key(self, slot: int, prefix: str = a2.ALARMS) -> str:
        k = a2.slot_key(self.owner.system, self.owner.service, slot, prefix)
        assert k is not None
        return k

    # -- the record (§2.9) -------------------------------------------------------------------

    def _load(self) -> list[a2.Kept]:
        if self.record_path is None or not self.record_path.is_file():
            return []
        try:
            doc = json.loads(self.record_path.read_text(encoding="utf-8"))
        except (OSError, ValueError):
            return []
        if doc.get("profile") == v1.PROFILE:
            return self._load_v1(doc)
        out = []
        for s, e in (doc.get("slots") or {}).items():
            w = e.get("words") or {}
            out.append(a2.Kept(int(s), e.get("record"), w.get("rule"), w.get("labels"), e.get("alert_key"),
                               w.get("summary"), w.get("category")))
        self.adoption["source"] = "record"
        return out

    def _load_v1(self, doc: dict[str, Any]) -> list[a2.Kept]:
        """§2.9 "From ``alarms.v1``": "An owner that moves adopts each firing
        v1 document onto a slot that §2.2 gives it. Its ``at`` is the
        document's ``timestamp`` in whole seconds". A v1 record is its
        documents by alert key (``{"profile": "alarms.v1", "alarms": {…}}``,
        the owner example's usage); each firing one takes the next slot, in
        alert-key order, each resolved one none."""
        out = []
        slot = 0
        docs = doc.get("alarms") or {}
        for ak in sorted(docs):
            d = docs[ak]
            if d.get("state") != v1.FIRING:
                continue
            w = a2.write_by(self.table, d["rule"], d.get("labels"), None, d.get("summary"), d.get("category"))
            rec = {"state": a2.FIRING, "severity": d["severity"], **w.fields()}
            at = a2.from_v1(int(d["timestamp"]))
            if at is not None:
                rec["at"] = at
            out.append(a2.Kept(slot, rec, d["rule"], d.get("labels"), ak, d.get("summary"), d.get("category")))
            slot += 1
        self.adoption["source"] = "record_v1"
        return out

    def _save(self) -> None:
        if self.record_path is None:
            return
        slots = {str(s): {"record": a.record, "words": a.words, "alert_key": a.alert_key,
                          **({"reading": a.reading} if a.reading is not None else {})}
                 for s, a in sorted(self.alarms.items())}
        tmp = self.record_path.with_suffix(".tmp")
        tmp.write_text(json.dumps({"profile": IFACE, "slots": slots}, sort_keys=True), encoding="utf-8")
        tmp.replace(self.record_path)

    def _before_tokens(self, owner: Any) -> None:
        """§2.9: "MUST put each alarm it adopts again from its new instance,
        on its slot, before it answers a GET with it. It SHOULD do so before
        its new instance token." """
        kept = self._load()
        ev = (lambda name: True) if self.rules is None else (lambda name: name in self.rules)
        acts = a2.adopt(kept, ev, self.table, self.bound)
        by_slot = {k.slot: k for k in kept}
        for ad in acts:
            k = by_slot[ad.slot]
            name = {a2.ADOPT: "adopted", a2.PUT_RESOLVED: "resolved", a2.PAST_BOUND: "left"}.get(ad.action, "free")
            self.adoption[name].append(ad.slot)
            if ad.action not in (a2.ADOPT, a2.PUT_RESOLVED):
                continue
            assert ad.record is not None and k.alert_key is not None and k.rule is not None
            words = a2.words_document(k.rule, k.labels, k.summary or "", k.category)
            if ad.record.get("rule") == a2.NOT_AVAILABLE:
                owner.set_state(self.key(ad.slot, a2.WORDS_PREFIX), jcs.dumps(words))
            owner.set_state(self.key(ad.slot), a2.write(ad.record))
            owner.vouch(self.key(ad.slot), ad.action == a2.ADOPT)
            self.at.knows(k.alert_key, ad.record.get("at"))
            self.alarms[ad.slot] = Alarm(dict(ad.record), words, k.alert_key)
            self.slots.held[ad.slot] = a2.Held(k.alert_key, ad.action == a2.ADOPT)
        self._save()

    # -- lifecycle -----------------------------------------------------------------------------

    def start(self) -> None:
        self.owner.start()

    def close(self) -> None:
        self.owner.close()

    def _put(self, slot: int, a: Alarm, words_first: bool) -> None:
        """A record's put, after its words where they go first. §2.7: "A
        resolved record has its bit clear, from its resolved put on", and a
        firing one's is set again by its put (round 124: zk2py's first owner
        left a resolved record's bit set, which the live run found)."""
        if words_first:
            self.owner.set_state(self.key(slot, a2.WORDS_PREFIX), jcs.dumps(a.words))
        self.owner.set_state(self.key(slot), a2.write(a.record))
        self.owner.vouch(self.key(slot), a.record.get("state") == a2.FIRING)

    def raise_(self, rule: str, severity: str, summary: str = "", labels: dict[str, str] | None = None,
               category: str | None = None, reading: Any = None) -> tuple[str, str, int, Alarm]:
        """The condition holds (§2.5): a raise, an escalation, a refresh
        (deferred within half the horizon of the last), or nothing (a
        duplicate observation). Returns (the act, the alert key, the slot,
        the alarm); :class:`Refused` at the bound."""
        with self._lock:
            w = a2.write_by(self.table, rule, labels, reading, summary, category)
            key = v1.derive(rule, labels).key
            words = a2.words_document(rule, labels, summary, category)
            slot = self.slots.slot_of(key)
            held = self.alarms.get(slot) if slot is not None else None
            now_ns = time.monotonic_ns()
            if held is not None and self.slots.held[slot].firing:
                rec = dict(held.record)
                if rec["severity"] != severity:
                    rec.update(w.fields())
                    rec["severity"] = severity
                    self._set_at(rec, key, a2.ESCALATE)
                    a = Alarm(rec, words, key, None if reading is None else str(reading), held.refreshed_ns)
                    self.alarms[slot] = a
                    self._put(slot, a, w.by_words and words != held.words)
                    self._save()
                    return "escalated", key, slot, a
                arg_moved = w.fields().get("argument") != rec.get("argument")
                words_moved = w.by_words and words != held.words
                if not arg_moved and not words_moved:
                    return "unchanged", key, slot, held
                window = self.ttl_s / 2 * NS
                if held.refreshed_ns is not None and now_ns - held.refreshed_ns < window:
                    return "deferred", key, slot, held
                rec.update(w.fields())
                a = Alarm(rec, words, key, None if reading is None else str(reading), now_ns)
                self.alarms[slot] = a
                if arg_moved:
                    self._put(slot, a, words_moved)
                else:
                    self.owner.set_state(self.key(slot, a2.WORDS_PREFIX), jcs.dumps(words))
                self._save()
                return "refreshed", key, slot, a
            got = self.slots.raise_(key)
            if got == a2.REFUSED:
                raise Refused(f"every slot below the bound {self.bound} fires (alarms.v2 §2.8)")
            assert isinstance(got, int)
            rec = {"state": a2.FIRING, "severity": severity, **w.fields()}
            self._set_at(rec, key, a2.RAISE)
            a = Alarm(rec, words, key, None if reading is None else str(reading))
            self.alarms[got] = a
            self._put(got, a, w.by_words)
            self._save()
            return "raised", key, got, a

    def _set_at(self, rec: dict[str, Any], key: str, act: str) -> None:
        at = self.at.act(key, act, self.clock_ms(), rec.get("at"))
        rec.pop("at", None)
        if at is not None:
            rec["at"] = at

    def resolve(self, alert_key: str) -> tuple[int, Alarm] | None:
        """§2.5 "resolve": "the record, and no delete", ``resolved``, a new
        ``at``, the severity it last fired at."""
        with self._lock:
            slot = self.slots.resolve(alert_key)
            if slot is None:
                return None
            held = self.alarms[slot]
            rec = dict(held.record, state=a2.RESOLVED)
            self._set_at(rec, alert_key, a2.RESOLVE)
            a = Alarm(rec, held.words, alert_key, held.reading)
            self.alarms[slot] = a
            self._put(slot, a, False)
            self._save()
            return slot, a

    def confirm(self, alert_key: str | None, on: bool) -> None:
        """§2.7: its bit set only while its owner still evaluates its
        condition; ``None`` is every alarm."""
        with self._lock:
            for slot, a in self.alarms.items():
                if alert_key is None or a.alert_key == alert_key:
                    self.owner.vouch(self.key(slot), on)

    def put_raw(self, slot: int, record: bytes, words: bytes | None = None) -> None:
        """A test owner's (scenarios-v2 §9, ``lab/liar``): bytes put as they
        are, the words first."""
        if words is not None:
            self.owner.set_state(self.key(slot, a2.WORDS_PREFIX), words)
        self.owner.set_state(self.key(slot), record)

    def held(self) -> dict[str, Any]:
        with self._lock:
            return {str(s): {"alert_key": a.alert_key, "record": a.record, "words": a.words,
                             **({"reading": a.reading} if a.reading is not None else {})}
                    for s, a in sorted(self.alarms.items())}


# -- the reader ---------------------------------------------------------------------------

class Listener:
    """S: a subscriber on ``zk2/<address>/alarms.v2/**`` and on the heartbeat
    key covering ``alarms.v2``, "before the owner starts". It "records each
    delivery's arrival on its monotonic clock, with its key, its kind, its
    payload and its in-band stamp", and "reads each heartbeat as
    freshness.v2 §2.5 says" (:meth:`zk2py.freshness.Subscriber.follow`)."""

    def __init__(self, session: Any, address: str, contract: Any = None, selectors: list[str] | None = None):
        import zenoh

        self.address = address
        self.raw: list[dict[str, Any]] = []
        self._lock = threading.Lock()
        sel = [f"zk2/{address}/{IFACE}/**"] if selectors is None else selectors
        self.sub = fr.Subscriber(session, sel)
        c = contract or standard()
        self.sub.follow(address, c.canonical, c.fingerprint)
        from . import heartbeat

        # the heartbeat key covering alarms.v2, kept as it came beside the view's reading of it
        hb = heartbeat.heartbeat_key(*address.split("/", 1), IFACE)
        self._subs = [session.declare_subscriber(s, zenoh.handlers.Callback(self._heard)) for s in [*sel, hb]]
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

    def beats(self, since: int = 0) -> list[tuple[int, dict[str, Any]]]:
        return [(t, a) for t, a in self.sub.beats.get((self.address, IFACE), []) if t >= since]

    def heartbeat_wires(self, since: int = 0) -> list[bytes]:
        """The heartbeats heard, as they came."""
        from . import heartbeat

        key = heartbeat.heartbeat_key(*self.address.split("/"), IFACE)
        with self._lock:
            return [x["wire"] for x in self.raw if x["key"] == key and x["at"] >= since]

    def observation(self, slot: int) -> Any:
        return self.sub.observation(f"zk2/{self.address}/{IFACE}/state/alarms/{slot}")

    def close(self) -> None:
        for s in self._subs:
            s.undeclare()
        self.sub.close()


def stamp_of(x: dict[str, Any]) -> Any:
    from . import zk2c

    try:
        return zk2c.parse(x["wire"], "put" if x["kind"] == "put" else "delete").stamp
    except zk2c.Refused:
        return None


def read_slots(listener: Listener, table: a2.Table = a2.NO_TABLE, since: int = 0) -> dict[int, list[Any]]:
    """S's deliveries on ``alarms/<slot>``, each read by identity against the
    record held before it (§2.6), slot by slot, in arrival order: a list of
    (the put's record, the reading's held fate and transition)."""
    out: dict[int, list[Any]] = {}
    readers: dict[int, a2.SlotReader] = {}
    for x in listener.all(since):
        s = a2.slot_of(x["key"])
        if s is None:
            continue
        slot = s["slot"]
        rd = readers.setdefault(slot, a2.SlotReader(table))
        st = stamp_of(x)
        run = None if st is None else st.run_hex()
        if x["kind"] == "delete":
            step = rd.step(a2.DELETE, run)
            out.setdefault(slot, []).append((None, step))
            continue
        r = a2.read_put(x["wire"])
        step = rd.step(r, run)
        out.setdefault(slot, []).append((r.value, step))
    return out


@dataclass
class Collection:
    """One collection read of ``zk2/<address>/alarms.v2/state/alarms/**``
    (§2.12, §2.15): every reply as it came, the pages of the alarms'
    snapshot read entry by entry (:func:`zk2py.alarms_v2.read_page`), and
    whether it ran to its final reply with no error reply."""

    selector: str
    prefix: str = a2.ALARMS
    replies: list[tuple[str, bytes]] = field(default_factory=list)
    errors: list[str] = field(default_factory=list)
    pages: list[dict[str, Any]] = field(default_factory=list)
    page_bytes: list[bytes] = field(default_factory=list)
    refused: list[str] = field(default_factory=list)
    finished: bool = False

    @property
    def complete_read(self) -> bool:
        return self.finished and not self.errors

    def snapshot(self) -> dict[str, Any] | None:
        """The alarms' snapshot, collection 0, where every page from 0 to the
        last (``more`` clear) is held under one run and asof: its header and
        its entries; None where no page, or one missing."""
        alarms = [p for p in self.pages if p["coll"] == a2.COLLECTION[self.prefix]]
        if not alarms:
            return None
        head = (alarms[0]["run_width"], alarms[0]["run"], alarms[0]["asof"])
        mine = {p["index"]: p for p in alarms if (p["run_width"], p["run"], p["asof"]) == head}
        last = [i for i, p in mine.items() if not p["more"]]
        if not last or set(mine) != set(range(last[0] + 1)):
            return None
        live = [e for i in sorted(mine) for e in mine[i]["live"]]
        tombs = [e for i in sorted(mine) for e in mine[i]["tombs"]]
        p0 = mine[0]
        return {"run_width": p0["run_width"], "run": p0["run"], "asof": p0["asof"], "time": p0["time"],
                "live": live, "tombs": tombs, "pages": len(mine)}

    def run_hex(self) -> str | None:
        s = self.snapshot()
        if s is None:
            return None
        return f"{s['run']:0{s['run_width'] // 4}x}"


def collection_read(session: Any, address: str, timeout: float = 1.0, prefix: str = a2.ALARMS) -> Collection:
    """S4's GET of the owner's alarms, a collection read (§2.12: "a
    collection read of its owner's ``zk2/<system>/<service>/alarms.v2/state/
    alarms/**``"): target All, consolidation None, every reply kept as it
    came, each page read."""
    import zenoh

    from . import snapshot, zk2c

    col = Collection(f"zk2/{address}/{IFACE}/state/{prefix}/**", prefix)
    q: queue.Queue = queue.Queue()
    done = object()
    session.get(col.selector, zenoh.handlers.Callback(q.put, lambda: q.put(done)),
                target=zenoh.QueryTarget.ALL, consolidation=zenoh.ConsolidationMode.NONE, timeout=timeout)
    while True:
        try:
            item = q.get(timeout=timeout + 5.0)
        except queue.Empty:
            break
        if item is done:
            col.finished = True
            break
        if item.ok is None:
            col.errors.append(item.err.payload.to_bytes().decode("utf-8", "replace"))
            continue
        key, raw = str(item.ok.key_expr), item.ok.payload.to_bytes()
        col.replies.append((key, raw))
        if key == f"zk2/{address}/{IFACE}/state/{prefix}":
            try:
                col.pages.append(a2.read_page(raw))
                col.page_bytes.append(raw)
            except (snapshot.Refused, zk2c.Refused) as e:
                col.refused.append(getattr(e, "token", str(e)))
    return col


@dataclass
class SlotJudged:
    """One slot as a reader reads it: its record, its name, and §2.14's
    answers."""

    slot: int
    record: dict[str, Any] | None
    named: a2.Named | None
    firing: Any
    stale: Any
    confirmation: str | None = None

    def shown(self) -> str:
        if self.record is None or self.named is None:
            return "no record"
        if self.named.unnamed is not None:
            return f"{a2.numbers_of(self.record)}, unnamed ({self.named.unnamed})"
        if self.named.rule is not None:
            return f"{self.named.rule} {self.named.labels} {self.named.summary!r}" + \
                (f" by words, {self.named.alert_key}" if self.named.by_words else "")
        return f"by words, {self.named.alert_key}"


@dataclass
class OwnerReading:
    """One reading of one owner (§2.14, §2.15): its presence, the major its
    descriptor lists, the run it states, the revisions it names, its table,
    the collection read and its verdict, and each slot judged."""

    address: str
    presence: str
    major: str | None = None
    run: str | None = None
    named: list[str] | None = None
    table: a2.Table = a2.NO_TABLE
    collection: Collection | None = None
    verdict: Any = None
    slots: dict[int, SlotJudged] = field(default_factory=dict)
    descriptor: dict[str, Any] | None = None
    words: dict[int, Any] = field(default_factory=dict)
    #: §2.7 (0.2): the alarms' horizon in the revision the owner names, how
    #: the reader had that revision (:func:`horizon_of`), and its fingerprint
    horizon_s: int | None = None
    revision: str = "not_named"
    revision_fp: str | None = None

    def owner_read(self) -> v1.OwnerRead:
        return v1.OwnerRead(self.verdict.verdict,
                            tuple(v1.AlarmRead(j.firing.answer, None if j.record is None else j.record.get("severity"))
                                  for j in self.slots.values()))


def _presence(tool: Any, address: str, timeout: float) -> tuple[str, dict[str, Any] | None]:
    from . import live

    pres = live.list_presence(tool, f"zk2/{address}/@zk/**")
    if not pres.instances:
        return ("absent" if pres.complete else "incomplete"), None
    d = live.get_descriptor(tool, f"zk2/{address}/@zk/instance/{pres.instances[0]['instance']}", timeout=timeout)
    if len(d) == 1 and d[0].ok:
        return "present", json.loads(d[0].payload)
    return "present", None


#: freshness.v1's reason for a horizon a reader cannot read (its §2.7 step 4,
#: ``judgements.json``: "no horizon in the value: unobservable,
#: not_a_horizon"), zk2py's for a revision not in hand (text 0.2, §2.7)
NO_REVISION = ("unobservable", "not_a_horizon")


def ttl_of(canonical: dict[str, Any] | None) -> int | None:
    """The alarms' ``freshness.ttl_s`` in a revision's canonical form, or
    None where it states none that is a horizon."""
    for r in (canonical or {}).get("resources") or []:
        if r.get("token") == "state" and r.get("template") == "alarms/{slot}":
            t = (r.get("annotations") or {}).get(fr.KEY)
            return t if isinstance(t, int) and not isinstance(t, bool) and t > 0 else None
    return None


def horizon_of(tool: Any, named: list[str] | None, held: dict[str, Any], *, retrieve: bool = True,
               timeout: float = 1.0, across: bool = False) -> tuple[int | None, str, str | None]:
    """§2.7 (0.2), "The horizon is the revision's": the alarms' horizon in
    the ``alarms.v2`` revision ``named`` holds (the descriptor's, or across a
    face the deployment's word), as (seconds or None, how it was had, its
    fingerprint): ``held``, ``retrieved`` (core §8.4, never across a face),
    ``given``, ``standard``, ``not_in_hand`` or ``not_named``. A fingerprint
    whose canonical form is held but is another interface's names no alarms
    revision.

    ``across`` (0.2's mend, F-622): "Across a face, the deployment's word
    names the revision (§2.11): the alarms.v2 revision a reader is given,
    else the standard one, which every reader holds." A word that names an
    alarms.v2 revision names it, held or not in hand. One that names none
    is the revision the reader was given beside the standard (``given``),
    else the standard one (``standard``). Two given and none named is no
    word zk2py can read: ``not_named``."""
    from . import live

    fps = [x for x in named or [] if isinstance(x, str)]
    for fp in fps:
        c = held.get(fp)
        if c is not None and c.get("interface") == IFACE:
            return ttl_of(c), "held", fp
    unknown = [fp for fp in fps if fp not in held]
    if not unknown and across:
        std = a2.standard()
        given = [(fp, c) for fp, c in held.items() if c.get("interface") == IFACE and fp != std.fingerprint]
        if len(given) == 1:
            return ttl_of(given[0][1]), "given", given[0][0]
        if not given:
            return ttl_of(std.canonical), "standard", std.fingerprint
        return None, "not_named", None
    if not unknown:
        return None, "not_named", None
    if retrieve and tool is not None:
        for fp in unknown:
            got = live.retrieve_bundle(tool, IFACE, fp, timeout)
            if got.available and got.verified is not None:
                return ttl_of(got.verified.contract), "retrieved", fp
    return None, "not_in_hand", unknown[0]


def live_fp(desc: dict[str, Any]) -> str | None:
    """The ``alarms.v2`` revision a descriptor names (core §3.3, §8.4)."""
    from . import live

    return live.fingerprint_of(desc, IFACE)


def read_owner(tool: Any, address: str, held: dict[str, Any] | None = None, *, listener: Listener | None = None,
               timeout: float = 1.0, across: str | None = None, named: list[str] | None = None,
               lost: bool = False, gap: bool = False, now_ns: int | None = None,
               epoch: int = 0) -> OwnerReading:
    """One reading of ``address`` by a reader of this text. ``held`` maps a
    fingerprint to a revision the reader holds (its canonical form): the
    instance's table is read from those its descriptor names (§4), across a
    face (``across`` ``granted`` or ``denied``) from ``named``, the
    deployment's word. ``lost`` and ``gap``: what its subscription saw since
    the read (§2.15 step 3). Each slot's freshness is its snapshot entry's
    confirmation, or ``listener``'s observation where it follows the owner."""
    from . import heartbeat

    if across is not None:
        presence, desc = "across_face", None
    else:
        presence, desc = _presence(tool, address, timeout)
    r = OwnerReading(address, presence, descriptor=desc)
    lists = None
    if desc is not None:
        ifs = [e.get("iface") for e in desc.get("interfaces") or []]
        r.major = a2.major_of(ifs)
        r.run = desc.get("run")
        r.named = [e.get("contract") for e in desc.get("interfaces") or [] if e.get("contract")]
        lists = {"lists": r.major == IFACE}
    elif across is not None:
        r.major = IFACE
        r.named = named
    # the reader holds the standard revision, as it holds this text
    std = a2.standard()
    held = {std.fingerprint: std.canonical, **(held or {})}
    r.table = a2.instance_table(r.named, held)
    if desc is not None:
        mine = [fp for fp in [live_fp(desc)] if fp is not None]
        r.horizon_s, r.revision, r.revision_fp = horizon_of(tool, mine, held, timeout=timeout)
    elif across is not None:
        r.horizon_s, r.revision, r.revision_fp = horizon_of(tool, r.named, held, retrieve=False, across=True)
    face = None if across is None else {"reachable": across == "granted"}
    col = None
    if (presence == "present" and lists is not None and lists["lists"]) or (across == "granted"):
        col = collection_read(tool, address, timeout)
    r.collection = col
    if col is None:
        read = None
    else:
        snap = col.snapshot()
        read = a2.Read2(True, col.complete_read, snap is not None, col.run_hex())
    runs = [r.run] if r.run else []
    r.verdict = a2.established(presence, lists, face, read, lost=lost, gap=gap, runs=runs)
    held_whole = r.verdict.verdict == v1.ESTABLISHED
    snap = None if col is None else col.snapshot()
    now = time.time_ns() // 1_000_000 if now_ns is None else now_ns // 1_000_000
    words_col: Collection | None = None
    for e in (snap or {}).get("live", []):
        if e["slot"] is None:
            continue
        rec = e["record"] if "undecodable" not in e["record"] else None
        entry_run = f"{e['run']:0{e['run_width'] // 4}x}" if "run" in e else col.run_hex()  # type: ignore[union-attr]
        if e["confirmation"] == "time":
            conf_ms = heartbeat.instant_of(snap["time"], epoch)
        elif e["confirmation"] == "conf":
            conf_ms = heartbeat.instant_of(e.get("conf"), epoch)
        else:
            conf_ms = None
        if r.horizon_s is None:
            # §2.7 (0.2): "A revision not in hand gives no horizon ... never
            # at 900 s"
            fv = NO_REVISION
        elif listener is not None:
            v = fr.judge("state", fr.horizon("state", {fr.KEY: r.horizon_s}), [listener.observation(e["slot"])])
            fv = (v.verdict, v.reason)
        elif conf_ms is None:
            fv = ("unobservable", "no_observation")
        else:
            age_s = (now - conf_ms) / 1000
            fv = ("fresh", "within_horizon") if age_s <= r.horizon_s else ("stale", "beyond_horizon")
        latest = None if rec is None else rec["state"]
        if e["record"] and "undecodable" in e["record"]:
            latest = "undecodable"
        prev = bool(r.run) and entry_run is not None and across is None and col.run_hex() not in (r.run,)  # type: ignore[union-attr]
        rd = a2.Reading(presence, lists, face, fv, latest, False, prev, held_whole, False)
        named_ = None if rec is None else a2.name_record(r.table, rec)
        if rec is not None and a2.by_words(rec) and across is None:
            # §2.4 "A reader takes words by the key": the slot's words, read
            # from the words' snapshot, taken only where they derive its key.
            if words_col is None:
                words_col = collection_read(tool, address, timeout, a2.WORDS_PREFIX)
            ws = words_col.snapshot()
            doc = next((w["words"] for w in (ws or {}).get("live", []) if w["slot"] == e["slot"]), None)
            t = a2.take_words(a2.Read(rec), doc)
            r.words[e["slot"]] = (t, doc)
            if t.taken and doc is not None:
                named_ = a2.Named(rule=doc["rule"], labels=doc.get("labels") or {}, summary=doc["summary"],
                                  category=doc.get("category"), alert_key=named_.alert_key if named_ else None,
                                  by_words=True)
        r.slots[e["slot"]] = SlotJudged(e["slot"], rec, named_, a2.judge_reading(rd, v1.Q_FIRING),
                                        a2.judge_reading(rd, v1.Q_STALE), e["confirmation"])
    return r


def any_firing(at_least: str, readings: list[Any], scope_whole: bool = True) -> dict[str, Any]:
    """"Is anything firing at or above *S*?" over owners of either major
    (§2.17: "Its answers, its reasons and its counts are one set for
    both"): each an :class:`OwnerReading` or an ``alarms.v1``
    :class:`zk2py.alarms.OwnerRead`."""
    owners = [x.owner_read() if isinstance(x, OwnerReading) else x for x in readings]
    return a2.any_firing(at_least, owners, scope_whole)

"""desired.v2 on a bus (``spec/profiles/desired/v2.md``, text 0.1, core
0.112): zk2py's author and target, written from the text (#609, round 118),
on the scenarios' contracts (``scenarios-v2.md``, "The contracts",
transcribed in ``impl/python/interop/desired-v2/``).

- :class:`Author` owns ``tasking.v2``'s ``tasks/{system}/{service}`` on
  zk2py's :class:`zk2py.owner.Owner`. Its desired resource "carries no Zenoh
  timestamp, and its author answers a GET by snapshot" (§2.4): the owner
  reads it so from the contract's ``uses`` (core 0.112). A document is put
  only when its content changes. Its record holds "each document as it was
  put, by its key, and each delete it made within its tombstone window with
  when it made it ... It holds no stamp" (§2.11). A new instance, by a
  restart or a re-mint, "takes a new run", makes its deletes again and puts
  its documents again before its tokens. It says, for each GET of a slice,
  "the held stamp it carried, or none, and whether it answered with that
  stamp alone" (scenarios-v2).
- :class:`Target` owns ``worker.v2``: its packed report on the resource its
  ``desired.report`` names, and what it runs. It subscribes, then seeds
  (§2.5); each read of its slice is a conditional read while it holds a
  version, one at a time, waiting at least a face's reply timeout; it
  re-reads on a period, or watches its author's heartbeat
  (:class:`zk2py.desired_v2.Watch`); it judges each version with
  :class:`zk2py.desired_v2.TargetRole`, acts through the scenarios' drive,
  and puts its report before its tokens and after each change (§2.10).

**Across a face** is the target's word here, and no router's: a target told
so reads no presence, and waits the reply timeout it is given. No
``link.v2`` policy stands between the two in zk2py's runs.
"""

from __future__ import annotations

import json
import os
import sys
import threading
import time
from pathlib import Path
from typing import Any

import zenoh

from . import desired, desired_v2 as d2, heartbeat, live, seed_held, zk2c
from .contract import load_contract
from .owner import Owner, OwnerRefused

REPO = Path(__file__).resolve().parents[3]
CONTRACTS = REPO / "impl" / "python" / "interop" / "desired-v2"
WATCHED = REPO / "impl" / "python" / "interop" / "desired-v2-watched"
TEMPLATE = "tasks/{system}/{service}"
AUTHOR_IFACE, TARGET_IFACE, ROLE = "tasking.v2", "worker.v2", "task"
BASELINE = {"revision": 0, "mode": "idle", "rate_hz": 1}
RATE_SUPPORTED = 100
#: scenarios-v2: "`rate_hz` above 100 is `invalid` at `/rate_hz`, index 2,
#: with line 0. A member named `endpoint` is `reach` at `/endpoint`, index 3,
#: with line 1"; §4's Expected 6, "at `/revision`: `pointer` 4 and `detail`
#: 2".
LINE_RATE = "above what the drive supports"
LINE_REACH = "names how the worker reaches the bus"
LINE_REVISION = "a revision above 4,294,967,295"

canonical = desired.canonical_json


def contract(name: str, watched: bool = False) -> Any:
    return load_contract((WATCHED if watched else CONTRACTS) / f"{name}.toml", spec_dir=REPO / "spec")


def slice_key(author: str, target: str) -> str:
    system, service = target.split("/")
    return desired.slice_key(author, AUTHOR_IFACE, False, TEMPLATE, {"system": system, "service": service})


def version_of(ib: Any) -> d2.Version | None:
    return None if ib is None else d2.Version.of(ib.width, ib.run, ib.seq)


def inband_of(v: d2.Version | None) -> Any:
    return None if v is None else zk2c.InBand(v.run.width, v.run.value, v.seq)


# -- the author -----------------------------------------------------------------------------------

class Author:
    """§2.11's author of ``tasks/{system}/{service}``, on ``tasking.v2``."""

    def __init__(self, address: str, endpoint: str, *, record: str | None = None, run_record: str | None = None,
                 window_s: float = 60.0, tasking: Any = None, watched: bool = False,
                 tokenless: tuple[str, ...] = (), catalog: str | None = None,
                 owner_heartbeat: bool = False) -> None:
        self.address, self.endpoint = address, endpoint
        self.tokenless = set(tokenless)
        #: core §8.6's catalog, by its file's path, and whether the author
        #: confirms its interface by owner heartbeats (freshness.v2 §2.4)
        self.catalog, self.owner_heartbeat = catalog, owner_heartbeat
        self.record_path = None if record is None else Path(record)
        #: core §4.3: "A 16-bit run is a counter in a record the owner keeps
        #: ... An owner that cannot keep such a record draws 32-bit runs."
        self.run_record = run_record
        self.window_s = window_s
        self.contract = tasking or contract("tasking.v2", watched)
        self.documents: dict[str, dict[str, Any]] = {}
        self.deletes: dict[str, dict[str, Any]] = {}
        self.adoption = {"found": "nothing", "reput": [], "redeleted": [], "expired": []}
        #: each state GET its queryable received: (selector, the held stamp
        #: it carried or None, whether it was answered with that stamp alone)
        self.gets_log: list[tuple[str, str | None, bool]] = []
        self.lock = threading.RLock()
        self.owner: Owner | None = None

    def start(self) -> str:
        system, service = self.address.split("/")
        self.owner = Owner(system, service, [self.contract], connect=self.endpoint, tombstone_s=self.window_s,
                           before_tokens=self._adopt, run_record=self.run_record, tokenless=self.tokenless)
        owner = self.owner
        if self.catalog is not None:
            from . import catalog as cat

            owner.catalog = cat.read(Path(self.catalog).read_bytes())
            owner.owner_heartbeats = True if self.owner_heartbeat else None

        def answered(query: Any) -> None:
            from . import held as held_stamp

            named = None
            try:
                payload = None if query.payload is None else query.payload.to_bytes()
                named = version_of(held_stamp.read_get(query.encoding, payload))
            except zk2c.Refused:
                pass
            answer = owner.state_answer(query)
            alone = any(item[0] == "held" for item in answer)
            with self.lock:
                self.gets_log.append((str(query.key_expr), None if named is None else named.text, alone))
            owner.answer_state(query, answer)

        owner.state_hold = answered
        owner.start()
        return owner.instance_key

    def key(self, target: str) -> str:
        return slice_key(self.address, target)

    def gets(self, target: str) -> list[dict[str, Any]]:
        """scenarios-v2, "What a section observes of a read": each GET that
        reached ``target``'s slice."""
        key = zenoh.KeyExpr(self.key(target))
        with self.lock:
            log = list(self.gets_log)
        return [{"held": held, "alone": alone} for sel, held, alone in log if zenoh.KeyExpr(sel).intersects(key)]

    # -- §2.11: the record ---------------------------------------------------

    def _read_record(self) -> dict[str, Any] | None:
        if self.record_path is None or not self.record_path.is_file():
            return None
        try:
            rec = desired.strict_json(self.record_path.read_bytes(), exact=True)
        except (OSError, desired.NotStrict):
            return None
        return rec if isinstance(rec, dict) and rec.get("profile") == d2.PROFILE else None

    def _write_record(self) -> None:
        """§2.11 "What the record holds": each document as it was put, by
        its key, and each delete within the window with when it was made. No
        stamp."""
        if self.record_path is None:
            return
        tmp = self.record_path.with_suffix(".tmp")
        tmp.write_bytes(canonical({"profile": d2.PROFILE,
                                   "documents": {t: {"document": e["document"]} for t, e in self.documents.items()},
                                   "deletes": {t: {"at_ms": e["at_ms"]} for t, e in self.deletes.items()}}))
        os.replace(tmp, self.record_path)

    def _again(self, owner: Owner, docs: dict[str, Any], dels: dict[str, Any], adopting: bool) -> None:
        """§2.11 "A new instance takes a new run ... An author MUST put again
        each document it keeps under its new run before it answers a GET
        with it, and MUST delete again each key its record holds a delete of
        within its window". The deletes first, each "for what is left of the
        window counted from the delete it repeats" (v1 §2.11)."""
        now_ms = time.time() * 1000
        self.documents, self.deletes = {}, {}
        for target, d in sorted(dels.items()):
            at_ms = d.get("at_ms") if isinstance(d, dict) else None
            if isinstance(at_ms, desired.Exact):
                at_ms = at_ms.double
            if isinstance(at_ms, bool) or not isinstance(at_ms, (int, float)):
                continue  # a delete it cannot date is dropped, never dated now
            at_ms = min(float(at_ms), now_ms)
            elapsed_s = (now_ms - at_ms) / 1000
            if elapsed_s >= self.window_s:
                if adopting:
                    self.adoption["expired"].append(target)
                continue  # v1 §2.11: "its record SHOULD drop a delete once that window has passed"
            stamp = owner.delete_state(self.key(target), elapsed_s=elapsed_s)
            self.deletes[target] = {"version": version_of(stamp).text, "at_ms": int(at_ms)}
            if adopting:
                self.adoption["redeleted"].append(target)
        for target, e in sorted(docs.items()):
            if not isinstance(e, dict) or "document" not in e:
                continue
            stamp = owner.set_state(self.key(target), canonical(e["document"]))
            self.documents[target] = {"version": version_of(stamp).text, "document": e["document"]}
            if adopting:
                self.adoption["reput"].append(target)
        self._write_record()

    def _adopt(self, owner: Owner) -> None:
        rec = self._read_record()
        if rec is None:
            return
        self.adoption["found"] = "record"
        self._again(owner, rec["documents"] if isinstance(rec.get("documents"), dict) else {},
                    rec["deletes"] if isinstance(rec.get("deletes"), dict) else {}, True)

    def put(self, target: str, document: Any) -> tuple[str, str, str | None]:
        """§2.11 "Only on a change": ("put", key, version), or ("unchanged",
        key, None)."""
        assert self.owner is not None
        key = self.key(target)
        with self.lock:
            held = self.documents.get(target)
            if held is not None and desired.jcs_equal(held["document"], document):
                return "unchanged", key, None
            v = version_of(self.owner.set_state(key, canonical(document)))
            self.documents[target] = {"version": v.text, "document": document}
            self.deletes.pop(target, None)
            self._write_record()
        return "put", key, v.text

    def delete(self, target: str, force: bool = False) -> tuple[str, str, str | None]:
        """A delete of the target's document. ``force`` makes one "though it
        holds no member there" (scenarios-v2 §15 step 2)."""
        assert self.owner is not None
        key = self.key(target)
        with self.lock:
            if target not in self.documents and not force:
                return "none", key, None
            v = version_of(self.owner.delete_state(key))
            self.documents.pop(target, None)
            self.deletes[target] = {"version": v.text, "at_ms": int(time.time() * 1000)}
            self._write_record()
        return "deleted", key, v.text

    def new_epoch(self) -> str:
        """A re-mint in its own session: "A new instance takes a new run
        (core §4.3), by a restart or a re-mint" (§2.11), and puts again what
        it keeps under it, before its new tokens."""
        assert self.owner is not None
        owner = self.owner
        with self.lock:
            docs, dels = dict(self.documents), dict(self.deletes)
            owner.new_epoch(before=lambda: self._again(owner, docs, dels, False))
        return owner.instance_key

    def close(self) -> None:
        if self.owner is not None:
            self.owner.close()


# -- the target -----------------------------------------------------------------------------------

class Target:
    """§2.5 to §2.10's target of ``tasks/{system}/{service}``, on
    ``worker.v2``."""

    def __init__(self, address: str, endpoint: str, *, author: str = "lab/author", unbound: bool = False,
                 presence_watch: bool = False, reread_ms: int | None = None, hold: bool = False, poll_ms: int = 500,
                 timeout: float = 1.0, across: bool = False, reply_timeout_ms: int | None = None, watch: bool = False,
                 moves: bool = True, worker: Any = None, tasking: Any = None, watched: bool = False,
                 run_record: str | None = None, catalog: str | None = None) -> None:
        self.address, self.endpoint, self.author = address, endpoint, author
        #: the catalogs the target holds (core §8.6), and the id its author
        #: names, read once it is needed
        self.catalogs: list[Any] = []
        if catalog is not None:
            from . import catalog as cat

            self.catalogs = [cat.read(Path(catalog).read_bytes())]
        self._named: str | None = None
        self.unbound, self.presence_watch = unbound, presence_watch
        self.reread_s = None if reread_ms is None else reread_ms / 1000
        self.poll_s = poll_ms / 1000
        #: §2.5 "Across a face. A target across a constrained face that
        #: states a reply timeout (link.v2 §2.20) MUST wait at least that
        #: long for its seed and for each re-read."
        self.across = across
        self.timeout = timeout if reply_timeout_ms is None else max(timeout, reply_timeout_ms / 1000)
        self.watching, self.moves = watch, moves
        self.contract = worker or contract("worker.v2")
        self.author_contract = tasking or contract("tasking.v2", watched)
        self.run_record = run_record
        self.task_schema = json.loads((CONTRACTS / "tasking.json").read_text())
        self.key = None if unbound else slice_key(author, address)
        self.rp = f"zk2/{address}/{TARGET_IFACE}/state/desired/task"
        self.run = f"zk2/{address}/{TARGET_IFACE}/state/running"
        self.role = d2.TargetRole()
        self.watch = d2.Watch()
        self.lock = threading.RLock()
        self._seed_due = False
        self.running_doc: Any = dict(BASELINE)
        self.acts = 0
        self.reads = 0
        #: each read of the slice: (why, the stamp it carried or None, what
        #: it drew)
        self.read_log: list[tuple[str, str | None, str]] = []
        #: each heartbeat heard: its key's class, and the watch's decision
        self.heartbeat_log: list[tuple[str, str]] = []
        self._hold_next = hold
        self._release = threading.Event()
        self.held = False
        self._fail_next = False
        self._last_report: bytes | None = None
        self._stop = threading.Event()
        self._entities: list[Any] = []
        self.watch_session: Any = None
        self.owner: Owner | None = None
        self.layout: Any = None
        self.resource = f"state/{TEMPLATE}"

    # -- the start --------------------------------------------------------------

    def _declarations(self) -> None:
        """What a target reads of the two contracts before it starts (§2.1,
        §2.2, §2.10). An invalid one refuses the start, as an invalid
        binding does."""
        c = self.contract.canonical
        resources = [d2.ReportResource(r["token"], r["template"], r.get("annotations") or {},
                                       r["type"].get("kind") == "packed" and r["type"].get("name") == d2.REPORT_TYPE
                                       and d2.is_published(r["type"].get("schema")))
                     for r in c["resources"]]
        naming = [r for r in resources if d2.names_role(r, ROLE)]
        if len(naming) != 1:
            raise OwnerRefused(f"role {ROLE!r}: {len(naming)} report resources name it (desired.v2 §2.10)")
        decl = d2.declare_report(naming[0], set(c.get("requires") or {}), resources)
        if decl.declaration != desired.DECLARED:
            raise OwnerRefused(f"role {ROLE!r}: its report resource is invalid ({', '.join(decl.reasons)})")
        self.rp = d2.report_key(self.address, c["interface"], naming[0], ROLE)
        self.role = d2.TargetRole(decl.pointers, decl.details)
        if self.unbound:
            return
        a = self.author_contract.canonical
        rr = (c.get("requires") or {}).get(ROLE) or {}
        named = desired.desired_named([{"kind": r.get("kind"), "template": r.get("template"),
                                        "annotations": r.get("annotations") or {}} for r in a["resources"]],
                                      {"resources": rr.get("resources")})
        if len(named) >= 2:
            raise OwnerRefused(f"role {ROLE!r}: its binding is invalid (two_documents) (desired.v1 §2.1, §2.3)")
        for r in a["resources"]:
            if r["template"] != TEMPLATE:
                continue
            decl2 = d2.declare(r["token"], r["template"], r.get("annotations") or {}, r.get("params") or {}, a["uses"])
            if decl2.declaration != desired.DECLARED:
                raise OwnerRefused(f"the author's desired resource is invalid ({', '.join(decl2.reasons)}) "
                                   "(desired.v2 §2.2)")
        if self.reread_s is not None and self.reread_s < self.timeout:
            # §2.5 "A period below the timeout is refused at the target's
            # start, as an invalid binding is (v1 §2.1). Such a target would
            # always hold a read outstanding."
            raise OwnerRefused(f"a period of {self.reread_s:g} s is below the read's timeout, {self.timeout:g} s "
                               "(desired.v2 §2.5)")

    def start(self) -> str:
        self._declarations()
        system, service = self.address.split("/")
        bound = not self.unbound
        self.owner = Owner(system, service, [self.contract], connect=self.endpoint,
                           bindings={ROLE: [self.author]} if bound else {},
                           params={ROLE: {"system": "self.system", "service": "self.service"}} if bound else {},
                           before_tokens=self._first_puts, run_record=self.run_record)
        self.owner.start()
        if not bound:
            return self.owner.instance_key
        s = self.owner.session
        assert s is not None
        # §2.5, v1 §2.5 "Subscribe, then seed"
        self._entities.append(s.declare_subscriber(self.key, zenoh.handlers.Callback(self._live)))
        if self.watching:
            self._watch_heartbeats(s)
        if self.across:
            # v1 §2.14: "Presence does not cross ..., so a target across a
            # face never sees its author's instance token appear. It seeds
            # at its start"
            threading.Thread(target=self.read, args=("seed",), daemon=True).start()
        elif self.presence_watch:
            self._follow_presence()
        else:
            threading.Thread(target=self._poll, daemon=True).start()
        if self.reread_s is not None:
            threading.Thread(target=self._reread_loop, daemon=True).start()
        return self.owner.instance_key

    def _first_puts(self, owner: Owner) -> None:
        """§2.10: "A target MUST put its first report of each role before
        its tokens (core §8.2), `source` `baseline`"."""
        owner.set_state(self.run, canonical(self.running_doc))
        self._put_report(owner)

    def _put_report(self, owner: Owner | None = None) -> None:
        owner = owner or self.owner
        assert owner is not None
        with self.lock:
            data = d2.write_report(self.role.report())
            if data == self._last_report:
                return
            self._last_report = data
            owner.set_state(self.rp, data)

    # -- the author's presence (v1 §2.5) ------------------------------------------

    def _instance_selector(self) -> str:
        return f"zk2/{self.author}/@zk/instance/*"

    def _follow_presence(self) -> None:
        self.watch_session = live.open_client(self.endpoint)

        def token(sample: Any) -> None:
            if sample.kind == zenoh.SampleKind.PUT:
                threading.Thread(target=self.read, args=("appearance",), daemon=True).start()

        self._entities.append(live.declare_liveliness_subscriber(self.watch_session, self._instance_selector(),
                                                                 token, history=True))
        live.settle(self.watch_session, 1.0)
        threading.Thread(target=self.read, args=("seed",), daemon=True).start()

    def _poll(self) -> None:
        self.read("seed")
        seen: set[str] | None = None
        while not self._stop.wait(self.poll_s if seen is not None else 0):
            assert self.owner is not None and self.owner.session is not None
            p = live.list_presence(self.owner.session, self._instance_selector(), 1.0)
            if p.kind != live.COMPLETE and not p.instances:
                continue
            now = {i["instance"] for i in p.instances}
            if seen is not None and now - seen:
                threading.Thread(target=self.read, args=("appearance",), daemon=True).start()
            seen = now if p.kind == live.COMPLETE else seen | now if seen is not None else now

    def _reread_loop(self) -> None:
        while self.reread_s is not None and not self._stop.wait(self.reread_s):
            self.read("period")

    def reread(self, ms: int | None) -> None:
        if ms is not None and ms / 1000 < self.timeout:
            raise ValueError(f"a period of {ms} ms is below the read's timeout (desired.v2 §2.5)")
        start = self.reread_s is None and ms is not None
        self.reread_s = None if ms is None else ms / 1000
        if start and not self.unbound:
            threading.Thread(target=self._reread_loop, daemon=True).start()

    # -- reading (§2.5) ----------------------------------------------------------

    def read(self, why: str) -> str:
        """One read of the slice. "One read at a time. A target MUST NOT read
        its slice while a read of it is outstanding. A seed that falls due
        meanwhile is made when that read ends." A period's read that finds
        one outstanding is not made: its next trigger is its next period."""
        if self.unbound or self.owner is None or self.owner.session is None:
            return "unbound"
        with self.lock:
            if not self.watch.reading():
                if why in ("seed", "appearance"):
                    self._seed_due = True
                    return "waits"
                return "outstanding"
        return self._reads(why)

    def _reads(self, why: str) -> str:
        """The read the watch's flag already stands for, then the seed that
        fell due while it was out."""
        while True:
            word = self._read_once(why)
            with self.lock:
                if not self._seed_due or self._stop.is_set():
                    return word
                self._seed_due = False
                if not self.watch.reading():
                    return word
            why = "seed"

    def _read_once(self, why: str) -> str:
        """A member read of the slice's one concrete key (core §4.2),
        conditional "while it holds a version": "The stamp it sends is that
        of the latest version it admitted"."""
        assert self.owner is not None
        session = self.owner.session
        with self.lock:
            sent = self.role.ask()
        try:
            st = live.get_state(session, self.key, self.timeout, held=inband_of(sent))
        except Exception as e:  # noqa: BLE001 - a read that fails is no verdict
            with self.lock:
                self.role.asked = None
                self.watch.no_answer()
                self.read_log.append((why, None if sent is None else sent.text, f"failed: {type(e).__name__}"))
            return "failed"
        self.reads += 1
        mine = [r for r in st.replies if r.key == self.key and r.refusal is None]
        no_entry = [h for h in st.snapshots if h.key == self.key and h.done
                    and not any(p["live"] or p["tombs"] for p in h.pages.values())]
        unchanged = [h for h in st.held if h[2] == seed_held.UNCHANGED]
        if unchanged and sent is not None:
            # "Answered unchanged, by the held stamp it sent, the target
            # judges nothing. ... it confirms a last-known one (§2.9)."
            with self.lock:
                word = self.role.unchanged(sent)
                self.watch.unchanged(self.role.stamp_to_send() == sent)
                self.read_log.append((why, sent.text, "unchanged"))
            if word == d2.CONFIRMED:
                self._put_report()
            return "unchanged"
        if mine or no_entry:
            # "Answered whole, by a snapshot, the target judges the entry as
            # §2.6 says."
            with self.lock:
                self.role.asked = None
                self.watch.whole()
                drew = "entry" if mine else "no_entry"
                self.read_log.append((why, None if sent is None else sent.text, drew))
            for r in mine:
                self._receive(self._sample(desired.OWNER, r.deleted, r.inband, r.payload), False)
            for h in no_entry if not mine else []:
                self._receive_no_entry(d2.NoEntry(d2.Run(h.header["run_width"], h.header["run"]), h.header["asof"]))
            return drew
        # "A held stamp it did not send is no answer (core §4.2, held)"; "No
        # reply at all is still no verdict (core O5)"; "A read that ends at
        # its timeout is no verdict".
        with self.lock:
            self.role.asked = None
            self.watch.no_answer()
            self.read_log.append((why, None if sent is None else sent.text, "held" if st.held else "silent"))
        return "silent"

    @staticmethod
    def _sample(origin: str, deleted: bool, ib: Any, payload: bytes) -> d2.Sample:
        if deleted:
            return d2.Sample(origin, True, version_of(ib))
        return d2.Sample(origin, False, version_of(ib), desired.Payload.of_bytes(payload, True),
                         desired.revision_of(payload))

    # -- versions (§2.6, §2.7, §2.8) --------------------------------------------------

    def _live(self, sample: Any) -> None:
        key = str(sample.key_expr)
        if "*" in key or "$" in key:
            return  # R6
        read = zk2c.read_sample(sample)
        if read.refused:
            return
        # "No envelope: a sample whose payload carries no in-band stamp ...
        # ignores it (unstamped)": its stamp reads None, and the role says so.
        self._receive(self._sample(desired.OWNER, sample.kind == zenoh.SampleKind.DELETE,
                                   read.stamp if read.enveloped else None,
                                   read.record if read.record is not None else b""), True)

    def _receive(self, s: d2.Sample, heard_live: bool) -> None:
        with self.lock:
            before = self.role.waiting if (self.role.running and self.role.waiting is not None) else self.role.current
            word = self.role.receive(s)
            if (heard_live and self.watching and self.moves and s.version is not None
                    and word in (d2.ACT, d2.REVERT, d2.ADOPTED, d2.HELD)):
                # §2.5 "Its own change costs nothing."
                old = None if before is None or before.version is None else (before.version, before.delete)
                self.watch.own_change(self.key, old, (s.version, s.delete))
        self._after(word)

    def _receive_no_entry(self, n: d2.NoEntry) -> None:
        with self.lock:
            word = self.role.receive_no_entry(n)
        self._after(word)

    def _after(self, word: str) -> None:
        if word in (d2.ACT, d2.REVERT):
            threading.Thread(target=self._drive, daemon=True).start()
        elif word in (d2.ADOPTED, d2.CONFIRMED):
            self._put_report()

    # -- the watch (§2.5) -----------------------------------------------------------

    def _watch_heartbeats(self, s: Any) -> None:
        """"A target subscribes to both forms' keys, as a freshness.v2
        subscriber does": the heartbeat covering its author's interface, and
        the owner heartbeat key of its collection's class (freshness.v2
        §2.5, "one for each class that the collections it follows have").
        It reads each "with the author's revision"."""
        from . import freshness_v2 as f2

        self.layout = heartbeat.layout_of(self.author_contract.canonical, self.author_contract.fingerprint)
        system, service = self.author.split("/")
        hb = heartbeat.heartbeat_key(system, service, AUTHOR_IFACE)
        self._entities.append(s.declare_subscriber(hb, zenoh.handlers.Callback(self._heartbeat)))
        cls = f2.classes_of(self.author_contract.canonical)[self.resource]
        self._entities.append(s.declare_subscriber(
            f2.owner_heartbeat_key(system, service, cls),
            zenoh.handlers.Callback(lambda smp, c=cls: self._owner_heartbeat(c, smp))))

    def _catalog_named(self) -> str | None:
        """The catalog the author names: its descriptor's (core §3.3), or
        across a face, where no descriptor is read, the one the target was
        provisioned (core §8.6)."""
        if self._named is None and self.catalogs:
            if self.across:
                self._named = self.catalogs[0].id
            else:
                from . import freshness_v2 as f2

                assert self.owner is not None and self.owner.session is not None
                self._named = f2.descriptor_catalog(self.owner.session, self.author)
        return self._named

    def _owner_heartbeat(self, cls: str, sample: Any) -> None:
        """§2.5: "A group of an owner heartbeat is read as a heartbeat is,
        its sequence the owner heartbeat key's. A group whose sections
        freshness.v2 §2.5 does not read is a heartbeat not read. An owner
        heartbeat with no group for the author's interface has no section."
        With no catalog it is one "in a group whose catalog it does not
        hold": counted, never read, and "the deployment's period stands"."""
        from . import catalog as cat, owner_heartbeat as oh

        if sample.kind != zenoh.SampleKind.PUT:
            return
        if not self.catalogs:
            with self.lock:
                self.heartbeat_log.append((cls, "no_catalog"))
            return
        why, section, stamp = None, None, None
        try:
            put = oh.read(sample.payload.to_bytes(), enveloped=zk2c.envelope_encoding(str(sample.encoding)))
            stamp = put.stamp
        except (oh.Refused, zk2c.Refused, heartbeat.Refused):
            why = "refused"
        if why is None:
            named = self._catalog_named()
            mine, unresolved = None, False
            for g in put.body.groups:
                try:
                    entry = cat.resolve(self.catalogs, named, g.index).entry
                except cat.Refused:
                    unresolved = True  # this group may be the author's interface's: it cannot tell
                    continue
                if entry is not None and entry[0] == AUTHOR_IFACE:
                    mine = (g, entry)
            if mine is None:
                why = "no_catalog" if unresolved else None  # else: no group for the interface, no section
            else:
                g, entry = mine
                if entry[1] != self.layout.fingerprint or g.tag != self.layout.tag:
                    why = "revision"
                else:
                    try:
                        sec = oh.read_sections(self.layout, g.bits).get(self.resource)
                        if sec is not None:
                            section = d2.Section(int(sec["digest"], 16), sec["live"])
                    except oh.Refused:
                        why = "refused"
        with self.lock:
            if why is not None or stamp is None:
                word = self.watch.heartbeat(d2.Run(16, 0), 0, None, read=False, key=cls)
            else:
                word = self.watch.heartbeat(d2.Run(stamp.width, stamp.run), stamp.seq, section, key=cls)
            self.heartbeat_log.append((cls, word if why is None else f"{word}:{why}"))
        if word == d2.READ:
            threading.Thread(target=self._reads, args=("heartbeat",), daemon=True).start()

    def _heartbeat(self, sample: Any) -> None:
        if sample.kind != zenoh.SampleKind.PUT:
            return
        payload = sample.payload.to_bytes()
        section: d2.Section | None = None
        stamp = None
        read = True
        try:
            env = heartbeat.put_of(payload, "stream", zk2c.envelope_encoding(str(sample.encoding)))
            stamp = env.stamp
            if env.tag != self.layout.tag:
                read = False  # "A heartbeat of another revision (revision)"
            else:
                sec = heartbeat.read(self.layout, env.record or b"")["sections"].get(self.resource)
                if sec is not None:
                    section = d2.Section(int(sec["digest"], 16), sec["live"])
        except (heartbeat.Refused, zk2c.Refused):
            read = False  # "a refused one (refused)"
        with self.lock:
            if not read or stamp is None:
                word = self.watch.heartbeat(d2.Run(16, 0), 0, None, read=False)
            else:
                word = self.watch.heartbeat(d2.Run(stamp.width, stamp.run), stamp.seq, section)
            self.heartbeat_log.append(("heartbeat", word))
        if word == d2.READ:
            # "It makes one conditional read of its slice"
            threading.Thread(target=self._reads, args=("heartbeat",), daemon=True).start()

    # -- the drive (scenarios-v2, "The target": "Its baseline and its drive are v1's") ---

    def _drive(self) -> None:
        while True:
            with self.lock:
                h = self.role.current
                self.acts += 1
                hold, self._hold_next = self._hold_next, False
            if hold:
                self.held = True
                self._release.wait()
                self._release.clear()
                self.held = False
            assert h is not None
            fin = self._act(h)
            with self.lock:
                word = self.role.finish(fin)
            self._put_report()
            if word not in (d2.ACT, d2.REVERT):
                if word == d2.ADOPTED:
                    self._put_report()
                return

    def _set_running(self, doc: Any) -> None:
        assert self.owner is not None
        self.running_doc = doc
        self.owner.set_state(self.run, canonical(doc))

    def _act(self, h: Any) -> d2.Finish:
        try:
            with self.lock:
                fail, self._fail_next = self._fail_next, False
            if h.delete:  # a delete, a tombstone, or a snapshot with no entry
                if fail:
                    self._set_running({**self.running_doc, "rate_hz": BASELINE["rate_hz"]})
                    return d2.Finish(d2.FAILED, "apply")
                self._set_running(dict(BASELINE))
                return d2.Finish(d2.DONE)
            if h.payload is None or h.payload.value is None:
                return d2.Finish(d2.REFUSED, "undecodable")
            if h.revision is not None and h.revision > d2.MAX_REVISION:
                # §2.4: "A target MUST refuse whole a document whose revision
                # is above 4,294,967,295, with the code invalid."
                return d2.Finish(d2.REFUSED, "invalid", "/revision", LINE_REVISION)
            doc = desired.exact_json(h.payload.value)
            ptr = desired.refusal_pointer(doc, self.task_schema["$defs"]["Task"], self.task_schema)
            if ptr is not None:
                return d2.Finish(d2.REFUSED, "invalid", ptr)
            if "endpoint" in doc:
                return d2.Finish(d2.REFUSED, "reach", "/endpoint", LINE_REACH)
            if doc["rate_hz"] > RATE_SUPPORTED:
                return d2.Finish(d2.REFUSED, "invalid", "/rate_hz", LINE_RATE)
            if fail:
                self._set_running({**self.running_doc, "rate_hz": doc["rate_hz"]})
                return d2.Finish(d2.FAILED, "apply")
            self._set_running(h.payload.value)
            return d2.Finish(d2.DONE)
        except Exception as e:  # noqa: BLE001 - v1 §2.7: an act whose end is unknown is failed, apply
            print(f"zk2py desired.v2 target: the act was lost: {e!r}", file=sys.stderr, flush=True)
            return d2.Finish(d2.FAILED, "apply")

    # -- knobs ------------------------------------------------------------------------

    def hold(self) -> None:
        with self.lock:
            self._hold_next = True

    def release(self) -> None:
        self._release.set()

    def fail_next(self) -> None:
        with self.lock:
            self._fail_next = True

    def report(self) -> dict[str, Any]:
        with self.lock:
            return self.role.report()

    def watch_says(self) -> dict[str, Any]:
        """scenarios-v2: "The target says what its watch expects"."""
        with self.lock:
            e, last = self.watch.expects, self.watch.last
            words: dict[str, int] = {}
            by_key: dict[str, dict[str, int]] = {}
            for k, w in self.heartbeat_log:
                words[w] = words.get(w, 0) + 1
                by_key.setdefault(k, {})[w] = by_key.setdefault(k, {}).get(w, 0) + 1
            return {"expects": None if e is None else {"digest": format(e.digest, "08x"), "live": e.live},
                    "last": None if last is None else [last[0].text, last[1]],
                    "outstanding": self.watch.outstanding, "heartbeats": words, "by_key": by_key,
                    "reads": [{"why": w, "held": s, "drew": d} for w, s, d in self.read_log]}

    def close(self) -> None:
        self._stop.set()
        self._release.set()
        for e in self._entities:
            try:
                e.undeclare()
            except Exception:  # noqa: BLE001 - closing anyway
                pass
        if self.watch_session is not None:
            live.close(self.watch_session)
        if self.owner is not None:
            self.owner.close()

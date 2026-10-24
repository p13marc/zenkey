"""desired.v1 on a bus (``spec/profiles/desired/v1.md``, text 0.3, core 0.37):
zk2py's author and target, written from the text alone (#609, round 55), on
the scenarios' contracts (``scenarios.md``, "The contracts", transcribed in
``impl/python/interop/desired/``).

- :class:`Author` owns ``tasking.v1``'s ``tasks/{system}/{service}`` on
  zk2py's :class:`zk2py.owner.Owner`: a document put only when its content
  changes (§2.11), deterministically encoded; its record (a JSON file, the
  owner example's format), read at start before the tokens, each delete
  within its window made again for what is left of it, then each document
  put again, the clock caught up past every stamp the record holds (§2.11,
  core §4.3); a re-mint in its own session puts again what it keeps before
  its new tokens.
- :class:`Target` owns ``worker.v1`` (its report ``desired/task`` and what
  it runs, ``running``), its role ``task`` bound to its author with
  ``{system} = self.system`` and ``{service} = self.service`` (§2.3), or
  unbound, the kill switch (§2.12). It subscribes to its slice first, then
  seeds by S4's GET (§2.5); it follows its author's instance token live on a
  watch session (core §8.1, 0.37) or by reading it on a period, seeds again
  at each appearance (and up to four times more when a seed then draws
  nothing), reads an archive only when a seed draws nothing and a complete
  presence read shows its author absent (§2.5, §2.9, 0.3), judges every
  version with :class:`zk2py.desired.TargetRole`, acts through the
  scenarios' simulated drive, and puts its report before its tokens and
  after every change (§2.10).
"""

from __future__ import annotations

import json
import math
import os
import sys
import threading
import time
from pathlib import Path
from typing import Any

import zenoh

from . import alarms, desired, live
from .contract import load_contract
from .owner import Owner

REPO = Path(__file__).resolve().parents[3]
CONTRACTS = REPO / "impl" / "python" / "interop" / "desired"
TEMPLATE = "tasks/{system}/{service}"
#: scenarios.md: "Its baseline is {"revision": 0, "mode": "idle", "rate_hz": 1}".
BASELINE = {"revision": 0, "mode": "idle", "rate_hz": 1}
#: The drive: "The worker supports rate_hz up to 100".
RATE_SUPPORTED = 100


def canonical(doc: Any) -> bytes:
    """§2.11: "encode each document deterministically (JSON's members in one
    order …)", every number by its exact value (:func:`desired.canonical_json`)."""
    return desired.canonical_json(doc)


def read_document(text: str) -> Any:
    """A document as a command writes it: strict JSON (§2.10's reading), its
    numbers exact. Raises :class:`desired.NotStrict`."""
    return desired.strict_json(text.encode("utf-8", "surrogatepass"), exact=True)


def contract(name: str) -> Any:
    return load_contract(CONTRACTS / f"{name}.toml", spec_dir=REPO / "spec")


def slice_key(author: str, target: str) -> str:
    """§2.3: the target's slice of its author's template."""
    system, service = target.split("/")
    return desired.slice_key(author, "tasking.v1", False, TEMPLATE, {"system": system, "service": service})


def raw_time(stamp: str) -> int:
    return int(stamp.split("/", 1)[0])


def _record_time(stamp: Any) -> int | None:
    """A record entry's stamp as its raw NTP64 time, or None when it does
    not read as one (``<time>/<id>``)."""
    if not isinstance(stamp, str):
        return None
    try:
        t = raw_time(stamp)
    except ValueError:
        return None
    return t if 0 <= t < 1 << 64 else None


def _record_ms(at_ms: Any) -> float | None:
    """A record delete's ``at_ms``, when it made the delete, in Unix
    milliseconds: a finite number (an :class:`desired.Exact` by its double),
    or None for anything else, null, a boolean, a string or a missing
    member. §2.11 (0.3): the record holds "each delete it made within its
    tombstone window with when it made it"; one it cannot date it cannot
    place within its window, so it is dropped, never dated now, which would
    answer it for a window more."""
    if isinstance(at_ms, desired.Exact):
        at_ms = at_ms.double
    if isinstance(at_ms, bool) or not isinstance(at_ms, (int, float)):
        return None
    try:
        v = float(at_ms)
    except OverflowError:
        return None
    return v if math.isfinite(v) else None


def _ms_text(at_ms: float) -> int | float:
    return int(at_ms) if at_ms.is_integer() else at_ms


def ntp64_at_least(raw: int) -> zenoh.NTP64:
    """An NTP64 at or just above a raw value: zenoh-python builds one from
    seconds and nanoseconds, which cannot hit every raw value, so the
    nanoseconds round up."""
    secs, frac = raw >> 32, raw & 0xFFFFFFFF
    nanos = -(-(frac * 1_000_000_000) // (1 << 32))
    if nanos >= 1_000_000_000:
        secs, nanos = secs + 1, nanos - 1_000_000_000
    return zenoh.NTP64(secs, nanos)


def _sample(origin: str, deleted: bool, stamp_text: str | None, payload: bytes) -> desired.Sample:
    stamp = None if stamp_text is None else desired.Stamp.parse(stamp_text)
    if deleted:
        return desired.Sample(origin, True, stamp)
    return desired.Sample(origin, False, stamp, desired.Payload.of_bytes(payload, True), desired.revision_of(payload))


# -- the author ---------------------------------------------------------------------------

class Author:
    """§2.11's author of ``tasks/{system}/{service}``."""

    def __init__(self, address: str, endpoint: str, *, record: str | None = None, window_s: float = 60.0,
                 tokenless: tuple[str, ...] = ()) -> None:
        self.address, self.endpoint = address, endpoint
        self.record_path = None if record is None else Path(record)
        self.window_s, self.tokenless = window_s, set(tokenless)
        self.contract = contract("tasking.v1")
        self.documents: dict[str, dict[str, Any]] = {}
        self.deletes: dict[str, dict[str, Any]] = {}
        #: record entries a restart or a re-mint could not write again: kept
        #: in the record as they were, so that the next one tries again,
        #: until a put or a delete of the target replaces them
        self.carried: dict[str, dict[str, dict[str, Any]]] = {"documents": {}, "deletes": {}}
        self.adoption = {"found": "nothing", "reput": [], "redeleted": [], "expired": [], "failed": []}
        #: the state GETs its queryables received (scenarios.md §8), and each
        #: one's selector (``desired gets``, 0.4)
        self.queries = 0
        self.selectors: list[str] = []
        self.lock = threading.Lock()
        self.owner: Owner | None = None

    def start(self) -> str:
        system, service = self.address.split("/")
        self.owner = Owner(system, service, [self.contract], connect=self.endpoint, tombstone_s=self.window_s,
                           before_tokens=self._adopt, tokenless=self.tokenless)
        owner = self.owner

        def counted(query: Any) -> None:
            with self.lock:
                self.queries += 1
                self.selectors.append(str(query.key_expr))
            owner.answer_state(query, owner.state_answer(query))

        owner.state_hold = counted
        owner.start()
        return owner.instance_key

    def key(self, target: str) -> str:
        return slice_key(self.address, target)

    def gets(self, target: str) -> tuple[str, int]:
        """scenarios.md (0.4, F-197): "an author how many GETs reached a
        target's slice": the state GETs its queryables answered whose
        selector reaches ``target``'s document key."""
        key = zenoh.KeyExpr(self.key(target))
        with self.lock:
            sels = list(self.selectors)
        return str(key), sum(1 for x in sels if zenoh.KeyExpr(x).intersects(key))

    def _key_of(self, target: str) -> str | None:
        """A record's target name as its key, or None for one that names no
        ``system/service`` (a record edited by hand)."""
        try:
            return self.key(target)
        except ValueError:
            return None

    # §2.11 "A restart from its record"
    def _read_record(self) -> dict[str, Any] | None:
        if self.record_path is None or not self.record_path.is_file():
            return None
        try:
            rec = desired.strict_json(self.record_path.read_bytes(), exact=True)
        except (OSError, desired.NotStrict):
            return None
        return rec if isinstance(rec, dict) and rec.get("profile") == "desired.v1" else None

    def _write_record(self) -> None:
        if self.record_path is None:
            return
        docs = {**self.carried["documents"], **self.documents}
        dels = {t: d for t, d in {**self.carried["deletes"], **self.deletes}.items() if t not in docs}
        tmp = self.record_path.with_suffix(".tmp")
        tmp.write_bytes(canonical({"profile": "desired.v1", "documents": docs, "deletes": dels}))
        os.replace(tmp, self.record_path)

    def _uncarry(self, target: str) -> None:
        self.carried["documents"].pop(target, None)
        self.carried["deletes"].pop(target, None)

    @staticmethod
    def _log(text: str) -> None:
        print(f"zk2py desired author: {text}", file=sys.stderr, flush=True)

    def _failed(self, target: str, why: str, adopting: bool) -> None:
        self._log(f"{target}: {why}")
        if adopting:
            self.adoption["failed"].append(target)

    def _adopt(self, owner: Owner) -> None:
        """Before its first write and before its tokens: its record's stamps
        catch its clock up (core §4.3), its deletes within their window are
        made again for what is left of it, then its documents are put again
        (§2.11: "MUST put each document it keeps again … before it answers a
        GET with it, and SHOULD before its new instance token").

        Every entry stands alone: one that does not read is dropped and
        logged, one whose write fails is logged and carried in the record,
        and neither stops the rest or the tokens."""
        rec = self._read_record()
        if rec is None:
            return
        self.adoption["found"] = "record"
        docs = rec["documents"] if isinstance(rec.get("documents"), dict) else {}
        dels = rec["deletes"] if isinstance(rec.get("deletes"), dict) else {}
        times = [t for e in list(docs.values()) + list(dels.values()) if isinstance(e, dict)
                 for t in [_record_time(e.get("stamp"))] if t is not None]
        if times:
            owner.catch_up(ntp64_at_least(max(times)))
        now_ms = time.time() * 1000
        for target, d in sorted(dels.items()):
            at_ms = _record_ms(d.get("at_ms")) if isinstance(d, dict) else None
            key = self._key_of(target)
            if at_ms is None or key is None:
                self._failed(target, "a delete its record does not date (at_ms): dropped, never answered "
                             "for a window counted from now (§2.11)" if key is not None
                             else "no target's name: dropped", True)
                continue
            # one dated after now (the wall clock stepped back) counts from
            # now, and is kept so: its window is never answered twice
            at_ms = min(at_ms, now_ms)
            elapsed_s = (now_ms - at_ms) / 1000
            if elapsed_s >= self.window_s:
                self.adoption["expired"].append(target)
                continue
            try:
                # §2.11 (0.3): "for what is left of the window counted from
                # the delete it repeats, not a window more"
                stamp = owner.delete_state(key, elapsed_s=elapsed_s)
            except Exception as e:  # noqa: BLE001 - one entry never stops the rest
                self._failed(target, f"its delete again failed ({e}): carried in the record", True)
                self.carried["deletes"][target] = d
                continue
            self.deletes[target] = {"stamp": str(stamp), "at_ms": _ms_text(at_ms)}
            self.adoption["redeleted"].append(target)
        for target, e in sorted(docs.items()):
            key = self._key_of(target)
            if key is None or not isinstance(e, dict) or "document" not in e:
                self._failed(target, "a document entry with no document: dropped" if key is not None
                             else "no target's name: dropped", True)
                continue
            try:
                stamp = owner.set_state(key, canonical(e["document"]))
            except Exception as ex:  # noqa: BLE001
                self._failed(target, f"its put again failed ({ex}): carried in the record", True)
                self.carried["documents"][target] = e
                continue
            self.documents[target] = {"stamp": str(stamp), "document": e["document"]}
            self.adoption["reput"].append(target)
        self._write_record()

    def put(self, target: str, document: Any) -> tuple[str, str, str | None]:
        """§2.11 "Only on a change": ("put", key, stamp), or ("unchanged",
        key, None) for the document held."""
        assert self.owner is not None
        key = self.key(target)
        with self.lock:
            held = self.documents.get(target)
            if held is not None and desired.jcs_equal(held["document"], document):
                return "unchanged", key, None
            stamp = self.owner.set_state(key, canonical(document))
            self.documents[target] = {"stamp": str(stamp), "document": document}
            self.deletes.pop(target, None)
            self._uncarry(target)
            self._write_record()
        return "put", key, str(stamp)

    def delete(self, target: str) -> tuple[str, str, str | None]:
        assert self.owner is not None
        key = self.key(target)
        with self.lock:
            if target not in self.documents and target not in self.carried["documents"]:
                return "none", key, None
            stamp = self.owner.delete_state(key)
            self.documents.pop(target, None)
            self._uncarry(target)
            self.deletes[target] = {"stamp": str(stamp), "at_ms": int(time.time() * 1000)}
            self._write_record()
        return "deleted", key, str(stamp)

    def new_epoch(self) -> str:
        """A re-mint in its own session (core §8.1; §2.11 0.3 "Its zid": it
        keeps its zid, its re-put a later time): each document put again,
        each delete within its window made again, before the new tokens."""
        assert self.owner is not None
        with self.lock:
            now_ms = time.time() * 1000
            for target, d in sorted(self.deletes.items()):
                at_ms = _record_ms(d.get("at_ms"))
                if at_ms is None:
                    self._failed(target, "a delete with no time it was made (at_ms): dropped (§2.11)", False)
                    del self.deletes[target]
                    continue
                at_ms = min(at_ms, now_ms)
                d["at_ms"] = _ms_text(at_ms)
                elapsed_s = (now_ms - at_ms) / 1000
                if elapsed_s >= self.window_s:
                    # §2.11 (0.3): "its record SHOULD drop a delete once that
                    # window has passed"
                    del self.deletes[target]
                    continue
                try:
                    d["stamp"] = str(self.owner.delete_state(self.key(target), elapsed_s=elapsed_s))
                except Exception as e:  # noqa: BLE001
                    self._failed(target, f"its delete again failed ({e}): kept in the record", False)
            for target, e in sorted(self.documents.items()):
                try:
                    e["stamp"] = str(self.owner.set_state(self.key(target), canonical(e["document"])))
                except Exception as ex:  # noqa: BLE001
                    # this instance does not hold it, so a put of it is no "unchanged"
                    self._failed(target, f"its put again failed ({ex}): carried in the record", False)
                    self.carried["documents"][target] = self.documents.pop(target)
            self._write_record()
        self.owner.new_epoch()
        return self.owner.instance_key

    def close(self) -> None:
        if self.owner is not None:
            self.owner.close()


# -- the target ---------------------------------------------------------------------------

def schema_pointer(doc: Any, schema: dict[str, Any], root: dict[str, Any]) -> str | None:
    """Where a JSON Schema refuses a decoded document, or None when it
    accepts it: §2.7's rules (0.4, F-193), :func:`desired.refusal_pointer`."""
    return desired.refusal_pointer(doc, schema, root)


class Target:
    """§2.5 to §2.10's target of ``tasks/{system}/{service}``, on
    ``worker.v1``."""

    def __init__(self, address: str, endpoint: str, *, author: str = "lab/author", unbound: bool = False,
                 archive: str | None = None, watch: bool = False, reread_ms: int | None = None,
                 hold: bool = False, poll_ms: int = 500, timeout: float = 1.0) -> None:
        self.address, self.endpoint, self.author = address, endpoint, author
        self.unbound, self.archive, self.watch_mode = unbound, archive, watch
        self.reread_s = None if reread_ms is None else reread_ms / 1000
        self.poll_s, self.timeout = poll_ms / 1000, timeout
        self.contract = contract("worker.v1")
        self.task_schema = json.loads((CONTRACTS / "tasking.json").read_text())
        self.key = None if unbound else slice_key(author, address)
        self.role = desired.TargetRole(self.key)
        self.lock = threading.RLock()
        #: seeds coalesced: one runs, at most one waits (``_again``, an
        #: appearance's if any request it stands for was one), and a request
        #: while one waits is that one
        self._seed_flags = threading.Lock()
        self._seeding = False
        self._again: bool | None = None
        self.running_doc: Any = dict(BASELINE)
        self.acts = 0
        self.seeds = 0
        #: what each seed found: "answered", "silent", "archive", "not made"
        self.seed_log: list[str] = []
        #: §2.5's "a target says so where it says what its seed found" (0.4,
        #: F-197): the last seed's replies, its author's presence (read when
        #: the GET drew nothing and it decides the archive or a retry), and
        #: its archive read
        self.last_seed: dict[str, Any] | None = None
        self._hold_next = hold
        self._release = threading.Event()
        self.held = False
        self._fail_next = False
        self._refuse_next: tuple[str, str | None] | None = None
        self._last_report: bytes | None = None
        self._stop = threading.Event()
        self._entities: list[Any] = []
        self.watch_session: Any = None
        self.owner: Owner | None = None
        self.rp = f"zk2/{address}/worker.v1/state/desired/task"
        self.run = f"zk2/{address}/worker.v1/state/running"

    # -- bring-up --------------------------------------------------------
    def start(self) -> str:
        system, service = self.address.split("/")
        bound = not self.unbound
        self.owner = Owner(system, service, [self.contract], connect=self.endpoint,
                           bindings={"task": [self.author]} if bound else {},
                           params={"task": {"system": "self.system", "service": "self.service"}} if bound else {},
                           before_tokens=self._first_puts)
        self.owner.start()
        if bound:
            s = self.owner.session
            assert s is not None
            # §2.5 "Subscribe, then seed"
            self._entities.append(s.declare_subscriber(self.key, zenoh.handlers.Callback(self._live)))
            if self.watch_mode:
                self._follow_watch()
            else:
                threading.Thread(target=self._poll, daemon=True).start()
            if self.reread_s is not None:
                threading.Thread(target=self._reread_loop, daemon=True).start()
        return self.owner.instance_key

    def _first_puts(self, owner: Owner) -> None:
        """§2.10: "MUST put its first report of each role before its tokens …,
        source baseline"; what it runs, its baseline."""
        owner.set_state(self.run, canonical(self.running_doc))
        self._put_report(owner)

    def _put_report(self, owner: Owner | None = None) -> None:
        owner = owner or self.owner
        assert owner is not None
        with self.lock:
            data = canonical(self.role.report())
            if data == self._last_report:
                return
            self._last_report = data
        owner.set_state(self.rp, data)

    # -- presence (§2.5, core §8.1) ----------------------------------------
    def _instance_selector(self) -> str:
        return f"zk2/{self.author}/@zk/instance/*"

    def _follow_watch(self) -> None:
        """A watch session of its own, one round trip there, then the seed."""
        self.watch_session = live.open_client(self.endpoint)

        def token(sample: Any) -> None:
            if sample.kind == zenoh.SampleKind.PUT:
                threading.Thread(target=self.seed, kwargs={"appearance": True}, daemon=True).start()

        self._entities.append(live.declare_liveliness_subscriber(self.watch_session, self._instance_selector(),
                                                                 token, history=True))
        live.settle(self.watch_session, self.timeout)
        threading.Thread(target=self.seed, daemon=True).start()

    def _poll(self) -> None:
        """§2.5 (0.3): "One that does not reads its author's instance tokens on
        a period, and an appearance is a read showing a token the read before
        it did not." It seeds at once, then reads every period."""
        self.seed()
        seen: set[str] | None = None
        while not self._stop.wait(self.poll_s if seen is not None else 0):
            p = self.presence()
            if p.kind != live.COMPLETE and not p.instances:
                continue
            now = {i["instance"] for i in p.instances}
            if seen is not None and now - seen:
                threading.Thread(target=self.seed, kwargs={"appearance": True}, daemon=True).start()
            seen = now if p.kind == live.COMPLETE else seen | now if seen is not None else now

    def presence(self) -> live.Presence:
        """A presence read on the target's own session, which holds no
        liveliness subscriber (core §8.1, 0.37)."""
        assert self.owner is not None and self.owner.session is not None
        return live.list_presence(self.owner.session, self._instance_selector(), self.timeout)

    def author_absent(self) -> bool:
        """§2.5 (0.3, F-187): "a complete read without its instance token"."""
        return self.author_presence()[0] == "absent"

    def author_presence(self) -> tuple[str, str]:
        """The author's presence, by one read (core §8.1, 0.37, 0.38):
        ``present``, ``absent`` (a complete read without its token),
        ``incomplete`` (possibly incomplete, a read cut off while it ran
        among them) or ``not-made``, each with why where there is one."""
        p = self.presence()
        if p.kind == live.NOT_MADE:
            return "not-made", p.not_made or ""
        if p.instances:
            return "present", ""  # a read cannot invent a token
        if p.kind == live.COMPLETE:
            return "absent", ""
        return "incomplete", p.reading

    def _reread_loop(self) -> None:
        while self.reread_s is not None and not self._stop.wait(self.reread_s):
            self.seed()

    # -- seeds (§2.5) ------------------------------------------------------
    def seed(self, appearance: bool = False) -> str:
        """A seed, coalesced: while one runs, a request leaves one to run
        after it, and a request while that one waits is dropped, folded
        into it ("coalesced"); so appearances never pile up a thread each
        behind a seed's GETs. The one that runs after starts after every
        request it stands for, so it reads at least as late as each would
        have."""
        if self.unbound or self.owner is None or self.owner.session is None:
            return "unbound"
        with self._seed_flags:
            if self._seeding:
                self._again = appearance or bool(self._again)
                return "coalesced"
            self._seeding = True
        try:
            while True:
                word = self._seed_once(appearance)
                with self._seed_flags:
                    if self._again is None:
                        self._seeding = False
                        return word
                    appearance, self._again = self._again, None
        except BaseException:
            with self._seed_flags:
                self._seeding, self._again = False, None
            raise

    def _seed_once(self, appearance: bool) -> str:
        """S4's GET of the slice; at an appearance, made again up to four
        times, 100 ms more apart each time, while its author reads present
        (§2.5, 0.3); an archive read when it draws nothing and a complete
        presence read shows its author absent (§2.9, 0.3)."""
        if self.owner is None or self.owner.session is None:
            return "unbound"  # closed while it waited
        self.seeds += 1
        tries = 5 if appearance else 1
        seen: tuple[str, str] = ("not-read", "")
        for n in range(tries):
            st = live.get_state(self.owner.session, self.key, self.timeout)
            if st.replies:
                for r in st.replies:
                    self._receive(_sample(desired.OWNER, r.deleted, r.stamp, r.payload))
                self.seed_log.append("answered")
                self.last_seed = {"answered": len(st.replies), "author": "not-read", "archive": "-", "why": ""}
                return "answered"
            if n + 1 < tries:
                seen = self.author_presence()
                if seen[0] == "absent":
                    break
                time.sleep(0.1 * (n + 1))
        archive = "-"
        if self.archive is not None:
            if seen[0] != "absent":
                seen = self.author_presence()
            if seen[0] == "absent":
                form = alarms.archive_form(self.archive, self.key)
                st = live.get_state(self.owner.session, form, self.timeout)
                for r in st.replies:
                    self._receive(_sample(desired.ARCHIVE, r.deleted, r.stamp, r.payload))
                archive = "answered" if st.replies else "nothing"
        self.last_seed = {"answered": 0, "author": seen[0], "archive": archive, "why": seen[1]}
        if archive != "-":
            self.seed_log.append("archive" if archive == "answered" else "silent")
            return "archive"
        self.seed_log.append("silent")
        return "silent"

    def reread(self, ms: int | None) -> None:
        start = self.reread_s is None and ms is not None
        self.reread_s = None if ms is None else ms / 1000
        if start and not self.unbound:
            threading.Thread(target=self._reread_loop, daemon=True).start()

    # -- versions (§2.6, §2.7) ---------------------------------------------
    def _live(self, sample: Any) -> None:
        key = str(sample.key_expr)
        if "*" in key or "$" in key:
            return  # R6
        ts = sample.timestamp
        self._receive(_sample(desired.OWNER, sample.kind == zenoh.SampleKind.DELETE,
                              None if ts is None else str(ts), sample.payload.to_bytes()))

    def _receive(self, s: desired.Sample) -> None:
        with self.lock:
            word = self.role.receive(s)
        if word in (desired.ACT, desired.REVERT):
            threading.Thread(target=self._drive, daemon=True).start()
        elif word in (desired.ADOPTED, desired.CONFIRMED):
            self._put_report()

    # -- the drive (scenarios.md, "The target") ----------------------------
    def _drive(self) -> None:
        while True:
            with self.lock:
                v = self.role.current
                self.acts += 1
                hold, self._hold_next = self._hold_next, False
            if hold:
                self.held = True
                self._release.wait()
                self._release.clear()
                self.held = False
            assert v is not None
            fin = self._act(v)
            with self.lock:
                word = self.role.finish(fin)
            self._put_report()
            if word not in (desired.ACT, desired.REVERT):
                return

    def _set_running(self, doc: Any) -> None:
        """What the drive runs, put as it was given, its numbers exact."""
        assert self.owner is not None
        self.running_doc = doc
        self.owner.set_state(self.run, canonical(doc))

    def _act(self, v: Any) -> desired.Finish:
        try:
            with self.lock:
                fail, self._fail_next = self._fail_next, False
                refuse, self._refuse_next = self._refuse_next, None
            if v.delete:
                if fail:
                    self._set_running({**self.running_doc, "rate_hz": BASELINE["rate_hz"]})
                    return desired.Finish(desired.FAILED, {"code": "apply", "detail": "the drive failed after the rate"})
                self._set_running(dict(BASELINE))
                return desired.Finish(desired.DONE)
            if v.payload is None or v.payload.value is None:
                return desired.Finish(desired.REFUSED, {"code": "undecodable", "detail": "not strict JSON"})
            doc = desired.exact_json(v.payload.value)
            ptr = schema_pointer(doc, self.task_schema["$defs"]["Task"], self.task_schema)
            if ptr is not None:
                return desired.Finish(desired.REFUSED, {"code": "invalid", "pointer": ptr,
                                                        "detail": "not a tasking.v1 Task"})
            if refuse is not None:
                code, pointer = refuse
                return desired.Finish(desired.REFUSED, {"code": code, **({"pointer": pointer} if pointer else {})})
            if "endpoint" in doc:
                return desired.Finish(desired.REFUSED, {"code": "reach", "pointer": "/endpoint",
                                                        "detail": "names the endpoint the worker reaches the bus by"})
            if doc["rate_hz"] > RATE_SUPPORTED:
                return desired.Finish(desired.REFUSED, {"code": "invalid", "pointer": "/rate_hz",
                                                        "detail": f"the worker supports rate_hz up to {RATE_SUPPORTED}"})
            if fail:
                self._set_running({**self.running_doc, "rate_hz": doc["rate_hz"]})
                return desired.Finish(desired.FAILED, {"code": "apply", "pointer": "/mode",
                                                       "detail": "the drive failed after setting the rate"})
            self._set_running(v.payload.value)
            return desired.Finish(desired.DONE)
        except Exception as e:  # noqa: BLE001 - §2.7 (0.3): an act whose end is unknown is failed, apply
            return desired.Finish(desired.FAILED, {"code": "apply", "detail": f"the act was lost: {e!r}"})

    # -- knobs --------------------------------------------------------------
    def new_epoch(self) -> str:
        """§2.7 (0.3): "A re-mint is not a restart. A target that mints a new
        instance id in its own process (core §1.5, §8.1) still runs what it
        applied, and keeps what it acted on and its report"."""
        assert self.owner is not None
        self.owner.new_epoch()
        return self.owner.instance_key

    def hold(self) -> None:
        with self.lock:
            self._hold_next = True

    def release(self) -> None:
        self._release.set()

    def fail_next(self) -> None:
        with self.lock:
            self._fail_next = True

    def refuse_next(self, code: str, pointer: str | None) -> None:
        with self.lock:
            self._refuse_next = (code, pointer)

    def report(self) -> dict[str, Any]:
        with self.lock:
            return self.role.report()

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

"""``spec/scenarios/state.md`` §14 at core 0.100 (#977 E3; #609, round 105),
live: a collection's snapshot, its pages, and a seed from it.
``python -m zk2py.snapshot_live [--pairing P]… [--only N]…``.

**The pairings.**
- ``py-py``: zk2py's owner (:class:`zk2py.owner.Owner`) and zk2py's readers:
  a tool's late joiner T (:class:`zk2py.seed.LateJoiner`, subscribed to
  ``plans/*`` before each GET, its collections mapped by the owner's
  descriptor, :class:`zk2py.seed_snapshot.ByDescriptor`), a consumer's late
  joiner on its plan, and S4's GET (:func:`zk2py.live.get_state`) beside a
  raw GET that keeps each page's bytes;
- ``rust-py``: the Rust owner example (a black box) and zk2py's readers;
- ``py-rust``: zk2py's owner and the Rust readers: ``zenctl get --raw`` (each
  page's bytes), ``zenctl get state`` (the reference's S4 GET and its seed,
  its rows and its ``snapshots``) and ``consume`` (one state GET).

**The contract** is ``interop/state/zk2py_fleet.v1.toml``: ``plans/{vehicle}``
(collection 1), ``oper`` (0, its own snapshot key) and ``{peer}/route`` (2, no
snapshot key), none a per-key resource.

**The owners' records.** zk2py's owner holds §14's 32 plans, ``plans/v00`` to
``plans/v30`` and ``plans/vehicle-01``, each a record of 110 bytes, so its
pages hold 8 entries each. The owner example's records are its own, ``ok``,
so it is filled with 200 plans (``fill``), three pages, and
``plans/vehicle-01`` beside them.

**Steps 3, 5 and 7** have "a session of the test's own" play the owner,
"writing the snapshots it is given": the owner's own answer, its pages taken
by a raw GET, played from a queryable of the test's. So a page is lost (step
3), and an owner's answer is held while it puts (step 5), whichever owner
made it. zk2py's owner holds its answer itself (``Owner.state_hold``).

**Windows.** zk2py's owner runs a tombstone window of 1 s (core S3: "unless
the deployment configures the owner with another"); the owner example keeps
its 60 s, so its steps 5 and 6 wait it out once.

Every listener is on a port the OS picked. Exit 0 when every check passes, 1
when any fails.
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
import threading
import time
from pathlib import Path
from typing import Any

from . import seed as sd
from . import seed_snapshot as ss
from . import snapshot, zk2c
from .blackbox import BLACKBOX

REPO = Path(__file__).resolve().parents[3]
FLEET = REPO / "impl" / "python" / "interop" / "state" / "zk2py_fleet.v1.toml"
IFACE = "zk2py_fleet.v1"
PAIRINGS = ("py-py", "rust-py", "py-rust")
#: zk2py's owner's window here (core S3), and the owner example's default
PY_WINDOW_S, RUST_WINDOW_S = 1.0, 60.0
PAGE_SIZE = 1024


class Report:
    def __init__(self) -> None:
        self.rows: list[tuple[str, str, bool, str]] = []

    def check(self, section: str, name: str, ok: bool, detail: str = "") -> bool:
        self.rows.append((section, name, ok, detail))
        print(f"{'PASS' if ok else 'FAIL'} [{section}] {name}" + (f": {detail}" if detail else ""), flush=True)
        return ok

    def note(self, section: str, text: str) -> None:
        print(f"NOTE [{section}] {text}", flush=True)


class Fleet:
    """One owner of ``zk2py_fleet.v1`` at ``ground/<service>``, zk2py's or the
    owner example, a client of ``ep``."""

    def __init__(self, impl: str, ep: str, service: str = "fleet", record: str | None = None) -> None:
        self.impl, self.ep, self.service = impl, ep, service
        self.base = f"zk2/ground/{service}/{IFACE}/state"
        self.address = f"ground/{service}"
        self.contract = _contract()
        if impl == "zk2py":
            from .owner import Owner

            # a run record, so 16-bit runs: §14's "at most 975 bytes" is a
            # 16-bit run's figure (F-504; a 32-bit run's page holding
            # plans/vehicle-01 is 977)
            import tempfile

            # a second instance of the service shares its record: "It moves
            # with the service", and no instance reuses another's run (§4.3)
            self.own_record = record is None
            if record is None:
                record = tempfile.NamedTemporaryFile(prefix="zk2py-run-", suffix=".json", delete=False).name
                Path(record).unlink()
            self.record = record
            self.o: Any = Owner("ground", service, [self.contract], connect=ep, tombstone_s=PY_WINDOW_S,
                                run_record=self.record)
            self.o.start()
            self.window_s = PY_WINDOW_S
        else:
            from .live_interop import Owner as Exe

            self.p = Exe(BLACKBOX / "owner", self.address, [FLEET], connect=ep, flags=[])
            if self.p.wait_for("ready ", 120) is None:
                raise RuntimeError(f"the owner example did not start: {self.p.stderr[-3:]}")
            self.window_s = RUST_WINDOW_S

    def command(self, line: str) -> str:
        got = self.p.command(line)
        if got is None or got.startswith("refused"):
            raise RuntimeError(f"{line!r}: {got!r}")
        return got

    def put(self, member: str, record: bytes) -> None:
        if self.impl == "zk2py":
            self.o.set_state(f"{self.base}/plans/{member}", record)
        else:
            self.command(f"put {IFACE} state/plans/{{vehicle}} {member}")

    def delete(self, member: str) -> None:
        if self.impl == "zk2py":
            self.o.delete_state(f"{self.base}/plans/{member}")
        else:
            self.command(f"delete {IFACE} state/plans/{{vehicle}} {member}")

    def setup(self) -> list[str]:
        """§14's plans: zk2py's 32 of 110 bytes; the owner example's 200 and
        ``vehicle-01``. Returns the members."""
        if self.impl == "zk2py":
            members = [f"v{i:02d}" for i in range(31)] + ["vehicle-01"]
            for m in members:
                self.put(m, (m.encode() * 40)[:110])
            return members
        self.command(f"fill {IFACE} state/plans/{{vehicle}} 200")
        self.put("vehicle-01", b"")
        return [f"m{i}" for i in range(200)] + ["vehicle-01"]

    closed = False

    def close(self) -> None:
        if self.closed:
            return
        self.closed = True
        if self.impl == "zk2py":
            self.o.close()
            if self.own_record:
                Path(self.record).unlink(missing_ok=True)
        else:
            self.p.close()


def _contract() -> Any:
    from .contract import load_contract

    return load_contract(FLEET)


def raw_get(session: Any, selector: str, consolidation: str = "none", timeout: float = 5.0
            ) -> list[tuple[str, bytes]]:
    """Every reply's key and payload, as it arrived."""
    import zenoh

    mode = zenoh.ConsolidationMode.LATEST if consolidation == "latest" else zenoh.ConsolidationMode.NONE
    out = []
    for rep in session.get(selector, target=zenoh.QueryTarget.ALL, consolidation=mode, timeout=timeout):
        if rep.ok is not None:
            out.append((str(rep.ok.key_expr), rep.ok.payload.to_bytes()))
    return out


class Player:
    """A session of the test's own playing the owner (state.md §14, steps 3,
    5, 7): a queryable over ``selector`` that holds each query until
    :meth:`answer`, which replies with the pages it is given and ends it."""

    def __init__(self, session: Any, selector: str) -> None:
        import zenoh

        self.lock = threading.Lock()
        self.queries: list[Any] = []
        self.taken = threading.Event()
        self.q = session.declare_queryable(selector, zenoh.handlers.Callback(self._take), complete=False)

    def _take(self, query: Any) -> None:
        with self.lock:
            self.queries.append(query)
        self.taken.set()

    def answer(self, pages: list[tuple[str, bytes]]) -> None:
        with self.lock:
            qs, self.queries = self.queries, []
        for q in qs:
            for k, b in pages:
                q.reply(k, b, encoding=zk2c.ENCODING)
            q.drop()

    def close(self) -> None:
        self.answer([])
        try:
            self.q.undeclare()
        except Exception:  # noqa: BLE001
            pass


def _joiner(tool: Any, f: Fleet, consumer: bool = False, sub: str | None = None, get: str | None = None,
            timeout: float = 5.0) -> sd.LateJoiner:
    lj = sd.LateJoiner(tool, sub or f"{f.base}/plans/*", consumer=consumer, timeout=timeout,
                       get_selector=get or f"{f.base}/plans/**", collections=ss.ByDescriptor([f.contract]))
    lj.start()
    lj.wait()
    return lj


def _plans(lj: sd.LateJoiner, f: Fleet) -> list[str]:
    pre = f"{f.base}/plans/"
    return sorted(k[len(pre):] for k, e in lj.view().items() if k.startswith(pre) and not e.deleted)


def _pages(answer: list[tuple[str, bytes]]) -> list[tuple[str, dict[str, Any], int]]:
    out = []
    for k, b in answer:
        try:
            out.append((k, ss.read_page_of(k, b), len(b)))
        except (snapshot.Refused, zk2c.Refused):
            out.append((k, {"refused": True}, len(b)))
    return out


def section14(report: Report, pairing: str) -> None:
    """§14, every step a pairing reaches."""
    from . import live
    from .live_interop import _r1

    impl = "rust" if pairing == "rust-py" else "zk2py"
    sec = f"{pairing} §14"
    r, ep, _ = _r1(timestamping=False)
    tool, helper, cons = live.open_client(ep), live.open_client(ep), live.open_client(ep)
    f: Fleet | None = None
    others: list[Any] = []
    try:
        f = Fleet(impl, ep)
        members = f.setup()
        time.sleep(0.5)
        if pairing == "py-rust":
            _py_rust(report, sec, f, ep, members)
            return
        _steps(report, sec, f, ep, tool, helper, cons, members, others)
    except Exception as e:  # noqa: BLE001 - a step that could not run is a failure
        import traceback

        report.check(sec, "the section ran", False, f"{type(e).__name__}: {e} {traceback.format_exc()[-600:]}")
    finally:
        for x in others:
            try:
                x.close()
            except Exception:  # noqa: BLE001
                pass
        if f is not None:
            f.close()
        for s in (tool, helper, cons):
            s.close()
        r.close()


def _steps(report: Report, sec: str, f: Fleet, ep: str, tool: Any, helper: Any, cons: Any, members: list[str],
           others: list[Any]) -> None:
    import zenoh

    from . import live

    n = len(members)
    coll_key = f"{f.base}/plans"
    # -- step 1: a collection read ------------------------------------------------
    answer = raw_get(helper, f"{coll_key}/**")
    pages = _pages(answer)
    idx = [p["index"] for _, p, _ in pages]
    want_pages = 4 if f.impl == "zk2py" else 3
    report.check(sec, f"expected 1: one snapshot on …/state/plans in {want_pages} pages, indices 0 to "
                      f"{want_pages - 1}, the last with its more bit 0, each within {PAGE_SIZE} bytes, all with one "
                      "run, asof and time",
                 len(pages) == want_pages and all(k == coll_key for k, _, _ in pages)
                 and sorted(idx) == list(range(want_pages))
                 and [p["more"] for _, p, _ in sorted(pages, key=lambda x: x[1]["index"])] == [True] * (want_pages - 1)
                 + [False]
                 and all(size <= PAGE_SIZE for _, _, size in pages)
                 and len({(p["run_width"], p["run"], p["asof"], json.dumps(p["time"])) for _, p, _ in pages}) == 1,
                 f"{[(k[len(f.base):], p.get('index'), p.get('more'), size, len(p.get('live', []))) for k, p, size in pages]}")
    if f.impl == "zk2py":
        report.check(sec, "expected 1: 8 entries a page, at most 975 bytes (a 16-bit run)",
                     [len(p["live"]) for _, p, _ in sorted(pages, key=lambda x: x[1]["index"])] == [8, 8, 8, 8]
                     and max(size for _, _, size in pages) <= 975 and pages[0][1]["run_width"] == 16,
                     str([(p["run_width"], size) for _, p, size in pages]))
    held = {f"{coll_key}/{m}" for m in members}
    seqs_ok = True
    if f.impl == "zk2py":
        for _, p, _ in pages:
            for e in p["live"]:
                h = f.o._held.get(f"{coll_key}/{e['id']}")  # noqa: SLF001 - the owner's own record of its puts
                seqs_ok = seqs_ok and h is not None and h.inband.seq == e["seq"] and e["confirmation"] == "none"
    else:
        seqs_ok = all(e["confirmation"] == "none" for _, p, _ in pages for e in p["live"])
    report.check(sec, "expected 1: each live entry's id a vehicle, its seq its put's, its confirmation 0",
                 seqs_ok and sorted(f"{coll_key}/{e['id']}" for _, p, _ in pages for e in p["live"]) == sorted(held),
                 f"{sum(len(p['live']) for _, p, _ in pages)} entries")
    t = _joiner(tool, f)
    others.append(t)
    report.check(sec, "expected 1: T holds the snapshot complete at its last page, before the GET's final reply, and "
                      f"holds the {n} plans",
                 t.completed == [(coll_key, 1, False)] and t.seed.complete and _plans(t, f) == sorted(members),
                 f"{t.completed}; {len(_plans(t, f))} plans; {t.seed.errors}")
    # -- step 2: member reads ---------------------------------------------------------
    one = members[7]
    st = live.get_state(helper, f"{coll_key}/{one}", timeout=5.0)
    h1 = [h for h in st.snapshots if h.key == f"{coll_key}/{one}"]
    report.check(sec, f"expected 2: plans/{one}, one snapshot on …/plans/{one}, one page whose one entry has the "
                      "empty id",
                 len(st.snapshots) == 1 and len(h1) == 1 and sorted(h1[0].pages) == [0]
                 and [e["id"] for e in h1[0].pages[0]["live"]] == [""] and h1[0].header["coll"] == 1,
                 str([(h.key, sorted(h.pages)) for h in st.snapshots]))
    t.read_again(selector=f"{coll_key}/*")
    star = [h for h in t.trackers[-1].heard]
    report.check(sec, f"expected 2: plans/*, {n} snapshots, each on its plan's own key, and none drops another plan T "
                      "holds",
                 len(star) == n and all(h.done and len(h.pages) == 1 and h.key.startswith(coll_key + "/")
                                        and [e["id"] for e in h.pages[0]["live"]] == [""] for h in star)
                 and not t.omitted and _plans(t, f) == sorted(members),
                 f"{len(star)} snapshots; omitted {t.omitted[:3]}; {len(_plans(t, f))} plans")
    t.read_again(selector=f"{coll_key}/v99")
    nine = t.trackers[-1].heard
    report.check(sec, "expected 2: plans/v99, which the owner does not hold, one page with no entry on …/plans/v99",
                 len(nine) == 1 and nine[0].key == f"{coll_key}/v99" and nine[0].done
                 and not nine[0].pages[0]["live"] and not nine[0].pages[0]["tombs"] and not t.omitted,
                 str([(h.key, h.pages) for h in nine]))
    # a concrete key of each other kind of collection
    # each owner holds oper's "ok" from the start (core §8.2's state values),
    # its own snapshot key with the empty id; p9/route has no snapshot key
    for rel, coll, held_ in (("oper", 0, True), ("p9/route", 2, False)):
        st = live.get_state(helper, f"{f.base}/{rel}", timeout=5.0)
        hs = st.snapshots
        ids = [] if len(hs) != 1 else [e["id"] for e in hs[0].pages[0]["live"]]
        report.check(sec, f"{rel} (collection {coll}), a member read of one concrete key: " + (
                         "a snapshot of its one member, the empty id, on that key" if held_ else
                         "it holds no member there, so a snapshot of no entry on that key"),
                     len(hs) == 1 and hs[0].key == f"{f.base}/{rel}" and hs[0].header["coll"] == coll and hs[0].done
                     and ids == ([""] if held_ else []) and not hs[0].pages[0]["tombs"],
                     str([(h.key, h.header, h.pages) for h in hs]))
    if f.impl == "zk2py":
        _step3(report, sec, f, tool, helper, answer, others)
    else:
        report.note(sec, "steps 3 and 7 for the owner example are played last, from its answers, once it is closed: "
                         "it cannot hold its answer, and a session of the test's own plays it")
    # -- step 4: the control, Latest ----------------------------------------------------
    t4 = sd.LateJoiner(tool, f"{coll_key}/*", consumer=False, timeout=5.0, get_selector=f"{coll_key}/**",
                       collections=ss.ByDescriptor([f.contract]))
    others.append(t4)
    t4.consolidation = zenoh.ConsolidationMode.LATEST
    pg0 = ss.read_page_of(*answer[0])
    v99 = zk2c.InBand(pg0["run_width"], pg0["run"], 0)
    t4._merge(f"{coll_key}/v99", "reply", False, zk2c.mutation(v99, b"old"), zk2c.ENCODING, None)  # noqa: SLF001
    t4.start()
    t4.wait()
    heard4 = t4.trackers[-1].heard
    report.check(sec, "expected 4, Latest: T receives one page of the snapshot, its index showing it incomplete, and "
                      "drops nothing",
                 len(heard4) == 1 and len(heard4[0].pages) == 1 and not heard4[0].done and not t4.omitted
                 and f"{coll_key}/v99" in t4.view(),
                 f"{[(h.key, sorted(h.pages), h.done) for h in heard4]}; omitted {t4.omitted}")
    # -- step 5: a delete beyond the window ------------------------------------------------
    cj = sd.LateJoiner(cons, f"{coll_key}/vehicle-01", consumer=True, timeout=5.0,
                       collections=ss.ByDescriptor([f.contract]))
    others.append(cj)
    cj.start()
    cj.wait()
    had = f"{coll_key}/vehicle-01" in cj.view()
    t.close()  # the link cut: T's subscription gone
    cj.close()
    gone = [members[5], "vehicle-01"]
    for m in gone:
        f.delete(m)
    time.sleep(f.window_s + 1.5)
    t.subscribe()
    time.sleep(0.3)
    late = "v31" if f.impl == "zk2py" else "m200"
    if f.impl == "zk2py":
        from .state_scenarios import Hold

        hold = Hold()
        f.o.state_hold = hold
        t.get(f"{coll_key}/**")
        hold.taken.wait(5.0)
        made = f.o.state_answer(hold.queries[0])
        f.put(late, b"v31")
        time.sleep(0.3)
        f.o.answer_state(hold.queries[0], made)
        f.o.state_hold = None
    else:
        # the owner example answers at once: its late put comes before the
        # GET, heard live, and its snapshot lists it
        f.put(late, b"")
        time.sleep(0.3)
        t.get(f"{coll_key}/**")
    t.wait()
    left = sorted(set(members) - set(gone))
    report.check(sec, f"expected 5: the snapshot lists the {len(left)} plans left; T's omission rule drops "
                      f"plans/{gone[0]} and plans/vehicle-01" + (f"; plans/{late}, heard live at a seq above asof, is "
                                                                  "kept" if f.impl == "zk2py" else
                                                                  f"; plans/{late}, put before the GET, is listed"),
                 sorted(k for _, k in t.omitted[-2:]) == sorted(f"{coll_key}/{m}" for m in gone)
                 and _plans(t, f) == sorted(left + [late]),
                 f"omitted {t.omitted[-3:]}; {len(_plans(t, f))} plans, {late} "
                 f"{'held' if late in _plans(t, f) else 'missing'}")
    cj.subscribe()
    cj.read_again()
    h5 = cj.trackers[-1].heard
    report.check(sec, "expected 5: the consumer's member read draws one page with no entry on …/plans/vehicle-01, and "
                      "the consumer drops its plan",
                 had and len(h5) == 1 and h5[0].key == f"{coll_key}/vehicle-01" and h5[0].done
                 and not h5[0].pages[0]["live"] and not h5[0].pages[0]["tombs"]
                 and f"{coll_key}/vehicle-01" not in cj.view() and cj.omitted == [(f"{coll_key}/vehicle-01",
                                                                                   f"{coll_key}/vehicle-01")],
                 f"had {had}; {[(h.key, h.pages) for h in h5]}; omitted {cj.omitted}")
    # -- step 6: an empty collection -------------------------------------------------------
    for m in left + [late]:
        f.delete(m)
    time.sleep(f.window_s + 1.5)
    t.read_again(selector=f"{coll_key}/**")
    h6 = t.trackers[-1].heard
    report.check(sec, "expected 6: one page with no entry, complete: the owner's word; T's omission rule drops every "
                      "plan it held",
                 len(h6) == 1 and h6[0].done and not h6[0].pages[0]["live"] and not h6[0].pages[0]["tombs"]
                 and _plans(t, f) == [],
                 f"{[(h.key, sorted(h.pages), h.done) for h in h6]}; {len(_plans(t, f))} plans left")
    # -- step 7: two instances answer ------------------------------------------------------
    if f.impl == "zk2py":
        for m in members:
            f.put(m, (m.encode() * 40)[:110])
    else:
        f.command(f"fill {IFACE} state/plans/{{vehicle}} 200")
        f.put("vehicle-01", b"")
    time.sleep(0.5)
    first = raw_get(helper, f"{coll_key}/**")
    # another instance of the service, another run: its record the service's
    second = Fleet(f.impl, ep, record=getattr(f, "record", None))
    others.append(second)
    second.put("x1", b"x1")
    time.sleep(0.3)
    both = raw_get(helper, f"{coll_key}/**")
    run1 = ss.read_page_of(*first[0])["run"]
    ours = [kb for kb in both if ss.read_page_of(*kb)["run"] == run1]
    theirs = [kb for kb in both if ss.read_page_of(*kb)["run"] != run1]
    if f.impl != "zk2py":
        full = sorted(first, key=lambda kb: ss.read_page_of(*kb)["index"])
        f.close()
        second.close()
        others.remove(second)
        f.closed = True
        time.sleep(0.5)
        _step3(report, sec, f, tool, helper, full, others)
    off7 = _quiet(f, second)
    try:
        for order in ("first", "second"):
            p7 = Player(helper, f"{coll_key}/**")
            t7 = sd.LateJoiner(tool, f"{coll_key}/*", consumer=False, timeout=5.0, get_selector=f"{coll_key}/**",
                               collections=ss.ByDescriptor([f.contract]))
            others += [p7, t7]
            t7.subscribe()
            time.sleep(0.2)
            if f.impl != "zk2py":
                # the owners are closed, so no descriptor answers: the test
                # maps the index with the revision the two descriptors named
                t7.collections = ss.collections_from(f.contract, f"zk2/ground/fleet/{IFACE}")
            t7.get()
            p7.taken.wait(5.0)
            p7.answer(ours + theirs if order == "first" else theirs + ours)
            t7.wait()
            done7 = [h for h in t7.trackers[-1].heard if h.done]
            report.check(sec, f"expected 7 ({'the first instance' if order == 'first' else 'the second'}'s snapshot "
                              f"first): both snapshots arrive, one for each run, both complete; each one's omission "
                              f"rule spares the other run's plans; T holds the {n} plans and plans/x1",
                         len(done7) == 2 and len({h.header["run"] for h in done7}) == 2 and not t7.omitted
                         and _plans(t7, f) == sorted(members + ["x1"]),
                         f"{[(h.header['run'], sorted(h.pages)) for h in done7]}; omitted {t7.omitted[:3]}; "
                         f"{len(_plans(t7, f))} plans")
            p7.close()
    finally:
        off7()


def _step3(report: Report, sec: str, f: Fleet, tool: Any, helper: Any, answer: list[tuple[str, bytes]],
           others: list[Any]) -> None:
    """§14 step 3, a page lost: the owner's answer played with one page
    missing, then whole when T reads again."""
    coll_key = f"{f.base}/plans"
    lost = 2 if f.impl == "zk2py" else 1
    ordered = sorted(answer, key=lambda kb: ss.read_page_of(*kb)["index"])
    played = [kb for i, kb in enumerate(ordered) if i != lost]
    every = sorted(f"{coll_key}/{e['id']}" for kb in ordered for e in ss.read_page_of(*kb)["live"])
    cols = ss.ByDescriptor([f.contract]) if f.impl == "zk2py" else ss.collections_from(
        f.contract, f"zk2/ground/fleet/{IFACE}")
    t3 = sd.LateJoiner(tool, f"{coll_key}/*", consumer=False, timeout=5.0, get_selector=f"{coll_key}/**",
                       collections=cols)
    others.append(t3)
    t3.subscribe()
    pg0 = ss.read_page_of(*ordered[0])
    v99 = zk2c.InBand(pg0["run_width"], pg0["run"], 0)
    # "T holds plans/v99 from an earlier seed, which the owner no longer has"
    t3._merge(f"{coll_key}/v99", "reply", False, zk2c.mutation(v99, b"old"), zk2c.ENCODING, None)  # noqa: SLF001
    player = Player(helper, f"{coll_key}/**")
    others.append(player)
    owner_off = _quiet(f)
    try:
        time.sleep(0.3)
        t3.get()
        player.taken.wait(5.0)
        player.answer(played)
        t3.wait()
        inc = t3.incomplete()
        held = sorted(k for k, e in t3.view().items() if not e.deleted)
        report.check(sec, f"expected 3: with page {lost} lost T holds the snapshot incomplete, merges the other "
                          "pages' entries, and drops nothing: it still holds plans/v99",
                     len(inc) == 1 and sorted(inc[0].pages) == [i for i in range(len(ordered)) if i != lost]
                     and not t3.omitted and f"{coll_key}/v99" in t3.view()
                     and len(held) == sum(len(ss.read_page_of(*kb)["live"]) for kb in played) + 1,
                     f"{[(h.key, sorted(h.pages), h.done) for h in inc]}; omitted {t3.omitted}; {len(held)} held")
        threading.Timer(0.5, lambda: player.answer(ordered)).start()
        t3.read_again()
        held = sorted(k for k, e in t3.view().items() if not e.deleted)
        report.check(sec, "expected 3: T reads the collection again, and that snapshot, complete, drops plans/v99",
                     [h.done for h in t3.trackers[-1].heard] == [True] and (coll_key, f"{coll_key}/v99") in t3.omitted
                     and f"{coll_key}/v99" not in t3.view() and held == every,
                     f"omitted {t3.omitted}; {len(held)} held")
    finally:
        owner_off()
        player.close()


def _quiet(*owners: Fleet) -> Any:
    """Stop zk2py's owners answering state GETs while a test's session plays
    them, and return what restores them: each query dropped unanswered. The
    owner example cannot be held, so its steps 3 and 7 are played once it is
    closed (:func:`_steps`)."""
    restore = []
    for o in owners:
        if o.impl == "zk2py":
            o.o.state_hold = lambda q: q.drop()
            restore.append(o)

    def undo() -> None:
        for x in restore:
            x.o.state_hold = None

    return undo


def _py_rust(report: Report, sec: str, f: Fleet, ep: str, members: list[str]) -> None:
    """zk2py's owner, the Rust readers."""
    coll_key = f"{f.base}/plans"

    def zc(*a: str) -> tuple[int, Any, str]:
        p = subprocess.run([str(BLACKBOX / "zenctl"), *a], capture_output=True, text=True, timeout=60)
        try:
            doc = json.loads(p.stdout or "{}")
        except ValueError:
            doc = {"raw": p.stdout[-400:]}
        return p.returncode, doc, p.stderr[-400:]

    rc, doc, err = zc("get", "-c", ep, "--raw", "--format", "json", "--no-decode", f"{coll_key}/**")
    rows = [x for x in doc.get("rows", []) if x.get("row") == "answer"]
    pages = [(x["key"], ss.read_page_of(x["key"], bytes.fromhex(x["hex"].replace(" ", ""))),
              len(bytes.fromhex(x["hex"].replace(" ", "")))) for x in rows]
    report.check(sec, "zenctl get --raw plans/**: one snapshot on …/state/plans in 4 pages, indices 0 to 3, 8 "
                      "entries each, each within 1,024 bytes",
                 rc == 0 and len(pages) == 4 and all(k == coll_key for k, _, _ in pages)
                 and sorted(p["index"] for _, p, _ in pages) == [0, 1, 2, 3]
                 and all(len(p["live"]) == 8 and s <= PAGE_SIZE for _, p, s in pages),
                 f"{rc} {err}; {[(p['index'], len(p['live']), s) for _, p, s in pages]}")
    rc, doc, err = zc("get", "state", "-c", ep, "--format", "json", "--contracts", str(FLEET), f.address, IFACE,
                      "plans/{vehicle}")
    snaps = doc.get("snapshots", [])
    keys = sorted(x["key"].rsplit("/", 1)[1] for x in doc.get("rows", []) if x.get("state") == "value")
    report.check(sec, "zenctl get state plans/{vehicle} (the reference's S4 GET): one complete snapshot of 4 pages, "
                      "coll 1, and the 32 plans, each with its put's in-band stamp",
                 rc == 0 and len(snaps) == 1 and snaps[0].get("complete") is True and snaps[0].get("pages") ==
                 [0, 1, 2, 3] and snaps[0].get("coll") == 1 and keys == sorted(members)
                 and all(x.get("inband", {}).get("seq") == f.o._held[x["key"]].inband.seq  # noqa: SLF001
                         for x in doc.get("rows", [])),
                 f"{rc} {err}; {snaps}; {len(keys)} plans")
    rc, doc, err = zc("get", "-c", ep, "--raw", "--format", "json", "--no-decode", f"{coll_key}/v99")
    rows = [x for x in doc.get("rows", []) if x.get("row") == "answer"]
    p99 = [ss.read_page_of(x["key"], bytes.fromhex(x["hex"].replace(" ", ""))) for x in rows]
    report.check(sec, "zenctl get --raw plans/v99: one page with no entry on …/plans/v99",
                 rc == 0 and len(p99) == 1 and rows[0]["key"] == f"{coll_key}/v99" and not p99[0]["live"]
                 and not p99[0]["tombs"], f"{rc} {err}; {p99}")
    f.delete(members[5])
    time.sleep(0.3)
    rc, doc, err = zc("get", "state", "-c", ep, "--format", "json", "--contracts", str(FLEET), f.address, IFACE,
                      "plans/{vehicle}")
    dead = [x for x in doc.get("rows", []) if x.get("state") == "deleted"]
    report.check(sec, f"a tombstone: within the window zenctl get state shows plans/{members[5]} deleted, with the "
                      "delete's in-band stamp",
                 rc == 0 and [x["key"] for x in dead] == [f"{coll_key}/{members[5]}"]
                 and dead[0].get("inband", {}).get("seq") == f.o._held[f"{coll_key}/{members[5]}"].inband.seq,  # noqa: SLF001
                 f"{rc} {err}; {dead}")
    time.sleep(f.window_s + 1.0)
    rc, doc, err = zc("get", "state", "-c", ep, "--format", "json", "--contracts", str(FLEET), f.address, IFACE,
                      "plans/{vehicle}", "--param", f"vehicle={members[5]}")
    snaps = doc.get("snapshots", [])
    report.check(sec, f"a delete beyond the window: zenctl get state of plans/{members[5]} draws a complete snapshot "
                      "of no entry, and no row of a value",
                 len(snaps) == 1 and snaps[0].get("complete") is True
                 and not [x for x in doc.get("rows", []) if x.get("state") == "value"],
                 f"{rc} {err}; {snaps}; {doc.get('rows')}")
    p = subprocess.run([str(BLACKBOX / "consume"), ep, f.address, str(FLEET), "state/plans/{vehicle}", "@op/poke"],
                       capture_output=True, text=True, timeout=60)
    lines = [x for x in p.stdout.splitlines() if x.startswith("state ")]
    left = {f"{coll_key}/{m}": (m.encode() * 40)[:110].decode() for m in members if m != members[5]}
    report.check(sec, "consume (the Rust consumer, one state GET) reads the owner's snapshot: its one state line a "
                      "plan left, with the record the owner put",
                 p.returncode == 0 and len(lines) == 1 and lines[0].split()[1] in left
                 and lines[0].split()[3] == left[lines[0].split()[1]],
                 f"{p.returncode}: {[x[:90] for x in lines]}")
    # an incomplete snapshot, played: zk2py's owner holds its query, a
    # session of the test's own answers with page 2 lost
    import zenoh

    from . import live

    helper = live.open_client(ep)
    try:
        answer = sorted(raw_get(helper, f"{coll_key}/**"), key=lambda kb: ss.read_page_of(*kb)["index"])
        played = [kb for i, kb in enumerate(answer) if i != 2]
        player = Player(helper, f"{coll_key}/**")
        f.o.state_hold = lambda q: q.drop()
        out: dict[str, Any] = {}

        def run() -> None:
            out["got"] = zc("get", "state", "-c", ep, "--format", "json", "--contracts", str(FLEET), f.address, IFACE,
                            "plans/{vehicle}")

        th = threading.Thread(target=run)
        th.start()
        player.taken.wait(20.0)
        player.answer(played)
        th.join(60.0)
        f.o.state_hold = None
        player.close()
        rc, doc, err = out.get("got", (None, {}, "no answer"))
        snaps = doc.get("snapshots", [])
        report.check(sec, "an incomplete snapshot, page 2 lost: zenctl get state reports it incomplete, pages 0, 1 "
                          "and 3, and shows the entries it merged",
                     len(snaps) == 1 and snaps[0].get("complete") is False and snaps[0].get("pages") == [0, 1, 3]
                     and len([x for x in doc.get("rows", []) if x.get("state") == "value"])
                     == sum(len(ss.read_page_of(*kb)["live"]) for kb in played),
                     f"{rc} {err}; {snaps}; {len(doc.get('rows', []))} rows")
        _ = zenoh
    finally:
        helper.close()


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="python -m zk2py.snapshot_live", description=__doc__.split("\n")[0])
    ap.add_argument("--pairing", action="append", choices=PAIRINGS)
    args = ap.parse_args(argv)
    report = Report()
    for p in args.pairing or list(PAIRINGS):
        section14(report, p)
    failed = [r for r in report.rows if not r[2]]
    print(f"{len(report.rows) - len(failed)} passed, {len(failed)} failed", flush=True)
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())

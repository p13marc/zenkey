"""The state scenarios zk2py runs (``spec/scenarios/state.md``):
``python -m zk2py.state_scenarios``.

§10, a late joiner's seed (core §4.2, 0.26), on an in-process zenoh-python
router R1 with timestamping on. The owner is zk2py's
:class:`zk2py.owner.Owner` of ``interop/zk2py_plans.v1.toml``
(``ground/fleet-mgr``), a client of R1 whose puts, replies and ``reply_del``
carry the stamps it mints (§4.3). The test holds its answer to a GET
(``Owner.state_hold``): it takes the query, makes its answer
(``state_answer``), and sends it (``answer_state``) when the test says, so
that the owner's puts land between the late joiner's steps. The late joiner
is :class:`zk2py.seed.LateJoiner`, a tool's (it keeps a key that is not
concrete), over ``plans/*``.

§7 step 2 (S7, core §4.3, 0.67) on the same router: a consumer's
:class:`~zk2py.seed.LateJoiner` seeds rev 10, the owner restarts as a new
instance on a new session with its clock 5 s behind (``Owner.clock``) and
writes rev 11, and the consumer accepts it by S7's consumer rule after its
seed; a tool's late joiner, §4.2's merge, is the control.

Beside §10, two things observer §4.1 says a seed cannot know: a GET that ran to its
timeout seeds possibly incompletely, and an owner absent at the GET seeds
nothing, which is silence.

§11 (0.27), the merge's open cases, has no zk2 owner: foreign keys
``foreign/*``. Their writer W is one zenoh-python peer session without an
HLC, answering GETs on ``foreign/**`` from a held queryable as a storage
would, and setting a stamp where the scenario says so. The late joiner, a
client of W, is a tool's :class:`~zk2py.seed.LateJoiner`, and a
consumer's beside it shows R6.

§3 (S4) and §12 (0.48) run on one zenoh-python router in a process of its
own (:class:`zk2py.adminspace.RouterProcess`), its admin space on, read
only, whose own session plays the storage manager's document for
``plans``, ``key_expr`` ``zk2/ground/fleet-mgr/**``: a zenoh-python router
loads no plugin, and the document played on the router's session carries
the router's replier id, so "Who answered" counts it. The owner is zk2py's
:class:`~zk2py.owner.Owner` of the walkthrough's ``mission_plan.v1``
(``ground/fleet-mgr``), whose state queryable ends in ``**``.
- §3: the tool's S4 check finds the storage (zk2py's
  :func:`zk2py.live.check_s4`, and, when a zenctl reader is asked, the
  reference's ``doctor --check storage-on-state``); the router drops the
  document; the consumer's plan GET goes through a session that captures
  what it asks and who answers.
- §12: 200 rounds, each the reader's two reads, a put across the router
  and the consumer's S4 GET, each within 2 s, beside a client holding 200
  subscribers and 200 queryables on ``load/<i>/**`` and a third declaring
  and undeclaring a subscriber and a liveliness token flat out
  (:class:`zk2py.adminspace.LoadProcess`), while a queryable on ``@/**``
  (:class:`zk2py.adminspace.Recorder`) records every selector it hears.
  The readers: zk2py's (``--reader zk2py``, the default), the reference's
  doctor's S4 check (``zenctl-doctor``) and ``zenctl storage list``
  (``zenctl-storage``). Every wait is bounded, and no hazardous GET is
  asked: the router is killed at the end of each run, frozen or not.
  The recorder is a client's queryable. Expected 3 (0.51, F-257): it "may
  never hear a read that freezes the router", so the runner counts what
  it heard against the reads the reader makes, and Expected 2 fails such
  a reader either way.

Exit 0 when every check passes, 1 when any fails.
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import threading
import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

from . import seed as sd

REPO = Path(__file__).resolve().parents[3]
PLANS = REPO / "impl" / "python" / "interop" / "zk2py_plans.v1.toml"
MISSION_PLAN = REPO / "examples" / "zk2" / "walkthrough" / "mission_plan.v1.toml"
#: §3 and §12's storage, as the storage manager states it (observer §4.2)
STORAGE = ("plans", "zk2/ground/fleet-mgr/**")
DEFAULT_ZENCTL = REPO / "target" / "debug" / "zenctl"


@dataclass
class Run:
    """What ``main`` was told, handed to each section: §12's readers and
    rounds, and the reference's zenctl. §3 runs zenctl's doctor too whenever
    a zenctl reader is asked."""

    readers: list[str] = field(default_factory=lambda: ["zk2py"])
    rounds: int = 200
    zenctl: Path = Path(os.environ.get("ZK2PY_ZENCTL", DEFAULT_ZENCTL))

    @property
    def budget_s(self) -> float:
        """§12's bound on one reader's whole run, setup and rounds: a round
        takes 10–60 ms when the router routes, so this is room, never a
        wait."""
        return 60.0 + 0.5 * self.rounds


class Report:
    def __init__(self) -> None:
        self.rows: list[tuple[str, str, bool, str]] = []

    def check(self, section: str, name: str, ok: bool, detail: str = "") -> bool:
        self.rows.append((section, name, ok, detail))
        print(f"{'PASS' if ok else 'FAIL'} [{section}] {name}" + (f": {detail}" if detail else ""), flush=True)
        return ok

    def note(self, section: str, text: str) -> None:
        print(f"NOTE [{section}] {text}", flush=True)


class Hold:
    """A test's hold on the owner's state GETs: it takes each query, and
    the test answers it."""

    def __init__(self) -> None:
        self.queries: list[Any] = []
        self.taken = threading.Event()

    def __call__(self, query: Any) -> None:
        self.queries.append(query)
        self.taken.set()


def _owner(endpoint: str, service: str):
    from .contract import load_contract
    from .owner import Owner

    c = load_contract(PLANS)
    o = Owner("ground", service, [c], connect=endpoint)
    o.start()
    return o, f"{o.prefix(c)}/state/plans"


def _show(e: sd.Entry | None) -> str:
    if e is None:
        return "missing"
    return f"{'deleted' if e.deleted else e.payload.decode()} at {e.stamp} via {e.via}"


def section10(report: Report, run: Run | None = None) -> None:
    from . import live
    from .live_interop import _r1

    sec = "§10 a late joiner's seed"
    r1, ep, _ = _r1()
    tool = live.open_client(ep)
    owners: list[Any] = []
    try:
        o, base = _owner(ep, "fleet-mgr")
        owners.append(o)
        t1 = o.set_state(f"{base}/a", b"v1")
        o.set_state(f"{base}/b", b"v1")
        t2 = o.delete_state(f"{base}/b")
        hold = Hold()
        o.state_hold = hold
        time.sleep(0.3)
        # Step 1: subscribe, then GET (All + Latest).
        lj = sd.LateJoiner(tool, f"{base}/*", consumer=False, timeout=5.0)
        lj.start()
        # Step 2: the owner takes the GET and holds it; it puts d.
        if not report.check(sec, "the owner takes the late joiner's GET and holds it", hold.taken.wait(5.0)):
            return
        t3 = o.set_state(f"{base}/d", b"v1")
        # Step 3: it makes its answer.
        answer = o.state_answer(hold.queries[0])
        # Step 4: before sending it, it puts a newer a, then c.
        t4 = o.set_state(f"{base}/a", b"v2")
        t5 = o.set_state(f"{base}/c", b"v1")
        time.sleep(0.5)
        # Step 5: it sends its answer and ends the GET.
        o.answer_state(hold.queries[0], answer)
        ended = lj.wait()
        time.sleep(0.3)
        v = lj.view()
        report.check(sec, "the answer the owner made holds a (v1, T1), b's reply_del (T2) and d (v1, T3)",
                     sorted((k.rsplit("/", 1)[1], h.payload, str(h.stamp)) for k, h in answer)
                     == [("a", b"v1", str(t1)), ("b", None, str(t2)), ("d", b"v1", str(t3))],
                     str([(k.rsplit("/", 1)[1], h.payload) for k, h in answer]))

        def entry(k: str) -> sd.Entry | None:
            return v.get(f"{base}/{k}")

        a_reply = [x for x in lj.applied if x.key.endswith("/a") and x.via == "reply"]
        report.check(sec, "expected 1: a is v2 with T4; the reply (T1) is older than the live put, dropped by the "
                          "merge and never shown after v2",
                     entry("a") is not None and entry("a").payload == b"v2" and str(entry("a").stamp) == _t(t4)
                     and [x.outcome for x in a_reply] == ["dropped"],
                     f"{_show(entry('a'))}; the reply {[(str(x.stamp), x.outcome) for x in a_reply]}")
        report.check(sec, "expected 2: c is v1 with T5, the put the answer does not hold, arrived through the "
                          "subscription",
                     entry("c") is not None and entry("c").payload == b"v1" and str(entry("c").stamp) == _t(t5)
                     and entry("c").via == "live", _show(entry("c")))
        report.check(sec, "expected 3: b is a deletion with T2: the reply_del reached the merge as a deletion, "
                          "never as an absence",
                     entry("b") is not None and entry("b").deleted and str(entry("b").stamp) == _t(t2),
                     _show(entry("b")))
        d_all = [x for x in lj.applied if x.key.endswith("/d")]
        report.check(sec, "expected 4: d is v1 with T3, once: the live put and the reply carry one stamp, one "
                          "mutation",
                     entry("d") is not None and str(entry("d").stamp) == _t(t3)
                     and sorted(x.outcome for x in d_all) == ["kept", "same"],
                     f"{_show(entry('d'))}; arrivals {[(x.via, x.outcome) for x in d_all]}")
        report.check(sec, "expected 5: the seed ran to its final reply, with no error reply: complete",
                     ended and lj.seed.complete and not lj.seed.errors,
                     f"ended {ended}, complete {lj.seed.complete}, errors {lj.seed.errors}")
        lj.close()

        # Step 6, the control: the other order.
        oc, basec = _owner(ep, "fleet-mgr-control")
        owners.append(oc)
        oc.set_state(f"{basec}/a", b"v1")
        oc.set_state(f"{basec}/b", b"v1")
        oc.delete_state(f"{basec}/b")
        holdc = Hold()
        oc.state_hold = holdc
        time.sleep(0.3)
        ljc = sd.LateJoiner(tool, f"{basec}/*", consumer=False, timeout=5.0, order="get-first")
        ljc.start()
        holdc.taken.wait(5.0)
        answer_c = oc.state_answer(holdc.queries[0])
        oc.set_state(f"{basec}/c", b"v1")
        time.sleep(0.3)
        oc.answer_state(holdc.queries[0], answer_c)
        ljc.wait()
        ljc.subscribe()
        time.sleep(0.5)
        vc = ljc.view()
        report.check(sec, "expected 6, the control (GET first, then subscribe): c is missing, put after the answer "
                          "was made and before the subscription existed",
                     f"{basec}/c" not in vc and f"{basec}/a" in vc,
                     f"held {sorted(k.rsplit('/', 1)[1] for k in vc)}")
        ljc.close()

        # What a seed cannot know: a GET that ran to its timeout.
        holdt = Hold()
        o.state_hold = holdt
        ljt = sd.LateJoiner(tool, f"{base}/*", consumer=False, timeout=1.0)
        ljt.start()
        ljt.wait()
        report.check(sec, "a GET held past its timeout ends with an error reply: the seed is possibly incomplete "
                          "(observer §4.1, §3.3's reading applied)",
                     ljt.seed.ended and not ljt.seed.complete and bool(ljt.seed.errors), str(ljt.seed.errors))
        for q in holdt.queries:
            q.drop()
        ljt.close()
        o.state_hold = None
        # An owner absent at the GET seeds nothing: silence.
        lja = sd.seed(tool, "zk2/ground/nobody/zk2py_plans.v1/state/plans/*", consumer=False)
        report.check(sec, "an owner absent at the GET seeds nothing: silence (O5), never \"no state\"",
                     lja.seed.silent and not lja.view(), f"replies {lja.seed.replies}, errors {lja.seed.errors}")
        lja.close()
    finally:
        for x in owners:
            x.close()
        tool.close()
        r1.close()


def section7(report: Report, run: Run | None = None) -> None:
    """§7 step 2, a new epoch, after a consumer's seed (core §4.3, S7's
    consumer rule; 0.67, F-368). The owner writes rev 10; a consumer's late
    joiner and a tool's seed from it. The owner then restarts with no
    record and no owner-side archive, a new instance on a new session (a
    fresh zid), its clock 5 s behind, and writes rev 11: "The consumer sees
    a new timestamp id, accepts rev 11, and restarts its ordering." The
    tool's late joiner is the control: §4.2's merge, which still orders a
    tool's after its seed, keeps rev 10's later time."""
    import zenoh

    from . import live
    from .contract import load_contract
    from .live_interop import _r1
    from .owner import Owner

    sec = "§7 clocks (S7)"
    r1, ep, _ = _r1()
    tool = live.open_client(ep)
    owners: list[Any] = []
    joiners: list[sd.LateJoiner] = []
    try:
        c = load_contract(PLANS)
        o1 = Owner("ground", "fleet-mgr-s7", [c], connect=ep)
        o1.start()
        owners.append(o1)
        base = f"{o1.prefix(c)}/state/plans"
        t10 = o1.set_state(f"{base}/r", b"rev10")
        time.sleep(0.3)
        lc = sd.LateJoiner(tool, f"{base}/*", consumer=True, timeout=5.0)
        lt = sd.LateJoiner(tool, f"{base}/*", consumer=False, timeout=5.0)
        joiners += [lc, lt]
        for j in joiners:
            j.start()
        seeded = all(j.wait() for j in joiners)
        report.check(sec, "both late joiners seed rev 10 from the owner's GET",
                     seeded and all(j.view().get(f"{base}/r") is not None
                                    and j.view()[f"{base}/r"].payload == b"rev10" for j in joiners),
                     ", ".join(_show(j.view().get(f"{base}/r")) for j in joiners))
        # Step 2: a restart with no record: a new instance on a new session,
        # its clock 5 s behind.
        o1.close()
        owners.remove(o1)
        o2 = Owner("ground", "fleet-mgr-s7", [c], connect=ep)

        def behind() -> Any:
            # Owner.mint calls it from start() on, once the session is open:
            # the descriptor's first put is the first stamp it mints.
            if o2.session is None:
                raise RuntimeError("§7: the restarted owner's clock was read before its session opened")
            ts = o2.session.new_timestamp()
            n = ts.get_time_as_ntp64().as_nanos() - 5_000_000_000
            return zenoh.Timestamp(zenoh.NTP64(n // 1_000_000_000, n % 1_000_000_000), ts.get_id())

        o2.clock = behind
        o2.start()
        owners.append(o2)
        t11 = o2.set_state(f"{base}/r", b"rev11")
        time.sleep(0.5)
        s10, s11 = sd.Stamp.of(t10), sd.Stamp.of(t11)
        report.check(sec, "rev 11 carries a new timestamp id, its time below rev 10's",
                     s10 is not None and s11 is not None and s11.id != s10.id and s11.time < s10.time,
                     f"rev 10 {s10}, rev 11 {s11}")
        got = lc.view().get(f"{base}/r")
        arrival = [x for x in lc.applied if x.key == f"{base}/r" and x.via == "live" and str(x.stamp) == str(s11)]
        report.check(sec, "expected 2: the consumer sees a new timestamp id and accepts rev 11, by S7's consumer "
                          "rule after its seed",
                     got is not None and got.payload == b"rev11" and [(x.outcome, x.rule) for x in arrival]
                     == [("kept", "S7")], f"{_show(got)}; {[(x.outcome, x.rule) for x in arrival]}")
        t12 = o2.set_state(f"{base}/r", b"rev12")
        time.sleep(0.5)
        got = lc.view().get(f"{base}/r")
        report.check(sec, "expected 2: the consumer restarts its ordering under the new id: rev 12, later than "
                          "rev 11 and still below rev 10's time, replaces it",
                     got is not None and got.payload == b"rev12" and str(got.stamp) == str(sd.Stamp.of(t12)),
                     _show(got))
        held = lt.view().get(f"{base}/r")
        report.check(sec, "the control: a tool's late joiner, §4.2's merge after its seed, keeps rev 10's later "
                          "time over the new epoch's values",
                     held is not None and held.payload == b"rev10", _show(held))
    finally:
        for j in joiners:
            j.close()
        for x in owners:
            x.close()
        tool.close()
        r1.close()


def section11(report: Report, run: Run | None = None) -> None:
    import json

    import zenoh

    from .owner import free_loopback_port

    sec = "§11 a late joiner's merge: unstamped samples, equal times, a key that is not concrete"
    port = free_loopback_port()
    conf = zenoh.Config()
    conf.insert_json5("mode", json.dumps("peer"))
    conf.insert_json5("listen/endpoints", json.dumps([f"tcp/127.0.0.1:{port}"]))
    conf.insert_json5("scouting/multicast/enabled", "false")
    conf.insert_json5("timestamping/enabled", "false")
    w = zenoh.open(conf)
    cconf = zenoh.Config()
    cconf.insert_json5("mode", json.dumps("client"))
    cconf.insert_json5("connect/endpoints", json.dumps([f"tcp/127.0.0.1:{port}"]))
    cconf.insert_json5("scouting/multicast/enabled", "false")
    tool = zenoh.open(cconf)
    hold = Hold()
    q = w.declare_queryable("foreign/**", zenoh.handlers.Callback(hold), complete=False)

    def stamp(time_: int, ident: str) -> Any:
        return zenoh.Timestamp(zenoh.NTP64(time_ // sd_ns(), time_ % sd_ns()),
                               zenoh.TimestampId(int(ident, 16).to_bytes(16, "little")))

    try:
        time.sleep(0.5)
        base = time.time_ns()
        t1, t2, t3 = stamp(base, "a1"), stamp(base + 1_000_000, "a1"), stamp(base + 2_000_000, "a1")
        lj = sd.LateJoiner(tool, "foreign/**", consumer=False, timeout=5.0)
        cons = sd.LateJoiner(tool, "foreign/**", consumer=True, timeout=5.0)
        cons.subscribe()
        lj.start()
        if not report.check(sec, "step 1: W takes the late joiner's GET and holds it", hold.taken.wait(5.0)):
            return
        w.put("foreign/a", b"a")
        w.put("foreign/c", b"c-1")
        w.put("foreign/c", b"c-2")
        time.sleep(0.4)
        query = hold.queries[0]
        query.reply("foreign/a", b"a-T1", timestamp=t1)
        query.reply("foreign/b", b"b-T2", timestamp=t2)
        query.reply("foreign/c", b"c-0")
        query.drop()
        lj.wait()
        time.sleep(0.3)
        w.put("foreign/a", b"a-late")
        w.delete("foreign/b")
        w.put("foreign/b", b"b-T3", timestamp=t3)
        same = base + 5_000_000
        w.put("foreign/e", b"e-2", timestamp=stamp(same, "2"))
        w.put("foreign/e", b"e-100", timestamp=stamp(same, "100"))
        w.put("foreign/f", b"f-100", timestamp=stamp(same, "100"))
        w.put("foreign/f", b"f-2", timestamp=stamp(same, "2"))
        w.put("foreign/g", b"g-T1", timestamp=t1)
        w.put("foreign/*", b"wild-T3", timestamp=t3)
        w.put("foreign/g", b"g-T2", timestamp=t2)
        time.sleep(0.6)

        def kept_values(j: Any, k: str) -> list[str]:
            """Each arrival on ``k`` at the merge, in order: kept, the same
            mutation, or dropped."""
            return [x.outcome for x in j.applied if x.key == k]

        v = lj.view()
        report.check(sec, "steps 2-3: a is kept unstamped, then replaced by the stamped reply (T1); c keeps c-1, "
                          "c-2, then the reply's unstamped c-0, which arrived last; b is kept (T2)",
                     kept_values(lj, "foreign/a")[:2] == ["kept", "kept"]
                     and kept_values(lj, "foreign/c") == ["kept", "kept", "kept"] and v["foreign/c"].payload == b"c-0"
                     and kept_values(lj, "foreign/b")[:1] == ["kept"],
                     f"a {kept_values(lj, 'foreign/a')}, c {kept_values(lj, 'foreign/c')} -> {v['foreign/c'].payload!r}, "
                     f"b {kept_values(lj, 'foreign/b')}")
        report.check(sec, "step 4: neither unstamped sample is kept, on a or b, each key holding a stamp; the "
                          "stamped put on b (T3) is kept",
                     kept_values(lj, "foreign/a")[2:] == ["dropped"] and kept_values(lj, "foreign/b")[1:] == [
                         "dropped", "kept"] and v["foreign/a"].payload == b"a-T1" and v["foreign/b"].payload == b"b-T3",
                     f"a {kept_values(lj, 'foreign/a')} -> {v['foreign/a'].payload!r}; b "
                     f"{kept_values(lj, 'foreign/b')} -> {v['foreign/b'].payload!r}")
        report.check(sec, "step 5: on e only (t, 2) is kept, on f both, the second replacing the first: at one time "
                          "id 100 is below id 2 (the ids' bytes from the least significant)",
                     kept_values(lj, "foreign/e") == ["kept", "dropped"] and kept_values(lj, "foreign/f") == [
                         "kept", "kept"] and v["foreign/e"].payload == b"e-2" and v["foreign/f"].payload == b"f-2",
                     f"e {kept_values(lj, 'foreign/e')}, f {kept_values(lj, 'foreign/f')}")
        cv = cons.view()
        report.check(sec, "step 6: a tool's seed keeps all three, the wildcard put under foreign/* never merged with "
                          "foreign/g, so foreign/g at T2 is kept after it; a consumer's seed discards the wildcard "
                          "put (R6)",
                     kept_values(lj, "foreign/g") == ["kept", "kept"] and kept_values(lj, "foreign/*") == ["kept"]
                     and v["foreign/g"].payload == b"g-T2" and "foreign/*" not in cv and cv["foreign/g"].payload == b"g-T2",
                     f"tool g {kept_values(lj, 'foreign/g')}, * {kept_values(lj, 'foreign/*')}; consumer keys "
                     f"{sorted(cv)}")
        lj.close()
        cons.close()
    finally:
        q.undeclare()
        tool.close()
        w.close()


# -- §3 and §12: S4's check, and its reads beside a router that is declaring ------

READERS = ("zk2py", "zenctl-doctor", "zenctl-storage")


class Capture:
    """A consumer's session whose outgoing queries the test captures (§3
    step 2): each ``get``'s selector, target and consolidation as the
    consumer asked them, and each reply's replier id. It is captured at the
    session's API, not on the wire: zenoh-python sends what it is given."""

    def __init__(self, session: Any) -> None:
        self.session = session
        self.asked: list[dict[str, Any]] = []
        self.repliers: list[str | None] = []

    def get(self, selector: str, handler: Any = None, **kw: Any) -> Any:
        import zenoh

        from . import live

        self.asked.append({"selector": selector, "target": kw.get("target"),
                           "consolidation": kw.get("consolidation")})
        inner = handler.callback

        def cb(reply: Any) -> None:
            self.repliers.append(live.replier_of(reply))
            inner(reply)

        return self.session.get(selector, zenoh.handlers.Callback(cb, handler.drop), **kw)


class _Bus:
    """§3's and §12's bus: the router process, the owner of
    ``mission_plan.v1`` holding ``plans/vehicle-01``, and the consumer."""

    def __init__(self) -> None:
        from . import live
        from .adminspace import RouterProcess
        from .contract import load_contract
        from .owner import Owner

        self.router = RouterProcess({STORAGE[0]: STORAGE[1]})
        self.sessions: list[Any] = []
        self.owner = None
        c = load_contract(MISSION_PLAN)
        self.owner = Owner("ground", "fleet-mgr", [c], connect=self.router.endpoint)
        self.owner.start()
        self.plan = f"{self.owner.prefix(c)}/state/plans/vehicle-01"
        self.stamp = self.owner.set_state(self.plan, b"plan-v1")
        assert self.owner.session is not None
        self.owner_zid = str(self.owner.session.zid())
        self.consumer = self.open()

    def open(self) -> Any:
        from .adminspace import _client

        s = _client(self.router.endpoint)
        self.sessions.append(s)
        return s

    def close(self) -> None:
        from .adminspace import close_bounded

        self.router.kill()  # first: a client of a router that froze closes once its link is gone
        close_bounded(*self.sessions)
        if self.owner is not None:
            close_bounded(self.owner, timeout=5.0)


def _zenctl(zenctl: Path, argv: list[str], endpoint: str, timeout: float = 30.0) -> tuple[int | None, Any, str]:
    """One zenctl run as a black box: its exit code (None past ``timeout``,
    killed), its JSON document, and its stderr's tail."""
    try:
        p = subprocess.run([str(zenctl), *argv, "--format", "json", "--timeout", "2", "-c", endpoint],
                           capture_output=True, text=True, timeout=timeout)
    except subprocess.TimeoutExpired:
        return None, None, "killed past its timeout"
    try:
        doc = json.loads(p.stdout)
    except ValueError:
        doc = None
    # its stderr says what it read; only a run with no verdict needs it shown
    return p.returncode, doc, "" if p.returncode in (0, 1) and doc is not None else p.stderr[-300:]


def _doctor_s4(doc: Any) -> tuple[bool, bool, str]:
    """The reference doctor's ``storage-on-state`` as its JSON shows it:
    (a router answered, the storage found on owners' state, why)."""
    if not isinstance(doc, dict):
        return False, False, "no document"
    notes = " ".join(n.get("text", "") for n in doc.get("notes", []) if isinstance(n, dict))
    row = next((r for r in doc.get("rows", []) if isinstance(r, dict) and r.get("check") == "storage-on-state"), {})
    found = [f for f in row.get("findings", []) if isinstance(f, dict)
             and str(f.get("subject", "")).startswith(f"{STORAGE[0]}@") and STORAGE[1] in str(f.get("evidence", ""))]
    return "1 router(s) answered the admin space" in notes, bool(found), \
        f"{row.get('verdict')}; findings {[f.get('subject') for f in row.get('findings', [])]}"


def section3(report: Report, run: Run) -> None:
    from . import live

    sec = "§3 the owner is authoritative (S4)"
    bus = _Bus()
    try:
        tool = bus.open()
        time.sleep(0.5)
        # Step 1: a tool inspects the routers' storage admin space.
        s4 = live.check_s4(tool)
        report.check(sec, "expected 1 (zk2py's tool): the storage plans on zk2/ground/fleet-mgr/** is a finding, "
                          "read from the router's own, verified answer",
                     s4.verdict == "broken" and s4.routers == [bus.router.zid]
                     and [(x[2], x[3]) for x in s4.storages] == [(STORAGE[1], True)], f"{s4.verdict}: {s4.detail}")
        zc = [r for r in run.readers if r.startswith("zenctl")]
        if zc:
            rc, doc, err = _zenctl(run.zenctl, ["doctor", "--check", "storage-on-state"], bus.router.endpoint)
            answered, found, why = _doctor_s4(doc)
            report.check(sec, "expected 1 (zenctl doctor --check storage-on-state): exit 1, the finding names plans "
                              "and its key_expr", rc == 1 and answered and found, f"exit {rc}; {why} {err}")
        # Step 2: without the storage.
        dropped = bus.router.command(f"drop {STORAGE[0]}")
        time.sleep(0.3)
        clean = live.check_s4(tool)
        report.check(sec, "without the storage the tool's check reads clean: the router verified, no storage answer",
                     dropped == f"ok drop {STORAGE[0]}" and clean.verdict == "clean" and not clean.storages,
                     f"{dropped}; {clean.verdict}: {clean.detail}")
        if zc:
            rc, doc, err = _zenctl(run.zenctl, ["doctor", "--check", "storage-on-state"], bus.router.endpoint)
            answered, found, why = _doctor_s4(doc)
            report.check(sec, "without the storage zenctl's check exits 0: established clean",
                         rc == 0 and answered and not found, f"exit {rc}; {why} {err}")
        import zenoh

        cap = Capture(bus.consumer)
        got = live.get_state(cap, bus.plan, timeout=2.0)  # type: ignore[arg-type]
        report.check(sec, "expected 2: the consumer's query carries target All and consolidation Latest, set "
                          "explicitly, and the owner alone answers it: its one reply carries the owner's stamp, and "
                          "every reply's replier id is the owner's session",
                     cap.asked == [{"selector": bus.plan, "target": zenoh.QueryTarget.ALL,
                                    "consolidation": zenoh.ConsolidationMode.LATEST}]
                     and [r.stamp_id for r in got.replies] == [str(bus.stamp).split("/", 1)[1]]
                     and cap.repliers == [bus.owner_zid] and not got.errors,
                     f"asked {cap.asked}; replies {[(r.key, r.stamp_id) for r in got.replies]}, repliers "
                     f"{cap.repliers}, owner {bus.owner_zid}, errors {got.errors}")
    finally:
        bus.close()


class _Reader:
    """§12's reader under test: one S4 check's reads per :meth:`read`, as
    (the router found, ``plans`` found with its key_expr, why)."""

    def __init__(self, kind: str, bus: _Bus, zenctl: Path) -> None:
        self.kind, self.bus, self.zenctl = kind, bus, zenctl
        self.session = bus.open() if kind == "zk2py" else None
        #: how many reads of each selector one read makes, as Expected 3 counts them
        self.asks = {"zk2py": 2, "zenctl-doctor": 2, "zenctl-storage": 1}[kind]

    def read(self, timeout: float = 30.0) -> tuple[bool | None, bool, str]:
        """One read; a zenctl run is killed past ``timeout``."""
        from . import live

        if self.kind == "zk2py":
            s4 = live.check_s4(self.session, timeout=2.0)
            return (s4.routers == [self.bus.router.zid],
                    s4.verdict == "broken" and [(x[2], x[3]) for x in s4.storages] == [(STORAGE[1], True)],
                    f"{s4.verdict}: {s4.detail}")
        if self.kind == "zenctl-doctor":
            rc, doc, err = _zenctl(self.zenctl, ["doctor", "--check", "storage-on-state"], self.bus.router.endpoint,
                                   timeout)
            answered, found, why = _doctor_s4(doc)
            return answered, rc == 1 and found, f"exit {rc}; {why} {err}"
        rc, doc, err = _zenctl(self.zenctl, ["storage", "list"], self.bus.router.endpoint, timeout)
        rows = doc.get("rows", []) if isinstance(doc, dict) else []
        found = [r for r in rows if isinstance(r, dict) and r.get("name") == STORAGE[0]
                 and r.get("key_expr") == STORAGE[1] and r.get("zid") == self.bus.router.zid]
        # storage list reads the second selector alone: the first is not asked
        return None, rc == 0 and len(found) == 1 and len(rows) == 1, f"exit {rc}; rows {rows} {err}"


def section12(report: Report, run: Run) -> None:
    for kind in run.readers:
        if kind.startswith("zenctl") and not run.zenctl.is_file():
            report.check(f"§12 ({kind})", "the section ran", False, f"{run.zenctl} not found")
            continue
        _section12(report, run, kind)


def _section12(report: Report, run: Run, kind: str) -> None:
    """One reader's §12. It ends at the first round that misses Expected 2,
    a put not arrived or the owner's S4 answer missing within 2 s, and
    fails there with that round's number: a frozen router fails it at once,
    and never runs out the rounds. The whole run is bounded by
    :attr:`Run.budget_s`, and a zenctl read by what is left of it."""
    from . import live
    from .adminspace import LoadProcess, Probe, Recorder

    sec = f"§12 the check's reads beside a router that is declaring ({kind} reader, {run.rounds} rounds)"
    deadline = time.monotonic() + run.budget_s
    bus = _Bus()
    load = None
    try:
        load = LoadProcess(bus.router.endpoint, 200, churn=True)
        recorder = Recorder(bus.open())
        probe = Probe(bus.router.endpoint)
        bus.sessions += probe.sessions
        reader = _Reader(kind, bus, run.zenctl)
        time.sleep(0.5)
        recorder.take()
        rounds: list[dict[str, Any]] = []
        missed: int | None = None  # the round, from 1, that missed Expected 2
        out_of_time = False
        for i in range(run.rounds):
            left = deadline - time.monotonic()
            if left <= 0:
                out_of_time = True
                break
            found_router, found_plans, why = reader.read(min(30.0, left))
            put_s = probe.put()
            t0 = time.monotonic()
            got = live.get_state(bus.consumer, bus.plan, timeout=2.0)
            owner_s = time.monotonic() - t0
            answered = [r.stamp_id for r in got.replies] == [str(bus.stamp).split("/", 1)[1]] and owner_s <= 2.0
            rounds.append({"router": found_router, "plans": found_plans, "why": why, "put": put_s,
                           "owner": owner_s if answered else None, "owner_detail": f"{len(got.replies)} replies, "
                           f"errors {got.errors}, after {owner_s * 1000:.1f} ms"})
            if put_s is None or not answered:
                missed = i + 1
                break
        heard = recorder.take()
        n = len(rounds)
        if missed is not None:
            why_stop = f"stopped at round {missed}, which missed Expected 2"
        elif out_of_time:
            why_stop = f"stopped after round {n}: the section's {run.budget_s:g} s ran out"
        else:
            why_stop = ""
        report.check(sec, f"the rounds ran: {n} of {run.rounds}", n == run.rounds and missed is None, why_stop)
        first = [r["router"] for r in rounds if r["router"] is not None]
        bad1 = [i + 1 for i, r in enumerate(rounds) if r["router"] is False or not r["plans"]]
        report.check(sec, "expected 1: every read of the first selector finds the router, and every read of the "
                          "second finds plans with its key_expr"
                          + ("" if first else " (this reader does not read the first: the recorder shows it)"),
                     not bad1 and n > 0,
                     f"{len(first)} read(s) of the first; failing rounds {bad1[:5]}"
                     + (f": {rounds[bad1[0] - 1]['why']}" if bad1 else ""))
        puts = [r["put"] for r in rounds if r["put"] is not None]
        owners = [r["owner"] for r in rounds if r["owner"] is not None]
        if missed is not None:
            last = rounds[-1]
            detail = (f"round {missed} missed it: the put "
                      + ("did not arrive within 2 s" if last["put"] is None else f"arrived in {last['put'] * 1000:.1f} ms")
                      + ", the owner's S4 answer "
                      + ("missing within 2 s" if last["owner"] is None else f"in {last['owner'] * 1000:.1f} ms")
                      + f" ({last['owner_detail']})")
        else:
            detail = (f"{len(puts)} puts and {len(owners)} S4 answers of {n} rounds; slowest put "
                      f"{max(puts, default=0) * 1000:.1f} ms, slowest S4 answer {max(owners, default=0) * 1000:.1f} ms")
        report.check(sec, "expected 2: the router keeps routing throughout: in every round the put arrives, and the "
                          "owner answers the S4 GET, within 2 s of being made",
                     missed is None and len(puts) == n == len(owners) and n > 0, detail)
        keys = [h.split("?", 1)[0] for h in heard]
        allowed = {live.S4_ROUTERS, live.S4_STORAGES}
        reaching = sorted({k for k in keys if live.admin_records_reached(k)})
        report.check(sec, "expected 3: every selector the queryable on @/** heard is one of the two, and none "
                          "intersects a node's declaration records, its linkstate/* or its route/successor/**; it "
                          f"heard each of the reader's {reader.asks} read(s) a round (0.50, F-254; 0.51, F-257)",
                     set(keys) <= allowed and not reaching and len(keys) == reader.asks * n,
                     f"heard {len(keys)} GET(s) over {n} round(s): {sorted(set(keys))}; reaching the records {reaching}")
    finally:
        if load is not None:
            load.close()
        # the recorder's session is the bus's: it closes there, after the
        # router is killed, so a router that froze holds nothing up
        bus.close()
    if load is not None:
        report.note(sec, f"the third client declared and undeclared a subscriber and a token {load.churned} times")


def sd_ns() -> int:
    return 1_000_000_000


def _t(ts: Any) -> str:
    return str(sd.Stamp.of(ts))


SECTIONS = {"3": section3, "7": section7, "10": section10, "11": section11, "12": section12}


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="python -m zk2py.state_scenarios", description=__doc__.split("\n")[0])
    ap.add_argument("--only", action="append", choices=sorted(SECTIONS, key=int), help="run only these sections")
    ap.add_argument("--reader", action="append", choices=READERS,
                    help="§12's reader under test, repeatable (default zk2py); §3's tool is zenctl's doctor too "
                         "whenever a zenctl reader is asked")
    ap.add_argument("--rounds", type=int, default=200, help="§12's rounds, 200 as the section says")
    ap.add_argument("--zenctl", type=Path, default=Run().zenctl, help="the reference's zenctl, run as a black box")
    args = ap.parse_args(argv)
    run = Run(readers=args.reader or ["zk2py"], rounds=args.rounds, zenctl=args.zenctl)
    report = Report()
    for key, fn in SECTIONS.items():
        name = f"§{key}"
        if args.only and key not in args.only:
            continue
        try:
            fn(report, run)
        except Exception as e:  # noqa: BLE001 - a section that cannot run is a failure
            report.check(name, "the section ran", False, f"{type(e).__name__}: {e}")
    passed = sum(1 for r in report.rows if r[2])
    failed = len(report.rows) - passed
    print(f"state scenarios: {passed} passed, {failed} failed")
    for sec, name, ok, detail in report.rows:
        if not ok:
            print(f"FAIL [{sec}] {name}: {detail}")
    return 1 if failed else 0


if __name__ == "__main__":
    rc = main()
    sys.stdout.flush()
    # a session whose router was killed can hold the interpreter at exit
    os._exit(rc)

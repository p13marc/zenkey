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

Beside §10, two things §4.2 says a seed cannot know: a GET that ran to its
timeout seeds possibly incompletely, and an owner absent at the GET seeds
nothing, which is silence.

§11 (0.27), the merge's open cases, has no zk2 owner: foreign keys
``foreign/*``. Their writer W is one zenoh-python peer session without an
HLC, answering GETs on ``foreign/**`` from a held queryable as a storage
would, and setting a stamp where the scenario says so. The late joiner, a
client of W, is a tool's :class:`~zk2py.seed.LateJoiner`, and a
consumer's beside it shows R6.

Exit 0 when every check passes, 1 when any fails.
"""

from __future__ import annotations

import argparse
import sys
import threading
import time
from pathlib import Path
from typing import Any

from . import seed as sd

REPO = Path(__file__).resolve().parents[3]
PLANS = REPO / "impl" / "python" / "interop" / "zk2py_plans.v1.toml"


class Report:
    def __init__(self) -> None:
        self.rows: list[tuple[str, str, bool, str]] = []

    def check(self, section: str, name: str, ok: bool, detail: str = "") -> bool:
        self.rows.append((section, name, ok, detail))
        print(f"{'PASS' if ok else 'FAIL'} [{section}] {name}" + (f": {detail}" if detail else ""), flush=True)
        return ok


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


def section10(report: Report) -> None:
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
                          "(§4.2, §8.1's reading applied)",
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


def section11(report: Report) -> None:
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


def sd_ns() -> int:
    return 1_000_000_000


def _t(ts: Any) -> str:
    return str(sd.Stamp.of(ts))


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="python -m zk2py.state_scenarios", description=__doc__.split("\n")[0])
    ap.parse_args(argv)
    report = Report()
    for name, fn in (("§10", section10), ("§11", section11)):
        try:
            fn(report)
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
    sys.exit(main())

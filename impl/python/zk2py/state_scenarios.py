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


def _t(ts: Any) -> str:
    return str(sd.Stamp.of(ts))


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="python -m zk2py.state_scenarios", description=__doc__.split("\n")[0])
    ap.parse_args(argv)
    report = Report()
    try:
        section10(report)
    except Exception as e:  # noqa: BLE001 - a section that cannot run is a failure
        report.check("§10", "the section ran", False, f"{type(e).__name__}: {e}")
    passed = sum(1 for r in report.rows if r[2])
    failed = len(report.rows) - passed
    print(f"state scenarios: {passed} passed, {failed} failed")
    for sec, name, ok, detail in report.rows:
        if not ok:
            print(f"FAIL [{sec}] {name}: {detail}")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())

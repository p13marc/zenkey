"""core ``scenarios/presence.md`` §7 to §9 (core 0.37): what a complete
presence read is, run from Python on zenoh-python 1.10.1 routers in
process, with zk2py's reader (``zk2py.live``, ``zk2py.presence``) beside
zenoh's own answers::

    python -m zk2py.presence_scenarios [--only 7|8|9|follower]

Each section observes zenoh's behaviour as the scenario states it, then
what zk2py's reader made of it:
- §7: a read through a session cut off, which zk2py does not make, and a
  read in flight when the last transport closes (F-185), which zk2py reads
  as not complete;
- §8: a withdrawal heard live, which zk2py's follower keeps unconfirmed
  (unobservable, never down) until a read made after the heal;
- §9: a read beside a liveliness subscriber, which zk2py refuses;
- ``follower``: zk2py's own, beyond the scenario (F-187): one read for a
  burst, none for a make-before-break re-mint, down only once a complete
  read shows none.

Exit 0 when every check passes, 1 otherwise. No owner example, no cargo:
the owners are token holders, as presence is all a section reads.
"""

from __future__ import annotations

import argparse
import os
import sys
import threading
import time
from typing import Any

from .alarms_scenarios import Report, _wait

INSTANCE = "zk2/lab/owner/@zk/instance/" + "a" * 16
SELECTOR = "zk2/*/*/@zk/**"


def _raw_read(session: Any, selector: str = SELECTOR, timeout: float = 1.0) -> tuple[bool, list[str], int, float]:
    """zenoh's own answer to a liveliness GET, bypassing zk2py's guards:
    (final reply came, token keys, error replies, seconds)."""
    import zenoh

    replies: list[Any] = []
    done = threading.Event()
    t0 = time.monotonic()
    session.liveliness().get(selector, zenoh.handlers.Callback(replies.append, done.set), timeout=timeout)
    finished = done.wait(timeout + 5.0)
    oks = [str(r.ok.key_expr) for r in replies if r.ok is not None]
    return finished, oks, sum(1 for r in replies if r.ok is None), time.monotonic() - t0


class Bus:
    """R1, an owner (a client holding an instance token), and a tool that
    reaches R1 through a link the test can cut and heal; the tool's watch
    session goes the same way."""

    def __init__(self) -> None:
        from . import live
        from .budget_scenarios import Link
        from .live_interop import _r1

        self.router, self.endpoint, _ = _r1()
        self.link = Link(self.endpoint)
        self.owner = live.open_client(self.endpoint)
        self.token: Any = None
        self.tool = live.open_client(self.link.endpoint)
        self.extra: list[Any] = []

    def start_owner(self, key: str = INSTANCE) -> Any:
        self.token = self.owner.liveliness().declare_token(key)
        return self.token

    def tool_reads(self, key: str = INSTANCE) -> bool:
        from . import live

        return any(f"zk2/{i['system']}/{i['service']}/@zk/instance/{i['instance']}" == key
                   for i in live.list_presence(self.tool).instances)

    def cut(self) -> bool:
        from . import live

        self.link.cut()
        return _wait(lambda: live.transports_held(self.tool) is False, 15.0)

    def heal(self) -> bool:
        from . import live

        self.link.heal()
        return _wait(lambda: live.transports_held(self.tool) is True, 20.0, 0.1)

    def close(self) -> None:
        self.link.close()


def section7(report: Report) -> None:
    """§7 A read through a session cut off."""
    from . import live

    sec = "§7 a read through a session cut off"
    bus = Bus()
    try:
        bus.start_owner()
        report.check(sec, "1. the tool reads the owner's instance token", _wait(bus.tool_reads, 5.0))
        report.check(sec, "2. cut: the tool's session holds no transport (Session.info.transports())", bus.cut())
        fin, oks, errs, secs = _raw_read(bus.tool)
        report.check(sec, "3. zenoh answers the GET at once: a final reply, no token and no error reply",
                     fin and not oks and errs == 0 and secs < 0.5, f"{len(oks)} tokens, {errs} errors, {secs:.3f} s")
        own = _raw_read(bus.router)
        report.check(sec, "3. R1's own session reads the owner's instance token: it never left", INSTANCE in own[1])
        p = live.list_presence(bus.tool)
        report.check(sec, "3. zk2py's reader makes no such read, and says why: not made, cut off",
                     p.kind == live.NOT_MADE and p.not_made == live.CUT_OFF, p.reading)
        report.check(sec, "4. heal: the tool's session holds a transport again", bus.heal())
        report.check(sec, "4. the tool's read holds the owner's instance token", _wait(bus.tool_reads, 5.0))
        # F-185: a read in flight when the session's last transport closes.
        bus.link.stall()
        got: dict[str, Any] = {}
        th = threading.Thread(target=lambda: got.setdefault("p", live.list_presence(bus.tool, timeout=5.0)))
        th.start()
        time.sleep(0.3)
        bus.link.cut()
        th.join(10.0)
        p = got.get("p")
        report.check(sec, "F-185: a read in flight when the link closes is answered at once with no token, and "
                          "zk2py's check at its end reads it not complete",
                     p is not None and not p.instances and p.cut_off_during and p.kind != live.COMPLETE,
                     None if p is None else p.reading)
        bus.heal()
    finally:
        bus.close()


def section8(report: Report) -> None:
    """§8 A withdrawal heard live, read by zk2py's follower."""
    from . import live
    from . import presence as pr

    sec = "§8 a withdrawal heard live"
    bus = Bus()
    try:
        heard: list[tuple[str, str]] = []
        watch = live.watch_session(bus.tool)
        sub = live.declare_liveliness_subscriber(watch, "zk2/*/*/@zk/instance/*",
                                                 lambda s: heard.append((str(s.kind), str(s.key_expr))),
                                                 history=True)
        bus.extra.append(sub)
        f = pr.PresenceFollower(bus.tool, watch, "zk2/*/*/@zk/instance/*")
        bus.extra.append(f)
        bus.start_owner()
        report.check(sec, "1. the subscriber hears the owner's instance token, and the tool reads it",
                     _wait(lambda: any("PUT" in k for k, _ in heard), 5.0) and _wait(bus.tool_reads, 5.0))
        report.check(sec, "1. zk2py's follower reads the owner up", _wait(lambda: f.address("lab", "owner")[0] == pr.UP, 5.0),
                     str(f.address("lab", "owner")))
        made_before = len(f.reads)
        bus.cut()
        report.check(sec, "2. on the cut, the subscriber hears the token withdrawn",
                     _wait(lambda: any("DELETE" in k for k, _ in heard), 5.0))
        report.check(sec, "2. R1's own session still reads it: the owner never left", INSTANCE in _raw_read(bus.router)[1])
        time.sleep(0.6)
        st = f.address("lab", "owner")
        report.check(sec, "2. zk2py's follower reads the owner unobservable (withdrawn, unconfirmed), never down, "
                          "and its re-read is not made while cut off",
                     st == (pr.UNOBSERVABLE, pr.WITHDRAWN) and all(p.kind == live.NOT_MADE for p in f.reads[made_before:])
                     and len(f.reads) > made_before, f"{st}, {[p.kind for p in f.reads[made_before:]]}")
        bus.heal()
        report.check(sec, "2. on the heal, the subscriber hears it again",
                     _wait(lambda: [k for k, _ in heard].count("SampleKind.PUT") >= 2, 10.0))
        report.check(sec, "2. the tool's read holds it, and the follower reads the owner up",
                     _wait(bus.tool_reads, 5.0) and _wait(lambda: f.address("lab", "owner")[0] == pr.UP, 5.0),
                     str(f.address("lab", "owner")))
        bus.token.undeclare()
        report.check(sec, "3. the owner stops cleanly: the subscriber hears the token withdrawn",
                     _wait(lambda: [k for k, _ in heard].count("SampleKind.DELETE") >= 2, 5.0))
        report.check(sec, "3. the tool's read after it is complete and holds no instance token of the owner, and the "
                          "follower reads it down only from that read",
                     _wait(lambda: f.address("lab", "owner")[0] == pr.DOWN, 5.0) and f.reads[-1].kind == live.COMPLETE,
                     str(f.address("lab", "owner")))
    finally:
        for x in bus.extra:
            try:
                x.close() if hasattr(x, "close") else x.undeclare()
            except Exception:  # noqa: BLE001 - closing anyway
                pass
        bus.close()


def section9(report: Report) -> None:
    """§9 A read beside a liveliness subscriber."""
    import zenoh

    from . import live

    sec = "§9 a read beside a liveliness subscriber"
    bus = Bus()
    try:
        bus.start_owner()
        report.check(sec, "1. the tool reads the owner's instance token", _wait(bus.tool_reads, 5.0))
        watch = live.open_client(bus.endpoint)
        heard: list[str] = []
        sub = watch.liveliness().declare_subscriber("zk2/*/*/@zk/instance/*",
                                                    zenoh.handlers.Callback(lambda s: heard.append(str(s.key_expr))),
                                                    history=True)
        fin, oks, errs, _ = _raw_read(watch)
        report.check(sec, "2. the watch session's GET, made at once beside its history subscriber, ends with the "
                          "final reply and no error reply, and without the owner's instance token",
                     fin and errs == 0 and INSTANCE not in oks, f"{len(oks)} tokens")
        report.check(sec, "2. the subscriber does hear it", _wait(lambda: INSTANCE in heard, 5.0))
        fin, oks, errs, _ = _raw_read(bus.tool)
        report.check(sec, "3. the tool's GET, on a session with no liveliness subscriber, holds it",
                     fin and errs == 0 and INSTANCE in oks)
        marked = live.open_client(bus.endpoint)
        m_sub = live.declare_liveliness_subscriber(marked, "zk2/*/*/@zk/instance/*", lambda s: None, history=True)
        p = live.list_presence(marked)
        report.check(sec, "3. zk2py's reader makes no read on a session that holds a liveliness subscriber it "
                          "declared, and reports it not made",
                     p.kind == live.NOT_MADE and p.not_made == live.BESIDE_SUBSCRIBER, p.reading)
        for x in (sub, m_sub):
            x.undeclare()
    finally:
        bus.close()


def section_follower(report: Report) -> None:
    """zk2py's follower beyond §8: a burst, a re-mint (core §8.1, 0.37)."""
    from . import live
    from . import presence as pr

    sec = "follower: a burst and a re-mint"
    bus = Bus()
    try:
        watch = live.watch_session(bus.tool)
        holder = live.open_client(bus.endpoint)
        tokens = {n: holder.liveliness().declare_token(f"zk2/lab/b{n}/@zk/instance/" + f"{n}" * 16)
                  for n in range(1, 4)}
        f = pr.PresenceFollower(bus.tool, watch, "zk2/*/*/@zk/instance/*", reread_delay_s=0.2)
        report.check(sec, "three owners up", _wait(lambda: all(f.address("lab", f"b{n}")[0] == pr.UP
                                                               for n in range(1, 4)), 5.0))
        before = len(f.reads)
        for t in tokens.values():
            t.undeclare()
        ok = _wait(lambda: all(f.address("lab", f"b{n}")[0] == pr.DOWN for n in range(1, 4)), 5.0)
        time.sleep(0.5)
        report.check(sec, "a burst of three withdrawals: one read, then each down", ok and len(f.reads) - before == 1,
                     f"{len(f.reads) - before} reads, {[f.address('lab', f'b{n}') for n in range(1, 4)]}")
        old = holder.liveliness().declare_token("zk2/lab/m/@zk/instance/" + "c" * 16)
        report.check(sec, "an owner up", _wait(lambda: f.address("lab", "m")[0] == pr.UP, 5.0))
        before = len(f.reads)
        new = holder.liveliness().declare_token("zk2/lab/m/@zk/instance/" + "d" * 16)
        time.sleep(0.3)
        old.undeclare()
        time.sleep(0.6)
        report.check(sec, "a make-before-break re-mint owes no read: no read made, the owner up throughout",
                     len(f.reads) == before and f.address("lab", "m")[0] == pr.UP and not f.view.owes_read,
                     f"{len(f.reads) - before} reads, {f.address('lab', 'm')}")
        new.undeclare()
        f.close()
    finally:
        bus.close()


SECTIONS = {"7": section7, "8": section8, "9": section9, "follower": section_follower}


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="python -m zk2py.presence_scenarios", description=__doc__.split("\n")[0])
    ap.add_argument("--only", action="append", choices=sorted(SECTIONS), help="run only these sections")
    args = ap.parse_args(argv)
    report = Report()
    for name, fn in SECTIONS.items():
        if args.only and name not in args.only:
            continue
        try:
            fn(report)
        except Exception as e:  # noqa: BLE001 - a section that crashes is a failure, reported
            report.check(name, "the section ran", False, repr(e))
    passed = sum(1 for r in report.rows if r[2])
    failed = len(report.rows) - passed
    print(f"presence scenarios §7–§9 (core 0.37): {passed} passed, {failed} failed")
    for sec, name, ok, detail in report.rows:
        if not ok:
            print(f"FAIL [{sec}] {name}: {detail}")
    return 1 if failed else 0


if __name__ == "__main__":
    rc = main()
    sys.stdout.flush()
    # zenoh-python can hang at interpreter exit with sessions open
    os._exit(rc)

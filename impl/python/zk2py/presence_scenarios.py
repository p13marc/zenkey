"""core ``scenarios/presence.md`` §4 and §7 to §9 (core 0.38): what a
complete presence read is, run from Python on zenoh-python 1.10.1 routers
in process, with zk2py's reader (``zk2py.live``, ``zk2py.presence``) beside
zenoh's own answers::

    python -m zk2py.presence_scenarios [--only 4|7|8|9]

Each section observes zenoh's behaviour as the scenario states it, then
what zk2py's reader made of it (0.38 asks the reader under test, F-188,
F-189):
- §4: the watch session's round trip beside a subscriber to 1,200 tokens,
  under the pairings zenoh-python can run in process;
- §7: a read through a session cut off, which zk2py does not make, and a
  read in flight when the link is cut (step 5, F-185), which zk2py reads,
  by the session's links, as possibly incomplete, cut off during it;
- §8: a withdrawal heard live, which zk2py's follower keeps unconfirmed
  (unobservable, never down) until a read made after the heal; one read for
  a burst; none for a make-before-break re-mint;
- §9: a read beside a liveliness subscriber, which zk2py refuses; the
  follower's own liveliness GETs per session; and step 5's stalled watch
  link, run against a control that skips the round trip.

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
        return _wait(lambda: live.transports_held(self.tool) is False and live.links_held(self.tool) is False, 15.0)

    def heal(self) -> bool:
        from . import live

        self.link.heal()
        return _wait(lambda: live.links_held(self.tool) is True, 20.0, 0.1)

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
        report.check(sec, "2. cut: the tool's session holds no transport, and no link", bus.cut())
        fin, oks, errs, secs = _raw_read(bus.tool)
        report.check(sec, "3. zenoh answers the GET at once: a final reply, no token and no error reply",
                     fin and not oks and errs == 0 and secs < 0.5, f"{len(oks)} tokens, {errs} errors, {secs:.3f} s")
        own = _raw_read(bus.router)
        report.check(sec, "3. R1's own session reads the owner's instance token: it never left", INSTANCE in own[1])
        p = live.list_presence(bus.tool)
        report.check(sec, "3. zk2py's reader makes no such read, and says why: not made, cut off",
                     p.kind == live.NOT_MADE and p.not_made == live.CUT_OFF, p.reading)
        report.check(sec, "4. heal: the tool's session holds a link again", bus.heal())
        report.check(sec, "4. the tool's read holds the owner's instance token", _wait(bus.tool_reads, 5.0))
        # Step 5 (0.38, F-185): stall the link, ask a read, cut it in flight.
        made, listed = 0, 0
        for run in range(10):
            bus.link.stall()
            got: dict[str, Any] = {}
            t0 = time.monotonic()
            th = threading.Thread(target=lambda: got.setdefault("p", live.list_presence(bus.tool, timeout=5.0)))
            th.start()
            time.sleep(0.3)
            t_cut = time.monotonic()
            bus.link.cut()
            th.join(10.0)
            p = got.get("p")
            at_cut = p is not None and p.elapsed_s < (t_cut - t0) + 0.5
            if (p is not None and at_cut and not p.instances and not p.errors and p.cut_off_during
                    and p.kind == live.POSSIBLY_INCOMPLETE and live.CUT_OFF_DURING in p.reading):
                made += 1
            if p is not None and p.cut_off_during and p.transports_at_end:
                listed += 1
            bus.heal()
            _wait(bus.tool_reads, 5.0)
        report.check(sec, "5. (0.38) a read asked while the link is stalled, and cut in flight, is answered at the "
                          "cut, no token, no error reply; the check before it passed, and the one at its end, by the "
                          "session's links, reads it possibly incomplete, cut off during it, never the owner absent",
                     made == 10, f"{made} of 10 runs")
        report.note(sec, f"5. at the read's end, the session's links were gone in each of those runs, and "
                         f"Session.info.transports() still listed the closing transport in {listed} of 10 "
                         "(core §8.1, 0.38, \"What the check reads\")")
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
        _wait(lambda: len(f.reads) > made_before, 5.0)
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
        # 4. three owners on three addresses, stopped at once
        tokens = {n: bus.owner.liveliness().declare_token(f"zk2/lab/b{n}/@zk/instance/" + f"{n}" * 16)
                  for n in range(1, 4)}
        up = _wait(lambda: all(f.address("lab", f"b{n}")[0] == pr.UP for n in range(1, 4)), 5.0)
        _wait(lambda: f.idle, 5.0)
        before = len(f.reads)
        for t in tokens.values():
            t.undeclare()
        ok = _wait(lambda: all(f.address("lab", f"b{n}")[0] == pr.DOWN for n in range(1, 4)), 5.0)
        ok = _wait(lambda: f.idle, 5.0) and ok
        report.check(sec, "4. three owners up, stopped at once: the reader makes one read for the three withdrawals, "
                          "and reports all three down from it", up and ok and len(f.reads) - before == 1,
                     f"{len(f.reads) - before} reads, {[f.address('lab', f'b{n}') for n in range(1, 4)]}")
        # 5. a make-before-break re-mint
        old_key, new_key = "zk2/lab/m/@zk/instance/" + "c" * 16, "zk2/lab/m/@zk/instance/" + "d" * 16
        old = bus.owner.liveliness().declare_token(old_key)
        _wait(lambda: f.address("lab", "m")[0] == pr.UP, 5.0)
        _wait(lambda: f.idle, 5.0)
        before = len(f.reads)

        def heard_it(key: str, kind: str) -> bool:
            with f.lock:
                return any(k == key and what == kind for _, k, what in f.view.events)

        states = []
        new = bus.owner.liveliness().declare_token(new_key)
        _wait(lambda: heard_it(new_key, "put"), 5.0)
        states.append(f.address("lab", "m")[0])
        old.undeclare()
        _wait(lambda: heard_it(old_key, "delete"), 5.0)
        states.append(f.address("lab", "m")[0])
        _wait(lambda: f.idle, 5.0)
        states.append(f.address("lab", "m")[0])
        report.check(sec, "5. a make-before-break re-mint: the reader makes no read, and the address stays up "
                          "throughout", len(f.reads) == before and set(states) == {pr.UP} and not f.view.owes_read,
                     f"{len(f.reads) - before} reads, states {states}")
        new.undeclare()
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
        _section9_reader(report, sec, bus)
    finally:
        bus.close()


class _Counted:
    """A session seen through: every liveliness GET it is asked to make is
    counted, and ``on_answer`` runs when the first one's final reply comes,
    before the reader sees it (presence.md §9 step 5: "withdrawn as soon as
    the reader's seed is answered")."""

    def __init__(self, inner: Any, on_answer: Any = None) -> None:
        self._inner, self.on_answer = inner, on_answer
        self.gets: list[tuple[float, str]] = []

    def liveliness(self) -> Any:
        import zenoh

        lv, outer = self._inner.liveliness(), self

        class _Lv:
            def get(self, selector: str, handler: Any, **kw: Any) -> Any:
                outer.gets.append((time.monotonic(), selector))
                hook, outer.on_answer = outer.on_answer, None
                if hook is not None:
                    drop = handler.drop

                    def dropped() -> None:
                        hook()
                        drop()

                    handler = zenoh.handlers.Callback(handler.callback, dropped)
                return lv.get(selector, handler, **kw)

            def __getattr__(self, name: str) -> Any:
                return getattr(lv, name)

        return _Lv()

    def __getattr__(self, name: str) -> Any:
        return getattr(self._inner, name)


def _section9_reader(report: Report, sec: str, bus: Bus) -> None:
    """§9 steps 4 and 5 (0.38, F-189): the reader under test, zk2py's
    follower, observed."""
    from . import live
    from . import presence as pr
    from .budget_scenarios import Link

    # 4. its liveliness GETs, per session
    watch = _Counted(live.open_client(bus.endpoint))
    reading = _Counted(live.open_client(bus.endpoint))
    f = pr.PresenceFollower(reading, watch, "zk2/*/*/@zk/instance/*")
    _wait(lambda: f.address("lab", "owner")[0] == pr.UP, 5.0)
    w_gets, r_gets = list(watch.gets), list(reading.gets)
    asked = live.list_presence(watch, "zk2/*/*/@zk/instance/*")
    report.check(sec, "4. (0.38) the reader's watch session makes one liveliness GET, the round trip, after its "
                      "subscribers and before the seed, and no other; the seed and every re-read go through the "
                      "reading session",
                 len(w_gets) == 1 and w_gets[0][1] == "zk2/@zk/settle" and r_gets and w_gets[0][0] < r_gets[0][0],
                 f"watch {[g[1] for g in w_gets]}, reading {len(r_gets)}")
    report.check(sec, "4. a read asked on its watch session is not made, beside a liveliness subscriber",
                 asked.kind == live.NOT_MADE and asked.not_made == live.BESIDE_SUBSCRIBER
                 and len(watch.gets) == len(w_gets), asked.reading)
    f.close()
    for x in (watch._inner, reading._inner):
        live.close(x)
    # 5. the watch link stalled from before the subscribers until 150 ms later
    results = {True: [0, 0], False: [0, 0]}
    for round_trip in (True, False):
        for run in range(10):
            key = f"zk2/lab/s9r{run}/@zk/instance/" + "e" * 16
            holder = live.open_client(bus.endpoint)
            tok = holder.liveliness().declare_token(key)
            time.sleep(0.2)
            link = Link(bus.endpoint)
            w = live.open_client(link.endpoint)
            r = _Counted(live.open_client(bus.endpoint), on_answer=tok.undeclare)
            link.stall()
            threading.Timer(0.15, link.flowing.set).start()
            f = pr.PresenceFollower(r, w, f"zk2/lab/s9r{run}/@zk/instance/*", round_trip=round_trip)
            down = _wait(lambda: f.address("lab", f"s9r{run}")[0] == pr.DOWN, 3.0)
            with f.lock:
                heard = any(k == key and what == "delete" for _, k, what in f.view.events)
            results[round_trip][0] += 1
            results[round_trip][1] += 1 if (heard and down) else 0
            f.close()
            live.close(w)
            live.close(r._inner)
            live.close(holder)
            link.close()
    report.check(sec, "5. (0.38) a fresh reader, its watch link stalled 150 ms from before its subscribers: it hears "
                      "the token go, withdrawn as soon as its seed is answered, and reports the owner down after its "
                      "re-read, in every run",
                 results[True][1] == results[True][0], f"{results[True][1]} of {results[True][0]} runs")
    report.check(sec, "5. the control, a reader that skips the round trip, never hears it go, and goes on reading "
                      "the owner up, in every run", results[False][1] == 0,
                 f"it heard the withdrawal in {results[False][1]} of {results[False][0]} runs")


def section4(report: Report) -> None:
    """§4 (0.38, F-186): the watch session's round trip beside a liveliness
    subscriber to at least 1,200 tokens, each GET waiting 5 s; its answer
    discarded, so no token is counted."""
    import zenoh

    from . import live

    sec = "§4 reading presence at scale"
    from .live_interop import _r1

    router, endpoint, _ = _r1()
    holder = live.open_client(endpoint)
    tokens = [holder.liveliness().declare_token(f"zk2/scale/t{n}/@zk/instance/" + f"{n:016x}")
              for n in range(1200)]
    time.sleep(2.0)
    try:
        def round_trip(sub_callback: bool, get_callback: bool) -> tuple[bool, float, int]:
            w = live.open_client(endpoint)
            heard: list[Any] = []
            if sub_callback:
                sub = w.liveliness().declare_subscriber("zk2/*/*/@zk/instance/*",
                                                        zenoh.handlers.Callback(heard.append), history=True)
            else:
                sub = w.liveliness().declare_subscriber("zk2/*/*/@zk/instance/*", history=True)  # never drained
            t0 = time.monotonic()
            errors = 0
            if get_callback:
                done = threading.Event()
                errs: list[Any] = []
                w.liveliness().get("zk2/@zk/settle", zenoh.handlers.Callback(
                    lambda r: errs.append(r) if r.ok is None else None, done.set), timeout=5.0)
                finished = done.wait(15.0)
                errors = len(errs)
            else:
                replies = w.liveliness().get("zk2/@zk/settle", timeout=5.0)  # drained as replies arrive
                finished = True
                for rep_ in replies:
                    if rep_.ok is None:
                        errors += 1
            dt = time.monotonic() - t0
            sub.undeclare()
            live.close(w)
            return finished and errors == 0 and dt < 5.0, dt, errors

        ok, dt, errs = round_trip(True, True)
        report.check(sec, "callbacks for both: the round trip completes, at the routers' final reply, before its "
                          "timeout; its answer discarded", ok, f"{dt:.2f} s, {errs} error replies")
        ok, dt, errs = round_trip(True, False)
        report.check(sec, "a GET handler drained as replies arrive, beside a callback subscriber: it completes too",
                     ok, f"{dt:.2f} s, {errs} error replies")
        ok, dt, errs = round_trip(False, True)
        report.check(sec, "a bounded subscriber nobody drains: a callback round trip ends at its timeout, an error "
                          "reply, which a tool reads possibly incomplete",
                     not ok and errs >= 1 and dt >= 4.9, f"{dt:.2f} s, {errs} error replies")
        report.note(sec, "not run: a GET on a bounded handler beside an undrained bounded subscriber, which hangs "
                         "(round 54 measured 0 replies after 20 s), and wedges zenoh's threads in process, as the "
                         "reference found")
    finally:
        for t in tokens:
            t.undeclare()
        live.close(holder)
        router.close()


SECTIONS = {"4": section4, "7": section7, "8": section8, "9": section9}


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
    print(f"presence scenarios §4, §7–§9 (core 0.38): {passed} passed, {failed} failed")
    for sec, name, ok, detail in report.rows:
        if not ok:
            print(f"FAIL [{sec}] {name}: {detail}")
    return 1 if failed else 0


if __name__ == "__main__":
    rc = main()
    sys.stdout.flush()
    # zenoh-python can hang at interpreter exit with sessions open
    os._exit(rc)

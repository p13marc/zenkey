"""``config.v1`` 0.13 §2.12's re-mint, live, in its order (#609, the mend
after round 104): ``python -m zk2py.config_remint_live``.

zk2py's :class:`zk2py.config.ConfigOwner` of ``lab/rf0``, a client of a
router of the runner's own, its device simulated
(:class:`zk2py.config.SimDevice`):

1. **An act in flight**: "A re-mint asked while an act is in flight is
   refused before step 1": the pending change is still pending.
2. **Step 1's undo held**: "Each step ends before the next starts. The
   re-mint waits for step 1's undo, one act (§2.11), as a ``cancel``'s reply
   does (§2.10)": the device holds the undo, the re-mint waits, and once the
   undo completes the new instance starts; the change's event, ``rolled-back``,
   went out under the old instance.
3. **A device that refuses to return**: "A re-mint refused after step 1
   leaves steps 1 and 2 done. The change stays ended, and its event stands";
   the revision stays, and the view and status last put are the ended
   change's.
4. **Step 1's undo past zk2py's bound** (review-bot on #1090): the wait
   gives the lock up, the re-mint is refused with the undo in flight, and
   the change ends when the undo completes.

Exit 0 when every check passes, 1 when any fails.
"""

from __future__ import annotations

import sys
import threading
import time
from typing import Any

from . import config as cf


class Device(cf.SimDevice):
    """A simulated device whose return to its startup values can be refused."""

    refuse_boot = False

    def boot(self, values: dict[str, dict[str, Any]]) -> None:
        if self.refuse_boot:
            raise RuntimeError("the radio will not return to its startup values")
        super().boot(values)


def _pending(o: cf.ConfigOwner, ppm: int) -> bool:
    with o.cv:
        o._after(o.core.set("duty", {"values": {"fraction_ppm": ppm}, "confirm_s": 60}, time.time_ns()))  # noqa: SLF001
    end = time.monotonic() + 10.0
    while time.monotonic() < end:
        with o.cv:
            p = o.core.pending
            if p is not None and p.all_applied:
                return True
        time.sleep(0.1)
    return False


def run() -> list[tuple[str, bool, str]]:
    from .config_scenarios import STARTUP, rf_schema
    from .live_interop import _r1
    from .owner import OwnerRefused

    out: list[tuple[str, bool, str]] = []
    r, ep, _ = _r1(timestamping=False)
    dev = Device()
    o = cf.ConfigOwner("lab", "rf0", schema=rf_schema(), startup=STARTUP, floor_s=10, apply_after_s=0.2,
                       connect=ep, device=dev)
    try:
        o.start()
        time.sleep(0.3)
        # 1. an act in flight, refused before step 1
        applied = _pending(o, 20000)
        token = o.core.pending.token if o.core.pending else None
        with o.cv:
            o.core.in_flight = ("apply", lambda result: cf.Step())
        try:
            o.remint()
            refused = ""
        except OwnerRefused as e:
            refused = str(e)
        with o.cv:
            o.core.in_flight = None
        out.append(("1: a re-mint asked while an act is in flight is refused before step 1: the change still pending",
                    applied and "refused before step 1" in refused and o.core.pending is not None
                    and o.core.pending.token == token, f"{refused!r}; pending {o.core.pending and o.core.pending.token}"))
        # 2. step 1's undo held: the re-mint waits for it
        old, events0 = o.instance, len(o.events)
        dev.next_undo.append(cf.HOLD)
        got: dict[str, Any] = {}

        def remint() -> None:
            t0 = time.monotonic()
            try:
                got["new"] = o.remint()
            except Exception as e:  # noqa: BLE001 - named
                got["error"] = f"{type(e).__name__}: {e}"
            got["took"] = time.monotonic() - t0

        th = threading.Thread(target=remint)
        th.start()
        time.sleep(1.0)
        waiting = th.is_alive() and o.core.in_flight is not None
        o.complete("ok")
        th.join(30.0)
        ev = [e.get("outcome") for e in o.events[events0:]]
        out.append(("2: the re-mint waits for step 1's undo, which the device held; once it completes the change has "
                    "ended rolled-back under the old instance and the new instance starts",
                    waiting and "new" in got and got["new"] != old and o.instance == got["new"]
                    and got["took"] >= 1.0 and "rolled-back" in ev and o.core.pending is None,
                    f"waiting {waiting}; {got}; events {ev}"))
        # 3. a device that refuses to return: the change stays ended
        _pending(o, 30000)
        inst, rev = o.instance, None
        dev.refuse_boot = True
        events0 = len(o.events)
        try:
            o.remint()
            refused = ""
        except OwnerRefused as e:
            refused = str(e)
        dev.refuse_boot = False
        with o.cv:
            rev = o.core.revision
            last = (cf.encode_json(o.core.view()), cf.encode_json(o.core.status()))
            consistent = o._last_put == last  # noqa: SLF001
        ev = [e.get("outcome") for e in o.events[events0:]]
        out.append(("3: a device that refuses to return refuses the re-mint after step 1: the change stays ended, its "
                    "event stands, the instance runs on, and the view and status last put are the ended change's",
                    "refuses to return" in refused and o.core.pending is None and o.instance == inst
                    and "rolled-back" in ev and consistent,
                    f"{refused!r}; events {ev}; revision {rev}; last put consistent {consistent}"))
        # 4. review-bot on #1090, 1: step 1's undo that does not complete
        # within zk2py's bound. The wait releases the cv (an RLock, released
        # whole by Condition.wait), so the timer, the handlers and the act's
        # own completion take it meanwhile; past the bound the re-mint is
        # refused with the undo in flight, the instance running on, as one
        # asked while an act is in flight is (§2.12), and the change ends when
        # the undo completes, as a cancel's does (§2.10).
        _pending(o, 40000)
        inst, events0 = o.instance, len(o.events)
        o.remint_undo_wait_s = 1.5
        dev.next_undo.append(cf.HOLD)
        got = {}
        th = threading.Thread(target=remint)
        th.start()
        time.sleep(0.3)
        t0 = time.monotonic()
        took_cv: list[float] = []

        def grab() -> None:
            with o.cv:
                took_cv.append(time.monotonic() - t0)

        g = threading.Thread(target=grab)
        g.start()
        g.join(5.0)
        th.join(10.0)
        in_flight_after = o.core.in_flight is not None
        ev_before = [e.get("outcome") for e in o.events[events0:]]
        o.complete("ok")
        ev = [e.get("outcome") for e in o.events[events0:]]
        out.append(("4 (review-bot on #1090): step 1's undo held past zk2py's bound: the wait gives up the lock, so "
                    "another thread takes it at once; the re-mint is refused with the undo in flight and the instance "
                    "runs on; the change ends, rolled-back, when the undo completes",
                    bool(took_cv) and took_cv[0] < 1.0 and "still in flight" in got.get("error", "")
                    and o.instance == inst and in_flight_after and "rolled-back" not in ev_before
                    and "rolled-back" in ev and o.core.in_flight is None and o.core.pending is None,
                    f"lock taken after {took_cv}; {got}; in flight {in_flight_after}; events {ev_before} then {ev}"))
        o.remint_undo_wait_s = cf.ConfigOwner.remint_undo_wait_s
    except Exception as e:  # noqa: BLE001 - a run that could not finish is a failure
        out.append(("the run finished", False, f"{type(e).__name__}: {e}"))
    finally:
        o.close()
        r.close()
    return out


def main() -> int:
    failed = 0
    for name, ok, detail in run():
        print(f"{'PASS' if ok else 'FAIL'} {name}" + (f": {detail}" if detail else ""), flush=True)
        failed += not ok
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())

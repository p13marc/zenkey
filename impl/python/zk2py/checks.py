"""A live runner's checks and its exit, shared by ``zk2py.timing_interop`` and
``zk2py.arbitration_scenarios`` (#609, round 79; review-bot on PR #919), and
what ``zk2py.state_scenarios`` and ``zk2py.live_interop`` share to hold a
GET open (:class:`HeldQueries`; round 92, review-bot on PR #973) and to judge
``state.md`` §7 step 2's premise (:func:`premise`; core 0.70).

A runner keeps each check as ``(section, name, ok, detail, known)``, where
``known`` is the open finding the check is known to fail by, looked up by
the check's identity, ``(section, name)``, in the runner's own known set,
never inferred from a count. :func:`summarise` prints the run's summary,
every failure, and every known check that passed (XPASS), and gives the
exit: 1 when any check fails that the known set does not name.
"""

from __future__ import annotations

import threading
from typing import Any

Row = tuple[str, str, bool, str, "str | None"]

#: ``state.md`` §7 step 2's premise, stated since core 0.70 (F-408): "Rev 11
#: and rev 12 come within 5 s of rev 10."
PREMISE_S = 5.0


def premise(offset_s: float, t10: Any, t11: Any, t12: Any, elapsed_s: float) -> tuple[bool, str]:
    """Whether a run of ``state.md`` §7 step 2 met its premise, "Rev 11 and
    rev 12 come within 5 s of rev 10" (core 0.70, F-408), and the words that
    say why. ``t10``, ``t11`` and ``t12`` are the three values' stamps
    (:class:`zk2py.seed.Stamp`, or None when one was not heard), and
    ``elapsed_s`` rev 12's write after rev 10's on the runner's monotonic
    clock.

    The premise is judged as it is stated, on the runner's clock, whatever
    the new clock's offset ``offset_s``: rev 12 written within
    :data:`PREMISE_S` of rev 10. The step writes rev 11 before rev 12, so
    rev 11 then comes within it too. The stamps are said beside it, never
    judged: with the clock behind by 5 s, a run within the premise gives
    rev 11 and rev 12 times below rev 10's, and the tool's outcome, asserted
    after it, rests on that (review-bot on PR #994: the premise had been
    read from the stamps, which holds only at an offset of exactly 5 s).

    Round 92 read a run past it as inconclusive, the step then stating no
    premise. Since 0.70 states one, a run that misses it fails on it."""
    if t10 is None or t11 is None or t12 is None:
        return False, f"a value not heard: rev 10 {t10}, rev 11 {t11}, rev 12 {t12}"
    ok = elapsed_s < PREMISE_S
    gap11 = round((t11.time - t10.time) / 2**32, 3)
    gap12 = round((t12.time - t10.time) / 2**32, 3)
    return ok, (f"rev 12 written {elapsed_s:.3f} s after rev 10 by the runner's clock, "
                f"{'within' if ok else 'not within'} {PREMISE_S:g} s; rev 11's time {gap11} s and rev 12's "
                f"{gap12} s from rev 10's, the new clock {offset_s:+g} s")


class HeldQueries:
    """A queryable's handler that answers nothing and keeps each query until
    :meth:`release`, whose drop is the GET's final reply (state.md §7, "keeps
    each query it takes", then "lets the GET end"). A runner releases it in a
    ``finally`` too, so that an early return leaves no GET open; the timer,
    after ``limit`` seconds, is the last resort. :meth:`release` is
    idempotent, and a query taken after it is dropped at once."""

    def __init__(self, limit: float = 30.0) -> None:
        self.lock = threading.Lock()
        self.queries: list[Any] = []
        self.done = False
        self.timer = threading.Timer(limit, self.release)
        self.timer.daemon = True
        self.timer.start()

    def take(self, query: Any) -> None:
        with self.lock:
            if not self.done:
                self.queries.append(query)
                return
        query.drop()

    def release(self) -> None:
        with self.lock:
            self.done, queries, self.queries = True, self.queries, []
        self.timer.cancel()
        for q in queries:
            q.drop()


def tag(ok: bool, known: str | None) -> str:
    """PASS and FAIL, or for a known check XPASS and XFAIL."""
    return ("XPASS" if ok else "XFAIL") if known else ("PASS" if ok else "FAIL")


def summarise(title: str, rows: list[Row], tail: str) -> int:
    """The summary line, then each failure and each XPASS; exit 1 on a
    failing check no entry of the known set names, else 0. A known check
    that passes fails nothing, and is printed so its entry is removed."""
    passed = sum(1 for r in rows if r[2])
    xfail = [r for r in rows if not r[2] and r[4]]
    unexpected = [r for r in rows if not r[2] and not r[4]]
    xpass = [r for r in rows if r[2] and r[4]]
    print(f"{title}: {passed} passed, {len(unexpected) + len(xfail)} failed"
          + (f" ({len(xfail)} of them known: {', '.join(sorted({str(r[4]) for r in xfail}))})" if xfail else "")
          + (f", {len(xpass)} known to fail passed (XPASS)" if xpass else "") + f"; {tail}")
    for sec, name, ok, detail, why in rows:
        if not ok:
            print(f"{'XFAIL' if why else 'FAIL'} [{sec}] {name}" + (f" (known: {why})" if why else "") + f": {detail}")
        elif why:
            print(f"XPASS [{sec}] {name} (known: {why}): passes now; remove its entry: {detail}")
    return 1 if unexpected else 0


__all__ = ["Row", "tag", "summarise"]

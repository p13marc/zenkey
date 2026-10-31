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

    - **With the new clock behind** by ``offset_s`` (5 s), the premise is
      that rev 11's and rev 12's times fall below rev 10's: a clock 5 s
      behind stamps them so only while they come within 5 s of rev 10, as
      the step says. That is what the reference asserts before the tool's
      outcome ("A run past the premise then fails on the premise", core
      0.70's entry). The stamps measure it exactly, so no margin is needed.
    - **On the host's clock**, the stamps cannot say it, so it is rev 12's
      write within 5 s of rev 10's, on the runner's clock.

    Round 92 read a run past it as inconclusive, the step then stating no
    premise. Since 0.70 states one, a run that misses it fails on it."""
    if t10 is None or t11 is None or t12 is None:
        return False, f"a value not heard: rev 10 {t10}, rev 11 {t11}, rev 12 {t12}"
    gap = round((t12.time - t10.time) / 2**32, 3)
    said = f"rev 12 written {elapsed_s:.3f} s after rev 10 by the runner's clock, its time {gap} s from rev 10's"
    if offset_s:
        ok = t11.time < t10.time and t12.time < t10.time
        return ok, f"{said}; rev 11's and rev 12's times {'below' if ok else 'not both below'} rev 10's"
    return elapsed_s < PREMISE_S, said


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

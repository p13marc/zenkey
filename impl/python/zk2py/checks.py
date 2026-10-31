"""A live runner's checks and its exit, shared by ``zk2py.timing_interop`` and
``zk2py.arbitration_scenarios`` (#609, round 79; review-bot on PR #919), and
what ``zk2py.state_scenarios`` and ``zk2py.live_interop`` share to hold a
GET open (:class:`HeldQueries`; round 92, review-bot on PR #973).

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

#: How far inside the offset the runner's own clock must place rev 12's write
#: for ``state.md`` §7's clock-behind outcome to be asserted (F-408): the
#: owner's write and the runner's reading of the time are not one instant.
F408_MARGIN_S = 0.5


def within_offset(elapsed_s: float, offset_s: float, margin_s: float = F408_MARGIN_S) -> bool:
    """``state.md`` §7 step 2 with the new clock behind by ``offset_s``
    (round 92's F-408): whether rev 12 was written within the offset of
    rev 10, by the runner's monotonic clock, ``elapsed_s`` after it and with
    ``margin_s`` to spare. Only then is the step's "rev 10 with the clock
    behind" its premise's outcome, which a run asserts. Past it, or within
    the margin of it, the run is inconclusive, never a pass."""
    return elapsed_s < abs(offset_s) - margin_s


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

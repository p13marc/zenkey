"""A live runner's checks and its exit, shared by ``zk2py.timing_interop`` and
``zk2py.arbitration_scenarios`` (#609, round 79; review-bot on PR #919).

A runner keeps each check as ``(section, name, ok, detail, known)``, where
``known`` is the open finding the check is known to fail by, looked up by
the check's identity, ``(section, name)``, in the runner's own known set,
never inferred from a count. :func:`summarise` prints the run's summary,
every failure, and every known check that passed (XPASS), and gives the
exit: 1 when any check fails that the known set does not name.
"""

from __future__ import annotations

Row = tuple[str, str, bool, str, "str | None"]


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

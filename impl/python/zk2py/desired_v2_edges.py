"""desired.v2's four edges round 118 named (F-577 and F-585, F-581, F-582,
F-578), on real sessions, against each implementation (#609, round 119)::

    python -m zk2py.desired_v2_edges [--target zk2py|rust] [--author zk2py|rust]
        [--only done|failed|during|record]...

The first three are a **target's**. A session of the runner's own, P, plays
the author as ``scenarios-v2.md`` §13's does ("A played author"), under the
16-bit run R, so that each target is given the same bytes:

- ``done``: the target converged on v1 at (R, 7); P answers its next re-read
  with a snapshot with no entry, asof 9. What the target reports (§2.8,
  §2.10), what a reader concludes of it beside that snapshot (§2.17, F-585),
  the stamp its later re-reads carry (§2.5, F-581), and what it does with a
  version at exactly the asof, (R, 9), below it and above it (§2.6 step 4
  against §2.8, F-582).
- ``failed``: the same, the drive failing that revert (F-577): the report's
  bytes, whether they are consistent (§2.10), what a reader concludes, and
  the stamp of the re-reads after it.
- ``during``: the same, the drive holding that revert at its start: the
  stamp a re-read carries while the snapshot with no entry is acted on
  (F-581).

The fourth is an **author's**:

- ``record``: an author that keeps its run's record puts v1, ends without
  closing, loses that record's file, and starts again on the same path
  (core §4.3, F-578): the run its next put carries.

Nothing here is a verdict on either implementation: each row says what one
did. Where the two differ, the text has to decide.
"""

from __future__ import annotations

import argparse
import json
import sys
import time
from typing import Any

from . import desired, desired_v2 as d2, zk2c
from .alarms_scenarios import Report, _wait
from .desired_v2_scenarios import (AUTHOR, BASELINE, D, RP, RUN, SCRATCH, TARGET, Bus, Played, converged,
                                   impl_name, v)

ASOF = 9


def _raw_report(bus: Bus) -> bytes:
    st = bus.get(RP)
    return next((r.raw for r in st.replies if r.key == RP and r.raw), b"")


def _reading(raw: bytes) -> dict[str, Any]:
    """One report as zk2py's reader reads it (§2.10): its record's bytes,
    its fields, its findings, and whether it writes back to those bytes."""
    if not raw:
        return {"record": None}
    rd = d2.read_put(raw)
    record = zk2c.parse(raw, "put").record
    out: dict[str, Any] = {"record": record.hex(), "bytes": len(record), "readable": rd.readable,
                           "findings": list(rd.findings), "fields": rd.report}
    if rd.readable and rd.report is not None and not rd.findings:
        out["reads_back"] = d2.write_report(rd.report) == record
    return out


def _concludes(raw: bytes, cur: Any) -> dict[str, list[str]]:
    """§2.17, §5: what a tool answers of the target from that report, its
    author present and answering ``cur`` for the slice."""
    rd = d2.read_put(raw) if raw else None
    r = d2.Reading(desired.PRESENT, (True, True), rd, "fresh", desired.PRESENT, cur)
    return {q: [a.answer, a.reason] for q, a in d2.conclude(r).items()}


def _stamp(payload: bytes | None) -> str | None:
    """A GET's held stamp, shown as a version, or None for a GET that
    carried none."""
    if not payload:
        return None
    env = zk2c.parse(payload, "snapshot")
    return d2.Version.of(env.stamp.width, env.stamp.run, env.stamp.seq).text


def _setup(bus: Bus, p: Played, impl: str, flags: tuple[str, ...] = ()) -> tuple[Any, str]:
    p.default_doc, p.default_seq = v(1), 7
    p.script = [("snapshot", v(1), 7), ("held",)]
    target = bus.target(impl, reread_ms=2000, flags=flags)
    s7 = d2.Version.of(16, p.R, 7).text
    _wait(lambda: target.report() == converged(s7, 1), 10.0, 0.05)
    return target, s7


def edge_done(rp: Report, impl: str) -> dict[str, Any]:
    sec = f"a revert on a snapshot with no entry [{impl_name(impl)} target]"
    bus = Bus()
    p = Played(bus)
    out: dict[str, Any] = {}
    try:
        target, s7 = _setup(bus, p, impl)
        rp.check(sec, "setup: converged on v1 at (R, 7)", target.report() == converged(s7, 1), str(target.report()))
        acts0, n0 = target.acts(), p.count()
        p.script = [("snapshot", None, ASOF)]
        ok = _wait(lambda: bus.run_doc() == BASELINE, 8.0, 0.05)
        time.sleep(0.5)
        raw = _raw_report(bus)
        out["report"] = _reading(raw)
        out["acts"] = (target.acts() or 0) - (acts0 or 0)
        rp.check(sec, "P answers a snapshot with no entry under R, asof 9: the target reverts, and Run holds the "
                      "baseline (§2.8)", ok and out["acts"] == 1, json.dumps(out["report"], sort_keys=True))
        out["concludes"] = _concludes(raw, d2.NoEntry(d2.Run(16, p.R), ASOF))
        rp.note(sec, f"a reader of that report, beside the author's snapshot with no entry under R, answers "
                     f"{json.dumps(out['concludes'], sort_keys=True)} (§2.17, F-585)")
        n1 = p.count()
        _wait(lambda: p.count() >= n1 + 2, 6.0)
        out["stamps_after"] = [_stamp(g[1]) for g in p.gets[n1:p.count()]]
        rp.note(sec, f"its re-reads after the revert carry {out['stamps_after']} (§2.5, F-581)")
        tag = 0

        def x_put(doc: Any, seq: int) -> None:
            bus.x.put(D, zk2c.mutation(zk2c.InBand(16, p.R, seq), json.dumps(doc).encode(), tag),
                      encoding=zk2c.ENCODING)

        steps = []
        for seq, doc in ((ASOF - 1, v(2)), (ASOF, v(2)), (ASOF + 1, v(2))):
            a0, t0 = target.acts(), time.monotonic()
            x_put(doc, seq)
            time.sleep(1.2)
            steps.append({"seq": seq, "acts": (target.acts() or 0) - (a0 or 0),
                          "rp_puts": len(bus.since(t0, RP)), "run_puts": len(bus.since(t0, RUN)),
                          "word": target.word()})
        out["at_asof"] = steps
        rp.check(sec, "a version under R below the asof and at exactly the asof is not acted on, and one above it "
                      "is (§2.8, §2.6 step 4)", [s["acts"] for s in steps] == [0, 0, 1], json.dumps(steps))
        rp.note(sec, f"the report after (R, {ASOF + 1}): {json.dumps(_reading(_raw_report(bus)), sort_keys=True)}")
    finally:
        p.close()
        bus.close()
    return out


def edge_failed(rp: Report, impl: str) -> dict[str, Any]:
    sec = f"a revert on a snapshot with no entry that fails [{impl_name(impl)} target]"
    bus = Bus()
    p = Played(bus)
    out: dict[str, Any] = {}
    try:
        target, s7 = _setup(bus, p, impl)
        rp.check(sec, "setup: converged on v1 at (R, 7)", target.report() == converged(s7, 1), str(target.report()))
        out["fail_next"] = target.cmd("desired fail-next")
        acts0, t0 = target.acts(), time.monotonic()
        p.script = [("snapshot", None, ASOF)]
        moved = _wait(lambda: bool(bus.since(t0, RP)), 8.0, 0.05)
        time.sleep(0.5)
        raw = _raw_report(bus)
        out["report"] = _reading(raw)
        out["acts"] = (target.acts() or 0) - (acts0 or 0)
        out["run"] = bus.run_doc()
        out["own_word"] = target.report()
        rp.check(sec, "the drive fails the revert a snapshot with no entry began: the target puts a report",
                 moved and out["acts"] == 1, json.dumps(out["report"], sort_keys=True))
        rp.note(sec, f"Run holds {out['run']}; the target's own word of its report: {out['own_word']}")
        out["concludes"] = _concludes(raw, d2.NoEntry(d2.Run(16, p.R), ASOF))
        rp.note(sec, f"a reader of that report answers {json.dumps(out['concludes'], sort_keys=True)} (§2.17)")
        n1 = p.count()
        _wait(lambda: p.count() >= n1 + 2, 6.0)
        out["stamps_after"] = [_stamp(g[1]) for g in p.gets[n1:p.count()]]
        rp.note(sec, f"its re-reads after the failed revert carry {out['stamps_after']} (§2.5, F-581)")
        time.sleep(2.5)
        out["report_later"] = _reading(_raw_report(bus))
        out["acts_later"] = (target.acts() or 0) - (acts0 or 0)
        rp.note(sec, f"4 s on, P still answering no entry: {out['acts_later']} acts in all, the report "
                     f"{out['report_later'].get('record')}")
    finally:
        p.close()
        bus.close()
    return out


def edge_during(rp: Report, impl: str) -> dict[str, Any]:
    sec = f"the stamp a read carries while a snapshot with no entry is acted on [{impl_name(impl)} target]"
    bus = Bus()
    p = Played(bus)
    out: dict[str, Any] = {}
    try:
        target, s7 = _setup(bus, p, impl)
        rp.check(sec, "setup: converged on v1 at (R, 7)", target.report() == converged(s7, 1), str(target.report()))
        out["hold"] = target.cmd("desired hold")
        n0 = p.count()
        p.script = [("snapshot", None, ASOF)]
        _wait(lambda: p.count() >= n0 + 1, 6.0)
        held = _wait(lambda: target.held(), 3.0, 0.05)
        n1 = p.count()
        _wait(lambda: p.count() >= n1 + 2, 6.0)
        out["stamps_during"] = [_stamp(g[1]) for g in p.gets[n1:p.count()]]
        out["report_during"] = _reading(_raw_report(bus))
        rp.check(sec, "the drive holds the revert at its start, and the target re-reads on its period meanwhile",
                 held and len(out["stamps_during"]) >= 1, f"{out['stamps_during']}")
        rp.note(sec, f"while the revert runs its re-reads carry {out['stamps_during']} (§2.5, F-581); its report "
                     f"is {out['report_during'].get('record')}")
        target.cmd("desired release")
        _wait(lambda: bus.run_doc() == BASELINE, 5.0, 0.05)
        n2 = p.count()
        _wait(lambda: p.count() >= n2 + 2, 6.0)
        out["stamps_after"] = [_stamp(g[1]) for g in p.gets[n2:p.count()]]
        out["report_after"] = _reading(_raw_report(bus))
        rp.note(sec, f"released: its re-reads carry {out['stamps_after']}, and its report is "
                     f"{out['report_after'].get('record')}")
    finally:
        p.close()
        bus.close()
    return out


def edge_record(rp: Report, impl: str) -> dict[str, Any]:
    """The target is the other implementation's, so that the run reused is
    read by a target that did not write it."""
    other = "rust" if impl == "zk2py" else "zk2py"
    sec = f"a run's record that is gone [{impl_name(impl)} author, {impl_name(other)} target]"
    bus = Bus()
    out: dict[str, Any] = {}
    try:
        author = bus.author(impl=impl)
        target = bus.target(other)
        s1 = author.put(TARGET, v(1))
        _wait(lambda: target.report() == converged(s1, 1), 5.0)
        s2 = author.put(TARGET, v(2))
        _wait(lambda: target.report() == converged(s2, 2), 5.0)
        out["target_before"] = target.report() == converged(s2, 2)
        acts0 = target.acts()
        record = bus.run_record_path()
        out["first"] = [s1, s2]
        out["record_file"] = record.is_file()
        author.kill()
        if record.is_file():
            out["record_held"] = record.read_text()[:200]
            record.unlink()
        for f in bus.dir.glob("author.record*"):
            f.unlink()
        t0 = time.monotonic()
        author = bus.author(impl=impl)
        out["started"] = author.instance is not None
        out["stderr"] = "".join(author.stderr).strip()[:300]
        s3 = author.put(TARGET, v(3)) if author.instance else None
        out["after"] = s3
        kind, ver, _ = bus.entry(D)
        out["get"] = [kind, ver]
        a, b = d2.Version.parse(s2), d2.Version.parse(s3)
        reused = a is not None and b is not None and a.run == b.run
        out["reused"] = reused
        rp.check(sec, "the author kept a record of its run, a file; it is removed while the author is down, and the "
                      "author starts again on the same path", out["record_file"] and out["started"],
                 f"{out['stderr']}")
        rp.note(sec, f"before: v1 at {s1}, v2 at {s2}; after the record went: v3 at {s3} "
                     f"({'the same run again' if reused else 'another run'}"
                     f"{', at or below a seq it wrote before' if reused and b.seq <= a.seq else ''}) "
                     f"(core §4.3, F-578)")
        time.sleep(2.5)
        steps = [{"put": "v3", "at": s3, "acts": (target.acts() or 0) - (acts0 or 0),
                  "converged_on_it": target.report() == converged(s3, 3)}]
        for n in (4, 5):
            sn = author.put(TARGET, v(n))
            time.sleep(1.5)
            steps.append({"put": f"v{n}", "at": sn, "acts": (target.acts() or 0) - (acts0 or 0),
                          "converged_on_it": target.report() == converged(sn, n)})
        out["target_after"] = [{k: x for k, x in st.items() if k != "at"} for st in steps]
        out["target_run"] = bus.run_doc()
        rp.note(sec, f"the target, converged on v2 at {s2} before, then: {json.dumps(steps)}; it runs "
                     f"{out['target_run']}")
    finally:
        bus.close()
    return out


EDGES = {"done": edge_done, "failed": edge_failed, "during": edge_during, "record": edge_record}


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="python -m zk2py.desired_v2_edges", description=__doc__.split("\n")[0])
    ap.add_argument("--target", action="append", choices=["zk2py", "rust"])
    ap.add_argument("--author", action="append", choices=["zk2py", "rust"])
    ap.add_argument("--only", action="append", choices=sorted(EDGES))
    args = ap.parse_args(argv)
    SCRATCH.mkdir(parents=True, exist_ok=True)
    rp = Report()
    got: dict[str, dict[str, Any]] = {}
    for name in args.only or list(EDGES):
        impls = (args.author if name == "record" else args.target) or ["zk2py", "rust"]
        for impl in impls:
            try:
                got.setdefault(name, {})[impl] = EDGES[name](rp, impl)
            except Exception as e:  # noqa: BLE001 - an edge that raises fails, and the rest run
                import traceback

                traceback.print_exc()
                rp.check(f"{name} [{impl}]", "the edge ran to its end", False, f"{type(e).__name__}: {e}")
    for name, by in got.items():
        if len(by) == 2:
            a, b = by["zk2py"], by["rust"]
            for k in sorted(set(a) | set(b)):
                same = a.get(k) == b.get(k)
                print(f"{'SAME' if same else 'DIFFER'} [{name}] {k}: zk2py {json.dumps(a.get(k), sort_keys=True)}"
                      + ("" if same else f" | rust {json.dumps(b.get(k), sort_keys=True)}"), flush=True)
    passed = sum(1 for r in rp.rows if r[2])
    print(f"{passed} of {len(rp.rows)} checks passed; not run: {len(rp.not_run)}", flush=True)
    return 0 if passed == len(rp.rows) else 1


if __name__ == "__main__":
    sys.exit(main())

"""The ``timing.v1`` scenarios (``spec/profiles/timing/scenarios.md``) on
in-process zenoh-python routers, zk2py only: ``python -m
zk2py.timing_scenarios [--only 1 … 6|hlc|first]``.

- **The bus** is the text's: R1, a router with timestamping on and its
  HLC's delta the default, 500 ms; the owner, C and T are its clients, each
  with sessions of its own. §5 step 2's R2 is a router with timestamping
  off. Every listener is on a free loopback port (port 0's choice).
- **The contract** is ``pace.v1``, copied from the text into
  ``impl/python/interop/timing/pace.v1.toml``; its fingerprint is the
  reference CLI's (round 76). The owner is ``lab/pace``, zk2py's
  :class:`zk2py.owner.Owner`, its session's HLC on and ``meta.zid`` stated
  unless a section says otherwise, holding a member token per axis it
  publishes (:meth:`zk2py.owner.Owner.hold_member`). Each payload is its
  sequence number as text.
- **C** is :class:`zk2py.timing_live.Reader`: its role bound to
  ``lab/pace``, its liveliness subscriber on a watch session and its reads
  on another, a read 100 ms after a withdrawal it hears, the descriptor
  read for ``meta.zid``. **T** is :class:`zk2py.timing_live.Window` (§6),
  or a liveliness read (§1 step 4).
- **The owner's clock** is offset by stamping each put from the host's wall
  clock plus the offset under the owner's zid
  (:func:`zk2py.timing_live.stamped_clock`).
- **The host.** A due is checked exactly, from C's recorded arrivals; a
  report's instant against the due plus the 10 ms bound, with the 50 ms the
  text allows a runner beyond it, the lateness past the bound recorded
  apart.
- **When C stops, and when the owner first publishes** (F-266). Where a
  section expects no deadline transition, or nothing after a ``met``, C is
  closed 200 ms after the owner's last sample, before a miss could be due.
  Where it expects every sample attributed (§4, §5), the owner publishes
  once C holds its descriptor: zenoh-python handles each subscriber on a
  thread of its own, so a first sample put at once can be handled before
  the descriptor's put, sent first (``first`` measures it).
- **``first``** is not a section either: how often a new instance's
  first sample, put at once, is handled before its first descriptor.
- **``hlc``** is not a section: core 0.53's two facts measured on
  zenoh-python 1.10.1 (a stamp's time from the Unix epoch; an HLC moved to
  a later stamp it receives), and whether a client whose HLC is on stamps
  an unstamped put it receives (F-265).

Exit 0 when every check passes, 1 when any fails.
"""

from __future__ import annotations

import argparse
import json
import sys
import threading
import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Callable

from . import timing as tm

REPO = Path(__file__).resolve().parents[3]
PACE = REPO / "impl" / "python" / "interop" / "timing" / "pace.v1.toml"
ADDRESS = "lab/pace"
BASE = "zk2/lab/pace/pace.v1/stream"
CMD = f"{BASE}/cmd"
AXIS = f"{BASE}/axis"
RID_CMD, RID_AXIS = "stream/cmd", "stream/axis/{axis}"
MS = 1_000_000
NS = 1_000_000_000
BOUND_MS = 10
RUNNER_MS = 50


class Report:
    def __init__(self) -> None:
        self.rows: list[tuple[str, str, bool, str]] = []
        self.notes: list[tuple[str, str]] = []

    def check(self, section: str, name: str, ok: bool, detail: str = "") -> bool:
        self.rows.append((section, name, ok, detail))
        print(f"{'PASS' if ok else 'FAIL'} [{section}] {name}" + (f": {detail}" if detail else ""), flush=True)
        return ok

    def note(self, section: str, text: str) -> None:
        self.notes.append((section, text))
        print(f"NOTE [{section}] {text}", flush=True)


def _zv(text: Any) -> int | None:
    return tm.zid_value(str(text)) if text is not None else None


class Bus:
    """A router (R1, or R2 with timestamping off) and its clients."""

    def __init__(self, timestamping: bool = True) -> None:
        from .contract import load_contract
        from .live_interop import _r1

        self.router, self.endpoint, self.r_zid = _r1(timestamping=timestamping)
        self.contract = load_contract(PACE)
        assert self.contract.valid, self.contract.codes
        self.sessions: list[Any] = []
        self.readers: list[Any] = []
        self.owners: list[Any] = []

    def client(self, hlc: bool = False) -> Any:
        from . import live

        if not hlc:
            s = live.open_client(self.endpoint)
        else:
            import zenoh

            conf = zenoh.Config()
            conf.insert_json5("mode", json.dumps("client"))
            conf.insert_json5("connect/endpoints", json.dumps([self.endpoint]))
            conf.insert_json5("scouting/multicast/enabled", "false")
            conf.insert_json5("timestamping/enabled", "true")
            s = zenoh.open(conf)
        self.sessions.append(s)
        return s

    def reader(self, rids: list[str], epsilon_ms: int | None = None, **kw: Any):
        from .timing_live import Reader

        r = Reader(self.client(), self.client(), ADDRESS, self.contract, rids, epsilon_ms=epsilon_ms, **kw)
        self.readers.append(r)
        return r

    def window(self, rids: list[str], epsilon_ms: int | None = None, seed_first: bool = True):
        from .timing_live import Window

        w = Window(self.client(), self.client(), ADDRESS, self.contract, rids, epsilon_ms=epsilon_ms,
                   seed_first=seed_first)
        self.readers.append(w)
        return w

    def owner(self, **kw: Any):
        from .owner import Owner

        o = Owner("lab", "pace", [self.contract], connect=self.endpoint, **kw)
        o.start()
        self.owners.append(o)
        return o

    def close(self) -> None:
        for r in self.readers:
            try:
                r.close()
            except Exception:  # noqa: BLE001 - closing anyway
                pass
        for o in self.owners:
            try:
                o.close()
            except Exception:  # noqa: BLE001
                pass
        for s in self.sessions:
            try:
                s.close()
            except Exception:  # noqa: BLE001
                pass
        self.router.close()


def _sleep_until(mono_ns: int) -> None:
    d = (mono_ns - time.monotonic_ns()) / NS
    if d > 0:
        time.sleep(d)


def publish_every(owner: Any, key: str, period_s: float, duration_s: float, *, seq: list[int] | None = None,
                  clock: Callable[[], Any] | None = None, payload: Callable[[int], bytes] | None = None,
                  stop: threading.Event | None = None) -> list[int]:
    """Publish ``key`` every ``period_s`` for ``duration_s``, on a fixed
    schedule; returns each put's instant (monotonic ns). ``seq`` carries the
    sequence number across calls."""
    seq = seq if seq is not None else [0]
    t0 = time.monotonic_ns()
    n = max(1, int(round(duration_s / period_s)))
    out = []
    for i in range(n):
        if stop is not None and stop.is_set():
            break
        _sleep_until(t0 + int(i * period_s * NS))
        body = payload(seq[0]) if payload is not None else str(seq[0]).encode()
        owner.publish(key, body, timestamp=None if clock is None else clock())
        out.append(time.monotonic_ns())
        seq[0] += 1
    return out


def _due_ok(r: tm.Report, due: int) -> tuple[bool, int]:
    """A miss's due exact, and its report within the bound and the runner's
    allowance; the lateness past the bound (ns)."""
    late = r.at_ns - (r.due_ns or 0) - BOUND_MS * MS
    return r.due_ns == due and r.at_ns > due and late <= RUNNER_MS * MS, max(0, late)


def _ms(ns: int | None) -> str:
    return "None" if ns is None else f"{ns / MS:.3f} ms"


# -- §1 ---------------------------------------------------------------------------------

def section1(report: Report) -> None:
    from . import live

    sec = "§1 a commander paused, then resumed"
    bus = Bus()
    try:
        c = bus.reader([RID_CMD])
        time.sleep(0.3)
        o = bus.owner()
        seq = [0]
        t1 = time.monotonic_ns()
        publish_every(o, CMD, 0.1, 2.0, seq=seq)
        t1_end = time.monotonic_ns()
        time.sleep(0.05)  # the last sample lands
        step1 = [a for a in c.arrivals_of(CMD) if a.mono_ns <= t1_end + 50 * MS]
        reports1 = [r for r in c.reports_of(CMD) if r.at_ns <= t1_end + 50 * MS]
        gaps = [(b.mono_ns - a.mono_ns) / MS for a, b in zip(step1, step1[1:])]
        report.check(sec, "Expected 1: C reports nothing in step 1 (cmd known, its owner present, every gap at "
                          "most 500 ms), its first arrival included",
                     not reports1 and len(step1) >= 18 and max(gaps, default=0) <= 500
                     and c.present_at() is not None,
                     f"{len(step1)} arrivals of 20 sent, largest gap {max(gaps, default=0):.1f} ms, reports "
                     f"{reports1}; C saw its owner present {_ms((c.present_at() or 0) - t1)} after step 1 began, "
                     f"its first arrival {_ms(step1[0].mono_ns - t1) if step1 else None} after")
        # Step 2: nothing for 2 s, keeping the service, the publisher and the tokens.
        _sleep_until(t1_end + 2 * NS)
        t2_end = time.monotonic_ns()
        # Step 4: at the end of step 2, T reads lab/pace's presence.
        t_session = bus.client()
        pres = live.list_presence(t_session, "zk2/lab/pace/@zk/**")
        with c.lock:
            m = c.resources[RID_CMD].tracker.members[CMD]
            miss_stands = m.missed
        last1 = step1[-1].mono_ns
        in2 = [a for a in c.arrivals_of(CMD) if t1_end + 50 * MS < a.mono_ns <= t2_end]
        missed = c.reports_of(CMD, "missed")
        ok_due, late = (False, 0) if len(missed) != 1 else _due_ok(missed[0], last1 + 500 * MS)
        report.check(sec, "Expected 2: one missed for cmd, due exactly 500 ms after step 1's last arrival, "
                          "reported at most 10 ms after (the runner's 50 ms beyond); no sample of cmd in step 2 "
                          "(no runtime publishes on the owner's behalf); nothing more in step 2",
                     len(missed) == 1 and ok_due and not in2
                     and [r.event for r in c.reports_of(CMD) if r.at_ns <= t2_end] == ["missed"],
                     f"missed {[(_ms(r.due_ns - last1), _ms(r.at_ns - r.due_ns), r.owner) for r in missed]}, "
                     f"lateness past the bound {_ms(late)}, {len(in2)} samples in step 2")
        report.check(sec, "Expected 4: T reads the owner present, its instance and interface tokens held, a "
                          "complete read, and C's miss stands beside it: neither folded into the other",
                     pres.complete and len(pres.instances) == 1 and [a["iface"] for a in pres.alive] == ["pace.v1"]
                     and miss_stands and c.owner_state == tm.PRESENT,
                     f"presence: {pres.reading}; C's cmd missed {miss_stands}, C's owner {c.owner_state}")
        # Step 3.
        t3 = time.monotonic_ns()
        sent3 = publish_every(o, CMD, 0.1, 1.0, seq=seq)
        _sleep_until(sent3[-1] + 200 * MS)
        after = [r for r in c.reports_of(CMD) if r.at_ns > t2_end]
        first3 = next((a for a in c.arrivals_of(CMD) if a.mono_ns >= t3), None)
        report.check(sec, "Expected 3: met once, at step 3's first arrival, and nothing after (C closed 200 ms "
                          "after the last sample)",
                     [r.event for r in after] == ["met"] and first3 is not None and after[0].at_ns == first3.mono_ns,
                     f"{[(r.event, _ms(r.at_ns - t3)) for r in after]}, first arrival at "
                     f"{_ms(first3.mono_ns - t3) if first3 else None}")
        c.close()
    finally:
        bus.close()


# -- §2 ---------------------------------------------------------------------------------

def section2(report: Report) -> None:
    sec = "§2 a member gone, and a member silent"
    bus = Bus()
    try:
        c = bus.reader([RID_AXIS])
        time.sleep(0.3)
        o = bus.owner()
        kx, ky = f"{AXIS}/x", f"{AXIS}/y"
        o.hold_member("pace.v1", "x")
        o.hold_member("pace.v1", "y")
        stop_y, stop_x = threading.Event(), threading.Event()
        seqs = {"x": [0], "y": [0]}

        def run(key: str, which: str, stop: threading.Event) -> None:
            while not stop.is_set():
                publish_every(o, key, 0.1, 0.1, seq=seqs[which])
                time.sleep(0.1)

        t0 = time.monotonic_ns()
        tx = threading.Thread(target=run, args=(kx, "x", stop_x), daemon=True)
        ty = threading.Thread(target=run, args=(ky, "y", stop_y), daemon=True)
        tx.start()
        ty.start()
        _sleep_until(t0 + 1 * NS)
        # Step 2: y stops, and its member token is withdrawn.
        stop_y.set()
        ty.join()
        t_w = time.monotonic_ns()
        o.release_member("pace.v1", "y")
        reports1 = [r for r in c.reports_of() if r.at_ns <= t_w]
        report.check(sec, "Expected 1: C reports nothing in step 1: neither member is known (the parameter "
                          "unbound), each judged from its first arrival",
                     not reports1 and c.resources[RID_AXIS].known == []
                     and len(c.arrivals_of(kx)) >= 8 and len(c.arrivals_of(ky)) >= 8,
                     f"reports {reports1}; {len(c.arrivals_of(kx))} x, {len(c.arrivals_of(ky))} y")
        _sleep_until(t_w + 2 * NS)
        stop_x.set()
        tx.join()
        t_x = time.monotonic_ns()
        f = c.follower
        with f.lock:
            heard_del = [t for t, k, kind in f.view.events if kind == "delete" and "/@zk/member/pace.v1/y/" in k]
            reads = list(zip(f.reads, f.folds))
            asked = [r.asked_ns for r in f.view.reads]
        last_y = c.arrivals_of(ky)[-1].mono_ns
        ended = c.reports_of(ky, "ended")
        after_del = [(r, k, a) for (r, k), a in zip(reads, asked) if heard_del and a >= heard_del[0]]
        read_ok = bool(after_del) and after_del[0][1] == ("complete", True) \
            and not any(m["member"] == "y" for m in after_del[0][0].members)
        report.check(sec, "Expected 2: C hears y's member token withdrawn, reads again, the read complete and "
                          "showing no member token of y; ended for axis/y at that read, never missed, long "
                          "before its due; nothing of axis/x in step 2",
                     bool(heard_del) and read_ok and len(ended) == 1 and ended[0].reason == "gone"
                     and ended[0].at_ns > heard_del[0] and ended[0].at_ns < last_y + 500 * MS
                     and not c.reports_of(ky, "missed")
                     and not [r for r in c.reports_of(kx) if r.at_ns <= t_x],
                     f"withdrawal heard {_ms(heard_del[0] - last_y) if heard_del else None} after y's last arrival; "
                     f"read after it {after_del[0][1] if after_del else None}; ended "
                     f"{[(r.reason, _ms(r.at_ns - last_y)) for r in ended]} after y's last arrival")
        # Step 3: x silent 1 s, its token held.
        _sleep_until(t_x + 1 * NS)
        last_x = c.arrivals_of(kx)[-1].mono_ns
        missed = c.reports_of(kx, "missed")
        ok_due, late = (False, 0) if len(missed) != 1 else _due_ok(missed[0], last_x + 500 * MS)
        report.check(sec, "Expected 3: missed for axis/x, due 500 ms after its last arrival, within the bound: a "
                          "member whose token is held is still judged",
                     len(missed) == 1 and ok_due,
                     f"{[(_ms(r.due_ns - last_x), _ms(r.at_ns - r.due_ns)) for r in missed]}, "
                     f"lateness past the bound {_ms(late)}")
        c.close()
    finally:
        bus.close()


# -- §3 ---------------------------------------------------------------------------------

def section3(report: Report) -> None:
    sec = "§3 a member never heard"
    bus = Bus()
    try:
        c = bus.reader([RID_CMD])
        s_at = c.start_ns
        assert s_at is not None
        _sleep_until(s_at + 1 * NS)
        report.check(sec, "Expected 1: C reports nothing in the second before the owner: cmd is known, but its "
                          "owner not present, so nothing is waited for",
                     not c.reports_of() and c.owner_state in (tm.UNSEEN, tm.ABSENT),
                     f"reports {c.reports_of()}, owner {c.owner_state}")
        o = bus.owner()
        deadline = time.monotonic() + 3.0
        while c.present_at() is None and time.monotonic() < deadline:
            time.sleep(0.005)
        p_at = c.present_at()
        if p_at is None:
            report.check(sec, "Expected 2: C sees the owner's interface token", False, "never present")
            return
        _sleep_until(p_at + 1 * NS)
        o.publish(CMD, b"0")
        time.sleep(0.2)
        missed = c.reports_of(CMD, "missed")
        ok_due, late = (False, 0) if len(missed) != 1 else _due_ok(missed[0], max(s_at, p_at) + 500 * MS)
        report.check(sec, "Expected 2: missed for cmd, due at P + 500 ms (the later of S and P, plus the "
                          "deadline), within the bound",
                     len(missed) == 1 and ok_due and missed[0].owner == tm.PRESENT,
                     f"P {_ms(p_at - s_at)} after S; missed {[(_ms(r.due_ns - p_at), _ms(r.at_ns - r.due_ns)) for r in missed]}"
                     f" after P; lateness past the bound {_ms(late)}")
        arr = c.arrivals_of(CMD)
        met = c.reports_of(CMD, "met")
        report.check(sec, "Expected 3: met at the arrival",
                     len(arr) == 1 and len(met) == 1 and met[0].at_ns == arr[0].mono_ns,
                     f"{len(arr)} arrivals, met {[_ms(r.at_ns - p_at) for r in met]} after P")
        c.close()
        c2 = bus.reader([RID_AXIS])
        time.sleep(2.0)
        report.check(sec, "Expected 4: C2 reports nothing over 2 s: no axis member is known, and none arrived",
                     not c2.reports_of() and not c2.arrivals and c2.owner_state == tm.PRESENT,
                     f"reports {c2.reports_of()}, {len(c2.arrivals)} arrivals, owner {c2.owner_state}")
        c2.close()
    finally:
        bus.close()


# -- §4 ---------------------------------------------------------------------------------

def _held_by(c: Any, o: Any, timeout_s: float = 2.0) -> bool:
    """Until C holds the new instance's descriptor (F-266): zenoh-python
    handles each subscriber on a thread of its own, so the descriptor's put,
    though sent before the tokens (core §3.3), can be handled after the
    instance's first sample on another key."""
    end = time.monotonic() + timeout_s
    while time.monotonic() < end:
        with c.lock:
            if o.instance in c.descriptors:
                return True
        time.sleep(0.002)
    return False


def _lifespan_step(bus: Bus, epsilon: int | None, offset: Callable[[float], int]) -> tuple[Any, Any, list[int]]:
    """A new C, then a new owner instance publishing cmd every 100 ms for 2 s,
    its clock ``offset(seconds since its first put)`` off C's; C closed
    200 ms after the last put."""
    from .timing_live import stamped_clock

    handed: list[bytes] = []
    c = bus.reader([RID_CMD], epsilon, on_value=lambda k, p: handed.append(p))
    c.handed = handed  # type: ignore[attr-defined]
    time.sleep(0.3)
    o = bus.owner()
    _held_by(c, o)
    t0 = time.monotonic_ns()
    clock = stamped_clock(o.session, lambda: offset((time.monotonic_ns() - t0) / NS))
    sent = publish_every(o, CMD, 0.1, 2.0, clock=clock)
    _sleep_until(sent[-1] + 200 * MS)
    c.close()
    return c, o, sent


def section4(report: Report) -> None:
    sec = "§4 the lifespan on a stated bound"
    bus = Bus()
    try:
        steps = [
            ("1", 100, lambda s: 0),
            ("2", 100, lambda s: -700 * MS),
            ("3", 100, lambda s: 400 * MS if s < 1.0 else 0),
            ("4", None, lambda s: -700 * MS),
        ]
        for name, eps, off in steps:
            c, o, sent = _lifespan_step(bus, eps, off)
            arr = c.arrivals_of(CMD)
            zid = _zv(json.loads(o.descriptor)["meta"]["zid"])
            verdicts = [a.verdict.reason for a in arr]
            ages = [(a.wall_ns - a.stamp_ns) / MS for a in arr if a.stamp_ns is not None]
            reports = [(r.event, r.due_ns) for r in c.reports_of(CMD)]
            ours = all(_zv(a.stamp_id) == zid for a in arr)
            handed = len(c.handed)
            detail = (f"{len(arr)} arrivals of {len(sent)} sent; verdicts {sorted(set(verdicts))}; ages "
                      f"{min(ages, default=0):.1f} to {max(ages, default=0):.1f} ms; handed {handed}; "
                      f"stamps the owner's meta.zid {ours}; reports {[e for e, _ in reports]}")
            if name == "1":
                report.check(sec, "Expected 1: every stamp the owner's meta.zid, every sample live, all handed "
                                  "on, no deadline transition",
                             len(arr) >= 18 and ours and set(verdicts) == {"live"} and handed == len(arr)
                             and not reports, detail)
                report.check("hlc", "core 0.53 App. B: a stamp's time counts from the Unix epoch: the owner's "
                                    "stamps, same host, age 0 to 50 ms against C's wall clock",
                             bool(ages) and 0 <= min(ages) and max(ages) <= 50, f"ages {min(ages):.3f} to "
                                                                                   f"{max(ages):.3f} ms")
            elif name == "2":
                p_at = c.present_at()
                missed = c.reports_of(CMD, "missed")
                ok_due, late = (False, 0) if len(missed) != 1 or p_at is None else _due_ok(missed[0], p_at + 500 * MS)
                report.check(sec, "Expected 2 (700 ms behind): every sample expired, none handed on; missed for "
                                  "cmd, due 500 ms after C first saw its owner present (no sample is an arrival), "
                                  "no met",
                             len(arr) >= 18 and ours and set(verdicts) == {"expired"} and handed == 0
                             and len(missed) == 1 and ok_due and not c.reports_of(CMD, "met"),
                             detail + f"; missed due {_ms(missed[0].due_ns - p_at) if missed and p_at else None} "
                                      f"after P, lateness past the bound {_ms(late)}")
            elif name == "3":
                first = [a for a in arr if a.mono_ns < sent[0] + int(0.95 * NS)]
                second = [a for a in arr if a.mono_ns >= sent[0] + int(1.05 * NS)]
                a1 = [(a.wall_ns - a.stamp_ns) / MS for a in first]
                a2 = [(a.wall_ns - a.stamp_ns) / MS for a in second]
                report.check(sec, "Expected 3 (400 ms ahead, then agreeing): every sample clock_disagrees, the "
                                  "second second's too (falsified for the reading); all handed on; no deadline "
                                  "transition",
                             len(arr) >= 18 and ours and set(verdicts) == {"clock_disagrees"} and handed == len(arr)
                             and not reports and bool(a1) and max(a1) < -300 and bool(a2) and max(abs(x) for x in a2) < 50,
                             detail + f"; first second's ages {min(a1, default=0):.1f} to {max(a1, default=0):.1f} ms, "
                                      f"the second's {min(a2, default=0):.1f} to {max(a2, default=0):.1f} ms")
            else:
                report.check(sec, "Expected 4 (700 ms behind, no ε): every sample no_bound, though R1's delta is "
                                  "500 ms; all handed on; no deadline transition",
                             len(arr) >= 18 and ours and set(verdicts) == {"no_bound"} and handed == len(arr)
                             and not reports, detail)
            o.close()
            bus.owners.remove(o)
            time.sleep(0.3)
    finally:
        bus.close()


# -- §5 ---------------------------------------------------------------------------------

def _payload_10s_old(_: int) -> bytes:
    return str(time.time_ns() - 10 * NS).encode()


@dataclass
class _Beside:
    """A second subscriber beside C, on a client session whose own HLC is
    on: its zid, and each sample's stamp id (None: unstamped)."""

    zid: str = ""
    ids: list[str | None] = field(default_factory=list)


def _unstamped_step(bus: Bus, hlc: bool, beside_hlc: bool = False) -> tuple[Any, Any, list[int], _Beside]:
    handed: list[bytes] = []
    c = bus.reader([RID_CMD], 100, on_value=lambda k, p: handed.append(p))
    c.handed = handed  # type: ignore[attr-defined]
    beside = _Beside()
    if beside_hlc:
        import zenoh

        x = bus.client(hlc=True)
        beside.zid = str(x.zid())
        x.declare_subscriber(CMD, zenoh.handlers.Callback(
            lambda s: beside.ids.append(None if s.timestamp is None else str(s.timestamp.get_id()))))
    time.sleep(0.3)
    o = bus.owner(hlc=hlc)
    _held_by(c, o)
    sent = publish_every(o, CMD, 0.1, 2.0, payload=_payload_10s_old)
    _sleep_until(sent[-1] + 200 * MS)
    c.close()
    return c, o, sent, beside


def section5(report: Report) -> None:
    sec = "§5 an unstamped owner"
    # Step 1: through R1, the owner's session without an HLC.
    bus = Bus()
    try:
        c, o, sent, _ = _unstamped_step(bus, hlc=False)
        arr = c.arrivals_of(CMD)
        r1 = _zv(bus.r_zid)
        zid = _zv(json.loads(o.descriptor)["meta"]["zid"])
        report.check(sec, "Expected 1: every sample stamped by R1 (its id R1's zid, not the owner's meta.zid), "
                          "each foreign_stamp, all handed on, no deadline transition",
                     len(arr) >= 18 and all(_zv(a.stamp_id) == r1 for a in arr) and r1 != zid
                     and {a.verdict.reason for a in arr} == {"foreign_stamp"} and len(c.handed) == len(arr)
                     and not c.reports_of(),
                     f"{len(arr)} arrivals; ids {sorted({a.stamp_id for a in arr})} (R1 {bus.r_zid}); "
                     f"{sorted({a.verdict.reason for a in arr})}; handed {len(c.handed)}; reports {c.reports_of()}")
        o.close()
        bus.owners.remove(o)
    finally:
        bus.close()
    # Step 2: R2, timestamping off.
    bus = Bus(timestamping=False)
    try:
        c, o, sent, beside = _unstamped_step(bus, hlc=False, beside_hlc=True)
        arr = c.arrivals_of(CMD)
        report.check(sec, "Expected 2 (R2, timestamping off): every sample arrives with no stamp, unobservable "
                          "(unstamped), all handed on",
                     len(arr) >= 18 and all(a.stamp_ns is None for a in arr)
                     and {a.verdict.reason for a in arr} == {"unstamped"} and len(c.handed) == len(arr),
                     f"{len(arr)} arrivals; {sorted({a.verdict.reason for a in arr})}; handed {len(c.handed)}")
        own = sum(1 for i in beside.ids if i is not None and _zv(i) == _zv(beside.zid))
        report.note("hlc", f"through R2 (timestamping off), a client whose own HLC is on received "
                           f"{len(beside.ids)} samples of the unstamped owner: {own} stamped with its own zid, "
                           f"{sum(1 for i in beside.ids if i is None)} unstamped (F-265)")
        o.close()
        bus.owners.remove(o)
    finally:
        bus.close()
    # Step 3: control, through R1, the owner's HLC on.
    bus = Bus()
    try:
        c, o, sent, _ = _unstamped_step(bus, hlc=True)
        arr = c.arrivals_of(CMD)
        zid = _zv(json.loads(o.descriptor)["meta"]["zid"])
        report.check(sec, "Expected 3 (control): every stamp the owner's meta.zid, every sample live; in no step "
                          "does C read the payload's time (it is 10 s old)",
                     len(arr) >= 18 and all(_zv(a.stamp_id) == zid for a in arr)
                     and {a.verdict.reason for a in arr} == {"live"}
                     and all(time.time_ns() - int(a.payload) > 9 * NS for a in arr),
                     f"{len(arr)} arrivals; {sorted({a.verdict.reason for a in arr})}")
        o.close()
        bus.owners.remove(o)
    finally:
        bus.close()


# -- §6 ---------------------------------------------------------------------------------

def section6(report: Report) -> None:
    sec = "§6 a tool's window"
    bus = Bus()
    try:
        o = bus.owner()
        kx = f"{AXIS}/x"
        o.hold_member("pace.v1", "x")
        stop_axis = threading.Event()
        seq_x, seq_c = [0], [0]

        def axis() -> None:
            while not stop_axis.is_set():
                publish_every(o, kx, 0.1, 0.1, seq=seq_x)
                time.sleep(0.1)

        th = threading.Thread(target=axis, daemon=True)
        th.start()
        background = threading.Event()

        def cmd_background() -> None:
            while not background.is_set():
                publish_every(o, CMD, 0.1, 0.1, seq=seq_c)
                time.sleep(0.1)

        def run_window(plan: Callable[[int], None], seed_first: bool = True
                       ) -> tuple[Any, dict[str, dict[str, tm.Answer]]]:
            w = bus.window([RID_CMD, RID_AXIS], 100, seed_first)
            plan(w.start_ns)
            _sleep_until(w.start_ns + 5 * NS)
            got = w.end()
            w.close()
            bus.readers.remove(w)
            return w, got

        def every(period_s: float, pause: tuple[float, float] | None = None):
            def plan(start: int) -> None:
                def go() -> None:
                    i = 0
                    while True:
                        t = start + int(i * period_s * NS)
                        if t >= start + 5 * NS + 100 * MS:
                            return
                        _sleep_until(t)
                        rel = (t - start) / NS
                        if pause is None or not (pause[0] <= rel < pause[1]):
                            if o.session is None:
                                return  # step 4: the owner closed
                            try:
                                o.publish(CMD, str(seq_c[0]).encode())
                            except Exception:  # noqa: BLE001 - closed while putting
                                return
                            seq_c[0] += 1
                        i += 1
                threading.Thread(target=go, daemon=True).start()
            return plan

        def show(got: dict[str, dict[str, tm.Answer]]) -> str:
            return "; ".join(f"{k.rsplit('/stream/', 1)[1]}: " + ", ".join(f"{q} {a.answer} ({a.reason})"
                                                                         for q, a in v.items())
                             for k, v in got.items())

        def pairs(got: dict[str, dict[str, tm.Answer]], key: str) -> dict[str, tuple[str, str]]:
            return {q: (a.answer, a.reason) for q, a in got.get(key, {}).items()}

        # Step 1.
        w, got = run_window(every(0.1))
        report.check(sec, "Expected 1: cmd deadline yes (met), period yes (kept), lifespan no (live); axis/x "
                          "deadline yes, period yes, lifespan not asked (no_lifespan)",
                     pairs(got, CMD) == {"deadline": ("yes", "met"), "period": ("yes", "kept"),
                                         "lifespan": ("no", "live")}
                     and pairs(got, kx) == {"deadline": ("yes", "met"), "period": ("yes", "kept"),
                                            "lifespan": ("not_asked", "no_lifespan")}, show(got))
        # Step 2: cmd stopped 1 s, 2 s into the window.
        w, got = run_window(every(0.1, (2.0, 3.0)))
        rd = w.readings.get(CMD)
        runs = {q: tm.rollup(q, [v[q] for v in got.values()]) for q in ("deadline", "period", "lifespan")}
        finding = any(a.answer == tm.FINDING[q] for q, a in runs.items())
        misses = [r for r in w.reader.reports_of(CMD, "missed")]
        report.check(sec, "Expected 2: cmd deadline no (missed, the owner present when T reported it), period no "
                          "(not_kept: about 40 arrivals, 4,200 below 5,000), lifespan no; the run's verdict is "
                          "the finding",
                     pairs(got, CMD) == {"deadline": ("no", "missed"), "period": ("no", "not_kept"),
                                         "lifespan": ("no", "live")}
                     and all(r.owner == tm.PRESENT for r in misses) and finding,
                     show(got) + f"; cmd arrivals {len([a for a in w.reader.arrivals_of(CMD)])}, held "
                                 f"{_ms(rd.deadline.held_ns) if rd else None}; run "
                                 f"{ {q: (a.answer, a.reason) for q, a in runs.items()} }")
        # Step 3: cmd every 250 ms.
        w, got = run_window(every(0.25))
        report.check(sec, "Expected 3: cmd deadline yes; period no (not_kept: about 20 arrivals, 2,200 below "
                          "5,000)",
                     pairs(got, CMD).get("deadline") == ("yes", "met")
                     and pairs(got, CMD).get("period") == ("no", "not_kept"),
                     show(got) + f"; cmd arrivals {len(w.reader.arrivals_of(CMD))}")
        # F-263: step 2 again, the window opened (subscribed) before the
        # first presence read lands, as the setup lists the steps.
        w, got = run_window(every(0.1, (2.0, 3.0)), seed_first=False)
        at_start = w._state_at(w.reader.owner_states, w.start_ns)
        seen = _ms((w.reader.present_at() or w.start_ns) - w.start_ns)
        report.note(sec, f"step 2 with the window opened before its first presence read (F-263): the owner "
                         f"{at_start} at the window's start, seen present {seen} after it; cmd: "
                         + ", ".join(f"{q} {a.answer} ({a.reason})" for q, a in got.get(CMD, {}).items()))
        # Step 4: the owner closes cleanly 1 s into the window.

        def close_at_1s(start: int) -> None:
            every(0.1)(start)

            def go() -> None:
                _sleep_until(start + 1 * NS)
                stop_axis.set()
                o.close()
            threading.Thread(target=go, daemon=True).start()

        w, got = run_window(close_at_1s)
        rd = w.reader
        f = rd.follower
        with f.lock:
            dels = [t for t, k, kind in f.view.events if kind == "delete"]
            reads = list(zip([r.asked_ns for r in f.view.reads], f.folds, f.reads))
        after = [(a, k, p) for a, k, p in reads if dels and a >= dels[0]]
        last_cmd = max((a.mono_ns for a in rd.arrivals_of(CMD)), default=0)
        last_x = max((a.mono_ns for a in rd.arrivals_of(kx)), default=0)
        ended = rd.reports_of(kx, "ended")
        read_ok = bool(after) and after[0][1] == ("complete", True) and after[0][2].count == 0
        report.check(sec, "Expected 4: T hears the tokens withdrawn, its read after them complete and showing none, "
                          "long before either miss is due; cmd deadline unobservable (owner_not_present), period "
                          "unobservable (owner_not_present), lifespan no; axis/x ended at that read: deadline "
                          "not asked (ended), period yes (kept)",
                     read_ok and len(ended) == 1 and ended[0].reason == "gone"
                     and ended[0].at_ns < min(last_cmd, last_x) + 500 * MS
                     and pairs(got, CMD) == {"deadline": ("unobservable", "owner_not_present"),
                                             "period": ("unobservable", "owner_not_present"),
                                             "lifespan": ("no", "live")}
                     and pairs(got, kx).get("deadline") == ("not_asked", "ended")
                     and pairs(got, kx).get("period") == ("yes", "kept"),
                     show(got) + f"; the read after the withdrawals {after[0][1] if after else None}, "
                                 f"{after[0][2].count if after else None} tokens; axis/x ended "
                                 f"{[_ms(r.at_ns - last_x) for r in ended]} after its last arrival")
        bus.owners.remove(o)
    finally:
        stop_axis.set()
        bus.close()


# -- core 0.53's HLC facts ------------------------------------------------------------------

def section_hlc(report: Report) -> None:
    """Core 0.53 §4.1 and Appendix B on zenoh-python 1.10.1: a session whose
    HLC is on moves it to a later stamp it receives for its own subscriber;
    so does a router routing it; a stamp more than the delta ahead is
    re-stamped (here, by R1, before any client sees it)."""
    import zenoh

    from .timing_live import stamped_clock

    sec = "hlc"
    bus = Bus()
    try:
        x = bus.client(hlc=True)
        got: list[Any] = []
        sub = x.declare_subscriber(CMD, zenoh.handlers.Callback(lambda s: got.append(s.timestamp)))
        time.sleep(0.3)
        o = bus.owner()
        clock = stamped_clock(o.session, 300 * MS)
        for _ in range(5):
            o.publish(CMD, b"ahead", timestamp=clock())
            time.sleep(0.02)
        time.sleep(0.1)

        def off(s: Any) -> float:
            return (s.new_timestamp().get_time_as_ntp64().as_nanos() - time.time_ns()) / MS

        x_off, r_off, o_off = off(x), off(bus.router), off(o.session)
        report.check(sec, "core 0.53 §4.1: a client whose HLC is on, having received puts stamped 300 ms ahead for "
                          "its own subscriber, stamps ahead of its wall clock (its HLC moved to the later stamp), "
                          "and so does R1, which routed them",
                     len(got) == 5 and all(t is not None for t in got) and 150 <= x_off <= 400 and 150 <= r_off <= 400,
                     f"{len(got)} received; the client's next stamp {x_off:+.1f} ms, R1's {r_off:+.1f} ms off the "
                     f"wall clock")
        report.note(sec, f"the owner's own session, which put those stamps itself: its next stamp {o_off:+.1f} ms "
                         f"off its wall clock (a session's own put moves its HLC too)")
        got.clear()
        time.sleep(0.6)
        far = stamped_clock(o.session, 900 * MS)
        o.publish(CMD, b"far", timestamp=far())
        time.sleep(0.2)
        ids = [str(t.get_id()) for t in got if t is not None]
        own = str(o.session.zid())
        report.check(sec, "core 0.53 §4.1: a put its owner stamps 900 ms ahead, beyond its own session's 500 ms "
                          "delta, is re-stamped by that session from its own clock (\"a node … a session's "
                          "included … re-stamps the put from its own clock\")",
                     len(ids) == 1 and _zv(ids[0]) == _zv(own), f"ids {ids}, the owner's zid {own}, R1 {bus.r_zid}")
        o.close()
        bus.owners.remove(o)
        got.clear()
        time.sleep(0.6)
        o2 = bus.owner(hlc=False)
        far2 = stamped_clock(o2.session, 900 * MS)
        o2.publish(CMD, b"far", timestamp=far2())
        time.sleep(0.2)
        ids = [str(t.get_id()) for t in got if t is not None]
        report.check(sec, "core §4.1: the same put from an owner whose session has no HLC passes it unchanged, and "
                          "R1 re-stamps it",
                     len(ids) == 1 and _zv(ids[0]) == _zv(bus.r_zid), f"ids {ids}, R1 {bus.r_zid}")
        sub.undeclare()
    finally:
        bus.close()


def section_first(report: Report, starts: int = 20) -> None:
    """F-266's measurement: a new C, then a new owner instance that puts cmd
    at once after its start, ``starts`` times; how often C handled that
    first sample before the instance's first descriptor, though the
    descriptor's put was sent first (core §3.3)."""
    sec = "first"
    bus = Bus()
    early = 0
    gaps: list[float] = []
    try:
        for _ in range(starts):
            c = bus.reader([RID_CMD], 100)
            time.sleep(0.3)
            o = bus.owner()
            o.publish(CMD, b"0")
            time.sleep(0.15)
            arr = c.arrivals_of(CMD)
            d = c.descriptor_at.get(o.instance)
            if arr and d is not None:
                gaps.append((d[0] - arr[0].mono_ns) / MS)
                early += arr[0].verdict.reason == "unattributed"
            c.close()
            bus.readers.remove(c)
            o.close()
            bus.owners.remove(o)
            time.sleep(0.1)
    finally:
        bus.close()
    report.note(sec, f"{early} of {starts} first samples, put at once after the owner's start, handled before the "
                     f"instance's first descriptor and judged unattributed; the descriptor came "
                     f"{min(gaps, default=0):+.3f} to {max(gaps, default=0):+.3f} ms after the first sample "
                     f"(F-266)")


SECTIONS = {"1": section1, "2": section2, "3": section3, "4": section4, "5": section5, "6": section6,
            "hlc": section_hlc, "first": section_first}


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="python -m zk2py.timing_scenarios", description=__doc__.split("\n")[0])
    ap.add_argument("--only", action="append", choices=list(SECTIONS), help="run only these sections")
    args = ap.parse_args(argv)
    report = Report()
    for name, fn in SECTIONS.items():
        if args.only and name not in args.only:
            continue
        try:
            fn(report)
        except Exception as e:  # noqa: BLE001 - a section that crashed is a failure, the rest still run
            import traceback

            traceback.print_exc()
            report.check(name, "the section ran to its end", False, f"{type(e).__name__}: {e}")
    passed = sum(1 for r in report.rows if r[2])
    failed = len(report.rows) - passed
    print(f"timing scenarios: {passed} passed, {failed} failed")
    for sec, name, ok, detail in report.rows:
        if not ok:
            print(f"FAIL [{sec}] {name}: {detail}")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())

"""``timing.v1``'s scenarios (``spec/profiles/timing/scenarios.md``, text
0.2) in every pairing of zk2py's runtime and the Rust reference's: ``python
-m zk2py.timing_interop [--pair OWNER:READER]... [--only 1 … 6]...``, each of
OWNER and READER ``py`` or ``rs`` (#609, round 78).

- **The bus** is the text's: R1, an in-process zenoh-python router with
  timestamping on and its HLC's delta the default; §5 step 2's R2 has
  timestamping off. Every listener is on a free loopback port.
- **The owners.** zk2py's is :class:`PyPace`, :class:`zk2py.owner.Owner`
  driven as the reference's ``owner --timing`` is (its ``--help``): each
  member published every 100 ms while it is on, its sequence number as
  text, stamped from the host's wall clock plus the offset under the
  session's zid (own), or put unstamped with the wall clock less 10 s as
  its payload (none). The reference's is :class:`RsPace`, the black box
  ``owner --connect <R1> --timing [--no-hlc] lab/pace``, driven by its
  ``timing …`` commands.
- **The readers.** zk2py's is :class:`zk2py.timing_live.Reader`, the
  reference's the black box ``consume --timing``, one process per resource.
  Both are C: no HLC on their sessions, the owner's presence followed on a
  watch session. The black box's instants are on its own monotonic clock
  from its origin; the runner places them on its own by the ``subscribed``
  line's arrival (the host's monotonic clock is one clock, so the error is
  the pipe's, well under a millisecond), and compares a due with an
  arrival on the reader's own clock, exactly. Where a check needs the
  instant the reader first saw its owner present, the runner polls the
  black box's ``state`` every 10 ms, and checks the due falls between the
  owner's start and the first poll that answered ``present``.
- **§6** is a tool's: zk2py's :class:`zk2py.timing_live.Window`, beside
  each owner. The reference tool is #908's MC.

Exit 0 when every check passes, 1 when any fails.
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import threading
import time
from dataclasses import dataclass
from decimal import Decimal
from pathlib import Path
from typing import Any, Callable

from . import timing as tm

REPO = Path(__file__).resolve().parents[3]
PACE = REPO / "impl" / "python" / "interop" / "timing" / "pace.v1.toml"
BLACKBOX = Path(os.environ.get("ZK2PY_BLACKBOX", "/srv/dev/targets/zenkey-tier/blackbox"))
ADDRESS = "lab/pace"
BASE = "zk2/lab/pace/pace.v1/stream"
CMD = f"{BASE}/cmd"
AXIS = f"{BASE}/axis"
RID_CMD, RID_AXIS = "stream/cmd", "stream/axis/{axis}"
MS, NS = 1_000_000, 1_000_000_000
BOUND_MS, RUNNER_MS = 10, 50
PERIOD_S = 0.1


class Report:
    def __init__(self) -> None:
        self.rows: list[tuple[str, str, bool, str]] = []
        self.late: list[tuple[str, float]] = []

    def check(self, section: str, name: str, ok: bool, detail: str = "") -> bool:
        self.rows.append((section, name, ok, detail))
        print(f"{'PASS' if ok else 'FAIL'} [{section}] {name}" + (f": {detail}" if detail else ""), flush=True)
        return ok

    def note(self, section: str, text: str) -> None:
        print(f"NOTE [{section}] {text}", flush=True)


def _sleep_until(mono_ns: int) -> None:
    d = (mono_ns - time.monotonic_ns()) / NS
    if d > 0:
        time.sleep(d)


def _wait(cond: Callable[[], bool], timeout_s: float, step_s: float = 0.005) -> bool:
    end = time.monotonic() + timeout_s
    while time.monotonic() < end:
        if cond():
            return True
        time.sleep(step_s)
    return cond()


def _ms(ns: int | None) -> str:
    return "None" if ns is None else f"{ns / MS:.3f} ms"


def _zv(text: Any) -> int | None:
    return None if text is None else tm.zid_value(str(text))


def _ns(ms: Any) -> int:
    """A black box's milliseconds, "a float exact to the nanosecond", read
    through their decimal spelling."""
    return int((Decimal(str(ms)) * MS).to_integral_value())


# -- the bus ------------------------------------------------------------------------------

class Bus:
    """R1 (or R2), the contract, and the runner's own sessions; S, a
    subscriber of the runner's to ``lab/pace``'s data."""

    def __init__(self, timestamping: bool = True) -> None:
        import zenoh

        from .contract import load_contract
        from .live_interop import _r1

        self.router, self.endpoint, self.r_zid = _r1(timestamping=timestamping)
        self.contract = load_contract(PACE)
        assert self.contract.valid, self.contract.codes
        self.sessions: list[Any] = []
        self.parties: list[Any] = []
        self.seen: list[tuple[int, str, bytes]] = []
        self.lock = threading.Lock()
        s = self.client()

        def got(sample: Any) -> None:
            with self.lock:
                self.seen.append((time.monotonic_ns(), str(sample.key_expr), sample.payload.to_bytes()))

        self.sub = s.declare_subscriber(f"{BASE}/**", zenoh.handlers.Callback(got))

    def client(self) -> Any:
        from . import live

        s = live.open_client(self.endpoint)
        self.sessions.append(s)
        return s

    def seen_of(self, key: str, since: int = 0) -> list[tuple[int, bytes]]:
        with self.lock:
            return [(t, p) for t, k, p in self.seen if k == key and t >= since]

    def owner(self, impl: str, hlc: bool = True) -> Any:
        o = PyPace(self, hlc) if impl == "py" else RsPace(self, hlc)
        self.parties.append(o)
        return o

    def reader(self, impl: str, rid: str, epsilon_ms: int | None = None, params: dict[str, str] | None = None) -> Any:
        c = PyC(self, rid, epsilon_ms, params) if impl == "py" else RsC(self, rid, epsilon_ms, params)
        self.parties.append(c)
        return c

    def close(self) -> None:
        for p in reversed(self.parties):
            try:
                p.close()
            except Exception:  # noqa: BLE001 - closing anyway
                pass
        try:
            self.sub.undeclare()
        except Exception:  # noqa: BLE001
            pass
        for s in self.sessions:
            try:
                s.close()
            except Exception:  # noqa: BLE001
                pass
        self.router.close()


# -- the owners -----------------------------------------------------------------------------

def _payload_10s_old() -> bytes:
    return str(time.time_ns() - 10 * NS).encode()


class PyPace:
    """zk2py's owner of ``pace.v1`` at ``lab/pace``, driven as the
    reference's ``owner --timing``: ``cmd`` starts paused, an axis on once
    added; each member on is published every 100 ms."""

    impl = "zk2py"

    def __init__(self, bus: Bus, hlc: bool = True) -> None:
        from .owner import Owner
        from .timing_live import stamped_clock

        self.spawned = time.monotonic_ns()
        self.o = Owner("lab", "pace", [bus.contract], connect=bus.endpoint, hlc=hlc)
        self.o.start()
        self.instance = self.o.instance
        self.zid = json.loads(self.o.descriptor)["meta"]["zid"]
        self.offset_ns = 0
        self.mode = "own"
        self.clock = stamped_clock(self.o.session, lambda: self.offset_ns)
        self.lock = threading.Lock()
        self.on: dict[str, bool] = {"cmd": False}
        self.count: dict[str, int] = {"cmd": 0}
        self.closed = False
        self.threads: dict[str, threading.Thread] = {}
        self._go("cmd")

    def _key(self, member: str) -> str:
        return f"{BASE}/{member}"

    def _go(self, member: str) -> None:
        th = threading.Thread(target=self._run, args=(member,), daemon=True)
        self.threads[member] = th
        th.start()

    def _run(self, member: str) -> None:
        nxt: int | None = None
        while not self.closed:
            with self.lock:
                on = self.on.get(member, False)
            if not on:
                nxt = None
                time.sleep(0.002)
                continue
            now = time.monotonic_ns()
            if nxt is None:
                nxt = now
            if now < nxt:
                time.sleep(min((nxt - now) / NS, 0.002))
                continue
            with self.lock:
                n = self.count[member]
                self.count[member] = n + 1
                mode = self.mode
            try:
                if mode == "own":
                    self.o.publish(self._key(member), str(n).encode(), timestamp=self.clock())
                else:
                    self.o.publish(self._key(member), _payload_10s_old())
            except Exception:  # noqa: BLE001 - closed while putting
                return
            nxt += int(PERIOD_S * NS)

    def resume(self, member: str) -> None:
        with self.lock:
            self.on[member] = True

    def pause(self, member: str) -> None:
        with self.lock:
            self.on[member] = False

    def add(self, axis: str, publish: bool = True) -> int:
        """The member token declared, then the member published; ``publish``
        False declares the token alone (scenarios §3 step 5)."""
        self.o.hold_member("pace.v1", axis)
        at = time.monotonic_ns()
        m = f"axis/{axis}"
        with self.lock:
            self.count.setdefault(m, 0)
            self.on[m] = publish
        if m not in self.threads:
            self._go(m)
        return at

    def remove(self, axis: str) -> None:
        self.pause(f"axis/{axis}")
        time.sleep(0.005)
        self.o.release_member("pace.v1", axis)

    def offset(self, ms: int) -> None:
        self.offset_ns = ms * MS

    def stamp(self, mode: str) -> None:
        with self.lock:
            self.mode = mode

    def published(self) -> dict[str, int]:
        with self.lock:
            return dict(self.count)

    def close(self) -> None:
        if self.closed:
            return
        self.closed = True
        self.o.close()


class Lines:
    """A black box: its standard output's lines, each with its arrival on
    the runner's monotonic clock, and its standard error's."""

    def __init__(self, args: list[str]) -> None:
        self.args = args
        self.p = subprocess.Popen(args, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                  text=True, bufsize=1)
        self.out: list[tuple[int, str]] = []
        self.err: list[str] = []
        self.lock = threading.Lock()
        threading.Thread(target=self._pump, args=(self.p.stdout, True), daemon=True).start()
        threading.Thread(target=self._pump, args=(self.p.stderr, False), daemon=True).start()

    def _pump(self, f: Any, out: bool) -> None:
        for line in f:
            with self.lock:
                if out:
                    self.out.append((time.monotonic_ns(), line.rstrip("\n")))
                else:
                    self.err.append(line.rstrip("\n"))

    def send(self, line: str) -> None:
        assert self.p.stdin is not None
        self.p.stdin.write(line + "\n")
        self.p.stdin.flush()

    def lines(self) -> list[tuple[int, str]]:
        with self.lock:
            return list(self.out)

    def ask(self, line: str, pred: Callable[[str], bool], timeout_s: float = 5.0) -> str | None:
        n = len(self.lines())
        self.send(line)
        got: list[str] = []

        def found() -> bool:
            for _, x in self.lines()[n:]:
                if pred(x):
                    got.append(x)
                    return True
            return False

        _wait(found, timeout_s, 0.001)
        return got[0] if got else None

    def stop(self) -> None:
        try:
            if self.p.stdin is not None:
                self.p.stdin.close()
        except OSError:
            pass
        try:
            self.p.wait(timeout=10.0)
        except subprocess.TimeoutExpired:
            self.p.kill()
            self.p.wait(timeout=5.0)


class RsPace:
    """The reference's owner of ``pace.v1``: ``owner --connect <R1> --timing
    [--no-hlc] lab/pace``, a black box."""

    impl = "rust"

    def __init__(self, bus: Bus, hlc: bool = True) -> None:
        self.spawned = time.monotonic_ns()
        args = [str(BLACKBOX / "owner"), "--connect", bus.endpoint, "--timing"] + ([] if hlc else ["--no-hlc"])
        self.box = Lines(args + [ADDRESS])
        ok = _wait(lambda: any(x.startswith("ready ") for _, x in self.box.lines()) or self.box.p.poll() is not None,
                   15.0)
        ready = next((x for _, x in self.box.lines() if x.startswith("ready ")), None)
        if not ok or ready is None:
            raise RuntimeError(f"owner did not start: {self.box.err}")
        self.instance = ready.rsplit("/", 1)[1]
        self.zid = json.loads(self._ask("timing zid", "zid"))["zid"]
        self.closed = False

    def _ask(self, line: str, field: str) -> str:
        got = self.box.ask(line, lambda x: x.startswith("{") and json.loads(x).get("timing") == field
                           or x.startswith("refused"))
        if got is None or got.startswith("refused"):
            raise RuntimeError(f"{line!r}: {got}")
        return got

    def resume(self, member: str) -> None:
        self._ask(f"timing resume {member}", "resume")

    def pause(self, member: str) -> None:
        self._ask(f"timing pause {member}", "pause")

    def add(self, axis: str, publish: bool = True) -> int:
        if not publish:
            raise NotImplementedError("the reference's timing add publishes at once")
        at = time.monotonic_ns()
        self._ask(f"timing add {axis}", "add")
        return at

    def remove(self, axis: str) -> None:
        self._ask(f"timing remove {axis}", "remove")

    def offset(self, ms: int) -> None:
        self._ask(f"timing offset {ms}", "offset")

    def stamp(self, mode: str) -> None:
        self._ask(f"timing stamp {mode}", "stamp")

    def published(self) -> dict[str, int]:
        return json.loads(self._ask("timing published", "published"))["members"]

    def close(self) -> None:
        if self.closed:
            return
        self.closed = True
        self.box.stop()


# -- the readers ------------------------------------------------------------------------------

@dataclass
class A:
    """An arrival as a reader recorded it: ``c`` on the reader's own clock,
    ``at`` on the runner's."""

    key: str
    c: int
    at: int
    wall_ns: int
    stamp_id: str | None
    age_ns: int | None
    reason: str
    delivered: bool
    payload: str | None


@dataclass
class R:
    event: str
    key: str
    c: int
    at: int
    due: int | None
    by: int | None
    owner: str | None
    reason: str | None


class PyC:
    """zk2py's C (:class:`zk2py.timing_live.Reader`), its sessions clients
    of R1 without an HLC."""

    impl = "zk2py"

    def __init__(self, bus: Bus, rid: str, epsilon_ms: int | None, params: dict[str, str] | None) -> None:
        from .timing_live import Reader

        self.handed: list[str] = []
        self.r = Reader(bus.client(), bus.client(), ADDRESS, bus.contract, [rid], epsilon_ms=epsilon_ms,
                        params=params, on_value=lambda k, p: self.handed.append(k))
        assert self.r.start_ns is not None
        self.start_c = self.start_ns = self.r.start_ns

    def to_c(self, at: int) -> int:
        return at

    def arrivals(self, key: str | None = None) -> list[A]:
        with self.r.lock:
            return [A(a.key, a.mono_ns, a.mono_ns, a.wall_ns, a.stamp_id,
                      None if a.stamp_ns is None else a.wall_ns - a.stamp_ns, a.verdict.reason, a.handed,
                      a.payload.decode(errors="replace"))
                    for a in self.r.arrivals if key is None or a.key == key]

    def reports(self, key: str | None = None, event: str | None = None) -> list[R]:
        with self.r.lock:
            return [R(r.event, r.member, r.at_ns, r.at_ns, r.due_ns, r.by_ns, r.owner, r.reason)
                    for _, r in self.r.reports if (key is None or r.member == key) and (event is None or r.event == event)]

    def present_window(self) -> tuple[int | None, int | None]:
        """The instant C first saw its owner present, as an interval on C's
        clock: exact here."""
        p = self.r.present_at()
        return p, p

    def holds_zid(self, zid: str) -> bool:
        with self.r.lock:
            return any(_zv((d.get("meta") or {}).get("zid")) == _zv(zid) for d in self.r.descriptors.values())

    def seen_window(self, key: str, sent: int) -> tuple[int | None, int | None]:
        with self.r.lock:
            at = (self.r.member_seen_at.get(key) or [None])[0]
        return at, at

    def owner_state(self) -> str:
        return self.r.owner_state

    def track_presence(self, on: bool) -> None:
        pass

    def close(self) -> None:
        self.r.close()


class RsC:
    """The reference's C: ``consume --timing [--clock-bound-ms ε] [--param
    n=v]... <R1> lab/pace pace.v1.toml <resource>``, a black box."""

    impl = "rust"

    def __init__(self, bus: Bus, rid: str, epsilon_ms: int | None, params: dict[str, str] | None) -> None:
        args = [str(BLACKBOX / "consume"), "--timing"]
        if epsilon_ms is not None:
            args += ["--clock-bound-ms", str(epsilon_ms)]
        for k, v in (params or {}).items():
            args += ["--param", f"{k}={v}"]
        self.box = Lines(args + [bus.endpoint, ADDRESS, str(PACE), rid])
        ok = _wait(lambda: self._subscribed() is not None or self.box.p.poll() is not None, 15.0)
        sub = self._subscribed()
        if not ok or sub is None:
            raise RuntimeError(f"consume did not subscribe: {self.box.err}")
        t_recv, line = sub
        self.start_c = _ns(line["since_ms"])
        #: the reader's origin on the runner's clock, by the line's arrival
        self.origin = t_recv - self.start_c
        self.start_ns = self.origin + self.start_c
        self._present: int | None = None
        self._zids: list[str] = []
        #: the owner's state as the last ``state`` answer gave it
        self._state: str | None = None
        self._polling = threading.Event()
        self._polled_at = 0
        self.closed = False
        threading.Thread(target=self._poll, daemon=True).start()

    def _subscribed(self) -> tuple[int, dict[str, Any]] | None:
        for t, x in self.box.lines():
            if x.startswith("{"):
                d = json.loads(x)
                if d.get("timing") == "subscribed":
                    return t, d
        return None

    def to_c(self, at: int) -> int:
        return at - self.origin

    def _json(self) -> list[tuple[int, dict[str, Any]]]:
        out = []
        for t, x in self.box.lines():
            if x.startswith("{"):
                try:
                    out.append((t, json.loads(x)))
                except ValueError:
                    pass
        return out

    def arrivals(self, key: str | None = None) -> list[A]:
        out = []
        for _, d in self._json():
            if d.get("timing") != "arrival" or (key is not None and d["key"] != key):
                continue
            st = d.get("stamp")
            c = _ns(d["at_ms"])
            out.append(A(d["key"], c, self.origin + c, int(d["wall_ns"]) if d.get("wall_ns") is not None else -1, None if st is None else st["id"],
                         None if st is None else _ns(st["age_ms"]), d["lifespan"]["reason"], bool(d["delivered"]),
                         None))
        return sorted(out, key=lambda a: a.c)

    def reports(self, key: str | None = None, event: str | None = None) -> list[R]:
        out = []
        for _, d in self._json():
            ev = d.get("timing")
            if ev not in ("missed", "met", "ended", "beyond_bound"):
                continue
            if (key is not None and d["key"] != key) or (event is not None and ev != event):
                continue
            c = _ns(d["at_ms"])
            out.append(R(ev, d["key"], c, self.origin + c, _ns(d["due_ms"]) if "due_ms" in d else None,
                         _ns(d["by_ms"]) if "by_ms" in d else None, d.get("owner"), d.get("reason")))
        return sorted(out, key=lambda r: r.c)

    def track_presence(self, on: bool) -> None:
        """Poll ``state`` every 10 ms while on."""
        if on:
            self._polling.set()
        else:
            self._polling.clear()

    def _poll(self) -> None:
        while not self.closed:
            if not self._polling.is_set():
                time.sleep(0.005)
                continue
            got = self.box.ask(f"state {ADDRESS}", lambda x: x.startswith("{") and json.loads(x).get("timing") == "state",
                               1.0)
            now = time.monotonic_ns()
            if got is not None:
                d = json.loads(got)
                self._zids = list(d.get("owner_zids") or [])
                self._state = d.get("owner")
                if d.get("owner") == "present" and self._present is None:
                    self._present = now
            self._polled_at = now
            time.sleep(0.01)

    def present_window(self) -> tuple[int | None, int | None]:
        """The instant C first saw its owner present, as an interval on C's
        clock: no earlier than the owner's start (the caller's), no later
        than the first poll that answered present."""
        return None, None if self._present is None else self.to_c(self._present) + 2 * MS

    def holds_zid(self, zid: str) -> bool:
        return any(_zv(z) == _zv(zid) for z in self._zids)

    def seen_window(self, key: str, sent: int) -> tuple[int | None, int | None]:
        return self.to_c(sent), None

    def owner_state(self) -> str:
        got = self.box.ask(f"state {ADDRESS}", lambda x: x.startswith("{") and json.loads(x).get("timing") == "state")
        return "?" if got is None else json.loads(got)["owner"]

    @property
    def handed(self) -> list[str]:
        return [a.key for a in self.arrivals() if a.delivered]

    def close(self) -> None:
        if self.closed:
            return
        self.closed = True
        self.box.stop()


def _due_ok(r: R, due_c: int) -> tuple[bool, int]:
    late = r.c - (r.due or 0) - BOUND_MS * MS
    return r.due == due_c and r.c > due_c and late <= RUNNER_MS * MS, max(0, late)


# -- §1 ----------------------------------------------------------------------------------------

def section1(report: Report, oi: str, ci: str) -> None:
    from . import live

    sec = f"§1 [{oi}→{ci}]"
    bus = Bus()
    try:
        c = bus.reader(ci, RID_CMD)
        time.sleep(0.3)
        o = bus.owner(oi)
        o.resume("cmd")
        t1 = time.monotonic_ns()
        _sleep_until(t1 + 2 * NS)
        o.pause("cmd")
        t1_end = time.monotonic_ns()
        time.sleep(0.05)
        step1 = [a for a in c.arrivals(CMD) if a.at <= t1_end + 50 * MS]
        rep1 = [r for r in c.reports(CMD) if r.at <= t1_end + 50 * MS]
        gaps = [(b.c - a.c) / MS for a, b in zip(step1, step1[1:])]
        report.check(sec, "Expected 1: C reports nothing in step 1, its first arrival included (cmd known, its "
                          "owner present, every gap at most 500 ms)",
                     not rep1 and len(step1) >= 18 and max(gaps, default=0) <= 500,
                     f"{len(step1)} arrivals, largest gap {max(gaps, default=0):.1f} ms, reports "
                     f"{[(r.event, _ms(r.at - t1)) for r in rep1]}")
        _sleep_until(t1_end + 2 * NS)
        t2_end = time.monotonic_ns()
        pres = live.list_presence(bus.client(), "zk2/lab/pace/@zk/**")
        last1 = step1[-1].c if step1 else 0
        in2 = [a for a in c.arrivals(CMD) if t1_end + 50 * MS < a.at <= t2_end]
        missed = c.reports(CMD, "missed")
        ok_due, late = (False, 0) if len(missed) != 1 else _due_ok(missed[0], last1 + 500 * MS)
        report.late.append((sec, late / MS))
        report.check(sec, "Expected 2: one missed for cmd, due exactly 500 ms after step 1's last arrival, reported "
                          "within the 10 ms bound (50 ms beyond allowed); no sample in step 2; nothing more",
                     len(missed) == 1 and ok_due and not in2
                     and [r.event for r in c.reports(CMD) if r.at <= t2_end] == ["missed"],
                     f"missed {[(_ms(r.due - last1), _ms(r.c - r.due), r.owner) for r in missed]}; "
                     f"{len(in2)} samples in step 2")
        report.check(sec, "Expected 4: T reads the owner present (instance and interface tokens, complete), and "
                          "C's miss stands beside it",
                     pres.complete and len(pres.instances) == 1 and [a["iface"] for a in pres.alive] == ["pace.v1"]
                     and len(missed) == 1 and missed[0].owner == "present" and not c.reports(CMD, "met"),
                     f"presence {pres.reading}; C's miss owner {missed[0].owner if missed else None}")
        t3 = time.monotonic_ns()
        o.resume("cmd")
        time.sleep(1.0)
        o.pause("cmd")
        time.sleep(0.2)
        after = [r for r in c.reports(CMD) if r.at > t2_end]
        first3 = next((a for a in c.arrivals(CMD) if a.at >= t3), None)
        report.check(sec, "Expected 3: met once, at step 3's first arrival, and nothing after (C stopped within a "
                          "deadline of the last sample)",
                     [r.event for r in after] == ["met"] and first3 is not None and after[0].c == first3.c,
                     f"{[(r.event, _ms(r.at - t3)) for r in after]}; first arrival "
                     f"{_ms(first3.at - t3) if first3 else None} after step 3 began")
        c.close()
    finally:
        bus.close()


# -- §2 ----------------------------------------------------------------------------------------

def section2(report: Report, oi: str, ci: str) -> None:
    sec = f"§2 [{oi}→{ci}]"
    bus = Bus()
    try:
        c = bus.reader(ci, RID_AXIS)
        time.sleep(0.3)
        o = bus.owner(oi)
        kx, ky = f"{AXIS}/x", f"{AXIS}/y"
        o.add("x")
        o.add("y")
        t0 = time.monotonic_ns()
        _sleep_until(t0 + 1 * NS)
        t_w = time.monotonic_ns()
        o.remove("y")
        rep1 = [r for r in c.reports() if r.at <= t_w]
        report.check(sec, "Expected 1: C reports nothing in step 1: neither member known, each judged from its "
                          "first arrival",
                     not rep1 and len(c.arrivals(kx)) >= 8 and len(c.arrivals(ky)) >= 8,
                     f"reports {[(r.event, r.key) for r in rep1]}; {len(c.arrivals(kx))} x, {len(c.arrivals(ky))} y")
        _sleep_until(t_w + 2 * NS)
        o.pause("axis/x")
        t_x = time.monotonic_ns()
        ys = c.arrivals(ky)
        last_y = ys[-1].c if ys else 0
        ended = c.reports(ky, "ended")
        detail = ""
        read_ok = True
        if isinstance(c, PyC):
            f = c.r.follower
            with f.lock:
                heard = [t for t, k, kind in f.view.events if kind == "delete" and "/@zk/member/pace.v1/y/" in k]
                reads = list(zip(f.reads, f.folds, [r.asked_ns for r in f.view.reads]))
            after = [(p, k) for p, k, a in reads if heard and a >= heard[0]]
            read_ok = bool(after) and after[0][1] == ("complete", True) \
                and not any(m["member"] == "y" for m in after[0][0].members)
            detail = f"; the read after the withdrawal {after[0][1] if after else None}"
        report.check(sec, "Expected 2: ended for axis/y (gone) at the read after its member token's withdrawal, "
                          "never missed, long before its due; nothing of axis/x in step 2",
                     read_ok and len(ended) == 1 and ended[0].reason == "gone" and ended[0].at > t_w
                     and ended[0].c < last_y + 500 * MS and not c.reports(ky, "missed")
                     and not [r for r in c.reports(kx) if r.at <= t_x],
                     f"ended {[(r.reason, _ms(r.at - t_w)) for r in ended]} after the withdrawal" + detail)
        _sleep_until(t_x + 1 * NS)
        xs = c.arrivals(kx)
        last_x = xs[-1].c if xs else 0
        missed = c.reports(kx, "missed")
        ok_due, late = (False, 0) if len(missed) != 1 else _due_ok(missed[0], last_x + 500 * MS)
        report.late.append((sec, late / MS))
        report.check(sec, "Expected 3: missed for axis/x, due 500 ms after its last arrival, within the bound: a "
                          "member whose token is held is still judged",
                     len(missed) == 1 and ok_due,
                     f"{[(_ms(r.due - last_x), _ms(r.c - r.due), r.owner) for r in missed]}")
        c.close()
    finally:
        bus.close()


# -- §3 ----------------------------------------------------------------------------------------

def _raw_member_token(bus: Bus, axis: str) -> tuple[Any, int]:
    """A member token at ``lab/pace``'s address declared by a raw session of
    the runner's: the stand-in, with the reference's owner, for "the owner
    declares x's member token … and publishes nothing on it", which its
    ``timing add`` cannot do (it publishes at once)."""
    s = bus.client()
    at = time.monotonic_ns()
    tok = s.liveliness().declare_token(f"zk2/lab/pace/@zk/member/pace.v1/{axis}/00000000000000f1")
    return tok, at


def section3(report: Report, oi: str, ci: str) -> None:
    sec = f"§3 [{oi}→{ci}]"
    bus = Bus()
    try:
        c = bus.reader(ci, RID_CMD)
        c.track_presence(True)
        s_c = c.start_c
        _sleep_until(c.start_ns + 1 * NS)
        report.check(sec, "Expected 1: nothing in the second before the owner (cmd known, its owner not present)",
                     not c.reports(), f"reports {[(r.event, r.key) for r in c.reports()]}")
        o = bus.owner(oi)
        spawned = c.to_c(o.spawned)
        _wait(lambda: c.present_window()[1] is not None, 3.0)
        p_lo, p_hi = c.present_window()
        p_lo = spawned if p_lo is None else p_lo
        c.track_presence(False)
        if p_hi is None:
            report.check(sec, "Expected 2: C sees the owner present", False, "never present")
            return
        _sleep_until((p_hi if isinstance(c, PyC) else c.origin + p_hi) + 1 * NS)
        t_pub = time.monotonic_ns()
        if isinstance(o, PyPace):
            o.o.publish(CMD, b"0", timestamp=o.clock())
        else:
            o.resume("cmd")
            _wait(lambda: bool(bus.seen_of(CMD, t_pub)), 1.0, 0.001)
            o.pause("cmd")
        time.sleep(0.2)
        missed = c.reports(CMD, "missed")
        p_c = None if len(missed) != 1 or missed[0].due is None else missed[0].due - 500 * MS
        ok = len(missed) == 1 and p_c is not None and p_lo <= p_c <= p_hi and missed[0].c > missed[0].due \
            and missed[0].c - missed[0].due <= (BOUND_MS + RUNNER_MS) * MS and missed[0].owner == "present"
        if missed:
            report.late.append((sec, max(0, missed[0].c - (missed[0].due or 0) - BOUND_MS * MS) / MS))
        report.check(sec, "Expected 2: missed for cmd, due at P + 500 ms (P, the later of S and P, C's first sight "
                          "of its owner present), within the bound",
                     ok, f"P {_ms(p_c - s_c) if p_c is not None else None} after S, within "
                         f"[{_ms(p_lo - s_c)}, {_ms(p_hi - s_c)}]; reported {_ms(missed[0].c - missed[0].due) if missed else None} "
                         f"after the due")
        arr = c.arrivals(CMD)
        met = c.reports(CMD, "met")
        report.check(sec, "Expected 3: met at the arrival",
                     len(arr) == 1 and len(met) == 1 and met[0].c == arr[0].c,
                     f"{len(arr)} arrivals, met {[_ms(r.at - t_pub) for r in met]} after the put")
        c.close()
        c2 = bus.reader(ci, RID_AXIS)
        time.sleep(2.0)
        report.check(sec, "Expected 4: C2 reports nothing over 2 s: no axis member known, none arrived",
                     not c2.reports() and not c2.arrivals(), f"reports {[(r.event, r.key) for r in c2.reports()]}")
        c2.close()
        # Step 5 (text 0.2, F-261): C3 binds axis = x.
        c3 = bus.reader(ci, RID_AXIS, params={"axis": "x"})
        kx = f"{AXIS}/x"
        time.sleep(1.0)
        quiet = not c3.reports()
        raw = None
        if isinstance(o, PyPace):
            sent = o.add("x", publish=False)
        else:
            raw, sent = _raw_member_token(bus, "x")
        time.sleep(0.7)
        m_lo, m_hi = c3.seen_window(kx, sent)
        missed = c3.reports(kx, "missed")
        m_c = None if len(missed) != 1 or missed[0].due is None else missed[0].due - 500 * MS
        window_ok = m_c is not None and m_lo is not None and m_lo <= m_c and (m_hi is None or m_c <= m_hi) \
            and (m_hi is not None or m_c <= m_lo + 50 * MS)
        report.check(sec, "Expected 5: C3 (axis = x, known) reports nothing in its first second, its owner present; "
                          "then missed for axis/x, due at M + 500 ms (M its first sight of x's member token), "
                          "within the bound"
                          + ("" if raw is None else " (x's token declared by a raw session: the reference owner's "
                                                    "timing add publishes at once)"),
                     quiet and len(missed) == 1 and window_ok and not c3.arrivals()
                     and missed[0].c - missed[0].due <= (BOUND_MS + RUNNER_MS) * MS,
                     f"first second quiet {quiet}; missed {[(_ms(r.due - c3.to_c(sent)), _ms(r.c - r.due), r.owner) for r in missed]} "
                     f"(due after the declaration, reported after the due); M within "
                     f"[{_ms(None if m_lo is None else m_lo - c3.to_c(sent))}, "
                     f"{_ms(None if m_hi is None else m_hi - c3.to_c(sent))}] of the declaration")
        if raw is not None:
            raw.undeclare()
        c3.close()
    finally:
        bus.close()


# -- §4 ----------------------------------------------------------------------------------------

def section4(report: Report, oi: str, ci: str) -> None:
    sec = f"§4 [{oi}→{ci}]"
    bus = Bus()
    try:
        steps = [("1", 100, [(0.0, 0)]), ("2", 100, [(0.0, -700)]), ("3", 100, [(0.0, 400), (1.0, 0)]),
                 ("4", None, [(0.0, -700)])]
        for name, eps, offsets in steps:
            c = bus.reader(ci, RID_CMD, eps)
            c.track_presence(True)
            time.sleep(0.3)
            # The reference's owner offsets its stamps alone with its HLC
            # off (its --help: "an HLC would follow them", core §4.1): with
            # it on, its own stamp 400 ms ahead holds its HLC there, and its
            # later stamps stand still until its wall clock catches up.
            # zk2py's stamps from the offset wall clock either way.
            o = bus.owner(oi, hlc=oi == "py")
            spawned = c.to_c(o.spawned)
            # F-266: the owner publishes once C holds its descriptor, where C
            # judges a lifespan; with no ε, once C sees its owner present (the
            # reference's C reads no meta.zid then, and step 2 of the
            # scenarios is nothing to it)
            if eps is not None:
                held = _wait(lambda: c.holds_zid(o.zid), 3.0)
            else:
                held = _wait(lambda: c.present_window()[1] is not None, 3.0)
            c.track_presence(False)
            o.offset(offsets[0][1])
            t0 = time.monotonic_ns()
            o.resume("cmd")
            for at_s, off in offsets[1:]:
                _sleep_until(t0 + int(at_s * NS))
                o.offset(off)
            _sleep_until(t0 + 2 * NS)
            o.pause("cmd")
            time.sleep(0.2)
            arr = c.arrivals(CMD)
            verdicts = [a.reason for a in arr]
            ages = [a.age_ns / MS for a in arr if a.age_ns is not None]
            ours = all(_zv(a.stamp_id) == _zv(o.zid) for a in arr)
            handed = sum(1 for a in arr if a.delivered)
            reps = [r.event for r in c.reports(CMD)]
            detail = (f"{len(arr)} arrivals; descriptor held first {held}; verdicts {sorted(set(verdicts))}; ages "
                      f"{min(ages, default=0):.1f} to {max(ages, default=0):.1f} ms; handed {handed}; stamps the "
                      f"owner's meta.zid {ours}; reports {reps}")
            if name == "1":
                report.check(sec, "Expected 1: every stamp the owner's meta.zid, every sample live and handed on, no "
                                  "deadline transition",
                             len(arr) >= 18 and ours and set(verdicts) == {"live"} and handed == len(arr) and not reps,
                             detail)
            elif name == "2":
                p_lo, p_hi = c.present_window()
                p_lo = spawned if p_lo is None else p_lo
                missed = c.reports(CMD, "missed")
                p_c = None if len(missed) != 1 else missed[0].due - 500 * MS
                due_ok = p_c is not None and p_hi is not None and p_lo <= p_c <= p_hi \
                    and missed[0].c - missed[0].due <= (BOUND_MS + RUNNER_MS) * MS
                report.check(sec, "Expected 2 (700 ms behind): every sample expired, none handed on; missed for cmd, "
                                  "due 500 ms after C first saw its owner present; no met",
                             len(arr) >= 18 and ours and set(verdicts) == {"expired"} and handed == 0 and due_ok
                             and not c.reports(CMD, "met"),
                             detail + f"; P within [{_ms(None if p_hi is None else p_lo - spawned)}, "
                                      f"{_ms(None if p_hi is None else p_hi - spawned)}] of the owner's start, the "
                                      f"due's P {_ms(None if p_c is None else p_c - spawned)}")
            elif name == "3":
                first = [a for a in arr if a.at < t0 + int(0.95 * NS)]
                second = [a for a in arr if a.at >= t0 + int(1.05 * NS)]
                a1 = [a.age_ns / MS for a in first if a.age_ns is not None]
                a2 = [a.age_ns / MS for a in second if a.age_ns is not None]
                report.check(sec, "Expected 3 (400 ms ahead, then agreeing): every sample clock_disagrees, the second "
                                  "second's too (falsified for the reading); all handed on; no deadline transition",
                             len(arr) >= 18 and ours and set(verdicts) == {"clock_disagrees"} and handed == len(arr)
                             and not reps and bool(a1) and max(a1) < -300 and bool(a2),
                             detail + f"; first second's ages {min(a1, default=0):.1f} to {max(a1, default=0):.1f}, "
                                      f"the second's {min(a2, default=0):.1f} to {max(a2, default=0):.1f} ms")
            else:
                report.check(sec, "Expected 4 (700 ms behind, no ε): every sample no_bound, though R1's delta is "
                                  "500 ms; all handed on; no deadline transition",
                             len(arr) >= 18 and ours and set(verdicts) == {"no_bound"} and handed == len(arr)
                             and not reps, detail)
            c.close()
            o.close()
            time.sleep(0.3)
    finally:
        bus.close()


# -- §5 ----------------------------------------------------------------------------------------

def section5(report: Report, oi: str, ci: str) -> None:
    sec = f"§5 [{oi}→{ci}]"
    for step, timestamping, hlc in (("1", True, False), ("2", False, False), ("3", True, True)):
        bus = Bus(timestamping=timestamping)
        try:
            c = bus.reader(ci, RID_CMD, 100)
            c.track_presence(True)
            time.sleep(0.3)
            o = bus.owner(oi, hlc=hlc)
            held = _wait(lambda: c.holds_zid(o.zid), 3.0)
            c.track_presence(False)
            o.stamp("none")
            o.resume("cmd")
            time.sleep(2.0)
            o.pause("cmd")
            time.sleep(0.2)
            arr = c.arrivals(CMD)
            reasons = sorted({a.reason for a in arr})
            handed = sum(1 for a in arr if a.delivered)
            ids = sorted({str(a.stamp_id) for a in arr})
            detail = f"{len(arr)} arrivals; descriptor held first {held}; {reasons}; ids {ids}; handed {handed}"
            if step == "1":
                report.check(sec, "Expected 1: every sample stamped by R1 (not the owner's meta.zid), foreign_stamp, "
                                  "all handed on, no deadline transition",
                             len(arr) >= 18 and all(_zv(a.stamp_id) == _zv(bus.r_zid) for a in arr)
                             and _zv(bus.r_zid) != _zv(o.zid) and reasons == ["foreign_stamp"]
                             and handed == len(arr) and not c.reports(), detail + f" (R1 {bus.r_zid})")
            elif step == "2":
                report.check(sec, "Expected 2 (R2, timestamping off): every sample unstamped, unobservable "
                                  "(unstamped), all handed on",
                             len(arr) >= 18 and all(a.stamp_id is None for a in arr) and reasons == ["unstamped"]
                             and handed == len(arr), detail)
            else:
                old = [a for a in arr if a.payload is not None]
                report.check(sec, "Expected 3 (control, the owner's HLC on, the same payloads): every stamp the "
                                  "owner's meta.zid, every sample live; C never reads the payload's time",
                             len(arr) >= 18 and all(_zv(a.stamp_id) == _zv(o.zid) for a in arr) and reasons == ["live"]
                             and all(time.time_ns() - int(a.payload) > 9 * NS for a in old),
                             detail + f" (meta.zid {o.zid})")
            c.close()
            o.close()
        finally:
            bus.close()


# -- §6, zk2py's tool beside each owner ----------------------------------------------------------

def section6(report: Report, oi: str) -> None:
    from .timing_live import Window

    sec = f"§6 [{oi}→zk2py's tool]"
    bus = Bus()
    try:
        o = bus.owner(oi)
        kx = f"{AXIS}/x"
        o.add("x")
        time.sleep(0.3)

        def run(plan: Callable[[int], threading.Thread]) -> tuple[Window, dict[str, dict[str, tm.Answer]]]:
            w = Window(bus.client(), bus.client(), ADDRESS, bus.contract, [RID_CMD, RID_AXIS], epsilon_ms=100)
            assert w.start_ns is not None
            th = plan(w.start_ns)
            _sleep_until(w.start_ns + 5 * NS)
            got = w.end()
            w.close()
            # the step's own commands end before the next window opens
            th.join(2.0)
            return w, got

        def every(period_s: float, pause: tuple[float, float] | None = None, close_at: float | None = None):
            def plan(start: int) -> threading.Thread:
                def go() -> None:
                    if period_s == PERIOD_S and pause is None:
                        o.resume("cmd")
                        if close_at is not None:
                            _sleep_until(start + int(close_at * NS))
                            o.close()
                            return
                        _sleep_until(start + 5 * NS + 50 * MS)
                        o.pause("cmd")
                        return
                    if pause is not None:
                        o.resume("cmd")
                        _sleep_until(start + int(pause[0] * NS))
                        o.pause("cmd")
                        _sleep_until(start + int(pause[1] * NS))
                        o.resume("cmd")
                        _sleep_until(start + 5 * NS + 50 * MS)
                        o.pause("cmd")
                        return
                    # one sample every period_s: resumed until a sample is seen, then paused
                    i = 0
                    while True:
                        t = start + int(i * period_s * NS)
                        if t >= start + 5 * NS:
                            return
                        _sleep_until(t)
                        t_on = time.monotonic_ns()
                        o.resume("cmd")
                        _wait(lambda: bool(bus.seen_of(CMD, t_on)), 0.2, 0.001)
                        o.pause("cmd")
                        i += 1
                th = threading.Thread(target=go, daemon=True)
                th.start()
                return th
            return plan

        def pairs(got: dict[str, dict[str, tm.Answer]], key: str) -> dict[str, tuple[str, str]]:
            return {q: (a.answer, a.reason) for q, a in got.get(key, {}).items()}

        def show(got: dict[str, dict[str, tm.Answer]]) -> str:
            return "; ".join(f"{k.rsplit('/stream/', 1)[1]}: " + ", ".join(f"{q} {a.answer} ({a.reason})"
                                                                         for q, a in v.items())
                             for k, v in got.items())

        w, got = run(every(PERIOD_S))
        report.check(sec, "Expected 1: cmd met, kept, live; axis/x met, kept, no_lifespan",
                     pairs(got, CMD) == {"deadline": ("yes", "met"), "period": ("yes", "kept"),
                                         "lifespan": ("no", "live")}
                     and pairs(got, kx) == {"deadline": ("yes", "met"), "period": ("yes", "kept"),
                                            "lifespan": ("not_asked", "no_lifespan")}, show(got))
        w, got = run(every(PERIOD_S, (2.0, 3.0)))
        runs = {q: tm.rollup(q, [v[q] for v in got.values()]) for q in ("deadline", "period", "lifespan")}
        report.check(sec, "Expected 2: cmd missed (the owner present), not_kept, live; the run's verdict the finding",
                     pairs(got, CMD) == {"deadline": ("no", "missed"), "period": ("no", "not_kept"),
                                         "lifespan": ("no", "live")}
                     and all(r.owner == tm.PRESENT for r in w.reader.reports_of(CMD, "missed"))
                     and any(a.answer == tm.FINDING[q] for q, a in runs.items()),
                     show(got) + f"; {len(w.reader.arrivals_of(CMD))} cmd arrivals")
        w, got = run(every(0.25))
        report.check(sec, "Expected 3: cmd every 250 ms: deadline met, period not_kept",
                     pairs(got, CMD).get("deadline") == ("yes", "met")
                     and pairs(got, CMD).get("period") == ("no", "not_kept"),
                     show(got) + f"; {len(w.reader.arrivals_of(CMD))} cmd arrivals")
        w, got = run(every(PERIOD_S, close_at=1.0))
        ended = w.reader.reports_of(kx, "ended")
        last_x = max((a.mono_ns for a in w.reader.arrivals_of(kx)), default=0)
        report.check(sec, "Expected 4: the owner closed cleanly 1 s in: cmd owner_not_present twice, lifespan no; "
                          "axis/x ended (gone) at the read, deadline not asked (ended), period kept",
                     pairs(got, CMD) == {"deadline": ("unobservable", "owner_not_present"),
                                         "period": ("unobservable", "owner_not_present"),
                                         "lifespan": ("no", "live")}
                     and len(ended) == 1 and ended[0].reason == "gone" and ended[0].at_ns < last_x + 500 * MS
                     and pairs(got, kx).get("deadline") == ("not_asked", "ended")
                     and pairs(got, kx).get("period") == ("yes", "kept"),
                     show(got) + f"; axis/x ended {[_ms(r.at_ns - last_x) for r in ended]} after its last arrival")
    finally:
        bus.close()


SECTIONS: dict[str, Callable[..., None]] = {"1": section1, "2": section2, "3": section3, "4": section4,
                                            "5": section5}
PAIRS = ["py:py", "rs:py", "py:rs", "rs:rs"]


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="python -m zk2py.timing_interop", description=__doc__.split("\n")[0])
    ap.add_argument("--pair", action="append", choices=PAIRS, help="owner:reader, each py or rs")
    ap.add_argument("--only", action="append", choices=[*SECTIONS, "6"], help="run only these sections")
    args = ap.parse_args(argv)
    report = Report()
    names = {"py": "zk2py", "rs": "rust"}
    pairs = list(args.pair or PAIRS)
    missing = [str(BLACKBOX / b) for b in ("owner", "consume") if not os.access(BLACKBOX / b, os.X_OK)]
    if any("rs" in p for p in pairs) and missing:
        # the reference's black boxes are a path on this machine, ZK2PY_BLACKBOX
        # their override: with them missing, the rs pairings cannot run
        msg = f"the reference's black boxes are missing: {', '.join(missing)} (set ZK2PY_BLACKBOX to their directory)"
        if args.pair:
            print(f"error: {msg}", file=sys.stderr)
            return 2
        print(f"SKIP the rs pairings: {msg}", flush=True)
        pairs = [p for p in pairs if "rs" not in p]
    for pair in pairs:
        oi, ci = pair.split(":")
        for name, fn in SECTIONS.items():
            if args.only and name not in args.only:
                continue
            try:
                fn(report, oi, ci)
            except Exception as e:  # noqa: BLE001 - a section that crashed is a failure, the rest still run
                import traceback

                traceback.print_exc()
                report.check(f"§{name} [{names[oi]}→{names[ci]}]", "the section ran to its end", False,
                             f"{type(e).__name__}: {e}")
    if not args.only or "6" in args.only:
        for oi in sorted({p.split(':')[0] for p in pairs}):
            try:
                section6(report, oi)
            except Exception as e:  # noqa: BLE001
                import traceback

                traceback.print_exc()
                report.check(f"§6 [{oi}]", "the section ran to its end", False, f"{type(e).__name__}: {e}")
    passed = sum(1 for r in report.rows if r[2])
    failed = len(report.rows) - passed
    late = max((x for _, x in report.late), default=0.0)
    print(f"timing interop: {passed} passed, {failed} failed; the largest lateness past the 10 ms bound {late:.3f} ms")
    for sec, name, ok, detail in report.rows:
        if not ok:
            print(f"FAIL [{sec}] {name}: {detail}")
    return 1 if failed else 0


if __name__ == "__main__":
    code = main()
    sys.stdout.flush()
    os._exit(code)

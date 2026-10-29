"""``timing.v1``'s scenarios (``spec/profiles/timing/scenarios.md``, text
0.4) in every pairing of zk2py's runtime and the Rust reference's: ``python
-m zk2py.timing_interop [--pair OWNER:READER]... [--only 1 … 6|face|gone|again]...``, each of
OWNER and READER ``py`` or ``rs`` (#609, rounds 78 to 80).

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
  ``timing …`` commands: ``timing declare`` for §3 step 5 and ``timing
  every`` for §6 step 3 (text 0.3, F-276). §4's owner has no HLC, both
  implementations' (text 0.3, F-273).
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
- **§6** is a tool's, run with both tools at once beside each owner (text
  0.3): zk2py's (``python -m zk2py.timing_tool``) and the reference's
  (``zenctl timing``, a black box), each a process, their windows' starts
  their ``timing: listening`` lines; each tool's answers checked against
  Expected, each tool's roll-ups against §5's rule (text 0.4, F-277), then
  the two compared token by token. The checks an open finding is known to
  fail are named one by one in :data:`KNOWN`, each printing XFAIL when it
  fails and XPASS when it passes; any other failing check fails the run
  (:mod:`zk2py.checks`). None of §1 to §6 is there since text 0.4 (round
  80): F-277's twelve went with it.
- **Beside §6, not scenarios**, both tools beside each owner: ``face``
  (§2.8, across a constrained face on the deployment's word, round 79) and
  ``gone`` (§2.7, a known member never heard whose owner leaves before its
  miss is due, round 80): the two tools' agreement there is F-278's, and a
  roll-up's reason over no member F-279's, in :data:`KNOWN`; ``again``
  (§2.7, a member ended and heard again in one window, round 80, F-280).

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


#: The two tools' readings across a face beside §6 (§2.8), by name.
FACE_ZID, FACE_NONE = "the owner's zid on the deployment's word", "no zid stated"
#: The name of the check, at each of §6's steps and §2.8's readings, that
#: each tool's resource and run roll-ups are §5's over its own member rows
#: (text 0.4, F-277), and that ``stream/axis/{axis}``'s lifespan, no
#: member declaring one, is not asked.
ROLLUP = ("§5's roll-up, each tool over its own members: a resource or run whose every member is set aside not "
          "asked (text 0.4, F-277), axis's lifespan not_asked")
#: The name of the check beside §6 (§2.7, not a scenario) that the two
#: tools agree on a known member never heard whose owner leaves before its
#: miss is due.
GONE_AGREE = "the two tools' answers agree, token by token, and their exits"
#: … and that a resource's roll-up over no member carries the same reason in
#: both (F-279: zk2py ``no_member``, zenctl none; §5 states no roll-up's).
GONE_EMPTY = "a roll-up over no member: the two tools' reasons agree"
#: The checks an open finding is known to fail, each by its identity
#: (section, name), never by a count (review-bot on PR #919). Round 79's
#: twelve F-277 entries went with text 0.4 (round 80), when every one
#: passed. A failing check not here fails the run; one here that passes
#: prints XPASS, so its entry is seen and removed.
KNOWN: dict[tuple[str, str], str] = {
    # F-278 (round 80): §2.7's held time of a known member never heard whose
    # owner went absent before its miss was due, zk2py by the letter (to the
    # window's end: yes, met), zenctl only while it waited (window_too_short)
    **{(f"§2.7 beside §6 [{oi}→both tools]", GONE_AGREE): "F-278" for oi in ("py", "rs")},
    # F-279 (round 80): §5 states no reason for a roll-up; over no member
    # zk2py says ``no_member`` and zenctl prints none
    **{(f"§2.7 beside §6 [{oi}→both tools]", GONE_EMPTY): "F-279" for oi in ("py", "rs")},
}


class Report:
    def __init__(self) -> None:
        self.rows: list[tuple[str, str, bool, str, str | None]] = []
        self.late: list[tuple[str, float]] = []

    def check(self, section: str, name: str, ok: bool, detail: str = "") -> bool:
        """A check whose identity is in :data:`KNOWN` prints XFAIL when it
        fails, naming its finding, and XPASS when it passes."""
        known = KNOWN.get((section, name))
        self.rows.append((section, name, ok, detail, known))
        from .checks import tag

        print(f"{tag(ok, known)} [{section}] {name}" + (f" (known: {known})" if known else "")
              + (f": {detail}" if detail else ""), flush=True)
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
        #: each member's interval, its period unless ``every`` set another
        self.interval: dict[str, float] = {}
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
            with self.lock:
                nxt += int(self.interval.get(member, PERIOD_S) * NS)

    def resume(self, member: str) -> None:
        with self.lock:
            self.on[member] = True

    def every(self, member: str, ms: int) -> None:
        """``timing every <member> <ms>`` (scenarios conventions, text 0.3):
        the member published at another interval than its period, on."""
        with self.lock:
            self.interval[member] = ms / 1000
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
        """``timing add``, or with ``publish`` False ``timing declare``, which
        declares the member token alone (text 0.3, F-276)."""
        at = time.monotonic_ns()
        self._ask(f"timing {'add' if publish else 'declare'} {axis}", "add" if publish else "declare")
        return at

    def every(self, member: str, ms: int) -> None:
        self._ask(f"timing every {member} {ms}", "every")

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
        # the owner declares x's member token alone: the reference's
        # ``timing declare x`` since text 0.3 (F-276)
        sent = o.add("x", publish=False)
        time.sleep(0.7)
        m_lo, m_hi = c3.seen_window(kx, sent)
        missed = c3.reports(kx, "missed")
        m_c = None if len(missed) != 1 or missed[0].due is None else missed[0].due - 500 * MS
        window_ok = m_c is not None and m_lo is not None and m_lo <= m_c and (m_hi is None or m_c <= m_hi) \
            and (m_hi is not None or m_c <= m_lo + 50 * MS)
        report.check(sec, "Expected 5: C3 (axis = x, known) reports nothing in its first second, its owner present; "
                          "then missed for axis/x, due at M + 500 ms (M its first sight of x's member token), "
                          "within the bound",
                     quiet and len(missed) == 1 and window_ok and not c3.arrivals()
                     and missed[0].c - missed[0].due <= (BOUND_MS + RUNNER_MS) * MS,
                     f"first second quiet {quiet}; missed {[(_ms(r.due - c3.to_c(sent)), _ms(r.c - r.due), r.owner) for r in missed]} "
                     f"(due after the declaration, reported after the due); M within "
                     f"[{_ms(None if m_lo is None else m_lo - c3.to_c(sent))}, "
                     f"{_ms(None if m_hi is None else m_hi - c3.to_c(sent))}] of the declaration")
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
            # text 0.3 (F-273): "The owner's session has no HLC, so the
            # runner's offset moves its stamps alone"; the reference's owner
            # runs with --no-hlc, zk2py's with its session's HLC off
            o = bus.owner(oi, hlc=False)
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
                # text 0.3 (F-276): the reference's C prints an expired
                # arrival's wall_ns, the wall clock read at the arrival
                walls = all(a.wall_ns > 0 for a in arr)
                report.check(sec, "Expected 2 (700 ms behind): every sample expired, none handed on; missed for cmd, "
                                  "due 500 ms after C first saw its owner present; no met; each arrival's wall clock "
                                  "recorded, an expired one's too",
                             len(arr) >= 18 and ours and set(verdicts) == {"expired"} and handed == 0 and due_ok
                             and not c.reports(CMD, "met") and walls,
                             detail + f"; wall clock recorded on every arrival {walls}"
                                      f"; P within [{_ms(None if p_hi is None else p_lo - spawned)}, "
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


# -- §6, both tools beside each owner -------------------------------------------------------------

class ToolRun:
    """One tool's window as a process: zk2py's (``python -m
    zk2py.timing_tool``) or the reference's (``zenctl timing``, a black box),
    each ``<selector> --for 5 --clock-bound 100``; standard error's ``timing:
    listening`` line is the window's start, its arrival recorded on the
    runner's clock; standard output is one JSON document."""

    def __init__(self, tool: str, endpoint: str, window_s: float = 5.0, extra: list[str] | None = None) -> None:
        sel = "zk2/lab/pace/**"
        if tool == "zenctl":
            args = [str(BLACKBOX / "zenctl"), "timing", sel, "--for", str(window_s), "--clock-bound", "100",
                    "--format", "json", "-c", endpoint, *(extra or [])]
            env = None
        else:
            args = [sys.executable, "-m", "zk2py.timing_tool", sel, "--for", str(window_s), "--clock-bound", "100",
                    "-c", endpoint, *(extra or [])]
            env = dict(os.environ, PYTHONPATH=str(REPO / "impl" / "python"))
        self.tool = tool
        self.p = subprocess.Popen(args, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, env=env)
        self.err: list[tuple[int, str]] = []
        self.out: list[str] = []
        self.lock = threading.Lock()
        self.threads = [threading.Thread(target=self._pump, args=(self.p.stderr, True), daemon=True),
                        threading.Thread(target=self._pump, args=(self.p.stdout, False), daemon=True)]
        for th in self.threads:
            th.start()

    def _pump(self, f: Any, err: bool) -> None:
        for line in f:
            with self.lock:
                if err:
                    self.err.append((time.monotonic_ns(), line.rstrip("\n")))
                else:
                    self.out.append(line)

    def listening(self) -> int | None:
        with self.lock:
            return next((t for t, x in self.err if x.startswith("timing: listening")), None)

    def result(self, timeout_s: float = 30.0) -> tuple[int | None, dict[str, Any] | None]:
        try:
            code = self.p.wait(timeout=timeout_s)
        except subprocess.TimeoutExpired:
            self.p.kill()
            code = None
        for th in self.threads:
            th.join(2.0)
        with self.lock:
            text = "".join(self.out)
        try:
            return code, json.loads(text)
        except ValueError:
            return code, None


def tool_answers(doc: dict[str, Any] | None) -> dict[tuple[str, str], dict[str, Any]]:
    """A tool's answers as tokens: per member, per resource and over the run,
    each question's (answer, reason); a member's misses' owner states and its
    end's reason beside them."""
    out: dict[tuple[str, str], dict[str, Any]] = {}
    if doc is None:
        return out
    for r in doc.get("rows") or []:
        if r.get("row") == "member":
            out[("member", r["key"])] = {q: (r[q]["answer"], r[q]["reason"]) for q in ("deadline", "period", "lifespan")
                                         if q in r}
            # §2.7 (text 0.3): a tool's known members, beside the answers
            out[("member", r["key"])]["known"] = bool(r.get("known"))
        elif r.get("row") == "resource":
            # a roll-up over no member carries no reason in zenctl's output
            out[("resource", r["resource"])] = {q: (v["answer"], v.get("reason"))
                                                for q, v in r["questions"].items()}
    if doc.get("run"):
        out[("run", "")] = {q: (v["answer"], v.get("reason")) for q, v in doc["run"].items()}
    return out


def rollup_mismatches(doc: dict[str, Any] | None) -> list[tuple[str, str, str, str]]:
    """Each resource row's and the run's answer against §5's roll-up (text
    0.4, :func:`zk2py.timing.rollup`) over the tool's own member rows, the
    answer alone (``conclusions.json``'s roll-up cases give no reason):
    ``(row, question, printed, by the rule)`` for each that differs, and
    for ``stream/axis/{axis}``'s lifespan, which no member declares, any
    answer but ``not_asked`` (F-277's case)."""
    out: list[tuple[str, str, str, str]] = []
    if not doc:
        return [("document", "", "none", "")]
    members = [r for r in doc.get("rows") or [] if r.get("row") == "member"]

    def rolled(q: str, rows: list[dict[str, Any]]) -> str:
        return tm.rollup(q, [tm.Answer(r[q]["answer"], r[q].get("reason", "")) for r in rows if q in r]).answer

    for res in (r for r in doc.get("rows") or [] if r.get("row") == "resource"):
        mine = [m for m in members if m.get("resource") == res.get("resource")
                and m.get("address") == res.get("address") and m.get("iface") == res.get("iface")]
        for q, v in (res.get("questions") or {}).items():
            if v["answer"] != (want := rolled(q, mine)):
                out.append((res["resource"], q, v["answer"], want))
        if res.get("resource") == RID_AXIS and mine and (res.get("questions") or {}).get("lifespan", {}).get(
                "answer") != "not_asked":
            out.append((res["resource"], "lifespan", str(res["questions"].get("lifespan")), "not_asked"))
    for q, v in (doc.get("run") or {}).items():
        if v["answer"] != (want := rolled(q, members)):
            out.append(("run", q, v["answer"], want))
    return out


def _member_row(doc: dict[str, Any] | None, key: str) -> dict[str, Any]:
    return next((r for r in (doc or {}).get("rows") or [] if r.get("row") == "member" and r.get("key") == key), {})


def _misses(row: dict[str, Any]) -> list[str]:
    """The owner's state at each miss a tool reported of a member."""
    return [m.get("owner") for m in row.get("misses") or []]


def _ended(row: dict[str, Any]) -> str | None:
    e = row.get("ended")
    return e.get("reason") if isinstance(e, dict) else None


def section6(report: Report, oi: str) -> None:
    """§6 with both tools, each a process, their windows run at once over the
    same owner: each tool's answers checked against Expected, then the two
    compared token by token (a member's answers, each resource's and the
    run's, the exit). A step's owner commands are timed from the later of the
    two windows' starts, and ``cmd`` runs past both windows' ends."""
    sec = f"§6 [{oi}→both tools]"
    bus = Bus()
    try:
        o = bus.owner(oi)
        kx = f"{AXIS}/x"
        o.add("x")
        time.sleep(0.3)
        results: list[tuple[str, dict[str, Any]]] = []

        def run(step: str, plan: Callable[[int], None]) -> dict[str, tuple[int | None, dict[str, Any] | None]]:
            runs = {t: ToolRun(t, bus.endpoint) for t in ("zk2py", "zenctl")}
            ok = _wait(lambda: all(r.listening() is not None or r.p.poll() is not None for r in runs.values()), 15.0)
            starts = [r.listening() for r in runs.values()]
            if not ok or any(t is None for t in starts):
                for r in runs.values():
                    r.p.kill()
                report.check(sec, f"step {step}: both tools' windows start", False,
                             "; ".join(f"{t}: {r.err[-3:]}" for t, r in runs.items()))
                return {t: (None, None) for t in runs}
            later = max(t for t in starts if t is not None)
            plan(later)
            got = {t: r.result() for t, r in runs.items()}
            report.note(sec, f"step {step}: the windows' starts {abs(starts[0] - starts[1]) / MS:.1f} ms apart "
                             f"(zk2py's {'first' if starts[0] <= starts[1] else 'second'})")
            return got

        def steady(later: int) -> None:
            o.resume("cmd")
            _sleep_until(later + 5 * NS + 300 * MS)
            o.pause("cmd")

        def paused(later: int) -> None:
            o.resume("cmd")
            _sleep_until(later + 2 * NS)
            o.pause("cmd")
            _sleep_until(later + 3 * NS)
            o.resume("cmd")
            _sleep_until(later + 5 * NS + 300 * MS)
            o.pause("cmd")

        def slow(later: int) -> None:
            # the reference's ``timing every`` sets the interval and leaves the
            # member paused or not as it was: resumed after it
            o.every("cmd", 250)
            o.resume("cmd")
            _sleep_until(later + 5 * NS + 300 * MS)
            o.pause("cmd")
            o.every("cmd", 100)
            o.pause("cmd")

        def closing(later: int) -> None:
            o.resume("cmd")
            _sleep_until(later + 1 * NS)
            o.close()

        def pairs(doc: dict[str, Any] | None, key: str) -> dict[str, tuple[str, str]]:
            return {q: v for q, v in tool_answers(doc).get(("member", key), {}).items() if q != "known"}

        def known(doc: dict[str, Any] | None, key: str) -> Any:
            return tool_answers(doc).get(("member", key), {}).get("known")

        steps = [("1", steady), ("2", paused), ("3", slow), ("4", closing)]
        for step, plan in steps:
            got = run(step, plan)
            for tool, (code, doc) in got.items():
                tsec = f"§6 [{oi}→{tool}]"
                c, x = pairs(doc, CMD), pairs(doc, kx)
                if step == "1":
                    report.check(tsec, "Expected 1: cmd met, kept, live; axis/x met, kept, no_lifespan",
                                 c == {"deadline": ("yes", "met"), "period": ("yes", "kept"),
                                       "lifespan": ("no", "live")}
                                 and x == {"deadline": ("yes", "met"), "period": ("yes", "kept"),
                                           "lifespan": ("not_asked", "no_lifespan")} and code == 0,
                                 f"exit {code}; cmd {c}; axis/x {x}")
                    # §2.7 (text 0.3): "a member is known to it only when its
                    # template has no parameter"
                    report.check(tsec, "§2.7: cmd known, axis/x not (a tool holds no binding)",
                                 known(doc, CMD) is True and known(doc, kx) is False,
                                 f"cmd {known(doc, CMD)}, axis/x {known(doc, kx)}")
                elif step == "2":
                    owners = _misses(_member_row(doc, CMD))
                    run_ = tool_answers(doc).get(("run", ""), {})
                    report.check(tsec, "Expected 2: cmd missed (the owner present when T reported it), not_kept, "
                                       "live; the run's verdict the finding",
                                 c == {"deadline": ("no", "missed"), "period": ("no", "not_kept"),
                                       "lifespan": ("no", "live")}
                                 and bool(owners) and all(w == "present" for w in owners)
                                 and run_.get("deadline", ("",))[0] == "no" and code == 1,
                                 f"exit {code}; cmd {c}; misses' owner {owners}; "
                                 f"{_member_row(doc, CMD).get('arrivals')} cmd arrivals; run {run_}")
                elif step == "3":
                    report.check(tsec, "Expected 3: cmd every 250 ms: deadline met, period not_kept",
                                 c.get("deadline") == ("yes", "met") and c.get("period") == ("no", "not_kept"),
                                 f"exit {code}; cmd {c}; {_member_row(doc, CMD).get('arrivals')} cmd arrivals")
                else:
                    report.check(tsec, "Expected 4: the owner closed cleanly 1 s in: cmd owner_not_present twice, "
                                       "lifespan no; axis/x ended (gone) at the read, deadline not asked (ended), "
                                       "period kept",
                                 c == {"deadline": ("unobservable", "owner_not_present"),
                                       "period": ("unobservable", "owner_not_present"),
                                       "lifespan": ("no", "live")}
                                 and _ended(_member_row(doc, kx)) == "gone"
                                 and x.get("deadline") == ("not_asked", "ended") and x.get("period") == ("yes", "kept"),
                                 f"exit {code}; cmd {c}; axis/x {x}, ended {_ended(_member_row(doc, kx))}")
            # the two tools, token by token
            (ca, da), (cb, db) = got["zk2py"], got["zenctl"]
            ta, tb = tool_answers(da), tool_answers(db)
            diffs = []
            for k in sorted(set(ta) | set(tb)):
                for q in ("deadline", "period", "lifespan", "known"):
                    va, vb = ta.get(k, {}).get(q), tb.get(k, {}).get(q)
                    if va != vb:
                        diffs.append((k, q, va, vb))
            results.append((step, {"diffs": diffs, "exits": (ca, cb)}))
            report.check(sec, f"step {step}: the two tools' answers agree, token by token, members, resources and "
                              f"the run",
                         not diffs and ca == cb,
                         f"exits zk2py {ca}, zenctl {cb}; differences {diffs or 'none'}")
            bad = {t: rollup_mismatches(d) for t, (_, d) in got.items()}
            report.check(sec, f"step {step}: {ROLLUP}", not any(bad.values()),
                         "; ".join(f"{t} {v or 'none'}" for t, v in bad.items()))
            if step != "4":
                time.sleep(0.5)
    finally:
        bus.close()


def section6_face(report: Report, oi: str) -> None:
    """Beside §6, not a scenario: both tools told the owner sits across a
    constrained face (§2.8), on the runner's bus, where the deployment's word
    is all they read: the contract offline, no presence and no descriptor,
    and ``meta.zid`` only as ``--owner-zid`` states it (text 0.3). Read with
    the owner's zid, then with none."""
    sec = f"§2.8 beside §6 [{oi}→both tools]"
    bus = Bus()
    try:
        o = bus.owner(oi)
        kx = f"{AXIS}/x"
        o.add("x")
        o.resume("cmd")
        time.sleep(0.3)
        face = ["--across-face", "--contracts", str(PACE)]
        for name, extra, life in ((FACE_ZID, ["--owner-zid", o.zid], ("no", "live")),
                                  (FACE_NONE, [], ("unobservable", "unattributed"))):
            runs = {t: ToolRun(t, bus.endpoint, 2.0, face + extra) for t in ("zk2py", "zenctl")}
            got = {t: r.result() for t, r in runs.items()}
            ans = {t: tool_answers(d) for t, (_, d) in got.items()}
            for t, a in ans.items():
                c, x = a.get(("member", CMD), {}), a.get(("member", kx), {})
                report.check(sec, f"{name} ({t}): cmd's deadline and period unobservable (across_face), its "
                                  f"lifespan {life[0]} ({life[1]}); axis/x across_face, no lifespan",
                             c.get("deadline") == ("unobservable", "across_face")
                             and c.get("period") == ("unobservable", "across_face") and c.get("lifespan") == life
                             and x.get("deadline") == ("unobservable", "across_face")
                             and x.get("lifespan") == ("not_asked", "no_lifespan"),
                             f"exit {got[t][0]}; cmd {c}; axis/x {x}"
                             + ("" if got[t][1] else f"; stderr {runs[t].err[-3:]}"))
            ta, tb = ans["zk2py"], ans["zenctl"]
            diffs = [(k, q, ta.get(k, {}).get(q), tb.get(k, {}).get(q)) for k in sorted(set(ta) | set(tb))
                     for q in ("deadline", "period", "lifespan", "known")
                     if ta.get(k, {}).get(q) != tb.get(k, {}).get(q)]
            report.check(sec, f"{name}: the two tools' answers agree, token by token, and their exits",
                         not diffs and got["zk2py"][0] == got["zenctl"][0],
                         f"exits zk2py {got['zk2py'][0]}, zenctl {got['zenctl'][0]}; differences {diffs or 'none'}")
            bad = {t: rollup_mismatches(d) for t, (_, d) in got.items()}
            report.check(sec, f"{name}: {ROLLUP}", not any(bad.values()),
                         "; ".join(f"{t} {v or 'none'}" for t, v in bad.items()))
        o.pause("cmd")
    finally:
        bus.close()


def section6_gone(report: Report, oi: str) -> None:
    """Beside §6, not a scenario (round 80, F-278): a known member never
    heard, ``cmd`` never published, its owner present at the window's start
    and closing cleanly 0.15 s into a 3 s window, before ``cmd``'s miss is
    due (500 ms). Its wait stops at the read that shows the owner absent
    (§2.5: "waits for nothing until the owner is present again"), so no miss
    is reported. §2.7 counts how long a tool held a known member "from …
    the instant it began to wait for it … to the window's end, or to the
    member's end", and a known member is never forgotten (§2.5): zk2py
    holds it to the window's end, 3 s, and answers its deadline yes, met
    (§5); the reference counts only while it waited, about 0.3 s, and
    answers unobservable, window_too_short. Both read its period
    unobservable (owner_not_present) and its lifespan unobservable
    (no_sample)."""
    sec = f"§2.7 beside §6 [{oi}→both tools]"
    bus = Bus()
    try:
        o = bus.owner(oi)
        time.sleep(0.3)
        runs = {t: ToolRun(t, bus.endpoint, 3.0) for t in ("zk2py", "zenctl")}
        ok = _wait(lambda: all(r.listening() is not None or r.p.poll() is not None for r in runs.values()), 15.0)
        starts = [r.listening() for r in runs.values()]
        if not ok or any(t is None for t in starts):
            for r in runs.values():
                r.p.kill()
            report.check(sec, "both tools' windows start", False,
                         "; ".join(f"{t}: {r.err[-3:]}" for t, r in runs.items()))
            return
        later = max(t for t in starts if t is not None)
        _sleep_until(later + 150 * MS)
        o.close()
        got = {t: r.result() for t, r in runs.items()}
        ans = {t: tool_answers(d) for t, (_, d) in got.items()}
        for t, a in ans.items():
            c, row = a.get(("member", CMD), {}), _member_row(got[t][1], CMD)
            report.check(sec, f"{t}: cmd known, no miss and no arrival; its period unobservable "
                              f"(owner_not_present), its lifespan unobservable (no_sample)",
                         c.get("known") is True and not row.get("misses") and row.get("arrivals") == 0
                         and c.get("period") == ("unobservable", "owner_not_present")
                         and c.get("lifespan") == ("unobservable", "no_sample"),
                         f"exit {got[t][0]}; cmd {c}; held_ms {row.get('held_ms')}")
            report.note(sec, f"{t}: cmd's deadline {c.get('deadline')}, held {row.get('held_ms')} ms of 3,000")
        ta, tb = ans["zk2py"], ans["zenctl"]
        diffs = [(k, q, ta.get(k, {}).get(q), tb.get(k, {}).get(q)) for k in sorted(set(ta) | set(tb))
                 for q in ("deadline", "period", "lifespan", "known")
                 if ta.get(k, {}).get(q) != tb.get(k, {}).get(q)]
        # a roll-up over no member (axis, none heard): its answer alike,
        # its reason zk2py's ``no_member`` and none in zenctl's (F-279)
        empty = [d for d in diffs if d[0][0] in ("resource", "run") and d[2] is not None and d[3] is not None
                 and d[2][0] == d[3][0] == "unobservable" and d[2][1] == "no_member" and d[3][1] is None]
        rest = [d for d in diffs if d not in empty]
        report.check(sec, GONE_AGREE, not rest and got["zk2py"][0] == got["zenctl"][0],
                     f"exits zk2py {got['zk2py'][0]}, zenctl {got['zenctl'][0]}; differences {rest or 'none'}")
        report.check(sec, GONE_EMPTY, not empty, f"{empty or 'none'}")
    finally:
        bus.close()


def section6_again(report: Report, oi: str) -> None:
    """Beside §6, not a scenario (round 80, F-280): ``axis/x`` ends in the
    window and is heard again. Its member token is withdrawn 1 s into a 5 s
    window (``timing remove x``), so a tool ends it (gone) at its read, and
    added again 2 s in (``timing add x``), so it is heard again, "judged
    from that arrival, as a member first heard" (§2.5). §2.7's period span
    runs "to the member's end when it ended in the window", and the text
    reads no second life: both tools read the first end, its period kept
    over its first life and its second life judged by neither; its deadline
    not asked (``ended``). zk2py's round-80 first reading, an end followed
    by an arrival dropped, counted the second's absence against the period:
    no, not_kept, exit 1, beside zenctl's exit 0."""
    sec = f"§2.7 again beside §6 [{oi}→both tools]"
    bus = Bus()
    try:
        o = bus.owner(oi)
        kx = f"{AXIS}/x"
        o.add("x")
        o.resume("cmd")
        time.sleep(0.3)
        runs = {t: ToolRun(t, bus.endpoint, 5.0) for t in ("zk2py", "zenctl")}
        ok = _wait(lambda: all(r.listening() is not None or r.p.poll() is not None for r in runs.values()), 15.0)
        starts = [r.listening() for r in runs.values()]
        if not ok or any(t is None for t in starts):
            for r in runs.values():
                r.p.kill()
            report.check(sec, "both tools' windows start", False,
                         "; ".join(f"{t}: {r.err[-3:]}" for t, r in runs.items()))
            return
        later = max(t for t in starts if t is not None)
        _sleep_until(later + 1 * NS)
        o.remove("x")
        _sleep_until(later + 2 * NS)
        o.add("x")
        _sleep_until(later + 5 * NS + 300 * MS)
        o.pause("cmd")
        got = {t: r.result() for t, r in runs.items()}
        ans = {t: tool_answers(d) for t, (_, d) in got.items()}
        for t, a in ans.items():
            x, row = a.get(("member", kx), {}), _member_row(got[t][1], kx)
            report.check(sec, f"{t}: axis/x ended (gone) about 1.1 s in and heard again: no miss, its deadline not "
                              f"asked (ended), its period kept over its first life; the run clean, exit 0",
                         _ended(row) == "gone" and not row.get("misses")
                         and x.get("deadline") == ("not_asked", "ended") and x.get("period") == ("yes", "kept")
                         and got[t][0] == 0,
                         f"exit {got[t][0]}; axis/x {x}; ended {row.get('ended')}; held_ms {row.get('held_ms')}; "
                         f"{row.get('arrivals')} arrivals")
        ta, tb = ans["zk2py"], ans["zenctl"]
        diffs = [(k, q, ta.get(k, {}).get(q), tb.get(k, {}).get(q)) for k in sorted(set(ta) | set(tb))
                 for q in ("deadline", "period", "lifespan", "known")
                 if ta.get(k, {}).get(q) != tb.get(k, {}).get(q)]
        report.check(sec, "the two tools' answers agree, token by token, and their exits",
                     not diffs and got["zk2py"][0] == got["zenctl"][0],
                     f"exits zk2py {got['zk2py'][0]}, zenctl {got['zenctl'][0]}; differences {diffs or 'none'}")
        report.note(sec, "axis/x held " + ", ".join(f"{t} {_member_row(got[t][1], kx).get('held_ms')} ms"
                                                    for t in got) + " (not compared: §2.7 reads no second life)")
    finally:
        bus.close()


SECTIONS: dict[str, Callable[..., None]] = {"1": section1, "2": section2, "3": section3, "4": section4,
                                            "5": section5}
PAIRS = ["py:py", "rs:py", "py:rs", "rs:rs"]


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="python -m zk2py.timing_interop", description=__doc__.split("\n")[0])
    ap.add_argument("--pair", action="append", choices=PAIRS, help="owner:reader, each py or rs")
    ap.add_argument("--only", action="append", choices=[*SECTIONS, "6", "face", "gone", "again"],
                    help="run only these sections (face: §2.8 beside §6, both tools; gone: §2.7 beside §6, a "
                         "known member's owner gone before its miss is due, F-278; again: §2.7 beside §6, a "
                         "member ended and heard again, F-280)")
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
    six = not args.only or "6" in args.only
    if six and not os.access(BLACKBOX / "zenctl", os.X_OK):
        print(f"SKIP §6: it runs both tools, and the reference's is missing: {BLACKBOX / 'zenctl'}", flush=True)
        six = False
    face = (not args.only or "face" in args.only) and os.access(BLACKBOX / "zenctl", os.X_OK)
    gone = (not args.only or "gone" in args.only) and os.access(BLACKBOX / "zenctl", os.X_OK)
    again = (not args.only or "again" in args.only) and os.access(BLACKBOX / "zenctl", os.X_OK)
    for fn, on in ((section6, six), (section6_face, face), (section6_gone, gone), (section6_again, again)):
        if not on:
            continue
        for oi in sorted({p.split(':')[0] for p in pairs}):
            try:
                fn(report, oi)
            except Exception as e:  # noqa: BLE001
                import traceback

                traceback.print_exc()
                report.check(f"§6 [{oi}]", "the section ran to its end", False, f"{type(e).__name__}: {e}")
    from .checks import summarise

    return summarise("timing interop", report.rows,
                     f"the largest lateness past the 10 ms bound {max((x for _, x in report.late), default=0.0):.3f} ms")



if __name__ == "__main__":
    code = main()
    sys.stdout.flush()
    os._exit(code)

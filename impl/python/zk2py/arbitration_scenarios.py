"""The ``arbitration.v1`` scenarios (``spec/profiles/arbitration/scenarios.md``)
on an in-process zenoh-python router, zk2py only: ``python -m
zk2py.arbitration_scenarios [--only 1 … 10|7c]``.

- **The bus** is the text's: R1, a router on a free loopback port; every
  party a client of it. §7's watch links go through a TCP relay the test
  cuts and heals (:class:`zk2py.budget_scenarios.Link`), one per actuator.
- **The contracts** are ``steer.v1`` and ``rudder.v1``, copied from the text
  into ``impl/python/interop/arbitration/``, their fingerprints the
  reference CLI's; §8's and §10's copies are written to a temporary
  directory at run time.
- **The commanders** ``C_s``, ``C_t`` and ``C_a`` are
  ``python -m zk2py.arbitration_owner commander``, each a process of its own,
  zk2py's owner with its HLC on, putting ``cmd`` every 50 ms as ``<service>
  <n>``. §4's ``X`` is a raw session of the runner's, its tokens declared by
  hand.
- **The actuators** ``A_l`` and ``A_r`` are ``python -m
  zk2py.arbitration_owner actuator``, processes too, each with three
  sessions (its service's, a watch, a reader). Each prints one JSON line per
  switch; asked, its counts, its reads again and its round trips. Its
  standard error gives its start on the host's monotonic clock, which every
  process shares, so its ``at_ms`` is placed on the runner's.
- **S** subscribes to ``zk2/lab/**`` and follows ``zk2/lab/*/@zk/**`` on a
  watch session, recording each delivery's arrival, key and payload.
- **Margins** are the text's: a switch's ``at_ms − last_ms`` against the
  detection bound with 50 ms of scheduling beyond it, the largest lateness
  recorded; an instant the test takes itself, 200 ms.

Exit 0 when every check passes, 1 when any fails.
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path
from typing import Any, Callable

REPO = Path(__file__).resolve().parents[3]
DIR = REPO / "impl" / "python" / "interop" / "arbitration"
STEER, RUDDER = DIR / "steer.v1.toml", DIR / "rudder.v1.toml"
NS, MS = 1_000_000_000, 1_000_000
DEADLINE_MS, BOUND_MS, MARGIN_MS = 300, 306, 50
EXACT = ["lab/safety", "lab/teleop", "lab/autopilot"]


class Report:
    def __init__(self) -> None:
        self.rows: list[tuple[str, str, bool, str]] = []
        self.late_ms: list[float] = []

    def check(self, section: str, name: str, ok: bool, detail: str = "") -> bool:
        self.rows.append((section, name, ok, detail))
        print(f"{'PASS' if ok else 'FAIL'} [{section}] {name}" + (f": {detail}" if detail else ""), flush=True)
        return ok

    def note(self, section: str, text: str) -> None:
        print(f"NOTE [{section}] {text}", flush=True)


def _wait(cond: Callable[[], bool], timeout_s: float, step_s: float = 0.01) -> bool:
    end = time.monotonic() + timeout_s
    while time.monotonic() < end:
        if cond():
            return True
        time.sleep(step_s)
    return cond()


class Proc:
    """A party in a process of its own: its standard output's lines kept
    with their arrival (the runner's monotonic ns), its standard error's
    too."""

    def __init__(self, args: list[str]) -> None:
        env = dict(os.environ, PYTHONPATH=str(REPO / "impl" / "python"))
        self.p = subprocess.Popen([sys.executable, "-u", "-m", "zk2py.arbitration_owner", *args],
                                  stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                  text=True, env=env, bufsize=1)
        self.out: list[tuple[int, str]] = []
        self.err: list[str] = []
        self.lock = threading.Lock()
        threading.Thread(target=self._pump, args=(self.p.stdout, self.out, True), daemon=True).start()
        threading.Thread(target=self._pump, args=(self.p.stderr, self.err, False), daemon=True).start()

    def _pump(self, f: Any, into: list, stamped: bool) -> None:
        for line in f:
            with self.lock:
                into.append((time.monotonic_ns(), line.rstrip("\n")) if stamped else line.rstrip("\n"))

    def send(self, cmd: str) -> None:
        assert self.p.stdin is not None
        self.p.stdin.write(cmd + "\n")
        self.p.stdin.flush()

    def lines(self) -> list[str]:
        with self.lock:
            return [x for _, x in self.out]

    def stop(self) -> None:
        try:
            if self.p.stdin is not None:
                self.p.stdin.close()
        except OSError:
            pass
        try:
            self.p.wait(timeout=5.0)
        except subprocess.TimeoutExpired:
            self.p.kill()

    def kill(self) -> None:
        self.p.kill()
        self.p.wait(timeout=5.0)


class Commander(Proc):
    def __init__(self, endpoint: str, service: str, contract: Path = STEER) -> None:
        super().__init__(["commander", f"lab/{service}", str(contract), "--connect", endpoint])
        self.service = service
        self.ready = _wait(lambda: any(x.startswith("ready ") for x in self.lines()), 10.0)

    def ask(self, cmd: str, prefix: str) -> str | None:
        n = len(self.lines())
        self.send(cmd)
        _wait(lambda: any(x.startswith(prefix) for x in self.lines()[n:]), 5.0)
        return next((x for x in self.lines()[n:] if x.startswith(prefix)), None)

    def hold(self) -> None:
        self.ask("hold", "ok hold")

    def resume(self) -> None:
        self.ask("resume", "ok resume")


class Actuator(Proc):
    def __init__(self, endpoint: str, service: str, bind: list[str], *, contract: Path = RUDDER,
                 held: Path = STEER, watch: str | None = None, wait: bool = True, infer: bool = True) -> None:
        args = ["actuator", f"lab/{service}", str(contract), str(held), "--connect", endpoint,
                "--bind", ",".join(bind)]
        if not infer:
            args.append("--no-infer-moves")
        if watch is not None:
            args += ["--watch-connect", watch]
        super().__init__(args)
        self.service = service
        self.t0: int | None = None
        if wait:
            _wait(lambda: self._ready() is not None or self.p.poll() is not None, 15.0)

    def _ready(self) -> str | None:
        with self.lock:
            r = next((x for x in self.err if x.startswith("ready ")), None)
        if r is not None and self.t0 is None:
            self.t0 = int(r.split(" t0=")[1].split()[0])
        return r

    @property
    def instance_key(self) -> str | None:
        r = self._ready()
        return None if r is None else r.split()[1]

    def switches(self) -> list[dict[str, Any]]:
        """Every switch line, each with ``at`` on the runner's clock."""
        out = []
        with self.lock:
            rows = list(self.out)
        for _, line in rows:
            try:
                x = json.loads(line)
            except ValueError:
                continue
            if "switch" in x:
                x["at"] = None if self.t0 is None else self.t0 + int(x["at_ms"] * MS)
                out.append(x)
        return out

    def ask(self, cmd: str, key: str) -> Any:
        n = len(self.lines())
        self.send(cmd)

        def got() -> Any:
            for line in self.lines()[n:]:
                try:
                    x = json.loads(line)
                except ValueError:
                    continue
                if key in x:
                    return x
            return None

        _wait(lambda: got() is not None, 5.0)
        return got()


def sw(x: dict[str, Any]) -> tuple[Any, Any, str]:
    s = x["switch"]
    return s["from"], s["to"], s["cause"]


def gap(x: dict[str, Any]) -> float:
    return x["at_ms"] - x["last_ms"]


class Bus:
    def __init__(self) -> None:
        import zenoh

        from . import live
        from .contract import load_contract
        from .live_interop import _r1

        self.router, self.endpoint, self.r_zid = _r1()
        self.steer = load_contract(STEER)
        self.rudder = load_contract(RUDDER)
        self.procs: list[Proc] = []
        self.links: list[Any] = []
        self.extra: list[Any] = []
        self.s = live.open_client(self.endpoint)
        self.s_watch = live.open_client(self.endpoint)
        self.seen: list[tuple[int, str, str]] = []
        self.tokens: list[tuple[int, str, str]] = []
        self.lock = threading.Lock()

        def data(sample: Any) -> None:
            with self.lock:
                self.seen.append((time.monotonic_ns(), str(sample.key_expr), sample.payload.to_bytes().decode()))

        def tok(sample: Any) -> None:
            kind = "delete" if sample.kind == zenoh.SampleKind.DELETE else "put"
            with self.lock:
                self.tokens.append((time.monotonic_ns(), str(sample.key_expr), kind))

        self.extra.append(self.s.declare_subscriber("zk2/lab/**", zenoh.handlers.Callback(data)))
        self.extra.append(live.declare_liveliness_subscriber(self.s_watch, "zk2/lab/*/@zk/**", tok))
        self.tmp = tempfile.mkdtemp(prefix="zk2py-arbitration-")

    def commander(self, service: str) -> Commander:
        c = Commander(self.endpoint, service)
        self.procs.append(c)
        return c

    def actuator(self, service: str, bind: list[str] | None = None, **kw: Any) -> Actuator:
        a = Actuator(self.endpoint, service, EXACT if bind is None else bind, **kw)
        self.procs.append(a)
        return a

    def angle(self, a: Actuator, since: int = 0, until: int | None = None) -> list[tuple[int, str]]:
        key = f"zk2/lab/{a.service}/rudder.v1/state/angle"
        with self.lock:
            return [(t, p) for t, k, p in self.seen if k == key and t >= since and (until is None or t <= until)
                    and " " in p]

    def cmd_seen(self, service: str, since: int = 0) -> list[tuple[int, str]]:
        key = f"zk2/lab/{service}/steer.v1/stream/cmd"
        with self.lock:
            return [(t, p) for t, k, p in self.seen if k == key and t >= since]

    def copy(self, src: Path, name: str, old: str, new: str) -> Path:
        d = Path(tempfile.mkdtemp(prefix=f"{name}-", dir=self.tmp))
        text = src.read_text()
        assert old in text, (old, src)
        p = d / src.name
        p.write_text(text.replace(old, new, 1))
        return p

    def close(self) -> None:
        from . import live

        for p in self.procs:
            try:
                p.stop() if p.p.poll() is None else None
            except Exception:  # noqa: BLE001 - closing anyway
                try:
                    p.kill()
                except Exception:  # noqa: BLE001
                    pass
        for x in self.extra:
            try:
                x.undeclare()
            except Exception:  # noqa: BLE001
                pass
        for s in (self.s, self.s_watch):
            live.close(s)
        for link in self.links:
            link.close()
        self.router.close()


def _following(bus: Bus, a: Actuator, provider: str) -> bool:
    sws = a.switches()
    return bool(sws) and sws[-1]["switch"]["to"] == provider


def _setup_all(bus: Bus, report: Report, sec: str, watch_links: bool = False,
               infer: bool = True) -> tuple[dict[str, Commander], list[Actuator]]:
    """§2's setup: all three commanders run, and each actuator follows
    safety."""
    cs = {s: bus.commander(s) for s in ("autopilot", "teleop", "safety")}
    acts = []
    for svc in ("rudder-l", "rudder-r"):
        watch = None
        if watch_links:
            from .budget_scenarios import Link

            link = Link(bus.endpoint)
            bus.links.append(link)
            watch = link.endpoint
        a = bus.actuator(svc, watch=watch, infer=infer)
        a.link = link if watch_links else None  # type: ignore[attr-defined]
        acts.append(a)
    ok = all(c.ready for c in cs.values()) and all(_wait(lambda a=a: _following(bus, a, "lab/safety"), 5.0)
                                                   for a in acts)
    report.check(sec, "setup: the three commanders run, and each actuator follows safety", ok,
                 "; ".join(f"{a.service}: {[sw(x) for x in a.switches()]}" for a in acts))
    time.sleep(0.5)
    return cs, acts


def _deadline_ok(x: dict[str, Any], report: Report) -> bool:
    g = gap(x)
    report.late_ms.append(max(0.0, g - BOUND_MS))
    return DEADLINE_MS < g <= BOUND_MS + MARGIN_MS


# -- §1 ---------------------------------------------------------------------------------------------

def section1(report: Report) -> None:
    sec = "§1 the dead-man at start, and each higher priority taking over"
    bus = Bus()
    try:
        ca = bus.commander("autopilot")
        time.sleep(0.5)
        acts = [bus.actuator("rudder-l"), bus.actuator("rudder-r")]
        t1 = time.monotonic_ns()
        time.sleep(2.0)
        for a in acts:
            sws = a.switches()
            first = sws[:2]
            follow = bus.angle(a, sws[1]["at"] if len(sws) > 1 else 0)
            report.check(sec, f"Expected 1 ({a.service}): its first line the start switch, none to none; within 1 s "
                              f"its next to autopilot, cause sample; S sees it follow autopilot",
                         len(first) == 2 and sw(first[0]) == (None, None, "start") and first[0]["at_ms"] == 0
                         and sw(first[1]) == (None, "lab/autopilot", "sample") and first[1]["at_ms"] <= 1000
                         and bool(follow) and all(p.startswith("autopilot ") for _, p in follow),
                         f"{[(sw(x), x['at_ms']) for x in sws]}; {len(follow)} angle puts, "
                         f"{sorted({p.split()[0] for _, p in follow})}")
        t2 = time.monotonic_ns()
        bus.commander("teleop")
        time.sleep(2.0)
        first_t = next((p for _, p in bus.cmd_seen("teleop", t2)), None)
        for a in acts:
            sws = a.switches()
            ang = bus.angle(a, t2)
            first_on = next((i for i, (_, p) in enumerate(ang) if p.startswith("teleop ")), None)
            after = ang[first_on:] if first_on is not None else []
            report.check(sec, f"Expected 2 ({a.service}): to teleop, cause sample, at teleop's first arrival: the "
                              f"first teleop payload on its angle the first C_t's that S saw, and no autopilot one "
                              f"after it",
                         len(sws) == 3 and sw(sws[2]) == ("lab/autopilot", "lab/teleop", "sample")
                         and first_on is not None and after[0][1] == first_t
                         and all(p.startswith("teleop ") for _, p in after),
                         f"switch {sw(sws[2]) if len(sws) > 2 else None}; first teleop on angle "
                         f"{after[0][1] if after else None}, S's first {first_t}; "
                         f"{sorted({p.split()[0] for _, p in after})} after")
        t3 = time.monotonic_ns()
        bus.commander("safety")
        time.sleep(2.0)
        first_s = next((p for _, p in bus.cmd_seen("safety", t3)), None)
        for a in acts:
            sws = a.switches()
            ang = bus.angle(a, t3)
            first_on = next((i for i, (_, p) in enumerate(ang) if p.startswith("safety ")), None)
            after = ang[first_on:] if first_on is not None else []
            report.check(sec, f"Expected 3 ({a.service}): to safety, cause sample, at its first arrival; S sees it "
                              f"follow safety",
                         len(sws) == 4 and sw(sws[3]) == ("lab/teleop", "lab/safety", "sample")
                         and first_on is not None and after[0][1] == first_s
                         and all(p.startswith("safety ") for _, p in after),
                         f"switch {sw(sws[3]) if len(sws) > 3 else None}; first safety on angle "
                         f"{after[0][1] if after else None}, S's first {first_s}")
        for a in acts:
            c = a.ask("counts", "counts")["counts"]
            ang = bus.angle(a)  # from its start: A_l delivered before A_r had started
            by = {p: sum(1 for _, x in ang if x.startswith(p.split("/")[1] + " ")) for p in EXACT}
            prov = c["providers"]
            ok = all(abs(prov.get(p, {}).get("delivered", 0) - by[p]) <= 1 for p in EXACT) \
                and prov["lab/teleop"]["not_active"] > 0 and prov["lab/autopilot"]["not_active"] > 0 \
                and c["wild"] == 0 and c["unresolved"] == 0 and c["unbound"] == 0
            report.check(sec, f"Expected 4 ({a.service}): each delivered count the payloads S saw on its angle, "
                              f"give or take one; teleop's and autopilot's not_active above 0; wild, unresolved, "
                              f"unbound 0; its own start line and three switches",
                         ok and [sw(x)[2] for x in a.switches()] == ["start", "sample", "sample", "sample"],
                         f"counts {json.dumps(c)}; S saw {by}")
        report.check(sec, "Expected 4: A_l and A_r decided apart: each printed its own lines, at its own instants",
                     all(len(a.switches()) == 4 for a in acts)
                     and acts[0].switches()[1]["at"] != acts[1].switches()[1]["at"],
                     f"{[[x['at'] for x in a.switches()] for a in acts]}")
        ca.stop()
    finally:
        bus.close()


# -- §2 ---------------------------------------------------------------------------------------------

def section2(report: Report) -> None:
    sec = "§2 a commander silent past its deadline"
    bus = Bus()
    try:
        cs, acts = _setup_all(bus, report, sec)
        n0 = {a.service: len(a.switches()) for a in acts}
        holds = []
        for _ in range(5):
            t = time.monotonic_ns()
            cs["safety"].hold()
            time.sleep(2.0)
            cs["safety"].resume()
            holds.append((t, time.monotonic_ns()))
            time.sleep(1.0)
        time.sleep(0.3)
        for a in acts:
            new = a.switches()[n0[a.service]:]
            pairs = [sw(x) for x in new]
            want = [("lab/safety", "lab/teleop", "deadline"), ("lab/teleop", "lab/safety", "sample")] * 5
            deadlines = [x for x in new if sw(x)[2] == "deadline"]
            in_hold = all(any(h0 <= x["at"] <= h1 for h0, h1 in holds) for x in deadlines)
            outside = [p for t, p in bus.angle(a, holds[0][0])
                       if p.startswith("teleop ") and not any(h0 <= t <= h1 + 200 * MS for h0, h1 in holds)]
            report.check(sec, f"Expected 1–2 ({a.service}): five times, to teleop cause deadline with at_ms − "
                              f"last_ms above 300 and at most 306 (50 ms margin), inside the hold, then back to "
                              f"safety cause sample: ten switches, none to the dead-man; no teleop payload on its "
                              f"angle outside a hold",
                         pairs == want and all(_deadline_ok(x, report) for x in deadlines) and in_hold and not outside,
                         f"{len(new)} switches; at_ms − last_ms {[round(gap(x), 3) for x in deadlines]}; "
                         f"{len(outside)} teleop payloads outside")
        for c in cs.values():
            c.stop()
    finally:
        bus.close()


# -- §3 ---------------------------------------------------------------------------------------------

def section3(report: Report) -> None:
    sec = "§3 a commander killed"
    bus = Bus()
    try:
        cs, acts = _setup_all(bus, report, sec)
        n0 = {a.service: len(a.switches()) for a in acts}
        k = time.monotonic_ns()
        cs["safety"].kill()
        time.sleep(2.0)
        for a in acts:
            new = a.switches()[n0[a.service]:]
            reads = a.ask("reads", "reads")["reads"]
            safety_reads = [r for r in reads if r["address"] == "lab/safety"]
            report.check(sec, f"Expected 1 ({a.service}): to teleop, cause withdrawn, before safety's deadline "
                              f"(at_ms − last_ms below 300); a read again of lab/safety finds no token held; it "
                              f"stays on teleop",
                         [sw(x) for x in new] == [("lab/safety", "lab/teleop", "withdrawn")] and gap(new[0]) < 300
                         and new[0]["at"] - k <= 200 * MS and len(safety_reads) == 1
                         and safety_reads[0]["complete"] and safety_reads[0]["held"] == [],
                         f"{[(sw(x), round(gap(x), 3), round((x['at'] - k) / MS, 3)) for x in new]} (the last "
                         f"two: at_ms − last_ms, and ms after the kill); reads {safety_reads}")
        n1 = {a.service: len(a.switches()) for a in acts}
        na = {a.service: a.ask("counts", "counts")["counts"]["providers"]["lab/safety"]["not_active"] for a in acts}
        t_tok = time.monotonic_ns()
        cs["safety"] = bus.commander("safety")
        _wait(lambda: any(t > t_tok and "/lab/safety/@zk/alive/steer.v1/" in k_ and kind == "put"
                          for t, k_, kind in bus.tokens), 5.0)
        seen_at = next((t for t, k_, kind in bus.tokens if t > t_tok and "/lab/safety/@zk/alive/" in k_
                        and kind == "put"), None)
        time.sleep(1.0)
        first = next((t for t, _ in bus.cmd_seen("safety", t_tok)), None)
        for a in acts:
            new = a.switches()[n1[a.service]:]
            # the token as the actuator heard it, on its own clock: nothing then
            log = a.ask("log", "log")["log"]
            decl = next((row[0] for row in log if row[1] == "declared" and "/lab/safety/@zk/alive/" in row[2]
                         and a.t0 is not None and a.t0 + int(row[0] * MS) > t_tok), None)
            report.check(sec, f"Expected 2 ({a.service}): nothing at the new instance's interface token; at its "
                              f"first arrival, to safety, cause sample",
                         [sw(x) for x in new] == [("lab/teleop", "lab/safety", "sample")] and seen_at is not None
                         and decl is not None and new[0]["at_ms"] > decl and first is not None
                         and abs(new[0]["at"] - first) <= 200 * MS,
                         f"{[sw(x) for x in new]}; its watch heard the token at {decl} ms of its clock, the switch at "
                         f"{new[0]['at_ms'] if new else None}; S saw the token {_rel(seen_at, t_tok)} ms and the first sample "
                         f"{_rel(first, t_tok)} ms after the start; the switch "
                         f"{_rel(new[0]['at'] if new else None, t_tok)} ms")
            before = a.ask("counts", "counts")["counts"]["providers"]["lab/safety"]["not_active"] - na[a.service]
            report.check(sec, f"Expected 2 ({a.service}), strictly: the switch at the new instance's first arrival, "
                              f"none of its samples counted not_active before it (F-269)",
                         before == 0, f"{before} of the new instance's samples counted not_active, handled before "
                                      f"its watch heard the token")
        for c in cs.values():
            c.stop()
    finally:
        bus.close()


def _rel(t: int | None, t0: int) -> float | None:
    return None if t is None else round((t - t0) / MS, 1)


# -- §4 ---------------------------------------------------------------------------------------------

class RawSafety:
    """§4's X: a raw session playing safety, its tokens declared by hand at
    core §8.1's keys, ``cmd`` put by hand every 50 ms."""

    def __init__(self, bus: Bus) -> None:
        from . import live

        self.bus = bus
        self.s = live.open_client(bus.endpoint)
        self.fp16 = bus.steer.fingerprint.removeprefix("sha256:")[:16]
        self.tokens: list[Any] = []
        self.n = 0
        self.putting = threading.Event()
        self.stop = threading.Event()
        threading.Thread(target=self._run, daemon=True).start()

    def declare(self, instance: str) -> None:
        lv = self.s.liveliness()
        self.instance = lv.declare_token(f"zk2/lab/safety/@zk/instance/{instance}")
        self.alive = lv.declare_token(f"zk2/lab/safety/@zk/alive/steer.v1/{instance}/{self.fp16}")

    def withdraw(self) -> None:
        self.alive.undeclare()
        self.instance.undeclare()

    def _run(self) -> None:
        t0 = time.monotonic()
        i = 0
        while not self.stop.is_set():
            d = t0 + i * 0.05 - time.monotonic()
            if d > 0:
                time.sleep(d)
            i += 1
            if self.putting.is_set() and not self.stop.is_set():
                try:
                    self.s.put("zk2/lab/safety/steer.v1/stream/cmd", f"safety {self.n}".encode())
                except Exception:  # noqa: BLE001 - closed while putting
                    return
                self.n += 1

    def close(self) -> None:
        from . import live

        self.stop.set()
        live.close(self.s)


def section4(report: Report) -> None:
    sec = "§4 a clean stop that publishes after its withdrawal"
    bus = Bus()
    x = None
    try:
        ct = bus.commander("teleop")
        x = RawSafety(bus)
        x.declare("00000000000000a1")
        x.putting.set()
        time.sleep(0.3)
        acts = [bus.actuator("rudder-l"), bus.actuator("rudder-r")]
        report.check(sec, "setup: each actuator follows safety, X",
                     all(_wait(lambda a=a: _following(bus, a, "lab/safety"), 5.0) for a in acts),
                     "; ".join(str([sw(s) for s in a.switches()]) for a in acts))
        time.sleep(0.5)
        n0 = {a.service: len(a.switches()) for a in acts}
        c0 = {a.service: a.ask("counts", "counts")["counts"]["providers"]["lab/safety"]["not_active"] for a in acts}
        t_w = time.monotonic_ns()
        x.withdraw()
        n_before = x.n
        time.sleep(1.0)
        x.putting.clear()
        after_n = x.n - n_before
        time.sleep(0.5)
        t_end = time.monotonic_ns()
        for a in acts:
            new = a.switches()[n0[a.service]:]
            c1 = a.ask("counts", "counts")["counts"]["providers"]["lab/safety"]["not_active"]
            reads = [r for r in a.ask("reads", "reads")["reads"] if r["address"] == "lab/safety"]
            ang = bus.angle(a, t_w + 50 * MS, t_end)
            report.check(sec, f"Expected 1 ({a.service}): to teleop at the withdrawal, cause withdrawn; nothing back "
                              f"to safety while X's samples arrive, none delivered (S sees only teleop on its "
                              f"angle), each counted not_active for lab/safety; the read again finds no token and "
                              f"re-admits nothing",
                         [sw(s) for s in new] == [("lab/safety", "lab/teleop", "withdrawn")]
                         and new[0]["at"] - t_w <= 200 * MS and bool(ang) and all(p.startswith("teleop ") for _, p in ang)
                         and c1 - c0[a.service] >= after_n - 2 and len(reads) == 1 and reads[0]["held"] == []
                         and reads[0]["complete"],
                         f"{[sw(s) for s in new]}; X put {after_n} after its withdrawal, not_active +"
                         f"{c1 - c0[a.service]}; reads {reads}; angle {sorted({p.split()[0] for _, p in ang})}")
        time.sleep(1.5)
        n1 = {a.service: len(a.switches()) for a in acts}
        t_tok = time.monotonic_ns()
        x.declare("00000000000000a2")
        time.sleep(0.5)
        quiet = {a.service: len(a.switches()) == n1[a.service] for a in acts}
        t_put = time.monotonic_ns()
        x.putting.set()
        time.sleep(0.5)
        for a in acts:
            new = a.switches()[n1[a.service]:]
            report.check(sec, f"Expected 2 ({a.service}): nothing at X's new tokens; at its first sample after them, "
                              f"to safety, cause sample",
                         quiet[a.service] and [sw(s) for s in new] == [("lab/teleop", "lab/safety", "sample")]
                         and new[0]["at"] >= t_put,
                         f"{[sw(s) for s in new]}; tokens at +0, puts from {_rel(t_put, t_tok)} ms, the switch "
                         f"{_rel(new[0]['at'] if new else None, t_tok)} ms")
        ct.stop()
    finally:
        if x is not None:
            x.close()
        bus.close()


# -- §5 ---------------------------------------------------------------------------------------------

def section5(report: Report) -> None:
    sec = "§5 a re-mint with no drop"
    bus = Bus()
    try:
        cs, acts = _setup_all(bus, report, sec)
        n0 = {a.service: len(a.switches()) for a in acts}
        t0 = time.monotonic_ns()
        done = []
        for _ in range(10):
            r = cs["safety"].ask("remint", "reminted")
            done.append(r is not None)
            time.sleep(1.0)
        for a in acts:
            new = a.switches()[n0[a.service]:]
            reads = a.ask("reads", "reads")["reads"]
            ang = bus.angle(a, t0)
            report.check(sec, f"Expected 1 ({a.service}): ten re-mints, no line, S sees only safety on its angle, "
                              f"and no read again",
                         all(done) and not new and bool(ang) and all(p.startswith("safety ") for _, p in ang)
                         and not reads,
                         f"{sum(done)} re-mints; lines {[sw(s) for s in new]}; reads {reads}; angle "
                         f"{sorted({p.split()[0] for _, p in ang})}")
        for c in cs.values():
            c.stop()
    finally:
        bus.close()


# -- §6 ---------------------------------------------------------------------------------------------

def section6(report: Report) -> None:
    sec = "§6 all silent: the dead-man"
    bus = Bus()
    try:
        cs, acts = _setup_all(bus, report, sec)
        n0 = {a.service: len(a.switches()) for a in acts}
        cs["safety"].hold()
        time.sleep(1.0)
        cs["teleop"].hold()
        time.sleep(1.0)
        cs["autopilot"].hold()
        time.sleep(2.0)
        t_resume = time.monotonic_ns()
        cs["autopilot"].resume()
        time.sleep(0.5)
        for a in acts:
            new = a.switches()[n0[a.service]:]
            want = [("lab/safety", "lab/teleop", "deadline"), ("lab/teleop", "lab/autopilot", "deadline"),
                    ("lab/autopilot", None, "deadline"), (None, "lab/autopilot", "sample")]
            dead = new[2] if len(new) > 2 else None
            silent = [] if dead is None else bus.angle(a, dead["at"], t_resume)
            report.check(sec, f"Expected 1–4 ({a.service}): to teleop, to autopilot, to none, each cause deadline, "
                              f"the dead-man's at_ms − last_ms above 300 and at most 306 (50 ms margin); no angle "
                              f"put from then until autopilot resumes; then to autopilot, cause sample",
                         [sw(s) for s in new] == want and all(_deadline_ok(s, report) for s in new[:3])
                         and not silent,
                         f"{[(sw(s), round(gap(s), 3) if s['last_ms'] is not None else None) for s in new]}; "
                         f"{len(silent)} angle puts in the dead-man")
        for c in cs.values():
            c.stop()
    finally:
        bus.close()


# -- §7 ---------------------------------------------------------------------------------------------

def section7(report: Report) -> None:
    sec = "§7 the watch session's own transport moves"
    bus = Bus()
    try:
        cs, acts = _setup_all(bus, report, sec, watch_links=True)
        n0 = {a.service: len(a.switches()) for a in acts}
        t0 = time.monotonic_ns()
        heals = 0
        for _ in range(5):
            for a in acts:
                a.link.cut()  # type: ignore[attr-defined]
            time.sleep(1.0)
            for a in acts:
                a.link.heal()  # type: ignore[attr-defined]
            heals += 1
            time.sleep(3.0)
        time.sleep(2.0)
        for a in acts:
            new = a.switches()[n0[a.service]:]
            trips = a.ask("trips", "round_trips")
            stood = [r for r in trips["round_trips"] if r["gen"] > 0 and r["stands"]]
            ang = bus.angle(a, t0)
            around = ""
            if new:
                # what the arbiter was handed around the first unexpected switch
                log = a.ask("log", "log")["log"]
                at = new[0]["at_ms"]
                around = f"; its log around it: {[r for r in log if abs(r[0] - at) <= 60]}"
            report.check(sec, f"Expected 1 ({a.service}): five cuts and heals of its watch link alone: no line, S "
                              f"sees only safety on its angle; after each heal a round trip on its watch session "
                              f"stood",
                         not new and bool(ang) and all(p.startswith("safety ") for _, p in ang)
                         and trips["moves"] >= 2 * heals and len(stood) >= heals,
                         f"lines {[(sw(s), s['at_ms']) for s in new]}; {trips['moves']} moves heard, {len(stood)} round "
                         f"trips stood after a move; angle {sorted({p.split()[0] for _, p in ang})}{around}")
        n1 = {a.service: len(a.switches()) for a in acts}
        for a in acts:
            a.link.cut()  # type: ignore[attr-defined]
        time.sleep(0.3)
        k = time.monotonic_ns()
        cs["safety"].kill()
        time.sleep(1.0)
        before_heal = {a.service: len(a.switches()) for a in acts}
        for a in acts:
            a.link.heal()  # type: ignore[attr-defined]
        time.sleep(4.0)
        for a in acts:
            new = a.switches()[n1[a.service]:]
            report.check(sec, f"Expected 2 ({a.service}): its watch link cut, safety killed: to teleop, cause "
                              f"deadline, at_ms − last_ms within §2's bounds; nothing more at the heal",
                         [sw(s) for s in new] == [("lab/safety", "lab/teleop", "deadline")]
                         and _deadline_ok(new[0], report) and before_heal[a.service] == n1[a.service] + 1,
                         f"{[(sw(s), round(gap(s), 3), _rel(s['at'], k)) for s in new]} (at_ms − last_ms, ms after "
                         f"the kill)")
        for c in cs.values():
            if c.p.poll() is None:
                c.stop()
    finally:
        bus.close()


def section7_control(report: Report) -> None:
    """F-268's control, not a section: §7 step 1's cuts with an actuator
    that begins a move only at its watch session's transport event."""
    sec = "7c"
    bus = Bus()
    try:
        cs, acts = _setup_all(bus, report, "§7 control (F-268)", watch_links=True, infer=False)
        n0 = {a.service: len(a.switches()) for a in acts}
        for _ in range(3):
            for a in acts:
                a.link.cut()  # type: ignore[attr-defined]
            time.sleep(1.0)
            for a in acts:
                a.link.heal()  # type: ignore[attr-defined]
            time.sleep(3.0)
        for a in acts:
            log = a.ask("log", "log")["log"]
            closes = [i for i, row in enumerate(log) if row[1] == "moved (close)"]
            first = sum(1 for i in closes if i > 0 and log[i - 1][1] == "withdrawn")
            report.note(sec, f"{a.service}, the transport event alone: lines "
                             f"{[sw(x) for x in a.switches()[n0[a.service]:]]}; at {first} of {len(closes)} closes "
                             f"a withdrawal was heard before the move's event")
        for c in cs.values():
            c.stop()
    finally:
        bus.close()


# -- §8 ---------------------------------------------------------------------------------------------

def section8(report: Report) -> None:
    sec = "§8 an invalid declaration, refused at start"
    bus = Bus()
    try:
        cs = {s: bus.commander(s) for s in ("autopilot", "teleop", "safety")}
        ctl = bus.actuator("rudder-l")
        ok = _wait(lambda: bool(ctl.switches()), 3.0) and sw(ctl.switches()[0]) == (None, None, "start") \
            and ctl.instance_key is not None
        report.check(sec, "control: A_l configured plainly prints its start line and serves within 3 s", ok,
                     f"{[sw(s) for s in ctl.switches()]}")
        ctl.stop()
        time.sleep(0.5)
        cases = [
            ("freshest", bus.copy(RUDDER, "freshest", '"arbitration.policy" = "priority"',
                                  '"arbitration.policy" = "freshest"'), STEER, "policy_unsupported"),
            ("vote", bus.copy(RUDDER, "vote", '"arbitration.policy" = "priority"', '"arbitration.policy" = "vote"'),
             STEER, "value"),
            ("cardinality one", bus.copy(RUDDER, "one", 'cardinality = "many"', 'cardinality = "one"'), STEER,
             "cardinality"),
            ("no deadline", RUDDER, bus.copy(STEER, "nodeadline", ', "timing.deadline_ms" = 300', ""),
             "no_deadline"),
            ("a deadline below the period", RUDDER,
             bus.copy(STEER, "below", '"timing.deadline_ms" = 300', '"timing.deadline_ms" = 20'), "no_deadline"),
        ]
        for name, contract, held, reason in cases:
            t = time.monotonic_ns()
            a = bus.actuator("rudder-l", contract=contract, held=held, wait=False)
            try:
                code = a.p.wait(timeout=5.0)
            except subprocess.TimeoutExpired:
                code = None
            time.sleep(0.2)
            with bus.lock:
                appeared = [k for tt, k, kind in bus.tokens if tt >= t and "/lab/rudder-l/" in k]
                put = [k for tt, k, _ in bus.seen if tt >= t and "/lab/rudder-l/" in k]
            err = " ".join(a.err)
            report.check(sec, f"step {name}: A_l refuses to start: no token of lab/rudder-l, nothing put, no switch "
                              f"printed, the refusal naming {reason}",
                         code == 1 and not appeared and not put and not a.lines()
                         and f"invalid: {reason}" in err and "not started:" in err,
                         f"exit {code}; stderr {err!r}; {len(appeared)} tokens, {len(put)} puts")
        for c in cs.values():
            c.stop()
    finally:
        bus.close()


# -- §9 ---------------------------------------------------------------------------------------------

def section9(report: Report) -> None:
    from . import live

    sec = "§9 a wildcard binding's order"
    bus = Bus()
    try:
        ct, ca = bus.commander("teleop"), bus.commander("autopilot")
        time.sleep(0.5)
        a = bus.actuator("rudder-l", ["lab/safety", "lab/*"])
        time.sleep(1.0)
        d = live.get_descriptor(bus.s, a.instance_key or "")
        doc = json.loads(d[0].payload) if d and d[0].ok else {}
        req = next((r for r in doc.get("requires", []) if r.get("role") == "cmd"), {})
        sws = a.switches()
        steps = [sw(s) for s in sws]
        ok1 = steps in ([(None, None, "start"), (None, "lab/autopilot", "sample")],
                        [(None, None, "start"), (None, "lab/teleop", "sample"), ("lab/teleop", "lab/autopilot", "sample")])
        c = a.ask("counts", "counts")["counts"]
        report.check(sec, "Expected 1: the descriptor lists cmd's bindings [lab/safety, lab/*] in that order; within "
                          "1 s A_l follows autopilot (byte order within the wildcard), through teleop first only by "
                          "its first sample; lab/rudder-l never a provider with arrivals",
                     req.get("bindings") == ["lab/safety", "lab/*"] and ok1 and sws[-1]["at_ms"] <= 1000
                     and "lab/rudder-l" not in c["providers"],
                     f"bindings {req.get('bindings')}; {steps}; providers counted {sorted(c['providers'])}")
        time.sleep(1.0)
        n = len(a.switches())
        ca.hold()
        time.sleep(1.0)
        new = a.switches()[n:]
        report.check(sec, "Expected 2: to teleop, cause deadline",
                     [sw(s) for s in new] == [("lab/autopilot", "lab/teleop", "deadline")]
                     and all(_deadline_ok(s, report) for s in new), f"{[(sw(s), round(gap(s), 3)) for s in new]}")
        n = len(a.switches())
        ca.resume()
        time.sleep(1.0)
        new = a.switches()[n:]
        report.check(sec, "Expected 3: to autopilot, cause sample",
                     [sw(s) for s in new] == [("lab/teleop", "lab/autopilot", "sample")], f"{[sw(s) for s in new]}")
        n = len(a.switches())
        cs_ = bus.commander("safety")
        time.sleep(1.0)
        new = a.switches()[n:]
        report.check(sec, "Expected 4: to safety, cause sample: the exact entry above the wildcard",
                     [sw(s) for s in new] == [("lab/autopilot", "lab/safety", "sample")], f"{[sw(s) for s in new]}")
        for p in (ct, ca, cs_):
            p.stop()
    finally:
        bus.close()


# -- §10 --------------------------------------------------------------------------------------------

def section10(report: Report) -> None:
    from . import arbitration as ab
    from . import live
    from .arbitration_live import ToolView
    from .contract import load_contract

    sec = "§10 a tool's questions"
    bus = Bus()
    try:
        t_s, t_w = live.open_client(bus.endpoint), live.open_client(bus.endpoint)
        rudder, steer = bus.rudder, bus.steer
        rudder_role = ab.role_of(rudder.canonical, "cmd")

        def read(a: Actuator, revisions: list[Any], listen_s: float | None) -> dict[str, ab.Answer]:
            """T: the consumer's descriptor (its role, bindings and parameter
            bindings), the contracts it holds offline, and its own view."""
            d = live.get_descriptor(bus.s, a.instance_key or "")
            doc = json.loads(d[0].payload)
            req = next(r for r in doc["requires"] if r["role"] == "cmd")
            params = ab.resolve_params(dict(req.get("params") or {}), doc["service"])
            declared = ab.tool_declaration(rudder_role if req.get("declared_by") == "rudder.v1" else None,
                                           [ab.provider_of(r.canonical) for r in revisions], params)
            view = None
            if listen_s is not None and isinstance(declared, ab.Declaration) and declared.valid:
                tv = ToolView(t_s, t_w, req["interface"], declared, list(req["bindings"]), params)
                time.sleep(listen_s)
                view = tv.view()
                tv.close()
            return ab.conclude(declared, list(req["bindings"]), view)

        def show(got: dict[str, ab.Answer]) -> str:
            return "; ".join(f"{q} {json.dumps(a.expect())}" for q, a in got.items())

        cs = {s: bus.commander(s) for s in ("autopilot", "teleop", "safety")}
        a = bus.actuator("rudder-l")
        time.sleep(1.0)
        got = read(a, [steer], 2.0)
        report.check(sec, "Expected 1: the declaration yes (valid); the selection yes (selected, lab/safety), the "
                          "tool's own view; the binding no (exact)",
                     got["declaration"].expect() == {"answer": "yes", "reason": "valid"}
                     and got["selection"].expect() == {"answer": "yes", "reason": "selected", "provider": "lab/safety"}
                     and got["binding"].expect() == {"answer": "no", "reason": "exact", "wildcards": [], "shadowed": []},
                     show(got))
        for c in cs.values():
            c.hold()
        time.sleep(0.5)
        got = read(a, [steer], 2.0)
        report.check(sec, "Expected 2: all three holding, the selection no (dead_man)",
                     got["selection"].expect() == {"answer": "no", "reason": "dead_man"}, show(got))
        got6 = read(a, [steer], 0.1)
        steer200 = load_contract(bus.copy(STEER, "d200", '"timing.deadline_ms" = 300', '"timing.deadline_ms" = 200'))
        got5 = read(a, [steer, steer200], 2.0)
        a.stop()
        cs["safety"].stop()
        for c in (cs["teleop"], cs["autopilot"]):
            c.resume()
        a9 = bus.actuator("rudder-l", ["lab/safety", "lab/*"])
        time.sleep(1.0)
        got = read(a9, [steer], 2.0)
        report.check(sec, "Expected 3: bound as §9, C_t and C_a running: the selection yes (selected, "
                          "lab/autopilot); the binding yes (wildcard), naming entry 1",
                     got["selection"].expect() == {"answer": "yes", "reason": "selected", "provider": "lab/autopilot"}
                     and got["binding"].expect() == {"answer": "yes", "reason": "wildcard", "wildcards": [1],
                                                     "shadowed": []}, show(got))
        a9.stop()
        fresh = load_contract(bus.copy(RUDDER, "freshest10", '"arbitration.policy" = "priority"',
                                       '"arbitration.policy" = "freshest"'))
        got = ab.conclude(ab.tool_declaration(ab.role_of(fresh.canonical, "cmd"), [ab.provider_of(steer.canonical)]),
                          [], None)
        report.check(sec, "Expected 4: a copy of rudder.v1 saying freshest, offline, no consumer: the declaration "
                          "no (invalid, policy_unsupported)",
                     got["declaration"].expect() == {"answer": "no", "reason": "invalid",
                                                     "reasons": ["policy_unsupported"]}, show(got))
        report.check(sec, "Expected 5: steer.v1 and a copy with a 200 ms deadline held: the declaration and the "
                          "selection unobservable (revisions_disagree)",
                     got5["declaration"].expect() == {"answer": "unobservable", "reason": "revisions_disagree"}
                     and got5["selection"].expect() == {"answer": "unobservable", "reason": "revisions_disagree"},
                     show(got5))
        report.check(sec, "Expected 6: listened 100 ms only, all three holding: the selection unobservable "
                          "(listening)",
                     got6["selection"].expect() == {"answer": "unobservable", "reason": "listening"}, show(got6))
        for c in (cs["teleop"], cs["autopilot"]):
            c.stop()
        live.close(t_s)
        live.close(t_w)
    finally:
        bus.close()


SECTIONS = {"1": section1, "2": section2, "3": section3, "4": section4, "5": section5, "6": section6,
            "7": section7, "8": section8, "9": section9, "10": section10, "7c": section7_control}


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="python -m zk2py.arbitration_scenarios", description=__doc__.split("\n")[0])
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
            report.check(f"§{name}", "the section ran to its end", False, f"{type(e).__name__}: {e}")
    passed = sum(1 for r in report.rows if r[2])
    failed = len(report.rows) - passed
    late = max(report.late_ms, default=0.0)
    print(f"arbitration scenarios: {passed} passed, {failed} failed; the largest lateness past the detection bound "
          f"{late:.3f} ms")
    for sec, name, ok, detail in report.rows:
        if not ok:
            print(f"FAIL [{sec}] {name}: {detail}")
    return 1 if failed else 0


if __name__ == "__main__":
    code = main()
    sys.stdout.flush()
    os._exit(code)

"""The ``arbitration.v1`` scenarios (``spec/profiles/arbitration/scenarios.md``,
text 0.2) on an in-process zenoh-python router, in every pairing of zk2py's
parties and the reference's: ``python -m zk2py.arbitration_scenarios
[--pair COMMANDERS:ACTUATORS]... [--only 1 … 11|7c]``, each side ``py`` or
``rs`` (round 78; zk2py alone in round 77).

- **The bus** is the text's: R1, a router on a free loopback port; every
  party a client of it. §7's watch links go through a TCP relay the test
  cuts and heals (:class:`zk2py.budget_scenarios.Link`), one per actuator.
- **The contracts** are ``steer.v1`` and ``rudder.v1``, copied from the text
  into ``impl/python/interop/arbitration/``, their fingerprints the
  reference CLI's; §8's and §10's copies are written to a temporary
  directory at run time.
- **The commanders** ``C_s``, ``C_t`` and ``C_a`` are, for ``py``,
  ``python -m zk2py.arbitration_owner commander``, zk2py's owner with its HLC
  on; for ``rs``, the reference's black box ``owner --commander``. Each is a
  process of its own, putting ``cmd`` every 50 ms as ``<service> <n>``, its
  first put 500 ms after its tokens (text 0.2, F-269). §4's ``X`` is a raw
  session of the runner's, its tokens declared by hand. §11's are
  ``redundancy.v1`` candidates (``--standby``) of ``steer.v1`` minor 1.
- **The actuators** ``A_l`` and ``A_r`` are, for ``py``, ``python -m
  zk2py.arbitration_owner actuator`` (three sessions: its service's, a
  watch, a reader); for ``rs``, the black box ``owner --arbitrate``. Each
  prints one JSON line per switch and answers ``arbitrate counts`` and
  ``arbitrate reads``. zk2py's gives its start on the host's monotonic
  clock, which every process shares; the reference's start is placed by its
  ``start`` line's arrival, the pipe's error well under a millisecond.
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
GOVERNED = DIR / "steer-governed.v1.toml"
BLACKBOX = Path(os.environ.get("ZK2PY_BLACKBOX", "/srv/dev/targets/zenkey-tier/blackbox"))
OWNER = BLACKBOX / "owner"
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

    def __init__(self, args: list[str], python: bool = True) -> None:
        env = dict(os.environ, PYTHONPATH=str(REPO / "impl" / "python"))
        argv = [sys.executable, "-u", "-m", "zk2py.arbitration_owner", *args] if python else args
        self.p = subprocess.Popen(argv, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
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
    """A commander, zk2py's (``py``) or the reference's (``rs``), each driven
    by the reference owner's ``commander …`` words, which zk2py's takes too."""

    def __init__(self, endpoint: str, service: str, contract: Path = STEER, impl: str = "py",
                 standby: bool = False, instance: str | None = None) -> None:
        extra = ["--standby", "--instance-id", instance or "0000000000000001"] if standby else []
        if impl == "py":
            super().__init__(["commander", f"lab/{service}", str(contract), "--connect", endpoint, *extra])
        else:
            super().__init__([str(OWNER), "--connect", endpoint, "--commander", *extra, f"lab/{service}",
                              str(contract)], python=False)
        self.service, self.impl = service, impl
        self.ready = _wait(lambda: any(x.startswith("ready ") for x in self.lines()) or self.p.poll() is not None,
                           15.0) and any(x.startswith("ready ") for x in self.lines())

    def ask(self, cmd: str, key: str = "commander") -> dict[str, Any] | None:
        n = len(self.lines())
        self.send(cmd)

        def got() -> dict[str, Any] | None:
            for line in self.lines()[n:]:
                if line.startswith("{"):
                    try:
                        x = json.loads(line)
                    except ValueError:
                        continue
                    if key in x:
                        return x
            return None

        _wait(lambda: got() is not None, 5.0)
        return got()

    def hold(self) -> None:
        self.ask("commander hold")

    def resume(self) -> None:
        self.ask("commander resume")

    def raw(self, on: bool) -> None:
        self.ask(f"commander raw {'on' if on else 'off'}")

    def sent(self) -> tuple[int, int]:
        x = self.ask("commander sent") or {}
        return x.get("sent", -1), x.get("refused", -1)

    def zid(self) -> str | None:
        return (self.ask("commander zid") or {}).get("zid")

    def remint(self) -> str | None:
        return (self.ask("commander new-epoch") or {}).get("instance")

    def role(self) -> str | None:
        x = self.ask("redundancy role", "redundancy") or {}
        return next((v.get("role") for v in (x.get("interfaces") or {}).values()), None)


class Actuator(Proc):
    """An actuator, zk2py's (``py``) or the reference's (``rs``)."""

    def __init__(self, endpoint: str, service: str, bind: list[str], *, contract: Path = RUDDER,
                 held: Path = STEER, watch: str | None = None, wait: bool = True, infer: bool = True,
                 impl: str = "py") -> None:
        self.impl = impl
        if impl == "py":
            args = ["actuator", f"lab/{service}", str(contract), str(held), "--connect", endpoint,
                    "--bind", ",".join(bind)]
            if not infer:
                args.append("--no-infer-moves")
            if watch is not None:
                args += ["--watch-connect", watch]
            super().__init__(args)
        else:
            if not infer:
                raise ValueError("the reference's actuator has no control of F-268")
            args = [str(OWNER), "--connect", endpoint, "--arbitrate", ",".join(bind)]
            if watch is not None:
                args += ["--watch-connect", watch]
            super().__init__(args + [f"lab/{service}", str(contract), str(held)], python=False)
        self.service = service
        self.t0: int | None = None
        if wait:
            _wait(lambda: (self._ready() is not None and self.t0 is not None) or self.p.poll() is not None, 15.0)

    def _ready(self) -> str | None:
        if self.impl == "py":
            with self.lock:
                r = next((x for x in self.err if x.startswith("ready ")), None)
            if r is not None and self.t0 is None:
                self.t0 = int(r.split(" t0=")[1].split()[0])
            return r
        with self.lock:
            rows = list(self.out)
        r = next((x for _, x in rows if x.startswith("ready ")), None)
        if self.t0 is None:
            # the start line's arrival, less its at_ms: the reference's start
            # on the runner's clock
            for t, x in rows:
                if x.startswith("{") and '"switch"' in x:
                    d = json.loads(x)
                    if d["switch"]["cause"] == "start":
                        self.t0 = t - int(d["at_ms"] * MS)
                    break
        return r

    @property
    def instance_key(self) -> str | None:
        r = self._ready()
        return None if r is None else r.split()[1]

    def counts(self) -> dict[str, Any]:
        return self.ask("arbitrate counts", "counts")["counts"]

    def reads_count(self) -> int:
        """The reads again made (§2.7), the read at start not counted."""
        return self.ask("arbitrate reads", "arbitrate")["reads"]

    def round_trips(self) -> int:
        return self.ask("arbitrate reads", "arbitrate")["round_trips"]

    def reads_detail(self) -> list[dict[str, Any]] | None:
        """zk2py's alone: each read again, its address, completeness and
        holders."""
        return None if self.impl != "py" else self.ask("reads", "reads")["reads"]

    def log(self) -> list[Any] | None:
        return None if self.impl != "py" else self.ask("log", "log")["log"]

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
    def __init__(self, oi: str = "py", ci: str = "py") -> None:
        import zenoh

        self.oi, self.ci = oi, ci

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

        #: each cmd sample S sees, with its stamp's id
        self.stamped: list[tuple[int, str, str, str | None]] = []

        def data(sample: Any) -> None:
            t, k, body = time.monotonic_ns(), str(sample.key_expr), sample.payload.to_bytes().decode()
            with self.lock:
                self.seen.append((t, k, body))
                if k.endswith("/stream/cmd"):
                    ts = sample.timestamp
                    self.stamped.append((t, k, body, None if ts is None else str(ts.get_id())))

        def tok(sample: Any) -> None:
            kind = "delete" if sample.kind == zenoh.SampleKind.DELETE else "put"
            with self.lock:
                self.tokens.append((time.monotonic_ns(), str(sample.key_expr), kind))

        self.extra.append(self.s.declare_subscriber("zk2/lab/**", zenoh.handlers.Callback(data)))
        self.extra.append(live.declare_liveliness_subscriber(self.s_watch, "zk2/lab/*/@zk/**", tok))
        self.tmp = tempfile.mkdtemp(prefix="zk2py-arbitration-")

    def commander(self, service: str, **kw: Any) -> Commander:
        c = Commander(self.endpoint, service, impl=self.oi, **kw)
        self.procs.append(c)
        return c

    def actuator(self, service: str, bind: list[str] | None = None, **kw: Any) -> Actuator:
        a = Actuator(self.endpoint, service, EXACT if bind is None else bind, impl=self.ci, **kw)
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

def section1(report: Report, oi: str = "py", ci: str = "py") -> None:
    sec = "§1 the dead-man at start, and each higher priority taking over" + f" [{oi}→{ci}]"
    bus = Bus(oi, ci)
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
            c = a.counts()
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

def section2(report: Report, oi: str = "py", ci: str = "py") -> None:
    sec = "§2 a commander silent past its deadline" + f" [{oi}→{ci}]"
    bus = Bus(oi, ci)
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

def section3(report: Report, oi: str = "py", ci: str = "py") -> None:
    sec = "§3 a commander killed" + f" [{oi}→{ci}]"
    bus = Bus(oi, ci)
    try:
        cs, acts = _setup_all(bus, report, sec)
        n0 = {a.service: len(a.switches()) for a in acts}
        k = time.monotonic_ns()
        cs["safety"].kill()
        time.sleep(2.0)
        for a in acts:
            new = a.switches()[n0[a.service]:]
            reads = a.reads_detail()
            if reads is None:
                n_reads = a.reads_count()
                read_ok, shown = n_reads == 1, f"{n_reads} read(s) again"
            else:
                safety_reads = [r for r in reads if r["address"] == "lab/safety"]
                read_ok = len(safety_reads) == 1 and safety_reads[0]["complete"] and safety_reads[0]["held"] == []
                shown = f"reads {safety_reads}"
            report.check(sec, f"Expected 1 ({a.service}): to teleop, cause withdrawn, before safety's deadline "
                              f"(at_ms − last_ms below 300); a read again of lab/safety finds no token held; it "
                              f"stays on teleop",
                         [sw(x) for x in new] == [("lab/safety", "lab/teleop", "withdrawn")] and gap(new[0]) < 300
                         and new[0]["at"] - k <= 200 * MS and read_ok,
                         f"{[(sw(x), round(gap(x), 3), round((x['at'] - k) / MS, 3)) for x in new]} (the last "
                         f"two: at_ms − last_ms, and ms after the kill); {shown}")
        n1 = {a.service: len(a.switches()) for a in acts}
        na = {a.service: a.counts()["providers"]["lab/safety"]["not_active"] for a in acts}
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
            log = a.log()
            decl = None if log is None else next(
                (row[0] for row in log if row[1] == "declared" and "/lab/safety/@zk/alive/" in row[2]
                 and a.t0 is not None and a.t0 + int(row[0] * MS) > t_tok), None)
            report.check(sec, f"Expected 2 ({a.service}): nothing at the new instance's interface token; at its "
                              f"first arrival after its watch heard the token, to safety, cause sample",
                         [sw(x) for x in new] == [("lab/teleop", "lab/safety", "sample")] and seen_at is not None
                         and (log is None or (decl is not None and new[0]["at_ms"] > decl)) and first is not None
                         and abs(new[0]["at"] - first) <= 200 * MS,
                         f"{[sw(x) for x in new]}; its watch heard the token at {decl} ms of its clock, the switch at "
                         f"{new[0]['at_ms'] if new else None}; S saw the token {_rel(seen_at, t_tok)} ms and the first sample "
                         f"{_rel(first, t_tok)} ms after the start; the switch "
                         f"{_rel(new[0]['at'] if new else None, t_tok)} ms")
            before = a.counts()["providers"]["lab/safety"]["not_active"] - na[a.service]
            report.check(sec, f"Expected 2 ({a.service}), with the commander's 500 ms lead (text 0.2, F-269): its "
                              f"first, none of its samples counted not_active before it",
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


def section4(report: Report, oi: str = "py", ci: str = "py") -> None:
    sec = "§4 a clean stop that publishes after its withdrawal" + f" [{oi}→{ci}]"
    bus = Bus(oi, ci)
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
        c0 = {a.service: a.counts()["providers"]["lab/safety"]["not_active"] for a in acts}
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
            c1 = a.counts()["providers"]["lab/safety"]["not_active"]
            detail = a.reads_detail()
            if detail is None:
                n_reads = a.reads_count()
                reads_ok, reads = n_reads == 1, f"{n_reads} read(s) again"
            else:
                rs_ = [r for r in detail if r["address"] == "lab/safety"]
                reads_ok, reads = len(rs_) == 1 and rs_[0]["held"] == [] and rs_[0]["complete"], str(rs_)
            ang = bus.angle(a, t_w + 50 * MS, t_end)
            report.check(sec, f"Expected 1 ({a.service}): to teleop at the withdrawal, cause withdrawn; nothing back "
                              f"to safety while X's samples arrive, none delivered (S sees only teleop on its "
                              f"angle), each counted not_active for lab/safety; the read again finds no token and "
                              f"re-admits nothing",
                         [sw(s) for s in new] == [("lab/safety", "lab/teleop", "withdrawn")]
                         and new[0]["at"] - t_w <= 200 * MS and bool(ang) and all(p.startswith("teleop ") for _, p in ang)
                         and c1 - c0[a.service] >= after_n - 2 and reads_ok,
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

def section5(report: Report, oi: str = "py", ci: str = "py") -> None:
    sec = "§5 a re-mint with no drop" + f" [{oi}→{ci}]"
    bus = Bus(oi, ci)
    try:
        cs, acts = _setup_all(bus, report, sec)
        n0 = {a.service: len(a.switches()) for a in acts}
        t0 = time.monotonic_ns()
        done = []
        for _ in range(10):
            r = cs["safety"].remint()
            done.append(r is not None)
            time.sleep(1.0)
        for a in acts:
            new = a.switches()[n0[a.service]:]
            reads = a.reads_count()
            ang = bus.angle(a, t0)
            report.check(sec, f"Expected 1 ({a.service}): ten re-mints, no line, S sees only safety on its angle, "
                              f"and no read again",
                         all(done) and not new and bool(ang) and all(p.startswith("safety ") for _, p in ang)
                         and reads == 0,
                         f"{sum(done)} re-mints; lines {[sw(s) for s in new]}; reads {reads}; angle "
                         f"{sorted({p.split()[0] for _, p in ang})}")
        for c in cs.values():
            c.stop()
    finally:
        bus.close()


# -- §6 ---------------------------------------------------------------------------------------------

def section6(report: Report, oi: str = "py", ci: str = "py") -> None:
    sec = "§6 all silent: the dead-man" + f" [{oi}→{ci}]"
    bus = Bus(oi, ci)
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

def section7(report: Report, oi: str = "py", ci: str = "py") -> None:
    sec = "§7 the watch session's own transport moves" + f" [{oi}→{ci}]"
    bus = Bus(oi, ci)
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
            ang = bus.angle(a, t0)
            around = ""
            if a.impl == "py":
                trips = a.ask("trips", "round_trips")
                stood = [r for r in trips["round_trips"] if r["gen"] > 0 and r["stands"]]
                trips_ok = trips["moves"] >= 2 * heals and len(stood) >= heals
                trips_shown = f"{trips['moves']} moves heard, {len(stood)} round trips stood after a move"
                if new:
                    # what the arbiter was handed around the first unexpected switch
                    log = a.ask("log", "log")["log"]
                    at = new[0]["at_ms"]
                    around = f"; its log around it: {[r for r in log if abs(r[0] - at) <= 60]}"
            else:
                n_trips = a.round_trips()
                trips_ok = n_trips >= heals + 1
                trips_shown = f"{n_trips} round trips on its watch session, the start's included"
            report.check(sec, f"Expected 1 ({a.service}): five cuts and heals of its watch link alone: no line, S "
                              f"sees only safety on its angle; after each heal a round trip on its watch session "
                              f"stood",
                         not new and bool(ang) and all(p.startswith("safety ") for _, p in ang) and trips_ok,
                         f"lines {[(sw(s), s['at_ms']) for s in new]}; {trips_shown}; angle "
                         f"{sorted({p.split()[0] for _, p in ang})}{around}")
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


def section7_control(report: Report, oi: str = "py", ci: str = "py") -> None:
    """F-268's control, not a section: §7 step 1's cuts with an actuator
    that begins a move only at its watch session's transport event."""
    sec = "7c"
    bus = Bus(oi, ci)
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

def section8(report: Report, oi: str = "py", ci: str = "py") -> None:
    sec = "§8 an invalid declaration, refused at start" + f" [{oi}→{ci}]"
    bus = Bus(oi, ci)
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
            with bus.lock:
                puts = [(k, p_) for tt, k, p_ in bus.seen if tt >= t and "/lab/rudder-l/" in k]
            err = " ".join(a.err)
            report.check(sec, f"step {name}: A_l refuses to start: exit 1, no token of lab/rudder-l, no switch "
                              f"printed, the refusal naming {reason}",
                         code == 1 and not appeared and not a.switches()
                         and not any(x.startswith("ready ") for x in a.lines())
                         and f"invalid: {reason}" in err and "not started:" in err,
                         f"exit {code}; stderr {err!r}; {len(appeared)} tokens")
            report.check(sec, f"step {name}: nothing of A_l is put (§2.2: \"nothing of the instance is declared or "
                              f"put\")", not put, f"{len(put)} puts: {puts}")
        for c in cs.values():
            c.stop()
    finally:
        bus.close()


# -- §9 ---------------------------------------------------------------------------------------------

def section9(report: Report, oi: str = "py", ci: str = "py") -> None:
    from . import live

    sec = "§9 a wildcard binding's order" + f" [{oi}→{ci}]"
    bus = Bus(oi, ci)
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
        c = a.counts()
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

def section10(report: Report, oi: str = "py", ci: str = "py") -> None:
    from . import arbitration as ab
    from . import live
    from .arbitration_live import ToolView
    from .contract import load_contract

    sec = "§10 a tool's questions" + f" [{oi}→{ci}]"
    bus = Bus(oi, ci)
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
        # text 0.2 (F-272): with no consumer, no descriptor, so no binding
        got = ab.conclude(ab.tool_declaration(ab.role_of(fresh.canonical, "cmd"), [ab.provider_of(steer.canonical)]),
                          None, None)
        report.check(sec, "Expected 4: a copy of rudder.v1 saying freshest, offline, no consumer: the declaration "
                          "no (invalid, policy_unsupported); the selection and the binding not asked (no_descriptor)",
                     got["declaration"].expect() == {"answer": "no", "reason": "invalid",
                                                     "reasons": ["policy_unsupported"]}
                     and got["selection"].expect() == {"answer": "not_asked", "reason": "no_descriptor"}
                     and got["binding"].expect() == {"answer": "not_asked", "reason": "no_descriptor",
                                                     "wildcards": [], "shadowed": []}, show(got))
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


# -- core 0.56: a round trip asked at an open ----------------------------------------------------------

def section_open(report: Report, oi: str = "py", ci: str = "py", reopens: int = 20) -> None:
    """Core 0.56 §8.1 and Appendix B on zenoh-python 1.10.1, not a section:
    "a round trip asked at once at the open can find no face, and is
    answered at once, complete and empty, before the reopen tells a token",
    and one asked 100 ms after the event follows the telling. A watch
    session through a link the runner cuts and heals, its subscriber without
    history on one interface token a raw session of the runner's holds;
    at each open event a round trip asked at once (alternate reopens) or
    100 ms after; each compared with the instant the reopen told the token."""
    import zenoh

    from . import live
    from .budget_scenarios import Link

    sec = "0.56"
    bus = Bus(oi, ci)
    try:
        holder = live.open_client(bus.endpoint)
        key = "zk2/lab/safety/@zk/alive/steer.v1/00000000000000c1/0000000000000000"
        tok = holder.liveliness().declare_token(key)
        link = Link(bus.endpoint)
        bus.links.append(link)
        w = live.open_client(link.endpoint)
        told: list[tuple[int, str]] = []
        lock = threading.Lock()

        def heard(sample: Any) -> None:
            with lock:
                told.append((time.monotonic_ns(), "delete" if sample.kind == zenoh.SampleKind.DELETE else "put"))

        sel = "zk2/lab/safety/@zk/alive/**"
        sub = live.declare_liveliness_subscriber(w, sel, heard, history=False)
        live.settle(w, 5.0, selector=sel)
        results: list[tuple[str, bool, int | None, int | None, str]] = []
        cur: dict[str, Any] = {}
        opened = threading.Event()

        def moved(event: Any) -> None:
            if event.kind == zenoh.SampleKind.DELETE:
                return
            at = time.monotonic_ns()
            margin = cur.get("margin", 0.0)
            opened.set()

            def ask() -> None:
                if margin:
                    time.sleep(margin)
                asked = time.monotonic_ns()
                rt = live.settle(w, 5.0, selector=sel)
                ended = time.monotonic_ns()
                with lock:
                    tells = [t for t, k in told if t >= at and k == "put"]
                first = tells[0] if tells else None
                results.append(("at the event" if not margin else "100 ms after", bool(rt.stands),
                                None if first is None else first - ended, asked - at, rt.why))
            threading.Thread(target=ask, daemon=True).start()

        lst = live._info(w).declare_transport_events_listener(zenoh.handlers.Callback(moved), history=False)
        for i in range(reopens):
            cur["margin"] = 0.0 if i % 2 == 0 else 0.1
            opened.clear()
            link.cut()
            time.sleep(0.3)
            link.heal()
            # zenoh's client reconnects on its own schedule: the next cut only
            # once this reopen was heard, and its round trip answered
            opened.wait(10.0)
            time.sleep(0.5)
        lst.undeclare()
        sub.undeclare()
        tok.undeclare()
        for how in ("at the event", "100 ms after"):
            rows = [r for r in results if r[0] == how]
            before = [r for r in rows if r[1] and r[2] is not None and r[2] > 0]
            report.note(sec, f"a round trip asked {how}: {len(rows)} reopens, {sum(1 for r in rows if r[1])} stood, "
                             f"{len(before)} of them before the reopen told the token (the telling "
                             f"{', '.join(f'{r[2] / MS:+.2f}' for r in rows if r[2] is not None)} ms after the "
                             f"round trip's reply); not standing: {sorted({r[4] for r in rows if not r[1]})}")
        live.close(w)
        live.close(holder)
    finally:
        bus.close()


# -- §11 --------------------------------------------------------------------------------------------

def section11(report: Report, oi: str = "py", ci: str = "py") -> None:
    """A governed commander's standby (text 0.2, F-270): ``steer.v1`` minor
    1, every commander a ``redundancy.v1`` candidate; ``C_s2`` stands by and
    its own session puts raw, which no actuator may count as an arrival."""
    from .arbitration import PER_PROVIDER

    sec = "§11 a governed commander's standby" + f" [{oi}→{ci}]"
    bus = Bus(oi, ci)
    try:
        ct = bus.commander("teleop", contract=GOVERNED, standby=True, instance="00000000000000a0")
        cs1 = bus.commander("safety", contract=GOVERNED, standby=True, instance="0000000000000001")
        up = _wait(lambda: cs1.role() == "serving" and ct.role() == "serving", 10.0, 0.2)
        cs2 = bus.commander("safety", contract=GOVERNED, standby=True, instance="0000000000000002")
        stands = _wait(lambda: cs2.role() == "standby", 10.0, 0.2)
        z1, z2 = cs1.zid(), cs2.zid()
        report.check(sec, "setup: C_t and C_s1 serve, C_s2 (the higher instance id) stands by",
                     all(c.ready for c in (ct, cs1, cs2)) and up and stands,
                     f"roles: C_t {ct.role()}, C_s1 {cs1.role()}, C_s2 {cs2.role()}; zids C_s1 {z1}, C_s2 {z2}")
        time.sleep(0.6)  # the commanders' lead
        acts = [bus.actuator(svc, ["lab/safety", "lab/teleop"], held=GOVERNED) for svc in ("rudder-l", "rudder-r")]
        t_a = time.monotonic_ns()
        followed = {a.service: _wait(lambda a=a: _following(bus, a, "lab/safety"), 1.5) for a in acts}
        time.sleep(0.3)
        n0 = {a.service: len(a.switches()) for a in acts}
        c0 = {a.service: a.counts()["providers"].get("lab/safety", {k: 0 for k in PER_PROVIDER}) for a in acts}
        t1 = time.monotonic_ns()
        cs2.raw(True)
        time.sleep(1.0)
        t1_end = time.monotonic_ns()
        c1 = {a.service: a.counts()["providers"].get("lab/safety", {k: 0 for k in PER_PROVIDER}) for a in acts}
        with bus.lock:
            by1 = {body for t, k, body, i in bus.stamped if "/lab/safety/" in k and _zid_eq(i, z1)}
            raw = [(t, body) for t, k, body, i in bus.stamped if "/lab/safety/" in k and _zid_eq(i, z2)
                   and t1 <= t <= t1_end]
        for a in acts:
            sws = a.switches()
            ang = [x for _, x in bus.angle(a, t1, t1_end)]
            foreign = [x for x in ang if x not in by1]
            ni = c1[a.service]["not_incumbent"] - c0[a.service]["not_incumbent"]
            report.check(sec, f"Expected 1 ({a.service}): follows lab/safety within 1 s of its start, its incumbent "
                              f"learned by its read at start and C_s1's meta.zid; through step 1 no line, no standby "
                              f"payload on its angle, and its not_incumbent count for lab/safety grows with X's puts, "
                              f"about 20 a second",
                         # §2.9's stated cost: a lower provider may drive at start
                         # until a higher one's first arrival
                         followed[a.service] and sw(sws[0]) == (None, None, "start")
                         and all(sw(x)[2] == "sample" for x in sws[1:n0[a.service]])
                         and sw(sws[n0[a.service] - 1])[1] == "lab/safety"
                         and sws[n0[a.service] - 1]["at"] - t_a <= 1000 * MS + 500 * MS and len(sws) == n0[a.service]
                         and bool(ang) and not foreign and 15 <= ni <= 25 and abs(ni - len(raw)) <= 2,
                         f"{[(sw(x), x['at_ms']) for x in sws]}; {len(ang)} angle puts in step 1, "
                         f"{len(foreign)} not C_s1's; not_incumbent +{ni}, X put {len(raw)} raw (S saw them)")
        n1 = {a.service: len(a.switches()) for a in acts}
        cs1.hold()
        time.sleep(2.0)
        for a in acts:
            new = a.switches()[n1[a.service]:]
            report.check(sec, f"Expected 2 ({a.service}): C_s1 holds: to teleop, cause deadline, at_ms − last_ms "
                              f"within §2's bounds: X's samples, counted not_incumbent, keep nothing eligible",
                         [sw(x) for x in new] == [("lab/safety", "lab/teleop", "deadline")]
                         and all(_deadline_ok(x, report) for x in new),
                         f"{[(sw(x), round(gap(x), 3)) for x in new]}")
        n2 = {a.service: len(a.switches()) for a in acts}
        t3 = time.monotonic_ns()
        cs1.resume()
        time.sleep(1.0)
        first = next((t for t, k, body, i in sorted(bus.stamped) if t >= t3 and "/lab/safety/" in k
                      and _zid_eq(i, z1)), None)
        for a in acts:
            new = a.switches()[n2[a.service]:]
            c = a.counts()
            report.check(sec, f"Expected 3 ({a.service}): at C_s1's first arrival, to lab/safety, cause sample",
                         [sw(x) for x in new] == [("lab/teleop", "lab/safety", "sample")] and first is not None
                         and abs(new[0]["at"] - first) <= 200 * MS,
                         f"{[sw(x) for x in new]}; the switch {_rel(new[0]['at'] if new else None, first or 0)} ms "
                         f"after S saw C_s1's first; counts {json.dumps(c['providers'])}")
        cs2.raw(False)
        for c in (ct, cs1, cs2):
            c.stop()
    finally:
        bus.close()


def _zid_eq(a: str | None, b: str | None) -> bool:
    from .timing import zid_value

    return a is not None and b is not None and zid_value(a) is not None and zid_value(a) == zid_value(b)


SECTIONS = {"1": section1, "2": section2, "3": section3, "4": section4, "5": section5, "6": section6,
            "7": section7, "8": section8, "9": section9, "10": section10, "11": section11, "7c": section7_control,
            "open": section_open}
PAIRS = ["py:py", "rs:py", "py:rs", "rs:rs"]


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="python -m zk2py.arbitration_scenarios", description=__doc__.split("\n")[0])
    ap.add_argument("--only", action="append", choices=list(SECTIONS), help="run only these sections")
    ap.add_argument("--pair", action="append", choices=PAIRS, help="commanders:actuators, each py or rs")
    args = ap.parse_args(argv)
    report = Report()
    for pair in args.pair or ["py:py"]:
        oi, ci = pair.split(":")
        for name, fn in SECTIONS.items():
            if args.only and name not in args.only:
                continue
            if name in ("7c", "open") and (ci != "py" or oi != "py"):
                continue  # F-268's control and core 0.56's measurement: zenoh-python's alone, once
            try:
                fn(report, oi, ci)
            except Exception as e:  # noqa: BLE001 - a section that crashed is a failure, the rest still run
                import traceback

                traceback.print_exc()
                report.check(f"§{name} [{oi}→{ci}]", "the section ran to its end", False, f"{type(e).__name__}: {e}")
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

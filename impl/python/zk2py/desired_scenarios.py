"""desired.v1 ``scenarios.md`` §1 to §8 and §10 (text 0.3), live, from Python
(#609, round 55)::

    python -m zk2py.desired_scenarios [--author zk2py|<owner exe>] [--target zk2py|<owner exe>]
        [--only 1|2|3|4|5|6|7|8|10]... [--poll]

Each party is a process driven as a black box through the owner example's
interface (``owner --help``: its ``--desired-*`` options and ``desired …``
commands): zk2py's own (``python -m zk2py.desired_owner``), or the Rust
owner example at a path, or one of each. The bus is zk2py's: R1, and where
a section cuts a link R2 behind a link the runner cuts and heals; S on R1
(every delivery under ``zk2/lab/**``, and the instance tokens, on a watch
session), S2 on R2 (``Rp`` and ``Run``), G (S4's GETs), X (§2's puts with a
timestamp it sets), and a stand-in archive (core §4.4) for §7 and §10.

Targets follow their author on a watch session (``--desired-watch``, core
§8.1, 0.37) unless ``--poll``. Exit 0 when every check passes.
"""

from __future__ import annotations

import argparse
import json
import os
import queue
import signal
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path
from typing import Any

from .alarms_scenarios import Report, _wait

REPO = Path(__file__).resolve().parents[3]
AUTHOR, TARGET, SPARE = "lab/author", "lab/worker", "lab/spare"
D = "zk2/lab/author/tasking.v1/state/tasks/lab/worker"
D2 = "zk2/lab/author/tasking.v1/state/tasks/lab/spare"
RP = "zk2/lab/worker/worker.v1/state/desired/task"
RUN = "zk2/lab/worker/worker.v1/state/running"
BASELINE = {"revision": 0, "mode": "idle", "rate_hz": 1}


def v(n: int, **kw: Any) -> dict[str, Any]:
    """scenarios.md: "v<n> is a Task with revision n, mode run and rate_hz 10
    unless a step says otherwise"."""
    return {"revision": n, "mode": "run", "rate_hz": 10, **kw}


class Party:
    """An owner process: zk2py's (``impl == "zk2py"``) or the owner example."""

    def __init__(self, impl: Any, address: str, endpoint: str, flags: list[str]) -> None:
        if impl == "zk2py":
            argv = [sys.executable, "-m", "zk2py.desired_owner", "--connect", endpoint, *flags, address]
        else:
            argv = [str(impl), "--connect", endpoint, *flags, address]
        env = dict(os.environ, PYTHONPATH=str(REPO / "impl" / "python"))
        self.impl, self.address, self.flags = impl, address, flags
        self.proc = subprocess.Popen(argv, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                     text=True, env=env)
        self.lines: queue.Queue[str] = queue.Queue()
        self.stderr: list[str] = []
        threading.Thread(target=lambda: [self.lines.put(x.rstrip("\n")) for x in self.proc.stdout],
                         daemon=True).start()
        threading.Thread(target=lambda: [self.stderr.append(x) for x in self.proc.stderr], daemon=True).start()
        self.instance = self._ready(20.0)

    @property
    def who(self) -> str:
        return "zk2py" if self.impl == "zk2py" else "rust"

    def _ready(self, timeout: float) -> str | None:
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            try:
                line = self.lines.get(timeout=0.2)
            except queue.Empty:
                if self.proc.poll() is not None:
                    return None
                continue
            if line.startswith("ready "):
                return line[len("ready "):]
        return None

    def cmd(self, line: str, timeout: float = 10.0) -> str | None:
        try:
            self.proc.stdin.write(line + "\n")
            self.proc.stdin.flush()
        except OSError:
            return None
        try:
            return self.lines.get(timeout=timeout)
        except queue.Empty:
            return None

    def _json(self, word: str) -> Any:
        got = self.cmd(f"desired {word}")
        prefix = f"desired {word} "
        if got is None or not got.startswith(prefix):
            return None
        return json.loads(got[len(prefix):])

    def report(self) -> Any:
        return self._json("report")

    def running(self) -> Any:
        return self._json("running")

    def acts(self) -> tuple[int, int, bool] | None:
        got = self.cmd("desired acts")
        if got is None or not got.startswith("desired acts "):
            return None
        n, seeds, held = got[len("desired acts "):].split(" ")
        return int(n), int(seeds.split("=")[1]), held.split("=")[1] == "true"

    def put(self, target: str, doc: Any) -> str | None:
        """The stamp of the author's put, as a report writes it."""
        got = self.cmd(f"desired put {target} {json.dumps(doc)}")
        return got.rsplit(" ", 1)[1] if got and got.startswith("desired put ") else None

    def delete(self, target: str) -> str | None:
        got = self.cmd(f"desired delete {target}")
        return got.rsplit(" ", 1)[1] if got and got.startswith("desired deleted ") else None

    def kill(self) -> None:
        """"Its process ends without closing"."""
        if self.proc.poll() is None:
            self.proc.send_signal(signal.SIGKILL)
        self.proc.wait(10)

    def close(self) -> int | None:
        """A clean close: standard input closed."""
        try:
            self.proc.stdin.close()
        except OSError:
            pass
        try:
            return self.proc.wait(timeout=15)
        except subprocess.TimeoutExpired:
            self.kill()
            return None


class Bus:
    """R1; R2 behind a link when ``cut``; S, its token watch, S2, G and X."""

    def __init__(self, cut: bool = False) -> None:
        import zenoh

        from . import live
        from .budget_scenarios import Link
        from .live_interop import _r1
        from .owner import free_loopback_port

        self.zenoh = zenoh
        self.r1, self.ep1, _ = _r1()
        self.link = None
        self.r2 = None
        self.ep2 = self.ep1
        if cut:
            self.link = Link(self.ep1)
            port = free_loopback_port()
            conf = zenoh.Config()
            conf.insert_json5("mode", json.dumps("router"))
            conf.insert_json5("listen/endpoints", json.dumps([f"tcp/127.0.0.1:{port}"]))
            conf.insert_json5("connect/endpoints", json.dumps([self.link.endpoint]))
            conf.insert_json5("scouting/multicast/enabled", "false")
            conf.insert_json5("timestamping/enabled", "true")
            self.r2 = zenoh.open(conf)
            self.ep2 = f"tcp/127.0.0.1:{port}"
        self.lock = threading.Lock()
        #: S: (monotonic s, key, "put"|"delete", payload, stamp text, the zenoh timestamp)
        self.got: list[tuple[float, str, str, bytes, str | None, Any]] = []
        self.tokens: list[tuple[float, str, str]] = []
        self.got2: list[tuple[float, str, str, bytes, str | None]] = []
        self.s = live.open_client(self.ep1)
        self.entities = [self.s.declare_subscriber("zk2/lab/**", zenoh.handlers.Callback(self._s))]
        sub, _ = live.watch_or_beside(self.s, "zk2/lab/*/@zk/instance/*", zenoh.handlers.Callback(self._token),
                                      history=True)
        self.entities.append(sub)
        self.g1 = live.open_client(self.ep1)
        self.g2 = live.open_client(self.ep2) if cut else self.g1
        self.s2 = None
        if cut:
            self.s2 = live.open_client(self.ep2)
            for k in (RP, RUN):
                self.entities.append(self.s2.declare_subscriber(k, zenoh.handlers.Callback(self._s2)))
        self.x = live.open_client(self.ep1)
        self.parties: list[Party] = []
        self.extra: list[Any] = []
        time.sleep(0.3)

    def _s(self, smp: Any) -> None:
        ts = smp.timestamp
        with self.lock:
            self.got.append((time.monotonic(), str(smp.key_expr), "delete" if smp.kind == self.zenoh.SampleKind.DELETE
                             else "put", smp.payload.to_bytes(), None if ts is None else str(ts), ts))

    def _s2(self, smp: Any) -> None:
        ts = smp.timestamp
        with self.lock:
            self.got2.append((time.monotonic(), str(smp.key_expr), "delete" if smp.kind == self.zenoh.SampleKind.DELETE
                              else "put", smp.payload.to_bytes(), None if ts is None else str(ts)))

    def _token(self, smp: Any) -> None:
        with self.lock:
            self.tokens.append((time.monotonic(), "put" if smp.kind == self.zenoh.SampleKind.PUT else "delete",
                                str(smp.key_expr)))

    def since(self, t: float, key: str | None = None) -> list[tuple[float, str, str, bytes, str | None, Any]]:
        with self.lock:
            return [g for g in self.got if g[0] >= t and (key is None or g[1] == key)]

    def since2(self, t: float, key: str | None = None) -> list[tuple[float, str, str, bytes, str | None]]:
        with self.lock:
            return [g for g in self.got2 if g[0] >= t and (key is None or g[1] == key)]

    def token_put(self, address: str, after: float) -> str | None:
        """The key of the first instance token of ``address`` S saw appear at
        or after ``after``."""
        with self.lock:
            for t, kind, key in self.tokens:
                if t >= after and kind == "put" and key.startswith(f"zk2/{address}/@zk/instance/"):
                    return key
        return None

    def party(self, impl: Any, address: str, flags: list[str], on: int = 1) -> Party:
        p = Party(impl, address, self.ep1 if on == 1 else self.ep2, flags)
        self.parties.append(p)
        return p

    def get(self, key: str, on: int = 1) -> Any:
        from . import live

        return live.get_state(self.g1 if on == 1 else self.g2, key)

    def get_json(self, key: str, on: int = 1) -> Any:
        st = self.get(key, on)
        r = [x for x in st.replies if x.key == key and not x.deleted]
        return (json.loads(r[0].payload), r[0].stamp) if r else (None, None)

    def descriptor(self, instance_key: str, on: int = 1) -> Any:
        from . import live

        d = live.get_descriptor(self.g1 if on == 1 else self.g2, instance_key)
        return json.loads(d[0].payload) if d and d[0].ok else None

    def author_seen(self, on: int) -> bool:
        from . import live

        p = live.list_presence(self.g1 if on == 1 else self.g2, f"zk2/{AUTHOR}/@zk/instance/*")
        return bool(p.instances)

    def cut(self) -> bool:
        assert self.link is not None
        self.link.cut()
        return _wait(lambda: not self.author_seen(2), 15.0, 0.05)

    def heal(self) -> bool:
        assert self.link is not None
        self.link.heal()
        return _wait(lambda: self.author_seen(2), 30.0, 0.1)

    def close(self) -> None:
        from . import live

        for p in self.parties:
            try:
                p.kill()
            except Exception:  # noqa: BLE001 - closing anyway
                pass
        for x in self.extra:
            try:
                x.close()
            except Exception:  # noqa: BLE001
                pass
        for e in self.entities:
            try:
                e.undeclare()
            except Exception:  # noqa: BLE001
                pass
        for s in {id(x): x for x in (self.s, self.g1, self.g2, self.s2, self.x) if x is not None}.values():
            live.close(s)
        if self.link is not None:
            self.link.close()
        for r in (self.r2, self.r1):
            if r is not None:
                try:
                    r.close()
                except Exception:  # noqa: BLE001
                    pass


def n0(a: tuple[int, int, bool] | None) -> int | None:
    return None if a is None else a[0]


def n_acts(p: Party) -> int | None:
    """The acts a target made so far (``desired acts``), its seeds aside."""
    return n0(p.acts())


def zid_of(stamp: str | None) -> int | None:
    return None if stamp is None else int(stamp.split("/", 1)[1], 16)


def converged_on(rep: Any, stamp: str | None, rev: int, last_known: bool = False) -> bool:
    ver = {"stamp": stamp, "revision": rev}
    want = {"key": D, "source": "desired", "in_force": ver, "observed": ver, "outcome": "converged"}
    if last_known:
        want["last_known"] = True
    return rep == want


class Ctx:
    def __init__(self, report: Report, author: Any, target: Any, watch: bool) -> None:
        self.report, self.author_impl, self.target_impl, self.watch = report, author, target, watch

    def target_flags(self, *extra: str) -> list[str]:
        return ["--desired-target", AUTHOR, *(["--desired-watch"] if self.watch else []), *extra]

    def name(self, sec: str) -> str:
        a = "zk2py" if self.author_impl == "zk2py" else "rust"
        t = "zk2py" if self.target_impl == "zk2py" else "rust"
        return f"{sec} [{a} author, {t} target]"


def _converge_v1(ctx: Ctx, bus: Bus, author: Party, target: Party, n: int = 1) -> str | None:
    stamp = author.put(TARGET, v(n))
    _wait(lambda: converged_on(target.report(), stamp, n), 10.0, 0.1)
    return stamp


def section1(ctx: Ctx) -> None:
    sec, rp = ctx.name("§1 converge after an outage"), ctx.report
    bus = Bus(cut=True)
    try:
        author = bus.party(ctx.author_impl, AUTHOR, ["--desired-author", "--tokenless", "tasking.v1"], on=1)
        from . import live

        d = bus.descriptor(author.instance or "")
        alive = live.list_presence(bus.g1, f"zk2/{AUTHOR}/@zk/alive/**")
        rp.check(sec, "setup: the author marks tasking.v1 \"token\": false and holds no alive/tasking.v1 token",
                 d is not None and [x.get("token") for x in d["interfaces"]] == [False] and not alive.alive)
        t0 = time.monotonic()
        target = bus.party(ctx.target_impl, TARGET, ctx.target_flags(), on=2)
        key = None
        _wait(lambda: bus.token_put(TARGET, t0) is not None, 10.0, 0.01)
        got, stamp = bus.get_json(RP)
        run, _ = bus.get_json(RUN)
        rp.check(sec, "1. G's GET at the target's instance token holds {key: D, source: baseline}; Run holds the "
                      "baseline", got == {"key": D, "source": "baseline"} and run == BASELINE, f"{got}, {run}")
        t1 = time.monotonic()
        s1 = author.put(TARGET, v(1))
        meta = (d or {}).get("meta", {}).get("zid")
        ok = _wait(lambda: any(g[1] == D for g in bus.since(t1)), 5.0)
        dput = next((g for g in bus.since(t1, D)), None)
        rp.check(sec, "2. S receives a put on D stamped by the author (core S1: its id the author's meta.zid)",
                 ok and dput is not None and dput[4] == s1 and meta is not None and zid_of(s1) == int(meta, 16),
                 f"{dput and dput[4]}, meta.zid {meta}")
        ok = _wait(lambda: any(g[1] == RP and converged_on(json.loads(g[3]), s1, 1) for g in bus.since(t1, RP)), 10.0)
        rp.check(sec, "2. then on Rp a report converged on v1", ok and bus.get_json(RUN)[0] == v(1))
        acts0 = target.acts()
        rep0 = target.report()
        rp.check(sec, "3. the link cut", bus.cut())
        t3 = time.monotonic()
        s2 = author.put(TARGET, v(2))
        time.sleep(2.0)
        rp.check(sec, "3. through S2: Rp and Run receive nothing during the cut; the target acted on nothing, and its "
                      "report is unchanged",
                 not bus.since2(t3) and n_acts(target) == n0(acts0) and target.report() == rep0,
                 f"S2 {len(bus.since2(t3))}, acts {acts0} -> {target.acts()}")
        t4 = time.monotonic()
        bus.heal()
        ok = _wait(lambda: converged_on(target.report(), s2, 2), 10.0, 0.05)
        took = time.monotonic() - t4
        a = target.acts()
        rp.check(sec, "4. after the heal the target seeds and acts on v2: converged on v2 within 5 s, acted on once",
                 ok and took <= 5.0 and a is not None and acts0 is not None and a[0] == acts0[0] + 1
                 and bus.get_json(RUN, on=2)[0] == v(2), f"{took:.2f} s, acts {acts0} -> {a}")
        rp.note(sec, f"converged {took:.2f} s after the heal (spike S12: 1.4 to 1.6 s)")
        del key
    finally:
        bus.close()


def section2(ctx: Ctx) -> None:
    import zenoh

    sec, rp = ctx.name("§2 never an older version"), ctx.report
    bus = Bus()
    try:
        author = bus.party(ctx.author_impl, AUTHOR, ["--desired-author"])
        target = bus.party(ctx.target_impl, TARGET, ctx.target_flags())
        _converge_v1(ctx, bus, author, target, 1)
        t = time.monotonic()
        s2 = author.put(TARGET, v(2))
        ok = _wait(lambda: converged_on(target.report(), s2, 2), 10.0, 0.05)
        ts2 = next((g[5] for g in bus.since(t, D) if g[4] == s2), None)
        rp.check(sec, "setup: the target converged on v2, and S holds s2's timestamp", ok and ts2 is not None)
        acts0 = target.acts()
        t1 = time.monotonic()
        n = ts2.get_time_as_ntp64()
        earlier = zenoh.Timestamp(zenoh.NTP64(n.as_secs() - 1, n.subsec_nanos()), ts2.get_id())
        bus.x.put(D, json.dumps(v(1)).encode(), encoding="application/json", timestamp=earlier)
        time.sleep(1.0)
        got, _ = bus.get_json(RP)
        rp.check(sec, "1. X's v1 under s2's id, 1 s earlier: older; nothing acted, Rp and Run receive nothing, a GET "
                      "of Rp holds converged on v2",
                 n_acts(target) == n0(acts0) and not bus.since(t1, RP) and not bus.since(t1, RUN) and converged_on(got, s2, 2),
                 f"acts {acts0} -> {target.acts()}, {len(bus.since(t1, RP))} Rp puts")
        t2 = time.monotonic()
        bus.x.put(D, json.dumps(v(3)).encode(), encoding="application/json", timestamp=ts2)
        time.sleep(1.0)
        rp.check(sec, "2. X's v3 under s2 exactly: the same version, nothing",
                 n_acts(target) == n0(acts0) and not bus.since(t2, RP) and not bus.since(t2, RUN),
                 f"acts {acts0} -> {target.acts()}")
        t3 = time.monotonic()
        rr = target.cmd("desired reread 2000")
        time.sleep(3.0)
        a = target.acts()
        target.cmd("desired reread off")
        g = bus.get(D)
        rp.check(sec, "3. the target re-reads on a 2 s period: the author answers v2 under s2, the same version, "
                      "nothing acted, nothing put on Rp",
                 rr == "desired reread 2000" and a is not None and acts0 is not None and a[1] > acts0[1]
                 and a[0] == acts0[0] and [x.stamp for x in g.replies] == [s2] and not bus.since(t3, RP),
                 f"acts {acts0} -> {a}, GET {[x.stamp for x in g.replies]}")
        rp.note(sec, "X builds every timestamp from the one S received with s2 (0.4, F-196): s2 exactly is that "
                     "timestamp, and 1 s earlier its seconds less one, its nanoseconds kept")
    finally:
        bus.close()


def section3(ctx: Ctx) -> None:
    sec, rp = ctx.name("§3 a delete reverts, within the window"), ctx.report
    bus = Bus(cut=True)
    try:
        author = bus.party(ctx.author_impl, AUTHOR, ["--desired-author", "--desired-window", "10"], on=1)
        target = bus.party(ctx.target_impl, TARGET, ctx.target_flags(), on=2)
        s1 = _converge_v1(ctx, bus, author, target, 1)
        rp.check(sec, "setup: the target converged on v1", converged_on(target.report(), s1, 1))
        sd = author.delete(TARGET)
        reverted = {"key": D, "source": "baseline", "observed": {"stamp": sd}, "outcome": "reverted"}
        ok = _wait(lambda: target.report() == reverted, 10.0, 0.05)
        rp.check(sec, "1. the author deletes D: the target reverts, Run holds the baseline",
                 ok and bus.get_json(RUN, on=2)[0] == BASELINE, str(target.report()))
        s2 = author.put(TARGET, v(2))
        rp.check(sec, "2. the author puts v2: converged on v2", _wait(lambda: converged_on(target.report(), s2, 2), 10.0))
        bus.cut()
        sd3 = author.delete(TARGET)
        time.sleep(3.0)
        bus.heal()
        want = {"key": D, "source": "baseline", "observed": {"stamp": sd3}, "outcome": "reverted"}
        ok = _wait(lambda: target.report() == want, 10.0, 0.05)
        rp.check(sec, "3. a delete during a 3 s cut: the seed after the heal holds its reply_del, and the target "
                      "reverts", ok and bus.get_json(RUN, on=2)[0] == BASELINE, str(target.report()))
        s3 = author.put(TARGET, v(3))
        ok = _wait(lambda: converged_on(target.report(), s3, 3), 10.0)
        acts0 = target.acts()
        bus.cut()
        author.delete(TARGET)
        time.sleep(15.0)
        bus.heal()
        time.sleep(3.0)
        a = target.acts()
        rp.check(sec, "4. a delete during a 15 s cut, past the 10 s window: the seed draws nothing for D, and the "
                      "target keeps v3 in force, converged on v3",
                 ok and converged_on(target.report(), s3, 3) and a is not None and acts0 is not None
                 and a[0] == acts0[0] and a[1] > acts0[1] and not bus.get(D, on=2).replies,
                 f"acts {acts0} -> {a}, {target.report()}")
    finally:
        bus.close()


def section4(ctx: Ctx) -> None:
    sec, rp = ctx.name("§4 a refused document keeps the previous in force"), ctx.report
    bus = Bus()
    try:
        author = bus.party(ctx.author_impl, AUTHOR, ["--desired-author"])
        target = bus.party(ctx.target_impl, TARGET, ctx.target_flags())
        s1 = _converge_v1(ctx, bus, author, target, 1)
        v1 = {"stamp": s1, "revision": 1}
        s2 = author.put(TARGET, v(2, endpoint="tcp/10.0.0.9:7447"))

        def refused(rep: Any, stamp: str | None, rev: int, code: str, pointer: str) -> bool:
            return isinstance(rep, dict) and {k: rep.get(k) for k in ("key", "source", "in_force", "observed",
                                                                       "outcome")} == {
                "key": D, "source": "desired", "in_force": v1, "observed": {"stamp": stamp, "revision": rev},
                "outcome": "refused"} and {k: rep.get("refusal", {}).get(k) for k in ("code", "pointer")} == {
                "code": code, "pointer": pointer}

        ok = _wait(lambda: refused(target.report(), s2, 2, "reach", "/endpoint"), 10.0, 0.05)
        rp.check(sec, "1. v2 with an endpoint: refused reach at /endpoint, v1 in force, Run v1",
                 ok and bus.get_json(RUN)[0] == v(1), str(target.report()))
        s3 = author.put(TARGET, v(3, rate_hz=500))
        ok = _wait(lambda: refused(target.report(), s3, 3, "invalid", "/rate_hz"), 10.0, 0.05)
        rp.check(sec, "2. v3 at rate_hz 500: refused invalid at /rate_hz", ok, str(target.report()))
        acts0 = target.acts()
        t = time.monotonic()
        ne = author.cmd("desired new-epoch")
        ok = _wait(lambda: any(g[2] == "put" and g[4] != s3 for g in bus.since(t, D)), 5.0)
        reput = next((g[4] for g in bus.since(t, D) if g[2] == "put"), None)
        same_id = reput is not None and s3 is not None and zid_of(reput) == zid_of(s3) and \
            int(reput.split("/")[0]) > int(s3.split("/")[0])
        ok = ok and _wait(lambda: refused(target.report(), reput, 3, "invalid", "/rate_hz"), 10.0, 0.05)
        time.sleep(0.5)
        rp.check(sec, "3. a re-mint in its own session puts v3 again, the same id, a later time: adopted refused, "
                      "nothing applied, observed the re-put, in_force v1",
                 ne is not None and ne.startswith("desired new-epoch ") and same_id and ok
                 and target.acts() is not None and acts0 is not None and target.acts()[0] == acts0[0]
                 and bus.get_json(RUN)[0] == v(1), f"{ne}, re-put {reput}, {target.report()}")
        s4 = author.put(TARGET, v(4))
        rp.check(sec, "4. v4: converged on v4", _wait(lambda: converged_on(target.report(), s4, 4), 10.0, 0.05)
                 and bus.get_json(RUN)[0] == v(4))
        target.cmd("desired fail-next")
        s5 = author.put(TARGET, v(5, rate_hz=20, mode="halt"))

        def failed(rep: Any) -> bool:
            return isinstance(rep, dict) and rep.get("outcome") == "failed" and rep.get("source") == "desired" and \
                rep.get("observed") == {"stamp": s5, "revision": 5} and "in_force" not in rep and \
                rep.get("refusal", {}).get("code") == "apply"

        ok = _wait(lambda: failed(target.report()), 10.0, 0.05)
        run, _ = bus.get_json(RUN)
        rp.check(sec, "5. the drive fails v5 after setting the rate: Run holds rate_hz 20 and mode run, the report "
                      "failed apply with no in_force",
                 ok and isinstance(run, dict) and run.get("rate_hz") == 20 and run.get("mode") == "run",
                 f"{target.report()}, Run {run}")
    finally:
        bus.close()


def section5(ctx: Ctx) -> None:
    sec, rp = ctx.name("§5 a restart acts again"), ctx.report
    bus = Bus()
    try:
        author = bus.party(ctx.author_impl, AUTHOR, ["--desired-author"])
        target = bus.party(ctx.target_impl, TARGET, ctx.target_flags())
        s1 = _converge_v1(ctx, bus, author, target, 1)
        rp.check(sec, "setup: the target converged on v1", converged_on(target.report(), s1, 1))
        target.kill()
        t = time.monotonic()
        target2 = bus.party(ctx.target_impl, TARGET, ctx.target_flags("--desired-hold"))
        tok = None
        _wait(lambda: bus.token_put(TARGET, t) is not None, 10.0, 0.01)
        got, stamp = bus.get_json(RP)
        d = bus.descriptor(target2.instance or "")
        zid = None if d is None else d.get("meta", {}).get("zid")
        rp.check(sec, "1. G's GET at the new instance token holds {key: D, source: baseline} under the new "
                      "instance's stamp",
                 got == {"key": D, "source": "baseline"} and zid is not None and zid_of(stamp) == int(zid, 16),
                 f"{got} {stamp}, meta.zid {zid}")
        t1 = time.monotonic()
        rel = target2.cmd("desired release")
        ok = _wait(lambda: converged_on(target2.report(), s1, 1), 10.0, 0.05)

        def runs_v1() -> list[Any]:
            return [g for g in bus.since(t1, RUN) if g[2] == "put" and json.loads(g[3]) == v(1)]

        _wait(lambda: bool(runs_v1()), 5.0)  # S's delivery can follow the report's answer
        runs = runs_v1()
        rp.check(sec, "1. released, the target acts on v1 again: converged on v1 under s1, and S receives a put on Run "
                      "holding v1 from the new instance",
                 rel == "desired release" and ok and bool(runs) and zid is not None and zid_of(runs[0][4]) == int(zid, 16),
                 f"{target2.report()}, Run puts {[g[4] for g in runs]}")
        del tok
    finally:
        bus.close()


def section6(ctx: Ctx) -> None:
    sec, rp = ctx.name("§6 the author's restart moves no target to act again"), ctx.report
    bus = Bus()
    rec = Path(tempfile.mkdtemp(prefix="zk2py-desired-")) / "record.json"
    try:
        flags = ["--desired-author", "--desired-record", str(rec)]
        author = bus.party(ctx.author_impl, AUTHOR, flags)
        target = bus.party(ctx.target_impl, TARGET, ctx.target_flags())
        s1 = _converge_v1(ctx, bus, author, target, 1)
        author.put(SPARE, v(7))
        author.delete(SPARE)
        time.sleep(5.0)
        rp.check(sec, "setup: the target converged on v1; D2's document deleted 5 s ago, in the record",
                 converged_on(target.report(), s1, 1) and rec.is_file() and "lab/spare" in
                 json.loads(rec.read_text()).get("deletes", {}))
        acts0 = target.acts()
        author.kill()
        t = time.monotonic()
        author2 = bus.party(ctx.author_impl, AUTHOR, flags)
        _wait(lambda: bus.token_put(AUTHOR, t) is not None, 10.0, 0.01)
        gd, gd2 = bus.get(D), bus.get(D2)
        time.sleep(3.0)
        d = bus.descriptor(author2.instance or "")
        zid = None if d is None else int(d["meta"]["zid"], 16)
        reput = next((g for g in bus.since(t, D) if g[2] == "put"), None)
        redel = next((g for g in bus.since(t, D2) if g[2] == "delete"), None)
        rp.check(sec, "1. S receives a put on D holding v1's content under the new zid, and a delete of D2 under the "
                      "same id",
                 reput is not None and json.loads(reput[3]) == v(1) and zid_of(reput[4]) == zid
                 and zid_of(s1) != zid and redel is not None and zid_of(redel[4]) == zid,
                 f"put {reput and reput[4]}, delete {redel and redel[4]}, zid {zid and hex(zid)}")
        token_at = next((x[0] for x in bus.tokens if x[0] >= t and x[1] == "put" and x[2].startswith(
            f"zk2/{AUTHOR}/@zk/instance/")), None)
        rp.note(sec, f"S's receipt of the re-put {'before' if reput and token_at and reput[0] <= token_at else 'after'} "
                     "the new instance token (a SHOULD; S's order across data and tokens is not zenoh's to keep)")
        rp.check(sec, "1. G's GET at the new instance token holds that put for D and a reply_del for D2, each stamped "
                      "by the new instance, whose meta.zid is their id (the MUST)",
                 [(x.deleted, zid_of(x.stamp)) for x in gd.replies] == [(False, zid)]
                 and [(x.deleted, zid_of(x.stamp)) for x in gd2.replies] == [(True, zid)],
                 f"D {[(x.deleted, x.stamp) for x in gd.replies]}, D2 {[(x.deleted, x.stamp) for x in gd2.replies]}")
        rep = target.report()
        rp.check(sec, "1. the target adopts the re-put: Run receives nothing, its report observed and in_force the "
                      "re-put's stamp, revision 1, converged; it acted on nothing",
                 reput is not None and converged_on(rep, reput[4], 1) and not [g for g in bus.since(t, RUN)]
                 and target.acts() is not None and acts0 is not None and target.acts()[0] == acts0[0], str(rep))
        s2 = author2.put(TARGET, v(2))
        ok = _wait(lambda: converged_on(target.report(), s2, 2), 10.0, 0.05)
        rp.check(sec, "2. v2, later than the re-put's stamp: converged on v2",
                 ok and reput is not None and zid_of(s2) == zid_of(reput[4]) and raw(s2) > raw(reput[4]))
        acts1 = target.acts()
        author2.kill()
        rec.unlink()
        t3 = time.monotonic()
        author3 = bus.party(ctx.author_impl, AUTHOR, flags)
        author3.put(TARGET, v(2))
        ok = _wait(lambda: any(g[2] == "put" for g in bus.since(t3, D)), 5.0)
        reput3 = next((g for g in bus.since(t3, D) if g[2] == "put"), None)
        ok = ok and reput3 is not None and _wait(lambda: converged_on(target.report(), reput3[4], 2), 10.0, 0.05)
        rp.check(sec, "3. with no record, a new epoch: v2's content again under a new zid, adopted; Run nothing; "
                      "a GET of D2 draws nothing",
                 ok and reput3 is not None and zid_of(reput3[4]) not in (zid, zid_of(s1)) and not bus.since(t3, RUN)
                 and n_acts(target) == n0(acts1) and not bus.get(D2).replies, f"{reput3 and reput3[4]}, {target.report()}")
        s4 = author3.put(TARGET, v(3))
        rp.check(sec, "4. v3: converged on v3", _wait(lambda: converged_on(target.report(), s4, 3), 10.0, 0.05))
    finally:
        bus.close()


def raw(stamp: str | None) -> int:
    return -1 if stamp is None else int(stamp.split("/", 1)[0])


def _archive(bus: Bus, on: int) -> Any:
    from .desired_live import contract
    from .health_scenarios import StandInArchive

    c = contract("tasking.v1")
    r = next(r for r in c.canonical["resources"] if r["template"] == "tasks/{system}/{service}")
    a = StandInArchive(bus.ep1 if on == 1 else bus.ep2, "lab/archive", "zk2/lab/author/tasking.v1/state/**",
                       {"iface": "tasking.v1", "contract": c.fingerprint, "type": r["type"]})
    bus.extra.append(a)
    return a


def section7(ctx: Ctx) -> None:
    sec, rp = ctx.name("§7 last-known from an archive, the author absent"), ctx.report
    bus = Bus()
    rec = Path(tempfile.mkdtemp(prefix="zk2py-desired-")) / "record.json"
    try:
        from . import live

        archive = _archive(bus, 1)
        flags = ["--desired-author", "--desired-record", str(rec)]
        author = bus.party(ctx.author_impl, AUTHOR, flags)
        tflags = ctx.target_flags("--desired-archive", "lab/archive")
        target = bus.party(ctx.target_impl, TARGET, tflags)
        s1 = _converge_v1(ctx, bus, author, target, 1)
        ok = _wait(lambda: D in archive.held, 3.0)
        rp.check(sec, "setup: the target converged on v1, and the archive recorded it",
                 converged_on(target.report(), s1, 1) and ok)
        author.close()
        target.kill()
        _wait(lambda: not bus.author_seen(1), 15.0, 0.1)
        target2 = bus.party(ctx.target_impl, TARGET, tflags)
        ok = _wait(lambda: converged_on(target2.report(), s1, 1, last_known=True), 10.0, 0.05)
        p = live.list_presence(bus.g1, f"zk2/{AUTHOR}/@zk/instance/*")
        rp.check(sec, "1. a complete presence read shows no author token, its GET draws no reply; the target reads "
                      "D's archive form, acts on v1 under s1, converged with last_known: true",
                 ok and p.kind == live.COMPLETE and not p.instances and bus.get(D).silent, str(target2.report()))
        acts0 = target2.acts()
        t = time.monotonic()
        author2 = bus.party(ctx.author_impl, AUTHOR, flags)
        ok = _wait(lambda: any(g[2] == "put" for g in bus.since(t, D)), 5.0)
        reput = next((g for g in bus.since(t, D) if g[2] == "put"), None)
        ok = ok and reput is not None and _wait(lambda: converged_on(target2.report(), reput[4], 1), 10.0, 0.05)
        time.sleep(0.5)
        rp.check(sec, "2. the restarted author's re-put under its new zid: adopted, Run nothing, observed and "
                      "in_force the re-put's stamp, revision 1, no last_known",
                 ok and reput is not None and zid_of(reput[4]) != zid_of(s1) and not bus.since(t, RUN)
                 and target2.acts() is not None and acts0 is not None and target2.acts()[0] == acts0[0],
                 f"{reput and reput[4]}, {target2.report()}")
        del author2
    finally:
        bus.close()


def section8(ctx: Ctx) -> None:
    sec, rp = ctx.name("§8 the kill switch"), ctx.report
    bus = Bus()
    try:
        author = bus.party(ctx.author_impl, AUTHOR, ["--desired-author"])
        author.put(TARGET, v(1))
        q0 = author.cmd(f"desired gets {TARGET}")
        t = time.monotonic()
        target = bus.party(ctx.target_impl, TARGET, ["--desired-unbound"])
        _wait(lambda: bus.token_put(TARGET, t) is not None, 10.0, 0.01)
        d = bus.descriptor(target.instance or "")
        got, _ = bus.get_json(RP)
        req = [] if d is None else d.get("requires", [])
        rp.check(sec, "1. the descriptor lists task with \"bindings\": [] and \"params\": {}",
                 [(r.get("role"), r.get("bindings"), r.get("params")) for r in req] == [("task", [], {})], str(req))
        rp.check(sec, "1. Rp holds {source: baseline}, with no key", got == {"source": "baseline"}, str(got))
        t1 = time.monotonic()
        time.sleep(3.0)
        a = target.acts()
        rp.check(sec, "1. Run holds the baseline, and stays so; the target acts on nothing and makes no seed",
                 bus.get_json(RUN)[0] == BASELINE and not bus.since(t1, RUN) and a is not None and a[0] == 0 and a[1] == 0,
                 f"acts {a}")
        q1 = author.cmd(f"desired gets {TARGET}")
        rp.check(sec, "1. the author's queryable receives no GET of D (desired gets lab/worker, 0.4)",
                 q0 is not None and q0.startswith(f"desired gets {D} ") and q0 == q1, f"{q0} -> {q1}")
    finally:
        bus.close()


def section10(ctx: Ctx) -> None:
    sec, rp = ctx.name("§10 last-known confirmed by its author"), ctx.report
    bus = Bus(cut=True)
    rec = Path(tempfile.mkdtemp(prefix="zk2py-desired-")) / "record.json"
    try:
        from . import live

        archive = _archive(bus, 2)
        author = bus.party(ctx.author_impl, AUTHOR, ["--desired-author", "--desired-record", str(rec)], on=1)
        tflags = ctx.target_flags("--desired-archive", "lab/archive")
        target = bus.party(ctx.target_impl, TARGET, tflags, on=2)
        s1 = _converge_v1(ctx, bus, author, target, 1)
        ok = _wait(lambda: D in archive.held, 3.0)
        rp.check(sec, "setup: the target converged on v1, and the archive on R2 recorded it",
                 converged_on(target.report(), s1, 1) and ok)
        bus.cut()
        target.kill()
        t1 = time.monotonic()
        target2 = bus.party(ctx.target_impl, TARGET, tflags, on=2)
        ok = _wait(lambda: converged_on(target2.report(), s1, 1, last_known=True), 10.0, 0.05)
        p = live.list_presence(bus.g2, f"zk2/{AUTHOR}/@zk/instance/*")
        s2rep = [json.loads(g[3]) for g in bus.since2(t1, RP)]
        rp.check(sec, "1. through R2 a complete read shows no author token, the GET draws no reply; the target reads "
                      "the archive, acts on v1 under s1, and, read through S2, converged with last_known: true",
                 ok and p.kind == live.COMPLETE and not p.instances and bus.get(D, on=2).silent
                 and any(converged_on(r, s1, 1, last_known=True) for r in s2rep), str(target2.report()))
        acts0 = target2.acts()
        t2 = time.monotonic()
        bus.heal()
        ok = _wait(lambda: converged_on(target2.report(), s1, 1), 10.0, 0.05)
        time.sleep(0.5)
        s2rep = [json.loads(g[3]) for g in bus.since2(t2, RP)]
        rp.check(sec, "2. after the heal the author answers v1 under s1, its own stamp: confirmed; nothing acted, Run "
                      "nothing, the report put again without last_known, observed and in_force still s1",
                 ok and target2.acts() is not None and acts0 is not None and target2.acts()[0] == acts0[0]
                 and not bus.since2(t2, RUN) and any(converged_on(r, s1, 1) for r in s2rep),
                 f"{target2.report()}, acts {acts0} -> {target2.acts()}, S2 {s2rep}")
        # 3 (0.4, F-197): the author present on R2 by X's token alone, its
        # GET silent there, the archive holding v1, which v2 replaced
        bus.cut()
        s2 = author.put(TARGET, v(2))
        x = live.open_client(bus.ep2)
        bus.extra.append(x)
        xtok = x.liveliness().declare_token(f"zk2/{AUTHOR}/@zk/instance/" + "f" * 16)
        _wait(lambda: bool(live.list_presence(bus.g2, f"zk2/{AUTHOR}/@zk/instance/*").instances), 5.0, 0.05)
        target2.kill()
        t3 = time.monotonic()
        target3 = bus.party(ctx.target_impl, TARGET, tflags, on=2)
        time.sleep(4.0)
        seeds = target3.cmd("desired seeds") or ""
        rep3 = target3.report()
        s2rep = [json.loads(g[3]) for g in bus.since2(t3, RP)]
        words = dict(w.split("=", 1) for w in seeds.split()[3:] if "=" in w)
        rp.check(sec, "3. the author present on R2 (X's token) and its GET silent: the target reads no archive and "
                      "acts on nothing; its report stays {key: D, source: baseline}, Run the baseline, and it says so "
                      "(desired seeds: no reply, the author present, no archive read)",
                 rep3 == {"key": D, "source": "baseline"} and n_acts(target3) == 0
                 and all(r == {"key": D, "source": "baseline"} for r in s2rep)
                 and bus.get_json(RUN, on=2)[0] == BASELINE
                 and words.get("answered") == "0" and words.get("author") == "present" and words.get("archive") == "-",
                 f"seeds {seeds!r}, report {rep3}, acts {target3.acts()}")
        # 4: the heal; once the author's own token is on R2, X withdraws
        t4 = time.monotonic()
        bus.heal()
        _wait(lambda: len(live.list_presence(bus.g2, f"zk2/{AUTHOR}/@zk/instance/*").instances) >= 2, 10.0, 0.05)
        xtok.undeclare()
        ok = _wait(lambda: converged_on(target3.report(), s2, 2), 10.0, 0.05)
        time.sleep(0.5)
        rep4 = target3.report()
        rp.check(sec, "4. the author's instance token appears on R2: the target seeds, the author answers v2 under "
                      "s2, and the target acts on it: converged on v2, without last_known; X's withdrawal moves "
                      "nothing", ok and isinstance(rep4, dict) and "last_known" not in rep4 and n_acts(target3) == 1,
                 f"{rep4}, acts {target3.acts()}, {round(time.monotonic() - t4, 2)} s after the heal")
    finally:
        bus.close()


ZENCTL = Path(os.environ.get("ZK2PY_ZENCTL", "/srv/dev/repos/zenkey-wt/.s840/bin/zenctl-dd"))
#: §9's Expected, per step: (converged, refused, last_known, in_force),
#: each (answer, reason).
S9_EXPECT = {
    1: {"converged": ("yes", "converged"), "refused": ("no", "converged"), "last_known": ("no", "current"),
        "in_force": ("yes", "desired")},
    2: {"converged": ("no", "refused"), "refused": ("yes", "refused"), "last_known": ("no", "current"),
        "in_force": ("yes", "desired")},
    3: {"converged": ("no", "failed"), "refused": ("yes", "failed"), "last_known": ("no", "current"),
        "in_force": ("yes", "partial")},
    4: {"converged": ("not_asked", "author_absent"), "refused": ("no", "converged"),
        "last_known": ("yes", "last_known"), "in_force": ("yes", "desired")},
    5: {"converged": ("not_asked", "unbound"), "refused": ("not_asked", "unbound"),
        "last_known": ("not_asked", "unbound"), "in_force": ("no", "unbound")},
    6: {"converged": ("not_yet", "behind"), "refused": ("no", "converged"), "last_known": ("no", "current"),
        "in_force": ("yes", "desired")},
}


def zenctl(args: list[str], timeout: float = 90.0) -> tuple[int | None, Any, str]:
    """The Rust tool as a black box: (exit, its JSON, stderr)."""
    env = dict(os.environ, ZENKEY_EXPLORER_CONFIG_DIR=tempfile.mkdtemp(prefix="zk2py-zenctl-"))
    try:
        p = subprocess.run([str(ZENCTL), *args, "--format", "json"], capture_output=True, text=True, env=env,
                           timeout=timeout)
    except subprocess.TimeoutExpired:
        return None, None, "timed out"
    try:
        doc = json.loads(p.stdout)
    except ValueError:
        doc = None
    return p.returncode, doc, p.stderr


def _z_row(doc: Any, role: str = "task") -> dict[str, Any] | None:
    for r in (doc or {}).get("rows") or []:
        if r.get("row") == "desired" and r.get("role") == role:
            return r
    return None


def _z_answers(row: dict[str, Any] | None) -> dict[str, tuple[str, str]] | None:
    if row is None:
        return None
    return {q: (a.get("answer"), a.get("reason")) for q, a in (row.get("answers") or {}).items()}


def _z_check(doc: Any, check: str) -> tuple[str | None, list[Any]]:
    for r in (doc or {}).get("rows") or []:
        if r.get("check") == check:
            return (r.get("verdict") or {}).get("answer"), r.get("findings") or []
    return None, []


def _z_case(doc: Any, case: str) -> list[str | None]:
    return [(r.get("verdict") or {}).get("answer") for r in (doc or {}).get("rows") or [] if r.get("case") == case]


def section9(ctx: Ctx) -> None:
    """§9 (0.4): a tool's questions, by zk2py's tool (:mod:`zk2py.desired_tool`)
    and the Rust one (``zenctl desired``, the doctor's ``desired-*`` checks,
    ``check conform``'s ``desired-*`` cases) as a black box, each against
    §9's Expected and against each other."""
    from . import desired_tool as dt, live
    from .contract import load_contract

    sec, rp = ctx.name("§9 a tool's questions"), ctx.report
    if not ZENCTL.is_file():
        rp.note(sec, f"{ZENCTL} not found: the Rust tool is not run")
    disagree: list[str] = []

    def ask(bus: Bus, step: int, extra: list[str] = ()) -> tuple[Any, Any, Any]:
        mine = dt.read(bus.g1, TARGET, "task")
        rc, doc, err = zenctl(["desired", TARGET, "-c", bus.ep1, "--timeout", "1", *extra]) if ZENCTL.is_file() \
            else (None, None, "")
        z = _z_answers(_z_row(doc))
        m = mine[0].pairs() if mine else None
        want = S9_EXPECT[step]
        rp.check(sec, f"{step}. zk2py's tool answers as §9 Expected {step}", m == want, f"{m}")
        if ZENCTL.is_file():
            rp.check(sec, f"{step}. zenctl desired answers as §9 Expected {step}", z == want, f"{z}, exit {rc}")
            if z != m:
                disagree.append(f"step {step}: zk2py {m}, zenctl {z}")
        return mine[0] if mine else None, doc, rc

    def doctor(bus: Bus, extra: list[str] = ()) -> Any:
        rc, doc, _ = zenctl(["doctor", "-c", bus.ep1, "--timeout", "1", "--check", "desired-refused", "--check",
                             "desired-behind", "--check", "desired-last-known", *extra])
        return rc, doc

    # 1–3 and 6 on §1's bus
    bus = Bus(cut=True)
    try:
        author = bus.party(ctx.author_impl, AUTHOR, ["--desired-author", "--tokenless", "tasking.v1"], on=1)
        target = bus.party(ctx.target_impl, TARGET, ctx.target_flags(), on=2)
        _converge_v1(ctx, bus, author, target, 1)
        bus.cut()
        s2 = author.put(TARGET, v(2))
        time.sleep(1.0)
        bus.heal()
        _wait(lambda: converged_on(target.report(), s2, 2), 10.0, 0.05)
        row, doc, rc = ask(bus, 1)
        rp.check(sec, "1. zenctl desired exits 0: every role asked converged on its author's current document",
                 not ZENCTL.is_file() or rc == 0, f"exit {rc}")
        rp.check(sec, "1. both tools take the slice from the author's contract",
                 row is not None and row.slice == D and row.slice_from == "author_contract"
                 and (not ZENCTL.is_file() or (_z_row(doc) or {}).get("slice_from") == "author_contract"),
                 f"zk2py {row and row.slice_from}, zenctl {(_z_row(doc) or {}).get('slice_from')}")
        if ZENCTL.is_file():
            rc, cdoc, _ = zenctl(["check", "conform", TARGET, "worker.v1", "-c", bus.ep1, "--timeout", "1",
                                  "--for", "2"])
            rp.check(sec, "check conform on lab/worker: desired-report and desired-binding pass",
                     _z_case(cdoc, "desired-report") == ["not_established"]
                     and _z_case(cdoc, "desired-binding") == ["not_established"],
                     f"desired-report {_z_case(cdoc, 'desired-report')}, desired-binding "
                     f"{_z_case(cdoc, 'desired-binding')}")
            rc, adoc, _ = zenctl(["check", "conform", AUTHOR, "tasking.v1", "-c", bus.ep1, "--timeout", "1",
                                  "--for", "2"])
            rp.check(sec, "check conform on lab/author: desired-author passes",
                     _z_case(adoc, "desired-author") == ["not_established"],
                     f"desired-author {_z_case(adoc, 'desired-author')}")
            drc, ddoc = doctor(bus)
            rp.check(sec, "1. the doctor: no desired-refused, no desired-behind, no desired-last-known",
                     [_z_check(ddoc, c)[0] for c in ("desired-refused", "desired-behind", "desired-last-known")]
                     == ["not_established"] * 3, f"{[_z_check(ddoc, c)[0] for c in ('desired-refused', 'desired-behind', 'desired-last-known')]}")
        s3 = author.put(TARGET, v(3, endpoint="tcp/10.0.0.9:7447"))
        _wait(lambda: (target.report() or {}).get("outcome") == "refused", 10.0, 0.05)
        ask(bus, 2)
        if ZENCTL.is_file():
            drc, ddoc = doctor(bus)
            ans, finds = _z_check(ddoc, "desired-refused")
            rp.check(sec, "2. the doctor's desired-refused finds the target's refused act", ans == "established"
                     and any("lab/worker" in str(f.get("subject")) for f in finds), f"{ans}, {finds}")
        target.cmd("desired fail-next")
        author.put(TARGET, v(4, rate_hz=20, mode="halt"))
        _wait(lambda: (target.report() or {}).get("outcome") == "failed", 10.0, 0.05)
        ask(bus, 3)
        if ZENCTL.is_file():
            drc, ddoc = doctor(bus)
            rp.check(sec, "3. the doctor's desired-refused finds the failed act",
                     _z_check(ddoc, "desired-refused")[0] == "established", f"{_z_check(ddoc, 'desired-refused')}")
        # 6: v5 converged, the next act held, v6 put
        s5 = author.put(TARGET, v(5))
        _wait(lambda: converged_on(target.report(), s5, 5), 10.0, 0.05)
        target.cmd("desired hold")
        s6 = author.put(TARGET, v(6))
        time.sleep(0.5)
        row, doc, rc = ask(bus, 6)
        rp.check(sec, "6. once: the report's observed v5's stamp, the author's v6's; no verdict (a reading alone "
                      "never finds a target behind)",
                 row is not None and (row.reading.report.report or {}).get("observed", {}).get("stamp") == s5
                 and row.author_stamp == s6 and (not ZENCTL.is_file() or rc == 2),
                 f"zk2py observed {row and (row.reading.report.report or {}).get('observed')}, author "
                 f"{row and row.author_stamp}; zenctl exit {rc}")
        w = dt.watch(bus.g1, TARGET, 2.0, "task")
        rp.check(sec, "6. over a 2 s window, zk2py's tool finds the target behind v6 (its finding), every answer "
                      "still not yet", w.behind.get("task", (None, None))[0] == s6
                 and all(r.answers["converged"].answer == "not_yet" for r in w.rows),
                 f"{w.readings} readings, behind {w.behind}")
        if ZENCTL.is_file():
            rc, doc, _ = zenctl(["desired", TARGET, "-c", bus.ep1, "--timeout", "1", "--for", "2"])
            zb = (_z_row(doc) or {}).get("behind")
            rp.check(sec, "6. over the window, zenctl desired --for 2 finds the target behind v6, exit 1",
                     rc == 1 and isinstance(zb, dict) and zb.get("by") == "window", f"exit {rc}, behind {zb}")
            drc, ddoc = doctor(bus)
            rp.check(sec, "6. the doctor without a trusted clock: one reading, no desired-behind",
                     _z_check(ddoc, "desired-behind")[0] in ("not_established", "unobservable"),
                     f"{_z_check(ddoc, 'desired-behind')}")
            drc, ddoc = doctor(bus, ["--clocks-synced", "--desired-behind", "1"])
            rp.check(sec, "6. the doctor on a trusted clock, v6 older than 1 s: desired-behind",
                     _z_check(ddoc, "desired-behind")[0] == "established", f"{_z_check(ddoc, 'desired-behind')}")
            rc, doc, _ = zenctl(["desired", TARGET, "-c", bus.ep1, "--timeout", "1", "--for", "1",
                                 "--clocks-synced"])
            zb = (_z_row(doc) or {}).get("behind")
            rp.check(sec, "6. with a clock trusted, zenctl desired finds it behind by the clock, v6 older than the "
                          "window", rc == 1 and isinstance(zb, dict) and zb.get("by") == "clock",
                     f"exit {rc}, behind {zb}")
        w = dt.watch(bus.g1, TARGET, 1.0, "task", clocks_synced=True)
        rp.check(sec, "6. with a clock trusted, zk2py's tool finds it behind by the clock",
                 w.behind.get("task", (None, None))[1] == "clock", f"behind {w.behind}")
        target.cmd("desired release")
    finally:
        bus.close()
    # 4 on §7's bus: the author closed, the target restarted on the archive
    bus = Bus()
    rec = Path(tempfile.mkdtemp(prefix="zk2py-desired-")) / "record.json"
    try:
        from .owner import Owner

        archive = _archive(bus, 1)
        holder = Owner("lab", "archive", [load_contract(REPO / "impl/python/interop/stand-in/archive.v1.toml",
                                                        spec_dir=REPO / "spec")], connect=bus.ep1)
        holder.start()
        bus.extra.append(holder)
        author = bus.party(ctx.author_impl, AUTHOR, ["--desired-author", "--desired-record", str(rec)])
        tflags = ctx.target_flags("--desired-archive", "lab/archive")
        target = bus.party(ctx.target_impl, TARGET, tflags)
        s1 = _converge_v1(ctx, bus, author, target, 1)
        _wait(lambda: D in archive.held, 3.0)
        author.close()
        target.kill()
        target = bus.party(ctx.target_impl, TARGET, tflags)
        _wait(lambda: converged_on(target.report(), s1, 1, last_known=True), 10.0, 0.05)
        row, doc, rc = ask(bus, 4)
        z = _z_row(doc) or {}
        rp.check(sec, "4. with its author absent, both tools take D from the target's requirement and binding",
                 row is not None and row.slice == D and row.slice_from == "requirement"
                 and (not ZENCTL.is_file() or (z.get("slice") == D and z.get("slice_from") == "requirement")),
                 f"zk2py {row and row.slice_from}, zenctl {z.get('slice_from')}")
        rp.check(sec, "4. both show the archive's v1 as last-known, never current",
                 row is not None and any(st == s1 for _, st, _ in row.archived)
                 and (not ZENCTL.is_file() or s1 in json.dumps(z.get("archived"))),
                 f"zk2py {row and [(a, st) for a, st, _ in row.archived]}, zenctl {z.get('archived')}")
        if ZENCTL.is_file():
            drc, ddoc = doctor(bus)
            rp.check(sec, "4. the doctor's desired-last-known (info): the target runs a document read from an "
                          "archive", _z_check(ddoc, "desired-last-known")[0] == "established",
                     f"{_z_check(ddoc, 'desired-last-known')}")
    finally:
        bus.close()
    # 5 against §8's target
    bus = Bus()
    try:
        author = bus.party(ctx.author_impl, AUTHOR, ["--desired-author"])
        author.put(TARGET, v(1))
        target = bus.party(ctx.target_impl, TARGET, ["--desired-unbound"])
        time.sleep(1.0)
        ask(bus, 5)
    finally:
        bus.close()
    if disagree:
        rp.note(sec, "the two tools disagree: " + "; ".join(disagree))
    else:
        rp.note(sec, "the two tools agree on every step")


class Fake:
    """A test session standing in for a target or an author (§9's shapes no
    scenario makes): an instance token, a descriptor it writes, and state
    queryables answering what the test sets."""

    def __init__(self, endpoint: str, address: str, doc: dict[str, Any], answers: dict[str, list[tuple[bytes, bool]]],
                 interface_token: str | None = None) -> None:
        import zenoh

        from . import live

        self.session = live.open_client(endpoint)
        inst = doc["instance"]
        key = f"zk2/{address}/@zk/instance/{inst}"
        data = json.dumps({**doc, "meta": {"zid": str(self.session.zid())}}).encode()
        self.entities = [self.session.declare_queryable(key, zenoh.handlers.Callback(
            lambda q: q.reply(key, data, encoding="application/json")), complete=True)]
        for prefix, replies in answers.items():
            def answer(q: Any, replies: list[tuple[bytes, bool]] = replies, prefix: str = prefix) -> None:
                for payload, stamped in replies:
                    kw = {"timestamp": self.session.new_timestamp()} if stamped else {}
                    q.reply(prefix, payload, encoding="application/json", **kw)

            self.entities.append(self.session.declare_queryable(prefix, zenoh.handlers.Callback(answer),
                                                                complete=False))
        lv = self.session.liveliness()
        self.entities.append(lv.declare_token(key))
        if interface_token:
            self.entities.append(lv.declare_token(interface_token))

    def close(self) -> None:
        from . import live

        for e in reversed(self.entities):
            try:
                e.undeclare()
            except Exception:  # noqa: BLE001
                pass
        live.close(self.session)


def section_tool_shapes(ctx: Ctx) -> None:
    """zk2py's own, beyond scenarios.md: §2.17's 0.4 readings no scenario
    makes, built by test sessions, asked of both tools with the contracts
    held offline (core §8.5), each against the text."""
    from . import desired_tool as dt
    from .desired_live import contract

    sec, rp = "§9 beyond: the tool's 0.4 readings", ctx.report
    if ctx.author_impl != "zk2py" or ctx.target_impl != "zk2py":
        return  # no party runs here: once is enough
    worker, tasking, two = contract("worker.v1"), contract("tasking.v1"), contract("zk2py_worker2.v1")
    offline = str(REPO / "impl" / "python" / "interop" / "desired")

    def target_doc(bindings: list[str], c: Any = None) -> dict[str, Any]:
        c = c or worker
        return {"format": "zk2-descriptor/0.1", "service": TARGET, "instance": "7" * 16,
                "interfaces": [{"iface": c.interface, "contract": c.fingerprint, "minor": 0}],
                "requires": [{"role": "task", "interface": "tasking.v1", "declared_by": c.interface,
                              "bindings": bindings, "params": {"system": "self.system", "service": "self.service"},
                              "optional": True}],
                "profiles": ["desired.v1"]}

    def author_doc(standby: bool = False) -> dict[str, Any]:
        e: dict[str, Any] = {"iface": "tasking.v1", "contract": tasking.fingerprint, "minor": 0}
        if standby:
            e["standby"] = True
        return {"format": "zk2-descriptor/0.1", "service": AUTHOR, "instance": "8" * 16, "interfaces": [e],
                "profiles": ["desired.v1"]}

    conv = json.dumps({"key": D, "source": "desired", "in_force": {"stamp": "100/a1", "revision": 1},
                       "observed": {"stamp": "100/a1", "revision": 1}, "outcome": "converged"}).encode()
    conv2 = json.dumps({"key": D, "source": "desired", "in_force": {"stamp": "200/a1", "revision": 2},
                        "observed": {"stamp": "200/a1", "revision": 2}, "outcome": "converged"}).encode()
    doc = json.dumps(v(1)).encode()
    rp_prefix, d_prefix = RP, D
    two_a = f"zk2/{TARGET}/zk2py_worker2.v1/state/desired/task"
    two_b = f"zk2/{TARGET}/zk2py_worker2.v1/state/desired/again"
    shapes = [
        ("two report resources' readable reports claiming the slice, differing: unreadable (§2.10, 0.4)",
         target_doc([AUTHOR], two), {two_a: [(conv, True)], two_b: [(conv2, True)]}, author_doc(),
         {d_prefix: [(doc, True)]},
         {q: ("unobservable", "report_invalid") for q in ("converged", "refused", "last_known", "in_force")}),
        ("two readable reports on one key, differing: S4's Latest keeps the later, which reads as the report "
         "(F-212)", target_doc([AUTHOR]), {rp_prefix: [(conv, True), (conv2, True)]}, author_doc(),
         {d_prefix: [(doc, True)]},
         {"converged": ("not_yet", "other_epoch"), "refused": ("no", "converged"), "last_known": ("no", "current"),
          "in_force": ("yes", "desired")}),
        ("none readable claiming it, one that does not read: unreadable (§2.10, 0.4)",
         target_doc([AUTHOR]), {rp_prefix: [(b"{not json", True)]}, author_doc(), {d_prefix: [(doc, True)]},
         {q: ("unobservable", "report_invalid") for q in ("converged", "refused", "last_known", "in_force")}),
        ("the same report twice, equal: the report (§2.10)",
         target_doc([AUTHOR]), {rp_prefix: [(conv, True), (conv, True)]}, author_doc(), {d_prefix: [(doc, True)]},
         {"converged": ("not_yet", "other_epoch"), "refused": ("no", "converged"), "last_known": ("no", "current"),
          "in_force": ("yes", "desired")}),
        ("an unstamped reply of the author's: author_read_incomplete (§2.17, 0.4)",
         target_doc([AUTHOR]), {rp_prefix: [(conv, True)]}, author_doc(), {d_prefix: [(doc, False)]},
         {"converged": ("unobservable", "author_read_incomplete"), "refused": ("no", "converged"),
          "last_known": ("no", "current"), "in_force": ("yes", "desired")}),
        ("a standby author answers no GET: author_silent (§2.17, 0.4)",
         target_doc([AUTHOR]), {rp_prefix: [(conv, True)]}, author_doc(standby=True), {},
         {"converged": ("unobservable", "author_silent"), "refused": ("no", "converged"),
          "last_known": ("no", "current"), "in_force": ("yes", "desired")}),
        ("a binding §2.3 finds invalid (two authors) gives no slice: no_report (§2.17, 0.4)",
         target_doc([AUTHOR, "lab/author2"]), {rp_prefix: [(conv, True)]}, author_doc(), {d_prefix: [(doc, True)]},
         {q: ("unobservable", "no_report") for q in ("converged", "refused", "last_known", "in_force")}),
    ]
    at_key = D.replace("/tasking.v1/state/", "/tasking.v1/@state/")
    at_conv = json.dumps({"key": at_key, "source": "desired", "in_force": {"stamp": "100/a1", "revision": 1},
                          "observed": {"stamp": "100/a1", "revision": 1}, "outcome": "converged"}).encode()
    absent = ("absent", None)
    shapes += [
        ("the author absent, tasking.v1 held offline, the report keyed under @state: the slice from the offline "
         "revision, state, which no report claims: no report (F-213)", target_doc([AUTHOR]),
         {rp_prefix: [(at_conv, True)]}, absent, {},
         {q: ("unobservable", "no_report") for q in ("converged", "refused", "last_known", "in_force")}),
        ("the author absent, no contract of its interface in hand, the report keyed under @state: the slice from "
         "the requirement, the kind token the report's key carries (§2.17, 0.4)", target_doc([AUTHOR]),
         {rp_prefix: [(at_conv, True)]}, absent, {},
         {"converged": ("not_asked", "author_absent"), "refused": ("no", "converged"),
          "last_known": ("no", "current"), "in_force": ("yes", "desired")}),
        ("the author absent, and no report: no slice to take, no report (§2.17, 0.4)",
         target_doc([AUTHOR]), {}, absent, {},
         {q: ("unobservable", "no_report") for q in ("converged", "refused", "last_known", "in_force")}),
        ("the target's own contract not in hand, nor served: no template, no slice, no report",
         target_doc([AUTHOR]), {rp_prefix: [(conv, True)]}, author_doc(), {d_prefix: [(doc, True)]},
         {q: ("unobservable", "no_report") for q in ("converged", "refused", "last_known", "in_force")}),
    ]
    disagree = []
    for name, tdoc, tans, adoc, aans, want in shapes:
        bus = Bus()
        fakes = []
        try:
            fakes.append(Fake(bus.ep1, TARGET, tdoc, tans))
            if adoc is not absent:
                fakes.append(Fake(bus.ep1, AUTHOR, adoc, aans))
            time.sleep(0.5)
            held = [tasking] if "not in hand" in name else [worker, two] if "no contract of its interface" in name \
                else [worker, tasking, two]
            mine = dt.read(bus.g1, TARGET, "task", contracts=held)
            m = mine[0].pairs() if mine else None
            rp.check(sec, f"zk2py's tool: {name}", m == want, f"{m}; problems {mine[0].problems if mine else None}")
            if ZENCTL.is_file():
                path = (str(REPO / "impl/python/interop/desired/tasking.v1.toml") if "not in hand" in name
                        else str(REPO / "impl/python/interop/desired/worker.v1.toml")
                        if "no contract of its interface" in name else offline)
                rc, zdoc, err = zenctl(["desired", TARGET, "-c", bus.ep1, "--timeout", "1", "--contracts", path])
                z = _z_answers(_z_row(zdoc))
                rp.check(sec, f"zenctl desired: {name}", z == want, f"{z}, exit {rc}{(' ' + err[-200:]) if z is None else ''}")
                if z != m:
                    disagree.append(f"{name}: zk2py {m}, zenctl {z}")
        finally:
            for f in fakes:
                f.close()
            bus.close()
    rp.note(sec, "the two tools disagree: " + "; ".join(disagree) if disagree else "the two tools agree on every shape")


#: zk2py's own, beyond scenarios.md: a document text each target reads as
#: §2.4, §2.7 and §2.10 (0.3) say -> its outcome, its refusal's pointer
#: (None when none), and its observed revision (None when none).
EDGES = [
    ('{"mode": "run", "rate_hz": 10}', "refused", "", None),
    ('{"revision": 3.0, "mode": "run", "rate_hz": 10}', "converged", None, 3),
    ('{"revision": 1.5, "mode": "run", "rate_hz": 10}', "refused", "/revision", None),
    ('{"revision": 4, "mode": "fly", "rate_hz": 10}', "refused", "/mode", 4),
    ('{"revision": 5, "mode": "run", "rate_hz": 0}', "refused", "/rate_hz", 5),
    ('[1, 2]', "refused", "", None),
    ('{"revision": 18446744073709551616, "mode": "run", "rate_hz": 10}', "converged", None, None),
    ('{"revision": 30e-1, "mode": "run", "rate_hz": 11}', "converged", None, 3),
    ('{"revision": 7, "mode": "run", "rate_hz": 10.5}', "refused", "/rate_hz", 7),
]


def section_edges(ctx: Ctx) -> None:
    """Edge documents (zk2py's own): §2.7 (0.3) "A document that decodes and
    that its JSON Schema (core §7.3) refuses is invalid … its pointer is
    where the schema refuses it"; §2.4 (0.3) a revision "read by its exact
    value as a report's is: 3.0 is 3, 1.5 is no revision"; and what the
    author writes on D of a number past a double's precision (a NOTE)."""
    from . import desired as ds

    sec, rp = ctx.name("edges: refusals, pointers, exact revisions"), ctx.report
    bus = Bus()
    try:
        author = bus.party(ctx.author_impl, AUTHOR, ["--desired-author"])
        target = bus.party(ctx.target_impl, TARGET, ctx.target_flags())
        time.sleep(0.3)
        for text, outcome, pointer, rev in EDGES:
            ans = author.cmd(f"desired put {TARGET} {text}")
            stamp = ans.rsplit(" ", 1)[1] if ans and ans.startswith("desired put ") else None
            _wait(lambda: (target.report() or {}).get("observed", {}).get("stamp") == stamp, 5.0, 0.05)
            rep = target.report() or {}
            got = (rep.get("outcome"), rep.get("refusal", {}).get("pointer") if "refusal" in rep else None,
                   rep.get("observed", {}).get("revision"), rep.get("refusal", {}).get("code"))
            want = (outcome, pointer, rev, "invalid" if outcome == "refused" else None)
            rp.check(sec, f"{text}: {outcome}" + (f" invalid at {pointer!r}" if outcome == "refused" else "")
                     + f", observed revision {rev}", stamp is not None and got == want, f"{got}")
        for text in ('{"revision": 20, "mode": "run", "rate_hz": 10, "gain": 0.10000000000000000555}',
                     '{"revision": 21, "mode": "run", "rate_hz": 10, "seq": 18446744073709551616}'):
            author.cmd(f"desired put {TARGET} {text}")
            time.sleep(0.3)
            on_d = [x.payload for x in bus.get(D).replies]
            kept = bool(on_d) and ds.jcs_equal(ds.strict_json(on_d[0], exact=True),
                                               ds.strict_json(text.encode(), exact=True))
            rp.note(sec, f"the author writes {text} on D {'by its exact value' if kept else 'otherwise'}: "
                         f"{on_d[0].decode() if on_d else None}")
    finally:
        bus.close()


def section_remint(ctx: Ctx) -> None:
    """zk2py's own, beyond scenarios.md: §2.7 (0.3) "A re-mint is not a
    restart" for a target, against §5's restart."""
    sec, rp = ctx.name("re-mint: a target keeps its acts"), ctx.report
    bus = Bus()
    try:
        author = bus.party(ctx.author_impl, AUTHOR, ["--desired-author"])
        target = bus.party(ctx.target_impl, TARGET, ctx.target_flags())
        s1 = _converge_v1(ctx, bus, author, target, 1)
        n = n_acts(target)
        ne = target.cmd("desired new-epoch")
        time.sleep(1.0)
        rp.check(sec, "the target re-mints in its own process: its report and its acts kept, nothing acted",
                 ne is not None and ne.startswith("desired new-epoch ") and converged_on(target.report(), s1, 1)
                 and n_acts(target) == n, f"{ne}, acts {n} -> {n_acts(target)}")
        t = time.monotonic()
        author.cmd("desired new-epoch")
        ok = _wait(lambda: any(g[2] == "put" for g in bus.since(t, D)), 5.0)
        reput = next((g[4] for g in bus.since(t, D) if g[2] == "put"), None)
        ok = ok and _wait(lambda: converged_on(target.report(), reput, 1), 5.0, 0.05)
        rp.check(sec, "then the author re-mints and puts v1 again: the target adopts it, never acts again (a restart "
                      "would have acted)", ok and n_acts(target) == n and not bus.since(t, RUN),
                 f"acts {n} -> {n_acts(target)}, {target.report()}")
    finally:
        bus.close()


SECTIONS = {"1": section1, "2": section2, "3": section3, "4": section4, "5": section5, "6": section6,
            "7": section7, "8": section8, "9": section9, "10": section10, "edges": section_edges,
            "remint": section_remint, "tool-shapes": section_tool_shapes}


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="python -m zk2py.desired_scenarios", description=__doc__.split("\n")[0])
    ap.add_argument("--author", default="zk2py", help="zk2py, or the owner example's path")
    ap.add_argument("--target", default="zk2py", help="zk2py, or the owner example's path")
    ap.add_argument("--owner", default=None, help="both parties the owner example at this path")
    ap.add_argument("--only", action="append", choices=list(SECTIONS), help="run only these sections (edges: "
                    "zk2py's own documents beyond scenarios.md)")
    ap.add_argument("--poll", action="store_true", help="targets read their author's presence on a period")
    args = ap.parse_args(argv)
    author = args.owner or args.author
    target = args.owner or args.target
    for p in (author, target):
        if p != "zk2py" and not Path(p).is_file():
            print(f"error: {p} not found", file=sys.stderr)
            return 2
    report = Report()
    ctx = Ctx(report, author if author == "zk2py" else Path(author), target if target == "zk2py" else Path(target),
              watch=not args.poll)
    for name, fn in SECTIONS.items():
        if args.only and name not in args.only:
            continue
        try:
            fn(ctx)
        except Exception as e:  # noqa: BLE001 - a section that crashes is a failure, reported
            import traceback

            traceback.print_exc()
            report.check(ctx.name(f"§{name}"), "the section ran", False, repr(e))
    passed = sum(1 for r in report.rows if r[2])
    failed = len(report.rows) - passed
    a = "zk2py" if author == "zk2py" else "rust"
    t = "zk2py" if target == "zk2py" else "rust"
    print(f"desired scenarios ({a} author, {t} target{', polling' if args.poll else ''}): {passed} passed, "
          f"{failed} failed")
    for sec, name, ok, detail in report.rows:
        if not ok:
            print(f"FAIL [{sec}] {name}: {detail}")
    return 1 if failed else 0


if __name__ == "__main__":
    rc = main()
    sys.stdout.flush()
    os._exit(rc)

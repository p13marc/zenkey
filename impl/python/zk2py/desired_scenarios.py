"""desired.v1 ``scenarios.md`` §1 to §8 and §10 (text 0.3), live, from Python
(#609, round 55)::

    python -m zk2py.desired_scenarios [--author zk2py|<owner exe>] [--target zk2py|<owner exe>]
        [--only 1|2|…|12|edges|config-fed|remint|tool-shapes]... [--poll] [--zenctl PATH | --no-zenctl]

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

§12 (text 0.10, F-336) is the runtime's, and runs in process with zk2py's
own parties: a :class:`zk2py.config.ConfigOwner` fed through this profile,
once refusing the re-mint and once re-minting as a restart. With the owner
example as either party it is not run, and says so: the example has no
``config.v1`` owner fed through ``desired.v1``. ``config-fed`` is zk2py's
own, the rest of v1.md §2.16 that owner does.
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


#: the Rust tool (§9): ``--zenctl`` or ``ZK2PY_ZENCTL``, and no default;
#: ``--no-zenctl`` runs zk2py's tool alone, said loudly. Neither given, or a
#: path that is not a file, fails §9 and the shapes, never a quiet pass.
ZENCTL: Path | None = Path(os.environ["ZK2PY_ZENCTL"]) if os.environ.get("ZK2PY_ZENCTL") else None
#: set by --no-zenctl
ZENCTL_OFF = False


def _zenctl_ready(rp: Report, sec: str, checks: int) -> bool:
    """Whether §9's Rust tool runs: True when it is a file; else, with
    --no-zenctl, a NOT RUN line counting its checks; else a failure."""
    if ZENCTL_OFF:
        rp.cannot(sec, f"zenctl not asked (--no-zenctl): {checks} of the section's checks not run")
        return False
    if ZENCTL is None:
        rp.check(sec, "the Rust tool is given (--zenctl or ZK2PY_ZENCTL; --no-zenctl to run without it)",
                 False, "neither given")
        return False
    if not ZENCTL.is_file():
        rp.check(sec, f"the Rust tool is at {ZENCTL} (--zenctl, ZK2PY_ZENCTL; --no-zenctl to run without it)",
                 False, "not a file")
        return False
    return True
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
    use_z = _zenctl_ready(rp, sec, 17)
    disagree: list[str] = []

    def ask(bus: Bus, step: int, extra: list[str] = ()) -> tuple[Any, Any, Any]:
        mine = dt.read(bus.g1, TARGET, "task")
        rc, doc, err = zenctl(["desired", TARGET, "-c", bus.ep1, "--timeout", "1", *extra]) if use_z \
            else (None, None, "")
        z = _z_answers(_z_row(doc))
        m = mine[0].pairs() if mine else None
        want = S9_EXPECT[step]
        rp.check(sec, f"{step}. zk2py's tool answers as §9 Expected {step}", m == want, f"{m}")
        if use_z:
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
                 not use_z or rc == 0, f"exit {rc}")
        rp.check(sec, "1. both tools take the slice from the author's contract",
                 row is not None and row.slice == D and row.slice_from == "author_contract"
                 and (not use_z or (_z_row(doc) or {}).get("slice_from") == "author_contract"),
                 f"zk2py {row and row.slice_from}, zenctl {(_z_row(doc) or {}).get('slice_from')}")
        if use_z:
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
        if use_z:
            drc, ddoc = doctor(bus)
            ans, finds = _z_check(ddoc, "desired-refused")
            rp.check(sec, "2. the doctor's desired-refused finds the target's refused act", ans == "established"
                     and any("lab/worker" in str(f.get("subject")) for f in finds), f"{ans}, {finds}")
        target.cmd("desired fail-next")
        author.put(TARGET, v(4, rate_hz=20, mode="halt"))
        _wait(lambda: (target.report() or {}).get("outcome") == "failed", 10.0, 0.05)
        ask(bus, 3)
        if use_z:
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
                 and row.author_stamp == s6 and (not use_z or rc == 2),
                 f"zk2py observed {row and (row.reading.report.report or {}).get('observed')}, author "
                 f"{row and row.author_stamp}; zenctl exit {rc}")
        w = dt.watch(bus.g1, TARGET, 2.0, "task")
        rp.check(sec, "6. over a 2 s window, zk2py's tool finds the target behind v6 (its finding), every answer "
                      "still not yet", w.behind.get("task", (None, None))[0] == s6
                 and all(r.answers["converged"].answer == "not_yet" for r in w.rows),
                 f"{w.readings} readings, behind {w.behind}")
        if use_z:
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
                 and (not use_z or (z.get("slice") == D and z.get("slice_from") == "requirement")),
                 f"zk2py {row and row.slice_from}, zenctl {z.get('slice_from')}")
        rp.check(sec, "4. both show the archive's v1 as last-known, never current",
                 row is not None and any(st == s1 for _, st, _ in row.archived)
                 and (not use_z or s1 in json.dumps(z.get("archived"))),
                 f"zk2py {row and [(a, st) for a, st, _ in row.archived]}, zenctl {z.get('archived')}")
        if use_z:
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
    elif use_z:
        rp.note(sec, "the two tools agree on every step")


def _row_of_mine(r: Any) -> dict[str, Any]:
    """zk2py's row, in the members zenctl's ``desired`` row shows."""
    from . import desired as ds

    if r is None:
        return {}
    rep = r.report_reply
    return {"listing": r.listing, "slice": r.slice, "slice_from": r.slice_from, "author": r.author,
            "author_presence": r.author_presence, "author_current": r.author_current,
            "report": "unreadable" if rep and rep.startswith("unreadable:") else rep,
            "requirement_unread": r.requirement_unread,
            "binding_invalid": [x for x in r.problems if x in ds.BINDING_REASONS],
            "declaration_invalid": [y for x in r.problems if x.startswith("declaration_invalid:")
                                    for y in x.split(":", 1)[1].split(",")]}


def _row_of_zenctl(z: Any) -> dict[str, Any]:
    """zenctl's ``desired`` row (``--format json``), in the same members."""
    if not isinstance(z, dict):
        return {}
    a = z.get("author") or {}
    rep = z.get("report") or {}
    return {"listing": z.get("listing"), "slice": z.get("slice"), "slice_from": z.get("slice_from"),
            "author": a.get("address"), "author_presence": a.get("presence"), "author_current": a.get("current"),
            "report": "unreadable" if rep.get("unreadable") else rep.get("key"),
            "requirement_unread": z.get("requirement_unread"),
            "binding_invalid": list(z.get("binding_invalid") or []),
            "declaration_invalid": list(z.get("declaration_invalid") or [])}


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
    from .contract import load_contract
    from .desired_live import contract

    sec, rp = "§9 beyond: the tool's 0.4 to 0.8 readings", ctx.report
    if ctx.author_impl != "zk2py" or ctx.target_impl != "zk2py":
        return  # no party runs here: once is enough
    use_z = _zenctl_ready(rp, sec, 60)
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
    # 0.5's shapes (F-210, F-213): a role naming two templates, read from the
    # target alone (four candidates); a held revision declaring the resource
    # explicit. Each with the contracts zk2py's tool and zenctl hold.
    three = contract("zk2py_worker3.v1")
    explicit = load_contract(REPO / "impl/python/interop/desired-explicit/tasking.v1.toml", spec_dir=REPO / "spec")
    three_a = f"zk2/{TARGET}/zk2py_worker3.v1/state/desired/task"
    three_b = f"zk2/{TARGET}/zk2py_worker3.v1/state/desired/again"

    def claim(k: str, stamp: str = "100/a1") -> bytes:
        return json.dumps({"key": k, "source": "desired", "in_force": {"stamp": stamp, "revision": 1},
                           "observed": {"stamp": stamp, "revision": 1}, "outcome": "converged"}).encode()

    p3 = str(REPO / "impl/python/interop/desired/zk2py_worker3.v1.toml")
    p1 = str(REPO / "impl/python/interop/desired/worker.v1.toml")
    px = str(REPO / "impl/python/interop/desired-explicit/tasking.v1.toml")
    plans_at = "zk2/lab/author/tasking.v1/@state/plans/lab/worker"
    absent_answers = {"converged": ("not_asked", "author_absent"), "refused": ("no", "converged"),
                      "last_known": ("no", "current"), "in_force": ("yes", "desired")}
    no_report = {q: ("unobservable", "no_report") for q in ("converged", "refused", "last_known", "in_force")}
    invalid = {q: ("unobservable", "report_invalid") for q in ("converged", "refused", "last_known", "in_force")}
    #: each shape's contracts by its name: (zk2py's, zenctl's --contracts,
    #: the role asked); a shape not named here holds round 57's
    extra: dict[str, tuple[list[Any], list[str], str]] = {}
    from_three, held_explicit = ([three], [p3], "task"), ([worker, explicit], [p1, px], "task")
    rules05 = [
        ("two templates, the author absent, none of its interface in hand: four candidates, the report claiming "
         "@state plans is the slice (0.5, F-210)", target_doc([AUTHOR], three), {three_a: [(claim(plans_at), True)]},
         absent, {}, absent_answers, from_three),
        ("two templates, no report claiming a candidate: no_report (0.5, F-210)", target_doc([AUTHOR], three),
         {three_a: [(claim("zk2/lab/author/tasking.v1/state/other/lab/worker"), True)]}, absent, {}, no_report,
         from_three),
        ("two templates, two reports claiming two candidates and differing: report_invalid (0.5, F-210, §2.10)",
         target_doc([AUTHOR], three), {three_a: [(claim(D), True)], three_b: [(claim(plans_at), True)]}, absent, {},
         invalid, from_three),
        ("the author absent, a held revision declaring the resource explicit, the report under state: the "
         "held @state slice, which no report claims: no_report (0.5, F-213)", target_doc([AUTHOR]),
         {rp_prefix: [(conv, True)]}, absent, {}, no_report, held_explicit),
        ("the author absent, a held revision declaring the resource explicit, the report under @state: the "
         "report (0.5, F-213)", target_doc([AUTHOR]), {rp_prefix: [(at_conv, True)]}, absent, {}, absent_answers,
         held_explicit),
    ]
    for name, tdoc, tans, adoc, aans, want, how in rules05:
        shapes.append((name, tdoc, tans, adoc, aans, want))
        extra[name] = how
    # Shapes the text does not state (F-217, F-218), and a role naming every
    # resource: each answered as zk2py reads it, which the reference shares.
    watcher, everything = contract("zk2py_watcher.v1"), contract("zk2py_workerall.v1")
    tasking2, four = contract("zk2py_tasking2.v1"), contract("zk2py_worker4.v1")
    pd = REPO / "impl/python/interop/desired"
    pw, pall, pt2, p4 = (str(pd / f"{n}.toml") for n in ("zk2py_watcher.v1", "zk2py_workerall.v1",
                                                         "zk2py_tasking2.v1", "zk2py_worker4.v1"))
    pt = str(pd / "tasking.v1.toml")

    def doc_of(c: Any, role: str, iface: str, params: dict[str, str]) -> dict[str, Any]:
        return {"format": "zk2-descriptor/0.1", "service": TARGET, "instance": "7" * 16,
                "interfaces": [{"iface": c.interface, "contract": c.fingerprint, "minor": 0}],
                "requires": [{"role": role, "interface": iface, "declared_by": c.interface, "bindings": [AUTHOR],
                              "params": params, "optional": True}],
                "profiles": ["desired.v1"]}

    selfs = {"system": "self.system", "service": "self.service"}
    a_worker = {"format": "zk2-descriptor/0.1", "service": AUTHOR, "instance": "8" * 16,
                "interfaces": [{"iface": "worker.v1", "contract": worker.fingerprint, "minor": 0}],
                "profiles": ["desired.v1"]}
    a_t2 = {"format": "zk2-descriptor/0.1", "service": AUTHOR, "instance": "8" * 16,
            "interfaces": [{"iface": "zk2py_tasking2.v1", "contract": tasking2.fingerprint, "minor": 0}],
            "profiles": ["desired.v1"]}
    t2_key = "zk2/lab/author/zk2py_tasking2.v1/state/{}/lab/worker"
    four_rp = f"zk2/{TARGET}/zk2py_worker4.v1/state/desired/task"
    all_rp = f"zk2/{TARGET}/zk2py_workerall.v1/state/desired/task"
    not_a_target = {q: ("not_asked", "not_a_target") for q in ("converged", "refused", "last_known", "in_force")}
    unstated = [
        ("a role on no desired resource, its author's revision in hand: not a target (F-217)",
         doc_of(watcher, "watch", "worker.v1", {}), {}, a_worker,
         {"zk2/lab/author/worker.v1/state/running": [(doc, True)]}, not_a_target,
         ([watcher, worker], [pw, p1], "watch")),
        ("a role on no desired resource, its author absent, its revision held: not a target, never the target "
         "alone (0.6, F-217)", doc_of(watcher, "watch", "worker.v1", {}), {}, absent, {}, not_a_target,
         ([watcher, worker], [pw, p1], "watch")),
        ("a role naming two desired resources, the author absent, none held: the report's claim of tasks is the "
         "slice (F-218)", doc_of(four, "task", "zk2py_tasking2.v1", selfs),
         {four_rp: [(claim(t2_key.format("tasks")), True)]}, absent, {}, absent_answers, ([four], [p4], "task")),
        ("a role naming two desired resources, the author absent, its revision held: two_documents, no slice, "
         "no report (0.6, F-218)", doc_of(four, "task", "zk2py_tasking2.v1", selfs),
         {four_rp: [(claim(t2_key.format("tasks")), True)]}, absent, {}, no_report,
         ([four, tasking2], [p4, pt2], "task")),
        ("a role naming two desired resources, the author present: two_documents, no slice, no report "
         "(0.6, F-218)", doc_of(four, "task", "zk2py_tasking2.v1", selfs),
         {four_rp: [(claim(t2_key.format("tasks")), True)]}, a_t2,
         {t2_key.format(w): [(doc, True)] for w in ("plans", "tasks")}, no_report, ([four, tasking2], [p4, pt2], "task")),
        ("a role naming every resource, the author absent, none held: no template named, no candidate, no report",
         doc_of(everything, "task", "tasking.v1", selfs), {all_rp: [(conv, True)]}, absent, {}, no_report,
         ([everything], [pall], "task")),
        ("a role naming every resource, the author absent, tasking.v1 held: its desired resource gives the slice "
         "(F-213)", doc_of(everything, "task", "tasking.v1", selfs), {all_rp: [(conv, True)]}, absent, {},
         absent_answers, ([everything, tasking], [pall, pt], "task")),
    ]
    # 0.7: a role a component's manifest declares, its descriptor naming no
    # declaring contract, names every resource (core §3.1, §3.3)
    def manifest_doc(c: Any, iface: str) -> dict[str, Any]:
        d = doc_of(c, "task", iface, selfs)
        d["requires"][0].pop("declared_by")
        return d

    a_task = {"format": "zk2-descriptor/0.1", "service": AUTHOR, "instance": "8" * 16,
              "interfaces": [{"iface": "tasking.v1", "contract": tasking.fingerprint, "minor": 0}],
              "profiles": ["desired.v1"]}
    unstated += [
        ("a manifest's role, no declaring contract, on an author with two desired resources: every resource, "
         "two_documents, no slice, no report (0.7)", manifest_doc(four, "zk2py_tasking2.v1"),
         {four_rp: [(claim(t2_key.format("tasks")), True)]}, a_t2,
         {t2_key.format(w): [(doc, True)] for w in ("plans", "tasks")}, no_report, ([four, tasking2], [p4, pt2], "task")),
        ("a manifest's role, no declaring contract, on an author with one desired resource: its slice, the report "
         "(0.7)", manifest_doc(worker, "tasking.v1"), {rp_prefix: [(conv, True)]}, a_task, {d_prefix: [(doc, True)]},
         {"converged": ("not_yet", "other_epoch"), "refused": ("no", "converged"), "last_known": ("no", "current"),
          "in_force": ("yes", "desired")}, ([worker, tasking], [p1, pt], "task")),
    ]
    # 0.8 (F-225): a requirement whose declaring contract the descriptor
    # names and the tool does not hold is unread, never every resource
    def unread_doc(iface: str, declared: str, fp: str, role: str = "task", bindings: list[str] | None = None,
                   own: Any = None) -> dict[str, Any]:
        entries = [{"iface": declared, "contract": fp, "minor": 0}]
        if own is not None:
            entries.append({"iface": own.interface, "contract": own.fingerprint, "minor": 0})
        return {"format": "zk2-descriptor/0.1", "service": TARGET, "instance": "7" * 16, "interfaces": entries,
                "requires": [{"role": role, "interface": iface, "declared_by": declared,
                              "bindings": [AUTHOR] if bindings is None else bindings, "params": selfs,
                              "optional": True}],
                "profiles": ["desired.v1"]}

    nobody = "sha256:" + "ab" * 32
    unbound_answers = {"converged": ("not_asked", "unbound"), "refused": ("not_asked", "unbound"),
                       "last_known": ("not_asked", "unbound"), "in_force": ("no", "unbound")}
    unstated += [
        ("F-225's shape: zk2py_worker9.v1 named at a fingerprint no one serves, on an author with two desired "
         "resources: the requirement unread, no_report, no two_documents (0.8)",
         unread_doc("zk2py_tasking2.v1", "zk2py_worker9.v1", nobody), {}, a_t2,
         {t2_key.format(w): [(doc, True)] for w in ("plans", "tasks")}, no_report, ([tasking2], [pt2], "task")),
        ("unread on an author with one desired resource: unread too, no slice (0.8, decided)",
         unread_doc("tasking.v1", "zk2py_worker9.v1", nobody), {}, a_task, {d_prefix: [(doc, True)]}, no_report,
         ([tasking], [pt], "task")),
        ("unread on an author whose revision declares no desired resource at all: not a target (0.8; 0.6, "
         "F-217)", unread_doc("worker.v1", "zk2py_worker9.v1", nobody, role="watch"), {}, a_worker,
         {"zk2/lab/author/worker.v1/state/running": [(doc, True)]}, not_a_target, ([worker], [p1], "watch")),
        ("a declaring contract in hand that declares no such role: unread (0.8, decided)",
         unread_doc("tasking.v1", "worker.v1", worker.fingerprint, role="other"), {}, a_task,
         {d_prefix: [(doc, True)]}, no_report, ([worker, tasking], [p1, pt], "other")),
        ("unread and unbound: still the kill switch, the descriptor's word (0.8, decided)",
         unread_doc("tasking.v1", "zk2py_worker9.v1", nobody, bindings=[]), {}, a_task, {d_prefix: [(doc, True)]},
         unbound_answers, ([tasking], [pt], "task")),
    ]
    for name, tdoc, tans, adoc, aans, want, how in unstated:
        shapes.append((name, tdoc, tans, adoc, aans, want))
        extra[name] = how
    disagree: list[str] = []
    row_notes: list[str] = []
    for name, tdoc, tans, adoc, aans, want in shapes:
        bus = Bus()
        fakes = []
        try:
            fakes.append(Fake(bus.ep1, TARGET, tdoc, tans))
            if adoc is not absent:
                fakes.append(Fake(bus.ep1, AUTHOR, adoc, aans))
            time.sleep(0.5)
            role = "task"
            if name in extra:
                held, paths, role = extra[name]
            else:
                held = [tasking] if "not in hand" in name else [worker, two] if "no contract of its interface" in \
                    name else [worker, tasking, two]
                paths = [str(REPO / "impl/python/interop/desired/tasking.v1.toml") if "not in hand" in name
                         else p1 if "no contract of its interface" in name else offline]
            mine = dt.read(bus.g1, TARGET, role, contracts=held)
            m = mine[0].pairs() if mine else None
            rp.check(sec, f"zk2py's tool: {name}", m == want, f"{m}; problems {mine[0].problems if mine else None}; "
                     f"slice {mine[0].slice if mine else None} from {mine[0].slice_from if mine else None}")
            if use_z:
                cargs = [x for q in paths for x in ("--contracts", q)] + (["--role", role] if role != "task" else [])
                rc, zdoc, err = zenctl(["desired", TARGET, "-c", bus.ep1, "--timeout", "1", *cargs])
                z = _z_answers(_z_row(zdoc, role))
                rp.check(sec, f"zenctl desired: {name}", z == want, f"{z}, exit {rc}{(' ' + err[-200:]) if z is None else ''}")
                if z != m:
                    disagree.append(f"{name}: zk2py {m}, zenctl {z}")
                # the whole row, beside the answers: what each tool says it read
                mr, zr = _row_of_mine(mine[0] if mine else None), _row_of_zenctl(_z_row(zdoc, role))
                differ = {k: (mr.get(k), zr.get(k)) for k in sorted(set(mr) | set(zr)) if mr.get(k) != zr.get(k)}
                # Known in round 59, in the rows alone: for two reports that
                # claim the slice and differ, zenctl's row then carried no
                # report, where §2.10 calls the two unreadable; since 0.6's
                # build it says unreadable too, and this allowance is unused
                known = bool(mine) and mine[0].report_reply == "unreadable:two_differ" and \
                    differ == {"report": ("unreadable", None)}
                if known:
                    row_notes.append(name)
                rp.check(sec, f"the two rows agree: {name}", not differ or known,
                         f"differing (zk2py, zenctl): {differ}" + (" (known: two that differ)" if known else ""))
        finally:
            for f in fakes:
                f.close()
            bus.close()
    rp.note(sec, ("the two tools disagree: " + "; ".join(disagree)) if disagree
            else "the two tools agree on every shape" if use_z else "zk2py's tool alone")
    if row_notes:
        rp.note(sec, f"their rows differ only where two reports claim the slice and differ ({len(row_notes)} "
                     "shapes): zenctl's row carries no report, zk2py's says unreadable, as §2.10 does")


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


def _stop(p: subprocess.Popen) -> str:
    """The process ended, its standard input closed and its pipes drained:
    ``communicate`` within 10 s, else killed and waited for. Returns its
    standard error."""
    try:
        _, err = p.communicate(input="", timeout=10)
    except (subprocess.TimeoutExpired, ValueError):
        p.kill()
        _, err = p.communicate()
    return err or ""


def section11(ctx: Ctx) -> None:
    """scenarios.md §11 (0.7, F-224), "A role on two documents refuses the
    start": one router and no author.

    1. The run's target given the two copies, ``desired-two``'s worker.v1
       (its role naming tasks and plans) and tasking.v1 (both desired). It
       must exit 1, its standard error saying ``not started: …`` and
       ``two_documents``, with no instance token at S and no report on
       ``Rp``. Another refusal at the start that exits 1 with that text
       would pass too: the text, not the cause, is what is read.
    2. The control: the scenarios' worker.v1 (``interop/desired``'s, its
       requirement naming tasks alone), given explicitly, against
       ``desired-two``'s tasking.v1. It must start, S seeing its instance
       token and its report ``{key: D, source: baseline}`` on ``Rp``.

    S watches the instance tokens by liveliness, and ``Rp``. Each step
    waits until the process exits or S sees its instance token, within the
    5 s, never a fixed sleep (``_wait`` returns False at its deadline,
    never raising)."""
    sec, rp = ctx.name("§11 a role on two documents refuses the start"), ctx.report
    two = REPO / "impl/python/interop/desired-two"
    exe = ([sys.executable, "-m", "zk2py.desired_owner"] if ctx.target_impl == "zk2py" else [str(ctx.target_impl)])
    env = dict(os.environ, PYTHONPATH=str(REPO / "impl" / "python"))
    bus = Bus()
    try:
        one = REPO / "impl/python/interop/desired/worker.v1.toml"
        for step, files in (("1", [two / "worker.v1.toml", two / "tasking.v1.toml"]),
                            ("2", [one, two / "tasking.v1.toml"])):
            t = time.monotonic()
            p = subprocess.Popen([*exe, "--connect", bus.ep1, "--desired-target", AUTHOR, TARGET, *map(str, files)],
                                 stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
                                 env=env)
            _wait(lambda: p.poll() is not None or bus.token_put(TARGET, t) is not None, 5.0, 0.05)
            if step == "2":
                # the control's report, put before its tokens (§2.10)
                _wait(lambda: bool(bus.since(t, RP)), 2.0, 0.05)
            code = p.poll()
            token = bus.token_put(TARGET, t)
            reports = bus.since(t, RP)
            err = _stop(p).strip()
            if step == "1":
                rp.check(sec, "1. the role is two_documents, and the target refuses to start: S sees no instance "
                              "token within 5 s and no report on Rp; it stops, exit 1, saying not started: … "
                              "two_documents",
                         code == 1 and token is None and not reports and err.startswith("not started:")
                         and "two_documents" in err,
                         f"exit {code}; token {token}; reports {len(reports)}; {err[-300:]!r}")
            else:
                first = json.loads(reports[0][3]) if reports else None
                missing = [what for what, gone in (("the process ended", code is not None),
                                                   ("no instance token at S", token is None),
                                                   ("no report on Rp", not reports)) if gone]
                if reports and first != {"key": D, "source": "baseline"}:
                    missing.append(f"a report other than {{key: D, source: baseline}}: {first}")
                rp.check(sec, "2. the control, naming one of the author's two desired resources: S sees its "
                              "instance token, and its report on Rp is {key: D, source: baseline}",
                         not missing, (f"missing: {'; '.join(missing)}; " if missing else "")
                         + f"exit {code}; token {token}; report {first}; {err[-200:]!r}")
            time.sleep(0.5)
    finally:
        bus.close()


# -- §12: a config.v1 owner's re-mint is a restart (text 0.10, F-336) -----------------------

RF0 = "lab/rf0"
ST12 = "zk2/lab/rf0/config.v1/state/status"
EV12 = "zk2/lab/rf0/config.v1/events/change/"
DESIRED12 = {"revision": 1, "values": {"duty": {"fraction_ppm": 20000}}}


def _configs_key() -> str:
    """§12's slice: ``configs/{system}/{service}`` of ``lab/author``'s
    ``configs.v1``, ``{system} = self.system`` and ``{service} =
    self.service`` (v1.md §2.3)."""
    from . import desired

    return desired.slice_key(AUTHOR, "configs.v1", False, "configs/{system}/{service}",
                             {"system": "lab", "service": "rf0"})


def _rf0(bus: Bus, remint: str, device: Any = None, overlay: Any = None, events_at: Any = None) -> Any:
    """``config.v1``'s scenarios' owner ``lab/rf0`` (its schema, startup
    values, floor and apply delay: ``config_scenarios``'), fed through this
    profile (v1.md §2.16): its role ``configs`` bound to ``lab/author``,
    declared by ``interop/desired/zk2py_configured.v1``."""
    from . import config as cf
    from .config_scenarios import STARTUP, rf_schema
    from .contract import load_contract

    configured = load_contract(REPO / "impl/python/interop/desired/zk2py_configured.v1.toml", spec_dir=REPO / "spec")
    o = cf.ConfigOwner("lab", "rf0", schema=rf_schema(), startup=STARTUP, floor_s=10, apply_after_s=1.0,
                       connect=bus.ep1, overlay=overlay, device=device, events_at=events_at,
                       contracts=[configured], fed=(_configs_key(), AUTHOR), remint=remint,
                       bindings={"configs": [AUTHOR]},
                       params={"configs": {"system": "self.system", "service": "self.service"}})
    o.start()
    return o


def _st12(bus: Bus, t: float) -> list[dict[str, Any]]:
    return [json.loads(g[3]) for g in bus.since(t, ST12) if g[2] == "put"]


def _applied12(bus: Bus, t: float, instance: str) -> list[dict[str, Any]]:
    out = []
    for g in bus.since(t):
        if g[1].startswith(EV12) and g[2] == "put":
            ev = json.loads(g[3])
            if ev.get("instance") == instance and ev.get("outcome") == "applied":
                out.append(ev)
    return out


def _duty(doc: Any) -> Any:
    return ((doc or {}).get("values") or {}).get("duty", {}).get("fraction_ppm")


def _tokens12(bus: Bus, t: float) -> list[tuple[str, str]]:
    with bus.lock:
        return [(kind, key) for at, kind, key in bus.tokens
                if at >= t and key.startswith(f"zk2/{RF0}/@zk/instance/")]


def section12(ctx: Ctx) -> None:
    """scenarios.md §12 (text 0.10, F-336), "A ``config.v1`` owner's re-mint
    is a restart": the runtime's, so zk2py's own owner, in process, a
    :class:`zk2py.config.ConfigOwner` fed through this profile (the owner
    example has no such mode: the reference runs §12 as a ``zenkey`` test).
    Step 1 runs twice, once for each branch of its Expected: an owner that
    refuses the re-mint, as the reference does, and one that re-mints as a
    restart. The author is a zk2py owner of ``interop/desired/configs.v1``,
    its document put once, stamped. S is the Bus's: every delivery under
    ``zk2/lab/**`` and the instance tokens."""
    rp = ctx.report
    if ctx.target_impl != "zk2py" or ctx.author_impl != "zk2py":
        rp.cannot(ctx.name("§12 a config.v1 owner's re-mint is a restart"),
                  "the runtime's, run with zk2py's config.v1 owner alone: the owner example has no config.v1 "
                  "owner fed through desired.v1")
        return
    from . import config as cf
    from .contract import load_contract
    from .owner import Owner, OwnerRefused

    for branch in ("refuse", "restart"):
        sec = ctx.name(f"§12 a config.v1 owner's re-mint is a restart ({branch})")
        bus = Bus()
        author = None
        owners: list[Any] = []
        try:
            configs = load_contract(REPO / "impl/python/interop/desired/configs.v1.toml", spec_dir=REPO / "spec")
            author = Owner("lab", "author", [configs], connect=bus.ep1)
            author.start()
            author.set_state(_configs_key(), cf.encode_json(DESIRED12))
            o = _rf0(bus, branch)
            owners.append(o)
            converged = {"observed_revision": 1, "outcome": "converged"}

            def settled(own: Any) -> bool:
                st = bus.get_json(ST12)[0]
                return (st or {}).get("instance") == own.instance and (st or {}).get("desired") == converged \
                    and _duty(st) == 20000 and own.device.values.get("duty", {}).get("fraction_ppm") == 20000

            ok = _wait(lambda: settled(o), 10.0, 0.1)
            rp.check(sec, "setup: converged: St's desired is {observed_revision 1, converged}, and the device "
                          "holds fraction_ppm 20000", ok, f"St {bus.get_json(ST12)[0]}")
            if not ok:
                continue
            old = o.instance
            t1 = time.monotonic()
            n0 = len(o.device.log)
            applies = len(o.device.acts("apply"))
            if branch == "refuse":
                try:
                    o.remint()
                    refused = ""
                except OwnerRefused as e:
                    refused = str(e)
                time.sleep(2.0)
                st = bus.get_json(ST12)[0]
                rp.check(sec, "step 1: the owner refuses the re-mint (config.v1 §2.12): S sees no instance token of "
                              "lab/rf0 withdrawn or declared, St still shows desired {1, converged}, and the "
                              "device holds 20000",
                         bool(refused) and not _tokens12(bus, t1) and (st or {}).get("instance") == old
                         and (st or {}).get("desired") == converged
                         and o.device.values["duty"]["fraction_ppm"] == 20000
                         and len(o.device.acts("apply")) == applies,
                         f"refused {refused!r}; tokens {_tokens12(bus, t1)}; St {st}")
            else:
                new = o.remint()
                ok = _wait(lambda: settled(o) and bool(_applied12(bus, t1, new)), 10.0, 0.1)
                order = [x[1] for x in o.device.log[n0:]]
                toks = _tokens12(bus, t1)
                sts = [d for d in _st12(bus, t1) if d.get("instance") == new]
                wrong = [d for d in sts if d.get("desired") == converged and _duty(d) != 20000]
                rp.check(sec, "step 1: it re-mints as a restart: its device returns to fraction_ppm 10000 first "
                              "(a boot, then the apply); S sees the old instance token withdrawn and a new one "
                              "declared; the new instance acts on revision 1 again: an applied event of it, and "
                              "St shows fraction_ppm 20000 and desired {1, converged}",
                         ok and order[:1] == ["boot"] and "apply" in order
                         and ("delete", f"zk2/{RF0}/@zk/instance/{old}") in toks
                         and ("put", f"zk2/{RF0}/@zk/instance/{new}") in toks,
                         f"device acts {order}; tokens {toks}; events {len(_applied12(bus, t1, new))}; "
                         f"St {bus.get_json(ST12)[0]}")
                rp.check(sec, "step 1: no St of the new instance shows desired converged on revision 1 while it "
                              "says fraction_ppm 10000, and the device never held 10000 under a converged report",
                         bool(sts) and not wrong, f"{len(sts)} St of the new instance; wrong {wrong}")
            # Step 2: the owner's process stops, and starts again.
            last = o.instance
            t2 = time.monotonic()
            o.close()
            owners.remove(o)
            time.sleep(0.5)
            o2 = _rf0(bus, branch, device=o.device, overlay=o.core.overlay, events_at=o.core.events_at)
            owners.append(o2)
            ok = _wait(lambda: settled(o2) and bool(_applied12(bus, t2, o2.instance)), 10.0, 0.1)
            toks = _tokens12(bus, t2)
            sts = [d for d in _st12(bus, t2) if d.get("instance") == o2.instance]
            first = sts[0] if sts else None
            rp.check(sec, "step 2: S sees the old instance token withdrawn and a new one declared; the new "
                          "instance's first St shows fraction_ppm 10000 and no desired (the restart rolled the "
                          "document's value back)",
                     ("delete", f"zk2/{RF0}/@zk/instance/{last}") in toks
                     and ("put", f"zk2/{RF0}/@zk/instance/{o2.instance}") in toks
                     and first is not None and _duty(first) == 10000 and "desired" not in first,
                     f"tokens {toks}; first St {first}")
            rp.check(sec, "the feed raised nothing: no read, seed or act of the owners failed",
                     o.fed_errors == 0 and o2.fed_errors == 0, f"{o.fed_last_error}; {o2.fed_last_error}")
            rp.check(sec, "step 2: the new run acts on revision 1, the version the previous run acted on: an "
                          "applied event of the new instance, St shows fraction_ppm 20000 and desired "
                          "{1, converged}, and the device holds 20000",
                     ok and len(_applied12(bus, t2, o2.instance)) == 1,
                     f"events {_applied12(bus, t2, o2.instance)}; St {bus.get_json(ST12)[0]}; "
                     f"device {o2.device.values.get('duty')}")
        finally:
            for x in owners:
                try:
                    x.close()
                except Exception:  # noqa: BLE001 - closing anyway
                    pass
            if author is not None:
                author.close()
            bus.close()


def section_config_fed(ctx: Ctx) -> None:
    """zk2py's own, beyond scenarios.md: the rest of v1.md §2.16 that zk2py's
    config.v1 owner fed through this profile does, on §12's bus and parties.
    1. revision 1 put: acted on, converged;
    2. the same document put again under a new stamp: adopted, no act, no
       event (§2.7, §2.16);
    3. a document naming a reach group: refused whole, `{reach, radio}`, the
       device unchanged;
    4. a delete: "each parameter named by a document it acted on in this run
       goes back to its startup value", the refused document counting for
       none: `fraction_ppm` back to 10000 by one `set`, `frequency_hz`
       untouched, and the view and status with no `desired`."""
    rp = ctx.report
    sec = ctx.name("config.v1 fed through desired.v1 (§2.16)")
    if ctx.target_impl != "zk2py" or ctx.author_impl != "zk2py":
        rp.cannot(sec, "zk2py's config.v1 owner alone, as §12")
        return
    from . import config as cf
    from .contract import load_contract
    from .owner import Owner

    bus = Bus()
    author = None
    o = None
    try:
        configs = load_contract(REPO / "impl/python/interop/desired/configs.v1.toml", spec_dir=REPO / "spec")
        author = Owner("lab", "author", [configs], connect=bus.ep1)
        author.start()
        key = _configs_key()
        author.set_state(key, cf.encode_json(DESIRED12))
        o = _rf0(bus, "restart")
        conv = {"observed_revision": 1, "outcome": "converged"}
        ok = _wait(lambda: (bus.get_json(ST12)[0] or {}).get("desired") == conv, 10.0, 0.1)
        acts = len(o.fed_acts)
        t = time.monotonic()
        rp.check(sec, "1. revision 1 acted on and converged",
                 ok and acts == 1 and _duty(bus.get_json(ST12)[0]) == 20000,
                 f"acts {o.fed_acts}; St {bus.get_json(ST12)[0]}")
        author.set_state(key, cf.encode_json(DESIRED12))
        time.sleep(1.5)
        rp.check(sec, "2. the same document under a new stamp is adopted: no act and no event",
                 len(o.fed_acts) == acts and not [g for g in bus.since(t) if g[1].startswith(EV12)]
                 and (bus.get_json(ST12)[0] or {}).get("desired") == conv, f"acts {o.fed_acts}")
        refused = {"observed_revision": 2, "outcome": "refused", "refusal": {"code": "reach", "group": "radio"}}
        author.set_state(key, cf.encode_json({"revision": 2, "values": {"duty": {"fraction_ppm": 30000},
                                                                         "radio": {"frequency_hz": 869525000}}}))
        ok = _wait(lambda: (bus.get_json(ST12)[0] or {}).get("desired") == refused, 5.0, 0.1)
        rp.check(sec, "3. a document naming a reach group is refused whole: {reach, radio}, the device unchanged",
                 ok and o.device.values["duty"]["fraction_ppm"] == 20000, f"St {bus.get_json(ST12)[0]}")
        rp.check(sec, "3. the refusal leaves the role free for the next version: no act running, nothing failed "
                      "in the feed (review-bot on PR #963)",
                 not o._fed_running and not o.role.running and o.fed_errors == 0,
                 f"running {o._fed_running}/{o.role.running}; errors {o.fed_errors} {o.fed_last_error}")
        t = time.monotonic()
        author.delete_state(key)
        ok = _wait(lambda: "desired" not in (bus.get_json(ST12)[0] or {"desired": 1})
                   and o.device.values["duty"]["fraction_ppm"] == 10000, 5.0, 0.1)
        st = bus.get_json(ST12)[0]
        evs = [json.loads(g[3]) for g in bus.since(t) if g[1].startswith(EV12) and g[2] == "put"]
        rp.check(sec, "4. a delete reverts what revision 1 named: fraction_ppm back to 10000 by one applied set, "
                      "frequency_hz untouched (the refused revision 2 counts for none), and St with no desired",
                 ok and _duty(st) == 10000 and (st or {}).get("values", {}).get("radio", {}).get("frequency_hz")
                 == 868100000 and len(evs) == 1 and evs[0]["edits"] == [{"group": "duty", "parameter": "fraction_ppm",
                                                                         "old": 20000, "new": 10000}],
                 f"St {st}; events {evs}; acts {o.fed_acts[-1:]}")
    finally:
        if o is not None:
            o.close()
        if author is not None:
            author.close()
        bus.close()


SECTIONS = {"1": section1, "2": section2, "3": section3, "4": section4, "5": section5, "6": section6,
            "7": section7, "8": section8, "9": section9, "10": section10, "edges": section_edges,
            "11": section11, "12": section12, "config-fed": section_config_fed, "remint": section_remint,
            "tool-shapes": section_tool_shapes}


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="python -m zk2py.desired_scenarios", description=__doc__.split("\n")[0])
    ap.add_argument("--author", default="zk2py", help="zk2py, or the owner example's path")
    ap.add_argument("--target", default="zk2py", help="zk2py, or the owner example's path")
    ap.add_argument("--owner", default=None, help="both parties the owner example at this path")
    ap.add_argument("--only", action="append", choices=list(SECTIONS), help="run only these sections (edges: "
                    "zk2py's own documents beyond scenarios.md)")
    ap.add_argument("--poll", action="store_true", help="targets read their author's presence on a period")
    ap.add_argument("--zenctl", type=Path, help="§9's Rust tool (else $ZK2PY_ZENCTL; no default: without either, "
                    "§9 and the shapes fail unless --no-zenctl)")
    ap.add_argument("--no-zenctl", action="store_true", help="run §9 with zk2py's tool alone, its other checks "
                    "counted as not run")
    args = ap.parse_args(argv)
    global ZENCTL, ZENCTL_OFF
    if args.zenctl is not None:
        ZENCTL = args.zenctl
    ZENCTL_OFF = args.no_zenctl
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
          f"{failed} failed" + (f", {len(report.not_run)} not run" if report.not_run else ""))
    for sec, name, ok, detail in report.rows:
        if not ok:
            print(f"FAIL [{sec}] {name}: {detail}")
    return 1 if failed else 0


if __name__ == "__main__":
    rc = main()
    sys.stdout.flush()
    os._exit(rc)

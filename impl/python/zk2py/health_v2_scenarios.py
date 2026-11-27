"""The ``health.v2`` scenarios (``spec/profiles/health/scenarios-v2.md``, text
0.2) with zk2py's reader: ``python -m zk2py.health_v2_scenarios [--owner
zk2py|rust] [--archive stand-in|rust] [--only 1|2|3|4|5|6|7|8|8w|9|epoch|epoch-oh]...``
(#609, rounds 120 and 123).

**The owner** is a process of its own: zk2py's (``zk2py.health_v2_owner``)
by default, or with ``--owner rust`` the reference's owner example, a black
box (:class:`zk2py.health_v2_pairings.RustProc`; round 123, the reference's
owner with zk2py's reader). The router R1, the subscriber S, the GET reader
G and the tool T are this process's, zk2py's in every pairing. A part the
owner example's usage has no word for is zk2py's, and its check says so:
§7's ``lab/liar`` and ``lab/two``.

- **The bus** is the text's: R1 with timestamping on, every other session
  its client. Timeouts are 1 s.
- **A 16-bit run.** ``lab/svc`` keeps a run record, so its puts are the
  text's 12, 14 and 16 bytes. Where an owner draws a 32-bit run each is two
  bytes more.
- **§4's clock** is offset in the owner's runtime, the text's "Without
  root"; its reference is a key this runner puts, which R1 stamps.
- **§5** starts its 100 owners in one process.
- **§6's archive** is ``zk2py.health_scenarios``'s stand-in, or with
  ``--archive rust`` (the default under ``--owner rust``) the reference's
  ``archive`` black box.
- **§7's ``lab/two``** is zk2py's owner with ``--two-asofs``: it answers the
  status's snapshot at one asof and the checks' at a later one (round 123).
- **§8's face** is a router V with access control under ``allow``, whose
  one subject is selected by its link protocol: the owner is V's client over
  TCP, the ground session G over ``unixsock-stream`` (round 123: the owner
  example takes no credentials). ``--only 8`` denies what text 0.2 words;
  ``--only 8w`` what text 0.1 worded, on which its Expected did not hold
  (F-596).
- **``epoch``** runs an owner under a catalog's later epoch, and
  **``epoch-oh``** the same owner confirming its status by owner heartbeat
  (``freshness.v2`` §2.4), read by a reader that holds the catalog.

Exit 0 when every check passes, 1 when any fails.
"""

from __future__ import annotations

import argparse
import json
import os
import queue
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path
from typing import Any

from . import freshness as fr
from . import health as v1
from . import health_v2 as h2
from . import health_v2_live as hl
from . import heartbeat

REPO = Path(__file__).resolve().parents[3]
INTEROP = REPO / "impl" / "python" / "interop" / "health-v2"
NAV = REPO / "examples" / "zk2" / "walkthrough" / "nav.v2.toml"
NS = 1_000_000_000
JITTER_S = 0.5
#: scenarios-v2.md, Conventions: "No owner is given a catalog, so the
#: deployment epoch is 0: a record's since and at are Unix seconds".
#: Every reader of §1 to §9 holds it; :func:`under_a_catalog` holds another.
EPOCH = 0
#: The owner of every section: ``zk2py`` or ``rust`` (``--owner``).
OWNER = "zk2py"
#: §6's archive: ``stand-in`` or ``rust`` (``--archive``).
ARCHIVE = "stand-in"


def make_owner(endpoint: str, address: str, *flags: str, impl: str | None = None) -> Any:
    """One owner process: zk2py's, or the reference's owner example."""
    if (impl or OWNER) == "rust":
        from .health_v2_pairings import RustProc

        return RustProc(endpoint, address, *flags)
    return Proc(endpoint, address, *flags)


class Report:
    def __init__(self) -> None:
        self.rows: list[tuple[str, str, bool, str]] = []
        self.lock = threading.Lock()

    def check(self, section: str, name: str, ok: bool, detail: str = "") -> bool:
        with self.lock:
            self.rows.append((section, name, bool(ok), detail))
            print(f"{'ok  ' if ok else 'FAIL'} {section}: {name}" + (f"\n       {detail}" if detail else ""), flush=True)
        return bool(ok)


class Proc:
    """One owner process, driven by lines."""

    def __init__(self, endpoint: str, address: str, *flags: str):
        env = dict(os.environ, PYTHONPATH=str(REPO / "impl" / "python"))
        self.address = address
        self.p = subprocess.Popen([sys.executable, "-m", "zk2py.health_v2_owner", "--connect", endpoint, *flags,
                                   address], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True, env=env)
        self.ready = self._line(30.0)
        if not self.ready.startswith("ready"):
            raise RuntimeError(f"{address}: {self.ready!r}")
        words = self.ready.split()
        self.run = words[-1] if "run" in words else None

    def _line(self, timeout: float) -> str:
        got: list[str] = []
        t = threading.Thread(target=lambda: got.append(self.p.stdout.readline()), daemon=True)
        t.start()
        t.join(timeout)
        return got[0].strip() if got else ""

    def cmd(self, line: str) -> str:
        assert self.p.stdin is not None
        self.p.stdin.write(line + "\n")
        self.p.stdin.flush()
        return self._line(10.0)

    def state(self) -> dict[str, Any]:
        return json.loads(self.cmd("state"))

    def close(self) -> None:
        if self.p.poll() is None:
            try:
                self.p.stdin.write("close\n")
                self.p.stdin.flush()
                self.p.wait(10)
            except Exception:  # noqa: BLE001 - a process already gone
                self.p.kill()

    def end(self) -> None:
        """The process ends, with no word (scenarios §6 step 2)."""
        self.p.kill()
        self.p.wait(10)


class Bus:
    def __init__(self, drop_future: bool = False) -> None:
        from . import live
        from .live_interop import _r1

        self.router, self.endpoint, self.r1_zid = _r1(drop_future=drop_future)
        self.s = live.open_client(self.endpoint)
        self.g = live.open_client(self.endpoint)
        self.t = live.open_client(self.endpoint)
        self.procs: list[Proc] = []
        self.extra: list[Any] = []
        self.tmp = tempfile.mkdtemp(prefix="zk2py-health-v2-", dir=os.environ.get("ZK2PY_SCRATCH"))

    def owner(self, address: str, *flags: str, impl: str | None = None) -> Any:
        p = make_owner(self.endpoint, address, *flags, impl=impl)
        self.procs.append(p)
        return p

    def listener(self, address: str, selectors: list[str] | None = None, epoch: int = EPOCH) -> hl.Listener:
        s = hl.Listener(self.s, address, selectors, epoch=epoch)
        self.extra.append(s)
        return s

    def close(self) -> None:
        for p in self.procs:
            p.close()
        for e in self.extra:
            try:
                e.close()
            except Exception:  # noqa: BLE001
                pass
        for s in (self.s, self.g, self.t, self.router):
            s.close()


def _sleep_until(mono_ns: int) -> None:
    d = (mono_ns - time.monotonic_ns()) / NS
    if d > 0:
        time.sleep(d)


def _ans(a: h2.Answer) -> str:
    return f"{a.verdict}, {a.reason}" + (f", at {a.level}" if a.level else "")


def _vouches(a: dict[str, Any]) -> bool:
    sec = a.get("read", {}).get("sections", {}).get("state/status")
    return isinstance(sec, dict) and bool(sec.get("confirmed"))


def _wire(s: hl.Listener, x: dict[str, Any]) -> str:
    return f"{x['wire'].hex()} ({len(x['wire'])} B)"


HEALTHY = {"verdict": "healthy", "reason": "ok", "level": "ok"}


# -- §1 ------------------------------------------------------------------------------------

def section1(report: Report) -> None:
    import zenoh

    from . import live

    sec = "§1 bring-up, and both majors"
    bus = Bus()
    try:
        s = bus.listener("lab/svc")
        seen: queue.Queue = queue.Queue()

        def on_token(sample: Any) -> None:
            seen.put((str(sample.kind), str(sample.key_expr)))

        lv, _ = live.watch_or_beside(bus.t, "zk2/lab/*/@zk/**", zenoh.handlers.Callback(on_token), history=True)
        bus.extra.append(type("U", (), {"close": lambda self, lv=lv: lv.undeclare()})())
        answers: dict[str, list[Any]] = {}

        def worker() -> None:
            while True:
                item = seen.get()
                if item is None:
                    return
                kind, key = item
                if "PUT" not in kind:
                    continue
                parts = key.split("/")
                address = "/".join(parts[1:3])
                if parts[4] == "instance":
                    st = live.get_state(bus.t, f"zk2/{address}/health.v2/state/**")
                    answers.setdefault(f"{address} instance", []).append((st, live.get_descriptor(bus.t, key)))
                elif parts[4] == "alive" and parts[5] == "health.v2":
                    st = live.get_state(bus.t, f"zk2/{address}/health.v2/state/**")
                    answers.setdefault(f"{address} alive", []).append((st, None))

        threading.Thread(target=worker, daemon=True).start()
        o = bus.owner("lab/svc", "--run-record", os.path.join(bus.tmp, "svc.run"))
        time.sleep(1.0)

        def entries(st: Any) -> list[Any]:
            out = []
            for snap in st.snapshots:
                if snap.key.endswith("/state/status"):
                    for i in sorted(snap.pages):
                        pg = snap.pages[i]
                        for e in pg["live"]:
                            r = h2.read(h2.STATUS, pg["tag"], bytes.fromhex(e["record"]), EPOCH)
                            out.append((r.expect(), format(pg["run"], "04x" if pg["run_width"] == 16 else "08x"),
                                        e["confirmation"]))
            return out

        got = [entries(st) for k in ("lab/svc instance", "lab/svc alive") for st, _ in answers.get(k, [])]
        # "under the run the owner's descriptor states (core §3.3)"
        d0 = answers.get("lab/svc instance", [(None, None)])[0][1]
        desc = json.loads(d0[0].payload) if d0 and d0[0].ok else {}
        if o.run is None:
            o.run = desc.get("run")
        first = [x for x in s.of("state/status") if x["kind"] == "put"]
        r0 = s.record(first[0]) if first else None
        report.check(sec, "step 1: both of T's GETs, at the instance token and at the alive/health.v2 token, answer "
                          "the status's snapshot: one entry, unknown with the reason 1, under the run the owner's "
                          "descriptor states, with its confirmation",
                     len(got) == 2 and desc.get("run") == o.run
                     and all(len(x) == 1 and x[0][0].get("level") == "unknown"
                                           and x[0][0].get("reason") == 1 and x[0][1] == o.run
                                           and x[0][2] in ("time", "conf") for x in got),
                     f"{got}; the owner's run {o.run}, its descriptor's {desc.get('run')}")
        report.check(sec, "step 1: its put, as S heard it, is 12 bytes under a 16-bit run: CH 0x03, the run, the "
                          "seq, and a record of 6 bytes",
                     len(first) == 1 and len(first[0]["wire"]) == 12 and first[0]["wire"][0] == 0x03
                     and r0 is not None and r0.expect().get("level") == "unknown",
                     f"{[_wire(s, x) for x in first]}; {None if r0 is None else r0.expect()}")
        first_since = None if r0 is None else r0.at_ms
        # Step 2.
        bus.owner("lab/quiet", "--level", "ok", "--reason", "serving", "--tokenless", "health.v2",
                  "--tokenless", "freshness.v2")
        time.sleep(1.0)
        pres = live.list_presence(bus.t, "zk2/lab/quiet/@zk/**")
        qa = answers.get("lab/quiet instance", [])
        doc = json.loads(qa[0][1][0].payload) if qa and qa[0][1] and qa[0][1][0].ok else {}
        entry = next((e for e in doc.get("interfaces") or [] if e.get("iface") == "health.v2"), {})
        st_q = entries(qa[0][0]) if qa else []
        report.check(sec, "step 2: lab/quiet holds an instance token and no alive/health.v2 or alive/freshness.v2 "
                          "token, its descriptor lists health.v2 with \"token\": false, and T's GET at the instance "
                          "token answers its status ok",
                     len(pres.instances) == 1 and not [a for a in pres.alive if a["iface"] in ("health.v2",
                                                                                            "freshness.v2")]
                     and entry.get("token") is False and len(st_q) == 1 and st_q[0][0].get("level") == "ok",
                     f"{len(pres.instances)} instance, alive {[a['iface'] for a in pres.alive]}, entry {entry}, "
                     f"status {st_q}")
        # Step 3.
        time.sleep(1.1)   # a second later, so that since can be seen to move
        o.cmd("status ok serving")
        time.sleep(0.3)
        g = entries(live.get_state(bus.g, "zk2/lab/svc/health.v2/state/**"))
        report.check(sec, "step 3: G's entry is ok with the reason 2, its since not before the first status's",
                     len(g) == 1 and g[0][0].get("level") == "ok" and g[0][0].get("reason") == 2
                     and first_since is not None and g[0][0].get("since", 0) >= first_since,
                     f"{g}; the first since {first_since}")
        # Step 4.
        mark = time.monotonic_ns()
        time.sleep(65.0)
        puts = [x for x in s.of("state/status", mark) if x["kind"] == "put"]
        beats = s.beats(mark)
        arrivals = [mark] + [t for t, _ in beats]
        gaps = [(b - a) / NS for a, b in zip(arrivals, arrivals[1:])]
        raw = [x for x in _heartbeat_wires(bus, "lab/svc")]
        r4, _ = hl.read_near(bus.g, "lab/svc", listener=s, get=False)
        a4 = h2.judge(r4)
        report.check(sec, "step 4: in 65 s S receives no put of the status, a heartbeat at least every 30 s, of form "
                          "6, its tag 1, its section vouching for the status, and judges healthy, ok, past the "
                          "horizon",
                     not puts and bool(beats) and max(gaps) <= 30 + JITTER_S and all(_vouches(a) for _, a in beats)
                     and a4.expect() == HEALTHY,
                     f"{len(puts)} puts, {len(beats)} heartbeats, gaps {[round(x, 2) for x in gaps]} s; {_ans(a4)}")
        report.check(sec, "step 4: the heartbeat's payload is 16 bytes under a 16-bit run, CH 0xc1",
                     bool(raw) and all(len(w) == 16 and w[0] == 0xC1 for w in raw), f"{[w.hex() for w in raw[:2]]}")
        # Step 5: lab/old, health.v1's revision 1.1: zk2py's in this process,
        # or the owner example with --health
        if OWNER == "rust":
            bus.owner("lab/old", "--v1", "--level", "ok", "--reason", "serving")
        else:
            old = v1.HealthOwner("lab", "old", level=v1.OK, reason="serving", connect=bus.endpoint)
            old.start()
            bus.extra.append(old)
        time.sleep(1.0)
        st2 = live.get_state(bus.t, "zk2/lab/*/health.v2/state/**")
        st1 = live.get_state(bus.t, "zk2/lab/*/health.v1/state/**")
        judged: dict[str, Any] = {}
        majors: dict[str, Any] = {}
        for addr in ("lab/svc", "lab/quiet", "lab/old"):
            r, info = hl.read_near(bus.t, addr)
            majors[addr] = h2.major_of(r.interfaces)
            if majors[addr] == h2.V1:
                rv1, _ = v1.read_near(bus.t, addr, trust=fr.ClockTrust(word=True))
                a1 = v1.judge(rv1)
                judged[addr] = v1.Answer(a1.verdict, a1.reason, a1.level)
            else:
                judged[addr] = h2.judge(r)
        roll = v1.rollup([v1.Answer(a.verdict, a.reason, a.level) for a in judged.values()])
        report.check(sec, "step 5: lab/svc's descriptor lists health.v2 and not health.v1, and lab/old's the "
                          "reverse; T judges lab/svc and lab/quiet under this text and lab/old under health.v1's, "
                          "each healthy, ok; its roll-up counts three healthy services",
                     majors == {"lab/svc": h2.V2, "lab/quiet": h2.V2, "lab/old": h2.V1}
                     and all(a.expect() == HEALTHY for a in judged.values()) and roll["counts"]["healthy"] == 3,
                     f"{majors}; {[(k, a.expect()) for k, a in judged.items()]}; roll-up {roll}; the two reads "
                     f"{sorted({x.key for x in st2.replies})} and {sorted({x.key for x in st1.replies})}")
        # Step 6.
        o.close()
        time.sleep(3.0)
        deletes = [x for x in s.of("state/status") if x["kind"] == "delete"]
        report.check(sec, "step 6: S receives no delete of the status, before or after the close",
                     not deletes, f"{len(deletes)} deletes of {len(s.of('state/status'))} deliveries")
        seen.put(None)
    finally:
        bus.close()


_HB: dict[int, list[bytes]] = {}


def _heartbeat_wires(bus: Bus, address: str) -> list[bytes]:
    """The heartbeats' payloads as they came, heard for 31 s on a subscriber
    of its own (the listener keeps what it read of them, not their bytes)."""
    import zenoh

    got: list[bytes] = []
    sub = bus.g.declare_subscriber(heartbeat.heartbeat_key(*address.split("/"), h2.IFACE),
                                   zenoh.handlers.Callback(lambda smp: got.append(smp.payload.to_bytes())))
    deadline = time.monotonic() + 31.0
    while not got and time.monotonic() < deadline:
        time.sleep(0.1)
    sub.undeclare()
    return got


# -- §2 ------------------------------------------------------------------------------------

def section2(report: Report) -> None:
    from . import live

    sec = "§2 a stale status"
    bus = Bus()
    try:
        s = bus.listener("lab/svc")
        o = bus.owner("lab/svc", "--level", "ok", "--reason", "serving")
        t0 = time.monotonic_ns()
        time.sleep(65.0)
        rs, _ = hl.read_near(bus.g, "lab/svc", listener=s, get=False)
        rg, info = hl.read_near(bus.t, "lab/svc")
        puts = [x for x in s.of("state/status", t0 + NS) if x["kind"] == "put"]
        conf = info["joined"].confirmation
        report.check(sec, "step 1: S and G judge healthy, ok, past the horizon; S has received no put of the status "
                          "since step 1's; G's entry carries confirmation 1, vouched for at the snapshot's time",
                     h2.judge(rs).expect() == HEALTHY and h2.judge(rg).expect() == HEALTHY and not puts
                     and conf == "time", f"S {_ans(h2.judge(rs))}; G {_ans(h2.judge(rg))}; {len(puts)} puts; "
                                         f"confirmation {conf}")
        last_vouch = max((t for t, a in s.beats() if _vouches(a)), default=time.monotonic_ns())
        o.cmd("unvouch")
        t2 = time.monotonic_ns()
        _sleep_until(max(last_vouch, t2) + 65 * NS)
        later = s.beats(t2 + NS)
        puts2 = [x for x in s.of("state/status", t2) if x["kind"] == "put"]
        report.check(sec, "step 2: S receives heartbeats, each with the status's bit clear, and no put",
                     bool(later) and not any(_vouches(a) for _, a in later) and not puts2,
                     f"{len(later)} heartbeats, {sum(_vouches(a) for _, a in later)} vouching, {len(puts2)} puts")
        pres = live.list_presence(bus.t, "zk2/lab/svc/@zk/**")
        rs3, _ = hl.read_near(bus.g, "lab/svc", listener=s, get=False)
        rg3, info3 = hl.read_near(bus.t, "lab/svc")
        a_s, a_g = h2.judge(rs3), h2.judge(rg3)
        report.check(sec, "step 3: the owner's tokens are present; S: stale, beyond_horizon; G: stale, "
                          "beyond_horizon, its entry with confirmation 2; neither unhealthy, failed, nor ok as "
                          "current",
                     len(pres.instances) == 1 and a_s.expect() == {"verdict": "stale", "reason": "beyond_horizon",
                                                                   "level": None}
                     and a_g.expect() == a_s.expect() and info3["joined"].confirmation == "conf",
                     f"S {_ans(a_s)}; G {_ans(a_g)}; confirmation {info3['joined'].confirmation}")
    finally:
        bus.close()


# -- §3 ------------------------------------------------------------------------------------

def section3(report: Report) -> None:
    sec = "§3 aggregation, by seq"
    bus = Bus()
    try:
        s = bus.listener("lab/svc")
        o = bus.owner("lab/svc", "--level", "ok", "--reason", "serving")
        o.cmd("check disk ok ok")
        o.cmd("check net ok ok")
        time.sleep(0.5)
        t2 = time.monotonic_ns()
        o.cmd("check disk failed timeout")
        time.sleep(0.5)
        heard = [(x["key"].split("/health.v2/")[1], s.stamp(x), s.record(x)) for x in s.all(t2)]
        st = next((h for h in heard if h[0] == "state/status"), None)
        ck = next((h for h in heard if h[0] == "state/checks/disk"), None)
        # "At no delivery is S's latest status better than its latest disk"
        never_better, latest = True, {"status": None, "disk": None}
        for x in s.all():
            rel = x["key"].split("/health.v2/")[1]
            if x["kind"] != "put" or rel not in ("state/status", "state/checks/disk"):
                continue
            latest["status" if rel == "state/status" else "disk"] = s.record(x).level
            if latest["status"] and latest["disk"] and \
                    h2.LEVELS.index(latest["disk"]) > h2.LEVELS.index(latest["status"]):
                never_better = False
        report.check(sec, "step 2: S receives the status failed, with the reason check, before checks/disk failed; "
                          "both of one run, the status's seq the lower; at no delivery is S's latest status better "
                          "than its latest disk",
                     st is not None and ck is not None and heard.index(st) < heard.index(ck)
                     and st[2].expect().get("level") == "failed" and st[2].word == 4
                     and ck[2].expect().get("level") == "failed" and ck[2].word == 3
                     and st[1].same_run(ck[1]) and st[1].seq < ck[1].seq and never_better,
                     f"{[(k, str(i), r.expect()) for k, i, r in heard]}")
        rg, info = hl.read_near(bus.g, "lab/svc")
        j = info["joined"]
        report.check(sec, "step 2: G's answer is two snapshots under one run and one asof, with no entry above it: "
                          "one instant; it holds status failed, disk failed and net ok, and G judges unhealthy, "
                          "failed",
                     j.instant and j.status is not None and j.status.level == "failed"
                     and {k: r.level for k, r in j.checks.items()} == {"disk": "failed", "net": "ok"}
                     and h2.judge(rg).expect() == {"verdict": "unhealthy", "reason": "failed", "level": "failed"},
                     f"instant {j.instant} ({j.why_not}); {_ans(h2.judge(rg))}; "
                     f"{ {k: r.level for k, r in j.checks.items()} }")
        t3 = time.monotonic_ns()
        o.cmd("check disk ok ok")
        o.cmd("status ok serving")
        time.sleep(0.5)
        heard3 = [(x["key"].split("/health.v2/")[1], s.stamp(x), s.record(x)) for x in s.all(t3)]
        rg3, _ = hl.read_near(bus.g, "lab/svc")
        report.check(sec, "step 3: S receives checks/disk ok before the status ok, the check's seq the lower; G "
                          "judges healthy, ok",
                     [k for k, _, _ in heard3] == ["state/checks/disk", "state/status"]
                     and heard3[0][1].seq < heard3[1][1].seq and h2.judge(rg3).expect() == HEALTHY,
                     f"{[(k, str(i), r.expect()) for k, i, r in heard3]}; {_ans(h2.judge(rg3))}")
        t4 = time.monotonic_ns()
        o.cmd("retire net")
        time.sleep(0.5)
        heard4 = [(x["key"].split("/health.v2/")[1], x["kind"]) for x in s.all(t4)]
        rg4, info4 = hl.read_near(bus.g, "lab/svc")
        j4 = info4["joined"]
        check_puts = [x["key"].split("/checks/")[1] for x in s.all() if "/checks/" in x["key"] and x["kind"] == "put"]
        report.check(sec, "step 4: S receives a delete of checks/net, and no status put for it; G's answer holds the "
                          "status and disk, and net as a tombstone; no check is put again at any point",
                     heard4 == [("state/checks/net", "delete")] and sorted(j4.checks) == ["disk"]
                     and j4.retired == ("net",) and j4.status is not None
                     and check_puts == ["disk", "net", "disk", "disk"],
                     f"{heard4}; checks {sorted(j4.checks)}, retired {j4.retired}; check puts {check_puts}")
    finally:
        bus.close()


# -- §4 ------------------------------------------------------------------------------------

def _clock_run(report: Report, sec: str, drop_future: bool, short: bool) -> None:
    import zenoh

    bus = Bus(drop_future=drop_future)
    ref = "zk2py/health-v2/reference"
    stop = threading.Event()
    try:
        s = bus.listener("lab/ahead")
        o = bus.owner("lab/ahead", "--level", "ok", "--reason", "serving", "--reference", ref, "--offset-ms", "2000")
        time.sleep(1.0)
        tag = "step 6 (drop_future_timestamp)" if drop_future else "step"
        if not drop_future:
            # before any put of the reference, the owner has not detected: its
            # heartbeat is its first, sent at its tokens
            deadline = time.monotonic() + 31
            while not s.beats() and time.monotonic() < deadline:
                time.sleep(0.2)
            b1 = s.beats()
            report.check(sec, "step 1: S receives the status; each heartbeat it reads carries an instant, and S "
                              "answers the clock question no, confirmed",
                         len([x for x in s.of("state/status") if x["kind"] == "put"]) == 1 and bool(b1)
                         and all(heartbeat.reading_of(a.get("time")) == heartbeat.INSTANT for _, a in b1)
                         and s.clock() == ("no", "confirmed"), f"{len(b1)} heartbeats; {s.clock()}")

        def reference() -> None:
            while not stop.wait(0.3):
                bus.g.put(ref, b"beat")     # no stamp of its own: R1 stamps it

        threading.Thread(target=reference, daemon=True).start()
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline and not o.state()["ahead"]:
            time.sleep(0.05)
        t_det = time.monotonic_ns()
        if not report.check(sec, f"{tag} 2: the owner reports itself ahead from the router-stamped reference",
                            o.state()["ahead"], str(o.state()["events"])):
            return
        put = o.cmd("status ok fallback")
        time.sleep(1.0)
        faults = [(x, s.record(x)) for x in s.of("stream/faults")]
        f0 = faults[0][1].expect() if faults else {}
        owner_zid = None
        report.check(sec, f"{tag} 2: within 1 s of the detection S receives one faults sample: the code 1, the level "
                          "failed, an argument of 2 or 3; its put carries no Zenoh timestamp of the owner's",
                     len(faults) == 1 and faults[0][0]["at"] <= t_det + NS and f0.get("code") == 1
                     and f0.get("level") == "failed" and f0.get("argument") in (2, 3)
                     and (faults[0][0]["timestamp"] is None or faults[0][0]["timestamp"].endswith(bus.r1_zid)),
                     f"{[(r.expect(), x['timestamp'], len(x['wire'])) for x, r in faults]}; R1 {bus.r1_zid}")
        sp = [s.record(x) for x in s.of("state/status", t_det) if x["kind"] == "put"]
        report.check(sec, f"{tag} 2: the guard does not hold the status: the owner's put succeeds, and S receives it",
                     put.startswith("ok") and len(sp) == 1 and sp[0].word == 6, f"{put}; {[r.expect() for r in sp]}")
        wait = 35 if (drop_future or short) else 65
        late = None
        if not drop_future:
            _sleep_until(t_det + 35 * NS)
            late = hl.Listener(bus.t, "lab/ahead", [])      # L: the heartbeat key alone
            bus.extra.append(late)
            t_l = time.monotonic_ns()
        _sleep_until(t_det + wait * NS)
        beats = s.beats(t_det + 300_000_000)
        ahead_faults = [r for _, r in ((x, s.record(x)) for x in s.of("stream/faults")) if r.word == 1]
        first_ahead = next((t for t, a in beats if a.get("time") == heartbeat.AHEAD), None)
        report.check(sec, f"{tag} 2: S receives no second clock_ahead fault in the {wait} s; every heartbeat from the "
                          "detection on carries the ahead time, never all ones, and vouches for the status",
                     len(ahead_faults) == 1 and bool(beats)
                     and all(a.get("time") == heartbeat.AHEAD and _vouches(a) for _, a in beats),
                     f"{len(ahead_faults)} clock_ahead faults; {len(beats)} heartbeats, times "
                     f"{[a.get('time') for _, a in beats]}")
        rs, _ = hl.read_near(bus.g, "lab/ahead", listener=s, get=False)
        report.check(sec, f"{tag} 2: from its first such heartbeat, within 30 s of the detection, S answers the clock "
                          f"question yes, ahead; at {wait} s S judges healthy, ok; the owner's tokens stay present",
                     first_ahead is not None and (first_ahead - t_det) / NS <= 30 + JITTER_S
                     and s.clock() == ("yes", "ahead") and h2.judge(rs).expect() == HEALTHY,
                     f"first ahead heartbeat {None if first_ahead is None else round((first_ahead - t_det) / NS, 1)} s"
                     f" after the detection; {s.clock()}; {_ans(h2.judge(rs))}")
        if drop_future:
            return
        _sleep_until(t_l + 30 * NS)
        assert late is not None
        lb = late.beats()
        report.check(sec, "step 3: L hears no fault; from the first heartbeat it reads, within 30 s, L answers yes, "
                          "ahead",
                     not late.all() and bool(lb) and (lb[0][0] - t_l) / NS <= 30 + JITTER_S
                     and late.clock() == ("yes", "ahead"),
                     f"{len(late.all())} deliveries beside the heartbeats; {len(lb)} heartbeats; {late.clock()}")
        # Step 4: the clock set right.
        o.cmd("clock 0")
        t_fix = time.monotonic_ns()
        time.sleep(35.0)
        inst = [t for t, a in s.beats(t_fix) if heartbeat.reading_of(a.get("time")) == heartbeat.INSTANT]
        n_ahead = len([r for _, r in ((x, s.record(x)) for x in s.of("stream/faults")) if r.word == 1])
        rs4, _ = hl.read_near(bus.g, "lab/ahead", listener=s, get=False)
        report.check(sec, "step 4: within 30 s of the correction S receives a heartbeat whose time is an instant "
                          "again, and no clock_ahead fault after step 2's; S judges healthy, ok, and the clock "
                          "question no, confirmed",
                     bool(inst) and (inst[0] - t_fix) / NS <= 30 + JITTER_S and n_ahead == 1
                     and h2.judge(rs4).expect() == HEALTHY and s.clock() == ("no", "confirmed"),
                     f"first instant {None if not inst else round((inst[0] - t_fix) / NS, 1)} s after; {n_ahead} "
                     f"clock_ahead faults; {_ans(h2.judge(rs4))}; {s.clock()}")
        # Step 5: ahead again, a new hold.
        o.cmd("clock 2000")
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline and not o.state()["ahead"]:
            time.sleep(0.05)
        t5 = time.monotonic_ns()
        time.sleep(35.0)
        new = [(x["at"] - t5) / NS for x in s.of("stream/faults", t5 - 2 * NS) if s.record(x).word == 1]
        b5 = s.beats(t5 + 300_000_000)
        report.check(sec, "step 5: within 1 s of the new detection S receives one more clock_ahead fault, a new "
                          "hold; its heartbeats carry the ahead time again",
                     len(new) == 1 and new[0] <= 1.0 and bool(b5) and all(a.get("time") == heartbeat.AHEAD
                                                                          for _, a in b5)
                     and o.state()["holds"] == 2,
                     f"faults at {[round(x, 2) for x in new]} s; {len(b5)} heartbeats; holds {o.state()['holds']}")
    finally:
        stop.set()
        bus.close()


def section4(report: Report) -> None:
    _clock_run(report, "§4 clock ahead, answered by a heartbeat", False, False)
    _clock_run(report, "§4 clock ahead, answered by a heartbeat", True, False)


# -- §5 ------------------------------------------------------------------------------------

def section5(report: Report, n: int = 100) -> None:
    from . import live

    sec = f"§5 a tokenless set of {n}"
    bus = Bus()
    try:
        bus.owner("p5/x", "--many", str(n), "--nav", str(NAV), "--tokenless", "health.v2",
                  "--tokenless", "freshness.v2")
        time.sleep(2.0)
        pres = live.list_presence(bus.t, "zk2/p5/*/@zk/**")
        second = live.list_presence(bus.t, "zk2/p5/*/@zk/alive/health.v2/**")
        fv2 = live.list_presence(bus.t, "zk2/p5/*/@zk/alive/freshness.v2/**")
        report.check(sec, f"{2 * n} tokens: an instance token and an alive/nav.v2 token each, and no "
                          "alive/health.v2 or alive/freshness.v2 token; the second read finds nothing",
                     len(pres.instances) == n and len(pres.alive) == n
                     and all(a["iface"] == "nav.v2" for a in pres.alive) and pres.complete
                     and second.count == 0 and second.complete and fv2.count == 0,
                     f"{len(pres.instances)} instance, {len(pres.alive)} alive "
                     f"({sorted({a['iface'] for a in pres.alive})}), second read {second.count}")
        tokenless = healthy = 0
        for i in range(n):
            r, info = hl.read_near(bus.t, f"p5/dev{i}")
            e = next((e for e in info.get("descriptor", {}).get("interfaces", []) if e.get("iface") == "health.v2"), {})
            tokenless += int(e.get("token") is False)
            healthy += int(h2.judge(r).expect() == HEALTHY)
        report.check(sec, f"every descriptor lists health.v2 with \"token\": false; T finds all {n} providers, and "
                          "judges each healthy, ok", tokenless == n and healthy == n,
                     f"{tokenless} tokenless, {healthy} healthy")
    finally:
        bus.close()


# -- §6 ------------------------------------------------------------------------------------

class ReferenceArchive:
    """The reference's ``archive`` black box (its usage): ``archive
    --connect <R1> --system <s> --service <a> --record <owner> <selector>
    health.v2.toml``, up once it says ``aligned``. Its archive form is core
    §4.4's, as the stand-in's."""

    def __init__(self, endpoint: str, address: str, owner: str, selector: str):
        from .blackbox import BLACKBOX

        system, service = address.split("/")
        self.address = address
        self.p = subprocess.Popen([str(BLACKBOX / "archive"), "--connect", endpoint, "--system", system,
                                   "--service", service, "--record", owner, selector, str(h2.CONTRACT)],
                                  stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        self.lines: list[str] = []
        self.stderr: list[str] = []
        threading.Thread(target=lambda: [self.lines.append(x.rstrip("\n")) for x in self.p.stdout],
                         daemon=True).start()
        threading.Thread(target=lambda: [self.stderr.append(x) for x in self.p.stderr], daemon=True).start()
        deadline = time.monotonic() + 30
        while time.monotonic() < deadline and not any(x.startswith("aligned") for x in self.lines):
            if self.p.poll() is not None:
                raise RuntimeError(f"the archive did not start: {self.stderr[-3:]}")
            time.sleep(0.1)

    def form(self, key: str) -> str:
        from .slug import slug

        return f"zk2/{self.address}/archive.v1/@state/" + "/".join(slug(c) for c in key.split("/")[1:])

    def close(self) -> None:
        try:
            self.p.stdin.write("quit\n")
            self.p.stdin.close()
        except (OSError, ValueError):
            pass
        try:
            self.p.wait(15)
        except subprocess.TimeoutExpired:
            self.p.kill()
            self.p.wait()


def section6(report: Report) -> None:
    import zenoh

    from . import zk2c
    from .health_scenarios import StandInArchive

    sec = f"§6 an absent owner ({'the reference archive' if ARCHIVE == 'rust' else 'the stand-in archive'})"
    bus = Bus()
    try:
        o = bus.owner("lab/svc", "--level", "degraded", "--reason", "upstream_lost")
        c = hl.standard()
        status_r = next(r for r in c.canonical["resources"] if r["template"] == "status")
        if ARCHIVE == "rust":
            archive: Any = ReferenceArchive(bus.endpoint, "lab/archive", "lab/svc", "zk2/lab/svc/health.v2/state/**")
        else:
            archive = StandInArchive(bus.endpoint, "lab/archive", "zk2/lab/svc/health.v2/state/**",
                                     {"iface": "health.v2", "contract": c.fingerprint, "type": status_r["type"]})
        bus.extra.append(archive)
        time.sleep(0.5)
        r1, info1 = hl.read_near(bus.t, "lab/svc")
        report.check(sec, "step 1: unhealthy, degraded; T shows the reason as upstream_lost",
                     h2.judge(r1).expect() == {"verdict": "unhealthy", "reason": "degraded", "level": "degraded"}
                     and hl.shown(r1.status, info1["tables"]) == "degraded, upstream_lost",
                     f"{_ans(h2.judge(r1))}; shown {hl.shown(r1.status, info1['tables'])!r}")
        o.end()
        time.sleep(1.5 if ARCHIVE != "rust" else 3.0)
        r3, info3 = hl.read_near(bus.t, "lab/svc")
        form = archive.form("zk2/lab/svc/health.v2/state/status")
        got: list[Any] = []
        done = threading.Event()
        bus.t.get(form, zenoh.handlers.Callback(got.append, done.set), target=zenoh.QueryTarget.ALL,
                  consolidation=zenoh.ConsolidationMode.NONE, timeout=1.0)
        done.wait(3.0)
        shown = []
        for rep in got:
            if rep.ok is None:
                continue
            att = json.loads(rep.ok.attachment.to_bytes()) if rep.ok.attachment is not None else {}
            rec = h2.read_put(h2.STATUS, rep.ok.payload.to_bytes(), EPOCH,
                              zk2c.envelope_encoding(str(rep.ok.encoding)))
            shown.append({"last_known": hl.shown(rec, info3["tables"]), "confirmed": att.get("confirmed")})
        a3 = h2.judge(r3)
        st = info3.get("state")
        report.check(sec, "step 3: the owner's tokens are gone, and its GET draws no reply; T judges not asked, "
                          "absent: never failed, never stale; it shows the archived status, degraded, as last-known, "
                          "never as current (core S6)",
                     r3.presence == "absent" and st is not None and not st.replies
                     and a3.expect() == {"verdict": "not_asked", "reason": "absent", "level": None}
                     and len(shown) == 1 and shown[0]["last_known"] == "degraded, upstream_lost",
                     f"{_ans(a3)}; {len(st.replies) if st else None} replies; last-known {shown}; asked {form}")
    finally:
        bus.close()


# -- §7 ------------------------------------------------------------------------------------

def section7(report: Report) -> None:
    sec = "§7 an inconsistent status, from one reading"
    bus = Bus()
    try:
        # a test owner that breaks §2.2: zk2py's in every pairing (the owner
        # example's usage has none)
        bus.owner("lab/liar", "--level", "ok", "--reason", "serving", "--liar", "--check", "disk=failed:timeout",
                  impl="zk2py")
        bus.owner("lab/frank", "--level", "failed", "--reason", "misconfigured", "--check", "disk=ok:ok")
        flip = bus.owner("lab/flip", "--level", "ok", "--reason", "serving", "--check", "disk=ok:ok")
        time.sleep(0.5)
        rl, il = hl.read_near(bus.t, "lab/liar")
        rf, if_ = hl.read_near(bus.t, "lab/frank")
        jl, jf = il["joined"], if_["joined"]
        report.check(sec, "step 1: each reading's two snapshots are under one run and one asof, with no entry above "
                          "it: one instant",
                     jl.instant and jf.instant, f"liar {jl.instant} ({jl.why_not}), frank {jf.instant} ({jf.why_not})")
        report.check(sec, "step 1: lab/liar (zk2py's): unhealthy, inconsistent, at failed, never healthy; T reports "
                          "the break of §2.2 as a finding about the owner, from this one reading",
                     h2.judge(rl).expect() == {"verdict": "unhealthy", "reason": "inconsistent", "level": "failed"}
                     and jl.breaks and jl.report == "finding", f"{_ans(h2.judge(rl))}; report {jl.report}")
        report.check(sec, "step 1: lab/frank: unhealthy, failed, and no break",
                     h2.judge(rf).expect() == {"verdict": "unhealthy", "reason": "failed", "level": "failed"}
                     and not jf.breaks, f"{_ans(h2.judge(rf))}; breaks {jf.breaks}")
        flip.cmd("flip 100")
        rows = []
        for _ in range(50):
            t = time.monotonic()
            r, info = hl.read_near(bus.t, "lab/flip")
            j = info["joined"]
            rows.append((j.instant, j.breaks, j.status.level if j.status else None,
                         j.checks["disk"].level if "disk" in j.checks else None, j.why_not))
            time.sleep(max(0.0, 0.037 - (time.monotonic() - t)))
        done = flip.cmd("flip stop")
        seen = sorted({(a, b) for _, _, a, b, _ in rows}, key=str)
        report.check(sec, "step 2: every reading of lab/flip is one instant, and none shows a status better than its "
                          "check; T reports no break",
                     all(r[0] for r in rows) and not any(r[1] for r in rows) and len(seen) > 1,
                     f"{sum(r[0] for r in rows)} of {len(rows)} one instant ({sorted({str(r[4]) for r in rows})}), "
                     f"{sum(r[1] for r in rows)} breaks; (status, disk) seen {seen}; {done}")
        # Step 3: lab/two, zk2py's test owner in every pairing (round 123).
        bus.owner("lab/two", "--level", "ok", "--reason", "serving", "--liar", "--two-asofs",
                  "--check", "disk=failed:timeout", impl="zk2py")
        time.sleep(0.5)
        r1_, i1 = hl.read_near(bus.t, "lab/two")
        j1 = i1["joined"]
        a1 = h2.judge(r1_)
        report.check(sec, "step 3: lab/two (zk2py's): T's first reading is not one instant; T judges it unhealthy, "
                          "inconsistent, at failed, and reports no break from it alone",
                     not j1.instant and a1.expect() == {"verdict": "unhealthy", "reason": "inconsistent",
                                                        "level": "failed"}
                     and j1.breaks and j1.report == "two_readings",
                     f"instant {j1.instant} ({j1.why_not}); snapshots' asofs "
                     f"{[None if x is None else x.get('asof') for x in i1['snapshots']]}; {_ans(a1)}; "
                     f"report {j1.report}")
        time.sleep(2.0)
        r2_, i2 = hl.read_near(bus.t, "lab/two")
        j2 = i2["joined"]
        both = j1.breaks and j2.breaks
        report.check(sec, "step 3: T reports the break after the second reading, a grace apart (§2.2)",
                     not j2.instant and both and h2.judge(r2_).expect() == a1.expect(),
                     f"second reading: instant {j2.instant}, breaks {j2.breaks}; the break seen in both: {both}")
    finally:
        bus.close()


# -- §8 ------------------------------------------------------------------------------------

#: §8's face, beside the setup's ``zk2/**/@zk/**``, read two ways.
#: ``listed``: the denies text 0.2 words (F-596 answered): run A the faults'
#: key, run B the state, the heartbeats and the faults' key, run C the state
#: and the heartbeats. ``worded``: what text 0.1 worded, on which its
#: Expected did not hold (F-596): runs B and C deny the state alone.
FAULTS_KEY = "zk2/vehicle-01/nav/health.v2/stream/faults"
FACES = {
    "worded": {"A": [], "B": ["zk2/vehicle-01/*/*/state/**"], "C": ["zk2/vehicle-01/*/*/state/**"]},
    "listed": {"A": [FAULTS_KEY],
               "B": ["zk2/vehicle-01/*/*/state/**", "zk2/vehicle-01/*/freshness.v2/stream/**", FAULTS_KEY],
               "C": ["zk2/vehicle-01/*/*/state/**", "zk2/vehicle-01/*/freshness.v2/stream/**"]},
}


def face_router(workdir: str, denied: list[str]) -> tuple[Any, str, str]:
    """V: a router with access control under ``allow``, denying ``denied``
    both ways, every message, to its one subject, selected by its link
    protocol: what comes over ``unixsock-stream`` is the ground's. The owner
    is its client over TCP, as the owner example takes no credentials:
    (router, the TCP endpoint, the face's endpoint)."""
    import zenoh

    from .owner import free_loopback_port

    port = free_loopback_port()
    sock = os.path.join(workdir, f"v{len(denied)}.sock")
    conf = zenoh.Config()
    conf.insert_json5("mode", json.dumps("router"))
    conf.insert_json5("listen/endpoints", json.dumps([f"tcp/127.0.0.1:{port}", f"unixsock-stream/{sock}"]))
    conf.insert_json5("scouting/multicast/enabled", "false")
    conf.insert_json5("timestamping/enabled", "true")
    messages = ["put", "delete", "declare_subscriber", "query", "declare_queryable", "reply",
                "liveliness_token", "declare_liveliness_subscriber", "liveliness_query"]
    conf.insert_json5("access_control", json.dumps({
        "enabled": True, "default_permission": "allow",
        "rules": [{"id": "face", "messages": messages, "flows": ["ingress", "egress"], "permission": "deny",
                   "key_exprs": denied}],
        "subjects": [{"id": "ground", "link_protocols": ["unixsock-stream"]}],
        "policies": [{"id": "the-face", "rules": ["face"], "subjects": ["ground"]}],
    }))
    return zenoh.open(conf), f"tcp/127.0.0.1:{port}", f"unixsock-stream/{sock}"


def _face_run(report: Report, workdir: str, run: str, wait_s: float, face: str = "listed") -> None:
    from . import live

    sec = f"§8 across a constrained face ({face}), run {run}"
    crosses = run == "A"
    denied = ["zk2/**/@zk/**"] + FACES[face][run]
    v, ep, face_ep = face_router(os.path.join(workdir, run), denied)
    ground = live.open_client(face_ep)
    addr = "vehicle-01/nav"
    ref = "zk2py/health-v2/reference"
    stop = threading.Event()
    o = None
    s = None
    try:
        # "G subscribes to the owner's status, its heartbeat key and its
        # faults statically, without presence, before the owner starts."
        s = hl.Listener(ground, addr, [f"zk2/{addr}/health.v2/state/status", f"zk2/{addr}/health.v2/stream/faults"])
        t0 = time.monotonic_ns()
        _sleep_until(t0 + int(wait_s * NS))
        a1 = h2.judge(hl.read_across(s, addr, crosses))
        want = "nothing_crossed" if crosses else "face_closed"
        report.check(sec, f"step 1: before the owner starts, G receives nothing, and judges unobservable, {want}",
                     not s.all() and not s.beats() and a1.expect() == {"verdict": "unobservable", "reason": want,
                                                                       "level": None}, _ans(a1))
        o = make_owner(ep, addr, "--level", "ok", "--reason", "serving", "--reference", ref,
                       "--run-record", os.path.join(workdir, f"{run}.run"))
        t1 = time.monotonic_ns()
        _sleep_until(t1 + int(wait_s * NS))
        a2 = h2.judge(hl.read_across(s, addr, crosses))
        st = s.of("state/status")
        beats = s.beats(t1)
        tokens = live.list_presence(ground, "zk2/vehicle-01/*/@zk/**")
        if crosses:
            report.check(sec, "step 2: G receives the status and the heartbeats that vouch for it, under a 16-bit "
                              "run 12 bytes and 16 bytes each; G judges healthy, ok",
                         len(st) == 1 and len(st[0]["wire"]) == 12 and bool(beats)
                         and all(_vouches(a) for _, a in beats) and tokens.count == 0 and a2.expect() == HEALTHY,
                         f"{[_wire(s, x) for x in st]}; {len(beats)} heartbeats; {tokens.count} tokens; {_ans(a2)}")
        else:
            report.check(sec, "step 2: nothing crosses; G judges unobservable, face_closed, and the clock question "
                              "unobservable, nothing_heard",
                         not s.all() and not beats
                         and a2.expect() == {"verdict": "unobservable", "reason": "face_closed", "level": None}
                         and s.clock("across_face", None) == ("unobservable", "nothing_heard"),
                         f"{len(s.all())} deliveries, {len(beats)} heartbeats read; {_ans(a2)}; "
                         f"{s.clock('across_face', None)}")
        # Step 3: the owner's clock 2 s ahead, its guard holding.
        vehicle = live.open_client(ep)

        def reference() -> None:
            while not stop.wait(0.3):
                vehicle.put(ref, b"beat")

        threading.Thread(target=reference, daemon=True).start()
        o.cmd("clock 2000")
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline and not o.state()["ahead"]:
            time.sleep(0.05)
        t3 = time.monotonic_ns()
        _sleep_until(t3 + 35 * NS)
        faults = [(x, s.record(x)) for x in s.of("stream/faults", t3 - 2 * NS)]
        b3 = s.beats(t3 + 300_000_000)
        clock = s.clock("across_face", None)
        if run == "A":
            report.check(sec, "step 3: no fault crosses; G reads a heartbeat whose time is ahead, and answers yes, "
                              "ahead",
                         not faults and any(a.get("time") == heartbeat.AHEAD for _, a in b3)
                         and clock == ("yes", "ahead"),
                         f"{len(faults)} faults ({[len(x['wire']) for x, _ in faults]} B); {len(b3)} heartbeats; "
                         f"{clock}")
        elif run == "B":
            report.check(sec, "step 3: nothing crosses; the clock question unobservable, nothing_heard",
                         not faults and not b3 and clock == ("unobservable", "nothing_heard"),
                         f"{len(faults)} faults, {len(b3)} heartbeats read; {clock}")
        else:
            report.check(sec, "step 3: G receives one clock_ahead fault, 14 bytes under a 16-bit run, and shows it as "
                              "an occurrence; it answers the clock question unobservable, nothing_heard: no heartbeat "
                              "crosses; G judges health unobservable, face_closed",
                         len(faults) == 1 and len(faults[0][0]["wire"]) == 14 and faults[0][1].word == 1
                         and not b3 and clock == ("unobservable", "nothing_heard")
                         and h2.judge(hl.read_across(s, addr, False)).reason == "face_closed",
                         f"{[(r.expect(), len(x['wire'])) for x, r in faults]}; {len(b3)} heartbeats read; {clock}")
        if run == "A":
            o.cmd("clock 0")
            o.cmd("unvouch")
            t4 = time.monotonic_ns()
            _sleep_until(t4 + int(wait_s * NS) + 2 * NS)
            a4 = h2.judge(hl.read_across(s, addr, True))
            report.check(sec, "step 4: G judges stale: never down, never failed, never absent; its heartbeats carry "
                              "instants again, and it answers the clock question no",
                         a4.verdict == "stale" and a4.level is None
                         and s.clock("across_face", None) == ("no", "confirmed"),
                         f"{_ans(a4)}; {s.clock('across_face', None)}")
        vehicle.close()
    finally:
        stop.set()
        if o is not None:
            o.close()
        if s is not None:
            s.close()
        ground.close()
        v.close()


def section8(report: Report, wait_s: float = 65.0, face: str = "listed") -> None:
    workdir = tempfile.mkdtemp(prefix="zk2py-health-v2-face-", dir=os.environ.get("ZK2PY_SCRATCH"))
    threads = []
    for run in ("A", "B", "C"):
        os.makedirs(os.path.join(workdir, run), exist_ok=True)
        t = threading.Thread(target=_face_run, args=(report, workdir, run, wait_s, face))
        t.start()
        threads.append(t)
    for t in threads:
        t.join()


# -- §9 ------------------------------------------------------------------------------------

def section9(report: Report) -> None:
    from .contract import load_contract

    sec = "§9 words, by table"
    bus = Bus()
    try:
        tc_now, tc_earlier = load_contract(INTEROP / "tc.v1.toml"), load_contract(INTEROP / "earlier" / "tc.v1.toml")
        std = hl.standard()
        s_tc, s_plain = bus.listener("lab/tc"), bus.listener("lab/plain")
        tc = bus.owner("lab/tc", "--level", "ok", "--reason", "serving", "--contract", str(INTEROP / "tc.v1.toml"),
                       "--run-record", os.path.join(bus.tmp, "tc.run"))
        plain = bus.owner("lab/plain", "--level", "ok", "--reason", "serving")
        time.sleep(0.5)
        t1 = time.monotonic_ns()
        tc.cmd("status degraded tc_netns_gone")
        tc.cmd("fault tc_netns_restarted degraded 3")
        time.sleep(0.5)
        st = [x for x in s_tc.of("state/status", t1) if x["kind"] == "put"]
        fl = s_tc.of("stream/faults", t1)
        r_st, r_fl = (s_tc.record(st[0]) if st else None), (s_tc.record(fl[0]) if fl else None)
        text = any(b"tc_" in x["wire"] for x in st + fl)
        report.check(sec, "step 1: the status's record carries the reason 32, and the fault's the code 32 and the "
                          "argument 3; neither payload holds a byte of text",
                     r_st is not None and r_fl is not None and r_st.word == 32 and r_fl.word == 32
                     and r_fl.expect().get("argument") == 3 and not text,
                     f"{[_wire(s_tc, x) for x in st + fl]}; {r_st and r_st.expect()}; {r_fl and r_fl.expect()}")
        # T and T2 read step 1's status before step 2 replaces it.
        held_t = {std.fingerprint: std.canonical, tc_now.fingerprint: tc_now.canonical}
        held_t2 = {std.fingerprint: std.canonical, tc_earlier.fingerprint: tc_earlier.canonical}
        ra, ia = hl.read_near(bus.t, "lab/tc", held=held_t)
        rb, ib = hl.read_near(bus.t, "lab/tc", held=held_t2)
        shown_a, shown_b = hl.shown(ra.status, ia["tables"]), hl.shown(rb.status, ib["tables"])
        code_a = h2.read_word(h2.CODE, r_fl.word, ia["tables"][h2.CODE]).text() if r_fl else None
        code_b = h2.read_word(h2.CODE, r_fl.word, ib["tables"][h2.CODE]).text() if r_fl else None
        tc.cmd("status degraded kernel too old for this qdisc")
        time.sleep(0.3)
        st2 = [x for x in s_tc.of("state/status", t1) if x["kind"] == "put"]
        r2 = s_tc.record(st2[-1]) if len(st2) == 2 else None
        log = tc.state()["log"]
        report.check(sec, "step 2: the record carries out_of_range, the code 63; the words are in lab/tc's log",
                     r2 is not None and r2.word == h2.OUT_OF_RANGE and st2[-1]["wire"][-6] & 0x3F == 63
                     and log == ["reason: kernel too old for this qdisc"],
                     f"{None if r2 is None else r2.expect()}; {[_wire(s_tc, x) for x in st2[-1:]]}; log {log}")
        plain.cmd("status degraded tc_netns_gone")
        time.sleep(0.3)
        sp = [x for x in s_plain.of("state/status", t1) if x["kind"] == "put"]
        rp = s_plain.record(sp[-1]) if sp else None
        report.check(sec, "step 3: lab/plain has no table, so its record carries out_of_range too",
                     rp is not None and rp.word == h2.OUT_OF_RANGE, f"{None if rp is None else rp.expect()}")
        rc, ic = hl.read_near(bus.t, "lab/tc", held=held_t)
        rpa, ipa = hl.read_near(bus.t, "lab/plain", held=held_t)
        rpb, ipb = hl.read_near(bus.t, "lab/plain", held=held_t2)
        report.check(sec, "step 4: T shows lab/tc's reason as tc_netns_gone and its fault's code as "
                          "tc_netns_restarted, with the argument 3; it shows step 2's status as a word no table "
                          "names, never as a name and never as no word",
                     shown_a == "degraded, tc_netns_gone" and code_a == "tc_netns_restarted"
                     and hl.shown(rc.status, ic["tables"]) == "degraded, a word no table names",
                     f"{shown_a!r}; code {code_a!r}; step 2's {hl.shown(rc.status, ic['tables'])!r}")
        report.check(sec, "step 4: T2 does not hold the revision lab/tc's descriptor names: it shows the reason as "
                          "the number 32, never as tc_qdisc_missing, the name its own revision has at that index",
                     shown_b == "degraded, 32" and code_b == "32", f"{shown_b!r}; code {code_b!r}")
        report.check(sec, "step 4: both show lab/plain's reason as a word no table names; each judges both services "
                          "unhealthy, degraded",
                     hl.shown(rpa.status, ipa["tables"]) == hl.shown(rpb.status, ipb["tables"])
                     == "degraded, a word no table names"
                     and all(h2.judge(r).expect() == {"verdict": "unhealthy", "reason": "degraded",
                                                      "level": "degraded"} for r in (ra, rb, rc, rpa, rpb)),
                     f"{hl.shown(rpa.status, ipa['tables'])!r}; {[_ans(h2.judge(r)) for r in (ra, rb, rc, rpa, rpb)]}")
    finally:
        bus.close()


def make_catalog(path: str, epoch: int, files: list[Path]) -> Any:
    """A catalog made by the reference's ``zk2 catalog new --epoch``, as a
    deployment provisions one, and read by zk2py."""
    from . import catalog as cat
    from .blackbox import BLACKBOX

    made = subprocess.run([str(BLACKBOX / "zk2"), "catalog", "new", "--epoch", str(epoch), *map(str, files)],
                          capture_output=True, text=True, timeout=30)
    if made.returncode != 0:
        raise RuntimeError(f"zk2 catalog new: exit {made.returncode}; {made.stderr.strip()[-300:]}")
    Path(path).write_text(made.stdout, encoding="utf-8")
    return cat.read(made.stdout.encode())


def under_a_catalog(report: Report, owner_heartbeat: bool = False) -> None:
    """Beside the scenarios (review-bot on round 120; round 123 with the
    reference): an owner given a catalog writes every time from the
    catalog's epoch: its status's ``since``, its checks' ``at``, those it
    holds before its tokens too, a fault's ``at``, its snapshots' ``time``,
    an entry's ``conf``, and its heartbeat's time. A reader that holds the
    catalog reads each so, and one that holds 0 reads another instant.
    ``owner_heartbeat``: the owner confirms by owner heartbeat
    (``freshness.v2`` §2.4), and the reader follows it, its clock question
    answered from it, the ahead time included."""
    from .contract import load_contract

    sec = (f"under a catalog's epoch{', by owner heartbeat' if owner_heartbeat else ''} "
           f"({'the reference' if OWNER == 'rust' else 'zk2py'}'s owner)")
    epoch = 1767225600
    bus = Bus()
    stop = threading.Event()
    ref = "zk2py/health-v2/reference"
    try:
        std, tc = hl.standard(), load_contract(INTEROP / "tcd.v1.toml")
        path = os.path.join(bus.tmp, "catalog.json")
        catalog = make_catalog(path, epoch, [h2.CONTRACT])
        s = bus.listener("lab/cat", epoch=epoch) if not owner_heartbeat else \
            hl.Listener(bus.s, "lab/cat", None, epoch=epoch, catalogs=[catalog], named=catalog.id)
        s0 = None
        if owner_heartbeat:
            bus.extra.append(s)
            # F-598's first case: a reader with no catalog reads every group
            # unknown_catalog, "and the put is still a read one"
            s0 = hl.Listener(bus.s, "lab/cat", None, epoch=epoch, catalogs=[], named=catalog.id)
            bus.extra.append(s0)
        flags = ["--level", "degraded", "--reason", "serving", "--catalog", path,
                 "--contract", str(INTEROP / "tcd.v1.toml"), "--check", "disk=degraded:tcd_slow",
                 "--run-record", os.path.join(bus.tmp, "cat.run")]
        if owner_heartbeat:
            flags += ["--owner-heartbeat", "--reference", ref]
        o = bus.owner("lab/cat", *flags)
        time.sleep(0.5)
        now_ms = time.time_ns() // 1_000_000
        o.cmd("check net degraded tcd_slow")
        o.cmd("fault tcd_gone degraded 7")       # a code no table names: out_of_range, in either owner's log
        time.sleep(0.5)
        held = {std.fingerprint: std.canonical, tc.fingerprint: tc.canonical}
        r, info = hl.read_near(bus.t, "lab/cat", listener=s, held=held)
        r0, info0 = hl.read_near(bus.g, "lab/cat", held=held, epoch=0)
        # a reader that holds no catalog: core §8.6 "Never a guess" (F-610)
        rn, infon = hl.read_near(bus.g, "lab/cat", held=held, catalogs=[])
        rc, infoc = hl.read_near(bus.g, "lab/cat", held=held, catalogs=[catalog])
        j = info["joined"]
        doc = info.get("descriptor", {})
        first = [s.record(x) for x in s.of("state/checks/disk") if x["kind"] == "put"]
        later = [s.record(x) for x in s.of("state/checks/net") if x["kind"] == "put"]
        faults = [s.record(x) for x in s.of("stream/faults")]
        near = lambda ms: ms is not None and abs(ms - now_ms) <= 5000  # noqa: E731
        report.check(sec, "the owner's descriptor states the catalog's id, and its status's since, read under the "
                          "catalog's epoch, is this instant; read under 0 it is an instant in 1970",
                     doc.get("catalog") == catalog.id and r.status is not None and near(r.status.at_ms)
                     and r0.status is not None and r0.status.at_ms is not None
                     and r.status.at_ms - r0.status.at_ms == epoch * 1000,
                     f"catalog {doc.get('catalog')} (zk2's {catalog.id}); since {r.status and r.status.at_ms} under "
                     f"{epoch}, {r0.status and r0.status.at_ms} under 0; now {now_ms}")
        jn = infon["joined"]
        an = h2.judge(rn)
        report.check(sec, "a reader that holds no catalog refuses every timed record, unknown_catalog, and dates no "
                          "confirmation: unobservable, never a level read from the bits (core §8.6, v2 §2.6); one "
                          "that holds it reads the epoch from it",
                     infon["catalog"] == "unknown_catalog" and jn.status is not None and jn.status.refusal ==
                     "unknown_catalog" and all(c.refusal == "unknown_catalog" for c in jn.checks.values())
                     and an.verdict == "unobservable" and infoc["catalog"] is None
                     and rc.status is not None and near(rc.status.at_ms),
                     f"no catalog: {infon['catalog']}, status {jn.status and jn.status.expect()}, checks "
                     f"{[(k, c.expect()) for k, c in jn.checks.items()]}, {_ans(an)}; with it: "
                     f"{rc.status and rc.status.at_ms}")
        report.check(sec, "the check held before the tokens and the one put later count their at from the one "
                          "epoch, and each carries the detail 32, its instance's own word, never out_of_range",
                     len(first) == 1 and len(later) == 1 and near(first[0].at_ms) and near(later[0].at_ms)
                     and first[0].word == 32 and later[0].word == 32
                     and {k: (c.word, near(c.at_ms)) for k, c in j.checks.items()}
                     == {"disk": (32, True), "net": (32, True)}
                     and hl.shown(j.checks["disk"], info["tables"]) == "degraded, tcd_slow",
                     f"before the tokens {[c.expect() for c in first]}; later {[c.expect() for c in later]}; "
                     f"the GET's {[(k, c.expect()) for k, c in j.checks.items()]}; log {o.state()['log']}")
        report.check(sec, "a fault's at counts from the catalog's epoch",
                     len(faults) == 1 and near(faults[0].at_ms), f"{[f.expect() for f in faults]}")
        snaps = info["snapshots"]
        times = [None if x is None or x.get("time") is None else x["time"]["s"] * 1000 + x["time"]["ms"]
                 for x in snaps]
        report.check(sec, "each snapshot's time counts from the catalog's epoch: read under it, this instant",
                     all(t is not None and near(t + epoch * 1000) for t in times),
                     f"the two snapshots' times {times} ms after {epoch}; now {now_ms}")
        # an entry's conf: the status no longer vouched for, its last confirmation
        o.cmd("unvouch")
        time.sleep(0.5)
        r2, info2 = hl.read_near(bus.t, "lab/cat", listener=s, held=held)
        snap2 = info2["snapshots"][0] or {}
        entry = next(iter(snap2.get("live") or []), {})
        conf = entry.get("conf")
        conf_ms = None if not conf else conf["s"] * 1000 + conf["ms"]
        report.check(sec, "an entry's conf, its last confirmation once not vouched for, counts from the catalog's "
                          "epoch",
                     entry.get("confirmation") == "conf" and conf_ms is not None and near(conf_ms + epoch * 1000),
                     f"{entry.get('confirmation')} {conf}; now {now_ms}")
        o.cmd("vouch")
        if not owner_heartbeat:
            beats = s.beats()
            hb = [a.get("time") for _, a in beats if isinstance(a.get("time"), dict)]
            report.check(sec, "its heartbeat's time counts from the catalog's epoch: read under it, an instant of "
                              "this minute",
                         bool(hb) and all(abs(t["s"] * 1000 + t["ms"] + epoch * 1000 - now_ms) <= 60_000 for t in hb),
                         f"{len(beats)} heartbeats, times {hb[:3]}")
            return
        # owner heartbeats: wait for one, then hold the guard
        deadline = time.monotonic() + 35
        while time.monotonic() < deadline and not s.sub.owner_beats.get("lab/cat"):
            time.sleep(0.2)
        heard = s.sub.owner_beats.get("lab/cat", [])
        own_hb = [a for _, a in s.beats() if "owner_heartbeat" not in a]
        host = [(t, a, tm) for t, cls, a, tm in heard if cls == "host"]
        groups = [[g.get("iface") for g in a.get("read", {}).get("groups", [])] for _, a, _ in host]
        rows = [a for _, a in s.beats() if a.get("owner_heartbeat") == "host"]
        report.check(sec, "the owner heartbeat of the host class holds a group for health.v2, by its index in the "
                          "catalog; its time, read under the catalog's epoch, is an instant of this minute; it "
                          "vouches for the status, and the reader answers the clock question no, confirmed, from it",
                     bool(host) and all(g == ["health.v2"] for g in groups)
                     and all(isinstance(tm, dict) and abs(tm["s"] * 1000 + tm["ms"] + epoch * 1000 - now_ms) <= 60_000
                             for _, _, tm in host)
                     and any(_vouches(a) for a in rows) and s.clock() == ("no", "confirmed"),
                     f"{len(host)} host owner heartbeats, groups {groups}, times {[tm for _, _, tm in host]}; "
                     f"{len([x for x in heard if x[1] == 'link'])} link; clock {s.clock()}; "
                     f"heartbeats heard on health.v2's own key {len(own_hb)}")

        def reference() -> None:
            while not stop.wait(0.3):
                bus.g.put(ref, b"beat")

        threading.Thread(target=reference, daemon=True).start()
        time.sleep(1.0)
        o.cmd("clock 2000")
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline and not o.state()["ahead"]:
            time.sleep(0.05)
        t_det = time.monotonic_ns()
        time.sleep(32.0)
        held_host = [(t, a, tm) for t, cls, a, tm in s.sub.owner_beats.get("lab/cat", [])
                     if cls == "host" and t > t_det + 300_000_000]
        ahead_faults = [f for f in (s.record(x) for x in s.of("stream/faults")) if f.word == 1]
        report.check(sec, "its guard holding: every host owner heartbeat carries the ahead time, the reader "
                          "answers yes, ahead, from it, and one clock_ahead fault is heard, its at from the "
                          "catalog's epoch",
                     bool(held_host) and all(tm == heartbeat.AHEAD for _, _, tm in held_host)
                     and s.clock() == ("yes", "ahead") and len(ahead_faults) == 1
                     and ahead_faults[0].at_ms is not None
                     and abs(ahead_faults[0].at_ms - time.time_ns() // 1_000_000) <= 45_000,
                     f"{len(held_host)} owner heartbeats, times {[tm for _, _, tm in held_host]}; clock {s.clock()}; "
                     f"{[f.expect() for f in ahead_faults]}")
        assert s0 is not None
        groups0 = [[g.get("refused") or g.get("iface") for g in a.get("read", {}).get("groups", [])]
                   for t, cls, a, _ in s0.sub.owner_beats.get("lab/cat", [])
                   if cls == "host" and t > t_det + 300_000_000]
        report.check(sec, "F-598's first case: a reader that holds no catalog reads the owner heartbeat's group "
                          "unknown_catalog, counts the put whatever became of its groups, and answers yes, ahead",
                     bool(groups0) and all(g == ["unknown_catalog"] for g in groups0)
                     and s0.clock() == ("yes", "ahead"),
                     f"groups {groups0}; clock {s0.clock()}")
    finally:
        stop.set()
        bus.close()


def under_a_catalog_oh(report: Report) -> None:
    under_a_catalog(report, owner_heartbeat=True)


def section8_worded(report: Report) -> None:
    section8(report, face="worded")


#: ``8w`` is §8 on the face as the text words it, which its Expected do not
#: hold on (F-596): opt-in, never part of a whole run.
SECTIONS = {"1": section1, "2": section2, "3": section3, "4": section4, "5": section5, "6": section6,
            "7": section7, "8": section8, "9": section9, "8w": section8_worded, "epoch": under_a_catalog,
            "epoch-oh": under_a_catalog_oh}


def main(argv: list[str] | None = None) -> int:
    p = argparse.ArgumentParser(prog="zk2py.health_v2_scenarios")
    p.add_argument("--only", action="append", choices=sorted(SECTIONS))
    p.add_argument("--owner", choices=("zk2py", "rust"), default="zk2py")
    p.add_argument("--archive", choices=("stand-in", "rust"))
    args = p.parse_args(argv)
    global OWNER, ARCHIVE
    OWNER = args.owner
    ARCHIVE = args.archive or ("rust" if OWNER == "rust" else "stand-in")
    print(f"NOTE the owner {OWNER}'s, the archive {ARCHIVE}, the readers zk2py's", flush=True)
    report = Report()
    for name in args.only or [n for n in sorted(SECTIONS) if n != "8w"]:
        try:
            SECTIONS[name](report)
        except Exception as e:  # noqa: BLE001 - a section that raises is a failed check, the rest go on
            import traceback

            report.check(f"§{name}", "the section ran", False, f"{type(e).__name__}: {e}\n{traceback.format_exc()}")
    bad = [r for r in report.rows if not r[2]]
    by: dict[str, list[bool]] = {}
    for sec, _, ok, _ in report.rows:
        by.setdefault(sec, []).append(ok)
    for sec, oks in by.items():
        print(f"{sec}: {sum(oks)} of {len(oks)}")
    print(f"{len(report.rows) - len(bad)} of {len(report.rows)} checks passed")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())

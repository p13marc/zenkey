"""desired.v2 ``scenarios-v2.md`` §1 to §6, §8 and §13 to §15 (text 0.1), live,
between two zk2py processes (#609, round 118)::

    python -m zk2py.desired_v2_scenarios [--only 1|2|3|4|5|6|8|13|14|14-no-move|15]...

**The reference's runtime does not speak desired.v2** at a6aab761, so each
party is zk2py's own (``python -m zk2py.desired_v2_owner``): this runner
shows whether a section's steps and Expected can be run as written, and
that zk2py's author and target agree with each other. It is no interop.

The bus is zk2py's: R1, and where a section cuts a link R2 behind a TCP
relay the runner cuts and heals; S on R1 (every delivery under
``zk2/lab/**``, and the instance tokens), S2 on R2 (``Rp`` and ``Run``), G
(S4's GETs), X (§2's puts in envelopes it writes), and in §13 P, a session
that plays the author.

**A face** here is the relay and the target's word (``--desired-across``):
it reads no presence and waits the reply timeout it is given. No
``link.v2`` policy, and so no deny of ``@zk``, stands between R1 and R2.
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

from . import desired_v2 as d2, zk2c
from .alarms_scenarios import Report, _wait

REPO = Path(__file__).resolve().parents[3]
SCRATCH = Path("/srv/dev/scratch/zenkey/py")
AUTHOR, TARGET, SPARE = "lab/author", "lab/worker", "lab/spare"
D = "zk2/lab/author/tasking.v2/state/tasks/lab/worker"
D2 = "zk2/lab/author/tasking.v2/state/tasks/lab/spare"
RP = "zk2/lab/worker/worker.v2/state/desired/task"
RUN = "zk2/lab/worker/worker.v2/state/running"
HB = "zk2/lab/author/freshness.v2/stream/heartbeat/tasking.v2"
BASELINE = {"revision": 0, "mode": "idle", "rate_hz": 1}
BASELINE_REPORT = d2.baseline_report()


def v(n: int, **kw: Any) -> dict[str, Any]:
    return {"revision": n, "mode": "run", "rate_hz": 10, **kw}


def observed(version: str | None, revision: int | None = None, fields: tuple[str, ...] = d2.OBSERVED_FIELDS) -> dict:
    ver = d2.Version.parse(version)
    return {} if ver is None else d2.version_fields(ver, revision, fields)


def converged(version: str | None, revision: int) -> dict[str, Any]:
    """scenarios-v2: "Converged on vn means the report has source desired,
    outcome converged, in_force observed, the observed version s<n> and
    revision n"."""
    return {"source": "desired", "outcome": "converged", "last_known": False, "in_force": "observed",
            **observed(version, revision)}


class Party:
    """A zk2py desired.v2 owner process."""

    def __init__(self, address: str, endpoint: str, flags: list[str]) -> None:
        argv = [sys.executable, "-m", "zk2py.desired_v2_owner", "--connect", endpoint, *flags, address]
        env = dict(os.environ, PYTHONPATH=str(REPO / "impl" / "python"))
        self.address, self.flags = address, flags
        self.proc = subprocess.Popen(argv, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                     text=True, env=env)
        self.lines: queue.Queue[str] = queue.Queue()
        self.stderr: list[str] = []
        threading.Thread(target=lambda: [self.lines.put(x.rstrip("\n")) for x in self.proc.stdout],
                         daemon=True).start()
        threading.Thread(target=lambda: [self.stderr.append(x) for x in self.proc.stderr], daemon=True).start()
        self.instance = self._ready(20.0)

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
        prefix = f"desired {word.split(' ')[0]} "
        if got is None or not got.startswith(prefix):
            return None
        return json.loads(got[len(prefix):])

    def report(self) -> Any:
        return self._json("report")

    def running(self) -> Any:
        return self._json("running")

    def watch(self) -> Any:
        return self._json("watch")

    def gets(self, target: str = TARGET) -> list[dict[str, Any]]:
        return self._json(f"gets {target}") or []

    def acts(self) -> int | None:
        got = self.cmd("desired acts")
        return int(got.split(" ")[2]) if got and got.startswith("desired acts ") else None

    def put(self, target: str, doc: Any) -> str | None:
        got = self.cmd(f"desired put {target} {json.dumps(doc)}")
        return got.rsplit(" ", 1)[1] if got and got.startswith("desired put ") else None

    def delete(self, target: str, force: bool = False) -> str | None:
        got = self.cmd(f"desired delete{'!' if force else ''} {target}")
        return got.rsplit(" ", 1)[1] if got and got.startswith("desired deleted ") else None

    def kill(self) -> None:
        """"Its process ends without closing"."""
        if self.proc.poll() is None:
            self.proc.send_signal(signal.SIGKILL)
        self.proc.wait(10)

    def pause(self) -> None:
        self.proc.send_signal(signal.SIGSTOP)

    def resume(self) -> None:
        self.proc.send_signal(signal.SIGCONT)


class Bus:
    """R1; R2 behind a relay when ``cut``; S, its token watch, S2, G and X."""

    def __init__(self, cut: bool = False) -> None:
        import zenoh

        from . import live
        from .budget_scenarios import Link
        from .live_interop import _r1
        from .owner import free_loopback_port

        self.zenoh, self.live = zenoh, live
        self.r1, self.ep1, self.r1_zid = _r1()
        self.link = self.r2 = None
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
        #: S: (monotonic s, key, kind, the raw payload, its in-band version's
        #: text or None, the form, its Zenoh timestamp's id or None)
        self.got: list[tuple[float, str, str, bytes, str | None, str | None, str | None]] = []
        self.tokens: list[tuple[float, str, str]] = []
        self.got2: list[tuple[float, str]] = []
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
        self.dir = Path(tempfile.mkdtemp(prefix="r118-desired-v2-", dir=SCRATCH))
        time.sleep(0.3)

    def _s(self, smp: Any) -> None:
        ts = smp.timestamp
        read = zk2c.read_sample(smp)
        st = read.stamp
        with self.lock:
            self.got.append((time.monotonic(), str(smp.key_expr),
                             "delete" if smp.kind == self.zenoh.SampleKind.DELETE else "put",
                             smp.payload.to_bytes(),
                             None if st is None else d2.Version.of(st.width, st.run, st.seq).text, read.form,
                             None if ts is None else str(ts).split("/", 1)[1]))

    def _s2(self, smp: Any) -> None:
        with self.lock:
            self.got2.append((time.monotonic(), str(smp.key_expr)))

    def _token(self, smp: Any) -> None:
        with self.lock:
            self.tokens.append((time.monotonic(), "put" if smp.kind == self.zenoh.SampleKind.PUT else "delete",
                                str(smp.key_expr)))

    def since(self, t: float, key: str | None = None) -> list[Any]:
        with self.lock:
            return [g for g in self.got if g[0] >= t and (key is None or g[1] == key)]

    def since2(self, t: float) -> list[Any]:
        with self.lock:
            return [g for g in self.got2 if g[0] >= t]

    def token_put(self, address: str, after: float) -> str | None:
        with self.lock:
            for t, kind, key in self.tokens:
                if t >= after and kind == "put" and key.startswith(f"zk2/{address}/@zk/instance/"):
                    return key
        return None

    def party(self, address: str, flags: list[str], on: int = 1) -> Party:
        p = Party(address, self.ep1 if on == 1 else self.ep2, flags)
        self.parties.append(p)
        return p

    def author(self, *flags: str, name: str = "author", run_record: bool = True) -> Party:
        """The author, with its record and, "so it writes under 16-bit runs",
        its run's record. ``run_record`` False is an author that keeps none:
        "An owner that cannot keep such a record draws 32-bit runs" (core
        §4.3)."""
        runs = ["--desired-run-record", str(self.dir / f"{name}.run.json")] if run_record else []
        return self.party(AUTHOR, ["--desired-author", "--desired-record", str(self.dir / f"{name}.record.json"),
                                   *runs, *flags])

    def get(self, key: str, on: int = 1, timeout: float = 1.0) -> Any:
        return self.live.get_state(self.g1 if on == 1 else self.g2, key, timeout)

    def entry(self, key: str, on: int = 1) -> tuple[str, str | None, bytes]:
        """What a GET of one concrete key drew: (``live``, version, record),
        (``tombstone``, version, b""), (``no_entry``, the header's
        run/asof, b""), (``reply``, …) for a per-key reply, or (``silent``,
        None, b"")."""
        st = self.get(key, on)
        for r in st.replies:
            if r.key == key and r.refusal is None and r.inband is not None:
                ver = d2.Version.of(r.inband.width, r.inband.run, r.inband.seq).text
                if r.snapshot_key is None:
                    return "reply", ver, r.payload
                return ("tombstone" if r.deleted else "live"), ver, r.payload
        for h in st.snapshots:
            if h.key == key and h.done:
                return "no_entry", d2.Version.of(h.header["run_width"], h.header["run"], h.header["asof"]).text, b""
        return "silent", None, b""

    def report_at(self, on: int = 1) -> Any:
        """"The report" is Rp's record, read as §2.10 says: (fields, the
        put's CH byte, the entry's version)."""
        st = self.get(RP, on)
        for r in st.replies:
            if r.key == RP and r.refusal is None and r.raw:
                rd = d2.read_put(r.raw)
                return rd.report, r.raw[0], d2.Version.of(r.inband.width, r.inband.run, r.inband.seq).text
        return None, None, None

    def run_doc(self, on: int = 1) -> Any:
        kind, _, payload = self.entry(RUN, on)
        return json.loads(payload) if kind == "live" else None

    def author_seen(self, on: int) -> bool:
        p = self.live.list_presence(self.g1 if on == 1 else self.g2, f"zk2/{AUTHOR}/@zk/instance/*")
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
        for p in self.parties:
            try:
                p.resume()
                p.kill()
            except Exception:  # noqa: BLE001 - closing anyway
                pass
        for e in self.entities:
            try:
                e.undeclare()
            except Exception:  # noqa: BLE001
                pass
        for s in {id(x): x for x in (self.s, self.g1, self.g2, self.s2, self.x) if x is not None}.values():
            self.live.close(s)
        if self.link is not None:
            self.link.close()
        for r in (self.r2, self.r1):
            if r is not None:
                try:
                    r.close()
                except Exception:  # noqa: BLE001
                    pass


TARGET_FLAGS = ["--desired-target", AUTHOR, "--desired-presence-watch"]


def _converge(author: Party, target: Party, n: int = 1, **kw: Any) -> str | None:
    s = author.put(TARGET, v(n, **kw))
    _wait(lambda: target.report() == converged(s, n), 10.0, 0.1)
    return s


def section1(rp: Report) -> None:
    sec = "§1 converge after an outage"
    bus = Bus(cut=True)
    try:
        author = bus.party(AUTHOR, ["--desired-author", "--tokenless", "tasking.v2", "--desired-run-record",
                                    str(bus.dir / "run.json")], on=1)
        t0 = time.monotonic()
        target = bus.party(TARGET, TARGET_FLAGS, on=2)
        _wait(lambda: bus.token_put(TARGET, t0) is not None, 10.0, 0.01)
        rep, ch, _ = bus.report_at()
        st = bus.get(RP)
        raw = next((r.raw for r in st.replies if r.key == RP), b"")
        rp.check(sec, "1. G's reply is a snapshot of Rp whose entry is the baseline's report, under tag 12: the "
                      "record 000000", rep == BASELINE_REPORT and ch is not None and ch & 0x0F == 12
                 and raw[-3:] == bytes(3) and any(a[1] == "page" for a in st.arrivals) and bus.run_doc() == BASELINE,
                 f"{rep}, CH {ch}, {raw.hex()}")
        t1 = time.monotonic()
        s1 = author.put(TARGET, v(1))
        ok = _wait(lambda: any(g[2] == "put" for g in bus.since(t1, D)), 5.0)
        dput = next(iter(bus.since(t1, D)), None)
        kind, ver, _ = bus.entry(D)
        rp.check(sec, "2. S receives a put on D in a mutation's envelope; where it carries a Zenoh timestamp, the id "
                      "is R1's; G's GET of D draws a snapshot of one live entry whose stamp is that put's",
                 ok and dput is not None and dput[5] == "mutation" and dput[4] == s1
                 and dput[6] in (None, bus.r1_zid) and (kind, ver) == ("live", s1),
                 f"S {dput and dput[4:]}, R1 {bus.r1_zid}, GET {kind} {ver}")
        ok = _wait(lambda: bus.report_at()[0] == converged(s1, 1), 10.0, 0.1)
        rp.check(sec, "2. then Rp holds a report converged on v1, and Run holds v1", ok and bus.run_doc() == v(1),
                 str(bus.report_at()[0]))
        acts0, rep0 = target.acts(), target.report()
        rp.check(sec, "3. the link cut", bus.cut())
        t3 = time.monotonic()
        s2 = author.put(TARGET, v(2))
        time.sleep(2.0)
        rp.check(sec, "3. through S2: Rp and Run receive nothing during the cut, and the target's report is "
                      "unchanged", not bus.since2(t3) and target.acts() == acts0 and target.report() == rep0)
        g0 = len(author.gets())
        t4 = time.monotonic()
        bus.heal()
        ok = _wait(lambda: target.report() == converged(s2, 2), 10.0, 0.05)
        took = time.monotonic() - t4
        seeds = author.gets()[g0:]
        rp.check(sec, "4. the target's seed after the heal carries the held stamp s1, and the author answers whole",
                 bool(seeds) and seeds[0] == {"held": s1, "alone": False}, f"{seeds}, s1 {s1}")
        rp.check(sec, "4. the target acts on v2 once, converged on v2 within 5 s of the heal",
                 ok and took <= 5.0 and target.acts() == (acts0 or 0) + 1 and bus.run_doc(on=2) == v(2),
                 f"{took:.2f} s, acts {acts0} -> {target.acts()}")
        rp.note(sec, f"converged {took:.2f} s after the heal")
    finally:
        bus.close()


def section2(rp: Report) -> None:
    sec = "§2 never an older version"
    bus = Bus()
    try:
        author, target = bus.author(), bus.party(TARGET, TARGET_FLAGS)
        _converge(author, target, 1)
        t = time.monotonic()
        s2 = author.put(TARGET, v(2))
        ok = _wait(lambda: target.report() == converged(s2, 2), 10.0, 0.05)
        put2 = next((g for g in bus.since(t, D) if g[4] == s2), None)
        ver2 = d2.Version.parse(s2)
        rp.check(sec, "setup: the target converged on v2, s2 of the author's 16-bit run R at seq n",
                 ok and put2 is not None and ver2 is not None and ver2.run.width == 16, f"s2 {s2}")
        assert ver2 is not None and put2 is not None
        tag = put2[3][0] & 0x0F
        acts0 = target.acts()

        def x_put(doc: Any, ver: d2.Version) -> None:
            bus.x.put(D, zk2c.mutation(zk2c.InBand(ver.run.width, ver.run.value, ver.seq),
                                       json.dumps(doc).encode(), tag), encoding=zk2c.ENCODING)

        t1 = time.monotonic()
        x_put(v(1), d2.Version(ver2.run, ver2.seq - 1))
        time.sleep(1.0)
        rp.check(sec, "1. X's v1 under R at seq n − 1: older; nothing acted, Rp and Run receive nothing",
                 target.acts() == acts0 and not bus.since(t1, RP) and not bus.since(t1, RUN))
        t2 = time.monotonic()
        x_put(v(3), ver2)
        time.sleep(1.0)
        rp.check(sec, "2. X's v3 under R at seq n, s2 exactly: the same version, nothing",
                 target.acts() == acts0 and not bus.since(t2, RP) and not bus.since(t2, RUN))
        g0 = len(author.gets())
        t3 = time.monotonic()
        rr = target.cmd("desired reread 2000")
        time.sleep(4.5)
        target.cmd("desired reread off")
        rereads = author.gets()[g0:]
        rp.check(sec, "3. the re-read on a 2 s period carries the held stamp s2, and the author answers with that "
                      "stamp alone; nothing acted, nothing put on Rp",
                 rr == "desired reread 2000" and len(rereads) >= 1
                 and all(g == {"held": s2, "alone": True} for g in rereads)
                 and target.acts() == acts0 and not bus.since(t3, RP), f"{rereads}")
        mine = d2.Version.of(32, 0x0BADCAFE, 0)
        x_put(v(3), mine)
        ok = _wait(lambda: target.report() == converged(mine.text, 3), 5.0, 0.05)
        rp.check(sec, "4. X's v3 under a 32-bit run of its own at seq 0: another run, admitted and acted on; the "
                      "report is converged on v3 under X's version", ok and target.acts() == (acts0 or 0) + 1,
                 str(target.report()))
        t5 = time.monotonic()
        x_put(v(1), d2.Version(ver2.run, ver2.seq - 1))
        time.sleep(1.0)
        rp.check(sec, "5. X's v1 under R at seq n − 1 again: R is a run the target remembers; nothing",
                 target.acts() == (acts0 or 0) + 1 and not bus.since(t5, RP) and not bus.since(t5, RUN))
    finally:
        bus.close()


def section3(rp: Report) -> None:
    sec = "§3 a delete reverts, within the window and past it"
    bus = Bus(cut=True)
    try:
        author = bus.author("--desired-window", "10")
        target = bus.party(TARGET, TARGET_FLAGS, on=2)
        s1 = _converge(author, target, 1)
        rp.check(sec, "setup: the target converged on v1", target.report() == converged(s1, 1))

        def reverted(ver: str | None) -> dict[str, Any]:
            return {"source": "baseline", "outcome": "reverted", "last_known": False, "in_force": "none",
                    **observed(ver)}

        sd = author.delete(TARGET)
        ok = _wait(lambda: target.report() == reverted(sd), 10.0, 0.05)
        rp.check(sec, "1. the author deletes D: the target reverts; Run holds the baseline; the report is baseline, "
                      "reverted, the observed version the delete's, with no revision",
                 ok and bus.run_doc(on=2) == BASELINE, str(target.report()))
        s2 = author.put(TARGET, v(2))
        rp.check(sec, "2. the author puts v2: converged on v2",
                 _wait(lambda: target.report() == converged(s2, 2), 10.0))
        bus.cut()
        sd3 = author.delete(TARGET)
        g0 = len(author.gets())
        time.sleep(3.0)
        kind, ver, _ = bus.entry(D)
        bus.heal()
        ok = _wait(lambda: target.report() == reverted(sd3), 10.0, 0.05)
        seeds = author.gets()[g0 + 0:]
        seeds = [g for g in seeds if g["held"] is not None]
        rp.check(sec, "3. a delete during a 3 s cut: the seed after the heal carries the held stamp s2; the author "
                      "answers a snapshot whose one entry is a tombstone with the delete's stamp; the target reverts",
                 ok and (kind, ver) == ("tombstone", sd3) and bool(seeds) and seeds[0] == {"held": s2, "alone": False}
                 and bus.run_doc(on=2) == BASELINE, f"GET {kind} {ver}, seeds {seeds}, {target.report()}")
        s3 = author.put(TARGET, v(3))
        ok = _wait(lambda: target.report() == converged(s3, 3), 10.0)
        acts0 = target.acts()
        bus.cut()
        author.delete(TARGET)
        g0 = len(author.gets())
        time.sleep(15.0)
        kind, head, _ = bus.entry(D)
        bus.heal()
        ok = ok and _wait(lambda: target.report() == reverted(None), 10.0, 0.05)
        seeds = [g for g in author.gets()[g0:] if g["held"] is not None]
        s3v, headv = d2.Version.parse(s3), d2.Version.parse(head)
        rp.check(sec, "4. a delete during a 15 s cut, past the 10 s window: the seed carries the held stamp s3; the "
                      "author answers a snapshot with no entry, under the run it put v3 under; the target reverts; "
                      "Run holds the baseline; the report is baseline, reverted, with no observed version",
                 ok and kind == "no_entry" and s3v is not None and headv is not None and headv.run == s3v.run
                 and headv.seq >= s3v.seq and bool(seeds) and seeds[0] == {"held": s3, "alone": False}
                 and target.acts() == (acts0 or 0) + 1 and bus.run_doc(on=2) == BASELINE,
                 f"GET {kind} {head}, s3 {s3}, seeds {seeds}, {target.report()}")
        st = bus.get(RP, on=2)
        raw = next((r.raw for r in st.replies if r.key == RP), b"")
        rp.note(sec, f"the report after step 4 is {len(raw) - 6 if raw else '?'} bytes of record: "
                     f"{raw[6:].hex() if raw else ''}")
    finally:
        bus.close()


def section4(rp: Report) -> None:
    sec = "§4 a refused document keeps the previous in force"
    bus = Bus()
    try:
        author, target = bus.author(), bus.party(TARGET, TARGET_FLAGS)
        s1 = _converge(author, target, 1)

        def refused(ver: str | None, rev: int | None, code: str, pointer: int, detail: int) -> dict[str, Any]:
            return {"source": "desired", "outcome": "refused", "last_known": False, "in_force": "other",
                    **observed(ver, rev), **observed(s1, 1, d2.IN_FORCE_FIELDS), "code": code, "pointer": pointer,
                    "detail": detail}

        s2 = author.put(TARGET, v(2, endpoint="tcp/10.0.0.9:7447"))
        ok = _wait(lambda: target.report() == refused(s2, 2, "reach", 3, 1), 5.0)
        rp.check(sec, "1. v2 with an endpoint: nothing applied, Run still v1; refused, observed s2 with revision 2, "
                      "in_force other naming s1 with revision 1, code reach, pointer 3, detail 1",
                 ok and bus.run_doc() == v(1), str(target.report()))
        s3 = author.put(TARGET, v(3, rate_hz=500))
        ok = _wait(lambda: target.report() == refused(s3, 3, "invalid", 2, 0), 5.0)
        rp.check(sec, "2. v3 with rate_hz 500: the same, observed s3, code invalid, pointer 2, detail 0",
                 ok and bus.run_doc() == v(1), str(target.report()))
        acts0 = target.acts()
        run0 = author.cmd("desired run")
        t3 = time.monotonic()
        author.cmd("desired new-epoch")
        run1 = author.cmd("desired run")
        _wait(lambda: any(g[2] == "put" for g in bus.since(t3, D)), 5.0)
        again = next((g[4] for g in bus.since(t3, D) if g[2] == "put"), None)
        ok = _wait(lambda: target.report() == refused(again, 3, "invalid", 2, 0), 5.0)
        rp.check(sec, "3. the author's instance re-minted in its own session takes a new run and puts v3's content "
                      "again under it: adopted, never refused again; the observed version moves, still refused with "
                      "the same code, place and line; in_force still names s1",
                 ok and run0 != run1 and again is not None and again != s3 and target.acts() == acts0,
                 f"{run0} -> {run1}, {again}, {target.report()}")
        s4 = author.put(TARGET, v(4))
        rp.check(sec, "4. v4: converged on v4", _wait(lambda: target.report() == converged(s4, 4), 5.0))
        target.cmd("desired fail-next")
        s5 = author.put(TARGET, v(5, rate_hz=20, mode="halt"))
        want = {"source": "desired", "outcome": "failed", "last_known": False, "in_force": "none",
                **observed(s5, 5), "code": "apply"}
        ok = _wait(lambda: target.report() == want, 5.0)
        run = bus.run_doc() or {}
        rp.check(sec, "5. the drive fails after the rate: Run holds rate_hz 20 and mode run; failed, observed s5, "
                      "code apply, in_force none", ok and run.get("rate_hz") == 20 and run.get("mode") == "run",
                 f"{target.report()}, Run {run}")
        s6 = author.put(TARGET, v(4294967296))
        want = {"source": "desired", "outcome": "refused", "last_known": False, "in_force": "none",
                **observed(s6), "code": "invalid", "pointer": 4, "detail": 2}
        ok = _wait(lambda: target.report() == want, 5.0)
        rp.check(sec, "6. v6 with revision 4294967296: refused whole, invalid, at /revision, pointer 4 and detail 2; "
                      "the observed version is s6, and the report carries no revision; Run is unchanged",
                 ok and bus.run_doc() == run, f"{target.report()}")
        rep, _, _ = bus.report_at()
        rp.note(sec, f"Expected 6 does not say what in_force is: after step 5's failed act nothing is wholly in "
                     f"force, and zk2py's report says in_force {rep and rep.get('in_force')!r}")
    finally:
        bus.close()


def section5(rp: Report) -> None:
    sec = "§5 a restart acts again"
    bus = Bus()
    try:
        author, target = bus.author(), bus.party(TARGET, TARGET_FLAGS)
        s1 = _converge(author, target, 1)
        rp.check(sec, "setup: the target converged on v1", target.report() == converged(s1, 1))
        g0 = len(author.gets())
        target.kill()
        t0 = time.monotonic()
        new = bus.party(TARGET, [*TARGET_FLAGS, "--desired-hold"])
        _wait(lambda: bus.token_put(TARGET, t0) is not None, 10.0, 0.01)
        rep, ch, ver = bus.report_at()
        d = bus.live.get_descriptor(bus.g1, new.instance or "")
        doc = json.loads(d[0].payload) if d and d[0].ok else {}
        verp = d2.Version.parse(ver)
        rp.check(sec, "1. G's reply holds the baseline's report under the new instance's run",
                 rep == BASELINE_REPORT and verp is not None and verp.run.text == doc.get("run"),
                 f"{rep}, entry {ver}, descriptor run {doc.get('run')}")
        _wait(lambda: len(author.gets()) > g0, 5.0)
        seeds = author.gets()[g0:]
        new.cmd("desired release")
        ok = _wait(lambda: new.report() == converged(s1, 1), 10.0, 0.05)
        rp.check(sec, "1. the new process seeds with no held stamp, acts on v1 again, and its report is converged on "
                      "v1 under s1", ok and bool(seeds) and seeds[0]["held"] is None and new.acts() == 1
                 and bus.run_doc() == v(1), f"seeds {seeds}, {new.report()}")
    finally:
        bus.close()


def section6(rp: Report) -> None:
    sec = "§6 the author's restart moves no target to act again"
    bus = Bus()
    try:
        author, target = bus.author(), bus.party(TARGET, TARGET_FLAGS)
        s1 = _converge(author, target, 1)
        author.put(SPARE, v(1))
        author.delete(SPARE)
        time.sleep(5.0)
        rp.check(sec, "setup: the target converged on v1; D2's delete is 5 s old, in the author's record",
                 target.report() == converged(s1, 1) and bus.entry(D2)[0] == "tombstone")
        acts0 = target.acts()
        author.kill()
        t1 = time.monotonic()
        author = bus.author()
        _wait(lambda: bus.token_put(AUTHOR, t1) is not None, 10.0, 0.01)
        kd, vd, _ = bus.entry(D)
        k2, v2s, _ = bus.entry(D2)
        time.sleep(3.0)
        puts = [g for g in bus.since(t1, D) if g[2] == "put"]
        dels = [g for g in bus.since(t1, D2) if g[2] == "delete"]
        s1v = d2.Version.parse(s1)
        new = d2.Version.parse(puts[0][4]) if puts else None
        deln = d2.Version.parse(dels[0][4]) if dels else None
        rp.check(sec, "1. S receives a put on D holding v1's content under a new run, and a delete of D2 under the "
                      "same run; G's GET of D draws a snapshot whose entry carries that put's stamp, and of D2 a "
                      "tombstone",
                 new is not None and s1v is not None and deln is not None and new.run != s1v.run
                 and new.run == deln.run and (kd, vd) == ("live", new.text) and (k2, v2s) == ("tombstone", deln.text)
                 and json.loads(zk2c.parse(puts[0][3], "put").record) == v(1),
                 f"s1 {s1}, put {new and new.text}, delete {deln and deln.text}, GET D {kd} {vd}, D2 {k2} {v2s}")
        rp.check(sec, "1. the target adopts the put: Run receives nothing, and the report's observed version moves "
                      "to it, still converged, in_force observed",
                 new is not None and target.report() == converged(new.text, 1) and target.acts() == acts0
                 and not bus.since(t1, RUN), str(target.report()))
        s2 = author.put(TARGET, v(2))
        rp.check(sec, "2. v2: converged on v2, which the target acted on",
                 _wait(lambda: target.report() == converged(s2, 2), 5.0) and target.acts() == (acts0 or 0) + 1)
        author.kill()
        for f in bus.dir.glob("author.*"):
            f.unlink()
        t3 = time.monotonic()
        # "The author's new run is a 32-bit one where its run's record went
        # with its documents'": the author is told it keeps none. Given the
        # path of a file that is gone, zk2py's owner reads a first start and
        # takes run 0000 again (F-578).
        author = bus.author(run_record=False)
        s2b = author.put(TARGET, v(2))
        v2b = d2.Version.parse(s2b)
        ok = _wait(lambda: target.report() == converged(s2b, 2), 5.0)
        k2, head, _ = bus.entry(D2)
        rp.check(sec, "3. its record and its run's record removed: the new run is a 32-bit one; S receives a put on D "
                      "holding v2's content under it: adopted; the report moves to that version, still converged, "
                      "revision 2; a GET of D2 draws a snapshot with no entry",
                 ok and v2b is not None and v2b.run.width == 32 and target.acts() == (acts0 or 0) + 1
                 and not bus.since(t3, RUN) and k2 == "no_entry", f"{s2b}, D2 {k2} {head}, {target.report()}")
        s3 = author.put(TARGET, v(3))
        rp.check(sec, "4. v3: converged on v3", _wait(lambda: target.report() == converged(s3, 3), 5.0))
        rp.cannot(sec, "step 1's \"the author's clock is 5 s behind\": zk2py's desired.v2 author reads no clock for "
                       "a version, and has no offset to set")
    finally:
        bus.close()


def section8(rp: Report) -> None:
    sec = "§8 the kill switch"
    bus = Bus()
    try:
        author = bus.author()
        author.put(TARGET, v(1))
        t0 = time.monotonic()
        target = bus.party(TARGET, ["--desired-unbound"])
        _wait(lambda: bus.token_put(TARGET, t0) is not None, 10.0, 0.01)
        time.sleep(3.0)
        rep, ch, _ = bus.report_at()
        rp.check(sec, "Rp holds the baseline's report, and the author receives no GET of D",
                 rep == BASELINE_REPORT and target.report() == BASELINE_REPORT and author.gets() == []
                 and bus.run_doc() == BASELINE, f"{rep}, gets {author.gets()}")
    finally:
        bus.close()


class Played:
    """scenarios-v2, "A played author": "a session of the test's own, P ...
    holds an instance token at lab/author and a queryable on D. It puts
    nothing, and answers each GET as the step says, under a 16-bit run R"."""

    R = 0x0041

    def __init__(self, bus: Bus) -> None:
        import zenoh

        self.zenoh = zenoh
        self.s = bus.live.open_client(bus.ep1)
        self.lock = threading.Lock()
        #: each GET: [arrived, payload, encoding, answered at or None]
        self.gets: list[list[Any]] = []
        #: how to answer the GETs to come, in order; the last one repeats
        self.script: list[tuple[Any, ...]] = [("held",)]
        self.kept: list[Any] = []
        self.tokens = [self.s.liveliness().declare_token(f"zk2/{AUTHOR}/@zk/instance/00000000000000a1")]
        self.q = self.s.declare_queryable(D, zenoh.handlers.Callback(self._query))

    def page(self, doc: Any | None, seq: int) -> bytes:
        from . import heartbeat, snapshot

        live = [] if doc is None else [{"id": "", "run_width": None, "run": None, "seq": seq,
                                         "confirmation": "none", "record": json.dumps(doc).encode().hex()}]
        return snapshot.write_page({"tag": 0, "run_width": 16, "run": self.R, "asof": seq,
                                    "time": heartbeat.time_of(time.time_ns(), 0), "coll": 0, "index": 0,
                                    "more": False, "live": live, "tombs": []})

    def _query(self, query: Any) -> None:
        payload = None if query.payload is None else query.payload.to_bytes()
        with self.lock:
            rec = [time.monotonic(), payload, None if query.encoding is None else str(query.encoding), None]
            self.gets.append(rec)
            step = self.script.pop(0) if len(self.script) > 1 else self.script[0]
        threading.Thread(target=self._answer, args=(query, payload, rec, step), daemon=True).start()

    def _answer(self, query: Any, payload: bytes | None, rec: list[Any], step: tuple[Any, ...]) -> None:
        what = step[0]
        if what == "none":
            self.kept.append(query)  # left unanswered: the GET ends at its caller's timeout
            return
        if what == "after":
            time.sleep(step[1])
            what, step = step[2][0], step[2]
        if what == "held" and payload:
            query.reply(D, payload, encoding=zk2c.ENCODING)
        elif what == "stamp":
            query.reply(D, zk2c.write("snapshot", 16, self.R, step[1], 0, b""), encoding=zk2c.ENCODING)
        else:  # "snapshot", or a held stamp asked of a GET that carried none
            doc, seq = (step[1], step[2]) if what == "snapshot" else (self.default_doc, self.default_seq)
            query.reply(D, self.page(doc, seq), encoding=zk2c.ENCODING)
        rec[3] = time.monotonic()
        query.drop()

    default_doc: Any = None
    default_seq = 0

    def count(self) -> int:
        with self.lock:
            return len(self.gets)

    def close(self) -> None:
        for q in self.kept:
            try:
                q.drop()
            except Exception:  # noqa: BLE001
                pass
        self.q.undeclare()
        for t in self.tokens:
            t.undeclare()
        self.s.close()


def section13(rp: Report) -> None:
    sec = "§13 a conditional re-read, and what a read waits"
    bus = Bus()
    p = Played(bus)
    try:
        held7 = zk2c.write("snapshot", 16, p.R, 7, 0, b"")
        held9 = zk2c.write("snapshot", 16, p.R, 9, 0, b"")
        s7, s9 = d2.Version.of(16, p.R, 7).text, d2.Version.of(16, p.R, 9).text
        p.default_doc, p.default_seq = v(1), 7
        p.script = [("snapshot", v(1), 7), ("held",)]
        target = bus.party(TARGET, [*TARGET_FLAGS, "--desired-reread-ms", "2000"])
        ok = _wait(lambda: target.report() == converged(s7, 1), 10.0, 0.05)
        rp.check(sec, "setup: the seed draws v1's snapshot from P, its entry at (R, 7); the target converged on v1",
                 ok, str(target.report()))
        n0, t1, acts0 = p.count(), time.monotonic(), target.acts()
        time.sleep(5.0)
        mine = p.gets[n0:p.count()]
        doc_len = len(json.dumps(v(1)).encode())
        rp.check(sec, "1. each re-read carries the 6 bytes 0xe0, R and seq 7, under zenoh/bytes; the target acts on "
                      "nothing, and Rp and Run receive nothing",
                 len(mine) >= 2 and all(g[1] == held7 and g[2] == zk2c.ENCODING for g in mine)
                 and held7[0] == 0xE0 and len(held7) == 6 and target.acts() == acts0 and not bus.since(t1, RP)
                 and not bus.since(t1, RUN), f"{len(mine)} re-reads, {[g[1] and g[1].hex() for g in mine]}")
        rp.note(sec, f"an unchanged reply is 6 bytes of payload where the v1 document P answered is {doc_len} "
                     f"bytes, and {len(json.dumps(v(1), separators=(',', ':')))} with no space (the text: 44)")
        n0 = p.count()
        p.script = [("snapshot", v(2), 9), ("held",)]
        ok = _wait(lambda: target.report() == converged(s9, 2), 6.0, 0.05)
        _wait(lambda: p.count() >= n0 + 2, 6.0)
        nxt = p.gets[n0 + 1][1] if p.count() >= n0 + 2 else None
        rp.check(sec, "2. P answers the next re-read with v2's snapshot at (R, 9): a higher seq of the held run, "
                      "acted on; converged on v2; the next re-read carries (R, 9)",
                 ok and nxt == held9 and target.acts() == (acts0 or 0) + 1, f"{target.report()}, next {nxt and nxt.hex()}")
        n0, t3, rep0 = p.count(), time.monotonic(), target.report()
        p.script = [("stamp", 8), ("held",)]
        _wait(lambda: p.count() >= n0 + 2, 8.0)
        g = p.gets[n0:n0 + 2]
        rp.check(sec, "3. P answers with the held stamp (R, 8): no answer; nothing acted, the report unchanged; the "
                      "next re-read carries (R, 9) again",
                 len(g) == 2 and g[1][1] == held9 and target.report() == rep0 and target.acts() == (acts0 or 0) + 1
                 and not bus.since(t3, RP), f"{[x[1] and x[1].hex() for x in g]}")
        n0, t4 = p.count(), time.monotonic()
        p.script = [("snapshot", v(2), 9), ("held",)]
        _wait(lambda: p.count() >= n0 + 1, 6.0)
        time.sleep(0.5)
        rp.check(sec, "4. an author that reads no payload answers v2's snapshot at (R, 9): the same version, nothing "
                      "acted", target.acts() == (acts0 or 0) + 1 and target.report() == rep0 and not bus.since(t4, RP))
        n0 = p.count()
        p.script = [("none",), ("held",)]
        _wait(lambda: p.count() >= n0 + 1, 6.0)
        t5 = p.gets[n0][0]
        time.sleep(max(0.0, t5 + 0.2 - time.monotonic()))
        p.tokens.append(p.s.liveliness().declare_token(f"zk2/{AUTHOR}/@zk/instance/00000000000000a2"))
        time.sleep(max(0.0, t5 + 1.9 - time.monotonic()))
        later = [g[0] - t5 for g in p.gets[n0 + 1:p.count()]]
        reads = (target.watch() or {}).get("reads", [])
        rp.check(sec, "5. P leaves a re-read unanswered and declares a second instance token 0.2 s on: no second GET "
                      "while the first is unanswered; when it ends at its timeout, 1 s, the target makes one GET, "
                      "the seed", len(later) == 1 and 0.95 <= later[0] <= 1.6
                 and [r["why"] for r in reads[-2:]] == ["period", "seed"] and reads[-2]["drew"] == "silent",
                 f"GETs {[round(x, 2) for x in later]} s after the unanswered one, reads {reads[-2:]}")
        target.kill()
        time.sleep(0.5)
        n0, t6 = p.count(), time.monotonic()
        p.default_doc, p.default_seq = v(2), 9
        p.script = [("after", 4.0, ("snapshot", v(2), 9)), ("after", 4.0, ("held",))]
        far = bus.party(TARGET, ["--desired-target", AUTHOR, "--desired-across", "--desired-reply-timeout-ms", "6000",
                                 "--desired-reread-ms", "8000"])
        ok = _wait(lambda: far.report() == converged(s9, 2), 9.0, 0.05)
        took = time.monotonic() - t6
        time.sleep(max(0.0, t6 + 20.0 - time.monotonic()))
        g = p.gets[n0:p.count()]
        overlap = any(b[0] < (a[3] or float("inf")) for a, b in zip(g, g[1:]))
        rp.check(sec, "6. across a face of reply_timeout_ms 6000, a period of 8 s, P answering 4 s after each GET: "
                      "the seed draws v2's snapshot 4 s after it was asked and the target converges on v2; P "
                      "receives at most 3 GETs in the 20 s, and none while another is unanswered",
                 ok and far.instance is not None and len(g) <= 3 and not overlap and g[0][3] is not None
                 and 3.9 <= g[0][3] - g[0][0] <= 4.6, f"converged {took:.1f} s after its start, {len(g)} GETs, "
                 f"each answered after {[round((x[3] or 0) - x[0], 2) for x in g]} s")
        far.kill()
        time.sleep(0.5)
        t7 = time.monotonic()
        bad = bus.party(TARGET, ["--desired-target", AUTHOR, "--desired-across", "--desired-reply-timeout-ms", "6000",
                                 "--desired-reread-ms", "2000"])
        time.sleep(5.0)
        rp.check(sec, "7. the same face with a period of 2 s: the target refuses to start; S sees no instance token "
                      "of lab/worker", bad.instance is None and bad.proc.poll() == 1
                 and bus.token_put(TARGET, t7) is None and any("not started" in x for x in bad.stderr),
                 "".join(bad.stderr).strip()[:160])
    finally:
        p.close()
        bus.close()


def _hb(bus: Bus, t: float) -> list[Any]:
    return [g for g in bus.since(t, HB) if g[2] == "put"]


def section14(rp: Report, moves: bool = True) -> None:
    sec = "§14 the watch" + ("" if moves else ", a target that does not move what it expects")
    bus = Bus(cut=True)
    try:
        author = bus.author("--desired-watched")
        s1 = author.put(TARGET, v(1))
        flags = ["--desired-target", AUTHOR, "--desired-across", "--desired-watch", "--desired-watched",
                 *([] if moves else ["--desired-no-move"])]
        g0, t1 = len(author.gets()), time.monotonic()
        target = bus.party(TARGET, flags, on=2)
        time.sleep(10.0)
        gets = author.gets()[g0:]
        w = target.watch() or {}
        crossed = (w.get("heartbeats") or {})
        rp.check(sec, "1. the target seeds and converges on v1; the first heartbeat it reads makes it read once, "
                      "carrying s1 and answered with it alone; from then on every heartbeat's section is the one it "
                      "expects; no further GET of D in the 10 s, though 4 heartbeats or more crossed",
                 target.report() == converged(s1, 1) and gets == [{"held": None, "alone": False},
                                                                  {"held": s1, "alone": True}]
                 and crossed.get("read") == 1 and crossed.get("unmoved", 0) >= 3 and w.get("expects") is not None
                 and sum(crossed.values()) >= 4 and len(_hb(bus, t1)) >= 4,
                 f"gets {gets}, heartbeats {crossed}, expects {w.get('expects')}")
        if not moves:
            g0, t3 = len(author.gets()), time.monotonic()
            s2 = author.put(TARGET, v(2))
            time.sleep(6.0)
            gets = author.gets()[g0:]
            rp.check(sec, "3. the target hears v2 live and converges on it; one that does not move what it expects "
                          "makes one GET, at the next heartbeat, answered unchanged, and no more",
                     target.report() == converged(s2, 2) and gets == [{"held": s2, "alone": True}], f"gets {gets}")
            return
        g0, t2 = len(author.gets()), time.monotonic()
        author.put(SPARE, v(1))
        time.sleep(6.0)
        gets = author.gets()[g0:]
        rp.check(sec, "2. a document on D2: the next heartbeat's digest differs; one conditional read, answered "
                      "unchanged; Rp receives nothing; then it reads nothing more",
                 gets == [{"held": s1, "alone": True}] and not bus.since2(t2), f"gets {gets}")
        g0, t3 = len(author.gets()), time.monotonic()
        s2 = author.put(TARGET, v(2))
        time.sleep(6.0)
        gets = author.gets()[g0:]
        rp.check(sec, "3. v2 on D: the target hears it live and converges on it; a target that moves what it expects "
                      "makes no GET", target.report() == converged(s2, 2) and gets == [], f"gets {gets}")
        bus.cut()
        s3 = author.put(TARGET, v(3))
        time.sleep(5.0)
        acts0, g0 = target.acts(), len(author.gets())
        still = target.report() == converged(s2, 2)
        bus.heal()
        time.sleep(8.0)
        gets = author.gets()[g0:]
        rp.check(sec, "4. the face cut, v3 put, healed 5 s later: the first heartbeat after the heal follows a gap in "
                      "the seqs; the target reads with s2, the author answers whole, and the target acts on v3; the "
                      "next heartbeat makes it read once more, answered unchanged; then it reads nothing more",
                 still and target.report() == converged(s3, 3) and target.acts() == (acts0 or 0) + 1
                 and gets == [{"held": s2, "alone": False}, {"held": s3, "alone": True}], f"gets {gets}")
        acts0 = target.acts()
        author.kill()
        t5 = time.monotonic()
        author = bus.author("--desired-watched")
        time.sleep(8.0)
        puts = [g for g in bus.since(t5, D) if g[2] == "put"]
        new = puts[0][4] if puts else None
        gets = author.gets()
        hb_runs = {g[4].split("/")[0] for g in _hb(bus, t5) if g[4]}
        rp.check(sec, "5. the author restarts from its record: S receives a put on D holding v3's content under a new "
                      "run; the target hears it and adopts it; the first heartbeat of the new instance is of another "
                      "run; the target reads, and is answered unchanged; Run receives nothing",
                 new is not None and new != s3 and target.report() == converged(new, 3) and target.acts() == acts0
                 and gets == [{"held": new, "alone": True}] and not [g for g in bus.since2(t5) if g[1] == RUN]
                 and len(hb_runs) == 1, f"put {new}, gets {gets}, heartbeat runs {hb_runs}")
        g0, t6, w0 = len(author.gets()), time.monotonic(), target.watch() or {}
        author.pause()
        time.sleep(6.0)
        quiet = len(_hb(bus, t6 + 0.5))
        rep_during = target.report()
        author.resume()
        time.sleep(4.0)
        w1 = target.watch() or {}
        gets = author.gets()[g0:]
        h0, h1 = w0.get("heartbeats") or {}, w1.get("heartbeats") or {}
        rp.check(sec, "6. the author's process stopped for 6 s, past the 4 s horizon: no heartbeat crosses; the "
                      "target keeps v3 in force, Rp and Run receive nothing, and it makes no GET; when the author "
                      "runs again its next heartbeat is in sequence and unmoved: no GET either",
                 quiet == 0 and rep_during == converged(new, 3) and target.report() == converged(new, 3)
                 and not bus.since2(t6) and gets == [] and h1.get("unmoved", 0) > h0.get("unmoved", 0)
                 and h1.get("read") == h0.get("read"),
                 f"{quiet} heartbeats while stopped, gets {gets}, heartbeats {h0} -> {h1}")
    finally:
        bus.close()


def section15(rp: Report) -> None:
    sec = "§15 a delete past the window, then a restart"
    bus = Bus(cut=True)
    try:
        author = bus.author("--desired-window", "10")
        target = bus.party(TARGET, TARGET_FLAGS, on=2)
        s1 = _converge(author, target, 1)
        rp.check(sec, "setup: the target converged on v1, under the author's run R", target.report() == converged(s1, 1))
        acts0 = target.acts()
        bus.cut()
        author.delete(TARGET)
        time.sleep(15.0)
        author.kill()
        author = bus.author("--desired-window", "10")
        ad = author.cmd("desired adoption")
        kind, head, _ = bus.entry(D)
        bus.heal()
        _wait(lambda: any(g["held"] == s1 for g in author.gets()), 10.0, 0.1)
        time.sleep(2.0)
        # G's own GET of D, which carried no held stamp, is the new process's first
        gets = [g for g in author.gets() if g["held"] is not None]
        s1v, headv = d2.Version.parse(s1), d2.Version.parse(head)
        rp.check(sec, "1. past the window the author restarts from its record, which has dropped the delete; the "
                      "target sees its instance token appear and seeds with the held stamp s1; the new instance "
                      "answers a snapshot with no entry under another run: no verdict; the target keeps v1 in force, "
                      "its report stays converged on v1, and Run holds v1's content",
                 kind == "no_entry" and s1v is not None and headv is not None and headv.run != s1v.run
                 and bool(gets) and all(g == {"held": s1, "alone": False} for g in gets)
                 and target.report() == converged(s1, 1) and target.acts() == acts0 and bus.run_doc(on=2) == v(1),
                 f"adoption {ad}, GET {kind} {head}, s1 {s1}, gets {gets}, {target.report()}")
        sd = author.delete(TARGET, force=True)
        want = {"source": "baseline", "outcome": "reverted", "last_known": False, "in_force": "none", **observed(sd)}
        ok = _wait(lambda: target.report() == want, 5.0)
        rp.check(sec, "2. the author deletes D again, though it holds no member there: the target hears a delete "
                      "under the new run, admitted; it reverts; the report is reverted, its observed version that "
                      "delete's", ok and sd is not None and bus.run_doc(on=2) == BASELINE, f"{sd}, {target.report()}")
    finally:
        bus.close()


SECTIONS = {"1": section1, "2": section2, "3": section3, "4": section4, "5": section5, "6": section6, "8": section8,
            "13": section13, "14": section14, "14-no-move": lambda rp: section14(rp, moves=False), "15": section15}


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="python -m zk2py.desired_v2_scenarios", description=__doc__.split("\n")[0])
    ap.add_argument("--only", action="append", choices=sorted(SECTIONS))
    args = ap.parse_args(argv)
    SCRATCH.mkdir(parents=True, exist_ok=True)
    rp = Report()
    for name in args.only or list(SECTIONS):
        try:
            SECTIONS[name](rp)
        except Exception as e:  # noqa: BLE001 - a section that raises fails, and the rest run
            import traceback

            traceback.print_exc()
            rp.check(f"§{name}", "the section ran to its end", False, f"{type(e).__name__}: {e}")
    passed = sum(1 for r in rp.rows if r[2])
    print(f"{passed} of {len(rp.rows)} checks passed; not run: {len(rp.not_run)}", flush=True)
    return 0 if passed == len(rp.rows) else 1


if __name__ == "__main__":
    sys.exit(main())

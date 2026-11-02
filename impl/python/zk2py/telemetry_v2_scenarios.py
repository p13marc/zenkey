"""``spec/profiles/telemetry/scenarios-v2.md`` §1–§7, live (#609, round 98;
``telemetry.v2`` 0.1, #983): ``python -m zk2py.telemetry_v2_scenarios
[--pairing P]… [--only N]…``.

Written from ``scenarios-v2.md`` and ``v2.md`` as their implementer reads
them. Each section runs on a bus of its own: R1, a zk2py router on a free
loopback port; O at ``lab/meterframe``, a client of R1 whose session runs
with timestamping off (core §8.2); R, a reader on another client,
subscribed to ``zk2/lab/meterframe/meterframe.v1/stream/**`` before O
starts, reading no presence. §5's R sits across a constrained face: R2, a
router linked to R1 whose access control denies ``@zk`` on its
``unixsock-stream`` link (``link.v1`` §2.6), R its client over that link.

**The contract** is ``meterframe.v1``, read from
``spec/profiles/telemetry/conformance/contracts/``, which every owner and
reader is given.

**The pairings**, each an owner and a reader:
- ``py-py``: zk2py's owner (:meth:`zk2py.owner.Owner.record_frame`, its
  telemetry.v2 meter, :meth:`~zk2py.owner.Owner.raw_frame`) and zk2py's R
  (:class:`zk2py.telemetry_v2_live.SeriesSubscriber`);
- ``rust-py``: the owner example, a black box (``--telemetry-v2``, its
  ``telemetry frame`` commands, ``--run-file`` for §6), and zk2py's R;
- ``py-zenctl``: zk2py's owner, and ``zenctl watch`` (one per resource,
  given the contract) as R, with ``zenctl get`` reading the frames the
  runner captured, one reply each. zenctl's telemetry verbs read no frame
  yet (#983's T2): it shows each record's field states, and judges no
  series, pair or rate. Each step it cannot judge is named ``not judged``.

**The wire.** Beside R, the runner subscribes to ``zk2/lab/**`` and reads
each frame's body itself, so the owner's rules (the counts modulo each
width, a refusal leaving nothing on the wire) are checked there.

**Time.** A record's instant is given to the owner, ``at`` milliseconds
after its start on its monotonic clock (the owner example's ``at=<ms>``),
and the runner waits for each instant, so frames are put at the spacing the
section names and their times are exact.

**Restarts.** §2's and §6's "O's process ends" is the owner example's
process ending and starting again; zk2py's owner is closed and a new one
opened, with a new session, instance and run (from the run file in §6).

Exit 0 when every check passes, 1 when any fails, 2 when a pairing cannot
run (a black box missing).
"""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path
from typing import Any

from . import frame, telemetry_v2 as t2, zk2c

REPO = Path(__file__).resolve().parents[3]
CONTRACT = REPO / "spec" / "profiles" / "telemetry" / "conformance" / "contracts" / "meterframe.v1.toml"
BLACKBOX = Path(os.environ.get("ZK2PY_BLACKBOX", "/srv/dev/scratch/zenkey/bin/current"))
IFACE, ADDRESS = "meterframe.v1", "lab/meterframe"
LINK, TOTALS, BYTES, BARE = "frames/link", "frames/totals", "frames/bytes", "sdu/dropped_total"
PAIRINGS = ("py-py", "rust-py", "py-zenctl")
NOT_JUDGED = "not judged: zenctl reads no frame's series yet (#983 T2)"


def key_of(res: str) -> str:
    return f"zk2/{ADDRESS}/{IFACE}/stream/{res}"


def contract() -> Any:
    from .contract import load_contract

    c = load_contract(CONTRACT, check_file_name=True)
    assert c.valid and not c.diagnostics, c.codes
    return c


def layouts() -> dict[str, frame.Layout]:
    c = contract()
    return {r["template"]: frame.layout_of(c, r) for r in c.canonical["resources"] if r.get("frame")}


class Report:
    def __init__(self) -> None:
        self.rows: list[tuple[str, str, str, bool | None, str]] = []

    def check(self, pairing: str, section: str, name: str, ok: bool, detail: str = "") -> bool:
        self.rows.append((pairing, section, name, ok, detail))
        print(f"{'PASS' if ok else 'FAIL'} [{pairing}] [{section}] {name}" + (f": {detail}" if detail else ""),
              flush=True)
        return ok

    def not_judged(self, pairing: str, section: str, name: str) -> None:
        self.rows.append((pairing, section, name, None, NOT_JUDGED))
        print(f"SKIP [{pairing}] [{section}] {name}: {NOT_JUDGED}", flush=True)

    def note(self, pairing: str, section: str, text: str) -> None:
        print(f"NOTE [{pairing}] [{section}] {text}", flush=True)


# -- the owners -------------------------------------------------------------------------------

class PyOwner:
    """zk2py's owner of ``meterframe.v1`` at ``lab/meterframe``, a client of
    R1, its frames metered by telemetry.v2 (:meth:`zk2py.owner.Owner._meter`)."""

    impl = "zk2py"

    def __init__(self, ep: str, run_file: str | None = None) -> None:
        self.ep, self.run_file = ep, run_file
        self._start()

    def _start(self) -> None:
        from .owner import Owner

        self.o = Owner("lab", "meterframe", [contract()], connect=self.ep, run_record=self.run_file)
        self.o.start()
        self.t0 = time.monotonic_ns()

    def now_ms(self) -> int:
        return (time.monotonic_ns() - self.t0) // 1_000_000

    def wait_until(self, at_ms: int) -> None:
        d = at_ms - self.now_ms()
        if d > 0:
            time.sleep(d / 1000)

    @staticmethod
    def _put(p: dict[str, Any]) -> dict[str, Any]:
        return {"run": p["run"], "seq": p["seq"], "records": len(p["records"]["records"])}

    def record(self, res: str, values: dict[str, Any], at_ms: int | None = None) -> dict[str, Any]:
        from . import packed

        at = None if at_ms is None else self.t0 + at_ms * 1_000_000
        try:
            return {"put": [self._put(p) for p in self.o.record_frame(key_of(res), values, at)]}
        except (frame.Refused, packed.Refused) as e:
            return {"refused": str(e)}

    def flush(self, res: str) -> dict[str, Any] | None:
        p = self.o.end_frame(key_of(res))
        return None if p is None else self._put(p)

    def lose(self, res: str) -> None:
        self.o.lose_frame(key_of(res))

    def raw(self, res: str, values: dict[str, Any]) -> dict[str, Any]:
        return self._put(self.o.raw_frame(key_of(res), values))

    def bare(self, res: str, value: Any) -> str:
        self.o.publish(key_of(res), json.dumps(value).encode())
        return "put"

    bare_raw = bare

    def run(self) -> str:
        return self.o.run_hex()

    def instance(self) -> str:
        return self.o.instance

    def warnings(self) -> list[str]:
        return [w for m in self.o._meters.values() if m is not None for w in m.warnings]

    def restart(self) -> None:
        self.o.close()
        self._start()

    def close(self) -> None:
        self.o.close()


class RustOwner:
    """The owner example, a black box, with ``--telemetry-v2`` (its embedded
    ``meterframe.v1``, the spec's) and ``--run-file`` when given."""

    impl = "rust"

    def __init__(self, ep: str, run_file: str | None = None) -> None:
        self.ep, self.run_file = ep, run_file
        self._start()

    def _start(self) -> None:
        from .live_interop import Owner as Exe

        flags = ["--telemetry-v2"] + (["--run-file", self.run_file] if self.run_file else [])
        self.p = Exe(BLACKBOX / "owner", ADDRESS, [CONTRACT], connect=self.ep, flags=flags)
        got = self.p.wait_for("ready ", 120)
        if got is None:
            raise RuntimeError(f"the owner example did not start: {self.p.stderr[-3:]}")
        self.t0 = time.monotonic_ns()
        self._instance = got.rsplit("/", 1)[-1]
        self._run: str | None = None

    def now_ms(self) -> int:
        return (time.monotonic_ns() - self.t0) // 1_000_000

    def wait_until(self, at_ms: int) -> None:
        d = at_ms - self.now_ms()
        if d > 0:
            time.sleep(d / 1000)

    def _line(self, line: str) -> str:
        got = self.p.command(line)
        if got is None:
            raise RuntimeError(f"{line!r}: no answer ({self.p.stderr[-3:]})")
        return got

    def _json(self, line: str) -> dict[str, Any]:
        got = self._line(line)
        if got.startswith("refused"):
            return {"refused": got}
        out = json.loads(got)
        put = out.get("put")
        if isinstance(put, dict) and put.get("run"):
            self._run = put["run"]
        return out

    def record(self, res: str, values: dict[str, Any], at_ms: int | None = None) -> dict[str, Any]:
        got = self._json(f"telemetry frame record {res} {json.dumps(values)}"
                         + ("" if at_ms is None else f" at={at_ms}"))
        if "refused" in got:
            return got
        return {"put": [] if got.get("put") is None else [got["put"]]}

    def flush(self, res: str) -> dict[str, Any] | None:
        return self._json(f"telemetry frame flush {res}").get("put")

    def lose(self, res: str) -> None:
        self._json(f"telemetry frame lose {res}")

    def raw(self, res: str, values: dict[str, Any]) -> dict[str, Any]:
        return self._json(f"telemetry frame raw {res} {json.dumps(values)}").get("put")

    def bare(self, res: str, value: Any) -> str:
        return self._line(f"telemetry put {res} {json.dumps(value)}")

    def bare_raw(self, res: str, value: Any) -> str:
        return self._line(f"telemetry raw {res} {json.dumps(value)}")

    def run(self) -> str:
        return self._run or "?"

    def instance(self) -> str:
        return self._instance

    def warnings(self) -> list[str]:
        return [x for x in self.p.stderr if "telemetry.v2" in x and "WARN" in x]

    def restart(self) -> None:
        self.p.close()
        self._start()

    def close(self) -> None:
        self.p.close()


def make_owner(impl: str, ep: str, **kw: Any) -> Any:
    return RustOwner(ep, **kw) if impl == "rust" else PyOwner(ep, **kw)


# -- the readers ------------------------------------------------------------------------------

class PyReader:
    """zk2py's R (:class:`zk2py.telemetry_v2_live.SeriesSubscriber`)."""

    impl = "zk2py"

    def __init__(self, session: Any, ep: str) -> None:
        from .telemetry_v2_live import SeriesSubscriber

        self.s = SeriesSubscriber(session, ADDRESS, contract())

    def frames(self, res: str, n: int, timeout: float = 5.0) -> list[dict[str, Any]]:
        got = self.s.wait(key_of(res), n, timeout)
        if self.s.errors():
            # R's own failure (round 99's mend): never read as the owner's.
            raise RuntimeError(f"zk2py's R raised: {self.s.errors()}")
        return [g for g in got if "undecodable" not in g]

    def pairs(self, res: str) -> list[t2.Pair]:
        return self.s.pairs(key_of(res))

    def close(self) -> None:
        self.s.close()


class _Watch:
    """``zenctl watch lab/meterframe meterframe.v1 <resource>`` in ndjson,
    given the contract: each sample row's ``payload.value``."""

    def __init__(self, ep: str, res: str, for_s: float) -> None:
        self.proc = subprocess.Popen([str(BLACKBOX / "zenctl"), "watch", ADDRESS, IFACE, res, "--for", f"{for_s:g}",
                                      "--format", "ndjson", "-c", ep, "--contracts", str(CONTRACT)],
                                     stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        self.rows: list[dict[str, Any]] = []
        self.err: list[str] = []
        self.lock = threading.Lock()
        threading.Thread(target=self._pump, daemon=True).start()
        threading.Thread(target=lambda: self.err.extend(self.proc.stderr), daemon=True).start()

    def _pump(self) -> None:
        for line in self.proc.stdout:
            try:
                row = json.loads(line)
            except ValueError:
                continue
            with self.lock:
                self.rows.append(row)

    def values(self) -> list[Any]:
        with self.lock:
            return [r["payload"].get("value") for r in self.rows
                    if r.get("row") == "sample" and isinstance(r.get("payload"), dict)]

    def close(self) -> None:
        if self.proc.poll() is None:
            self.proc.terminate()
            try:
                self.proc.wait(timeout=10)
            except subprocess.TimeoutExpired:
                self.proc.kill()
                self.proc.wait()


class ZReader:
    """``zenctl watch``, one per resource, given the contract."""

    impl = "zenctl"

    def __init__(self, session: Any, ep: str, for_s: float = 40.0) -> None:
        self.watches = {res: _Watch(ep, res, for_s) for res in (LINK, TOTALS, BYTES, BARE)}
        time.sleep(2.5)  # each watch's subscriber declared before O's first frame

    def frames(self, res: str, n: int, timeout: float = 5.0) -> list[dict[str, Any]]:
        w = self.watches[res]
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline and len(w.values()) < n:
            time.sleep(0.05)
        return [v for v in w.values() if isinstance(v, dict) and "records" in v]

    def bare(self, n: int, timeout: float = 5.0) -> list[Any]:
        w = self.watches[BARE]
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline and len(w.values()) < n:
            time.sleep(0.05)
        return w.values()

    def close(self) -> None:
        for w in self.watches.values():
            w.close()


def make_reader(impl: str, session: Any, ep: str) -> Any:
    return ZReader(session, ep) if impl == "zenctl" else PyReader(session, ep)


class TelemetryTool:
    """``zenctl telemetry zk2/lab/meterframe/**`` for ``for_s`` seconds, in
    json, given the contract: its window opens at its "listening" line,
    which this waits for."""

    def __init__(self, ep: str, for_s: float) -> None:
        self.proc = subprocess.Popen([str(BLACKBOX / "zenctl"), "telemetry", f"zk2/{ADDRESS}/**", "--for",
                                      f"{for_s:g}", "--format", "json", "-c", ep, "--contracts", str(CONTRACT)],
                                     stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        self.err: list[str] = []
        threading.Thread(target=lambda: self.err.extend(self.proc.stderr), daemon=True).start()
        deadline = time.monotonic() + 30
        while time.monotonic() < deadline and not any("listening" in x for x in self.err):
            time.sleep(0.05)

    def result(self) -> dict[str, Any] | None:
        try:
            out, _ = self.proc.communicate(timeout=60)
        except subprocess.TimeoutExpired:
            self.proc.kill()
            out, _ = self.proc.communicate()
        try:
            return json.loads(out)
        except ValueError:
            return None


class Wire:
    """Every put under ``zk2/lab/**``, each frame read here with the layout."""

    def __init__(self, session: Any) -> None:
        import zenoh

        self.lock = threading.Lock()
        self.got: list[tuple[str, str, bytes, str]] = []
        self.sub = session.declare_subscriber("zk2/lab/**", zenoh.handlers.Callback(self._heard))
        self.layouts = layouts()

    def _heard(self, sample: Any) -> None:
        import zenoh

        kind = "delete" if sample.kind == zenoh.SampleKind.DELETE else "put"
        with self.lock:
            self.got.append((str(sample.key_expr), kind, sample.payload.to_bytes(), str(sample.encoding)))

    def on(self, key: str) -> list[tuple[str, bytes, str]]:
        with self.lock:
            return [(k, p, e) for kk, k, p, e in self.got if kk == key]

    def frames(self, res: str) -> list[dict[str, Any]]:
        key, lo = key_of(res), self.layouts[res]
        out = []
        for kind, payload, enc in self.on(key):
            r = zk2c.read_put(key, enc, payload) if kind == "put" else None
            row: dict[str, Any] = {"kind": kind, "form": None if r is None else r.form, "payload": payload}
            if r is not None and r.stamp is not None:
                row.update(run=r.stamp.run_hex(), seq=r.stamp.seq)
            if r is not None and r.form == "frame":
                try:
                    row["value"] = lo.read(r.tag, r.record or b"")
                except Exception as e:  # noqa: BLE001 - named
                    row["refused"] = str(e)
            out.append(row)
        return out

    def wait(self, res: str, n: int, timeout: float = 5.0) -> list[dict[str, Any]]:
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline and len(self.on(key_of(res))) < n:
            time.sleep(0.02)
        return self.frames(res)

    def close(self) -> None:
        try:
            self.sub.undeclare()
        except Exception:  # noqa: BLE001
            pass


class FaceRouter:
    """§5's face (core §8.5, ``link.v1`` §2.6): R2, linked to R1, its access
    control denying ``zk2/*/*/@zk/**`` to every session on its
    ``unixsock-stream`` link, in every message and both flows; R's session
    over that link, with a liveliness subscriber that shows no token
    crossing."""

    MESSAGES = ["put", "delete", "declare_subscriber", "query", "reply", "declare_queryable",
                "liveliness_token", "declare_liveliness_subscriber", "liveliness_query"]

    def __init__(self, r1_endpoint: str) -> None:
        import zenoh

        self.dir = tempfile.mkdtemp(prefix="zk2py-t2-face-", dir="/srv/dev/scratch/zenkey/py")
        sock = os.path.join(self.dir, "face.sock")
        self.endpoint = f"unixsock-stream/{sock}"
        conf = zenoh.Config()
        conf.insert_json5("mode", json.dumps("router"))
        conf.insert_json5("listen/endpoints", json.dumps([self.endpoint]))
        conf.insert_json5("connect/endpoints", json.dumps([r1_endpoint]))
        conf.insert_json5("scouting/multicast/enabled", "false")
        conf.insert_json5("timestamping/enabled", "false")
        conf.insert_json5("access_control", json.dumps({
            "enabled": True, "default_permission": "allow",
            "rules": [{"id": "face-zk", "messages": self.MESSAGES, "flows": ["egress", "ingress"],
                       "permission": "deny", "key_exprs": ["zk2/*/*/@zk/**"]}],
            "subjects": [{"id": "far", "link_protocols": ["unixsock-stream"]}],
            "policies": [{"rules": ["face-zk"], "subjects": ["far"]}],
        }))
        self.router = zenoh.open(conf)
        c = zenoh.Config()
        c.insert_json5("mode", json.dumps("client"))
        c.insert_json5("connect/endpoints", json.dumps([self.endpoint]))
        c.insert_json5("scouting/multicast/enabled", "false")
        self.session = zenoh.open(c)
        self.tokens: list[str] = []
        self.lv = self.session.liveliness().declare_subscriber(
            "zk2/**", zenoh.handlers.Callback(lambda s: self.tokens.append(str(s.key_expr))), history=True)

    def close(self) -> None:
        for x in (self.lv,):
            try:
                x.undeclare()
            except Exception:  # noqa: BLE001
                pass
        for x in (self.session, self.router):
            try:
                x.close()
            except Exception:  # noqa: BLE001
                pass
        shutil.rmtree(self.dir, ignore_errors=True)


class Bus:
    def __init__(self, face: bool = False) -> None:
        from . import live
        from .live_interop import _r1

        self.router, self.endpoint, _ = _r1(timestamping=False)
        self.sessions = [live.open_client(self.endpoint) for _ in range(3)]
        self.r_session, self.w_session, self.x_session = self.sessions
        self.face = FaceRouter(self.endpoint) if face else None
        if self.face is not None:
            self.r_session = self.face.session
        self.wire = Wire(self.w_session)
        self.closers: list[Any] = [self.wire]

    @property
    def r_endpoint(self) -> str:
        return self.face.endpoint if self.face is not None else self.endpoint

    def close(self) -> None:
        for x in reversed(self.closers):
            try:
                x.close()
            except Exception:  # noqa: BLE001 - closing anyway
                pass
        if self.face is not None:
            self.face.close()
        for s in self.sessions:
            s.close()
        self.router.close()


# -- reading the readings ---------------------------------------------------------------------

def zget(bus: Bus, res: str, payloads: list[bytes]) -> tuple[list[dict[str, Any]], str]:
    """``zenctl get`` of a frame key, answered by the runner with each frame
    the wire captured, one reply each: each reply as zenctl reads it alone
    (observer §7.6)."""
    import zenoh

    key = key_of(res)

    def answer(q: Any) -> None:
        for p in payloads:
            q.reply(key, p, encoding=zk2c.ENCODING)

    qb = bus.x_session.declare_queryable(key, zenoh.handlers.Callback(answer))
    try:
        time.sleep(0.5)
        r = subprocess.run([str(BLACKBOX / "zenctl"), "get", key, "--format", "json", "-c", bus.endpoint,
                            "--timeout", "2", "--contracts", str(CONTRACT)],
                           capture_output=True, text=True, timeout=60)
    finally:
        qb.undeclare()
    try:
        doc = json.loads(r.stdout)
    except ValueError:
        return [], f"exit {r.returncode}: {r.stdout[:300]} {r.stderr[-300:]}"
    found: list[dict[str, Any]] = []

    def walk(x: Any) -> None:
        if isinstance(x, dict):
            if "records" in x and "follows" in x:
                found.append(x)
                return
            for v in x.values():
                walk(v)
        elif isinstance(x, list):
            for v in x:
                walk(v)

    walk(doc)
    return found, f"exit {r.returncode}"


def zstate(g: dict[str, Any], i: int, name: str) -> str:
    """A zenctl frame reading's field, as ``known 10``, ``unknown gap``."""
    f = g["records"][i]["fields"].get(name, {})
    st = f.get("state")
    if st in ("known", "within"):
        return f"{st} {json.dumps(f.get('value'))}"
    if st == "unknown":
        return f"unknown {f.get('why')}"
    return str(st)


def rd(g: dict[str, Any], i: int, name: str) -> str:
    """zk2py's reading of a series field (§2.18), as ``sample 10``, ``held
    known 10``, ``unknown gap``, ``absent``."""
    x = g["records"][i]["fields"][name]
    r = x["reading"]
    if r == "sample":
        return f"sample {json.dumps(x['value'])}"
    if r == "held":
        return f"held {x['state']} {json.dumps(x['value'])}"
    if r == "unknown":
        return f"unknown {x['why']}"
    return r


def pshort(p: t2.Pair) -> str:
    rate = "" if p.rate is None else f" {p.rate:g} {p.rate_unit} over {p.elapsed_ns / 1e6:g} ms {p.time_basis}" \
        + ("" if p.jitter_ms is None else f" ±{p.jitter_ms} ms")
    return (f"{p.field} {p.continuity}" + (f"({p.why})" if p.why else "") + f" {p.epoch}/{p.via}" + rate
            + ("" if p.no_rate is None else f" no rate ({p.no_rate})"))


#: the conventions: "Jitter on loopback is allowed 50 ms", on an elapsed time
#: two frames' instants give (base 2); the rate is still v1 §2.11's formula
#: over the elapsed time the reader read, exactly
JITTER_NS = 50_000_000


def rated(p: t2.Pair, d: int, nominal_ns: int, basis: str, exact: bool = False) -> bool:
    """A pair with a rate: same_epoch, established in band, its basis, its
    elapsed time within the loopback's jitter of ``nominal_ns`` (exactly it
    with ``exact``: one frame's offsets), and its rate the formula's over
    the increase ``d`` and that elapsed time, bit for bit."""
    return (p.continuity, p.epoch, p.via, p.time_basis) == ("same_epoch", "established", "in_band", basis) \
        and p.elapsed_ns is not None and abs(p.elapsed_ns - nominal_ns) <= (0 if exact else JITTER_NS) \
        and p.rate == t2.tm.rate_of(d, p.elapsed_ns)


def _steps(o: Any, start_ms: int, steps: list[tuple[int, Any]]) -> None:
    """Each step at its instant, ``start_ms`` + its offset on the owner's
    clock, the runner waiting for it."""
    for off, fn in steps:
        o.wait_until(start_ms + off)
        fn(start_ms + off)


# -- §1 ---------------------------------------------------------------------------------------

LINK_BASE = {"queue_depth": 3, "duty_available": 1, "duty_wait_ms": "0"}


def section1(report: Report, pairing: str) -> None:
    sec = "§1 a counter's rate, measured and scheduled"
    owner_impl, reader_impl = pairing.split("-")
    bus = Bus()
    try:
        r = make_reader(reader_impl, bus.r_session, bus.endpoint)
        bus.closers.append(r)
        o = make_owner(owner_impl, bus.endpoint)
        bus.closers.append(o)
        time.sleep(1.0)
        t = o.now_ms() + 200
        got: list[Any] = []
        _steps(o, t, [
            (0, lambda at: got.append(o.record(LINK, {"dropped": 10, **LINK_BASE}, at))),
            (2000, lambda at: got.append(o.record(LINK, {"dropped": 20, **LINK_BASE}, at))),
            (2100, lambda at: got.append(o.record(BYTES, {"rx": 1000}, at))),
            (3100, lambda at: got.append(o.record(BYTES, {"rx": 1500}, at))),
            (4100, lambda at: got.append(o.record(BYTES, {"rx": 2100}, at))),
            (5100, lambda at: got.append(o.record(BYTES, {"rx": 2600}, at))),
            (5200, lambda at: got.append(o.flush(BYTES))),
        ])
        wl, wb = bus.wire.wait(LINK, 2), bus.wire.wait(BYTES, 2)
        report.check(pairing, sec, "steps 1 to 3: two frames/link frames of one full record, then a frames/bytes "
                                   "frame of three records (1 and 2 scheduled) and one of one record, one run",
                     [len(x.get("value", {}).get("records", [])) for x in wl] == [1, 1]
                     and [len(x.get("value", {}).get("records", [])) for x in wb] == [3, 1]
                     and [rr.get("at") for rr in wb[0]["value"]["records"][1:]] == ["scheduled", "scheduled"]
                     and len({x.get("run") for x in wl + wb}) == 1,
                     f"link {[(x.get('run'), x.get('seq')) for x in wl]}, bytes "
                     f"{[(x.get('seq'), [rr.get('at', 0) for rr in x['value']['records']]) for x in wb if 'value' in x]}")
        if reader_impl == "zenctl":
            fl, fb = r.frames(LINK, 2), r.frames(BYTES, 2)
            ok = len(fl) == 2 and all(zstate(g, 0, n) == f"known {json.dumps(v)}" for g, d in zip(fl, (10, 20))
                                      for n, v in {"dropped": d, **LINK_BASE}.items())
            report.check(pairing, sec, "expected 1 and 5, as zenctl shows them: every frames/link field known, "
                                       "dropped 10 then 20 (a sample is not a state zenctl names)", ok,
                         f"{[[zstate(g, 0, n) for n in ('dropped', 'queue_depth')] for g in fl]}")
            rec = [] if not fb else fb[0]["records"]
            report.check(pairing, sec, "expected 2, as zenctl shows it: frames/bytes' records 1 and 2 scheduled with "
                                       "their jitter_ms, never measured, rx 1000, 1500, 2100 then 2600",
                         len(fb) == 2 and [x.get("time") for x in rec] == ["measured", "scheduled", "scheduled"]
                         and all(x.get("jitter_ms") == 100 for x in rec[1:])
                         and [zstate(g, i, "rx") for g in fb for i in range(len(g["records"]))]
                         == ["known 1000", "known 1500", "known 2100", "known 2600"],
                         f"{[(x.get('time'), x.get('jitter_ms')) for x in rec]}")
            payloads = [x["payload"] for x in wl + wb]
            alone, how = zget(bus, LINK, [x["payload"] for x in wl])
            report.check(pairing, sec, "zenctl get, each frame read alone (one reply each): every field known",
                         len(alone) == 2 and all(g["records"][0]["fields"]["dropped"].get("state") == "known"
                                                 for g in alone), f"{how}, {len(alone)} of {len(payloads)}")
            for step in ("expected 1: dropped 10 → 20 same_epoch in band, 5 in 1/s over 2,000 ms, measured",
                         "expected 2: rx paired within the frame by offsets, 500 and 600 By/s, scheduled ±100 and "
                         "±200 ms",
                         "expected 3: rx 2100 → 2600 across two frames by instants, 500 By/s, scheduled ±100 ms",
                         "expected 4: each scheduled rate shown as scheduled with its bound",
                         "expected 5: the gauges form no pair"):
                report.not_judged(pairing, sec, step)
            return
        fl, fb = r.frames(LINK, 2), r.frames(BYTES, 2)
        report.check(pairing, sec, "expected 1: R reads every series field of frames/link as a sample",
                     len(fl) == 2 and all(x["reading"] == "sample" for g in fl for x in g["records"][0]["fields"].values()),
                     f"{[[rd(g, 0, n) for n in g['records'][0]['fields']] for g in fl]}")
        pl = r.pairs(LINK)
        report.check(pairing, sec, "expected 1: R pairs dropped 10 → 20 same_epoch, established in band, 5 in 1/s "
                                   "over 2,000 ms, measured",
                     len(pl) == 1 and rated(pl[0], 10, 2_000_000_000, "measured") and pl[0].rate_unit == "1/s",
                     "; ".join(pshort(p) for p in pl))
        pb = r.pairs(BYTES)
        report.check(pairing, sec, "expected 2: R pairs rx within the frame by the records' offsets: 1000 → 1500 at "
                                   "500 By/s, 1500 → 2100 at 600 By/s, each over 1,000 ms, scheduled ±100 and ±200 ms",
                     len(pb) == 3 and [(p.rate, p.elapsed_ns, p.jitter_ms) for p in pb[:2]]
                     == [(500.0, 1_000_000_000, 100), (600.0, 1_000_000_000, 200)]
                     and all(rated(p, d, 1_000_000_000, "scheduled", exact=True) and p.rate_unit == "By/s"
                             and p.earlier[0] == p.later[0] == 0 for p, d in zip(pb[:2], (500, 600))),
                     "; ".join(pshort(p) for p in pb[:2]))
        report.check(pairing, sec, "expected 3: R pairs 2100 → 2600 across the two frames by their instants: 500 "
                                   "By/s over 1,000 ms, scheduled ±100 ms",
                     len(pb) == 3 and rated(pb[2], 500, 1_000_000_000, "scheduled") and pb[2].jitter_ms == 100
                     and pb[2].earlier[0] != pb[2].later[0], "; ".join(pshort(p) for p in pb[2:]))
        report.check(pairing, sec, "expected 4: R shows each scheduled rate as scheduled, with its bound, never as "
                                   "measured",
                     all(p.time_basis == "scheduled" and p.jitter_ms is not None for p in pb) and len(pb) == 3,
                     f"{[(p.time_basis, p.jitter_ms) for p in pb]}")
        report.check(pairing, sec, "expected 5: queue_depth, duty_available and duty_wait_ms are gauges: samples, "
                                   "and no pair",
                     {p.field for p in pl} == {"dropped"} and all(
                         g["records"][0]["fields"][n]["reading"] == "sample" and
                         g["records"][0]["fields"][n]["kind"] == "gauge"
                         for g in fl for n in ("queue_depth", "duty_available", "duty_wait_ms")),
                     f"pairs of {sorted({p.field for p in pl})}")
    finally:
        bus.close()


# -- §2 ---------------------------------------------------------------------------------------

def section2(report: Report, pairing: str) -> None:
    sec = "§2 a restart: a new run, a reset"
    owner_impl, reader_impl = pairing.split("-")
    bus = Bus()
    try:
        r = make_reader(reader_impl, bus.r_session, bus.endpoint)
        bus.closers.append(r)
        o = make_owner(owner_impl, bus.endpoint)
        bus.closers.append(o)
        time.sleep(1.0)
        t = o.now_ms() + 200
        _steps(o, t, [(0, lambda at: o.record(TOTALS, {"frames_total": 100, "temperature": "21.5"}, at)),
                      (1000, lambda at: o.record(TOTALS, {"frames_total": 140, "temperature": "21.5"}, at))])
        i1, r1 = o.instance(), o.run()
        bus.wire.wait(TOTALS, 2)
        o.restart()
        time.sleep(1.0)
        t = o.now_ms() + 200
        _steps(o, t, [(0, lambda at: o.record(TOTALS, {"frames_total": 3, "temperature": "21.5"}, at)),
                      (1000, lambda at: o.record(TOTALS, {"frames_total": 9, "temperature": "21.5"}, at))])
        i2, r2 = o.instance(), o.run()
        w = bus.wire.wait(TOTALS, 4)
        runs = [x.get("run") for x in w]
        report.check(pairing, sec, "steps 1 and 2: I1 under r1, then I2 under r2, a new instance and a new run; "
                                   "I2's first frame a full record",
                     i1 != i2 and len(runs) == 4 and runs[0] == runs[1] != runs[2] == runs[3]
                     and w[2]["value"]["records"][0]["full"],
                     f"instances {i1} {i2}, runs {runs}, seqs {[x.get('seq') for x in w]}")
        g = r.frames(TOTALS, 4)
        if reader_impl == "zenctl":
            report.check(pairing, sec, "expected 2, as zenctl shows it: I2's first frame follows another run, every "
                                       "field known again from its keyframe",
                         len(g) == 4 and g[2].get("follows") == "run"
                         and zstate(g[2], 0, "frames_total") == "known 3",
                         f"{[(x.get('follows'), zstate(x, 0, 'frames_total')) for x in g]}")
            for step in ("expected 1: 100 → 140 same_epoch, established in band, with a rate",
                         "expected 3: 140 → 3 reset, established in band, no rate, no finding",
                         "expected 4: 3 → 9 same_epoch again, with a rate"):
                report.not_judged(pairing, sec, step)
            return
        p = r.pairs(TOTALS)
        f = [x for x in p if x.field == "frames_total"]
        report.check(pairing, sec, "expected 1: R pairs 100 → 140 as same_epoch, established in band, with a rate",
                     len(f) >= 1 and rated(f[0], 40, 1_000_000_000, "measured"), "; ".join(pshort(x) for x in f[:1]))
        report.check(pairing, sec, "expected 2: R reads I2's first frame as another run's; that keyframe makes each "
                                   "field known again: every field a sample",
                     len(g) == 4 and g[2]["follows"] == "run"
                     and [rd(g[2], 0, n) for n in ("frames_total", "temperature")] == ["sample 3", 'sample "21.5"'],
                     f"{[(x['follows'], [rd(x, 0, n) for n in ('frames_total', 'temperature')]) for x in g]}")
        report.check(pairing, sec, "expected 3: R pairs 140 → 3 as reset, established in band; no rate, no finding; "
                                   "no presence was read",
                     len(f) >= 2 and (f[1].continuity, f[1].epoch, f[1].via, f[1].no_rate)
                     == ("reset", "established", "in_band", "reset"),
                     "; ".join(pshort(x) for x in f[1:2]))
        report.check(pairing, sec, "expected 4: R pairs 3 → 9 as same_epoch again, with a rate",
                     len(f) == 3 and rated(f[2], 6, 1_000_000_000, "measured"),
                     "; ".join(pshort(x) for x in f[2:]))
        report.note(pairing, sec, f"runs {r1} then {r2}; frames_total's answer over the window: "
                                  f"{t2.series_answer('counter', 4, f)}")
    finally:
        bus.close()


# -- §3 ---------------------------------------------------------------------------------------

def section3(report: Report, pairing: str) -> None:
    sec = "§3 a 24-bit counter's wrap"
    owner_impl, reader_impl = pairing.split("-")
    bus = Bus()
    try:
        r = make_reader(reader_impl, bus.r_session, bus.endpoint)
        bus.closers.append(r)
        o = make_owner(owner_impl, bus.endpoint)
        bus.closers.append(o)
        time.sleep(1.0)
        t = o.now_ms() + 200
        got: list[Any] = []
        _steps(o, t, [(0, lambda at: got.append(o.record(BYTES, {"rx": 16_777_000}, at))),
                      (1000, lambda at: got.append(o.record(BYTES, {"rx": 16_777_316}, at))),
                      (2000, lambda at: got.append(o.record(BYTES, {"rx": 25_777_316}, at)))])
        w = bus.wire.wait(BYTES, 1)
        carried = [rr["values"].get("rx") for rr in w[0]["value"]["records"]] if w and "value" in w[0] else []
        report.check(pairing, sec, "step 1: the owner takes the three counts (no decrease: each above the last); "
                                   "expected 1: the frame carries rx as 16,777,000, 100 and 9,000,100, each count "
                                   "modulo 2^24",
                     all("refused" not in x for x in got) and carried == [16_777_000, 100, 9_000_100],
                     f"{carried}; answers {got}")
        report.note(pairing, sec, f"the owner's word on §2.20's SHOULD: {o.warnings()[-1:] or 'none'}")
        if reader_impl == "zenctl":
            g = r.frames(BYTES, 1)
            report.check(pairing, sec, "as zenctl shows it: rx known 16777000, 100, 9000100, the counts as carried",
                         len(g) == 1 and [zstate(g[0], i, "rx") for i in range(3)]
                         == ["known 16777000", "known 100", "known 9000100"],
                         f"{[zstate(g[0], i, 'rx') for i in range(len(g[0]['records']))] if g else None}")
            for step in ("expected 2: 16,777,000 → 100 same_epoch, an increase of 316 at 316 By/s, scheduled",
                         "expected 3: 100 → 9,000,100 unobservable (wrap_or_decrease), no rate"):
                report.not_judged(pairing, sec, step)
            return
        r.frames(BYTES, 1)
        p = r.pairs(BYTES)
        report.check(pairing, sec, "expected 2: R pairs 16,777,000 → 100 as same_epoch: an increase of 316, the wrap "
                                   "included, at 316 By/s, scheduled",
                     len(p) >= 1 and (p[0].continuity, p[0].rate, p[0].rate_unit, p[0].time_basis)
                     == ("same_epoch", 316.0, "By/s", "scheduled"), "; ".join(pshort(x) for x in p[:1]))
        report.check(pairing, sec, "expected 3: R pairs 100 → 9,000,100 as unobservable (wrap_or_decrease), no rate "
                                   "(wrap_or_decrease): no decrease, no finding, no rate of billions",
                     len(p) == 2 and (p[1].continuity, p[1].why, p[1].no_rate, p[1].rate)
                     == ("unobservable", "wrap_or_decrease", "wrap_or_decrease", None),
                     "; ".join(pshort(x) for x in p[1:]))
        report.note(pairing, sec, f"rx's answer over the window: {t2.series_answer('counter', 3, p)} "
                                  "(could not tell: a wrap_or_decrease pair and none decreased, §5)")
    finally:
        bus.close()


# -- §4 ---------------------------------------------------------------------------------------

def section4(report: Report, pairing: str) -> None:
    sec = "§4 a lost frame: unknown, never zero"
    owner_impl, reader_impl = pairing.split("-")
    bus = Bus()
    try:
        r = make_reader(reader_impl, bus.r_session, bus.endpoint)
        bus.closers.append(r)
        o = make_owner(owner_impl, bus.endpoint)
        bus.closers.append(o)
        time.sleep(1.0)
        t = o.now_ms() + 200
        _steps(o, t, [
            (0, lambda at: o.record(TOTALS, {"frames_total": 10, "temperature": "20"}, at)),
            (1000, lambda at: o.record(TOTALS, {"frames_total": 10, "temperature": "20.5"}, at)),
            (2000, lambda at: (o.lose(TOTALS), o.record(TOTALS, {"frames_total": 15, "temperature": "20.5"}, at))),
            (3000, lambda at: o.record(TOTALS, {"frames_total": 15, "temperature": "21"}, at)),
            (4000, lambda at: o.record(TOTALS, {"frames_total": 18, "temperature": "21"}, at)),
        ])
        w = bus.wire.wait(TOTALS, 4)
        inc = [(x.get("seq"), x["value"]["records"][0]["full"], sorted(x["value"]["records"][0]["values"]))
               for x in w if "value" in x]
        report.check(pairing, sec, "steps 1 and 2: four frames on the wire at seqs 0, 1, 3, 4 (seq 2 lost): a "
                                   "keyframe, a delta of temperature, a delta of temperature, a keyframe",
                     inc == [(0, True, ["frames_total", "temperature"]), (1, False, ["temperature"]),
                             (3, False, ["temperature"]), (4, True, ["frames_total", "temperature"])], f"{inc}")
        g = r.frames(TOTALS, 4)
        if reader_impl == "zenctl":
            report.check(pairing, sec, "expected 1 and 2, as zenctl shows them: record 2's frames_total known 10 "
                                       "(held); record 4 after a gap of one, frames_total unknown (gap), no value, "
                                       "temperature known 21; record 5 frames_total known 18",
                         len(g) == 4 and zstate(g[1], 0, "frames_total") == "known 10"
                         and zstate(g[2], 0, "frames_total") == "unknown gap"
                         and "value" not in g[2]["records"][0]["fields"]["frames_total"]
                         and zstate(g[2], 0, "temperature") == 'known "21"'
                         and zstate(g[3], 0, "frames_total") == "known 18",
                         f"{[(x.get('follows'), zstate(x, 0, 'frames_total'), zstate(x, 0, 'temperature')) for x in g]}")
            report.not_judged(pairing, sec, "expected 3: 10 → 18 same_epoch over the 4 s between the instants, "
                                            "2 in 1/s, measured")
            return
        report.check(pairing, sec, "expected 1: R reads record 2's frames_total as held, known 10: no sample",
                     len(g) == 4 and rd(g[1], 0, "frames_total") == "held known 10",
                     f"{[rd(x, 0, 'frames_total') for x in g]}")
        report.check(pairing, sec, "expected 2: record 4 after a gap of one record: frames_total unknown (gap), never "
                                   "10, never 0, no sample; temperature a sample",
                     len(g) == 4 and g[2]["follows"] == "gap" and g[2].get("lost") == 1
                     and g[2]["records"][0]["fields"]["frames_total"] == {"reading": "unknown", "why": "gap"}
                     and rd(g[2], 0, "temperature") == 'sample "21"',
                     f"{g[2]['follows'] if len(g) > 2 else None}: {[rd(x, 0, 'temperature') for x in g]}")
        p = [x for x in r.pairs(TOTALS) if x.field == "frames_total"]
        report.check(pairing, sec, "expected 3: R reads record 5's frames_total 18, a sample, and pairs it with "
                                   "record 1's 10: same_epoch, established in band, 2 in 1/s over 4 s, measured",
                     rd(g[3], 0, "frames_total") == "sample 18" and len(p) == 1
                     and (p[0].earlier, p[0].later) == ((0, 0), (3, 0)) and rated(p[0], 8, 4_000_000_000, "measured"),
                     "; ".join(pshort(x) for x in p))
    finally:
        bus.close()


# -- §5 ---------------------------------------------------------------------------------------

def section5(report: Report, pairing: str) -> None:
    sec = "§5 a 64-bit decrease across a face"
    owner_impl, reader_impl = pairing.split("-")
    bus = Bus(face=True)
    try:
        assert bus.face is not None
        r = make_reader(reader_impl, bus.r_session, bus.r_endpoint)
        bus.closers.append(r)
        o = make_owner(owner_impl, bus.endpoint)
        bus.closers.append(o)
        time.sleep(1.5)
        t = o.now_ms() + 200
        refused: list[Any] = []
        _steps(o, t, [
            (0, lambda at: o.record(TOTALS, {"frames_total": 500, "temperature": "21.5"}, at)),
            (500, lambda at: o.bare(BARE, 500)),
            (1000, lambda at: refused.append(o.record(TOTALS, {"frames_total": 480, "temperature": "21.5"}, at))),
            (1100, lambda at: o.raw(TOTALS, {"frames_total": 480, "temperature": "21.5"})),
            (1500, lambda at: o.bare_raw(BARE, 480)),
        ])
        w = bus.wire.wait(TOTALS, 2)
        report.check(pairing, sec, "setup: R, across the face, holds no instance token: none crosses",
                     not [k for k in bus.face.tokens if "/@zk/" in k], f"{bus.face.tokens[:3]}")
        report.check(pairing, sec, "expected 2: an owner that follows §2.19 never writes it: its runtime refuses "
                                   "480 within the run, and nothing of it reaches the wire",
                     len(refused) == 1 and "refused" in refused[0] and "decreased" in refused[0]["refused"]
                     and [x["value"]["records"][0]["values"].get("frames_total") for x in w if "value" in x]
                     == [500, 480] and w[1]["seq"] == 1 and w[0]["run"] == w[1]["run"],
                     f"{str(refused[0].get('refused', refused[0]))[:160] if refused else None}; wire "
                     f"{[(x.get('seq'), x.get('value', {}).get('records', [{}])[0].get('values')) for x in w]}")
        if reader_impl == "zenctl":
            g = r.frames(TOTALS, 2)
            report.check(pairing, sec, "as zenctl across the face shows it: frames_total known 500 then 480, the raw "
                                       "frame continuing the run",
                         len(g) == 2 and g[1].get("follows") == "continues"
                         and [zstate(x, 0, "frames_total") for x in g] == ["known 500", "known 480"],
                         f"{[(x.get('follows'), zstate(x, 0, 'frames_total')) for x in g]}")
            for step in ("expected 1: 500 → 480 decreased, established in band, the finding, with no presence",
                         "expected 3: the bare JSON counter's same decrease an assumed reset across the face"):
                report.not_judged(pairing, sec, step)
            return
        r.frames(TOTALS, 2)
        p = [x for x in r.pairs(TOTALS) if x.field == "frames_total"]
        report.check(pairing, sec, "expected 1: R pairs 500 → 480 as decreased, established in band: the finding, "
                                   "with no presence",
                     len(p) == 1 and (p[0].continuity, p[0].epoch, p[0].via, p[0].no_rate)
                     == ("decreased", "established", "in_band", "decreased")
                     and t2.series_answer("counter", 2, p) == "established", "; ".join(pshort(x) for x in p))
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline and len(r.s.v1_pairs.get(key_of(BARE), [])) < 1:
            time.sleep(0.05)
        bp = r.s.v1_pairs.get(key_of(BARE), [])
        report.check(pairing, sec, "expected 3: the same decrease of the bare JSON counter, no in-band epoch, reads "
                                   "as an assumed reset across the face (v1 §2.14)",
                     len(bp) == 1 and (bp[0].continuity, bp[0].epoch, bp[0].via) == ("reset", "assumed", None),
                     f"{[x.expect() for x in bp]}")
    finally:
        bus.close()


# -- §6 ---------------------------------------------------------------------------------------

def section6(report: Report, pairing: str) -> None:
    sec = "§6 a rewind under a restored run file"
    owner_impl, reader_impl = pairing.split("-")
    work = tempfile.mkdtemp(prefix="zk2py-t2-run-", dir="/srv/dev/scratch/zenkey/py")
    run_file = os.path.join(work, "run.json")
    bus = Bus()
    try:
        r = make_reader(reader_impl, bus.r_session, bus.endpoint)
        bus.closers.append(r)
        # "A copy of the file is kept before O starts": an instance run once
        # first writes the record in the owner's own form, so the copy is
        # of a record r is then taken from.
        o = make_owner(owner_impl, bus.endpoint, run_file=run_file)
        o.close()
        copy = os.path.join(work, "run.copy")
        shutil.copyfile(run_file, copy)
        o = make_owner(owner_impl, bus.endpoint, run_file=run_file)
        bus.closers.append(o)
        time.sleep(1.0)
        t = o.now_ms() + 200
        _steps(o, t, [(0, lambda at: o.record(TOTALS, {"frames_total": 1000, "temperature": "21.5"}, at)),
                      (1000, lambda at: o.record(TOTALS, {"frames_total": 1010, "temperature": "21.5"}, at))])
        bus.wire.wait(TOTALS, 2)
        i1 = o.instance()
        o.close()
        bus.closers.remove(o)
        shutil.copyfile(copy, run_file)
        o = make_owner(owner_impl, bus.endpoint, run_file=run_file)
        bus.closers.append(o)
        time.sleep(1.0)
        t = o.now_ms() + 200
        _steps(o, t, [(0, lambda at: o.record(TOTALS, {"frames_total": 4, "temperature": "21.5"}, at))])
        w = bus.wire.wait(TOTALS, 3)
        report.check(pairing, sec, "steps 1 to 3: I1 under run r at seqs 0 and 1; I2, a new instance, takes r again "
                                   "from the restored file and puts frames_total 4 at seq 0",
                     len(w) == 3 and w[0]["run"] == w[1]["run"] == w[2]["run"] and [x["seq"] for x in w] == [0, 1, 0]
                     and o.instance() != i1,
                     f"{[(x.get('run'), x.get('seq')) for x in w]}, instances {i1} {o.instance()}")
        g = r.frames(TOTALS, 3)
        if reader_impl == "zenctl":
            report.check(pairing, sec, "expected 1, as zenctl shows it: I2's first frame read as a rewind, its "
                                       "keyframe making each field known",
                         len(g) == 3 and g[2].get("follows") == "rewind"
                         and zstate(g[2], 0, "frames_total") == "known 4",
                         f"{[(x.get('follows'), zstate(x, 0, 'frames_total')) for x in g]}")
            report.not_judged(pairing, sec, "expected 2: 1010 → 4 reset, established in band, never decreased")
            return
        report.check(pairing, sec, "expected 1: R reads I2's first frame as a rewind (a seq of the run it holds, "
                                   "older than expected); that keyframe makes each field known again",
                     len(g) == 3 and g[2]["follows"] == "rewind"
                     and [rd(g[2], 0, n) for n in ("frames_total", "temperature")] == ["sample 4", 'sample "21.5"'],
                     f"{[(x['follows'], rd(x, 0, 'frames_total')) for x in g]}")
        p = [x for x in r.pairs(TOTALS) if x.field == "frames_total"]
        report.check(pairing, sec, "expected 2: R pairs 1010 → 4 as reset, established in band: never decreased, "
                                   "no finding",
                     len(p) == 2 and (p[1].continuity, p[1].epoch, p[1].via) == ("reset", "established", "in_band")
                     and t2.series_answer("counter", 3, p) != "established", "; ".join(pshort(x) for x in p))
    finally:
        bus.close()
        shutil.rmtree(work, ignore_errors=True)


# -- §7 ---------------------------------------------------------------------------------------

def section7(report: Report, pairing: str) -> None:
    sec = "§7 one counter, two series"
    owner_impl, reader_impl = pairing.split("-")
    bus = Bus()
    try:
        r = make_reader(reader_impl, bus.r_session, bus.endpoint)
        bus.closers.append(r)
        o = make_owner(owner_impl, bus.endpoint)
        bus.closers.append(o)
        time.sleep(1.0)
        tool = TelemetryTool(bus.endpoint, 5) if reader_impl == "zenctl" else None
        t = o.now_ms() + 200
        _steps(o, t, [(0, lambda at: (o.bare(BARE, 7), o.record(LINK, {"dropped": 7, **LINK_BASE}, at))),
                      (1000, lambda at: (o.bare(BARE, 9), o.record(LINK, {"dropped": 9, **LINK_BASE}, at)))])
        w = bus.wire.wait(LINK, 2)
        # expected 4: frames/link's record type is core §7.7's fixtures' Link.
        spec = REPO / "spec"
        lt = __import__("zk2py.packed", fromlist=["lint"]).lint(
            json.loads((spec / "conformance" / "contracts" / "schemas" / "packed" / "frames.json").read_text()))
        mine = layouts()[LINK]
        report.check(pairing, sec, "expected 4: frames/link's record type is core §7.7's fixtures' Link, field for "
                                   "field, so its bodies are frame/bodies.json's",
                     mine.record.layout_form() == lt.types["Link"].layout_form() and len(w) == 2,
                     f"{len(w)} frames")
        if reader_impl == "zenctl":
            g, b = r.frames(LINK, 2), r.bare(2)
            report.check(pairing, sec, "as zenctl shows them: two keys, the bare counter 7 then 9, and frames/link's "
                                       "dropped known 7 then 9",
                         b[-2:] == [7, 9] and [zstate(x, 0, "dropped") for x in g] == ["known 7", "known 9"],
                         f"bare {b}, frames {[zstate(x, 0, 'dropped') for x in g]}")
            assert tool is not None
            doc = tool.result()
            rows = [x for x in (doc or {}).get("rows", []) if x.get("row") == "resource"]
            members = [x for x in (doc or {}).get("rows", []) if x.get("row") == "member"]
            pairs = [q for m in members for q in m.get("pairs", [])]
            report.check(pairing, sec, "zenctl telemetry (telemetry.v1's reader) reads the bare counter of this "
                                       "telemetry.v2 contract by v1's rules, 7 → 9 same_epoch, and no frame resource "
                                       "as a metric resource (v2 §2.1 to §2.15 by reference; #983 T2 to come)",
                         [x.get("resource") for x in rows] == [f"stream/{BARE}"]
                         and [(q.get("earlier"), q.get("later"), q.get("continuity")) for q in pairs]
                         == [(7, 9, "same_epoch")],
                         f"resources {[x.get('resource') for x in rows]}; pairs "
                         f"{[(q.get('earlier'), q.get('later'), q.get('continuity'), q.get('epoch'), q.get('via')) for q in pairs]}; "
                         f"judgement {(doc or {}).get('judgement')}")
            report.note(pairing, sec, "zenctl telemetry reads presence, so its pair's epoch is established by "
                                      "presence; R reads none, and v1 §2.9 assumes it")
            for step in ("expected 1: two series, the frame's labelled \"sdu/dropped_total (frames/link)\"",
                         "expected 2: the frame's series has the mirror's kind and unit, counter in 1",
                         "expected 3: the bare 7 → 9 by v1, its epoch assumed; the frame's 7 → 9 established in band; "
                         "never one with the other"):
                report.not_judged(pairing, sec, step)
            return
        r.frames(LINK, 2)
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline and len(r.s.v1_pairs.get(key_of(BARE), [])) < 1:
            time.sleep(0.05)
        series = r.s.series()
        mine_s = [x for x in series if x[0] in (key_of(BARE), key_of(LINK)) and x[1] in (None, "dropped")]
        report.check(pairing, sec, "expected 1: R holds two series: the key …/sdu/dropped_total, and dropped of "
                                   "…/frames/link, labelled \"sdu/dropped_total (frames/link)\"",
                     sorted(mine_s, key=str) == sorted([(key_of(BARE), None, BARE),
                                                        (key_of(LINK), "dropped", "sdu/dropped_total (frames/link)")],
                                                       key=str), f"{mine_s}")
        s = r.s.readers[key_of(LINK)].decl.of("dropped")
        report.check(pairing, sec, "expected 2: the frame's series has the mirror's kind and unit, counter in 1",
                     s is not None and (s.kind, s.unit, s.mirror, s.width) == ("counter", "1", BARE, 32),
                     f"{None if s is None else s.expect()}")
        bp, fp = r.s.v1_pairs.get(key_of(BARE), []), [x for x in r.pairs(LINK) if x.field == "dropped"]
        report.check(pairing, sec, "expected 3: R pairs the bare counter 7 → 9 by v1's rules, its epoch assumed with "
                                   "no presence read; the frame's 7 → 9 established in band; never one with the other",
                     len(bp) == 1 and (bp[0].continuity, bp[0].epoch, bp[0].via) == ("same_epoch", "assumed", None)
                     and len(fp) == 1 and rated(fp[0], 2, 1_000_000_000, "measured"),
                     f"bare {[x.expect() for x in bp]}; frame {[pshort(x) for x in fp]}")
    finally:
        bus.close()


SECTIONS = {"1": section1, "2": section2, "3": section3, "4": section4, "5": section5, "6": section6, "7": section7}


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="zk2py.telemetry_v2_scenarios")
    ap.add_argument("--pairing", action="append", choices=PAIRINGS)
    ap.add_argument("--only", action="append", choices=sorted(SECTIONS))
    a = ap.parse_args(argv)
    report = Report()
    for pairing in a.pairing or PAIRINGS:
        if ("rust" in pairing and not (BLACKBOX / "owner").is_file()) or \
                ("zenctl" in pairing and not (BLACKBOX / "zenctl").is_file()):
            print(f"CANNOT [{pairing}]: a black box is missing under {BLACKBOX}", flush=True)
            return 2
        for n in a.only or sorted(SECTIONS):
            try:
                SECTIONS[n](report, pairing)
            except Exception as e:  # noqa: BLE001 - a section that cannot run is a failure, named
                report.check(pairing, f"§{n}", "the section ran", False, f"{type(e).__name__}: {e}")
    by: dict[tuple[str, str], list[bool | None]] = {}
    for p, s, _, ok, _ in report.rows:
        by.setdefault((p, s), []).append(ok)
    print("\nSUMMARY", flush=True)
    for (p, s), oks in by.items():
        judged = [x for x in oks if x is not None]
        print(f"  [{p}] {s}: {sum(judged)}/{len(judged)}" + (f", {oks.count(None)} not judged" if None in oks else ""),
              flush=True)
    failed = [r for r in report.rows if r[3] is False]
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())

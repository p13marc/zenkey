"""Core §7.6's packed records live, in both directions (#609, the mend after
round 94; #979 B2): ``python -m zk2py.packed_live [--pairing P]…``.

Since #1016 a Rust runtime writes and reads packed records, so the codec
both implementations hold can meet on a bus.

**The contract** is the Rust tests' ``zenkey/tests/contracts/packed/
packed.v1.toml`` (a contract file, which the barrier lets zk2py read): one
stream per type of ``vectors.json`` but ``Wide64``, two templated states and
an event. Beside it, two other revisions of ``packed.v1``: ``packed-wide``,
whose ``sensor`` is ``SensorWide`` (another tag), and ``packed-collide``,
whose ``sensor`` is ``SensorCollide`` (Sensor's tag, one bit longer).

**The pairings.**
- ``rust-py``: ``zenctl gen`` at ``lab/packed`` writes every resource;
  zk2py reads. Its consumer (:meth:`zk2py.owner.Owner.subscribe_role`
  through ``interop/packed/zk2py_packed_reader.v1.toml``'s role) and its tool
  (:mod:`zk2py.echo_tool` ``--contracts``) read each record; S4's GET
  (:func:`zk2py.live.get_state`) reads each state member. Every record is
  re-written by zk2py's writer from its reading, which must give the same
  bytes, and its ``time`` fields are read under E = 0, since the deployment
  states no epoch (core 0.76). Then zk2py puts on the ``sensor`` key a
  ``SensorWide`` record, a ``SensorCollide`` record and a ``Sensor`` record
  in no envelope: its consumer and its tool refuse them by ``tag``, by
  ``length`` and as having no envelope, naming each. A consumer holding the
  other revisions refuses the Rust owner's own ``sensor`` records likewise.
- ``py-rust``: zk2py's owner of ``packed.v1`` at ``lab/pyp`` writes; ``zenctl
  get state`` (S4), ``zenctl watch`` (the runtime's consumer) and ``zenctl
  echo`` (a tool) read zk2py's packed puts and show their fields; ``zenctl
  watch`` holding ``packed-wide`` or ``packed-collide`` delivers none of
  zk2py's ``sensor`` records.

Every listener is on a port the OS picked. Exit 0 when every check passes, 1
when any fails, 2 when a black box is missing.
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import threading
import time
from pathlib import Path
from typing import Any

from . import packed, zk2c
from .state_pairings import BLACKBOX, Report, Tap, _wait

REPO = Path(__file__).resolve().parents[3]
CONTRACTS = REPO / "zenkey" / "tests" / "contracts"
PACKED = CONTRACTS / "packed" / "packed.v1.toml"
WIDE = CONTRACTS / "packed-wide.v1.toml"
COLLIDE = CONTRACTS / "packed-collide.v1.toml"
READER = REPO / "impl" / "python" / "interop" / "packed" / "zk2py_packed_reader.v1.toml"
IFACE = "packed.v1"
PAIRINGS = ("rust-py", "py-rust")

#: The resources zk2py's consumer reads through its role, as its contract's
#: ``requires.src`` lists them, by their key below the interface.
CONSUMED = ("stream/sensor", "stream/status", "stream/clock", "stream/chain", "state/reading/a", "state/status/a")


def _held(path: Path) -> Any:
    from .contract import load_contract

    c = load_contract(path)
    if not c.valid:
        raise RuntimeError(f"{path}: {c.codes}")
    return c


def record_of(c: Any, rid: str) -> packed.Record:
    """The packed record ``rid`` (``stream/sensor``, ``state/reading/a``,
    ``events/mode_change``) declares in contract ``c``."""
    from . import templates

    token, _, rest = rid.partition("/")
    for r in c.canonical["resources"]:
        if r["token"] == token and templates.match(templates.parse_template(r["template"]),
                                                   rest.split("/")) is not None:
            rec = c.schemas.packed_record(r["type"])
            if rec is None:
                raise RuntimeError(f"{rid}: not a clean packed type")
            return rec
    raise KeyError(rid)


class Consumer:
    """zk2py's consumer: an owner of ``zk2py_packed_reader.v1`` whose role
    ``src`` is bound to the owner under test, each resource read through the
    packed record of the revision of ``packed.v1`` it holds."""

    def __init__(self, ep: str, provider: str, holds: Any, service: str,
                 rids: tuple[str, ...] = (*CONSUMED, "events/mode_change")) -> None:
        from .owner import Owner

        self.o = Owner("lab", service, [_held(READER)], connect=ep, bindings={"src": [provider]})
        self.o.start()
        self.got: list[tuple[str, dict[str, Any]]] = []
        self.lock = threading.Lock()
        for rid in rids:
            sel = f"{rid}/*" if rid.startswith("events/") else rid
            self.o.subscribe_role("src", IFACE, sel, self._take, record=record_of(holds, rid))

    def _take(self, key: str, value: Any) -> None:
        if value is None:
            return  # a delete: no record (Owner.subscribe_role)
        with self.lock:
            self.got.append((key, value))

    def of(self, suffix: str) -> list[dict[str, Any]]:
        with self.lock:
            return [v for k, v in self.got if k.endswith(suffix)]

    @property
    def refused(self) -> list[tuple[str, str]]:
        return list(self.o.undecodable)

    def close(self) -> None:
        self.o.close()


class EchoTool:
    """zk2py's tool, :mod:`zk2py.echo_tool`, holding one revision, as a
    process: its JSON document once its window ends."""

    def __init__(self, ep: str, selector: str, for_s: float, contracts: Path) -> None:
        env = dict(os.environ, PYTHONPATH=str(REPO / "impl" / "python"))
        self.p = subprocess.Popen([sys.executable, "-m", "zk2py.echo_tool", "-c", ep, "--for", str(for_s),
                                   "--contracts", str(contracts), selector],
                                  stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, env=env)
        self.for_s = for_s

    def listening(self, limit: float = 30.0) -> bool:
        """Wait for its "listening" line on standard error."""
        assert self.p.stderr is not None
        end = time.monotonic() + limit
        while time.monotonic() < end:
            line = self.p.stderr.readline()
            if not line:
                return False
            if "echo: listening" in line:
                return True
        return False

    def doc(self) -> dict[str, Any]:
        out, _ = self.p.communicate(timeout=self.for_s + 60)
        return json.loads(out) if out.strip() else {}


def wrong_records(holds: Any) -> list[tuple[str, bytes, str, str]]:
    """What zk2py puts on the ``sensor`` key itself: (what, payload,
    Encoding, the refusal a ``Sensor`` reader names)."""
    value = {"temperature": "21.5", "humidity": "43.5", "pressure": "1013.2", "battery": "3.70"}
    wide, collide = record_of(_held(WIDE), "stream/sensor"), record_of(_held(COLLIDE), "stream/sensor")
    sensor = record_of(holds, "stream/sensor")
    return [
        ("a SensorWide record (packed-wide's revision), sealed with its own tag",
         packed.seal(wide, value, 0xBAD0, 1), zk2c.ENCODING, "tag"),
        ("a SensorCollide record (packed-collide's revision), Sensor's tag and one bit longer",
         packed.seal(collide, value, 0xBAD0, 2), zk2c.ENCODING, "length"),
        ("a Sensor record in no envelope, under application/octet-stream",
         sensor.write(value), "application/octet-stream", "no_envelope"),
    ]


def _gen(ep: str, address: str, duration: float) -> subprocess.Popen:
    return subprocess.Popen([str(BLACKBOX / "zenctl"), "gen", "-c", ep, "--contracts", str(PACKED),
                             "--duration", str(duration), "--rate", "4", "--member", "reading/{unit}=a",
                             "--member", "status/{unit}=a", "--format", "ndjson", address, IFACE],
                            stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)


def rust_py(report: Report) -> None:
    """``zenctl gen`` writes, zk2py reads."""
    from . import live
    from .live_interop import _r1

    sec = "rust-py"
    holds = _held(PACKED)
    r, ep, _ = _r1(timestamping=False)
    session = live.open_client(ep)
    base = f"zk2/lab/packed/{IFACE}"
    tap = Tap(session, [f"{base}/**"])
    readers: list[Any] = []
    try:
        consumer = Consumer(ep, "lab/packed", holds, "reader")
        wide = Consumer(ep, "lab/packed", _held(WIDE), "reader-wide", ("stream/sensor",))
        collide = Consumer(ep, "lab/packed", _held(COLLIDE), "reader-collide", ("stream/sensor",))
        readers = [consumer, wide, collide]
        tool = EchoTool(ep, f"{base}/**", 9.0, PACKED)
        if not report.check(sec, "zk2py's echo tool is listening", tool.listening()):
            return
        gen = _gen(ep, "lab/packed", 8.0)
        # Up once the tap has heard the Rust owner's sensor and both states.
        up = _wait(lambda: all(tap.of(f"/{rid}") for rid in ("stream/sensor", "state/reading/a",
                                                               "state/status/a")), 30.0)
        if not report.check(sec, "zenctl gen is up: the tap heard its sensor stream and both states", up):
            gen.terminate()
            gen.communicate(timeout=30)
            return
        wrong = wrong_records(holds)
        for _, payload, enc, _ in wrong:
            session.put(f"{base}/stream/sensor", payload, encoding=enc)
            time.sleep(0.2)
        states = {m: live.get_state(session, f"{base}/state/{m}") for m in ("reading/a", "status/a")}
        out, err = gen.communicate(timeout=60)
        doc = tool.doc()
        report.check(sec, "zenctl gen ran its window and sent its samples", gen.returncode == 0,
                     f"exit {gen.returncode}: {err.strip()[-300:]}")

        # -- the wire: every Rust record read back by zk2py's codec, and
        # written again by zk2py's writer to the same bytes.
        wrong_payloads = {p for _, p, _, _ in wrong}
        rust_rows = [x for x in tap.got if x["payload"] not in wrong_payloads and not x["delete"]]
        per: dict[str, int] = {}
        bad: list[str] = []
        clock_off: list[int] = []
        for x in rust_rows:
            rid = x["key"][len(base) + 1:]
            rid = "events/mode_change" if rid.startswith("events/mode_change/") else rid
            rec = record_of(holds, rid)
            got = packed.read_sample(rec, x["read"])
            if got.undecodable:
                bad.append(f"{rid}: {got.says(rec.name)}")
                continue
            again = rec.write(got.value, packed.deployment_epoch(None))
            if again != x["read"].record or x["read"].tag != rec.tag:
                bad.append(f"{rid}: {x['read'].record.hex()} read as {got.value} writes {again.hex()}")
            per[rid] = per.get(rid, 0) + 1
            if rid == "stream/clock":
                clock_off.append(abs(got.value["at_ms"] - int(time.time() * 1000)))
        report.check(sec, "every record zenctl gen wrote, on all 14 streams, both states and the event, reads "
                          "through zk2py's layout under its CH tag, and zk2py's writer writes its reading back to "
                          "the same bytes",
                     not bad and len(per) == 17, f"{len(rust_rows)} records over {len(per)} resources; {bad[:3]}")
        report.check(sec, "core 0.76's epoch: the Rust writer's time fields read under E = 0, the deployment "
                          "stating none, are now (Unix seconds)",
                     bool(clock_off) and max(clock_off) < 120_000,
                     f"{len(clock_off)} clock records, at most {max(clock_off, default=None)} ms from the wall "
                     "clock")

        # -- zk2py's consumer, through its role.
        heard = {rid: len(consumer.of(rid)) for rid in CONSUMED}
        report.check(sec, "zk2py's consumer reads every stream and state member it binds, and the event, as "
                          "JSON readings",
                     all(heard.values()) and consumer.of("") != [] and any(
                         "/events/mode_change/" in k for k, _ in consumer.got), json.dumps(heard))
        mine = [(k[len(base) + 1:], t) for k, t in consumer.refused]
        report.check(sec, "zk2py's consumer refuses zk2py's three wrong records on the sensor key, by tag, by "
                          "length and as having no envelope, and nothing of the Rust owner's",
                     mine == [("stream/sensor", w[3]) for w in wrong], json.dumps(mine))
        for name, reader, token in (("packed-wide", wide, "tag"), ("packed-collide", collide, "length")):
            mine = [t for k, t in reader.refused if k.endswith("/stream/sensor")]
            rust_sensor = sum(1 for x in rust_rows if x["key"].endswith("/stream/sensor"))
            # Of zk2py's three wrong records, the one of this reader's own
            # revision is a record it reads; the other two it refuses.
            report.check(sec, f"a consumer holding {name} refuses every Rust sensor record by {token}, and "
                              "delivers only the one record of its own revision zk2py put",
                         rust_sensor > 0 and mine.count(token) >= rust_sensor and len(mine) == rust_sensor + 2
                         and len(reader.of("stream/sensor")) == 1,
                         f"{rust_sensor} Rust sensor records; refused {len(mine)}, {mine.count(token)} by {token}; "
                         f"delivered {reader.of('stream/sensor')}")

        # -- S4's GET.
        s4 = {}
        for m, reading in states.items():
            rec = record_of(holds, f"state/{m}")
            vals = []
            for rep in reading.replies:
                got = packed.read_sample(rec, zk2c.read_put(rep.key, rep.encoding, rep.raw))
                vals.append(got.value if not got.undecodable else got.says(rec.name))
            s4[m] = vals
        report.check(sec, "S4's GET of state/reading/a and state/status/a: one reply each, read through its "
                          "layout", all(len(v) == 1 and isinstance(v[0], dict) for v in s4.values()), json.dumps(s4))

        # -- zk2py's tool.
        samples = doc.get("samples", [])
        packed_rows = [s for s in samples if s.get("declared_type", {}).get("kind") == "packed"]
        undec = [s for s in packed_rows if s.get("undecodable")]
        decoded = [s for s in packed_rows if isinstance(s.get("decoded"), dict)]
        report.check(sec, "zk2py's echo tool decodes every Rust record it heard by its declared type, each named "
                          "packed:<name>, none sniffed",
                     len(undec) == 3 and len(decoded) == len(packed_rows) - 3 > 0
                     and all(s["declared_type"]["name"].startswith("packed:") and s.get("sniffed") is None
                             for s in packed_rows),
                     f"{len(packed_rows)} packed rows of {len(rust_rows)} Rust records and zk2py's 3, "
                     f"{len(decoded)} decoded, {len(undec)} undecodable")
        said = [s["undecodable"] for s in undec]
        want = [f"undecodable as packed:Sensor: {w[3]}" for w in wrong]
        report.check(sec, "zk2py's echo tool shows the three wrong records undecodable, the refusal named "
                          "(tag, length, no envelope), and never sniffs them",
                     len(said) == 3 and all(s.startswith(w) for s, w in zip(said, want))
                     and all(s.get("sniffed") is None for s in undec), json.dumps(said))
    finally:
        tap.close()
        for x in readers:
            x.close()
        live.close(session)
        live.close(r)


class PyOwner:
    """zk2py's owner of ``packed.v1`` at ``lab/pyp``, a client of ``ep``,
    putting the values below, and its streams every 0.25 s."""

    SENSOR = {"temperature": "21.5", "humidity": "43.5", "pressure": "1013.2", "battery": "3.70"}
    STATUS = {"duty_available": "55.5", "signal_bars": 3, "oper": "up", "admin": "down"}

    def __init__(self, ep: str) -> None:
        from .owner import Owner

        self.o = Owner("lab", "pyp", [_held(PACKED)], connect=ep)
        self.o.start()
        self.base = f"zk2/lab/pyp/{IFACE}"
        for m, v in (("reading/a", self.SENSOR), ("status/a", self.STATUS)):
            k = f"{self.base}/state/{m}"
            self.o.set_state(k, self.o.encode_packed(k, v))
        self.stop = threading.Event()
        self.thread = threading.Thread(target=self._pump, daemon=True)
        self.thread.start()

    def _pump(self) -> None:
        n = 0
        while not self.stop.is_set():
            now = int(time.time() * 1000)
            for rid, v in (("stream/sensor", self.SENSOR), ("stream/status", self.STATUS),
                           ("stream/clock", {"at": now, "at_ms": now, "valid": n % 2 == 0}),
                           ("stream/chain", {"id": 9, "b": n % 8})):
                k = f"{self.base}/{rid}"
                self.o.publish(k, self.o.encode_packed(k, v))
            if n % 4 == 0:
                k = f"{self.base}/events/mode_change"
                rec = record_of(self.o.contracts[0], "events/mode_change")
                self.o.occur(k, rec.write({"mode": ["off", "on", "auto"][n % 3], "wide": "y"}))
            n += 1
            self.stop.wait(0.25)

    def close(self) -> None:
        self.stop.set()
        self.thread.join(5)
        self.o.close()


def _said(p: subprocess.Popen, words: str, limit: float) -> bool:
    """Wait until a process's standard error holds a line with ``words``."""
    assert p.stderr is not None
    end = time.monotonic() + limit
    while time.monotonic() < end:
        line = p.stderr.readline()
        if not line:
            return False
        if words in line:
            return True
    return False


def _zenctl(*args: str, timeout: float = 60.0) -> subprocess.CompletedProcess:
    return subprocess.run([str(BLACKBOX / "zenctl"), *args], capture_output=True, text=True, timeout=timeout)


def _values(rows: list[dict[str, Any]]) -> list[Any]:
    return [((x.get("payload") or {}).get("value")) for x in rows if x.get("row") in ("sample", "key")]


def _expected(rec: packed.Record, value: dict[str, Any]) -> dict[str, Any]:
    """A value as every reader shows it: read back from its record."""
    return rec.read(rec.tag, rec.write(value, 0), 0)


def py_rust(report: Report) -> None:
    """zk2py's owner writes, the Rust readers read."""
    from . import live
    from .live_interop import _r1

    sec = "py-rust"
    holds = _held(PACKED)
    r, ep, _ = _r1(timestamping=False)
    owner = None
    try:
        owner = PyOwner(ep)
        sensor = _expected(record_of(holds, "stream/sensor"), PyOwner.SENSOR)
        status = _expected(record_of(holds, "stream/status"), PyOwner.STATUS)
        # Up once a tap on the router hears the owner's pump.
        probe = live.open_client(ep)
        tap = Tap(probe, [f"{owner.base}/stream/sensor"])
        try:
            up = _wait(lambda: bool(tap.of("/stream/sensor")), 30.0)
        finally:
            tap.close()
            live.close(probe)
        if not report.check(sec, "zk2py's owner is up: a tap heard its sensor stream", up):
            return
        for res, want in (("reading/{unit}", sensor), ("status/{unit}", status)):
            p = _zenctl("get", "state", "-c", ep, "--format", "json", "--contracts", str(PACKED), "lab/pyp", IFACE,
                        res)
            doc = json.loads(p.stdout) if p.stdout.strip().startswith("{") else {}
            vals = _values(doc.get("rows", []))
            types = [((x.get("payload") or {}).get("declared")) for x in doc.get("rows", [])]
            report.check(sec, f"zenctl get state {res}: S4's GET reads zk2py's packed put, its fields shown",
                         p.returncode == 0 and vals == [want], f"exit {p.returncode}; {types}; {vals}")
        for rid, want in (("stream/sensor", sensor), ("stream/status", status), ("stream/clock", None),
                          ("stream/chain", None), ("events/mode_change", None)):
            p = _zenctl("watch", "-c", ep, "--format", "ndjson", "--contracts", str(PACKED), "--for", "2",
                        "lab/pyp", IFACE, rid)
            rows = [json.loads(x) for x in p.stdout.splitlines() if x.startswith("{")]
            vals = _values(rows)
            summary = next((x for x in rows if x.get("row") == "summary"), {})
            ok = p.returncode == 0 and vals and all(isinstance(v, dict) for v in vals) and (
                want is None or all(v == want for v in vals)) and not summary.get("nonconforming")
            if rid == "stream/clock" and ok:
                now = int(time.time() * 1000)
                ok = all(abs(v["at_ms"] - now) < 60_000 for v in vals)
            report.check(sec, f"zenctl watch {rid}: the runtime's consumer reads each of zk2py's packed records, "
                              "its fields shown" + (" (time fields under E = 0, now)" if rid == "stream/clock"
                                                     else ""),
                         bool(ok), f"exit {p.returncode}; {len(vals)} samples; {vals[:1]}; "
                                   f"nonconforming {summary.get('nonconforming')}")
        echo = subprocess.Popen([str(BLACKBOX / "zenctl"), "echo", "-c", ep, "--format", "ndjson", "--contracts",
                                 str(PACKED), "--count", "12", f"zk2/lab/pyp/{IFACE}/**"],
                                stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        try:
            out, _ = echo.communicate(timeout=30)
        except subprocess.TimeoutExpired:
            echo.terminate()
            out, _ = echo.communicate(timeout=10)
        rows = [json.loads(x) for x in out.splitlines() if x.startswith("{")]
        typed = [x for x in rows if str(x.get("type", "")).startswith("packed:")]
        report.check(sec, "zenctl echo, a tool, reads zk2py's packed records by their declared type, each named "
                          "packed:<name>, its fields shown and valid",
                     len(typed) == len(rows) > 0 and all(x.get("typed") and isinstance(x.get("value"), dict)
                                                          and x.get("verdict") == "valid" for x in typed),
                     f"{len(rows)} rows; types {sorted({x.get('type') for x in rows})}")
        # zk2py puts three records the Ties layout refuses on stream/ties,
        # which its owner does not pump, while zenctl watch and zenctl echo
        # read that key: observer §7.3's honest rendering, "It MUST NOT
        # show garbage, and MUST NOT drop the sample silently" (F-436).
        ties_key = f"{owner.base}/stream/ties"
        ties = record_of(holds, "stream/ties")
        tv = {"half": "1", "signed": "0", "odd_top": "0"}
        refused = [("a Sensor record sealed with Sensor's tag", packed.seal(record_of(holds, "stream/sensor"),
                                                                            PyOwner.SENSOR, 0xBAD0, 1), zk2c.ENCODING),
                   ("a Ties record cut to one byte", zk2c.mutation(zk2c.InBand(16, 0xBAD0, 2), ties.write(tv)[:1],
                                                                    ties.tag), zk2c.ENCODING),
                   ("a Ties record in no envelope", ties.write(tv), "application/octet-stream")]
        # Each reader is waited for by a signal, not a sleep: zenctl watch
        # says nothing on stderr when it listens, so a publisher's matching
        # status on the key, which no one else subscribes, tells its
        # subscriber; zenctl echo then says "echoing" on stderr.
        session = live.open_client(ep)
        try:
            matcher = session.declare_publisher(ties_key)
            watch = subprocess.Popen([str(BLACKBOX / "zenctl"), "watch", "-c", ep, "--format", "ndjson",
                                      "--contracts", str(PACKED), "--for", "6", "lab/pyp", IFACE, "stream/ties"],
                                     stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
            watching = _wait(lambda: matcher.matching_status.matching, 30.0)
            echo = subprocess.Popen([str(BLACKBOX / "zenctl"), "echo", "-c", ep, "--format", "ndjson",
                                     "--contracts", str(PACKED), "--count", "3", ties_key],
                                    stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
            echoing = _said(echo, "echoing", 30.0)
            report.check(sec, "zenctl watch and zenctl echo are listening on stream/ties (a matching subscriber, "
                              "then echo's \"echoing\")", watching and echoing,
                         f"watch matched {watching}, echo said echoing {echoing}")
            for _, payload, enc in refused:
                session.put(ties_key, payload, encoding=enc)
                time.sleep(0.2)
            matcher.undeclare()
        finally:
            live.close(session)
        out, err = watch.communicate(timeout=60)
        rows = [json.loads(x) for x in out.splitlines() if x.startswith("{")]
        summary = next((x for x in rows if x.get("row") == "summary"), {})
        shown = [x for x in rows if x.get("row") != "summary"]
        counted = {k: v for k, v in summary.items() if isinstance(v, int) and v and k not in ("elapsed_s",)}
        report.check(sec, "F-436: zenctl watch, holding packed.v1, shows or counts each of three records the Ties "
                          "layout refuses (tag, length, no envelope) on stream/ties (observer §7.3: MUST NOT drop "
                          "the sample silently)",
                     len(shown) == 3 or sum(counted.values()) >= 3,
                     f"{len(shown)} rows; the summary's nonzero counts {counted}; exit {watch.returncode}; "
                     f"stderr {err.strip()[-200:]!r}")
        try:
            out, _ = echo.communicate(timeout=20)
        except subprocess.TimeoutExpired:
            echo.terminate()
            out, _ = echo.communicate(timeout=10)
        rows = [json.loads(x) for x in out.splitlines() if x.startswith("{")]
        said = [{k: v for k, v in x.items() if k not in ("identity", "key", "qos_axes")} for x in rows]
        named = [str(x.get("decode_error", "")) for x in rows]
        report.check(sec, "zenctl echo, holding packed.v1, shows each of the three records undecodable as "
                          "packed:Ties, the refusal named (tag, length, no envelope)",
                     len(rows) == 3 and all(x.get("verdict") == "undecodable" and x.get("type") == "packed:Ties"
                                            for x in rows)
                     and "tag" in named[0] and "length" in named[1] and "envelope" in named[2],
                     json.dumps(said)[:900])
        import re

        # Round 109: the black boxes of 67eac08c write the size as "<n B"
        # and name the declared type and the refusal beside it.
        sizes = [re.fullmatch(r"<(\d+) (?:B|bytes?)\b(.*)>", str(x.get("value"))) for x in rows]
        report.check(sec, "F-436: zenctl echo renders each undecodable record as its declared type and size, "
                          "never its bytes as text (observer §7.3: MUST NOT show garbage)",
                     len(rows) == 3 and all(sizes) and [int(m.group(1)) for m in sizes] == [5, 1, 2]
                     and all("packed:Ties" in m.group(2) for m in sizes),
                     json.dumps([x.get("value") for x in rows]))
        for name, path in (("packed-wide", WIDE), ("packed-collide", COLLIDE)):
            p = _zenctl("watch", "-c", ep, "--format", "ndjson", "--contracts", str(path), "--for", "2", "lab/pyp",
                        IFACE, "stream/sensor")
            rows = [json.loads(x) for x in p.stdout.splitlines() if x.startswith("{")]
            summary = next((x for x in rows if x.get("row") == "summary"), {})
            # Round 109: since F-436's fix the reference rows each record it
            # refuses, undecodable with its refusal named, which delivers
            # nothing.
            shown = [x for x in rows if x.get("row") in ("sample", "key")]
            delivered = [x for x in shown if (x.get("payload") or {}).get("as") != "undecodable"]
            report.check(sec, f"zenctl watch holding {name}: the runtime's consumer delivers none of zk2py's "
                              "Sensor records (core §7.6's reader), and rows each one undecodable, its refusal "
                              "named (observer §7.3, F-436's fix)",
                         not delivered and bool(shown)
                         and all((x.get("payload") or {}).get("reason") for x in shown),
                         f"exit {p.returncode}; {len(shown)} rows, {len(delivered)} delivered; summary "
                         f"{json.dumps(summary)[:300]}")
            echo = subprocess.Popen([str(BLACKBOX / "zenctl"), "echo", "-c", ep, "--format", "ndjson",
                                     "--contracts", str(path), "--count", "2",
                                     f"zk2/lab/pyp/{IFACE}/stream/sensor"],
                                    stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
            try:
                out, _ = echo.communicate(timeout=20)
            except subprocess.TimeoutExpired:
                echo.terminate()
                out, _ = echo.communicate(timeout=10)
            rows = [json.loads(x) for x in out.splitlines() if x.startswith("{")]
            report.note(sec, f"zenctl echo holding {name} on stream/sensor: "
                             + json.dumps([{k: x.get(k) for k in ("type", "typed", "verdict", "value", "reason",
                                                                  "undecodable") if k in x} for x in rows])[:600])
    finally:
        if owner is not None:
            owner.close()
        live.close(r)


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="python -m zk2py.packed_live", description=__doc__.split("\n")[0])
    ap.add_argument("--pairing", action="append", choices=PAIRINGS)
    args = ap.parse_args(argv)
    if not (BLACKBOX / "zenctl").is_file():
        print(f"packed_live: no zenctl under {BLACKBOX} (ZK2PY_BLACKBOX)", file=sys.stderr)
        return 2
    report = Report()
    for pairing in args.pairing or PAIRINGS:
        try:
            {"rust-py": rust_py, "py-rust": py_rust}[pairing](report)
        except Exception as e:  # noqa: BLE001 - a pairing that crashed is a failure, named
            import traceback

            traceback.print_exc()
            report.check(pairing, "the pairing ran", False, f"{type(e).__name__}: {e}")
    failed = [x for x in report.rows if not x[2]]
    print(f"{len(report.rows) - len(failed)} passed, {len(failed)} failed", flush=True)
    return 1 if failed else 0


if __name__ == "__main__":
    code = main()
    sys.stdout.flush()
    os._exit(code)

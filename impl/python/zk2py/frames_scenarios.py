"""``spec/scenarios/frames.md`` §1–§6, live (#609, round 97; core §7.7, 0.87;
observer §7.6, 0.21): ``python -m zk2py.frames_scenarios [--pairing P]…
[--only N]…``.

Written from ``frames.md`` and core §7.7 as their implementer reads them.
Each section runs on a bus of its own: R1, a zk2py router with timestamping
on, on a free loopback port; O at ``lab/modem``, a client of R1, under a run
no reader has seen; S, a reader of O's frame keys on another client,
subscribed before O's first frame unless the section says otherwise.

**The contract** is ``linkframes.v1``, written from ``frames.md``'s
"Common setup" into ``impl/python/interop/frames/linkframes.v1.toml``, its
record types ``Link`` and ``Sample`` of the conformance's ``frames.json``
copied beside it as ``linkframes.json``. Every owner and every reader is
given it.

**The pairings**, each an owner and a reader:
- ``py-py``: zk2py's owner (:meth:`zk2py.owner.Owner.record_frame`) and
  zk2py's consumer (:class:`zk2py.frame_live.FrameSubscriber`);
- ``rust-py``: the owner example, a black box (``--frames``, given zk2py's
  contract), and zk2py's consumer;
- ``py-zenctl``: zk2py's owner, and ``zenctl watch`` (one per key, given
  the contract) and ``zenctl get`` as the readers.

**The wire.** Beside S, the runner subscribes to every key under
``zk2/lab/**`` and reads each frame's body itself: what the owner put, each
record full or delta, the fields it includes, and that no mutation reached
the key. The owner's rules are checked there, the reader's on its readings.

**A lost frame** (``frames.md``): "O takes the frame's seqs and puts
nothing": :meth:`~zk2py.owner.Owner.lose_frame` for zk2py's owner, ``frame
lose`` for the owner example. **Time**: §3's instants are given to the
owner, ``at`` milliseconds after its start on its monotonic clock (the
owner example's ``at=<ms>``), so no section waits 65 s.

**§1 step 5**, "A reader holding nothing reads each frame … alone": zk2py's
reader reads each frame the wire captured with a reader of its own that
held nothing; for zenctl, the runner answers a GET on the frame key with
the captured frames, one reply each, and ``zenctl get`` reads each reply
alone (observer §7.6: "That is one reply of a GET").

Exit 0 when every check passes, 1 when any fails, 2 when a pairing cannot
run (a black box missing).
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

from . import frame, zk2c

REPO = Path(__file__).resolve().parents[3]
CONTRACT = REPO / "impl" / "python" / "interop" / "frames" / "linkframes.v1.toml"
BLACKBOX = Path(os.environ.get("ZK2PY_BLACKBOX", "/srv/dev/scratch/zenkey/bin/current"))
IFACE = "linkframes.v1"
LINK, SAMPLE = "frames/link", "frames/sample"
BASE = {"dropped": 7, "queue_depth": 12, "duty_available": 1, "duty_wait_ms": "1500"}
NAMES = ("dropped", "queue_depth", "duty_available", "duty_wait_ms")
PAIRINGS = ("py-py", "rust-py", "py-zenctl")
NS = 1_000_000_000


def key_of(res: str, service: str = "modem") -> str:
    return f"zk2/lab/{service}/{IFACE}/stream/{res}"


def _contract() -> Any:
    from .contract import load_contract

    c = load_contract(CONTRACT, check_file_name=True)
    assert c.valid and not c.diagnostics, c.codes
    return c


def _layouts() -> dict[str, frame.Layout]:
    c = _contract()
    return {r["template"]: frame.layout_of(c, r) for r in c.canonical["resources"] if r.get("frame")}


class Report:
    def __init__(self) -> None:
        self.rows: list[tuple[str, str, str, bool, str]] = []

    def check(self, pairing: str, section: str, name: str, ok: bool, detail: str = "") -> bool:
        self.rows.append((pairing, section, name, ok, detail))
        print(f"{'PASS' if ok else 'FAIL'} [{pairing}] [{section}] {name}" + (f": {detail}" if detail else ""),
              flush=True)
        return ok

    def note(self, pairing: str, section: str, text: str) -> None:
        print(f"NOTE [{pairing}] [{section}] {text}", flush=True)


# -- the owners -------------------------------------------------------------------------------

class PyOwner:
    """zk2py's owner of ``linkframes.v1`` at ``lab/modem``, a client of R1."""

    impl = "zk2py"

    def __init__(self, ep: str, service: str = "modem") -> None:
        from .owner import Owner

        self.service = service
        self.o = Owner("lab", service, [_contract()], connect=ep)
        self.o.start()
        self.t0 = time.monotonic_ns()

    def _put(self, p: dict[str, Any] | None) -> dict[str, Any] | None:
        return None if p is None else {"run": p["run"], "seq": p["seq"], "records": len(p["records"]["records"])}

    def record(self, res: str, values: dict[str, Any], at_ms: int | None = None) -> list[dict[str, Any]]:
        at = None if at_ms is None else self.t0 + at_ms * 1_000_000
        return [self._put(p) for p in self.o.record_frame(key_of(res, self.service), values, at)]

    def flush(self, res: str) -> dict[str, Any] | None:
        return self._put(self.o.end_frame(key_of(res, self.service)))

    def lose(self, res: str) -> None:
        self.o.lose_frame(key_of(res, self.service))

    def mutation(self, res: str) -> tuple[bool, str]:
        from .owner import OwnerRefused

        try:
            self.o.publish(key_of(res, self.service), b"\x00")
        except OwnerRefused as e:
            return True, str(e)
        return False, "published"

    def new_run(self) -> str:
        self.o.new_epoch()
        return self.o.run_hex()

    def close(self) -> None:
        self.o.close()


class RustOwner:
    """The owner example, a black box, with ``--frames`` and zk2py's
    ``linkframes.v1`` ("unless a contract file given is it", its usage)."""

    impl = "rust"

    def __init__(self, ep: str, service: str = "modem") -> None:
        from .live_interop import Owner as Exe

        self.p = Exe(BLACKBOX / "owner", f"lab/{service}", [CONTRACT], connect=ep, flags=["--frames"])
        if self.p.wait_for("ready ", 120) is None:
            raise RuntimeError(f"the owner example did not start: {self.p.stderr[-3:]}")

    def command(self, line: str) -> dict[str, Any]:
        got = self.p.command(line)
        if got is None or got.startswith("refused"):
            raise RuntimeError(f"{line!r}: {got!r}")
        return json.loads(got)

    def record(self, res: str, values: dict[str, Any], at_ms: int | None = None) -> list[dict[str, Any]]:
        got = self.command(f"frame record {res} {json.dumps(values)}" + ("" if at_ms is None else f" at={at_ms}"))
        return [] if got.get("put") is None else [got["put"]]

    def flush(self, res: str) -> dict[str, Any] | None:
        return self.command(f"frame flush {res}").get("put")

    def lose(self, res: str) -> None:
        self.command(f"frame lose {res}")

    def mutation(self, res: str) -> tuple[bool, str]:
        got = self.command(f"frame mutation {res}")
        return "refused" in got, got.get("refused", json.dumps(got))

    def new_run(self) -> str:
        return self.command("frame new-epoch")["run"]

    def close(self) -> None:
        self.p.close()


def make_owner(impl: str, ep: str, **kw: Any) -> Any:
    return RustOwner(ep, **kw) if impl == "rust" else PyOwner(ep, **kw)


# -- the readers ------------------------------------------------------------------------------

class PyReader:
    """zk2py's consumer, :class:`zk2py.frame_live.FrameSubscriber`."""

    impl = "zk2py"

    def __init__(self, session: Any, ep: str, addresses: list[str]) -> None:
        from .frame_live import FrameSubscriber

        self.s = FrameSubscriber(session, addresses, _contract())

    def wait(self, key: str, n: int, timeout: float = 5.0) -> list[dict[str, Any]]:
        return [g for g in self.s.wait(key, n, timeout) if "undecodable" not in g]

    def undecodable(self) -> list[str]:
        return [g["undecodable"] for _, _, g in self.s.heard if "undecodable" in g]

    def close(self) -> None:
        self.s.close()


class _Watch:
    """``zenctl watch <address> linkframes.v1 <resource>`` in ndjson, each
    sample row's ``payload.value``, a frame as zenctl reads it."""

    def __init__(self, ep: str, address: str, res: str, for_s: float) -> None:
        self.proc = subprocess.Popen([str(BLACKBOX / "zenctl"), "watch", address, IFACE, res, "--for", f"{for_s:g}",
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

    def samples(self) -> list[dict[str, Any]]:
        with self.lock:
            return [r for r in self.rows if r.get("row") == "sample"]

    def close(self) -> None:
        if self.proc.poll() is None:
            self.proc.terminate()
            try:
                self.proc.wait(timeout=10)
            except subprocess.TimeoutExpired:
                self.proc.kill()
                self.proc.wait()


class ZReader:
    """``zenctl watch``, one per frame key of each address, given the
    contract."""

    impl = "zenctl"

    def __init__(self, session: Any, ep: str, addresses: list[str], for_s: float = 40.0) -> None:
        self.watches = {key_of(res, a.split("/")[1]): _Watch(ep, a, res, for_s)
                        for a in addresses for res in (LINK, SAMPLE)}
        time.sleep(2.5)  # each watch's subscriber declared before O's first frame

    def wait(self, key: str, n: int, timeout: float = 5.0) -> list[dict[str, Any]]:
        w = self.watches[key]
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline and len(w.samples()) < n:
            time.sleep(0.05)
        return [r["payload"]["value"] for r in w.samples()
                if isinstance(r.get("payload"), dict) and isinstance(r["payload"].get("value"), dict)
                and "records" in r["payload"]["value"]]

    def raw(self, key: str) -> list[dict[str, Any]]:
        return self.watches[key].samples()

    def undecodable(self) -> list[str]:
        out = []
        for w in self.watches.values():
            for r in w.samples():
                p = r.get("payload")
                if not (isinstance(p, dict) and isinstance(p.get("value"), dict) and "records" in p["value"]):
                    out.append(json.dumps(p)[:200])
        return out

    def close(self) -> None:
        for w in self.watches.values():
            w.close()


def make_reader(impl: str, session: Any, ep: str, addresses: list[str]) -> Any:
    return ZReader(session, ep, addresses) if impl == "zenctl" else PyReader(session, ep, addresses)


class Wire:
    """Every put and delete under ``zk2/lab/**``: (key, kind, payload,
    encoding), each frame key's frames read here with the layout."""

    def __init__(self, session: Any) -> None:
        import zenoh

        self.lock = threading.Lock()
        self.got: list[tuple[str, str, bytes, str]] = []
        self.sub = session.declare_subscriber("zk2/lab/**", zenoh.handlers.Callback(self._heard))
        self.layouts = _layouts()

    def _heard(self, sample: Any) -> None:
        import zenoh

        kind = "delete" if sample.kind == zenoh.SampleKind.DELETE else "put"
        with self.lock:
            self.got.append((str(sample.key_expr), kind, sample.payload.to_bytes(), str(sample.encoding)))

    def on(self, key: str) -> list[tuple[str, bytes, str]]:
        with self.lock:
            return [(k, p, e) for kk, k, p, e in self.got if kk == key]

    def frames(self, key: str) -> list[dict[str, Any]]:
        """Each put on ``key`` as the wire carries it: its form, run, seq,
        and a frame's value read with the layout (``value``), or why not."""
        lo = self.layouts[key.split("/stream/", 1)[1]]
        out = []
        for kind, payload, enc in self.on(key):
            r = zk2c.read_put(key, enc, payload) if kind == "put" else None
            row: dict[str, Any] = {"kind": kind, "form": None if r is None else r.form,
                                   "bytes": len(payload), "payload": payload}
            if r is not None and r.stamp is not None:
                row.update(run=r.stamp.run_hex(), seq=r.stamp.seq)
            if r is not None and r.form == "frame":
                try:
                    row["value"] = lo.read(r.tag, r.record or b"")
                except (frame.Refused, Exception) as e:  # noqa: BLE001 - named
                    row["refused"] = str(e)
            out.append(row)
        return out

    def wait(self, key: str, n: int, timeout: float = 5.0) -> list[dict[str, Any]]:
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline and len(self.on(key)) < n:
            time.sleep(0.02)
        return self.frames(key)

    def close(self) -> None:
        try:
            self.sub.undeclare()
        except Exception:  # noqa: BLE001
            pass


class Bus:
    def __init__(self) -> None:
        from . import live
        from .live_interop import _r1

        self.router, self.endpoint, _ = _r1()
        self.sessions = [live.open_client(self.endpoint) for _ in range(3)]
        self.s_session, self.w_session, self.x_session = self.sessions
        self.wire = Wire(self.w_session)
        self.closers: list[Any] = [self.wire]

    def close(self) -> None:
        for x in reversed(self.closers):
            try:
                x.close()
            except Exception:  # noqa: BLE001 - closing anyway
                pass
        for s in self.sessions:
            s.close()
        self.router.close()


class Echo:
    """zk2py's echo tool (:mod:`zk2py.echo_tool`), the observer, on
    ``zk2/lab/**`` for ``for_s`` seconds: given the contract it reads each
    frame key's frames through one reader (observer §7.6); without it, it
    shows each frame by its envelope, its body never sniffed."""

    def __init__(self, ep: str, for_s: float, held: bool = True) -> None:
        env = dict(os.environ, PYTHONPATH=str(REPO / "impl" / "python"))
        self.proc = subprocess.Popen([sys.executable, "-m", "zk2py.echo_tool", "-c", ep, "--for", f"{for_s:g}",
                                      *(["--contracts", str(CONTRACT)] if held else []), "zk2/lab/**"],
                                     stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, env=env)
        time.sleep(2.0)

    def rows(self, key: str, timeout: float = 60.0) -> list[dict[str, Any]]:
        try:
            out, _ = self.proc.communicate(timeout=timeout)
        except subprocess.TimeoutExpired:
            self.proc.kill()
            out, _ = self.proc.communicate()
        try:
            return [r for r in json.loads(out)["samples"] if r["key"] == key]
        except (ValueError, KeyError):
            return []

    def close(self) -> None:
        if self.proc.poll() is None:
            self.proc.kill()
            self.proc.communicate()


# -- reading the readings ---------------------------------------------------------------------

def states(rec: dict[str, Any]) -> dict[str, str]:
    """Each field's state, with its value or why: ``known 12``, ``within 12
    ±4``, ``unknown gap``, ``absent``."""
    out = {}
    for n, f in rec["fields"].items():
        st = f.get("state")
        if st == "known":
            out[n] = f"known {json.dumps(f.get('value'))}"
        elif st == "within":
            out[n] = f"within {json.dumps(f.get('value'))} ±{f.get('delta')}"
        elif st == "unknown":
            out[n] = f"unknown {f.get('why')}"
        else:
            out[n] = str(st)
    return out


def no_value_shown(readings: list[dict[str, Any]]) -> bool:
    """core §7.7 "Never a value": no ``unknown`` or ``absent`` field shows a
    value, its last value or 0 among them."""
    return all("value" not in f for g in readings for r in g["records"] for f in r["fields"].values()
               if f.get("state") in ("unknown", "absent"))


def by_seq(readings: list[dict[str, Any]]) -> dict[tuple[str, int], tuple[dict[str, Any], dict[str, Any]]]:
    """Each record read, by (run, seq): (the frame reading, the record)."""
    return {(g["run"], r["seq"]): (g, r) for g in readings for r in g["records"]}


def included(v: dict[str, Any], i: int = 0) -> list[str]:
    r = v["records"][i]
    return sorted([*r.get("values", {}), *r.get("absent", [])])


def _q(base: dict[str, Any], **kw: Any) -> dict[str, Any]:
    return {**base, **kw}


# -- §1 ---------------------------------------------------------------------------------------

def _get_alone(bus: Bus, key: str, payloads: list[bytes]) -> tuple[list[dict[str, Any]], str]:
    """``zenctl get`` of ``key``, answered by the runner with each captured
    frame, one reply each: each reply as zenctl reads it."""
    import zenoh

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


def section1(report: Report, pairing: str) -> None:
    sec = "§1 each frame decodes alone"
    owner_impl, reader_impl = pairing.split("-")
    bus = Bus()
    try:
        s = make_reader(reader_impl, bus.s_session, bus.endpoint, ["lab/modem"])
        bus.closers.append(s)
        o = make_owner(owner_impl, bus.endpoint)
        bus.closers.append(o)
        time.sleep(1.0)
        lk, sk = key_of(LINK), key_of(SAMPLE)
        # step 1
        o.record(LINK, BASE)
        o.record(LINK, _q(BASE, queue_depth=30, duty_wait_ms="2500"))
        w = bus.wire.wait(lk, 2)
        got = s.wait(lk, 2)
        report.check(pairing, sec, "step 1: two frames of one record each, at seqs 0 and 1 of one run",
                     [x.get("seq") for x in w] == [0, 1] and len({x.get("run") for x in w}) == 1
                     and all(len(x.get("value", {}).get("records", [])) == 1 for x in w),
                     f"{[(x.get('run'), x.get('seq'), x.get('form')) for x in w]}")
        report.check(pairing, sec, "step 1: the first is a keyframe, one full record; the second a delta record "
                                   "that includes queue_depth and duty_wait_ms alone",
                     len(w) == 2 and w[0]["value"]["records"][0]["full"] and not w[1]["value"]["records"][0]["full"]
                     and included(w[1]["value"]) == ["duty_wait_ms", "queue_depth"],
                     f"{[(x['value']['records'][0]['full'], included(x['value'])) for x in w if 'value' in x]}")
        ok = len(got) >= 2 and got[1]["follows"] == "continues" and states(got[1]["records"][0]) == {
            "dropped": "known 7", "queue_depth": "known 30", "duty_available": "known 1",
            "duty_wait_ms": 'known "2500"'}
        report.check(pairing, sec, "step 1: S reads the second as continuing the key, every field known: queue_depth "
                                   "30, duty_wait_ms 2500, the other two as the first frame held them", ok,
                     f"{[(g['follows'], states(g['records'][0])) for g in got[:2]]}")
        # step 2
        for k in range(1, 15):
            o.record(LINK, _q(BASE, queue_depth=30 + 10 * k, duty_wait_ms="2500"))
        w = bus.wire.wait(lk, 16)
        fulls = {x["seq"]: x["value"]["records"][0]["full"] for x in w if "value" in x}
        report.check(pairing, sec, "step 2: seqs 2 to 15; 2 to 14 delta records, 15 full, 15 records after the "
                                   "last full one",
                     sorted(fulls) == list(range(16)) and not any(fulls[q] for q in range(2, 15)) and fulls[15],
                     f"full at {[q for q, f in sorted(fulls.items()) if f]}")
        got = s.wait(lk, 16)
        report.check(pairing, sec, "step 2: S reads seqs 2 to 15 continuing the key, every field known",
                     [g["follows"] for g in got[2:16]] == ["continues"] * 14
                     and all(f["state"] == "known" for g in got[2:16] for f in g["records"][0]["fields"].values()),
                     f"{len(got)} frames read")
        # step 3
        samples = [{"temperature": "21.5", "rssi": -80, "link_up": True, "mode": "rx"},
                   {"temperature": "21.5", "rssi": -82, "link_up": True, "mode": "rx"},
                   {"temperature": "22", "rssi": -82, "link_up": True, "mode": "tx"}]
        puts = []
        for i, v in enumerate(samples):
            puts += o.record(SAMPLE, v, at_ms=10_000 * i)
        w = bus.wire.wait(sk, 1)
        v0 = w[0]["value"] if w and "value" in w[0] else None
        report.check(pairing, sec, "step 3: one frame of three records at seq 0 of frames/sample: record 0 full, "
                                   "records 1 and 2 delta records, both scheduled",
                     v0 is not None and w[0]["seq"] == 0 and len(v0["records"]) == 3 and v0["records"][0]["full"]
                     and not v0["records"][1]["full"] and not v0["records"][2]["full"]
                     and v0["records"][1].get("at") == "scheduled" and v0["records"][2].get("at") == "scheduled",
                     f"{None if v0 is None else [(r['full'], r.get('at')) for r in v0['records']]}")
        report.check(pairing, sec, "step 3: record 1 includes rssi alone, record 2 temperature and mode",
                     v0 is not None and included(v0, 1) == ["rssi"] and included(v0, 2) == ["mode", "temperature"],
                     f"{None if v0 is None else [included(v0, i) for i in (1, 2)]}")
        puts += o.record(SAMPLE, samples[2], at_ms=40_000)
        nxt = o.flush(SAMPLE)
        report.check(pairing, sec, "step 3: the key's next frame takes seq 3",
                     nxt is not None and nxt["seq"] == 3, f"{nxt}")
        gs = s.wait(sk, 2)
        report.check(pairing, sec, "step 3: S reads the frame's three records, every field known",
                     bool(gs) and len(gs[0]["records"]) == 3
                     and all(f["state"] == "known" for r in gs[0]["records"] for f in r["fields"].values()),
                     f"{[states(r) for r in gs[0]['records']] if gs else None}")
        # step 4
        before = len(bus.wire.on(lk))
        refused, why = o.mutation(LINK)
        time.sleep(0.5)
        after = bus.wire.frames(lk)[before:]
        report.check(pairing, sec, "step 4: a writer of mutations on frames/link is refused, and no mutation is "
                                   "put on the key", refused and not after and not s.undecodable(),
                     f"{why[:120]}; {len(after)} put after; undecodable {s.undecodable()[:2]}")
        # step 5
        caps = bus.wire.frames(lk)[:2] + bus.wire.frames(sk)[:1]
        if reader_impl == "py":
            lay = _layouts()
            alone = []
            for x, res in zip(caps, (LINK, LINK, SAMPLE)):
                r = frame.Reader(lay[res])
                alone.append(r.read_put(zk2c.read_put(key_of(res), zk2c.ENCODING, x["payload"])))
            how = "zk2py's reader, each frame with a reader that held nothing"
        else:
            a1, h1 = _get_alone(bus, lk, [x["payload"] for x in caps[:2]])
            a2, h2 = _get_alone(bus, sk, [x["payload"] for x in caps[2:]])
            a1 = sorted(a1, key=lambda g: g["seq"])
            alone, how = a1 + a2, f"zenctl get, one reply each ({h1}, {h2})"
        st = [[states(r) for r in g["records"]] for g in alone]
        want1 = {n: "known" for n in NAMES}
        ok = len(alone) == 3 and all(g["follows"] == "first" for g in alone) \
            and {n: v.split(" ")[0] for n, v in st[0][0].items()} == want1 \
            and st[1][0] == {"dropped": "unknown first", "queue_depth": "known 30", "duty_available": "unknown first",
                             "duty_wait_ms": 'known "2500"'} \
            and all(v.startswith("known") for r in st[2] for v in r.values()) and len(st[2]) == 3
        report.check(pairing, sec, "step 5: read alone, step 1's first frame every field known, its second "
                                   "queue_depth and duty_wait_ms known and the other two unknown, why first; step "
                                   "3's frame every field of its three records known", ok, f"{how}: {st}")
        report.check(pairing, sec, "never a value: no unknown or absent field shows one",
                     no_value_shown(s.wait(lk, 16) + s.wait(sk, 2) + alone))
    finally:
        bus.close()


# -- §2 ---------------------------------------------------------------------------------------

def section2(report: Report, pairing: str) -> None:
    sec = "§2 a lost delta frame"
    owner_impl, reader_impl = pairing.split("-")
    bus = Bus()
    try:
        s = make_reader(reader_impl, bus.s_session, bus.endpoint, ["lab/modem"])
        bus.closers.append(s)
        o = make_owner(owner_impl, bus.endpoint)
        bus.closers.append(o)
        time.sleep(1.0)
        lk = key_of(LINK)
        o.record(LINK, BASE)
        o.record(LINK, _q(BASE, queue_depth=30))
        got = s.wait(lk, 2)
        report.check(pairing, sec, "step 1: S reads seqs 0 and 1, every field known",
                     [g["seq"] for g in got] == [0, 1]
                     and all(f["state"] == "known" for g in got for f in g["records"][0]["fields"].values()),
                     f"{[(g['seq'], states(g['records'][0])) for g in got]}")
        o.lose(LINK)
        o.record(LINK, _q(BASE, queue_depth=30, duty_wait_ms="2500"))
        time.sleep(0.6)
        report.check(pairing, sec, "step 2: nothing reaches S", len(s.wait(lk, 3, 0.3)) == 2
                     and len(bus.wire.on(lk)) == 2, f"{len(bus.wire.on(lk))} on the wire")
        o.record(LINK, _q(BASE, queue_depth=40, duty_wait_ms="2500"))
        got = s.wait(lk, 3)
        g = got[2] if len(got) > 2 else None
        report.check(pairing, sec, "step 3: S reads seq 3 as following a gap of one record; queue_depth known, "
                                   "40; dropped, duty_available and duty_wait_ms unknown, why gap",
                     g is not None and g["seq"] == 3 and g["follows"] == "gap" and g.get("lost") == 1
                     and states(g["records"][0]) == {"dropped": "unknown gap", "queue_depth": "known 40",
                                                     "duty_available": "unknown gap", "duty_wait_ms": "unknown gap"},
                     f"{None if g is None else (g['seq'], g['follows'], g.get('lost'), states(g['records'][0]))}")
        for k in range(1, 13):
            o.record(LINK, _q(BASE, queue_depth=40 + 10 * k, duty_wait_ms="2500"))
        got = s.wait(lk, 15)
        seqs = {g["seq"]: states(g["records"][0]) for g in got}
        stay = all(seqs.get(q, {}).get(n) == "unknown gap" for q in range(4, 15)
                   for n in ("dropped", "duty_available", "duty_wait_ms"))
        healed = seqs.get(15) == {"dropped": "known 7", "queue_depth": "known 160", "duty_available": "known 1",
                                  "duty_wait_ms": 'known "2500"'}
        report.check(pairing, sec, "step 4: the three stay unknown until seq 15, the next full record; from there "
                                   "every field known, duty_wait_ms 2500", stay and healed,
                     f"seq 14 {seqs.get(14)}; seq 15 {seqs.get(15)}")
        w = bus.wire.frames(lk)
        report.check(pairing, sec, "the wire: seq 15 is the next full record, and the lost frame's seq 2 never put",
                     [x["seq"] for x in w if x.get("value", {}).get("records", [{}])[0].get("full")] == [0, 15]
                     and 2 not in [x.get("seq") for x in w], f"{[(x.get('seq')) for x in w]}")
        report.check(pairing, sec, "never a value: S shows and uses no value for an unknown field, neither the last "
                                   "it read nor 0", no_value_shown(got))
    finally:
        bus.close()


# -- §3 ---------------------------------------------------------------------------------------

def section3(report: Report, pairing: str) -> None:
    sec = "§3 scheduled and measured time"
    owner_impl, reader_impl = pairing.split("-")
    bus = Bus()
    try:
        s = make_reader(reader_impl, bus.s_session, bus.endpoint, ["lab/modem"])
        bus.closers.append(s)
        echo = Echo(bus.endpoint, 12) if reader_impl == "py" else None
        if echo is not None:
            bus.closers.append(echo)
        o = make_owner(owner_impl, bus.endpoint)
        bus.closers.append(o)
        time.sleep(1.0)
        sk = key_of(SAMPLE)
        v = {"temperature": "21.5", "rssi": -80, "link_up": True, "mode": "rx"}
        t = 0
        for at in (t, t + 10_200, t + 20_700):
            o.record(SAMPLE, v, at_ms=at)
        got = s.wait(sk, 1)
        g = got[0] if got else None
        recs = [] if g is None else g["records"]
        report.check(pairing, sec, "step 1: one frame of three records at seqs 0 to 2",
                     g is not None and g["seq"] == 0 and [r["seq"] for r in recs] == [0, 1, 2],
                     f"{None if g is None else [r['seq'] for r in recs]}")
        at0 = recs[0].get("at") if recs else None
        report.check(pairing, sec, "step 1: record 0's time is O's wall clock at t, from the deployment epoch "
                                   "(within 120 s of the runner's)",
                     recs != [] and recs[0].get("time") == "measured" and at0 is not None
                     and abs(at0["s"] - time.time()) < 120, f"{recs[0] if recs else None}")
        r1 = recs[1] if len(recs) > 1 else {}
        r2 = recs[2] if len(recs) > 2 else {}
        t0 = at0["s"] * 1000 + at0["ms"] if at0 else 0
        at1 = r1.get("at") or {"s": 0, "ms": 0}
        report.check(pairing, sec, "step 1: S shows record 1 at t + 10 s, scheduled, ±500 ms, never as measured "
                                   "or at t + 10.2 s",
                     r1.get("time") == "scheduled" and r1.get("after_ms") == 10_000 and r1.get("jitter_ms") == 500
                     and at1["s"] * 1000 + at1["ms"] - t0 == 10_000, f"{ {k: r1.get(k) for k in ('time', 'after_ms', 'jitter_ms', 'at')} }")
        at2 = r2.get("at") or {"s": 0, "ms": 0}
        report.check(pairing, sec, "step 1: S shows record 2 at t + 20.7 s, measured, its offset 20,700 ms",
                     r2.get("time") == "measured" and r2.get("after_ms") == 20_700 and "jitter_ms" not in r2
                     and at2["s"] * 1000 + at2["ms"] - t0 == 20_700,
                     f"{ {k: r2.get(k) for k in ('time', 'after_ms', 'jitter_ms', 'at')} }")
        T = t + 30_100
        o.record(SAMPLE, v, at_ms=T)
        o.record(SAMPLE, v, at_ms=T + 9_600)
        o.record(SAMPLE, v, at_ms=T + 35_000)
        got = s.wait(sk, 2)
        g = got[1] if len(got) > 1 else None
        report.check(pairing, sec, "step 2: the record at T + 35 s lies past the span and ends the frame: two "
                                   "records at seqs 3 and 4, record 1 scheduled",
                     g is not None and [r["seq"] for r in g["records"]] == [3, 4]
                     and g["records"][1].get("time") == "scheduled",
                     f"{None if g is None else [(r['seq'], r.get('time'), r.get('after_ms')) for r in g['records']]}")
        o.flush(SAMPLE)
        got = s.wait(sk, 3)
        g = got[2] if len(got) > 2 else None
        report.check(pairing, sec, "step 3: the record at T + 35 s opened the next frame as its record 0, and "
                                   "ending it puts it, the one record at seq 5",
                     g is not None and [r["seq"] for r in g["records"]] == [5] and g["records"][0]["time"] == "measured",
                     f"{None if g is None else [(r['seq'], r.get('time')) for r in g['records']]}")
        w = bus.wire.frames(sk)
        report.check(pairing, sec, "the wire: the owner marked scheduled only within 500 ms of the instant, and "
                                   "wrote record 2's offset", len(w) == 3 and "value" in w[0]
                     and [r.get("at") for r in w[0]["value"]["records"][1:]] == ["scheduled", 20_700]
                     and [r.get("at") for r in w[1]["value"]["records"][1:]] == ["scheduled"],
                     f"{[[r.get('at') for r in x['value']['records']] for x in w if 'value' in x]}")
        if echo is not None:
            rows = echo.rows(sk)
            fr0 = rows[0].get("frame") if rows else None
            r1 = fr0["records"][1] if fr0 and len(fr0.get("records", [])) > 1 else {}
            report.check(pairing, sec, "observer §7.6: zk2py's echo tool shows record 1's time as scheduled, with "
                                       "its jitter_ms, never as measured, and sniffs no frame",
                         r1.get("time") == "scheduled" and r1.get("jitter_ms") == 500 and r1.get("after_ms") == 10_000
                         and all(r.get("sniffed") is None for r in rows) and len(rows) == 3,
                         f"{len(rows)} rows; record 1 {r1}")
    finally:
        bus.close()


# -- §4 ---------------------------------------------------------------------------------------

def section4(report: Report, pairing: str) -> None:
    from .frame_live import RawFrames

    sec = "§4 the deadband"
    owner_impl, reader_impl = pairing.split("-")
    bus = Bus()
    try:
        s = make_reader(reader_impl, bus.s_session, bus.endpoint, ["lab/modem", "lab/rogue"])
        bus.closers.append(s)
        echo = Echo(bus.endpoint, 12) if reader_impl == "py" else None
        bare = Echo(bus.endpoint, 12, held=False) if reader_impl == "py" else None
        bus.closers += [e for e in (echo, bare) if e is not None]
        o = make_owner(owner_impl, bus.endpoint)
        bus.closers.append(o)
        time.sleep(1.0)
        lk, xk = key_of(LINK), key_of(LINK, "rogue")
        o.record(LINK, BASE)
        got = s.wait(lk, 1)
        report.check(pairing, sec, "step 1: S reads queue_depth known, 12",
                     bool(got) and states(got[0]["records"][0])["queue_depth"] == "known 12",
                     f"{states(got[0]['records'][0]) if got else None}")
        for q in (14, 15, 9, 16):
            o.record(LINK, _q(BASE, queue_depth=q))
        w = bus.wire.wait(lk, 5)
        report.check(pairing, sec, "step 2: O omits queue_depth from seqs 1 to 4, each within 4 of 12",
                     len(w) == 5 and all("queue_depth" not in included(x["value"]) for x in w[1:5]),
                     f"{[included(x['value']) for x in w if 'value' in x]}")
        got = s.wait(lk, 5)
        report.check(pairing, sec, "step 2: S reads it within at each, 12 give or take 4, and shows it with its delta",
                     [states(g["records"][0])["queue_depth"] for g in got[1:5]] == ["within 12 ±4"] * 4,
                     f"{[states(g['records'][0])['queue_depth'] for g in got[1:5]]}")
        o.record(LINK, _q(BASE, queue_depth=13))
        o.record(LINK, _q(BASE, queue_depth=20))
        w = bus.wire.wait(lk, 7)
        got = s.wait(lk, 7)
        report.check(pairing, sec, "step 3: O includes it at seq 5, five records after seq 0; S reads it known, 13",
                     len(w) >= 6 and "queue_depth" in included(w[5]["value"]) and len(got) >= 6
                     and states(got[5]["records"][0])["queue_depth"] == "known 13",
                     f"{states(got[5]['records'][0])['queue_depth'] if len(got) > 5 else None}")
        report.check(pairing, sec, "step 4: O includes it at seq 6, more than 4 from 13; S reads it known, 20",
                     len(w) >= 7 and "queue_depth" in included(w[6]["value"]) and len(got) >= 7
                     and states(got[6]["records"][0])["queue_depth"] == "known 20",
                     f"{states(got[6]['records'][0])['queue_depth'] if len(got) > 6 else None}")
        x = RawFrames(bus.x_session, xk, _layouts()[LINK])
        bus.closers.append(x)
        time.sleep(0.5)
        x.put(0, {"time": {"s": 1000, "ms": 0}, "records": [{"full": True, "values": dict(BASE), "absent": []}]})
        for q in range(1, 6):
            x.put(q, {"time": {"s": 1000 + q, "ms": 0}, "records": [{"full": False, "values": {}, "absent": []}]})
        got = s.wait(xk, 6)
        qs = [states(g["records"][0])["queue_depth"] for g in got]
        report.check(pairing, sec, "step 5: S reads X's queue_depth within at seqs 1 to 4, and unknown at seq 5, "
                                   "why deadband", qs == ["known 12"] + ["within 12 ±4"] * 4 + ["unknown deadband"],
                     f"{qs}")
        report.check(pairing, sec, "never a value", no_value_shown(got))
        if echo is not None and bare is not None:
            rows = echo.rows(lk)
            qd = [r["frame"]["records"][0]["fields"]["queue_depth"] for r in rows if "records" in r.get("frame", {})]
            report.check(pairing, sec, "observer §7.6: zk2py's echo tool shows a within field with its delta, never "
                                       "as its value alone", qd[1:5] == [{"state": "within", "value": 12,
                                                                          "delta": "4"}] * 4, f"{qd[:6]}")
            rows = bare.rows(lk)
            env = [r.get("frame") for r in rows]
            report.check(pairing, sec, "observer §7.6: without the declaration, zk2py's echo tool shows each frame "
                                       "by its envelope, its run, seq and size, its body never sniffed",
                         len(rows) == 7 and all(e is not None and e.get("form") == "frame" and "seq" in e and "bytes" in e
                                                and r.get("sniffed") is None for r, e in zip(rows, env)),
                         f"{env[:2]}")
    finally:
        bus.close()


# -- §5 ---------------------------------------------------------------------------------------

def section5(report: Report, pairing: str) -> None:
    sec = "§5 a new run"
    owner_impl, reader_impl = pairing.split("-")
    bus = Bus()
    try:
        s = make_reader(reader_impl, bus.s_session, bus.endpoint, ["lab/modem"])
        bus.closers.append(s)
        o = make_owner(owner_impl, bus.endpoint)
        bus.closers.append(o)
        time.sleep(1.0)
        lk = key_of(LINK)
        o.record(LINK, BASE)
        o.record(LINK, _q(BASE, queue_depth=30))
        got = s.wait(lk, 2)
        r = {g["run"] for g in got}
        report.check(pairing, sec, "step 1: S reads seqs 0 and 1 of R", [g["seq"] for g in got] == [0, 1]
                     and len(r) == 1, f"{[(g['run'], g['seq']) for g in got]}")
        r2 = o.new_run()
        time.sleep(0.3)
        o.record(LINK, _q(BASE, queue_depth=30))
        got = s.wait(lk, 3)
        w = bus.wire.wait(lk, 3)
        g = got[2] if len(got) > 2 else None
        report.check(pairing, sec, "step 2: O's first frame under R' is a keyframe at seq 0 of R', though nothing "
                                   "changed", len(w) >= 3 and w[2].get("run") == r2 and w[2].get("seq") == 0
                     and w[2]["value"]["records"][0]["full"], f"{w[2].get('run') if len(w) > 2 else None} {r2}")
        report.check(pairing, sec, "step 2: S reads it at a run it does not hold, every field known",
                     g is not None and g["follows"] == "run" and g["run"] == r2
                     and all(v.startswith("known") for v in states(g["records"][0]).values()),
                     f"{None if g is None else (g['follows'], states(g['records'][0]))}")
        r3 = o.new_run()
        time.sleep(0.3)
        o.lose(LINK)
        o.record(LINK, _q(BASE, queue_depth=40))
        o.record(LINK, _q(BASE, queue_depth=50))
        got = s.wait(lk, 4)
        g = got[3] if len(got) > 3 else None
        report.check(pairing, sec, "step 3: S reads seq 1 of R'' at a run it does not hold: queue_depth known, 50, "
                                   "every other field unknown, why run",
                     g is not None and g["run"] == r3 and g["seq"] == 1 and g["follows"] == "run"
                     and states(g["records"][0]) == {"dropped": "unknown run", "queue_depth": "known 50",
                                                     "duty_available": "unknown run", "duty_wait_ms": "unknown run"},
                     f"{None if g is None else (g['run'], g['seq'], g['follows'], states(g['records'][0]))}")
    finally:
        bus.close()


# -- §6 ---------------------------------------------------------------------------------------

def section6(report: Report, pairing: str) -> None:
    sec = "§6 a late joiner"
    owner_impl, reader_impl = pairing.split("-")
    bus = Bus()
    try:
        o = make_owner(owner_impl, bus.endpoint)
        bus.closers.append(o)
        time.sleep(0.8)
        lk = key_of(LINK)
        o.record(LINK, BASE)
        for k in range(1, 5):
            o.record(LINK, _q(BASE, queue_depth=12 + 10 * k))
        bus.wire.wait(lk, 5)
        s = make_reader(reader_impl, bus.s_session, bus.endpoint, ["lab/modem"])
        bus.closers.append(s)
        time.sleep(0.8)
        report.check(pairing, sec, "step 1: nothing reaches S", s.wait(lk, 1, 0.3) == [])
        for k in range(5, 16):
            o.record(LINK, _q(BASE, queue_depth=12 + 10 * k))
        got = s.wait(lk, 11)
        g = got[0] if got else None
        report.check(pairing, sec, "step 2: S's first frame is seq 5, read as a first frame: queue_depth known, "
                                   "the other three unknown, why first, never 0",
                     g is not None and g["seq"] == 5 and g["follows"] == "first"
                     and states(g["records"][0]) == {"dropped": "unknown first", "queue_depth": "known 62",
                                                     "duty_available": "unknown first", "duty_wait_ms": "unknown first"},
                     f"{None if g is None else (g['seq'], g['follows'], states(g['records'][0]))}")
        seqs = {x["seq"]: states(x["records"][0]) for x in got}
        stay = all(seqs.get(q, {}).get(n) == "unknown first" for q in range(5, 15)
                   for n in ("dropped", "duty_available", "duty_wait_ms"))
        report.check(pairing, sec, "step 2: they stay unknown until seq 15, the next full record, from which every "
                                   "field is known", stay and all(v.startswith("known") for v in seqs.get(15, {"x": "-"}).values()),
                     f"seq 14 {seqs.get(14)}; seq 15 {seqs.get(15)}")
        report.check(pairing, sec, "never a value", no_value_shown(got))
    finally:
        bus.close()


SECTIONS = {1: section1, 2: section2, 3: section3, 4: section4, 5: section5, 6: section6}


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="python -m zk2py.frames_scenarios")
    ap.add_argument("--pairing", action="append", choices=PAIRINGS)
    ap.add_argument("--only", type=int, action="append", choices=sorted(SECTIONS))
    args = ap.parse_args(argv)
    pairings = args.pairing or list(PAIRINGS)
    need = (["owner"] if "rust-py" in pairings else []) + (["zenctl"] if "py-zenctl" in pairings else [])
    missing = [n for n in need if not (BLACKBOX / n).is_file()]
    if missing:
        print(f"error: the black boxes {missing} are not in {BLACKBOX}", file=sys.stderr)
        return 2
    report = Report()
    for p in pairings:
        for n in args.only or sorted(SECTIONS):
            try:
                SECTIONS[n](report, p)
            except Exception as e:  # noqa: BLE001 - a section that crashes is a failure, named
                import traceback

                traceback.print_exc()
                report.check(p, f"§{n}", "the section ran", False, f"{type(e).__name__}: {e}")
    failed = [r for r in report.rows if not r[3]]
    print(f"\n{len(report.rows) - len(failed)} of {len(report.rows)} checks pass")
    by: dict[tuple[str, str], list[bool]] = {}
    for p, s, _, ok, _ in report.rows:
        by.setdefault((p, s.split(" ")[0]), []).append(ok)
    for (p, s), oks in sorted(by.items()):
        print(f"  {p:10s} {s:4s} {sum(oks)}/{len(oks)}")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())

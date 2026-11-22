"""The ahead time on a bus, in both directions with the reference (core
0.113 §7.5, ``freshness.v2`` 0.12 §2.10; #609 round 120)::

    python -m zk2py.ahead_live [--only rust-owner|rust-owner-heartbeat|py-owner]...

The black boxes are ``owner`` and ``zenctl`` of ``bin/current``, run and
never read; their words are their ``--help``'s. Every router is
zenoh-python's.

- ``rust-owner``: the reference's owner example, ``--health
  --clock-reference <key> --clock-offset-ms 2000``, its clock guard holding;
  zk2py reads its heartbeats covering ``health.v1``. Each one heard while the
  guard holds is read as ahead, by zk2py's reader and byte by byte, and
  refused ``code`` by zk2py's reader of round 119; ``clock-offset 0`` brings
  an instant back.
- ``rust-owner-heartbeat``: the same owner with ``--catalog`` and
  ``--owner-heartbeat``: its owner heartbeat of the ``host`` class carries
  the ahead time, one for every group.
- ``py-owner``: zk2py's ``health.v2`` owner, its guard holding, read by
  ``zenctl echo`` on its heartbeat key: what the reference's tool shows of
  the ahead time (``freshness.v2`` §5: "shows the ahead time as ahead: never
  as unknown, and never as an instant").

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
from typing import Any

from . import heartbeat
from . import health as v1
from . import health_v2 as h2
from .health_v2_scenarios import NS, Bus, Report

REPO = Path(__file__).resolve().parents[3]
HEALTH_V1 = REPO / "spec" / "profiles" / "health" / "health.v1.toml"
FRESHNESS_V2 = REPO / "spec" / "profiles" / "freshness" / "freshness.v2.toml"
#: a second covered interface, so that an owner heartbeat holds two groups
BEACON = REPO / "impl" / "python" / "interop" / "freshness" / "beacon.v2.toml"
REF = "zk2py/ahead/reference"
AHEAD_BODY = "ffffffffff"       # the time's first 40 bits; the last two are 1 and 0


def _time_bits(wire: bytes) -> str:
    """The 42 bits of a heartbeat's time, from its put's payload."""
    from . import zk2c

    env = zk2c.parse(wire, "put")
    body = env.record or b""
    return format(int.from_bytes(body[:6], "big") >> 6, "042b")


def _reference(bus: Bus, stop: threading.Event) -> None:
    def run() -> None:
        while not stop.wait(0.3):
            bus.g.put(REF, b"beat")     # no stamp of its own: R1 stamps it

    threading.Thread(target=run, daemon=True).start()


def _rust_owner(report: Report, owner_heartbeat: bool) -> None:
    import zenoh

    from . import catalog as cat, owner_heartbeat as oh
    from .blackbox import BLACKBOX
    from .conformance import _earlier_body, _earlier_owner
    from .contract import load_contract
    from .live_interop import Owner as Exe

    sec = "the reference's owner, its guard holding" + (", by owner heartbeat" if owner_heartbeat else "")
    bus = Bus()
    stop = threading.Event()
    p = None
    try:
        hc = load_contract(HEALTH_V1)
        lo = heartbeat.layout_of(hc.canonical, hc.fingerprint)
        flags = ["--health", "--health-status", "OK:serving", "--clock-reference", REF, "--clock-offset-ms", "2000"]
        key = heartbeat.heartbeat_key("lab", "ahead", "health.v1")
        contracts: list[Path] = []
        expected: list[int] = []
        if owner_heartbeat:
            path = os.path.join(bus.tmp, "catalog.json")
            files = [HEALTH_V1, BEACON, FRESHNESS_V2]
            made = subprocess.run([str(BLACKBOX / "zk2"), "catalog", "new", *map(str, files)],
                                  capture_output=True, text=True, timeout=30)
            if not report.check(sec, "`zk2 catalog new` made the catalog", made.returncode == 0,
                                f"exit {made.returncode}; {made.stderr.strip()[-400:]}"):
                return
            Path(path).write_text(made.stdout, encoding="utf-8")
            flags += ["--catalog", path, "--owner-heartbeat"]
            contracts = [BEACON]
            key = "zk2/lab/ahead/freshness.v2/stream/heartbeats/host"
            # the groups a `host` owner heartbeat holds (freshness.v2 §2.4):
            # "one group for each covered interface whose revision has a
            # collection of that class", each by its index in the catalog
            from . import freshness_v2 as f2

            catalog = cat.read(made.stdout)
            by_fp = {c.fingerprint: c for c in map(load_contract, files)}
            for index, (_, fp) in enumerate(catalog.entries):
                c = by_fp[fp]
                names = {col.resource for col in heartbeat.layout_of(c.canonical, c.fingerprint).collections}
                if any(cls == "host" for r, cls in f2.classes_of(c.canonical).items() if r in names):
                    expected.append(index)
        wires: list[tuple[int, bytes]] = []
        faults: list[tuple[int, Any]] = []
        subs = [bus.s.declare_subscriber(key, zenoh.handlers.Callback(
                    lambda smp: wires.append((time.monotonic_ns(), smp.payload.to_bytes())))),
                bus.s.declare_subscriber("zk2/lab/ahead/health.v1/stream/faults", zenoh.handlers.Callback(
                    lambda smp: faults.append((time.monotonic_ns(), smp))))]
        time.sleep(0.3)
        p = Exe(BLACKBOX / "owner", "lab/ahead", contracts, connect=bus.endpoint, flags=flags)
        if p.wait_for("ready ", 120) is None:
            report.check(sec, "the owner example started", False, f"{p.stderr[-3:]}")
            return
        _reference(bus, stop)
        deadline = time.monotonic() + 15
        while time.monotonic() < deadline and not faults:
            time.sleep(0.05)
        if not report.check(sec, "the owner reports itself ahead: a faults sample is heard", bool(faults), ""):
            return
        t_det = faults[0][0]
        time.sleep(32.0)
        held = [w for t, w in wires if t > t_det + 300_000_000]

        def read(w: bytes) -> Any:
            if owner_heartbeat:
                return oh.read(w, "stream").body.time
            return heartbeat.read_put(lo, w, "stream")["time"]

        def earlier(w: bytes) -> Any:
            from . import zk2c

            if owner_heartbeat:
                return _earlier_owner(w)
            return _earlier_body(lo, zk2c.parse(w, "put").record or b"")

        times = [read(w) for w in held]
        report.check(sec, f"each of its {'owner ' if owner_heartbeat else ''}heartbeats heard while the guard holds, "
                          "over 32 s, is read as ahead by zk2py: never unknown, never an instant",
                     bool(held) and all(t == heartbeat.AHEAD for t in times), f"{len(held)} heard; times {times}")
        report.check(sec, "byte by byte: the seconds all ones and the millisecond code 1022, 0x3fffffffffe, never "
                          "all 42 bits set",
                     bool(held) and all(_time_bits(w) == "1" * 41 + "0" for w in held),
                     f"{[w.hex() for w in held[:2]]}")
        report.check(sec, "zk2py's reader of round 119 refuses each, code: to it the heartbeat confirms nothing",
                     bool(held) and all(earlier(w) == {"refused": "code"} for w in held),
                     f"{[earlier(w) for w in held[:2]]}")
        if owner_heartbeat:
            groups = [[g.index for g in oh.read(w, "stream").body.groups] for w in held]
            report.check(sec, "one ahead time for every group: each owner heartbeat holds a group for each interface "
                              "of the catalog that has a collection of the host class, by its index there, and no "
                              "other, all under its one time",
                         bool(groups) and len(expected) >= 2 and all(g == expected for g in groups),
                         f"groups' indices {groups[:2]}; the catalog's host interfaces {expected}; catalog "
                         f"{catalog.id} {[i for i, _ in catalog.entries]}")
        t_fix = time.monotonic_ns()
        fixed = p.command("clock-offset 0") or ""
        time.sleep(32.0)
        after = [(t, read(w)) for t, w in wires if t > t_fix]
        inst = [t for t, x in after if heartbeat.reading_of(x) == heartbeat.INSTANT]
        report.check(sec, "the clock set right: within 30 s a heartbeat carries an instant again",
                     bool(inst) and (inst[0] - t_fix) / NS <= 30.5,
                     f"{fixed!r}; {[(round((t - t_fix) / NS, 1), heartbeat.reading_of(x)) for t, x in after]}")
        for s_ in subs:
            s_.undeclare()
    finally:
        stop.set()
        if p is not None:
            p.close()
        bus.close()


def rust_owner(report: Report) -> None:
    _rust_owner(report, False)


def rust_owner_heartbeat(report: Report) -> None:
    _rust_owner(report, True)


def py_owner(report: Report) -> None:
    """zk2py's health.v2 owner, its guard holding, and the reference's
    ``zenctl echo`` on its heartbeat key."""
    from .blackbox import BLACKBOX

    sec = "zk2py's owner, its guard holding, read by zenctl"
    bus = Bus()
    stop = threading.Event()
    try:
        o = bus.owner("lab/ahead", "--level", "ok", "--reason", "serving", "--reference", REF, "--offset-ms", "2000",
                      "--run-record", os.path.join(bus.tmp, "ahead.run"))
        key = heartbeat.heartbeat_key("lab", "ahead", h2.IFACE)

        def echo(count: int, seconds: int) -> list[dict[str, Any]]:
            out = subprocess.run([str(BLACKBOX / "zenctl"), "echo", key, "-c", bus.endpoint,
                                  "--contracts", str(h2.CONTRACT), "--contracts", str(FRESHNESS_V2),
                                  "--format", "ndjson", "--count", str(count)],
                                 capture_output=True, text=True, timeout=seconds)
            rows = []
            for line in out.stdout.splitlines():
                try:
                    rows.append(json.loads(line))
                except ValueError:
                    rows.append({"line": line})
            if out.stderr.strip():
                rows.append({"stderr": out.stderr.strip()[-600:]})
            return rows

        _reference(bus, stop)
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline and not o.state()["ahead"]:
            time.sleep(0.05)
        report.check(sec, "the owner reports itself ahead", o.state()["ahead"], str(o.state()["events"]))
        try:
            rows = echo(1, 40)
        except subprocess.TimeoutExpired as e:
            rows = [{"timeout": (e.stdout or b"")[-600:].decode("utf-8", "replace") if isinstance(e.stdout, bytes)
                     else str(e.stdout)[-600:]}]
        def times(rs: list[dict[str, Any]]) -> list[Any]:
            return [r["value"].get("time", "(no time)") for r in rs if isinstance(r.get("value"), dict)]

        report.check(sec, "zenctl echo shows the heartbeat heard while the guard holds, 16 bytes, and its time as "
                          "ahead: never as unknown, and never as an instant",
                     times(rows) == ["ahead"] and rows[0].get("payload_bytes") == 16
                     and rows[0].get("verdict") == "valid" and "dated" not in rows[0]["value"],
                     json.dumps(rows)[:1500])
        o.cmd("clock 0")
        time.sleep(1.0)
        try:
            rows2 = echo(1, 40)
        except subprocess.TimeoutExpired:
            rows2 = [{"timeout": True}]
        t2 = times(rows2)
        report.check(sec, "the clock set right: zenctl echo shows the next heartbeat's time as an instant, dated",
                     len(t2) == 1 and isinstance(t2[0], dict) and set(t2[0]) == {"s", "ms"}
                     and "dated" in rows2[0]["value"], json.dumps(rows2)[:1500])
    finally:
        stop.set()
        bus.close()


SECTIONS = {"rust-owner": rust_owner, "rust-owner-heartbeat": rust_owner_heartbeat, "py-owner": py_owner}


def main(argv: list[str] | None = None) -> int:
    p = argparse.ArgumentParser(prog="zk2py.ahead_live")
    p.add_argument("--only", action="append", choices=sorted(SECTIONS))
    args = p.parse_args(argv)
    report = Report()
    for name in args.only or sorted(SECTIONS):
        try:
            SECTIONS[name](report)
        except Exception as e:  # noqa: BLE001
            import traceback

            report.check(name, "the section ran", False, f"{type(e).__name__}: {e}\n{traceback.format_exc()}")
    bad = [r for r in report.rows if not r[2]]
    print(f"{len(report.rows) - len(bad)} of {len(report.rows)} checks passed")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())

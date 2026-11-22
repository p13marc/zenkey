"""zk2py's ``health.v2`` owner as a process, so that
``zk2py.health_v2_scenarios`` runs two zk2py processes against each other
(#609, round 120)::

    python -m zk2py.health_v2_owner --connect <endpoint> [--tokenless <iface>]...
        [--level <level>] [--reason <word>] [--check <name>=<level>[:<detail>]]...
        [--contract <file>]... [--reference <key>] [--offset-ms <ms>] [--liar]
        [--auth <user>:<password>] [--run-record <file>] [--catalog <file>]
        [--many <n> --nav <file>]
        <system>/<service>

Standard output: ``ready zk2/<system>/<service>/@zk/instance/<id> run <hex>``,
then one line per command read on standard input:

- ``status <level> [<word>]``, ``check <name> <level> [<word>]``,
  ``retire <name>``, ``fault <code> <level> [<argument>]``;
- ``clock <ms>``: the clock's offset from now on (the scenarios' "Without
  root": "offsets the owner's clock in its runtime");
- ``unvouch`` and ``vouch``: stop and resume vouching for the status;
- ``flip <ms>`` and ``flip stop``: every ``<ms>`` the check ``disk`` to
  ``failed`` and back, the status following in §2.2's order;
- ``state``: one JSON line, what the owner holds, its guard and its log;
- ``close``.

``--many <n>`` starts ``<system>/dev0`` to ``dev<n-1>`` in this one process,
each implementing ``health.v2`` and the contract ``--nav`` names, with
status ``ok`` (scenarios §5), and takes no command but ``close``.
"""

from __future__ import annotations

import argparse
import json
import sys
import threading
import time
from typing import Any

from . import health_v2 as h2
from .health_v2_live import HealthV2Owner, RuleError


class Offset:
    """The owner's clock, offset in its runtime."""

    def __init__(self, ms: int = 0):
        self.ms = ms

    def ms_now(self) -> int:
        return time.time_ns() // 1_000_000 + self.ms

    def clock(self, owner: Any) -> Any:
        import zenoh

        def now() -> Any:
            ts = owner.session.new_timestamp()
            n = ts.get_time_as_ntp64().as_nanos() + self.ms * 1_000_000
            return zenoh.Timestamp(zenoh.NTP64(n // 10**9, n % 10**9), ts.get_id())

        return now


def answer(line: str, o: HealthV2Owner, off: Offset, flip: dict[str, Any]) -> str:
    w = line.split()
    try:
        if w[0] == "status":
            o.set_status(w[1], " ".join(w[2:]) or None)
        elif w[0] == "check":
            o.set_check(w[1], w[2], " ".join(w[3:]) or None)
        elif w[0] == "retire":
            o.retire_check(w[1])
        elif w[0] == "fault":
            arg: Any = h2.NOT_AVAILABLE if len(w) < 4 else int(w[3])
            o.publish_fault(None if w[1] == "-" else w[1], w[2], arg)
        elif w[0] == "clock":
            off.ms = int(w[1])
        elif w[0] == "unvouch":
            o.owner.close_writer(o.status_key)
        elif w[0] == "vouch":
            o.owner.reopen_writer(o.status_key)
        elif w[0] == "flip":
            if w[1] == "stop":
                flip["stop"].set()
                flip["thread"].join()
                return f"ok flip stopped after {flip['n']}"
            flip.update(stop=threading.Event(), n=0)
            period = int(w[1]) / 1000

            def run() -> None:
                while not flip["stop"].wait(period):
                    o.set_check("disk", "failed", "timeout")          # the status first, then the check
                    flip["stop"].wait(period)
                    o.set_check("disk", "ok", "ok")                   # the check first, then the status
                    o.set_status("ok", "serving")
                    flip["n"] += 1

            flip["thread"] = threading.Thread(target=run, daemon=True)
            flip["thread"].start()
        elif w[0] == "state":
            return json.dumps({"level": o.level, "reason": o.reason, "checks": o.checks, "ahead": o.owner.ahead,
                               "holds": o.holds, "faults": o.faults, "log": o.log, "events": o.owner.events[-4:],
                               "tables": {k: t.expect() for k, t in o.tables.items()}}, sort_keys=True)
        else:
            return f"refused {line}: unknown command"
    except RuleError as e:
        return f"refused {line}: {e}"
    except Exception as e:  # noqa: BLE001 - reported, the process goes on
        return f"failed {line}: {type(e).__name__}: {e}"
    return f"ok {line}"


def main(argv: list[str] | None = None) -> int:
    from .contract import load_contract

    p = argparse.ArgumentParser(prog="zk2py.health_v2_owner")
    p.add_argument("address")
    p.add_argument("--connect", required=True)
    p.add_argument("--tokenless", action="append", default=[])
    p.add_argument("--level", default=h2.UNKNOWN)
    p.add_argument("--reason", default="starting")
    p.add_argument("--check", action="append", default=[])
    p.add_argument("--contract", action="append", default=[])
    p.add_argument("--reference")
    p.add_argument("--offset-ms", type=int, default=0)
    p.add_argument("--liar", action="store_true")
    p.add_argument("--auth")
    p.add_argument("--run-record")
    p.add_argument("--catalog")
    p.add_argument("--many", type=int, default=0)
    p.add_argument("--nav")
    a = p.parse_args(argv)
    system, service = a.address.split("/")
    kw: dict[str, Any] = {"connect": a.connect}
    if a.tokenless:
        kw["tokenless"] = set(a.tokenless)
    if a.auth:
        kw["auth"] = tuple(a.auth.split(":", 1))
    if a.run_record:
        kw["run_record"] = a.run_record   # a 16-bit run, kept in this file (core §4.3)
    if a.many:
        nav = load_contract(a.nav)
        owners = []
        for i in range(a.many):
            o = HealthV2Owner(system, f"dev{i}", contracts=[nav], level="ok", reason="serving", **kw)
            o.start()
            owners.append(o)
        print(f"ready {a.many} owners", flush=True)
        for line in sys.stdin:
            if line.strip() == "close":
                break
        for o in owners:
            o.close()
        return 0
    off = Offset(a.offset_ms)
    if a.reference:
        kw["heartbeat"] = a.reference
    own = [load_contract(f) for f in a.contract]
    bad = [c for c in own if not c.valid]
    if bad:
        print(f"not started: {[c.codes for c in bad]}", file=sys.stderr)
        return 1
    o = HealthV2Owner(system, service, contracts=own, level=a.level, reason=a.reason or None, clock_ms=off.ms_now,
                      liar=a.liar, **kw)
    if a.reference or a.offset_ms:
        o.owner.clock = off.clock(o.owner)
    if a.catalog:
        # core §8.6: "An owner given a catalog counts its `time` fields from
        # the catalog's epoch, and states its id in its descriptor"
        from pathlib import Path

        from . import catalog as cat

        o.owner.catalog = cat.read(Path(a.catalog).read_bytes())
    # checks held before the tokens: a test owner's (scenarios §7)
    for spec in a.check:
        name, _, rest = spec.partition("=")
        level, _, detail = rest.partition(":")
        o.hold_check(name, level, detail or None)   # its table and its epoch, as every later check's
    o.start()
    inst = getattr(o.owner, "instance", None)
    print(f"ready zk2/{system}/{service}/@zk/instance/{inst} run {o.owner.run_hex()}", flush=True)
    flip: dict[str, Any] = {}
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        if line == "close":
            break
        print(answer(line, o, off, flip), flush=True)
    if flip.get("stop") is not None:
        flip["stop"].set()
    o.close()
    print("closed", flush=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())

"""``arbitration.v1``'s parties as processes (#609, round 77), driven as
black boxes by :mod:`zk2py.arbitration_scenarios`::

    python -m zk2py.arbitration_owner commander SYSTEM/SERVICE CONTRACT.toml --connect EP [--period-ms 50]
        [--lead-ms 500] [--standby] [--instance-id ID]
    python -m zk2py.arbitration_owner actuator SYSTEM/SERVICE CONTRACT.toml HELD.toml --connect EP
        --bind ENTRY[,ENTRY…] [--role cmd] [--param NAME=VALUE]... [--watch-connect EP] [--epsilon-ms N]
        [--no-infer-moves]

**A commander** prints ``ready <instance key>`` once started, then answers
one line per command on standard input: ``hold``, ``resume``, ``remint``
(``reminted <old> <new>``), ``count`` (``count <n>``, the puts made). Since
round 78 it also takes the reference owner's ``commander hold|resume``,
``commander raw on|off``, ``commander sent``, ``commander new-epoch`` and
``commander zid``, answered in its JSON, and ``redundancy role`` with
``--standby``; its first put is ``--lead-ms`` after its tokens (text 0.2,
F-269).

**An actuator** prints one JSON line per switch on standard output, as
``scenarios.md``'s conventions write it, its first the ``start`` switch;
``ready <instance key>`` goes to standard error. A refusal is ``not
started: <address>: invalid: <reasons>`` on standard error, exit 1, with
nothing declared. Commands on standard input, each answered with one JSON
line: ``counts`` (§2.5's counts), ``reads`` (each read again it made),
``trips`` (each round trip on its watch session) and ``log`` (each presence
event as handed to the arbiter, a move included); and the reference owner's
``arbitrate counts``, ``arbitrate reads`` and ``arbitrate state``, in its
JSON (round 78).

Standard input closed is a clean stop: core §8.2's, its tokens first.
"""

from __future__ import annotations

import argparse
import json
import os
import sys
import time
from typing import Any


def _commander(args: argparse.Namespace) -> int:
    from .arbitration_live import Commander
    from .contract import load_contract

    c = Commander(args.address, load_contract(args.contract), args.connect, args.period_ms / 1000,
                  lead_s=args.lead_ms / 1000, standby=args.standby, instance=args.instance_id,
                  contract_path=args.contract)
    c.start()
    print(f"connected {args.connect}", flush=True)
    print(f"ready zk2/{args.address}/@zk/instance/{c.owner.instance}", flush=True)

    def say(x: dict[str, Any]) -> None:
        print(json.dumps(x), flush=True)

    try:
        for line in sys.stdin:
            cmd = line.strip()
            if cmd == "hold":
                c.hold()
                print("ok hold", flush=True)
            elif cmd == "resume":
                c.resume()
                print("ok resume", flush=True)
            elif cmd == "remint":
                old, new = c.remint()
                print(f"reminted {old} {new}", flush=True)
            elif cmd == "count":
                print(f"count {c.count()}", flush=True)
            elif cmd in ("commander hold", "commander resume"):
                (c.hold if cmd.endswith("hold") else c.resume)()
                say({"commander": cmd.split()[1], "on": not c.held.is_set()})
            elif cmd in ("commander raw on", "commander raw off"):
                (c.raw.set if cmd.endswith("on") else c.raw.clear)()
                say({"commander": "raw", "raw": c.raw.is_set()})
            elif cmd == "commander sent":
                n, refused = c.sent()
                say({"commander": "sent", "sent": n, "refused": refused})
            elif cmd == "commander new-epoch":
                _, new = c.remint()
                say({"commander": "new-epoch", "instance": new})
            elif cmd == "commander zid":
                say({"commander": "zid", "zid": str(c.owner.session.zid())})
            elif cmd == "redundancy role" and c.candidate is not None:
                say({"redundancy": "role", "instance": c.candidate.instance,
                     "interfaces": {c.candidate.iface: {"role": c.candidate.role}}})
            elif cmd:
                print(f"refused {cmd}: unknown command", flush=True)
    finally:
        c.close()
    return 0


def _actuator(args: argparse.Namespace) -> int:
    from .arbitration_live import Actuator, Refused, emit_stdout
    from .contract import load_contract

    params = dict(p.split("=", 1) for p in args.param)
    try:
        a = Actuator(args.address, load_contract(args.contract), load_contract(args.held), connect=args.connect,
                     bindings=[b for b in args.bind.split(",") if b] if args.bind else [], role=args.role,
                     params=params, watch_connect=args.watch_connect, epsilon_ms=args.epsilon_ms, emit=emit_stdout,
                     infer_moves=not args.no_infer_moves)
    except Refused as e:
        print(f"not started: {e}", file=sys.stderr, flush=True)
        return 1
    # t0 on the host's monotonic clock, which every process shares, so a
    # runner can place each line's at_ms on its own clock
    print(f"ready zk2/{args.address}/@zk/instance/{a.owner.instance} t0={a.t0} now={time.monotonic_ns()}",
          file=sys.stderr, flush=True)
    try:
        for line in sys.stdin:
            cmd = line.strip()
            out: dict[str, Any]
            if cmd in ("counts", "arbitrate counts"):
                out = {"counts": a.counts()}
            elif cmd == "arbitrate reads":
                with a.lock:
                    out = {"arbitrate": "reads", "reads": sum(1 for r in a.reads if not r.get("at_start")),
                           "not_made": 0, "round_trips": len(a.round_trips)}
            elif cmd == "arbitrate state":
                with a.lock:
                    out = {"arbitrate": "state", "active": a.arb.active, "providers": list(a.arb.providers),
                           "withdrawn": [x for x, p in a.arb.providers.items() if p.withdrawn],
                           "moving": a.arb.pending, "watching": a.watch is not None,
                           "deadline_ms": a.decl.deadline_ms}
            elif cmd == "reads":
                with a.lock:
                    out = {"reads": list(a.reads)}
            elif cmd == "log":
                with a.lock:
                    out = {"log": list(a.log)}
            elif cmd == "trips":
                with a.lock:
                    out = {"round_trips": list(a.round_trips), "moves": len(a.moves)}
            elif cmd:
                out = {"unknown": cmd}
            else:
                continue
            emit_stdout(json.dumps(out))
    finally:
        a.close()
    return 0


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="python -m zk2py.arbitration_owner")
    sub = ap.add_subparsers(dest="what", required=True)
    c = sub.add_parser("commander")
    c.add_argument("address")
    c.add_argument("contract")
    c.add_argument("--connect", required=True)
    c.add_argument("--period-ms", type=int, default=50)
    c.add_argument("--lead-ms", type=int, default=500, help="the first put this long after the tokens (F-269)")
    c.add_argument("--standby", action="store_true", help="a redundancy.v1 candidate (scenarios §11)")
    c.add_argument("--instance-id")
    a = sub.add_parser("actuator")
    a.add_argument("address")
    a.add_argument("contract")
    a.add_argument("held")
    a.add_argument("--connect", required=True)
    a.add_argument("--watch-connect")
    a.add_argument("--bind", default="")
    a.add_argument("--role", default="cmd")
    a.add_argument("--param", action="append", default=[])
    a.add_argument("--epsilon-ms", type=int)
    a.add_argument("--no-infer-moves", action="store_true",
                   help="the control of F-268: a move only by the watch session's transport event")
    args = ap.parse_args(argv)
    return _commander(args) if args.what == "commander" else _actuator(args)


if __name__ == "__main__":
    code = main()
    sys.stdout.flush()
    sys.stderr.flush()
    # zenoh-python's threads can hold the interpreter's shutdown: every
    # session is closed by now
    os._exit(code)

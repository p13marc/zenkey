"""Helper processes for the budget measurements (``scenarios/budget.md``
§1–§3): ``python -m zk2py.proc router …`` and ``python -m zk2py.proc
publish …``.

§1 needs each router in a process of its own: "The sessions of one process
share zenoh's receive threads … That is a measurement of the process, not
of the path". A helper prints ``ready`` once it is up, and runs until its
stdin closes.
"""

from __future__ import annotations

import argparse
import json
import sys
import time


def _config(mode: str, listen: str | None, connect: list[str], lease_ms: int | None, timestamping: bool):
    import zenoh

    conf = zenoh.Config()
    conf.insert_json5("mode", json.dumps(mode))
    conf.insert_json5("listen/endpoints", json.dumps([listen] if listen else []))
    if connect:
        conf.insert_json5("connect/endpoints", json.dumps(connect))
    conf.insert_json5("scouting/multicast/enabled", "false")
    conf.insert_json5("timestamping/enabled", "true" if timestamping else "false")
    if lease_ms is not None:
        conf.insert_json5("transport/link/tx/lease", str(lease_ms))
    return conf


def router(args: argparse.Namespace) -> None:
    import zenoh

    s = zenoh.open(_config("router", args.listen, args.connect or [], args.lease_ms, True))
    print("ready", flush=True)
    sys.stdin.read()
    s.close()


def publish(args: argparse.Namespace) -> None:
    """§1's owner: a client of R2 holding an instance token, putting
    ``count`` numbered samples of ``size`` bytes as fast as its publisher
    lets it, once told ``go`` on stdin."""
    import zenoh

    s = zenoh.open(_config("client", None, [args.connect], None, True))
    token = s.liveliness().declare_token(args.token)
    pub = s.declare_publisher(
        args.key, encoding="text/plain",
        reliability=zenoh.Reliability.RELIABLE if args.reliability == "reliable" else zenoh.Reliability.BEST_EFFORT,
        congestion_control=zenoh.CongestionControl.BLOCK if args.congestion == "block" else zenoh.CongestionControl.DROP)
    print("ready", flush=True)
    for line in sys.stdin:
        if line.strip() == "go":
            break
    pad = b"x" * (args.size - 4)
    t0 = time.monotonic()
    for i in range(args.count):
        pub.put(i.to_bytes(4, "big") + pad)
    print(f"done {time.monotonic() - t0:.3f}", flush=True)
    sys.stdin.read()
    pub.undeclare()
    token.undeclare()
    s.close()


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="python -m zk2py.proc")
    sub = ap.add_subparsers(dest="cmd", required=True)
    r = sub.add_parser("router")
    r.add_argument("--listen", required=True)
    r.add_argument("--connect", action="append")
    r.add_argument("--lease-ms", type=int)
    p = sub.add_parser("publish")
    p.add_argument("--connect", required=True)
    p.add_argument("--key", required=True)
    p.add_argument("--token", required=True)
    p.add_argument("--count", type=int, default=1000)
    p.add_argument("--size", type=int, default=16384)
    p.add_argument("--reliability", choices=["reliable", "best_effort"], default="reliable")
    p.add_argument("--congestion", choices=["block", "drop"], default="block")
    args = ap.parse_args(argv)
    {"router": router, "publish": publish}[args.cmd](args)
    return 0


if __name__ == "__main__":
    sys.exit(main())

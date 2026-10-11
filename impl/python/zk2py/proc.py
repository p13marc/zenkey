"""Helper processes for the budget measurements (``scenarios/budget.md``
§1–§3): ``python -m zk2py.proc router …``, ``… publish …`` and, for §1 step
4 (0.27), ``… consume …``, a consumer whose whole process is stopped.

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


def consume(args: argparse.Namespace) -> None:
    """§1 step 4's consumer (0.27), in a process of its own: a client of R1
    that subscribes to the owner's key, takes 2 ms over each sample, holds a
    liveliness subscriber on the owner's token, watches its own transport
    events and runs a ticker (:class:`zk2py.budget.StallWatch`). It prints
    ``token`` once the owner's token has reached it, stops its own process
    (SIGSTOP) at sample ``--stop-at`` after printing ``stop``, and, once no
    sample has arrived for a while, prints ``result <json>``."""
    import os
    import signal

    import zenoh

    from .budget import StallWatch

    s = zenoh.open(_config("client", None, [args.connect], None, False))
    watch = StallWatch(s)
    got: list[int] = []
    withdrawn: list[float] = []
    last = [time.monotonic()]
    stopped = [False]

    def on_sample(sample) -> None:
        i = int.from_bytes(sample.payload.to_bytes()[:4], "big")
        got.append(i)
        last[0] = time.monotonic()
        if i == args.stop_at and not stopped[0]:
            stopped[0] = True
            print("stop", flush=True)
            os.kill(os.getpid(), signal.SIGSTOP)
        time.sleep(0.002)

    lv = s.liveliness().declare_subscriber(args.token, zenoh.handlers.Callback(
        lambda smp: withdrawn.append(time.monotonic()) if smp.kind == zenoh.SampleKind.DELETE else None),
        history=True)
    sub = s.declare_subscriber(args.key, zenoh.handlers.Callback(on_sample))
    print("ready", flush=True)
    deadline = time.monotonic() + 10
    while time.monotonic() < deadline:
        replies = list(s.liveliness().get(args.token, timeout=1.0))
        if any(r.ok is not None for r in replies):
            break
        time.sleep(0.1)
    print("token", flush=True)
    last[0] = time.monotonic()
    while time.monotonic() - last[0] < 9.0 and len(got) < args.count:
        time.sleep(0.1)
    gap, closed = watch.reading()
    print("result " + json.dumps({"got": len(got), "withdrawn": len(withdrawn), "closed": closed, "gap_ms": gap,
                                  "closes": len(watch.closes)}), flush=True)
    sys.stdin.read()
    sub.undeclare()
    lv.undeclare()
    watch.close()
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
    c = sub.add_parser("consume")
    c.add_argument("--connect", required=True)
    c.add_argument("--key", required=True)
    c.add_argument("--token", required=True)
    c.add_argument("--count", type=int, default=1000)
    c.add_argument("--stop-at", type=int, default=100)
    args = ap.parse_args(argv)
    {"router": router, "publish": publish, "consume": consume}[args.cmd](args)
    return 0


if __name__ == "__main__":
    sys.exit(main())

#!/usr/bin/env python3
"""A userspace stand-in for the lab's netem, between two udp-medium drivers (#989).

zenoh-modem's lab shapes the IP link between two `modem-driver` daemons with
netem, which needs root. This relay sits between the two drivers' UDP sockets on
127.0.0.1 instead and applies what netem would: a per-datagram loss, a one-way
latency with jitter, and nothing else. It keeps each direction in order, as the
medium does. The drivers pace themselves to the bit rate (`rate_bps`); the
relay never queues for bandwidth.

Each driver names the relay as its peer:

    driver A  --udp-->  [a-side port]  relay  [b-side port]  --udp-->  driver B

so driver A's datagrams arrive at the a-side port and leave from the b-side
port, which is the address driver B's `peers` lists for `a`, and back.

It also records every datagram, one JSON line each (time, direction, the lane
byte, the SDU's length, whether it was dropped, the SDU in hex), and prints a
per-direction summary on SIGTERM or SIGINT. Standard library only.
"""
import argparse
import asyncio
import json
import random
import signal
import sys
import time


def parse_addr(text):
    host, port = text.rsplit(":", 1)
    return host, int(port)


class Direction:
    def __init__(self, name, rng, loss, latency, jitter):
        self.name = name
        self.rng = rng
        self.loss = loss
        self.latency = latency
        self.jitter = jitter
        self.last_due = 0.0
        self.datagrams = 0
        self.sdu_bytes = 0
        self.dropped = 0
        self.dropped_bytes = 0

    def decide(self, now):
        """Whether to drop, and when to deliver: never before the last one (FIFO)."""
        drop = self.rng.random() < self.loss
        delay = self.latency + self.rng.uniform(-self.jitter, self.jitter)
        due = max(self.last_due, now + max(delay, 0.0))
        if not drop:
            self.last_due = due
        return drop, due


class Face(asyncio.DatagramProtocol):
    def __init__(self, relay, side):
        self.relay = relay
        self.side = side
        self.transport = None

    def connection_made(self, transport):
        self.transport = transport

    def datagram_received(self, data, addr):
        self.relay.received(self.side, data, addr)


class Relay:
    def __init__(self, args):
        self.args = args
        self.loop = asyncio.get_running_loop()
        rng = random.Random(args.seed)
        mk = lambda n: Direction(n, random.Random(rng.random()), args.loss,
                                 args.latency_ms / 1000, args.jitter_ms / 1000)
        self.dirs = {"ab": mk("ab"), "ba": mk("ba")}
        self.driver_a = parse_addr(args.driver_a)
        self.driver_b = parse_addr(args.driver_b)
        self.faces = {}
        self.strangers = 0
        self.log = open(args.capture, "w", buffering=1) if args.capture else None
        self.t0 = time.time()

    async def open(self):
        for side, addr in (("a", self.args.a_side), ("b", self.args.b_side)):
            _, face = await self.loop.create_datagram_endpoint(
                lambda s=side: Face(self, s), local_addr=parse_addr(addr))
            self.faces[side] = face

    def received(self, side, data, addr):
        now = self.loop.time()
        if side == "a" and addr[:2] == self.driver_a:
            d, out, to = self.dirs["ab"], self.faces["b"], self.driver_b
        elif side == "b" and addr[:2] == self.driver_b:
            d, out, to = self.dirs["ba"], self.faces["a"], self.driver_a
        else:
            self.strangers += 1
            return
        drop, due = d.decide(now)
        sdu = data[1:]
        d.datagrams += 1
        d.sdu_bytes += len(sdu)
        if drop:
            d.dropped += 1
            d.dropped_bytes += len(sdu)
        if self.log:
            self.log.write(json.dumps({
                "t": round(time.time(), 6), "dir": d.name, "lane": data[0] if data else None,
                "n": len(sdu), "drop": drop, "hex": sdu.hex()}) + "\n")
        if not drop:
            self.loop.call_at(due, out.transport.sendto, data, to)

    def summary(self):
        return {
            "seconds": round(time.time() - self.t0, 3),
            "loss": self.args.loss, "latency_ms": self.args.latency_ms,
            "jitter_ms": self.args.jitter_ms, "seed": self.args.seed,
            "strangers": self.strangers,
            **{n: {"datagrams": d.datagrams, "sdu_bytes": d.sdu_bytes, "dropped": d.dropped,
                   "dropped_bytes": d.dropped_bytes} for n, d in self.dirs.items()},
        }


async def main():
    p = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    p.add_argument("--a-side", required=True, help="where driver A sends (its peer b)")
    p.add_argument("--b-side", required=True, help="where driver B sends (its peer a)")
    p.add_argument("--driver-a", required=True, help="driver A's bind address")
    p.add_argument("--driver-b", required=True, help="driver B's bind address")
    p.add_argument("--loss", type=float, default=0.05)
    p.add_argument("--latency-ms", type=float, default=150)
    p.add_argument("--jitter-ms", type=float, default=30)
    p.add_argument("--seed", type=int, default=0)
    p.add_argument("--capture", help="a JSON line per datagram")
    p.add_argument("--summary", help="where to write the summary at exit")
    args = p.parse_args()
    relay = Relay(args)
    await relay.open()
    stop = asyncio.Event()
    for sig in (signal.SIGTERM, signal.SIGINT):
        relay.loop.add_signal_handler(sig, stop.set)
    print(json.dumps({"relay": "up", "a_side": args.a_side, "b_side": args.b_side}), flush=True)
    await stop.wait()
    out = json.dumps(relay.summary())
    print(out, flush=True)
    if args.summary:
        with open(args.summary, "w") as f:
            f.write(out + "\n")


if __name__ == "__main__":
    asyncio.run(main())

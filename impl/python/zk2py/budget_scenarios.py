"""The budget scenarios (``spec/scenarios/budget.md``, core §2.7, 0.25):
``python -m zk2py.budget_scenarios [--only 1 …] [--owner <exe>]``.

- **§1–§3** are measurements of zenoh, the premises §2.7's "lost no
  delivery" rests on, reproduced from Python on loopback:
  - §1's routers each run in a process of their own (``python -m zk2py.proc
    router``), as does its owner (``… publish``); the consumer is this
    process, its callback taking 2 ms over each sample;
  - §2's links are TCP relays of this runner's (:class:`Link`) that can be
    closed, or stalled (their forwarding stops and nothing closes), with a
    1 s lease on both routers and the consumer;
  - §3 uses zenoh-python's ``zenoh.ext`` advanced publisher and subscriber,
    its sample-miss detection, across a relay that can be closed.
- **§4–§7** are an owner keeping its bound and its rate, run against zk2py's
  owner and, when ``--owner`` names it, against the Rust owner example as a
  black box (its ``--cardinality`` flag and its ``put``, ``delete`` and
  ``fill`` stdin commands). Each write is checked accepted or refused, a
  refusal checked to name what the scenario says, and a subscriber checks
  that a refused write published nothing.

The contract of §2 and §4–§7 is ``budget.v1``, copied from the text into
``impl/python/interop/budget/budget.v1.toml``.

Exit 0 when every check passes, 1 when any fails.
"""

from __future__ import annotations

import argparse
import json
import os
import socket
import subprocess
import sys
import threading
import time
from pathlib import Path
from typing import Any

REPO = Path(__file__).resolve().parents[3]
BUDGET = REPO / "impl" / "python" / "interop" / "budget" / "budget.v1.toml"
NS = 1_000_000_000


class Report:
    def __init__(self) -> None:
        self.rows: list[tuple[str, str, bool, str]] = []

    def check(self, section: str, name: str, ok: bool, detail: str = "") -> bool:
        self.rows.append((section, name, ok, detail))
        print(f"{'PASS' if ok else 'FAIL'} [{section}] {name}" + (f": {detail}" if detail else ""), flush=True)
        return ok

    def info(self, section: str, text: str) -> None:
        print(f"info [{section}] {text}", flush=True)


def _free_port() -> int:
    from .owner import free_loopback_port

    return free_loopback_port()


class Proc:
    """A helper process (:mod:`zk2py.proc`), up once it prints ``ready``."""

    def __init__(self, *args: str):
        env = {**os.environ, "PYTHONPATH": str(REPO / "impl" / "python")}
        self.p = subprocess.Popen([sys.executable, "-m", "zk2py.proc", *args], stdin=subprocess.PIPE,
                                  stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, env=env)
        line = self.p.stdout.readline().strip()
        if line != "ready":
            raise RuntimeError(f"zk2py.proc {args[0]}: {line!r} {self.p.stderr.read()[-300:]!r}")

    def say(self, line: str) -> None:
        self.p.stdin.write(line + "\n")
        self.p.stdin.flush()

    def line(self) -> str:
        return self.p.stdout.readline().strip()

    def close(self) -> None:
        try:
            self.p.stdin.close()
        except OSError:
            pass
        try:
            self.p.wait(10)
        except subprocess.TimeoutExpired:
            self.p.kill()
            self.p.wait()


def router(lease_ms: int | None = None, connect: str | None = None) -> tuple[Proc, str]:
    port = _free_port()
    ep = f"tcp/127.0.0.1:{port}"
    args = ["router", "--listen", ep]
    if connect:
        args += ["--connect", connect]
    if lease_ms is not None:
        args += ["--lease-ms", str(lease_ms)]
    return Proc(*args), ep


def client(endpoint: str, lease_ms: int | None = None):
    import zenoh

    from .proc import _config

    return zenoh.open(_config("client", None, [endpoint], lease_ms, False))


class Link:
    """A TCP relay that can be closed (its connections closed, and new ones
    refused) or stalled (its forwarding stopped, both ways, and nothing
    closed), as a pulled cable or a radio out of range."""

    def __init__(self, upstream: str):
        host, port = upstream.removeprefix("tcp/").rsplit(":", 1)
        self.upstream = (host, int(port))
        self.flowing = threading.Event()
        self.flowing.set()
        self.open = True
        self.lock = threading.Lock()
        self.conns: list[socket.socket] = []
        self.sock = socket.socket()
        self.sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        self.sock.bind(("127.0.0.1", 0))
        self.sock.listen(16)
        self.endpoint = f"tcp/127.0.0.1:{self.sock.getsockname()[1]}"
        threading.Thread(target=self._accept, daemon=True).start()

    def _accept(self) -> None:
        while True:
            try:
                c, _ = self.sock.accept()
            except OSError:
                return
            if not self.open:
                c.close()
                continue
            try:
                u = socket.create_connection(self.upstream)
            except OSError:
                c.close()
                continue
            with self.lock:
                self.conns += [c, u]
            for a, b in ((c, u), (u, c)):
                threading.Thread(target=self._pump, args=(a, b), daemon=True).start()

    def _pump(self, src: socket.socket, dst: socket.socket) -> None:
        try:
            while True:
                data = src.recv(65536)
                if not data:
                    break
                self.flowing.wait()
                dst.sendall(data)
        except OSError:
            pass
        for s in (src, dst):
            try:
                s.close()
            except OSError:
                pass

    def cut(self) -> None:
        self.open = False
        with self.lock:
            conns, self.conns = self.conns, []
        for s in conns:
            try:
                s.shutdown(socket.SHUT_RDWR)
                s.close()
            except OSError:
                pass

    def stall(self) -> None:
        self.flowing.clear()

    def heal(self) -> None:
        """After a stall, the connections zenoh gave up on are dropped too."""
        if not self.flowing.is_set():
            with self.lock:
                conns, self.conns = self.conns, []
            for s in conns:
                try:
                    s.shutdown(socket.SHUT_RDWR)
                    s.close()
                except OSError:
                    pass
        self.open = True
        self.flowing.set()

    def close(self) -> None:
        self.cut()
        self.flowing.set()
        try:
            self.sock.close()
        except OSError:
            pass


def _wait(pred, timeout: float, step: float = 0.01) -> bool:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if pred():
            return True
        time.sleep(step)
    return pred()


# -- §1 ------------------------------------------------------------------------------

def section1(report: Report) -> None:
    import zenoh

    sec = "§1 a reliable, blocking path loses nothing to a slow reader"
    key, token = "zk2/lab/m/budget.v1/stream/load/c0", "zk2/lab/m/@zk/instance/0123456789abcdef"
    results = []
    for step, (rel, cong, stuck) in enumerate((("reliable", "block", None), ("best_effort", "drop", None),
                                                ("reliable", "block", 100)), 1):
        r1, ep1 = router()
        r2, ep2 = router(connect=ep1)
        consumer = client(ep1)
        got: list[int] = []
        withdrawn: list[float] = []
        last = [time.monotonic()]

        def on_sample(s: Any) -> None:
            i = int.from_bytes(s.payload.to_bytes()[:4], "big")
            got.append(i)
            last[0] = time.monotonic()
            time.sleep(7.0 if stuck is not None and i == stuck else 0.002)

        lv = consumer.liveliness().declare_subscriber(token, zenoh.handlers.Callback(
            lambda s: withdrawn.append(time.monotonic()) if s.kind == zenoh.SampleKind.DELETE else None))
        sub = consumer.declare_subscriber(key, zenoh.handlers.Callback(on_sample))
        owner = Proc("publish", "--connect", ep2, "--key", key, "--token", token, "--reliability", rel,
                     "--congestion", cong)
        try:
            from . import live

            _wait(lambda: bool(live.list_presence(consumer, token).instances), 5.0, 0.05)
            time.sleep(0.5)
            owner.say("go")
            done = owner.line()
            _wait(lambda: len(got) >= 1000 or time.monotonic() - last[0] > 9.0, 60.0, 0.1)
            results.append((step, rel, cong, stuck, list(got), list(withdrawn), done))
        finally:
            owner.close()
            sub.undeclare()
            lv.undeclare()
            consumer.close()
            r2.close()
            r1.close()
    (_, _, _, _, g1, w1, d1), (_, _, _, _, g2, w2, d2), (_, _, _, _, g3, w3, d3) = results
    elapsed = float(d1.split()[1]) if d1.startswith("done ") else None
    report.check(sec, "step 1, reliable, block: all 1,000 arrive, in order, and the owner's token is never "
                      "withdrawn", g1 == list(range(1000)) and not w1,
                 f"{len(g1)} arrived, in order {g1 == sorted(g1)}, {len(w1)} withdrawals")
    # "The owner's put waits for the path": here the 16 MiB fit in the
    # path's buffers (three loopback TCP connections and zenoh's queues), so
    # the puts returned before the reader had taken them. Reported, not
    # judged: the text's figure is the reference host's.
    report.info(sec, f"step 1: the owner's 1,000 puts took {elapsed} s, while the reader took about "
                     f"{len(g1) * 0.002:.1f} s: the path's buffers held what the reader had not yet taken")
    report.check(sec, "step 2, best effort, drop: some never arrive, and nothing the consumer receives says so (no "
                      "withdrawal)", len(g2) < 1000 and not w2,
                 f"{1000 - len(g2)} lost, {len(w2)} withdrawals ({d2})")
    report.check(sec, "step 3, reliable, block, the consumer stuck for 7 s on the 100th: samples are lost, and the "
                      "consumer sees the owner's token withdrawn",
                 len(g3) < 1000 and bool(w3), f"{1000 - len(g3)} lost, {len(w3)} withdrawals ({d3})")


# -- §2 ------------------------------------------------------------------------------

def section2(report: Report) -> None:
    import zenoh

    from .contract import load_contract
    from .owner import Owner

    sec = "§2 a failed path withdraws the owner's token"
    r1, ep1 = router(lease_ms=1000)
    l1 = Link(ep1)
    r2, ep2 = router(lease_ms=1000, connect=l1.endpoint)
    l2 = Link(ep1)
    consumer = client(l2.endpoint, lease_ms=1000)
    contract = load_contract(BUDGET)
    owner = Owner("lab", "budget", [contract], connect=ep2)
    events: list[tuple[str, float]] = []
    got: list[tuple[int, float]] = []
    stop = threading.Event()
    try:
        owner.start()
        key = f"{owner.prefix(contract)}/stream/load/c0"
        lv = consumer.liveliness().declare_subscriber(owner.instance_key, zenoh.handlers.Callback(
            lambda s: events.append((str(s.kind), time.monotonic()))), history=True)
        sub = consumer.declare_subscriber(key, zenoh.handlers.Callback(
            lambda s: got.append((int(s.payload.to_bytes()), time.monotonic()))))
        counter = [0]

        def puts() -> None:
            while not stop.wait(0.01):
                counter[0] += 1
                owner.publish(key, str(counter[0]).encode())

        threading.Thread(target=puts, daemon=True).start()

        def present() -> bool:
            return bool(events) and "PUT" in events[-1][0]

        if not report.check(sec, "the consumer sees the owner's instance token through both links", _wait(present, 10),
                            str(events)):
            return
        for name, link in (("between the routers", l1), ("on the consumer's own link", l2)):
            for how in ("closed", "stalled"):
                time.sleep(1.0)
                n_before = len(events)
                before = got[-1][0] if got else 0
                t0 = time.monotonic()
                link.cut() if how == "closed" else link.stall()
                seen = _wait(lambda: len(events) > n_before and "DELETE" in events[-1][0], 15, 0.001)
                delay = (events[-1][1] - t0) if seen else None
                at_withdrawal = counter[0]
                link.heal()
                back = _wait(present, 30, 0.05)
                _wait(lambda: bool(got) and got[-1][0] > counter[0] - 20, 10, 0.05)
                after = [i for i, _ in got if i > before]
                lost = (at_withdrawal - before) - len([i for i in after if i <= at_withdrawal])
                if how == "closed":
                    ok = seen and delay is not None and delay < 0.2 and back
                    want = "withdrawn at once"
                else:
                    ok = seen and delay is not None and 0.8 <= delay <= 2.5 and back and lost > 0
                    want = "withdrawn one lease (1 s) after the stall began, the samples put meanwhile lost"
                report.check(sec, f"{name}, {how}: the owner's token {want}; healed, it comes back", ok,
                             f"withdrawn after {None if delay is None else round(delay * 1000, 1)} ms, "
                             f"{lost} samples lost, back {back}")
        sub.undeclare()
        lv.undeclare()
    finally:
        stop.set()
        owner.close()
        consumer.close()
        for x in (l2, l1):
            x.close()
        r2.close()
        r1.close()


# -- §3 ------------------------------------------------------------------------------

def section3(report: Report) -> None:
    import zenoh
    import zenoh.ext as ext

    sec = "§3 miss detection cannot see a publisher it never heard"
    r1, ep1 = router()
    link = Link(ep1)
    r2, ep2 = router(connect=link.endpoint)
    owner_s, consumer = client(ep2), client(ep1)
    key = "zk2/lab/m/budget.v1/stream/load/c0"
    heard: list[int] = []
    misses: list[int] = []
    try:
        pub = ext.declare_advanced_publisher(
            owner_s, key, cache=ext.CacheConfig(max_samples=1),
            sample_miss_detection=ext.MissDetectionConfig(heartbeat=0.5, sporadic_heartbeat=None))
        n = [0]

        def put() -> None:
            n[0] += 1
            pub.put(str(n[0]).encode())

        time.sleep(0.5)
        for _ in range(5):
            put()
        time.sleep(0.3)
        sub = ext.declare_advanced_subscriber(consumer, key, zenoh.handlers.Callback(
            lambda s: heard.append(int(s.payload.to_bytes()))))
        listener = sub.sample_miss_listener(zenoh.handlers.Callback(lambda m: misses.append(m.nb)))
        while not heard and n[0] < 200:
            put()
            time.sleep(0.05)
        time.sleep(1.5)
        first, misses_2 = (heard[0] if heard else None), list(misses)
        report.check(sec, "step 2: the first sample heard is the sixth or later, and no miss is reported: zenoh-ext "
                          "takes a publisher's first sample heard as its start",
                     first is not None and first >= 6 and not misses_2, f"first heard {first}, misses {misses_2}")
        link.cut()
        time.sleep(0.5)
        for _ in range(3):
            put()
        link.heal()
        count = len(heard)
        t0 = time.monotonic()
        while len(heard) == count and time.monotonic() - t0 < 30:
            put()
            time.sleep(0.05)
        time.sleep(1.5)
        report.check(sec, "step 3: after the link is closed, three put, and healed, a miss is reported: a gap after "
                          "a sample heard is detected",
                     sum(misses) > 0, f"misses {misses}, heard again {len(heard) > count}")
        listener.undeclare()
        sub.undeclare()
        pub.undeclare()
    finally:
        owner_s.close()
        consumer.close()
        link.close()
        r2.close()
        r1.close()


# -- §4–§7 -----------------------------------------------------------------------------

class PyKeeper:
    """zk2py's owner of budget.v1, its writes named as the Rust owner
    example's commands are: (verb, resource, value) -> (accepted, message,
    the key written)."""

    name = "zk2py's owner"

    def __init__(self, endpoint: str, service: str, cardinality: dict[str, int] | None = None):
        from .contract import load_contract
        from .owner import Owner

        self.contract = load_contract(BUDGET)
        self.owner = Owner("lab", service, [self.contract], connect=endpoint,
                           cardinality={"budget.v1": cardinality} if cardinality else None)
        self.owner.start()
        self.base = self.owner.prefix(self.contract)

    def write(self, verb: str, resource: str, value: str) -> tuple[bool, str, str]:
        from .owner import BudgetExceeded

        key = f"{self.base}/{resource.split('/{')[0]}/{value}"
        try:
            if verb == "delete":
                self.owner.delete_state(key)
            elif resource.startswith("state/"):
                self.owner.set_state(key, b"x")
            elif resource.startswith("stream/"):
                self.owner.publish(key, b"x")
            else:
                key = self.owner.occur(key, b"!")
            return True, "accepted", key
        except BudgetExceeded as e:
            return False, str(e), key

    def close(self) -> None:
        self.owner.close()


class RustKeeper:
    """The Rust owner example, driven by its stdin: ``put <iface> <resource>
    <value>`` and ``delete …``, each answered with one line, ``put …``,
    ``deleted …`` or ``refused …``."""

    name = "the Rust owner example"

    def __init__(self, exe: Path, endpoint: str, service: str, cardinality: dict[str, int] | None = None):
        from .live_interop import Owner as RustOwner

        flags = []
        for res, n in (cardinality or {}).items():
            flags += ["--cardinality", f"budget.v1:{res}={n}"]
        self.owner = RustOwner(exe, f"lab/{service}", [BUDGET], connect=endpoint, flags=flags)
        if self.owner.wait_for("ready ", 120) is None:
            raise RuntimeError(f"the owner example did not start: {self.owner.stderr[-2:]}")
        self.base = f"zk2/lab/{service}/budget.v1"

    def write(self, verb: str, resource: str, value: str) -> tuple[bool, str, str]:
        line = self.owner.command(f"{verb} budget.v1 {resource} {value}") or ""
        key = f"{self.base}/{resource.split('/{')[0]}/{value}"
        if line.startswith("put ") or line.startswith("deleted "):
            return True, line, line.split(" ", 1)[1]
        return False, line, key

    def close(self) -> None:
        self.owner.close()


def keeping(report: Report, keepers: list[tuple[str, Any]], endpoint: str) -> None:
    import zenoh

    from . import live

    watcher = live.open_client(endpoint)
    seen: list[str] = []
    sub = watcher.declare_subscriber("zk2/lab/*/budget.v1/**", zenoh.handlers.Callback(
        lambda s: seen.append(str(s.key_expr))))
    time.sleep(0.3)
    try:
        for label, make in keepers:
            def published(key: str) -> bool:
                time.sleep(0.3)
                return any(k == key or k.startswith(key + "/") for k in seen)

            # §4
            sec = f"§4 an owner keeps its bound: a state ({label})"
            k = make("s4")
            k2 = make("s4low", {"state/items/{item}": 2})
            try:
                r1 = [k.write("put", "state/items/{item}", x) for x in "abc"]
                report.check(sec, "step 1: a, b, c accepted, three live members, at the bound",
                             all(ok for ok, _, _ in r1), str([m for _, m, _ in r1]))
                ok, msg, key = k.write("put", "state/items/{item}", "d")
                report.check(sec, "step 2: d refused, nothing published; the refusal names the resource, the bound and "
                                  "its source, the contract's",
                             not ok and not published(key) and "items/{item}" in msg and "3" in msg
                             and "contract" in msg, msg)
                ok, msg, _ = k.write("put", "state/items/{item}", "a")
                report.check(sec, "step 3: a again accepted: a put to a live key adds no member", ok, msg)
                okd, msgd, _ = k.write("delete", "state/items/{item}", "b")
                ok, msg, _ = k.write("put", "state/items/{item}", "d")
                report.check(sec, "step 4: b deleted, then d accepted: a delete frees a member", okd and ok,
                             f"{msgd}; {msg}")
                r5 = [k2.write("put", "state/items/{item}", x) for x in "abc"]
                report.check(sec, "step 5: another owner, its descriptor lowering the bound to 2: a, b accepted, c "
                                  "refused, naming the bound 2 and its source, the descriptor's",
                             r5[0][0] and r5[1][0] and not r5[2][0] and "2" in r5[2][1] and "descriptor" in r5[2][1]
                             and not published(r5[2][2]), r5[2][1])
            finally:
                k.close()
                k2.close()
            # §5
            sec = f"§5 an owner keeps its bound: a stream ({label})"
            k = make("s5")
            try:
                r1 = [k.write("put", "stream/load/{cpu}", c) for c in ("c0", "c1", "c2")]
                report.check(sec, "step 1: c0, c1, c2 accepted", all(ok for ok, _, _ in r1), str([m for _, m, _ in r1]))
                ok, msg, key = k.write("put", "stream/load/{cpu}", "c3")
                report.check(sec, "step 2: c3 refused, nothing published: three members published within the last "
                                  "hour are live", not ok and not published(key) and "3" in msg, msg)
                ok, msg, _ = k.write("put", "stream/load/{cpu}", "c0")
                report.check(sec, "step 3: c0 again accepted: c0 is live", ok, msg)
            finally:
                k.close()
            # §6
            sec = f"§6 an owner keeps its bound: an event ({label})"
            k = make("s6")
            try:
                r1 = [k.write("put", "events/alarms/{src}", x) for x in "ab"]
                report.check(sec, "step 1: occurrences of a, then b, accepted", all(ok for ok, _, _ in r1),
                             str([m for _, m, _ in r1]))
                ok, msg, key = k.write("put", "events/alarms/{src}", "c")
                report.check(sec, "step 2: an occurrence of c refused, nothing published: a and b have occurrences "
                                  "within the retention", not ok and not published(key) and "2" in msg, msg)
                time.sleep(2.1)
                ok, msg, _ = k.write("put", "events/alarms/{src}", "c")
                report.check(sec, "step 3: 2 s later, an occurrence of c accepted: a and b are no longer live", ok, msg)
            finally:
                k.close()
            # §7
            sec = f"§7 an owner keeps its rate ({label})"
            k = make("s7")
            try:
                r1 = [k.write("put", "events/alarms/{src}", "a") for _ in range(3)]
                report.check(sec, "step 1: three occurrences of a accepted", all(ok for ok, _, _ in r1),
                             str([m for _, m, _ in r1]))
                n_seen = len([x for x in seen if "/s7/" in x])
                ok, msg, key = k.write("put", "events/alarms/{src}", "a")
                time.sleep(0.3)
                report.check(sec, "step 2: a fourth occurrence of a refused, nothing published; the refusal names the "
                                  "rate", not ok and len([x for x in seen if "/s7/" in x]) == n_seen
                             and "burst(3/h)" in msg, msg)
                ok, msg, _ = k.write("put", "events/alarms/{src}", "b")
                report.check(sec, "step 3: an occurrence of b accepted: the rate bounds each member", ok, msg)
            finally:
                k.close()
    finally:
        sub.undeclare()
        watcher.close()


def sections4to7(report: Report, exe: Path | None) -> None:
    from .live_interop import _r1

    r1, ep, _ = _r1()
    try:
        keepers: list[tuple[str, Any]] = [("zk2py", lambda svc, card=None: PyKeeper(ep, f"py{svc}", card))]
        if exe is not None and exe.is_file():
            keepers.append(("the Rust owner example",
                            lambda svc, card=None: RustKeeper(exe, ep, f"rust{svc}", card)))
        else:
            report.info("§4-§7", f"{exe} not found: zk2py's owner alone")
        keeping(report, keepers, ep)
    finally:
        r1.close()


SECTIONS = {"1": section1, "2": section2, "3": section3}


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="python -m zk2py.budget_scenarios", description=__doc__.split("\n")[0])
    ap.add_argument("--only", action="append", choices=["1", "2", "3", "4-7"], help="run only these sections")
    ap.add_argument("--owner", type=Path, default=REPO / "target" / "debug" / "examples" / "owner",
                    help="the Rust owner example, for §4-§7")
    args = ap.parse_args(argv)
    report = Report()
    for name, fn in SECTIONS.items():
        if args.only and name not in args.only:
            continue
        try:
            fn(report)
        except Exception as e:  # noqa: BLE001 - a section that cannot run is a failure
            report.check(f"§{name}", "the section ran", False, f"{type(e).__name__}: {e}")
    if not args.only or "4-7" in args.only:
        try:
            sections4to7(report, args.owner)
        except Exception as e:  # noqa: BLE001
            report.check("§4-§7", "the sections ran", False, f"{type(e).__name__}: {e}")
    passed = sum(1 for r in report.rows if r[2])
    failed = len(report.rows) - passed
    print(f"budget scenarios: {passed} passed, {failed} failed")
    for sec, name, ok, detail in report.rows:
        if not ok:
            print(f"FAIL [{sec}] {name}: {detail}")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())

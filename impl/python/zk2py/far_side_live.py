"""``spec/scenarios/constrained.md`` §8 and §9 (core §8.1 and §8.5 at 0.91,
observer §7.5 at 0.23; #988 L4, #609 round 99), run in process with zk2py
alone.

``python -m zk2py.far_side_live [--out FILE]``

**Setup**, as the sections write it: one router of the runner's own, and
zk2py's owner of the reference's ``zenkey/tests/contracts/faced.v1.toml``
(a contract file, read as any contract is) at ``lab/faced``, its client. Its
``slow`` operation recommends ``timeout_ms = 1000`` and replies 1.5 s after
each call; its ``level`` stream, ``freshness.ttl_s = 1``, is put every
100 ms. The face is a ``link.v2`` policy's ``radio``, its
``reply_timeout_ms`` 3000 and its ``arq`` false (true for §9 step 3), read
by :func:`zk2py.far_side.far_side`; every far session runs its
``[transport]``.

**§8.** The caller, a client of the router: step 1 calls setting no
timeout of its own (§5.1: the contract's 1 s); step 2 calls across the face,
setting none either (at least the face's 3 s).

**§9.** Each far client attaches through :class:`DroppingProxy`, a TCP
relay that reads what the far client sends as zenoh's stream links frame it,
each batch after a little-endian 16-bit length, and on request drops the
next batch the far client sends (a keep-alive's byte aside), or the next that
holds given bytes, forwarding the rest: a frame lost below zenoh, its
transport left up.
1. zk2py's consumer (an owner of ``interop/constrained/zk2py_faced_reader.v1``
   whose role is bound to ``lab/faced``) subscribes through
   :meth:`zk2py.owner.Owner.subscribe_role`, hears the stream and
   undeclares; the proxy drops its next batch, and it subscribes again
   across the face (:class:`zk2py.far_side.Redeclaring`).
2. The same for zk2py's tool, :func:`zk2py.echo_tool.watch` over observer
   §7.5's scopes, across the face.
3. **Control:** step 1 across a face whose ``arq`` is true.
4. A consumer that never subscribed: the proxy drops the next batch holding
   the stream's key, its ``DeclareKeyExpr`` and the ``DeclareSubscriber``
   beside it.

Each step records every delivery's instant from the subscription's
declaration, the declarations made again, and the batch the proxy dropped
(its length, and whether it holds the key's text). It exits 0 when every
expectation holds, 1 when one does not, and 2 when a step could not run.
"""

from __future__ import annotations

import argparse
import json
import socket
import sys
import threading
import time
from pathlib import Path
from typing import Any

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[2]
FACED = REPO / "zenkey" / "tests" / "contracts" / "faced.v1.toml"
READER = HERE.parent / "interop" / "constrained" / "zk2py_faced_reader.v1.toml"
ADDRESS = "lab/faced"
IFACE = "faced.v1"
LEVEL = f"zk2/{ADDRESS}/{IFACE}/stream/level"
SLOW = f"zk2/{ADDRESS}/{IFACE}/@op/slow"
#: §8: "Its reply comes back 1.5 s after each call"
REPLY_AFTER_S = 1.5
#: §9: "puts a stream every 100 ms. Its freshness.ttl_s is 1."
PERIOD_S = 0.1


def policy(arq: bool) -> str:
    """The routers' link.v2 face policy for the run: one face, ``radio``,
    ``reply_timeout_ms = 3000``, at zenoh's own lease and keep-alives."""
    return f'''profile  = "link.v2"
system   = "lab"
services = ["faced"]
binding  = "zenoh-1.10.1"

[transport]
lease_ms             = 10000
keep_alive           = 4
wait_before_close_ms = 10000
sequence             = "32bit"

[face.radio]
far              = "ground"
attach           = "client"
select           = {{ link_protocols = ["tcp"] }}
class            = "constrained"
expose           = "link"
interval_s       = 1
max_outage_s     = 60
link_bps         = 1000000
loss_ppm         = 0
arq              = {"true" if arq else "false"}
reply_timeout_ms = 3000
'''


class DroppingProxy:
    """A TCP relay to ``upstream`` that can lose one batch the far client
    sends (constrained.md §9's proxy). :meth:`drop_next` arms it: the next
    batch toward the router, longer than a keep-alive's byte, or the next
    holding ``holding``, is dropped and kept in :attr:`dropped`."""

    def __init__(self, upstream: str) -> None:
        host, port = upstream.removeprefix("tcp/").rsplit(":", 1)
        self.upstream = (host, int(port))
        self.sock = socket.socket()
        self.sock.bind(("127.0.0.1", 0))
        self.sock.listen(4)
        self.endpoint = f"tcp/127.0.0.1:{self.sock.getsockname()[1]}"
        self.lock = threading.Lock()
        self.rule: bytes | None = None
        self.dropped: list[bytes] = []
        self.sent: list[tuple[float, int]] = []
        self.conns: list[socket.socket] = []
        threading.Thread(target=self._accept, daemon=True).start()

    def drop_next(self, holding: bytes = b"") -> None:
        with self.lock:
            self.rule = holding

    def _accept(self) -> None:
        while True:
            try:
                c, _ = self.sock.accept()
            except OSError:
                return
            u = socket.create_connection(self.upstream)
            for s in (c, u):
                s.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
            self.conns += [c, u]
            threading.Thread(target=self._up, args=(c, u), daemon=True).start()
            threading.Thread(target=self._down, args=(u, c), daemon=True).start()

    def _up(self, src: socket.socket, dst: socket.socket) -> None:
        buf = bytearray()
        try:
            while True:
                data = src.recv(65536)
                if not data:
                    break
                buf += data
                while len(buf) >= 2:
                    n = int.from_bytes(buf[:2], "little")
                    if len(buf) < 2 + n:
                        break
                    framed, batch = bytes(buf[:2 + n]), bytes(buf[2:2 + n])
                    del buf[:2 + n]
                    with self.lock:
                        rule = self.rule
                        drop = rule is not None and ((rule == b"" and n > 2) or (rule != b"" and rule in batch))
                        if drop:
                            self.rule = None
                            self.dropped.append(batch)
                        else:
                            self.sent.append((time.monotonic(), n))
                    if not drop:
                        dst.sendall(framed)
        except OSError:
            pass

    def _down(self, src: socket.socket, dst: socket.socket) -> None:
        try:
            while True:
                data = src.recv(65536)
                if not data:
                    break
                dst.sendall(data)
        except OSError:
            pass

    def close(self) -> None:
        for s in [self.sock, *self.conns]:
            try:
                s.close()
            except OSError:
                pass


class Arrivals:
    """Each delivery's instant on the monotonic clock."""

    def __init__(self) -> None:
        self.lock = threading.Lock()
        self.at: list[float] = []

    def take(self, *_: Any) -> None:
        with self.lock:
            self.at.append(time.monotonic())

    def since(self, t0: float) -> list[float]:
        with self.lock:
            return [round(t - t0, 3) for t in self.at if t >= t0]

    def wait(self, n: int = 1, limit: float = 10.0) -> bool:
        end = time.monotonic() + limit
        while time.monotonic() < end:
            with self.lock:
                if len(self.at) >= n:
                    return True
            time.sleep(0.02)
        return False


def run() -> dict[str, Any]:
    from . import echo_tool, face, far_side, live
    from .contract import load_contract
    from .live_interop import _r1
    from .owner import Owner

    faced, reader = load_contract(FACED), load_contract(READER)
    for c in (faced, reader):
        if not c.valid:
            raise RuntimeError(f"{c.path}: {c.codes}")
    level_r = next(r for r in faced.canonical["resources"] if r["template"] == "level")
    slow_r = next(r for r in faced.canonical["resources"] if r["template"] == "slow")
    horizon = far_side.resource_horizon_s(faced, level_r)
    no_arq, with_arq = far_side.far_side(policy(False), "radio"), far_side.far_side(policy(True), "radio")

    def slow(call: Any) -> None:
        time.sleep(REPLY_AFTER_S)
        call.reply(b"done")

    r1, ep, _ = _r1(timestamping=False)
    owner = Owner("lab", "faced", [faced], connect=ep, handlers={"@op/slow": slow})
    owner.start()
    stop = threading.Event()

    def putting() -> None:
        n = 0
        while not stop.wait(PERIOD_S):
            try:
                owner.publish(LEVEL, f"{n}".encode())
            except Exception:  # noqa: BLE001 - the owner closing
                return
            n += 1

    threading.Thread(target=putting, daemon=True).start()
    out: dict[str, Any] = {"horizon_s": horizon, "timeout_ms": slow_r.get("timeout_ms"), "steps": {}}
    try:
        # §8: a reply across the face.
        caller = live.open_client(ep, transport=no_arq.config())
        try:
            for step, across in (("8.1", None), ("8.2", no_arq)):
                t0 = time.monotonic()
                got = live.call(caller, SLOW, b"", timeout=None, recommended_ms=slow_r.get("timeout_ms"),
                                across=across, first=True)
                out["steps"][step] = {
                    "waited_s": far_side.call_timeout_s(None, slow_r.get("timeout_ms"), across),
                    "values": [r.payload.decode() for r in got.replies if r.kind == "value"],
                    "other": [r.kind for r in got.replies if r.kind != "value"],
                    "first_s": None if got.first_s is None else round(got.first_s, 3),
                    "ended_s": round(time.monotonic() - t0, 3)}
                time.sleep(REPLY_AFTER_S + 0.3)
        finally:
            live.close(caller)

        # §9 steps 1 and 3: a consumer whose DeclareSubscriber the proxy drops.
        def consumer_step(step: str, across: far_side.FarSide, primed: bool, holding: bytes = b"",
                          watch_s: float = 4.0) -> None:
            proxy = DroppingProxy(ep)
            consumer = None
            try:
                consumer = Owner("ground", f"reader{step.replace('.', '')}", [reader], connect=proxy.endpoint,
                                 bindings={"src": [ADDRESS]})
                consumer.start()
                row: dict[str, Any] = {"arq": across.arq}
                if primed:
                    first = Arrivals()
                    subs = consumer.subscribe_role("src", IFACE, "stream/level", first.take)
                    row["primed"] = first.wait(1)
                    for s in subs:
                        s.undeclare()
                    time.sleep(0.5)
                heard = Arrivals()
                proxy.drop_next(holding)
                t0 = time.monotonic()
                subs = consumer.subscribe_role("src", IFACE, "stream/level", heard.take, across=across,
                                               horizon_s=horizon)
                time.sleep(watch_s)
                row["arrivals"] = heard.since(t0)
                row["redeclared_s"] = [round(t - t0, 3) for s in subs
                                       for t in getattr(s, "redeclared", [])]
                row["dropped"] = [{"bytes": len(b), "holds_key": b"faced.v1/stream/level" in b}
                                  for b in proxy.dropped]
                for s in subs:
                    s.undeclare()
                out["steps"][step] = row
            finally:
                if consumer is not None:
                    consumer.close()
                proxy.close()

        consumer_step("9.1", no_arq, primed=True)

        # §9 step 2: the tool's watch.
        proxy = DroppingProxy(ep)
        try:
            session = live.open_client(proxy.endpoint, transport=no_arq.config())
            try:
                scopes = [s for s in face.scopes([faced], [ADDRESS]) if s.endswith("/stream/level")]
                horizons = far_side.scope_horizons([faced], [ADDRESS])
                first = Arrivals()
                echo_tool.watch(session, scopes, 1.5, 1, each=first.take)
                time.sleep(0.5)
                heard, again = Arrivals(), []
                proxy.drop_next()
                t0 = time.monotonic()
                echo_tool.watch(session, scopes, 4.0, each=heard.take, across=no_arq, horizons=horizons,
                                redeclared=again)
                out["steps"]["9.2"] = {"arq": False, "primed": bool(first.since(0.0)),
                                       "scopes": scopes, "horizons": {s: horizons.get(s) for s in scopes},
                                       "arrivals": heard.since(t0),
                                       "redeclared_s": [round(t - t0, 3) for _, t in again],
                                       "dropped": [{"bytes": len(b), "holds_key": b"faced.v1/stream/level" in b}
                                                   for b in proxy.dropped]}
            finally:
                live.close(session)
        finally:
            proxy.close()

        consumer_step("9.3", with_arq, primed=True, watch_s=3.5)
        consumer_step("9.4", no_arq, primed=False, holding=b"faced.v1/stream/level", watch_s=3.5)
    finally:
        stop.set()
        owner.close()
        r1.close()
    return out


def judge(report: dict[str, Any]) -> list[tuple[str, bool, str]]:
    """Each step's expectation, as constrained.md §8 and §9 write it."""
    out: list[tuple[str, bool, str]] = []
    st = report["steps"]
    h = report["horizon_s"]

    def add(name: str, ok: bool, row: Any) -> None:
        out.append((name, bool(ok), json.dumps(row)))

    s1 = st.get("8.1", {})
    add("§8 step 1: setting no timeout, the caller waits the contract's 1 s, and no reply comes back",
        s1.get("waited_s") == 1.0 and not s1.get("values"), s1)
    s2 = st.get("8.2", {})
    add("§8 step 2: across the face it waits at least the face's 3 s, and the value comes back after 1.5 s",
        s2.get("waited_s") == 3.0 and s2.get("values") == ["done"] and s2.get("first_s") is not None
        and REPLY_AFTER_S - 0.1 <= s2["first_s"] < 3.0, s2)

    def silent_then_heard(name: str, row: dict[str, Any]) -> None:
        a, again = row.get("arrivals", []), row.get("redeclared_s", [])
        dropped = row.get("dropped", [])
        add(f"{name}: one batch dropped, the far client's declaration", len(dropped) == 1, dropped)
        add(f"{name}: nothing delivered for two horizons ({2 * h:g} s)",
            not any(t < 2 * h for t in a), a[:5])
        add(f"{name}: declared again once, at two horizons of silence",
            len(again) == 1 and 2 * h <= again[0] < 2 * h + 0.5, again)
        add(f"{name}: the stream delivered from then on",
            bool(again) and len([t for t in a if t >= again[0]]) >= 10, len(a))

    for step in ("9.1", "9.2"):
        row = st.get(step, {})
        if not row:
            add(f"§9 step {step[2:]}", False, "not run")
            continue
        add(f"§9 step {step[2:]}: the far client heard the stream before (its prefix declared)", row.get("primed"),
            row.get("primed"))
        silent_then_heard(f"§9 step {step[2:]} ({'consumer' if step == '9.1' else 'tool'})", row)
    row = st.get("9.3", {})
    add("§9 step 3 (control, arq = true): one batch dropped", len(row.get("dropped", [])) == 1, row.get("dropped"))
    add("§9 step 3: nothing delivered for three horizons, never declared again",
        not row.get("arrivals") and not row.get("redeclared_s"), row)
    row = st.get("9.4", {})
    add("§9 step 4: the batch holding the stream's key dropped (its DeclareKeyExpr)",
        [d["holds_key"] for d in row.get("dropped", [])] == [True], row.get("dropped"))
    add("§9 step 4: nothing delivered for three horizons, though declared again once on the scope the router "
        "never learned", not row.get("arrivals") and len(row.get("redeclared_s", [])) == 1, row)
    return out


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="python -m zk2py.far_side_live", description=__doc__.split("\n")[0])
    ap.add_argument("--out", default=None, help="write the report as JSON here")
    args = ap.parse_args(argv)
    try:
        report = run()
    except Exception as e:  # noqa: BLE001 - a step that could not run is exit 2
        print(f"far_side_live: could not run: {type(e).__name__}: {e}", file=sys.stderr)
        return 2
    checks = judge(report)
    report["checks"] = [{"check": n, "ok": ok, "got": g} for n, ok, g in checks]
    if args.out:
        Path(args.out).write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    for n, ok, g in checks:
        print(f"{'PASS' if ok else 'FAIL'} {n}" + ("" if ok else f": {g}"))
    passed = sum(1 for _, ok, _ in checks if ok)
    print(f"far_side_live: {passed}/{len(checks)}")
    return 0 if passed == len(checks) else 1


if __name__ == "__main__":
    sys.exit(main())

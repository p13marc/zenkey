"""zk2py's own Zenoh facts, each run on the zenoh-python it is installed
with (#609, round 122).

Core 0.114 specifies zk2 against Zenoh 1.10.2, and leaves zenoh-python's
facts to that binding: "zenoh-pico's and zenoh-python's facts are those
bindings'. zk2py's move to 1.10.2 is its own round" (`spec/CHANGELOG.md`,
0.114). Each fact here is one zk2py's README or core Appendix B names on
zenoh-python 1.10.1: its API, and what it puts on the wire. Each is run,
never read from zenoh's source, and the run says whether it holds.

```bash
PYTHONPATH=impl/python timeout 600 taskset -c 4-5 \\
  target/py-venv/bin/python -m zk2py.zenoh_facts [--only NAME]... [--json PATH]
```

Every router is zenoh-python's, on a loopback port picked by binding port
0, in this process but for the receive threads' (``proc shared``). A fact
zk2py relies on holds or not; a fact marked ``measured`` is one of
Appendix B's, or one that races, and is reported, never judged. Exit 0
when every relied-on fact holds, 1 when one does not.
"""

from __future__ import annotations

import argparse
import hashlib
import inspect
import json
import socket
import struct
import sys
import threading
import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Callable

import zenoh

from .owner import free_loopback_port

#: The sha256 of each of zenoh-python 1.10.1's stubs, as installed from its
#: abi3 wheel: the API zk2py was written against.
STUBS_1_10_1: dict[str, str] = {
    "__init__.pyi": "249f36167df67d28a2bd4ea099aa004a5db8b6fbf8e9d2ea0c5e5345e4b7e387",
    "ext.pyi": "5500fb6cba855531995a2462dfd56bfafa1170ce57d19cb069452222cb87a1a7",
    "handlers.pyi": "b57c16ef9ceb40afe31751ec31b97331247771e4fd03434fff57b7cc3858a0f5",
    "shm.pyi": "436f1903fb9b41637aaed26a46fe6e2c45b0007c2ad7be9aa5a3e7e530dd153c",
}


def _stub_digests() -> dict[str, str]:
    root = Path(zenoh.__file__).parent
    return {p.name: hashlib.sha256(p.read_bytes()).hexdigest() for p in sorted(root.glob("*.pyi"))}


def _version() -> str:
    from importlib.metadata import version

    return version("eclipse-zenoh")


# -- the bus ------------------------------------------------------------------------


def router(*, timestamping: bool = True) -> tuple[Any, str]:
    port = free_loopback_port()
    conf = zenoh.Config()
    conf.insert_json5("mode", json.dumps("router"))
    conf.insert_json5("listen/endpoints", json.dumps([f"tcp/127.0.0.1:{port}"]))
    conf.insert_json5("scouting/multicast/enabled", "false")
    conf.insert_json5("timestamping/enabled", json.dumps(timestamping))
    return zenoh.open(conf), f"tcp/127.0.0.1:{port}"


def client(endpoint: str, *, timestamping: bool | None = None, interests_ms: int | None = None) -> Any:
    conf = zenoh.Config()
    conf.insert_json5("mode", json.dumps("client"))
    conf.insert_json5("connect/endpoints", json.dumps([endpoint]))
    conf.insert_json5("scouting/multicast/enabled", "false")
    if timestamping is not None:
        conf.insert_json5("timestamping/enabled", json.dumps(timestamping))
    if interests_ms is not None:
        conf.insert_json5("routing/interests/timeout", json.dumps(interests_ms))
    return zenoh.open(conf)


class Relay:
    """A TCP relay to a router. It can hold the router-to-client direction
    (the bytes wait, and flow when released), cut both sockets, and it
    counts the client-to-router batches by the stream link's 2-byte length
    prefix."""

    def __init__(self, upstream: str) -> None:
        host, port = upstream.removeprefix("tcp/").rsplit(":", 1)
        self.upstream = (host, int(port))
        self.flowing = threading.Event()
        self.flowing.set()
        self.sock = socket.socket()
        self.sock.bind(("127.0.0.1", 0))
        self.sock.listen(8)
        self.endpoint = f"tcp/127.0.0.1:{self.sock.getsockname()[1]}"
        self.conns: list[socket.socket] = []
        self.up_batches: list[tuple[float, int]] = []
        self.closed_at: float | None = None
        threading.Thread(target=self._accept, daemon=True).start()

    def _accept(self) -> None:
        while True:
            try:
                c, _ = self.sock.accept()
            except OSError:
                return
            try:
                u = socket.create_connection(self.upstream)
            except OSError:
                c.close()  # the router is gone: refuse this one, keep accepting
                continue
            self.conns += [c, u]
            threading.Thread(target=self._pump, args=(c, u, None, True), daemon=True).start()
            threading.Thread(target=self._pump, args=(u, c, self.flowing, False), daemon=True).start()

    def _pump(self, src: socket.socket, dst: socket.socket, gate: threading.Event | None, up: bool) -> None:
        buf = b""
        try:
            while True:
                data = src.recv(65536)
                if not data:
                    break
                if up:
                    buf += data
                    while len(buf) >= 2:
                        n = struct.unpack_from("<H", buf)[0]
                        if len(buf) < 2 + n:
                            break
                        self.up_batches.append((time.monotonic(), n))
                        buf = buf[2 + n:]
                if gate is not None:
                    gate.wait()
                dst.sendall(data)
        except OSError:
            pass
        finally:
            for s in (src, dst):
                try:
                    s.close()
                except OSError:
                    pass

    def hold(self) -> None:
        self.flowing.clear()

    def release(self) -> None:
        self.flowing.set()

    def cut(self) -> None:
        self.closed_at = time.monotonic()
        for c in self.conns:
            try:
                c.shutdown(socket.SHUT_RDWR)
            except OSError:
                pass
            try:
                c.close()
            except OSError:
                pass
        self.conns = []

    def close(self) -> None:
        self.release()
        self.cut()
        self.sock.close()


@dataclass
class Fact:
    name: str
    source: str
    holds: bool
    detail: str
    data: dict[str, Any] = field(default_factory=dict)
    #: "fact", one zk2py relies on, which holds or not; "measured", one of
    #: core Appendix B's zenoh facts that zk2py measures and never judges
    kind: str = "fact"


def _live_get(session: Any, selector: str, timeout_s: float) -> tuple[list[str], list[tuple[str, str]], float]:
    """A liveliness GET through a callback: the keys, the error replies
    (payload, encoding), and the seconds until the drop function ran."""
    keys: list[str] = []
    errs: list[tuple[str, str]] = []
    done = threading.Event()

    def on(reply: Any) -> None:
        if reply.ok is not None:
            keys.append(str(reply.ok.key_expr))
        else:
            errs.append((reply.err.payload.to_bytes().decode("utf-8", "replace"), str(reply.err.encoding)))

    t0 = time.monotonic()
    session.liveliness().get(selector, zenoh.handlers.Callback(on, done.set), timeout=timeout_s)
    done.wait(timeout_s + 15)
    return keys, errs, time.monotonic() - t0


# -- the facts ------------------------------------------------------------------------


def fact_stubs() -> Fact:
    got = _stub_digests()
    same = bool(STUBS_1_10_1) and got == STUBS_1_10_1
    differ = sorted(k for k in set(got) | set(STUBS_1_10_1) if got.get(k) != STUBS_1_10_1.get(k))
    return Fact("stubs", "README \"Dependencies\": the API zk2py was written against, zenoh-python 1.10.1",
                same, "the four stubs are 1.10.1's byte for byte" if same else f"differ: {differ}", {"sha256": got})


def fact_handlers() -> Fact:
    names = sorted(n for n in dir(zenoh.handlers) if not n.startswith("_"))
    unbounded = [n for n in names if "unbounded" in n.lower()]
    holds = not unbounded and "Callback" in names
    return Fact("handlers", "core Appendix B \"One binding's facts\"; README §8.1's handler rule: no unbounded "
                "handler, so a callback", holds, f"zenoh.handlers: {names}", {"names": names})


def fact_unstable() -> Fact:
    stub = (Path(zenoh.__file__).parent / "__init__.pyi").read_text(encoding="utf-8")
    marked = {}
    for name, pat in (("Reliability", "@_unstable\nclass Reliability"),
                      ("Reply.replier_id", "@property\n    @_unstable\n    def replier_id"),
                      ("declare_transport_events_listener", "def declare_transport_events_listener")):
        marked[name] = pat in stub
    put = inspect.signature(zenoh.Session.put) if hasattr(zenoh.Session, "put") else None
    put_stub = stub.split("    def put(\n", 2)
    session_put = put_stub[2].split(")", 1)[0] if len(put_stub) > 2 else ""
    no_rel = "reliability" not in session_put
    holds = marked["Reliability"] and marked["Reply.replier_id"] and no_rel
    return Fact("unstable", "core Appendix B: `Reliability` unstable, `Session.put` takes none; README: "
                "`Reply.replier_id` marked unstable in the stub", holds,
                f"marked unstable {marked}; Session.put takes reliability: {not no_rel}; {put}", marked)


def fact_publisher_defaults() -> Fact:
    r, ep = router()
    s = client(ep)
    try:
        pub = s.declare_publisher("zk2py/facts/pub")
        got = {"reliability": str(pub.reliability), "congestion_control": str(pub.congestion_control),
               "priority": str(pub.priority)}
        pub.undeclare()
    finally:
        s.close()
        r.close()
    holds = "RELIABLE" in got["reliability"] and "DROP" in got["congestion_control"] and "DATA" == \
        got["priority"].rsplit(".", 1)[-1]
    return Fact("publisher_defaults", "core Appendix B: a publisher declared without a reliability reads back "
                "reliable, congestion `drop`, priority `data`", holds, json.dumps(got), got)


def fact_no_config_accessor() -> Fact:
    s = zenoh.open(_peer_conf())
    try:
        attrs = [a for a in ("config", "get_config") if hasattr(s, a)]
    finally:
        s.close()
    return Fact("no_config_accessor", "core Appendix B: zenoh-python gives an open session no accessor for its "
                "interests timeout; README: zenoh-python reads no configuration back", not attrs,
                f"Session's config accessors: {attrs or 'none'}")


def _peer_conf() -> Any:
    conf = zenoh.Config()
    conf.insert_json5("mode", json.dumps("peer"))
    conf.insert_json5("scouting/multicast/enabled", "false")
    conf.insert_json5("listen/endpoints", json.dumps([]))
    return conf


def fact_alone() -> Fact:
    """A session with no transport: every liveliness GET is answered at
    once with the final reply alone."""
    s = zenoh.open(_peer_conf())
    try:
        times = []
        for _ in range(5):
            keys, errs, dt = _live_get(s, "zk2py/facts/**", 5.0)
            times.append((len(keys), len(errs), round(dt * 1000, 2)))
        links = len(s.info.links())
    finally:
        s.close()
    holds = links == 0 and all(k == 0 and e == 0 and ms < 500 for k, e, ms in times)
    return Fact("alone", "core Appendix B, presence.md §7: a session with no transport has every liveliness GET "
                "answered at once with the final reply alone; README: `Session.info.links()`", holds,
                f"links {links}; (tokens, errors, ms): {times}")


def _replier_id() -> Fact:
    r, ep = router()
    a, b = client(ep), client(ep)
    try:
        zid = str(a.info.zid())
        q = a.declare_queryable("zk2py/facts/q", lambda query: query.reply("zk2py/facts/q", b"v"), complete=True)
        time.sleep(0.3)
        replies = list(b.get("zk2py/facts/q", timeout=3.0))
        ids = [str(rep.replier_id.zid) if rep.replier_id is not None else None for rep in replies]
        q.undeclare()
    finally:
        a.close()
        b.close()
        r.close()
    return Fact("replier_id", "README (0.12): `Reply.replier_id`, present at run time, names the session that "
                "answered", ids == [zid], f"replier ids {ids}, the queryable's session {zid}")


def fact_callback_get() -> Fact:
    """A callback liveliness GET through a router: its drop function marks
    completion, and a finished read carries no error reply."""
    r, ep = router()
    a, b = client(ep), client(ep)
    try:
        toks = [a.liveliness().declare_token(f"zk2py/facts/t/{i}") for i in range(20)]
        time.sleep(0.3)
        keys, errs, dt = _live_get(b, "zk2py/facts/t/**", 5.0)
        for t in toks:
            t.undeclare()
    finally:
        a.close()
        b.close()
        r.close()
    holds = len(keys) == 20 and not errs and dt < 4.0
    return Fact("callback_get", "README §8.1: a liveliness GET with a `Callback`, its drop function marking "
                "completion; core Appendix B: one the routers finished ends with no error reply", holds,
                f"{len(keys)} of 20 tokens, errors {errs}, {dt * 1000:.1f} ms")


def fact_get_timeout() -> Fact:
    """A liveliness GET through a link held back ends at its timeout with
    the error reply `Timeout` under `zenoh/string`."""
    r, ep = router()
    a = client(ep)
    relay = Relay(ep)
    b = client(relay.endpoint)
    try:
        tok = a.liveliness().declare_token("zk2py/facts/held")
        time.sleep(0.3)
        relay.hold()
        keys, errs, dt = _live_get(b, "zk2py/facts/**", 1.0)
        relay.release()
        tok.undeclare()
    finally:
        b.close()
        relay.close()
        a.close()
        r.close()
    holds = errs == [("Timeout", "zenoh/string")] and not keys
    return Fact("get_timeout", "core Appendix B, README 0.8: a liveliness GET that reaches its timeout ends with "
                "the error reply `Timeout` (encoding `zenoh/string`)", holds,
                f"tokens {keys}, errors {errs}, {dt:.2f} s at a 1 s timeout")


def fact_query_timeout(trials: int = 12) -> Fact:
    """A call no queryable answers in time: the querier's own `Timeout`
    under `zenoh/string`, the router's under `zenoh/bytes`, or both."""
    r, ep = router()
    a, b = client(ep), client(ep)
    per_get: list[list[tuple[str, str]]] = []
    try:
        held: list[Any] = []
        q = a.declare_queryable("zk2py/facts/slow", lambda query: held.append(query), complete=True)
        time.sleep(0.3)
        for i in range(trials):
            got = []
            for rep in b.get("zk2py/facts/slow", timeout=1.0 + i % 2):
                got.append(("value", "") if rep.err is None else
                           (rep.err.payload.to_bytes().decode("utf-8", "replace"), str(rep.err.encoding)))
            per_get.append(got)
        held.clear()
        q.undeclare()
    finally:
        a.close()
        b.close()
        r.close()
    own, routers = ("Timeout", "zenoh/string"), ("Timeout", "zenoh/bytes")
    shapes: dict[str, int] = {}
    for g in per_get:
        k = "+".join("own" if x == own else "router's" if x == routers else repr(x) for x in g) or "none"
        shapes[k] = shapes.get(k, 0) + 1
    # Measured, never judged: the router's timer and the querier's race, and
    # either is silence (core §5.2). Round 122: the router's in 0 to 2 of 12
    # on 1.10.1 and 1 to 5 of 12 on 1.10.2, once both on one GET.
    return Fact("query_timeout", "README round 101: a call left unanswered: zenoh-python's caller drew its own "
                "`Timeout` under `zenoh/string` 6 times of 6, at 1 s and 2 s", shapes == {"own": trials},
                f"{trials} GETs at 1 s and 2 s, the error replies each drew: {shapes}", {"shapes": shapes}, "measured")


def fact_starved() -> Fact:
    """F-47: a bounded liveliness subscriber nobody drains starves a
    callback GET on its session (1.10.1: 257 of 2,002, at the timeout)."""
    r, ep = router()
    a, b = client(ep), client(ep)
    try:
        toks = [a.liveliness().declare_token(f"zk2py/facts/s/{i}") for i in range(2002)]
        time.sleep(1.0)
        sub = b.liveliness().declare_subscriber("zk2py/facts/s/**", history=True)  # the default FIFO, undrained
        time.sleep(0.5)
        keys, errs, dt = _live_get(b, "zk2py/facts/s/**", 3.0)
        sub.undeclare()
        for t in toks:
            t.undeclare()
    finally:
        a.close()
        b.close()
        r.close()
    starved = len(keys) < 2002
    return Fact("starved", "F-47, core Appendix B (observer §3.1): with zenoh-python 1.10.1 a bounded subscriber "
                "nobody drained starved a callback GET on its session: 257 of 2,002 tokens, at its timeout, and "
                "said nothing", starved, f"{len(keys)} of 2,002, errors {errs}, {dt:.2f} s at a 3 s timeout",
                {"tokens": len(keys), "errors": errs, "s": round(dt, 3)})


def fact_history_race(trials: int = 60) -> Fact:
    """A GET right after declaring a history liveliness subscriber on the
    same session (1.10.1: the token held in 7 of 120)."""
    held = 0
    r, ep = router()
    a = client(ep)
    try:
        tok = a.liveliness().declare_token("zk2py/facts/race")
        time.sleep(0.3)
        for _ in range(trials):
            b = client(ep)
            try:
                sub = b.liveliness().declare_subscriber("zk2py/facts/race", zenoh.handlers.Callback(lambda s: None),
                                                        history=True)
                keys, _errs, _dt = _live_get(b, "zk2py/facts/race", 3.0)
                held += 1 if keys else 0
                sub.undeclare()
            finally:
                b.close()
        tok.undeclare()
    finally:
        a.close()
        r.close()
    return Fact("history_race", "core Appendix B: a GET right after a history subscriber on the same session missed "
                "nearly every time; zenoh-python 1.10.1 held the token in 7 of 120", held < trials,
                f"held in {held} of {trials}", {"held": held, "trials": trials})


def fact_attachment() -> Fact:
    r, ep = router()
    a, b = client(ep), client(ep)
    got: list[tuple[str, Any]] = []
    try:
        sub = b.declare_subscriber("zk2py/facts/att",
                                   lambda s: got.append((str(s.kind), None if s.attachment is None
                                                         else s.attachment.to_bytes())))
        time.sleep(0.3)
        a.put("zk2py/facts/att", b"v", attachment=b"")
        a.put("zk2py/facts/att", b"v")
        a.delete("zk2py/facts/att", attachment=b"")
        a.delete("zk2py/facts/att")
        time.sleep(0.5)
        sub.undeclare()
    finally:
        a.close()
        b.close()
        r.close()
    want = [("SampleKind.PUT", b""), ("SampleKind.PUT", None), ("SampleKind.DELETE", b""), ("SampleKind.DELETE", None)]
    return Fact("attachment", "core Appendix B, measured by the second implementation: a receiver reads `b\"\"` "
                "for an empty attachment and `None` for none, on a put and on a delete", got == want, repr(got))


def fact_client_hlc() -> Fact:
    """A client whose HLC is on receives an unstamped owner's puts, through
    a router that stamps nothing, stamped with its own zid (1.10.1: 20 of
    20)."""
    r, ep = router(timestamping=False)
    owner, reader = client(ep, timestamping=False), client(ep, timestamping=True)
    stamps: list[str | None] = []
    try:
        zid = str(reader.info.zid())
        sub = reader.declare_subscriber("zk2py/facts/hlc", lambda s: stamps.append(
            None if s.timestamp is None else str(s.timestamp.get_id())))
        time.sleep(0.3)
        for i in range(20):
            owner.put("zk2py/facts/hlc", str(i).encode())
        time.sleep(0.6)
        sub.undeclare()
    finally:
        owner.close()
        reader.close()
        r.close()
    own = sum(1 for x in stamps if x == zid)
    return Fact("client_hlc", "core Appendix B, measured by the second implementation: a client whose HLC is on "
                "received 20 of 20 samples of an unstamped owner, through a router that stamps nothing, stamped "
                "with its own zid", own == 20 == len(stamps), f"{own} of {len(stamps)} stamped with its zid")


def fact_ntp64() -> Fact:
    ts = None
    s = zenoh.open(_peer_conf())
    try:
        ts = s.new_timestamp()
    finally:
        s.close()
    n = ts.get_time_as_ntp64()
    a = zenoh.NTP64(n.as_secs(), n.subsec_nanos())
    b = zenoh.NTP64(n.as_secs(), n.subsec_nanos() + 1) if n.subsec_nanos() < 999_999_999 else None
    holds = b is not None and b > a and b.as_nanos() - a.as_nanos() == 1 and \
        zenoh.Timestamp(a, ts.get_id()) <= zenoh.Timestamp(b, ts.get_id())
    return Fact("ntp64", "core Appendix B: zenoh-python builds an NTP64 from seconds and nanoseconds, so its "
                "smallest step above a timestamp is 1 ns", holds,
                f"NTP64(seconds, nanos); a step of 1 ns: {b is not None and b.as_nanos() - a.as_nanos()} ns")


def fact_history_keyword() -> Fact:
    stub = (Path(zenoh.__file__).parent / "__init__.pyi").read_text(encoding="utf-8")
    part = stub.split("class Liveliness:", 1)[1].split("def declare_subscriber(", 1)[1].split(")", 1)[0]
    kw_only = "*," in part and part.index("*,") < part.index("history")
    return Fact("history_keyword", "README round 99's mend: `history=` is zenoh-python 1.10.1's keyword-only "
                "argument", kw_only, " ".join(part.split())[:160])


def _one_close() -> tuple[int, float | None, list[str]]:
    """One cut: the withdrawals a watch session heard, how long before
    (positive) or after (negative) the last of them its transport events
    listener heard the close, in ms, and the order of what it heard."""
    r, ep = router()
    a = client(ep)
    relay = Relay(ep)
    w = client(relay.endpoint)
    events: list[tuple[float, str]] = []
    try:
        toks = [a.liveliness().declare_token(f"zk2py/facts/wd/{i}") for i in range(3)]
        time.sleep(0.3)
        sub = w.liveliness().declare_subscriber(
            "zk2py/facts/wd/**", zenoh.handlers.Callback(lambda s: events.append((time.monotonic(), str(s.kind)))),
            history=True)
        lst = w.info.declare_transport_events_listener(
            zenoh.handlers.Callback(lambda e: events.append((time.monotonic(), "transport " + str(e.kind)))))
        time.sleep(0.5)
        events.clear()
        relay.cut()
        time.sleep(1.0)
        lst.undeclare()
        sub.undeclare()
        for t in toks:
            t.undeclare()
    finally:
        w.close()
        relay.close()
        a.close()
        r.close()
    dels = [t for t, k in events if k == "SampleKind.DELETE"]
    close = [t for t, k in events if k.startswith("transport") and k.endswith("DELETE")]
    lead = round((close[0] - max(dels)) * 1000, 3) if dels and close else None
    return len(dels), lead, ["close" if k.startswith("transport") else "withdrawal" for _, k in events]


def fact_withdrawals_first(trials: int = 10) -> Fact:
    """A close's withdrawals come before its transport event (zk2py on
    1.10.1: 0.15-0.5 ms)."""
    runs = [_one_close() for _ in range(trials)]
    before = [lead for n, lead, _ in runs if n == 3 and lead is not None and lead >= 0]
    after = [lead for n, lead, _ in runs if n == 3 and lead is not None and lead < 0]
    other = [(n, lead) for n, lead, _ in runs if n != 3 or lead is None]
    # Measured, never judged: the subscriber's callback and the listener's
    # run on zenoh's threads, and each takes the GIL before it reads the
    # clock, so the order a Python callback sees is not zenoh's (round 122:
    # 8 to 10 of 10 on each release, an inversion 0.02 to 1.6 ms).
    return Fact("withdrawals_first", "core Appendix B \"A close's withdrawals come before its transport event\": "
                "on zenoh-python 1.10.1, zk2py measured 0.15-0.5 ms", len(before) == trials,
                f"{len(before)} of {trials} cuts told the three withdrawals first, the close "
                f"{min(before, default=None)}-{max(before, default=None)} ms after the last; {len(after)} told the "
                f"close first, by {sorted(after)} ms; {len(other)} other {other}",
                {"before_ms": before, "after_ms": after, "other": other}, "measured")


def fact_wire() -> Fact:
    """What a put adds on the wire, counted in its batch: a timestamp, a
    predefined Encoding, an attachment."""
    r, ep = router()
    relay = Relay(ep)
    s = client(relay.endpoint)
    rd = client(ep)
    out: dict[str, int] = {}
    try:
        sub = rd.declare_subscriber("zk2py/facts/wire", lambda x: None)
        time.sleep(0.5)
        zid = str(s.info.zid())
        for name, kw in (("plain", {}), ("stamped", {"timestamp": s.new_timestamp()}),
                         ("json", {"encoding": zenoh.Encoding.APPLICATION_JSON}),
                         ("bytes", {"encoding": zenoh.Encoding.ZENOH_BYTES}),
                         ("attachment2", {"attachment": b"ab"}), ("attachment0", {"attachment": b""})):
            time.sleep(0.2)
            n0 = len(relay.up_batches)
            s.put("zk2py/facts/wire", b"x", **kw)
            time.sleep(0.3)
            sizes = [n for _, n in relay.up_batches[n0:] if n > 1]
            out[name] = sizes[0] if len(sizes) == 1 else -1
        sub.undeclare()
    finally:
        s.close()
        rd.close()
        relay.close()
        r.close()
    zid_bytes = (len(zid) + 1) // 2
    d = {k: out[k] - out["plain"] for k in out}
    want = {"plain": 0, "stamped": 9 + 1 + zid_bytes, "json": 1, "bytes": 0, "attachment2": 4, "attachment0": 2}
    return Fact("wire", "core Appendix B: a stamp is its 9-byte time, a length and its id's significant bytes "
                "(26 with a 16-byte zid); a put leaves out `zenoh/bytes`; a predefined Encoding is 1 byte; an "
                "attachment under 128 bytes costs 2 over its own", d == want,
                f"batch bytes {out}; over the plain put {d}; want {want} (zid {zid_bytes} bytes)", {"bytes": out})


def fact_null_config() -> Fact:
    """A configuration with a `null` at each of link.v2 §2.19's seven keys
    is refused at load, and one at `routing/interests/timeout` loads."""
    keys = ["transport/link/tx/lease", "transport/link/tx/keep_alive",
            "transport/link/tx/queue/congestion_control/block/wait_before_close",
            "transport/link/tx/sequence_number_resolution", "transport/link/tx/batch_size",
            "transport/unicast/open_timeout", "transport/unicast/accept_timeout", "routing/interests/timeout"]
    got = {}
    for k in keys:
        doc: dict[str, Any] = {}
        cur = doc
        parts = k.split("/")
        for p in parts[:-1]:
            cur = cur.setdefault(p, {})
        cur[parts[-1]] = None
        try:
            zenoh.Config.from_json5(json.dumps(doc))
            got[k] = "loaded"
        except Exception:  # noqa: BLE001 - zenoh raises its own ZError
            got[k] = "refused"
    want = {k: ("loaded" if k == "routing/interests/timeout" else "refused") for k in keys}
    return Fact("null_config", "link.v2 §2.19 as zk2py measured it (round 112): zenoh-python refuses the file "
                "at each of the seven and loads it at `routing/interests/timeout`", got == want, json.dumps(got))


def fact_receive_threads(trials: int = 6) -> Fact:
    """Core Appendix B: "With two routers and a reader whose callback
    blocks in one process, 3 of 6 runs lost 674-970 of 1,000 reliable,
    `block` samples. The routers' transport closes stalled, and no token was
    withdrawn while the runs lasted. That was 1.10.1." The owner is a
    process of its own, a client of R2, as budget.md §1's."""
    from .budget_scenarios import Proc, _free_port

    key, token = "zk2/lab/m/budget.v1/stream/load/c0", "zk2/lab/m/@zk/instance/0123456789abcdef"
    runs: list[dict[str, Any]] = []
    for _ in range(trials):
        l1, l2 = f"tcp/127.0.0.1:{_free_port()}", f"tcp/127.0.0.1:{_free_port()}"
        shared = Proc("shared", "--listen", l1, "--listen2", l2, "--key", key, "--token", token)
        owner = None
        try:
            owner = Proc("publish", "--connect", l2, "--key", key, "--token", token)
            heard = shared.line() == "token"
            owner.say("go")
            res_line = shared.line()
            res = json.loads(res_line.split(" ", 1)[1]) if res_line.startswith("result ") else {"line": res_line}
            res["token"] = heard
            runs.append(res)
        finally:
            if owner is not None:
                owner.close()
            shared.close()
    lost = [1000 - r.get("got", 0) for r in runs]
    with_loss = [x for x in lost if x > 0]
    withdrawn = sum(1 for r in runs if r.get("withdrawn"))
    closes = [(r.get("reader_closes"), r.get("r1_closes")) for r in runs]
    return Fact("receive_threads", "core Appendix B (\"A process's receive threads\"): on 1.10.1, 3 of 6 runs "
                "lost 674-970 of 1,000, the routers' closes stalled and no token was withdrawn; on 1.10.2 not "
                "measured", True,
                f"{len(with_loss)} of {trials} runs lost samples ({sorted(with_loss)}); withdrawals heard in "
                f"{withdrawn}; transport closes (reader, R1), ms from the block: {closes}",
                {"runs": runs}, "measured")


def _best_matching(complete_first: bool, trials: int) -> dict[str, int]:
    r, ep = router()
    a, b, c = client(ep), client(ep), client(ep)
    seen: dict[str, int] = {}
    try:
        order = (True, False) if complete_first else (False, True)
        qs = [a.declare_queryable("zk2py/facts/bm", (lambda tag: lambda q: q.reply("zk2py/facts/bm", tag))(
            b"a" if done else b"a-incomplete"), complete=done) for done in order]
        qs.append(b.declare_queryable("zk2py/facts/bm", lambda q: q.reply("zk2py/facts/bm", b"b"), complete=True))
        time.sleep(0.5)
        for _ in range(trials):
            got = sorted(rep.ok.payload.to_bytes().decode() for rep in
                         c.get("zk2py/facts/bm", target=zenoh.QueryTarget.BEST_MATCHING,
                               consolidation=zenoh.ConsolidationMode.NONE, timeout=2.0) if rep.ok is not None)
            k = "+".join(got) or "none"
            seen[k] = seen.get(k, 0) + 1
        for q in qs:
            q.undeclare()
    finally:
        for x in (a, b, c):
            x.close()
        r.close()
    return seen


def fact_best_matching(trials: int = 40) -> Fact:
    """Core Appendix B's `BestMatching` (0.114): "A face's queryables on one
    resource count as `complete` when any of them is ... In 1.10.1, on a
    router's south-bound face, a client's among them, the last one declared
    decided (zenoh#2845)." Client A declares a complete queryable and an
    incomplete one on one key, in either order; client B a complete one;
    client C calls it under `BestMatching`. Where A's face counts complete,
    A may answer; where it does not, only B can."""
    first = _best_matching(True, trials)
    last = _best_matching(False, trials)
    return Fact("best_matching", "core Appendix B, `BestMatching` (0.114): a face's queryables count as complete "
                "when any of them is; in 1.10.1 the last one declared decided", True,
                f"{trials} calls each, who answered: A's complete queryable declared first {first}, declared last "
                f"{last}", {"complete_first": first, "complete_last": last}, "measured")


FACTS: dict[str, Callable[[], Fact]] = {
    "stubs": fact_stubs, "handlers": fact_handlers, "unstable": fact_unstable,
    "publisher_defaults": fact_publisher_defaults, "no_config_accessor": fact_no_config_accessor,
    "history_keyword": fact_history_keyword, "ntp64": fact_ntp64, "null_config": fact_null_config,
    "alone": fact_alone, "replier_id": _replier_id, "callback_get": fact_callback_get,
    "get_timeout": fact_get_timeout, "query_timeout": fact_query_timeout, "attachment": fact_attachment,
    "client_hlc": fact_client_hlc, "wire": fact_wire, "withdrawals_first": fact_withdrawals_first,
    "starved": fact_starved, "history_race": fact_history_race, "best_matching": fact_best_matching,
    "receive_threads": fact_receive_threads,
}


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="zk2py.zenoh_facts", description=__doc__.split("\n\n", 1)[0])
    ap.add_argument("--only", action="append", choices=sorted(FACTS), help="run this fact alone (repeatable)")
    ap.add_argument("--json", type=Path, help="write every fact's result there")
    ap.add_argument("--digests", action="store_true", help="print the stubs' sha256 and exit")
    args = ap.parse_args(argv)
    if args.digests:
        print(json.dumps(_stub_digests(), indent=1))
        return 0
    print(f"eclipse-zenoh {_version()}")
    results: list[Fact] = []
    for name in args.only or list(FACTS):
        try:
            f = FACTS[name]()
        except Exception as e:  # noqa: BLE001 - a fact that cannot run is reported, never a pass
            f = Fact(name, "", False, f"could not run: {type(e).__name__}: {e}")
        results.append(f)
        word = "MEASURED" if f.kind == "measured" else "HOLDS" if f.holds else "DIFFERS"
        print(f"{word} {f.name}: {f.detail}", flush=True)
    if args.json:
        args.json.write_text(json.dumps({"zenoh": _version(), "facts": [f.__dict__ for f in results]}, indent=1,
                                        default=str), encoding="utf-8")
    facts = [f for f in results if f.kind == "fact"]
    held = sum(f.holds for f in facts)
    print(f"zenoh facts: {held} of {len(facts)} hold on eclipse-zenoh {_version()}, "
          f"{len(results) - len(facts)} measured")
    return 0 if held == len(facts) else 1


if __name__ == "__main__":
    sys.exit(main())

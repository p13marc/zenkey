"""``spec/scenarios/constrained.md`` §7, keys across the face (core §8.5,
0.78; observer §7.5, text 0.18; #609, round 95), run live with zk2py alone.

``python -m zk2py.constrained_live [--puts N] [--out FILE]``

**Setup**, as §7 writes it: one router of the runner's own (timestamping
disabled, as core §8.2 asks of a router onto a constrained face), and
zk2py's owner of ``interop/constrained/zk2py_face.v1.toml`` at
``site/rig``, its client. It writes the stream ``…/stream/level``, the
templated stream's member ``…/stream/axis/roll`` and the state
``…/state/mode``. Each step's far client is a client of the router through
:class:`CountingProxy`, a TCP relay that keeps every byte it relays toward
the far client, read as zenoh's stream links frame them: each batch after a
little-endian 16-bit length.

**Its owner is given a catalog** (core §8.6, 0.79): :func:`catalog_of`
lists the two contracts at their fingerprints. Step 0 GETs its descriptor
from the router's side and checks that it states the catalog's id (core
§3.3) and that the checker reports no code.

**Steps.** §7's four, each with a fresh far client, then zk2py's own:
1. a plain session subscribes to ``…/stream/level``, its exact key;
2. to the template's selector ``…/stream/axis/*``;
3. to ``zk2/**``;
4. to ``…/state/mode``, its exact key, then GETs it;
5. zk2py's tool, :func:`zk2py.echo_tool.watch` over
   :func:`zk2py.face.scopes` at ``site/rig`` (observer §7.5);
6. zk2py's consumer, an owner of ``zk2py_face_reader.v1`` whose role is
   bound to ``site/rig``, subscribing through
   :meth:`zk2py.owner.Owner.subscribe_role` to ``stream/level`` and
   ``stream/axis/{axis}`` (core §8.5);
7. the same consumer bound to the wildcard ``site/*`` (R1), F-439's case;
8. zk2py's probes of a template's selector (F-438): a rest parameter's
   member ``…/stream/path/a/b`` under ``…/stream/path/*/**``, and an event
   occurrence under ``…/events/alarm/*``.

**How a put is counted.** Each put's record is 8 bytes of text unique to
its step and its number, the same length in every step, so the batch that
carries it is found by its bytes and two steps' batches differ by their
keys alone. A put is
counted only where its batch carries it alone. Before a step counts, the
owner puts until the far client has heard one, so no count waits on a
sleep. What a step reports: each put's batch length, and with the framing's
two bytes, and whether the batch carries the key's text.

It exits 0 when every step's expectation holds, 1 when one does not, and 2
when a step could not run.
"""

from __future__ import annotations

import argparse
import json
import socket
import statistics
import sys
import threading
import time
from pathlib import Path
from typing import Any, Callable

HERE = Path(__file__).resolve().parent
CONTRACTS = HERE.parent / "interop" / "constrained"
IFACE = "zk2py_face.v1"
ADDRESS = "site/rig"
PREFIX = f"zk2/{ADDRESS}/{IFACE}"


class CountingProxy:
    """A TCP relay to ``upstream``: every byte toward the client is kept
    with its arrival time, and read back as zenoh's batches."""

    def __init__(self, upstream: str) -> None:
        host, port = upstream.removeprefix("tcp/").rsplit(":", 1)
        self.upstream = (host, int(port))
        self.sock = socket.socket()
        self.sock.bind(("127.0.0.1", 0))
        self.sock.listen(4)
        self.endpoint = f"tcp/127.0.0.1:{self.sock.getsockname()[1]}"
        self.lock = threading.Lock()
        self.down = bytearray()
        self.conns: list[socket.socket] = []
        threading.Thread(target=self._accept, daemon=True).start()

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
            threading.Thread(target=self._pump, args=(c, u, False), daemon=True).start()
            threading.Thread(target=self._pump, args=(u, c, True), daemon=True).start()

    def _pump(self, src: socket.socket, dst: socket.socket, toward_client: bool) -> None:
        try:
            while True:
                data = src.recv(65536)
                if not data:
                    break
                if toward_client:
                    with self.lock:
                        self.down += data
                dst.sendall(data)
        except OSError:
            pass

    def mark(self) -> int:
        with self.lock:
            return len(self.down)

    def batches(self, since: int = 0) -> list[bytes]:
        """The batches toward the client from offset ``since``, which must
        be a batch's start (a mark taken between batches)."""
        with self.lock:
            data = bytes(self.down)
        out, i = [], 0
        while i + 2 <= len(data):
            n = int.from_bytes(data[i:i + 2], "little")
            if i + 2 + n > len(data):
                break
            if i >= since:
                out.append(data[i + 2:i + 2 + n])
            i += 2 + n
        return out

    def framed_whole(self) -> bool:
        """Whether the bytes toward the client read as whole batches."""
        with self.lock:
            data = bytes(self.down)
        i = 0
        while i + 2 <= len(data):
            i += 2 + int.from_bytes(data[i:i + 2], "little")
        return i == len(data)

    def close(self) -> None:
        for s in [self.sock, *self.conns]:
            try:
                s.close()
            except OSError:
                pass


def _client(endpoint: str) -> Any:
    import zenoh

    conf = zenoh.Config()
    conf.insert_json5("mode", json.dumps("client"))
    conf.insert_json5("connect/endpoints", json.dumps([endpoint]))
    conf.insert_json5("scouting/multicast/enabled", "false")
    return zenoh.open(conf)


class Heard:
    """What a far client's subscriber delivered: each payload's bytes."""

    def __init__(self) -> None:
        self.lock = threading.Lock()
        self.got: list[bytes] = []
        self.event = threading.Event()

    def take(self, payload: bytes) -> None:
        with self.lock:
            self.got.append(payload)
        self.event.set()

    def holds(self, text: bytes) -> bool:
        with self.lock:
            return any(text in p for p in self.got)


def _wait_heard(put: Callable[[bytes], None], heard: Heard, tag: str, limit: float = 20.0) -> bool:
    """Put warm-up records until the far client hears one: the route stands."""
    end, n = time.monotonic() + limit, 0
    while time.monotonic() < end:
        text = f"{tag:_<4.4}w{n:03d}".encode()
        put(text)
        n += 1
        t = time.monotonic() + 0.2
        while time.monotonic() < t:
            if heard.holds(text):
                return True
            time.sleep(0.01)
    return False


def _count(proxy: CountingProxy, put: Callable[[bytes], None], heard: Heard, tag: str, n: int,
           key_text: bytes) -> dict[str, Any]:
    """Put ``n`` records spaced apart, and read the batch carrying each."""
    time.sleep(0.3)
    start = proxy.mark()
    texts = [f"{tag:_<4.4}p{i:03d}".encode() for i in range(n)]
    for t in texts:
        put(t)
        time.sleep(0.12)
    end = time.monotonic() + 3.0
    while time.monotonic() < end and not all(heard.holds(t) for t in texts):
        time.sleep(0.02)
    time.sleep(0.2)
    batches = proxy.batches(start)
    sizes, carries_key, alone, first = [], [], 0, None
    for t in texts:
        found = [b for b in batches if t in b]
        if len(found) != 1 or sum(1 for x in texts if x in found[0]) != 1:
            continue
        alone += 1
        sizes.append(len(found[0]))
        carries_key.append(key_text in found[0])
        first = first or found[0]
    return {"puts": n, "heard": sum(1 for t in texts if heard.holds(t)), "counted": alone,
            "batch_bytes": sorted(set(sizes)), "median": statistics.median(sizes) if sizes else None,
            "framed_bytes": sorted({s + 2 for s in sizes}), "carries": key_text.decode(),
            "carries_in": sum(carries_key), "a_batch": first.hex() if first else None}


def catalog_of(contracts: list[Any], epoch: int = 1767225600) -> Any:
    """The deployment's catalog for the run: each contract at its index,
    read back as §8.6 reads one, so its id is what any reader computes."""
    from . import catalog

    doc = {"format": catalog.FORMAT, "epoch": epoch,
           "entries": [{"index": i, "iface": c.interface, "contract": c.fingerprint}
                       for i, c in enumerate(contracts)]}
    return catalog.read(json.dumps(doc))


def run(puts: int = 10) -> dict[str, Any]:
    import zenoh

    from . import echo_tool, face
    from .contract import load_contract
    from .live_interop import _r1
    from .owner import Owner

    face_c = load_contract(CONTRACTS / "zk2py_face.v1.toml")
    reader_c = load_contract(CONTRACTS / "zk2py_face_reader.v1.toml")
    for c in (face_c, reader_c):
        if not c.valid:
            raise RuntimeError(f"{c.path}: {c.codes}")
    r1, ep, _ = _r1(timestamping=False)
    owner = Owner("site", "rig", [face_c], connect=ep)
    cat = catalog_of([face_c, reader_c])
    owner.catalog = cat
    owner.start()
    steps: dict[str, Any] = {}
    after_zk2 = f"/{ADDRESS}/{IFACE}"

    def stream(rid: str) -> Callable[[bytes], None]:
        return lambda text: owner.publish(f"{PREFIX}/{rid}", text)

    def plain_step(name: str, selector: str, put: Callable[[bytes], None], key_text: str,
                   get: str | None = None) -> None:
        proxy = CountingProxy(ep)
        try:
            session = _client(proxy.endpoint)
            try:
                heard = Heard()
                session.declare_subscriber(selector, zenoh.handlers.Callback(
                    lambda s: heard.take(s.payload.to_bytes())))
                if not _wait_heard(put, heard, name):
                    steps[name] = {"selector": selector, "ran": False,
                                   "why": "the far client heard no warm-up put"}
                    return
                row = {"selector": selector, "ran": True, "declared": face.nonwild_prefix(selector),
                       **_count(proxy, put, heard, name, puts, key_text.encode())}
                if get is not None:
                    mark = proxy.mark()
                    # core 0.100 S4: consolidation None
                    replies = list(session.get(get, target=zenoh.QueryTarget.ALL,
                                               consolidation=zenoh.ConsolidationMode.NONE, timeout=5.0))
                    time.sleep(0.2)
                    got = [b for b in proxy.batches(mark) if get.encode() in b]
                    row["get"] = {"selector": get, "replies": len([r for r in replies if r.ok is not None]),
                                  "reply_batch_bytes": sorted(len(b) for b in got),
                                  "carries_whole_key": bool(got)}
                row["framed_whole"] = proxy.framed_whole()
                steps[name] = row
            finally:
                session.close()  # its subscriber goes with it
        finally:
            proxy.close()

    try:
        # 0: the owner's descriptor states its catalog's id (core §3.3, 0.79).
        from . import live
        from .descriptor import check_descriptor

        near = _client(ep)
        try:
            answers = [a for a in live.get_descriptor(near, owner.instance_key) if a.ok]
        finally:
            near.close()
        doc = json.loads(answers[0].payload) if answers else {}
        steps["0"] = {"catalog": cat.id, "epoch": cat.epoch, "stated": doc.get("catalog"),
                      "codes": check_descriptor(answers[0].payload, [face_c]) if answers else None,
                      "writing_epoch": owner.writing_epoch()}

        # §7 steps 1 to 4: a plain session as the far client.
        plain_step("1", f"{PREFIX}/stream/level", stream("stream/level"), "level")
        plain_step("2", f"{PREFIX}/stream/axis/*", stream("stream/axis/roll"), "stream/axis")
        plain_step("3", "zk2/**", stream("stream/level"), f"{after_zk2}/stream/level")
        plain_step("4", f"{PREFIX}/state/mode", lambda t: owner.set_state(f"{PREFIX}/state/mode", t), "mode",
                   get=f"{PREFIX}/state/mode")

        # 5: zk2py's tool, by the scopes of the contract it holds (observer
        # §7.5). echo_tool.watch hands its rows back at its window's end, so
        # a put is known to have crossed when the proxy carried it.
        scopes = face.scopes([face_c], [ADDRESS])
        proxy = CountingProxy(ep)
        try:
            session = _client(proxy.endpoint)
            try:
                rows: list[dict[str, Any]] = []
                listening = threading.Event()
                window_s = 12.0 + 4.0 * puts

                def tool() -> None:
                    rows.extend(echo_tool.watch(session, scopes, window_s, 0, listening.set, None,
                                                echo_tool.Held([face_c]), None))

                class Carried(Heard):
                    """A put is heard when the proxy carried its record."""

                    def holds(self, text: bytes) -> bool:
                        return any(text in b for b in proxy.batches())

                th = threading.Thread(target=tool, daemon=True)
                th.start()
                listening.wait(10)
                tool_rows: dict[str, Any] = {"scopes": scopes,
                                             "declared": [face.nonwild_prefix(s) for s in scopes]}
                for rid, tag, put, text in (
                        ("stream/level", "5lv", stream("stream/level"), "level"),
                        ("stream/axis/roll", "5ax", stream("stream/axis/roll"), "stream/axis"),
                        ("state/mode", "5md", lambda t: owner.set_state(f"{PREFIX}/state/mode", t), "mode")):
                    if not _wait_heard(put, Carried(), tag):
                        tool_rows[rid] = {"ran": False}
                        continue
                    tool_rows[rid] = _count(proxy, put, Carried(), tag, puts, text.encode())
                th.join(window_s + 10)
                tool_rows["rows"] = len(rows)
                tool_rows["undecodable"] = sum(1 for r in rows if r.get("undecodable"))
                tool_rows["framed_whole"] = proxy.framed_whole()
                steps["5"] = tool_rows
            finally:
                session.close()
        finally:
            proxy.close()

        # 6 and 7: zk2py's consumer, bound exactly, then to a wildcard.
        for name, binding in (("6", ADDRESS), ("7", "site/*")):
            proxy = CountingProxy(ep)
            try:
                consumer = Owner("ground", f"reader{name}", [reader_c], connect=proxy.endpoint,
                                 bindings={"src": [binding]})
                try:
                    consumer.start()
                    hs = {"stream/level": Heard(), "stream/axis/{axis}": Heard()}
                    for rid, h in hs.items():
                        consumer.subscribe_role("src", IFACE, rid, lambda k, x, h_=h: h_.take(x))
                    row: dict[str, Any] = {"binding": binding,
                                           "selectors": consumer.role_keys("src", IFACE, "stream/level")
                                           + consumer.role_keys("src", IFACE, "stream/axis/{axis}")}
                    row["declared"] = [face.nonwild_prefix(s) for s in row["selectors"]]
                    exact = binding == ADDRESS
                    for rid, tag, h, put, text in (
                            ("stream/level", f"{name}lv", hs["stream/level"], stream("stream/level"),
                             "level" if exact else f"/rig/{IFACE}/stream/level"),
                            ("stream/axis/roll", f"{name}ax", hs["stream/axis/{axis}"], stream("stream/axis/roll"),
                             "stream/axis" if exact else f"/rig/{IFACE}/stream/axis")):
                        if not _wait_heard(put, h, tag):
                            row[rid] = {"ran": False}
                            continue
                        row[rid] = _count(proxy, put, h, tag, puts, text.encode())
                    row["framed_whole"] = proxy.framed_whole()
                    steps[name] = row
                finally:
                    consumer.close()
            finally:
                proxy.close()

        # 8: a rest parameter's member, and an event's occurrence (F-438).
        plain_step("8a", face.scopes([face_c], [ADDRESS])[4], stream("stream/path/a/b"), "stream/path")
        plain_step("8b", face.scopes([face_c], [ADDRESS])[0],
                   lambda t: owner.occur(f"{PREFIX}/events/alarm", t), "events/alarm")
    finally:
        owner.close()
        r1.close()
    return {"steps": steps, "key_after_zk2": f"{after_zk2}/stream/level",
            "key_after_zk2_bytes": len(f"{after_zk2}/stream/level")}


def judge(report: dict[str, Any]) -> list[tuple[str, bool, str]]:
    """§7's expectations, and zk2py's own, over a run's report."""
    s = report["steps"]
    out: list[tuple[str, bool, str]] = []

    def ok(step: str, sub: str | None = None) -> dict[str, Any] | None:
        row = s.get(step, {})
        row = row.get(sub, {}) if sub else row
        return row if row.get("counted") else None

    zero = s.get("0", {})
    out.append(("core §3.3: an owner given a catalog states its id, and the checker reports no code",
                bool(zero.get("stated") == zero.get("catalog") and zero.get("codes") == []
                     and zero.get("writing_epoch") == zero.get("epoch")), json.dumps(zero)))
    one, two, three, four = ok("1"), ok("2"), ok("3"), ok("4")
    out.append(("§7 step 1: each put rides its scope, none of its key's text",
                bool(one and one["carries_in"] == 0), json.dumps(one and one["batch_bytes"])))
    out.append(("§7 step 2: each put rides the scope, and carries /roll",
                bool(two and two["carries_in"] == 0 and two["counted"] == two["puts"]),
                json.dumps(two and two["batch_bytes"])))
    grow = report["key_after_zk2_bytes"] + 1
    out.append(("§7 step 3: each put carries its key's text after zk2, larger than step 1's by it and a length byte",
                bool(three and one and three["carries_in"] == three["counted"]
                     and abs(three["median"] - one["median"] - grow) <= 1),
                f"{three and three['median']} against {one and one['median']} + {grow}"))
    out.append(("§7 step 4: the reply carries its whole key as text",
                bool(s.get("4", {}).get("get", {}).get("carries_whole_key")), json.dumps(s.get("4", {}).get("get"))))
    for rid in ("stream/level", "stream/axis/roll", "state/mode"):
        base = {"stream/level": one, "stream/axis/roll": two, "state/mode": four}[rid]
        row = ok("5", rid)
        out.append((f"observer §7.5: zk2py's tool by its scopes, {rid} as its own step's",
                    bool(row and base and row["carries_in"] == 0 and row["median"] == base["median"]),
                    f"{row and row['batch_bytes']} against {base and base['batch_bytes']}"))
    for rid in ("stream/level", "stream/axis/roll"):
        base = {"stream/level": one, "stream/axis/roll": two}[rid]
        row = ok("6", rid)
        out.append((f"core §8.5: zk2py's consumer bound exactly, {rid} as its own step's",
                    bool(row and base and row["carries_in"] == 0 and row["median"] == base["median"]),
                    f"{row and row['batch_bytes']} against {base and base['batch_bytes']}"))
        row = ok("7", rid)
        out.append((f"F-439: zk2py's consumer bound to site/*, {rid} carries its key's text after zk2/site",
                    bool(row and row["carries_in"] == row["counted"]),
                    f"{row and row['batch_bytes']}"))
    for step, what in (("8a", "a rest parameter's member under */**"), ("8b", "an event under its ULID's *")):
        row = ok(step)
        out.append((f"F-438: {what} rides the literal prefix", bool(row and row["carries_in"] == 0),
                    f"{row and row['batch_bytes']}"))
    return out


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="python -m zk2py.constrained_live", description=__doc__.split("\n")[0])
    ap.add_argument("--puts", type=int, default=10)
    ap.add_argument("--out", type=Path, default=None)
    args = ap.parse_args(argv)
    try:
        report = run(args.puts)
    except Exception as e:  # noqa: BLE001 - a step that could not run
        print(f"constrained_live: could not run: {e}", file=sys.stderr)
        return 2
    verdicts = judge(report)
    report["verdicts"] = [{"case": c, "ok": k, "detail": d} for c, k, d in verdicts]
    text = json.dumps(report, indent=2)
    if args.out is not None:
        args.out.write_text(text + "\n")
    for c, k, d in verdicts:
        print(f"{'PASS' if k else 'FAIL'} {c}: {d}")
    return 0 if all(k for _, k, _ in verdicts) else 1


if __name__ == "__main__":
    sys.exit(main())

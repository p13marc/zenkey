"""``redundancy.v1`` ``scenarios.md`` §1 to §12 (text 0.1), live, from
Python (#609, round 56)::

    python -m zk2py.redundancy_scenarios [--only 1|2|…|12]... [--runs N]

The Rust runtime (#841, RB) does not exist yet, so every instance is
zk2py's: ``python -m zk2py.redundancy_owner``, a process driven as a black
box through its help (scenarios.md, "The owner example"), so that a crash
is a SIGKILL. §12's tool is zk2py's (:func:`zk2py.redundancy_live.tool_reading`),
run in process with its shapes built by X.

The bus is zk2py's: R1, and where a section cuts a link R2 behind a link
the runner cuts and heals (spike S12 cut a proxy). S follows
``zk2/lab/*/@zk/**`` with a liveliness subscriber on a watch session and
subscribes to ``zk2/lab/**`` and the instance keys (``@zk`` is verbatim, so
``**`` does not reach them), recording each delivery's arrival on the
host's monotonic clock; S2 does the same on R2. G reads presence on a
session that has never held a liveliness subscriber (core §8.1, 0.38), and
GETs and calls as X. A, B and C have chosen ids, A's lowest; the retry
interval and the grace are 2 s.

Exit 0 when every check passes.
"""

from __future__ import annotations

import argparse
import json
import os
import queue
import random
import signal
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path
from typing import Any

from .alarms_scenarios import Report, _wait

REPO = Path(__file__).resolve().parents[3]
LEDGER, VALVE = "lab/ledger", "lab/valve"
A_ID, B_ID, C_ID = "00000000000000a1", "00000000000000b2", "00000000000000c3"
ALIVE = "zk2/lab/ledger/@zk/alive/ledger.v1/"
BALANCE = "zk2/lab/ledger/ledger.v1/state/balance"
APPEND = "zk2/lab/ledger/ledger.v1/@op/append"


class Inst:
    """One candidate process (``zk2py.redundancy_owner``)."""

    def __init__(self, address: str, endpoint: str, *, instance: str, name: str, flags: list[str] | None = None,
                 contract: str = "ledger.v1") -> None:
        argv = [sys.executable, "-m", "zk2py.redundancy_owner", "--connect", endpoint, "--instance", instance,
                "--name", name, "--contract", contract, *(flags or []), address]
        env = dict(os.environ, PYTHONPATH=str(REPO / "impl" / "python"))
        self.address, self.instance, self.name = address, instance, name
        self.t_launch = time.monotonic()
        self.proc = subprocess.Popen(argv, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                     text=True, env=env)
        self.lines: queue.Queue[str] = queue.Queue()
        self.stderr: list[str] = []
        threading.Thread(target=lambda: [self.lines.put(x.rstrip("\n")) for x in self.proc.stdout],
                         daemon=True).start()
        threading.Thread(target=lambda: [self.stderr.append(x) for x in self.proc.stderr], daemon=True).start()
        self.ready: str | None = None
        self.refused: str | None = None
        if "--wait-go" not in (flags or []):
            self._await_ready()

    def _await_ready(self, timeout: float = 20.0) -> None:
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            try:
                line = self.lines.get(timeout=0.2)
            except queue.Empty:
                if self.proc.poll() is not None:
                    return
                continue
            if line.startswith("ready "):
                self.ready = line[len("ready "):]
                return
            if line.startswith("refused "):
                self.refused = line[len("refused "):]
                return

    def go(self) -> None:
        self.send("go")

    def send(self, line: str) -> None:
        try:
            self.proc.stdin.write(line + "\n")
            self.proc.stdin.flush()
        except OSError:
            pass

    def cmd(self, line: str, timeout: float = 10.0) -> str | None:
        self.send(line)
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            try:
                got = self.lines.get(timeout=0.2)
            except queue.Empty:
                if self.proc.poll() is not None:
                    return None
                continue
            if got.startswith("ready ") or got.startswith("refused "):
                continue
            return got
        return None

    def status(self) -> dict[str, str]:
        got = self.cmd("status") or ""
        return dict(kv.split("=", 1) for kv in got.split()[2:] if "=" in kv)

    def role(self) -> str | None:
        return self.status().get("role")

    def state(self) -> str | None:
        return self.status().get("state")

    def executions(self) -> int | None:
        e = self.status().get("executions")
        return None if e is None else int(e)

    def zid(self) -> str | None:
        return self.status().get("zid")

    def steps(self) -> list[Any]:
        got = self.cmd("steps") or ""
        return json.loads(got[len("redundancy steps "):]) if got.startswith("redundancy steps ") else []

    def reads(self) -> list[Any]:
        got = self.cmd("reads") or ""
        return json.loads(got[len("redundancy reads "):]) if got.startswith("redundancy reads ") else []

    def put(self, rid: str, value: str) -> str | None:
        return self.cmd(f"put {rid} {value}")

    def kill(self) -> float:
        t = time.monotonic()
        if self.proc.poll() is None:
            self.proc.send_signal(signal.SIGKILL)
        self.proc.wait(10)
        return t

    def close(self) -> float:
        """A clean stop: standard input closed."""
        t = time.monotonic()
        try:
            self.proc.stdin.close()
        except OSError:
            pass
        try:
            self.proc.wait(timeout=15)
        except subprocess.TimeoutExpired:
            self.kill()
        return t

    def serving(self) -> bool:
        return self.state() == "serving"

    def standing(self) -> bool:
        return self.state() == "standing_by"


class SilentLink:
    """A TCP relay to a router that can go silent: once :meth:`freeze`d it
    forwards nothing either way, and an end that closes is not passed on,
    so the router's side stays open and quiet until the router's lease
    closes it. A process behind it killed after the freeze is a host lost:
    its kernel's close never reaches the router (core Appendix B: "one
    lease after a link goes silent")."""

    def __init__(self, upstream: str) -> None:
        import socket

        host, port = upstream.removeprefix("tcp/").rsplit(":", 1)
        self.upstream = (host, int(port))
        self.frozen = False
        self.socks: list[Any] = []
        self.sock = socket.socket()
        self.sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        self.sock.bind(("127.0.0.1", 0))
        self.sock.listen(16)
        self.endpoint = f"tcp/127.0.0.1:{self.sock.getsockname()[1]}"
        threading.Thread(target=self._accept, daemon=True).start()

    def _accept(self) -> None:
        import socket

        while True:
            try:
                c, _ = self.sock.accept()
            except OSError:
                return
            try:
                u = socket.create_connection(self.upstream)
            except OSError:
                c.close()
                continue
            self.socks += [c, u]
            for a, b in ((c, u), (u, c)):
                threading.Thread(target=self._pump, args=(a, b), daemon=True).start()

    def _pump(self, src: Any, dst: Any) -> None:
        try:
            while True:
                data = src.recv(65536)
                if not data:
                    break
                if not self.frozen:
                    dst.sendall(data)
        except OSError:
            pass
        if self.frozen:
            return  # the other end stays open, and silent
        for x in (src, dst):
            try:
                x.close()
            except OSError:
                pass

    def freeze(self) -> None:
        self.frozen = True

    def close(self) -> None:
        for x in [self.sock, *self.socks]:
            try:
                x.close()
            except OSError:
                pass


class Watcher:
    """S (or S2): tokens, data and descriptors on one session, which makes
    no presence read (core §8.1's watch session), so that S sees one
    owner's descriptor put and token in the order its session sent them:
    zenoh orders one session's messages, never two sessions'."""

    def __init__(self, endpoint: str) -> None:
        import zenoh

        from . import live

        self.zenoh = zenoh
        self.lock = threading.Lock()
        self.tokens: list[tuple[float, str, str]] = []
        self.data: list[tuple[float, str, str, bytes, str | None]] = []
        #: core presence.md §1's observation: a GET of the instance's
        #: descriptor made the moment a token of it appears, (time, "instance"
        #: or "alive", instance, marked standby, or None for no answer, the
        #: answer's time)
        self.probes: list[tuple[float, str, str, bool | None, float]] = []
        self.session = live.open_client(endpoint)
        self.entities = [
            self.session.declare_subscriber("zk2/lab/**", zenoh.handlers.Callback(self._data)),
            self.session.declare_subscriber("zk2/lab/*/@zk/instance/*", zenoh.handlers.Callback(self._data)),
        ]
        self.entities.append(live.declare_liveliness_subscriber(self.session, "zk2/lab/*/@zk/**", self._token,
                                                                history=True))

    def _token(self, smp: Any) -> None:
        t, key = time.monotonic(), str(smp.key_expr)
        put = smp.kind == self.zenoh.SampleKind.PUT
        with self.lock:
            self.tokens.append((t, "put" if put else "delete", key))
        if put and ("/@zk/instance/" in key or "/@zk/alive/" in key):
            threading.Thread(target=self._probe, args=(t, key), daemon=True).start()

    def _probe(self, t: float, key: str) -> None:
        from . import live

        chunks = key.split("/")
        at = chunks.index("@zk")
        kind, instance = chunks[at + 1], chunks[at + 2] if chunks[at + 1] == "instance" else chunks[at + 3]
        ikey = "/".join(chunks[:at]) + f"/@zk/instance/{instance}"
        got = [a for a in live.get_descriptor(self.session, ikey) if a.ok]
        mark = None
        if len(got) == 1:
            try:
                e = (json.loads(got[0].payload).get("interfaces") or [{}])[0]
                mark = e.get("standby") is True
            except (ValueError, AttributeError, IndexError):
                pass
        with self.lock:
            self.probes.append((t, kind, instance, mark, time.monotonic()))

    def _data(self, smp: Any) -> None:
        ts = smp.timestamp
        with self.lock:
            self.data.append((time.monotonic(), str(smp.key_expr),
                              "delete" if smp.kind == self.zenoh.SampleKind.DELETE else "put",
                              smp.payload.to_bytes(), None if ts is None else str(ts)))

    def token_times(self, kind: str, prefix: str, after: float = 0.0) -> list[float]:
        with self.lock:
            return [t for t, k, key in self.tokens if k == kind and key.startswith(prefix) and t >= after]

    def first_token(self, kind: str, prefix: str, after: float = 0.0) -> float | None:
        ts = self.token_times(kind, prefix, after)
        return ts[0] if ts else None

    def descriptors(self, address: str, instance: str, after: float = 0.0) -> list[tuple[float, bool | None]]:
        """Each descriptor put of an instance S received: (time, marked
        standby for its first interface)."""
        key = f"zk2/{address}/@zk/instance/{instance}"
        out = []
        with self.lock:
            rows = [(t, p) for t, k, kind, p, _ in self.data if k == key and kind == "put" and t >= after]
        for t, p in rows:
            try:
                doc = json.loads(p)
                e = (doc.get("interfaces") or [{}])[0]
                out.append((t, e.get("standby") is True))
            except (ValueError, AttributeError, IndexError):
                out.append((t, None))
        return out

    def writes(self, prefix: str, after: float = 0.0) -> list[tuple[float, str, str, bytes, str | None]]:
        with self.lock:
            return [d for d in self.data if d[1].startswith(prefix) and d[0] >= after]

    def close(self) -> None:
        from . import live

        for e in self.entities:
            try:
                e.undeclare()
            except Exception:  # noqa: BLE001
                pass
        live.close(self.session)


class Bus:
    """R1; with ``cut``, R2 linked to R1 through a link the runner cuts."""

    def __init__(self, cut: bool = False, r1: Any = None) -> None:
        import zenoh

        from . import live
        from .budget_scenarios import Link
        from .live_interop import _r1
        from .owner import free_loopback_port

        if r1 is None:
            self.r1, self.ep1, _ = _r1()
        else:
            self.r1, self.ep1 = r1
        self.link = self.r2 = None
        self.ep2 = self.ep1
        if cut:
            self.link = Link(self.ep1)
            port = free_loopback_port()
            conf = zenoh.Config()
            conf.insert_json5("mode", json.dumps("router"))
            conf.insert_json5("listen/endpoints", json.dumps([f"tcp/127.0.0.1:{port}"]))
            conf.insert_json5("connect/endpoints", json.dumps([self.link.endpoint]))
            conf.insert_json5("scouting/multicast/enabled", "false")
            conf.insert_json5("timestamping/enabled", "true")
            self.r2 = zenoh.open(conf)
            self.ep2 = f"tcp/127.0.0.1:{port}"
        self.s = Watcher(self.ep1)
        self.s2 = Watcher(self.ep2) if cut else None
        self.g = live.open_client(self.ep1)
        self.g2 = live.open_client(self.ep2) if cut else self.g
        self.insts: list[Inst] = []
        self.extra: list[Any] = []
        time.sleep(0.3)

    def inst(self, name: str, instance: str, on: int = 1, flags: list[str] | None = None,
             address: str = LEDGER, contract: str = "ledger.v1", endpoint: str | None = None) -> Inst:
        i = Inst(address, endpoint or (self.ep1 if on == 1 else self.ep2), instance=instance, name=name,
                 flags=flags, contract=contract)
        self.insts.append(i)
        return i

    def holders(self, on: int = 1, address: str = LEDGER, iface: str = "ledger.v1") -> tuple[bool, set[str]]:
        from . import live

        p = live.list_presence(self.g if on == 1 else self.g2, f"zk2/{address}/@zk/**")
        return p.complete, {a["instance"] for a in p.alive if a["iface"] == iface}

    def linked(self) -> bool:
        """R2 holds R1's view: a token of R1's side reads through G2."""
        from . import live

        p = live.list_presence(self.g2, "zk2/lab/*/@zk/instance/*")
        q = live.list_presence(self.g, "zk2/lab/*/@zk/instance/*")
        return {i["instance"] for i in p.instances} == {i["instance"] for i in q.instances}

    def close(self) -> None:
        from . import live

        for i in self.insts:
            try:
                i.kill()
            except Exception:  # noqa: BLE001
                pass
        for x in self.extra:
            try:
                x.close()
            except Exception:  # noqa: BLE001
                pass
        for w in (self.s, self.s2):
            if w is not None:
                w.close()
        for g in {id(x): x for x in (self.g, self.g2)}.values():
            live.close(g)
        if self.link is not None:
            self.link.close()
        for r in (self.r2, self.r1):
            if r is not None:
                try:
                    r.close()
                except Exception:  # noqa: BLE001
                    pass


def _ordered(bus: Bus, inst: Inst, after: float = 0.0, claim: bool = True,
             serving: bool = False) -> tuple[bool, str]:
    """§1 Expected 1, observed as core presence.md §1 observes a bring-up:
    a GET of the descriptor made the moment its instance token appears
    answers it marked standby, and one made the moment its interface token
    appears answers it unmarked: so each put came before its token (§8.2
    steps 3 and 4, 0.39). S's arrival order of a put and a token is noted,
    never judged: zenoh carries a declaration and a put on two priorities,
    and a subscriber session received one session's put-then-token token
    first in 72 of 100 (F-202). ``claim`` False checks the takeover alone;
    ``serving`` waits for its interface token."""
    def probe(kind: str) -> tuple[float, bool | None, float] | None:
        with bus.s.lock:
            for t, k, i, m, ta in bus.s.probes:
                if k == kind and i == inst.instance and t >= after:
                    return t, m, ta
        return None

    if serving or not claim:
        _wait(lambda: probe("alive") is not None, 3.0, 0.02)
    if claim:
        _wait(lambda: probe("instance") is not None, 3.0, 0.02)
    pi, pa = probe("instance"), probe("alive")
    # an instance that took over before the probe's answer came reads
    # unmarked by then: its interface token at S before that answer says so
    late = pi is not None and pi[1] is False and pa is not None and pa[0] < pi[2]
    ok = (pi is not None and (pi[1] is True or late)) if claim else True
    if serving or not claim:
        ok = ok and pa is not None and pa[1] is False
    descs = bus.s.descriptors(inst.address, inst.instance, after)
    order = "; S received " + ", ".join(f"{'marked' if m else 'unmarked'} +{t - after:.4f}" for t, m in descs[:3])
    return ok, (f"at its instance token the descriptor read {'marked' if pi and pi[1] else pi and 'unmarked'}, "
                f"at its interface token {'unmarked' if pa and pa[1] is False else pa and 'marked'}{order}")


# -- §1 ------------------------------------------------------------------------------

def section1(rp: Report, runs: int) -> None:
    from . import live

    sec = "§1 takeover after a crash, seen at the lease"
    bus = Bus()
    silent = None
    try:
        t0 = time.monotonic()
        a = bus.inst("A", A_ID)
        up = _wait(a.serving, 3.0)
        t_up = time.monotonic() - t0
        ok, why = _ordered(bus, a, t0, serving=True)
        rp.check(sec, "1. A claims before it reads: its descriptor marked, then its instance token; on taking "
                      "over its descriptor unmarked, then its interface token, the token last; A serves within 3 s",
                 up and ok and t_up <= 3.0 + 0.5, f"serving after {t_up:.2f} s; {why}")
        time.sleep(2.0)
        tb = time.monotonic()
        b = bus.inst("B", B_ID)
        _wait(b.standing, 5.0)
        results = []
        for _ in range(20):
            r = live.call(bus.g, APPEND, b"x")
            results.append(r.replies[0].payload if r.replies else None)
        ea, eb = a.executions(), b.executions()
        rp.check(sec, "3. 20 calls, BestMatching, each after the last reply: 20 executions on A, none on B",
                 results == [b"A"] * 20 and ea == 20 and eb == 0, f"replies {set(results)}, A {ea}, B {eb}")
        k = a.kill()
        withdrawn = _wait(lambda: bus.s.first_token("delete", ALIVE + A_ID, k) is not None, 20.0, 0.05)
        t_w = bus.s.first_token("delete", ALIVE + A_ID, k)
        took = _wait(lambda: bus.s.first_token("put", ALIVE + B_ID, k) is not None, 5.0, 0.02)
        t_b = bus.s.first_token("put", ALIVE + B_ID, k)
        call_b = live.call(bus.g, APPEND, b"x") if took else None
        b_alive_before = bus.s.token_times("put", ALIVE + B_ID, 0.0)
        rp.check(sec, "2. B stands by, and stays so: no interface token of B's before step 4, its descriptor "
                      "never losing the mark before K",
                 all(m for t, m in bus.s.descriptors(LEDGER, B_ID, tb) if t < k)
                 and all(t >= k for t in b_alive_before),
                 f"B's descriptors before K {[m for t, m in bus.s.descriptors(LEDGER, B_ID, tb) if t < k]}")
        lease = None if t_w is None else t_w - k
        rp.check(sec, "4. (as written, SIGKILL) S hears A's tokens withdrawn: at once, since A's kernel closes "
                      "its link (core Appendix B: 1–2 ms when the link closes), never at the lease the step "
                      "expects (F-201)", withdrawn and lease is not None and lease < 1.0,
                 f"withdrawn {None if lease is None else round(lease, 4)} s after K")
        ok_b, why_b = _ordered(bus, b, k, claim=False)
        gap = None if (t_b is None or t_w is None) else t_b - t_w
        rp.check(sec, "4. B, standing by, reads, finds no holder in a complete read, and takes over: its descriptor "
                      "unmarked, then its interface token, within 3 s of A's withdrawal",
                 took and gap is not None and gap <= 3.0 and ok_b, f"+{None if gap is None else round(gap, 3)} s; "
                                                                  f"{why_b}")
        rp.check(sec, "5. the call as soon as S sees B's interface token executes on B",
                 call_b is not None and [x.payload for x in call_b.replies] == [b"B"],
                 f"{None if call_b is None else [x.payload for x in call_b.replies]}")
        b.close()
        bus.insts.clear()
        time.sleep(0.5)
        # The crash the step means, seen at the lease: A's host lost, its
        # link silent, then A killed (SilentLink).
        silent = SilentLink(bus.ep1)
        a = bus.inst("A", A_ID, endpoint=silent.endpoint)
        _wait(a.serving, 5.0)
        b = bus.inst("B", B_ID)
        _wait(b.standing, 5.0)
        silent.freeze()
        k = a.kill()
        withdrawn = _wait(lambda: bus.s.first_token("delete", ALIVE + A_ID, k) is not None, 20.0, 0.05)
        t_w = bus.s.first_token("delete", ALIVE + A_ID, k)
        took = _wait(lambda: bus.s.first_token("put", ALIVE + B_ID, k) is not None, 5.0, 0.02)
        t_b = bus.s.first_token("put", ALIVE + B_ID, k)
        lease = None if t_w is None else t_w - k
        rp.check(sec, "4. (A's host lost: its link silent, then A killed) S hears A's tokens withdrawn when R1's "
                      "lease for A's session runs out, 10 s after K, give or take a keep-alive",
                 withdrawn and lease is not None and 7.0 <= lease <= 13.0,
                 f"withdrawn {None if lease is None else round(lease, 2)} s after K")
        gap = None if (t_b is None or t_w is None) else t_b - t_w
        ok_b, why_b = _ordered(bus, b, k, claim=False)
        rp.check(sec, "4. B takes over within 3 s of A's withdrawal, in §2.6's order",
                 took and gap is not None and gap <= 3.0 and ok_b,
                 f"+{None if gap is None else round(gap, 3)} s; {why_b}")
        reads = b.reads()
        steps = b.steps()
        between = [r for r in reads if k <= r[0] < (t_w or k)]
        stood = [s[0] for s in steps if s[2].get("step") == "stand_by" and s[2].get("why") == "incumbent"]
        polled = [r for r in reads if stood and stood[0] < r[0] < (t_w or k)]
        rp.check(sec, "4. B made no read between K and the withdrawal, and never polled: its reads follow its "
                      "watch's events (§2.7)", bool(stood) and not between and not polled,
                 f"reads between K and the withdrawal {len(between)}, after standing by on A {len(polled)}")
    finally:
        if silent is not None:
            silent.close()
        bus.close()


# -- §2 ------------------------------------------------------------------------------

def section2(rp: Report, runs: int) -> None:
    sec = "§2 takeover after a clean stop"
    bus = Bus()
    try:
        a = bus.inst("A", A_ID)
        _wait(a.serving, 5.0)
        b = bus.inst("B", B_ID)
        _wait(b.standing, 5.0)
        zid_a = a.zid()
        q = a.close()
        gone = _wait(lambda: bus.s.first_token("delete", ALIVE + A_ID, q) is not None, 3.0, 0.01)
        t_w = bus.s.first_token("delete", ALIVE + A_ID, q)
        took = _wait(lambda: bus.s.first_token("put", ALIVE + B_ID, q) is not None, 3.0, 0.01)
        t_b = bus.s.first_token("put", ALIVE + B_ID, q)
        ok, why = _ordered(bus, b, q, claim=False)
        rp.check(sec, "1. A's interface token is withdrawn at once, without a lease",
                 gone and t_w is not None and t_w - q < 1.0, f"{None if t_w is None else round(t_w - q, 3)} s after Q")
        rp.check(sec, "1. B serves within 2 s of Q, in §2.6's order",
                 took and t_b is not None and t_b - q <= 2.0 and ok,
                 f"{None if t_b is None else round(t_b - q, 3)} s after Q; {why}")
        a_writes = [w for w in bus.s.writes("zk2/lab/ledger/ledger.v1/", q)
                    if w[4] is not None and zid_a and w[4].split("/", 1)[1].lstrip("0") == zid_a.lstrip("0")]
        rp.check(sec, "1. A puts and deletes nothing of ledger.v1's keys while it closes", not a_writes,
                 f"{len(a_writes)} writes stamped with A's zid after Q")
    finally:
        bus.close()


# -- §3 ------------------------------------------------------------------------------

def section3(rp: Report, runs: int) -> None:
    sec = "§3 a simultaneous start"
    bus = Bus()
    healed, fails, worst = 0, [], 0.0
    own_seen = [0, 0]
    b_won: list[int] = []
    try:
        for n in range(runs):
            ids = sorted(f"{random.getrandbits(64):016x}" for _ in range(2))
            a = bus.inst("A", ids[0], flags=["--wait-go"])
            b = bus.inst("B", ids[1], flags=["--wait-go"])
            time.sleep(1.5)  # both processes up, waiting
            t = time.monotonic()
            a.go()
            b.go()
            end = _wait(lambda: (a.serving() and b.standing()) or (b.serving() and a.standing()
                                                                   and a.steps()[-1][2].get("why") == "incumbent"),
                        5.0 + 2 * 2.0, 0.1)
            dt = time.monotonic() - t
            if end and b.serving():
                # §2.5 step 2: B's read missed A's claim and B took over; A's
                # read then showed B's token, and a live incumbent wins
                b_won.append(n)
            worst = max(worst, dt)
            oa, wa = _ordered(bus, a, t - 0.1, serving=a.serving())
            ob, wb = _ordered(bus, b, t - 0.1, serving=b.serving())
            sa, sb = a.steps(), b.steps()
            first_read = True
            for inst, st in ((a, sa), (b, sb)):
                names = [s[2]["step"] for s in st]
                first_read = first_read and "read" in names and names.index("claim") < names.index("read")
                rs = inst.reads()
                if rs and inst.instance in [i[0] for i in rs[0][4]]:
                    own_seen[0] += 1
                own_seen[1] += 1
            if any(s[2]["step"] == "step_down" for s in sb):
                healed += 1
            if not (end and oa and ob and first_read):
                fails.append(f"run {n}: end {end} ({dt:.2f} s; A {a.state()}, B {b.state()}), A order {oa} ({wa}), "
                             f"B order {ob} ({wb}), first read after claim {first_read}; A's steps "
                             f"{[x[2] for x in sa]}, reads {[r[1:4] + [r[4]] for r in a.reads()]}; B's steps "
                             f"{[x[2] for x in sb]}, reads {[r[1:4] + [r[4]] for r in b.reads()]}")
            a.close()
            b.close()
            bus.insts.clear()
            time.sleep(0.3)
        rp.check(sec, f"1. in every one of {runs} runs, within 5 s plus two graces of the later start, one serves "
                      "and the other stands by: A, or B where B's read missed A's claim and A's then showed B "
                      "holding (§2.5 step 2: a live incumbent wins; scenarios.md's \"A serves\" in every run "
                      "contradicts it, F-203); each descriptor answers marked at its instance token (core "
                      "presence.md §1's observation, F-202); each first read follows its own claim",
                 not fails, "; ".join(fails[:3]))
        rp.note(sec, f"B served, A standing by on it, in {len(b_won)} of {runs} runs ({b_won})")
        rp.note(sec, f"{healed} of {runs} runs needed the heal (both took over, then B stepped down); "
                     f"the slowest run settled in {worst:.2f} s; {own_seen[0]} of {own_seen[1]} first reads held "
                     "their own claim")
    finally:
        bus.close()


# -- §4 ------------------------------------------------------------------------------

def section4(rp: Report, runs: int) -> None:
    sec = "§4 a live incumbent, and a lower newcomer"
    bus = Bus()
    try:
        b = bus.inst("B", B_ID)
        _wait(b.serving, 5.0)
        t = time.monotonic()
        a = bus.inst("A", A_ID)
        _wait(a.standing, 5.0)
        time.sleep(10.0)
        a_alive = bus.s.token_times("put", ALIVE + A_ID, t)
        steps_b = [s[2]["step"] for s in b.steps()]
        steps_a = a.steps()
        rp.check(sec, "1. A stands by: no interface token of A's appears at any time",
                 a.standing() and not a_alive,
                 f"A {a.state()}, its alive tokens {len(a_alive)}, its stand-by "
                 f"{[s[2] for s in steps_a if s[2]['step'] == 'stand_by'][:1]}")
        rp.check(sec, "2. B still serves, and never stepped down: a live incumbent wins whatever the newcomer's id",
                 b.serving() and "step_down" not in steps_b, f"B {b.state()}, steps {steps_b}")
    finally:
        bus.close()


# -- §5 ------------------------------------------------------------------------------

def _router(port: int, acl: bool = False) -> Any:
    import zenoh

    conf = zenoh.Config()
    conf.insert_json5("mode", json.dumps("router"))
    conf.insert_json5("listen/endpoints", json.dumps([f"tcp/127.0.0.1:{port}"]))
    conf.insert_json5("scouting/multicast/enabled", "false")
    conf.insert_json5("timestamping/enabled", "true")
    if acl:
        # core §8.1 (0.8): a liveliness_query deny on the query's ingress
        conf.insert_json5("access_control", json.dumps({
            "enabled": True, "default_permission": "allow",
            "rules": [{"id": "no-presence-reads", "messages": ["liveliness_query"], "flows": ["ingress"],
                       "permission": "deny", "key_exprs": ["zk2/*/*/@zk/**"]}],
            "subjects": [{"id": "everyone"}],
            "policies": [{"id": "deny", "rules": ["no-presence-reads"], "subjects": ["everyone"]}],
        }))
    return zenoh.open(conf)


def section5(rp: Report, runs: int) -> None:
    from .owner import free_loopback_port

    sec = "§5 an unreadable claim set"
    port = free_loopback_port()
    ep = f"tcp/127.0.0.1:{port}"
    a = Inst(LEDGER, ep, instance=A_ID, name="A", flags=["--peer"])
    bus = None
    try:
        time.sleep(4.5)
        reads = a.reads()
        steps = [s[2] for s in a.steps()]
        not_made = [r for r in reads if r[1] == "not made"]
        rp.check(sec, "1. A, a peer of R1 while R1 is down, makes no read through its cut-off session and stands "
                      "by: its reads not made (cut off), never \"the only candidate\"",
                 reads and len(not_made) == len(reads) and a.standing() and
                 all(s["step"] != "take_over" for s in steps) and all("cut off" in r[2] for r in not_made),
                 f"{len(reads)} reads, {len(not_made)} not made ({not_made[0][2][:40] if not_made else ''}…), "
                 f"stand-bys {[s.get('why') for s in steps if s['step'] == 'stand_by']}")
        r1 = _router(port)
        t = time.monotonic()
        bus = Bus(r1=(r1, ep))
        served = _wait(a.serving, 2.0 + 2.0 + 0.5, 0.05)
        dt = time.monotonic() - t
        last = a.reads()[-1] if a.reads() else None
        rp.check(sec, "2. within the retry interval plus 2 s of R1's start, A's next campaign reads complete, "
                      "finds its own claim, and A serves",
                 served and dt <= 4.5 and last is not None and last[3] and A_ID in [i[0] for i in last[4]],
                 f"served {dt:.2f} s after R1 started; last read {last[1:4] if last else None}")
        a.close()
        bus.close()
        bus = None
        port3 = free_loopback_port()
        r3 = _router(port3, acl=True)
        try:
            a3 = Inst(LEDGER, f"tcp/127.0.0.1:{port3}", instance=A_ID, name="A")
            time.sleep(10.0)
            reads = a3.reads()
            steps = [s[2] for s in a3.steps()]
            stand = [s.get("why") for s in steps if s["step"] == "stand_by"]
            rp.check(sec, "3. every read through R3, whose access control denies liveliness_query, is complete and "
                          "empty, its own claim missing: A stands by, campaigns again at each retry interval, and "
                          "never serves within the 10 s",
                     reads and all(r[3] and not r[4] for r in reads) and not a3.serving()
                     and set(stand) == {"own_claim_missing"} and len(reads) >= 4,
                     f"{len(reads)} reads, complete and empty {sum(1 for r in reads if r[3] and not r[4])}, "
                     f"stand-bys {set(stand)}")
            a3.close()
        finally:
            r3.close()
        bus = Bus()
        t = time.monotonic()
        ctl = bus.inst("A", A_ID)
        served = _wait(ctl.serving, 3.0, 0.05)
        rp.check(sec, "3. the control, the same instance as a client of a router with no access control, serves "
                      "within 3 s", served, f"{time.monotonic() - t:.2f} s")
    finally:
        try:
            a.kill()
        except Exception:  # noqa: BLE001
            pass
        if bus is not None:
            bus.close()


# -- §6 ------------------------------------------------------------------------------

def section6(rp: Report, runs: int) -> None:
    from .budget_scenarios import Link

    sec = "§6 a withdrawal that is no vacancy"
    bus = Bus(cut=True)
    link = None
    try:
        a = bus.inst("A", A_ID, on=1)
        _wait(a.serving, 5.0)
        link = Link(bus.ep2)
        b = bus.inst("B", B_ID, endpoint=link.endpoint)
        _wait(b.standing, 8.0)
        t_cut = time.monotonic()
        link.cut()
        time.sleep(1.0)
        link.heal()
        t_heal = time.monotonic()
        time.sleep(6.0)
        steps = b.steps()
        reads = b.reads()
        heard = [s for s in steps if s[1] == "withdrawn" and s[0] >= t_cut]
        during = [r for r in reads if t_cut <= r[0] < t_heal]
        after = [r for r in reads if r[0] >= t_heal]
        shows_a = [r for r in after if r[3] and any(i[0] == A_ID and i[1] for i in r[4])]
        rp.check(sec, "1. B's watch hears A's tokens withdrawn when its link drops", bool(heard),
                 f"{len(heard)} withdrawals handed to its machine")
        rp.check(sec, "1. B makes no read while its session reaches no node: each read in the cut not made",
                 all(r[1] in ("not made", "possibly incomplete") and not r[3] for r in during),
                 f"reads in the cut {[(r[1], r[2][:30]) for r in during]}")
        rp.check(sec, "1. once restored, its read shows A's interface token, and B stands by",
                 bool(shows_a) and b.standing(), f"{len(after)} reads after, {len(shows_a)} showing A; B {b.state()}")
        b_alive = bus.s.token_times("put", ALIVE + B_ID, 0.0)
        rp.check(sec, "1. B never takes over, and no interface token of B's appears at R1 (S) at any time",
                 not b_alive and all(s[2]["step"] != "take_over" for s in steps), f"B's alive at S {len(b_alive)}")
    finally:
        if link is not None:
            link.close()
        bus.close()


# -- §7 and §9: the split -------------------------------------------------------------

def _split(bus: Bus, rp: Report, sec: str, b_flags: list[str] | None = None) -> tuple[Inst, Inst, bool]:
    a = bus.inst("A", A_ID, on=1)
    _wait(a.serving, 5.0)
    b = bus.inst("B", B_ID, on=2, flags=b_flags)
    _wait(b.standing, 8.0)
    bus.link.cut()
    took = _wait(lambda: bus.s2.first_token("put", ALIVE + B_ID) is not None, 25.0, 0.05)
    return a, b, took


def section7(rp: Report, runs: int) -> None:
    from . import live

    sec = "§7 the heal of a split"
    bus = Bus(cut=True)
    try:
        t0 = time.monotonic()
        a, b, took = _split(bus, rp, sec)
        a_gone_s2 = bus.s2.first_token("delete", ALIVE + A_ID, t0) is not None
        b_at_s = bus.s.token_times("put", ALIVE + B_ID, 0.0)
        rp.check(sec, "1. through S2: A's tokens withdrawn, and B, finding no holder, takes over; through S: A still "
                      "serves, and sees nothing of B's interface token",
                 took and a_gone_s2 and a.serving() and not b_at_s,
                 f"B took over {took}, A gone at S2 {a_gone_s2}, A {a.state()}, B's alive at S {len(b_at_s)}")
        rp.check(sec, "2. two incumbents, one on each side: the partition's split", a.serving() and b.serving(),
                 f"A {a.state()}, B {b.state()}")
        tp = time.monotonic()
        a.put("balance", "a1")
        b.put("balance", "b1")
        time.sleep(0.5)
        s_vals = [w[3] for w in bus.s.writes(BALANCE, tp)]
        s2_vals = [w[3] for w in bus.s2.writes(BALANCE, tp)]
        rp.check(sec, "3. each side's subscribers receive their own incumbent's put",
                 s_vals == [b"a1"] and s2_vals == [b"b1"], f"S {s_vals}, S2 {s2_vals}")
        zid_b = b.zid()
        h = time.monotonic()
        bus.link.heal()
        both = _wait(lambda: bus.s.first_token("put", ALIVE + B_ID, h) is not None
                     and bus.s2.first_token("put", ALIVE + A_ID, h) is not None, 20.0, 0.05)
        down = _wait(b.standing, 2 * 2.0 + 2.0 + 6.0, 0.05)
        t_down = time.monotonic()
        rp.check(sec, "4. after H, S and S2 each see both interface tokens", both)
        _wait(lambda: any(m for _, m in bus.s.descriptors(LEDGER, B_ID, h)), 3.0, 0.02)
        w_b = bus.s.first_token("delete", ALIVE + B_ID, h)
        marked = [t for t, m in bus.s.descriptors(LEDGER, B_ID, h) if m]
        steps_b = b.steps()
        sd = [s for s in steps_b if s[2]["step"] == "step_down"]
        since_link = None if not sd else sd[0][0] - h
        rp.check(sec, "4. B, the higher, steps down within two graces plus 2 s of the heal's arrival",
                 down and bool(sd) and sd[0][2].get("instance") == A_ID,
                 f"step_down {sd[0][2] if sd else None}, {None if since_link is None else round(since_link, 2)} s "
                 f"after H (the link's reconnection is zenoh's, within {t_down - h:.1f} s)")
        rp.check(sec, "4. in §2.9's order: its interface token withdrawn first, then its descriptor put marked "
                      "standby", w_b is not None and bool(marked) and w_b < marked[0],
                 f"token gone +{None if w_b is None else round(w_b - h, 3)}, marked +"
                 f"{round(marked[0] - h, 3) if marked else None}")
        st = live.get_state(bus.g, "zk2/lab/ledger/ledger.v1/**")
        zid_a = a.zid()
        ids = {None if r.stamp is None else r.stamp.split("/", 1)[1].lstrip("0") for r in st.replies}
        rp.check(sec, "4. its queryables gone: an All GET on zk2/lab/ledger/ledger.v1/** is answered by A alone",
                 ids == {(zid_a or "").lstrip("0")}, f"reply stamps by {ids}")
        b_after = [x for x in bus.s.writes("zk2/lab/ledger/ledger.v1/", w_b or h)
                   if x[4] is not None and zid_b and x[4].split("/", 1)[1].lstrip("0") == zid_b.lstrip("0")]
        rp.check(sec, "4. B puts and deletes none of ledger.v1's keys from the withdrawal of its token on",
                 not b_after, f"{len(b_after)} writes stamped with B's zid")
        time.sleep(3.0)
        rp.check(sec, "4. A never steps down, and stands alone as the incumbent",
                 a.serving() and all(s[2]["step"] != "step_down" for s in a.steps())
                 and bus.holders(1)[1] == {A_ID}, f"A {a.state()}, holders {bus.holders(1)[1]}")
    finally:
        bus.close()


def section9(rp: Report, runs: int) -> None:
    import zenoh

    from . import live
    from .redundancy_live import Follower

    sec = "§9 a consumer following the incumbent during a split"
    bus = Bus(cut=True)
    k = None
    try:
        a, b, took = _split(bus, rp, sec, b_flags=["--hold-heal"])
        zid_a, zid_b = a.zid(), b.zid()
        p_seen: list[tuple[bytes, str]] = []
        p_last: dict[str, Any] = {}

        def p_sample(smp: Any) -> None:
            ts = smp.timestamp
            if ts is None:
                return
            sid = str(ts).split("/", 1)[1]
            # core S7 alone: the first value under a new id, else a later stamp
            if p_last.get("id") != sid or ts.get_time_as_ntp64().as_nanos() > p_last.get("t", 0):
                p_last.update(id=sid, t=ts.get_time_as_ntp64().as_nanos())
                p_seen.append((smp.payload.to_bytes(), sid))

        ps = live.open_client(bus.ep1)
        bus.extra.append(ps)
        bus.extra.append(ps.declare_subscriber(BALANCE, zenoh.handlers.Callback(p_sample)))
        h = time.monotonic()
        bus.link.heal()
        _wait(lambda: bus.s.first_token("put", ALIVE + B_ID, h) is not None, 20.0, 0.05)
        time.sleep(0.5)
        for n in range(1, 6):
            a.put("balance", f"a{n}")
            time.sleep(0.1)
            b.put("balance", f"b{n}")
            time.sleep(0.1)
            if n == 3:
                if k is not None:
                    k.close()
                k = Follower(bus.ep1, LEDGER, "ledger.v1", "balance")
                seeded = k.seed()
        time.sleep(0.5)
        vals = [v for v, _ in p_seen]
        flips = sum(1 for x, y in zip(p_seen, p_seen[1:]) if x[1] != y[1])
        rp.check(sec, "1. P, reading by core S7 alone, moves at each put between A's and B's values",
                 flips >= 6 and any(v.startswith(b"a") for v in vals) and any(v.startswith(b"b") for v in vals),
                 f"{len(vals)} values, {flips} flips between the two ids")
        acc = [v for _, v, _ in k.accepted] if k else []
        disc = [v for _, v, _ in k.discarded] if k else []
        rp.check(sec, "1. K's value moves to A's values only, in A's order, and K discards every sample stamped "
                      "with B's meta.zid", k is not None and acc == [b"a4", b"a5"]
                 and all(v.startswith(b"b") for v in disc) and len(disc) >= 2, f"accepted {acc}, discarded {disc}")
        rp.check(sec, "2. K's seed holds A's reply, or, where Latest kept B's later stamp, no value for balance "
                      "until A's next put: never B's", k is not None and seeded in ("accepted", "discarded")
                 and (k.value is None or k.value.startswith(b"a")), f"seed {seeded}")
        b.cmd("release-heal")
        down = _wait(b.standing, 10.0, 0.05)
        time.sleep(1.0)
        a.put("balance", "a6")
        time.sleep(0.5)
        rp.check(sec, "3. B steps down; K's value stays A's", down and k is not None and k.value == b"a6",
                 f"B {b.state()}, K {k.value if k else None}")
        rp.note(sec, f"A's zid {zid_a}, B's {zid_b}; K followed the lowest holder it saw, {A_ID}")
    finally:
        if k is not None:
            k.close()
        bus.close()


# -- §8 ------------------------------------------------------------------------------

def section8(rp: Report, runs: int) -> None:
    from . import live

    sec = "§8 a call finds no standby"
    bus = Bus()
    try:
        a = bus.inst("A", A_ID)
        _wait(a.serving, 5.0)
        b = bus.inst("B", B_ID)
        _wait(b.standing, 5.0)
        got = [live.call(bus.g, APPEND, b"x") for _ in range(20)]
        rp.check(sec, "1. 20 calls, BestMatching and None: 20 executions on A, none on B",
                 all([r.payload for r in c.replies] == [b"A"] for c in got) and a.executions() == 20
                 and b.executions() == 0, f"A {a.executions()}, B {b.executions()}")
        fan = live.call(bus.g, APPEND, b"x", fanout=True)
        rp.check(sec, "2. an All GET with consolidation None on the concrete key: one reply, A's",
                 [(r.kind, r.payload) for r in fan.replies] == [("value", b"A")], f"{fan.replies}")
        a.close()
        late = live.call(bus.g, APPEND, b"x")
        rp.check(sec, "3. a call at once after A's clean close, before B serves: no value and no envelope from B",
                 all(r.payload != b"B" and r.kind != "envelope" for r in late.replies),
                 f"{[(r.kind, r.payload[:20]) for r in late.replies] or 'silence'}")
    finally:
        bus.close()


# -- §10 -----------------------------------------------------------------------------

def section10(rp: Report, runs: int) -> None:
    from . import live

    sec = "§10 external"
    va, vb = "0000000000000f20", "0000000000000f10"  # V1's id, then V2's, lower
    alive = "zk2/lab/valve/@zk/alive/valve.v1/"
    tmp = tempfile.mkdtemp(prefix="zk2py-valve-")
    lock = os.path.join(tmp, "valve.lock")
    bus = Bus()
    poller_stop = threading.Event()
    both_seen: list[float] = []

    def poll() -> None:
        while not poller_stop.wait(1.0):
            _, hs = bus.holders(1, VALVE, "valve.v1")
            if {va, vb} <= hs:
                both_seen.append(time.monotonic())

    silent = SilentLink(bus.ep1)
    try:
        t = time.monotonic()
        # V1 behind a link that can go silent, so that its kill in step 3 is
        # a crash seen at the lease, as the step expects (F-201)
        v1 = bus.inst("V1", va, address=VALVE, contract="valve.v1", flags=["--lock", lock],
                      endpoint=silent.endpoint)
        up = _wait(v1.serving, 5.0)
        ok, why = _ordered(bus, v1, t, serving=True)
        rp.check(sec, "1. V1 takes the lock and serves, in §2.6's order", up and ok, why)
        threading.Thread(target=poll, daemon=True).start()
        v2 = bus.inst("V2", vb, address=VALVE, contract="valve.v1", flags=["--lock", lock])
        time.sleep(10.0)
        st2 = [s[2] for s in v2.steps()]
        marked = [m for _, m in bus.s.descriptors(VALVE, vb)]
        rp.check(sec, "2. V2 stands by, its descriptor marked: no election by ids moves it, though its id is lower",
                 v2.standing() and all(marked) and marked and
                 [s for s in st2 if s["step"] == "stand_by"][:1] == [{"step": "stand_by", "why": "no_lock",
                                                                     "retry": False}],
                 f"V2 {v2.state()}, marks {marked}, steps {[s['step'] for s in st2]}")
        silent.freeze()
        k = v1.kill()
        w = _wait(lambda: bus.s.first_token("delete", alive + va, k) is not None, 20.0, 0.05)
        t_w = bus.s.first_token("delete", alive + va, k)
        took = _wait(lambda: bus.s.first_token("put", alive + vb, k) is not None, 5.0, 0.02)
        t_b = bus.s.first_token("put", alive + vb, k)
        st2 = [s for s in v2.steps() if s[0] >= k]
        lock_at = [s[0] for s in st2 if s[1] == "lock"]
        rp.check(sec, "3. (V1 killed behind its silent link) V2 takes the lock at once, waits for V1's interface "
                      "token to go (V1's lease, 10 s after K), and serves within 3 s of that withdrawal",
                 w and took and bool(lock_at) and lock_at[0] - k < 1.0 and t_b is not None and t_w is not None
                 and t_b - t_w <= 3.0 and t_b > t_w and 7.0 <= t_w - k <= 13.0
                 and any(s[2]["step"] == "stand_by" and s[2].get("why") == "incumbent" for s in st2),
                 f"lock +{round(lock_at[0] - k, 3) if lock_at else None} s, V1 gone +"
                 f"{None if t_w is None else round(t_w - k, 2)} s, V2 serving +"
                 f"{None if t_b is None else round(t_b - k, 2)} s; steps {[s[2]['step'] for s in st2]}")
        rp.check(sec, "3. at no time do two reads show V1 and V2 both holding", not both_seen,
                 f"{len(both_seen)} reads showed both")
        poller_stop.set()
        v2.close()
        time.sleep(0.5)
        # 4. two lock files, V2 first
        l1, l2 = os.path.join(tmp, "v1.lock"), os.path.join(tmp, "v2.lock")
        v2 = bus.inst("V2", vb, address=VALVE, contract="valve.v1", flags=["--lock", l2])
        _wait(v2.serving, 5.0)
        v1 = bus.inst("V1", va, address=VALVE, contract="valve.v1", flags=["--lock", l1])
        time.sleep(10.0)
        st1 = [s[2] for s in v1.steps()]
        rp.check(sec, "4. two lock files, V2 first: V2 serves; V1, holding its own lock, stands by on V2's token, "
                      "as §2.11 and elections.json say (\"once a read shows no other holder\"); the scenario's "
                      "Expected 4, both serving, contradicts them (F-200)",
                 v2.serving() and v1.standing() and
                 any(s["step"] == "stand_by" and s.get("why") == "incumbent" for s in st1),
                 f"V2 {v2.state()}, V1 {v1.state()}, V1's stand-bys "
                 f"{[s.get('why') for s in st1 if s['step'] == 'stand_by']}")
        v1.close()
        v2.close()
        time.sleep(0.5)
        # 5. V3 refuses while V1 holds the lock; a control starts
        v1 = bus.inst("V1", va, address=VALVE, contract="valve.v1", flags=["--lock", lock])
        _wait(v1.serving, 5.0)
        t = time.monotonic()
        v3 = bus.inst("V3", "0000000000000f30", address=VALVE, contract="valve.v1",
                      flags=["--lock", lock, "--refuse-without-lock"])
        ctl = bus.inst("C", "0000000000000f40", address=VALVE, contract="valve.v1",
                       flags=["--lock", os.path.join(tmp, "ctl.lock")])
        time.sleep(5.0)
        v3_tok = bus.s.token_times("put", "zk2/lab/valve/@zk/instance/0000000000000f30", t)
        ctl_tok = bus.s.token_times("put", "zk2/lab/valve/@zk/instance/0000000000000f40", t)
        rp.check(sec, "5. V3, configured to refuse rather than stand by, refuses to start while V1 holds the lock: "
                      "no instance token of V3's within 5 s, against a control whose token appears",
                 v3.refused is not None and v3.proc.poll() == 2 and not v3_tok and bool(ctl_tok),
                 f"V3 {v3.refused!r} exit {v3.proc.poll()}, its tokens {len(v3_tok)}, the control's {len(ctl_tok)}")
    finally:
        poller_stop.set()
        silent.close()
        bus.close()


# -- §11 -----------------------------------------------------------------------------

def section11(rp: Report, runs: int) -> None:
    sec = "§11 the tokenless set, and an invalid declaration"
    bus = Bus()
    try:
        t = time.monotonic()
        ctl = bus.inst("A", A_ID)
        rp.check(sec, "control: A configured plainly serves within 3 s", _wait(ctl.serving, 3.0),
                 f"{time.monotonic() - t:.2f} s")
        ctl.close()
        time.sleep(0.5)
        for step, kw, why in (("1", {"flags": ["--tokenless", "ledger.v1"]}, "tokenless"),
                              ("2", {"contract": "ledger-vote.v1"}, "value")):
            t = time.monotonic()
            x = bus.inst("A", "00000000000000a9", **kw)
            time.sleep(5.0)
            toks = bus.s.token_times("put", "zk2/lab/ledger/@zk/instance/00000000000000a9", t)
            writes = bus.s.writes("zk2/lab/ledger/", t)
            rp.check(sec, f"{step}. A refuses to start: no instance token within 5 s, nothing of lab/ledger declared "
                          f"or put ({why})",
                     x.refused is not None and why in x.refused and not toks and not writes and x.proc.poll() == 2,
                     f"{x.refused!r}; tokens {len(toks)}, puts {len(writes)}")
    finally:
        bus.close()


# -- §12 -----------------------------------------------------------------------------

class X:
    """A test session that declares tokens under ``lab/ledger`` and serves a
    descriptor it writes (scenarios.md §12, "X")."""

    def __init__(self, endpoint: str, instance: str, *, standby: bool | None, alive: bool,
                 fingerprint: str) -> None:
        import zenoh

        from . import live

        self.session = live.open_client(endpoint)
        key = f"zk2/lab/ledger/@zk/instance/{instance}"
        entry: dict[str, Any] = {"iface": "ledger.v1", "contract": fingerprint, "minor": 0, "token": True}
        if standby:
            entry["standby"] = True
        doc = json.dumps({"format": "zk2-descriptor/0.1", "service": "lab/ledger", "instance": instance,
                          "interfaces": [entry], "profiles": ["redundancy.v1"],
                          "meta": {"zid": str(self.session.zid())}}).encode()
        self.entities = [self.session.declare_queryable(key, zenoh.handlers.Callback(
            lambda q: q.reply(key, doc, encoding="application/json")), complete=True)]
        self.session.put(key, doc, encoding="application/json")
        lv = self.session.liveliness()
        self.entities.append(lv.declare_token(key))
        if alive:
            self.entities.append(lv.declare_token(
                f"zk2/lab/ledger/@zk/alive/ledger.v1/{instance}/{fingerprint.removeprefix('sha256:')[:16]}"))

    def close(self) -> None:
        from . import live

        for e in reversed(self.entities):
            try:
                e.undeclare()
            except Exception:  # noqa: BLE001
                pass
        live.close(self.session)


def section12(rp: Report, runs: int) -> None:
    from . import live, redundancy as rd
    from .redundancy_live import contract_at, tool_reading

    sec = "§12 a tool's questions"
    ledger = contract_at("ledger.v1")  # the tool's offline contract (core §8.4)
    fp = ledger.fingerprint

    def ask(bus: Bus) -> tuple[dict[str, Any], Any]:
        reading, ps = tool_reading(bus.g, LEDGER, "ledger.v1", grace_s=2.0, contracts=[ledger])
        c = rd.conclude(reading)
        flat = {"one": c["one_incumbent"].as_json(), "vacancy": c["vacancy"].as_json(),
                "standbys": c["standbys"].as_json(), **{k: v.as_json() for k, v in c["twins"].items()}}
        return flat, reading

    def pair(a: dict[str, Any]) -> tuple[str, str]:
        return a["answer"], a["reason"]

    bus = Bus()
    try:
        a = bus.inst("A", A_ID)
        _wait(a.serving, 5.0)
        b = bus.inst("B", B_ID)
        _wait(b.standing, 5.0)
        got, reading = ask(bus)
        rp.check(sec, "1. exactly one: yes, one; a vacancy: no, held; standbys: yes, 1, exact; A: no, incumbent; "
                      "B: no, standby",
                 pair(got["one"]) == ("yes", "one") and pair(got["vacancy"]) == ("no", "held")
                 and got["standbys"] == {"answer": "yes", "reason": "standing_by", "count": 1, "exact": True}
                 and pair(got[A_ID]) == ("no", "incumbent") and pair(got[B_ID]) == ("no", "standby")
                 and reading.governance == "claim", f"{got}")
        faults, passing = live.presence_faults(bus.g, "zk2/lab/*/@zk/**")
        rp.check(sec, "7. on step 1's shape, zk2py's tool reports no split and no exposed interface without a token "
                      "for B (core §6, 0.39: its listing claims nothing exposed)",
                 not any(f[1] == "no token" for f in faults) and pair(got["one"]) != ("no", "split"),
                 f"faults {sorted(faults)}")
        b.close()
        a.close()
        time.sleep(0.5)
        # 3. B stands by and no instance serves: X plays it, marked
        x = X(bus.ep1, B_ID, standby=True, alive=False, fingerprint=fp)
        time.sleep(0.5)
        got, _ = ask(bus)
        rp.check(sec, "3. exactly one: no, vacant; a vacancy: yes, lasting",
                 pair(got["one"]) == ("no", "vacant") and pair(got["vacancy"]) == ("yes", "lasting"), f"{got}")
        x.close()
        # 4. A serves; X lists ledger.v1 unmarked with no interface token
        a = bus.inst("A", A_ID)
        _wait(a.serving, 5.0)
        x = X(bus.ep1, B_ID, standby=False, alive=False, fingerprint=fp)
        time.sleep(0.5)
        got, _ = ask(bus)
        rp.check(sec, "4. X: yes, unmarked", pair(got[B_ID]) == ("yes", "unmarked"), f"{got}")
        x.close()
        a.close()
        time.sleep(0.5)
        # 5. no instance; X holds an interface token, its descriptor marked
        x = X(bus.ep1, C_ID, standby=True, alive=True, fingerprint=fp)
        time.sleep(0.5)
        got, _ = ask(bus)
        rp.check(sec, "5. X: yes, marked_holding; exactly one incumbent: yes, one (a holder is an incumbent "
                      "whatever its mark)",
                 pair(got[C_ID]) == ("yes", "marked_holding") and pair(got["one"]) == ("yes", "one"), f"{got}")
        faults, _ = live.presence_faults(bus.g, "zk2/lab/*/@zk/**")
        rp.check(sec, "5. and core §6's shape: a token for an interface its descriptor says it exposes nothing of",
                 any(f[1] == "standby holding" for f in faults), f"faults {sorted(faults)}")
        x.close()
    finally:
        bus.close()
    # 2. the split, T a client of R1 after the heal, B's step-down held
    bus = Bus(cut=True)
    try:
        a, b, took = _split(bus, rp, sec, b_flags=["--hold-heal"])
        h = time.monotonic()
        bus.link.heal()
        _wait(lambda: bus.s.first_token("put", ALIVE + B_ID, h) is not None, 20.0, 0.05)
        time.sleep(0.5)
        got, _ = ask(bus)
        rp.check(sec, "2. the split: exactly one: no, split; A: yes, split_lowest; B: yes, split",
                 pair(got["one"]) == ("no", "split") and pair(got[A_ID]) == ("yes", "split_lowest")
                 and pair(got[B_ID]) == ("yes", "split"), f"{got}")
        rp.check(sec, "7. on step 2's shape, one split-brain finding naming A and B (§6's two reads a grace apart)",
                 pair(got["one"]) == ("no", "split") and got[A_ID]["answer"] == got[B_ID]["answer"] == "yes",
                 f"A {got[A_ID]}, B {got[B_ID]}")
        b.cmd("release-heal")
    finally:
        bus.close()
    rp.note(sec, "step 6 (hostid.v1 §2.12's recognised standby) runs as hostid_scenarios §6 step 6")


def section_remint(rp: Report, runs: int) -> None:
    """zk2py's own, beyond scenarios.md: §2.8's "A re-mint is no vacancy",
    from the incumbent's side too (F-209): A re-mints make-before-break to a
    lower id while B stands by."""
    sec = "re-mint: no vacancy, and no heal of itself"
    bus = Bus()
    try:
        a = bus.inst("A", A_ID)
        _wait(a.serving, 5.0)
        b = bus.inst("B", B_ID)
        _wait(b.standing, 5.0)
        new = "000000000000000a"
        t = time.monotonic()
        got = a.cmd(f"remint {new}")
        time.sleep(2 * 2.0 + 2.0)
        put_new = bus.s.first_token("put", ALIVE + new, t)
        gone_old = bus.s.first_token("delete", ALIVE + A_ID, t)
        rp.check(sec, "A's new interface token appears before its old one goes: an overlap, never a gap",
                 got == f"redundancy remint {new}" and put_new is not None and gone_old is not None
                 and put_new < gone_old, f"new +{None if put_new is None else round(put_new - t, 3)}, old gone +"
                                         f"{None if gone_old is None else round(gone_old - t, 3)}")
        steps_b = [s[2]["step"] for s in b.steps() if s[0] >= t]
        steps_a = [s[2]["step"] for s in a.steps() if s[0] >= t]
        rp.check(sec, "B hears no vacancy and never takes over", b.standing() and "take_over" not in steps_b,
                 f"B {b.state()}, its steps since {steps_b}")
        rp.check(sec, "A, told of its own re-mint, reads no heal and never steps down, serving under its new id",
                 a.serving() and "step_down" not in steps_a and "read" not in steps_a
                 and bus.holders(1)[1] == {new}, f"A {a.state()}, its steps since {steps_a}, "
                                                 f"holders {bus.holders(1)[1]}")
    finally:
        bus.close()


SECTIONS = {"1": section1, "2": section2, "3": section3, "4": section4, "5": section5, "6": section6,
            "7": section7, "8": section8, "9": section9, "10": section10, "11": section11, "12": section12,
            "remint": section_remint}


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="python -m zk2py.redundancy_scenarios", description=__doc__.split("\n")[0])
    ap.add_argument("--only", action="append", choices=list(SECTIONS))
    ap.add_argument("--runs", type=int, default=20, help="§3's runs (20 by the text)")
    a = ap.parse_args(argv)
    rp = Report()
    for name, fn in SECTIONS.items():
        if a.only and name not in a.only:
            continue
        try:
            fn(rp, a.runs)
        except Exception as e:  # noqa: BLE001 - a section that crashes fails, the rest run
            import traceback

            traceback.print_exc()
            rp.check(f"§{name}", "the section ran to its end", False, f"{type(e).__name__}: {e}")
    passed = sum(1 for r in rp.rows if r[2])
    failed = len(rp.rows) - passed
    print(f"redundancy scenarios: {passed} passed, {failed} failed")
    return 0 if failed == 0 else 1


if __name__ == "__main__":
    sys.exit(main())

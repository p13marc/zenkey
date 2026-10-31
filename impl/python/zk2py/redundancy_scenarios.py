"""``redundancy.v1`` ``scenarios.md`` §1 to §12 (text 0.3), live, from
Python (#609, rounds 56, 58 and 60)::

    python -m zk2py.redundancy_scenarios [--a zk2py|PATH] [--b zk2py|PATH] [--owner PATH]
        [--only 1|2|…|12|remint|poll|poll-miss|heal-retry|heal-retry-pause|remint-campaign]...
        [--runs N] [--zenctl PATH]

Each candidate is a process driven as a black box through the owner
example's command line and its ``redundancy`` commands, one JSON line each
(``owner --help``): zk2py's (``python -m zk2py.redundancy_owner``, which
takes the same command line since round 58) or the Rust owner example at a
path. ``--a`` runs A (and V1, V3), the lower id, ``--b`` runs B (and V2);
``--owner`` both. A crash is a SIGKILL; a lost host is a SIGKILL behind
:class:`SilentLink`, a relay that goes silent and keeps the router's side
open, with a short lease (scenarios.md, "A crash"). §12's tools are zk2py's
(:func:`zk2py.redundancy_live.read_tool`) and, given by ``--zenctl`` or
``ZK2PY_ZENCTL``, zenctl's ``redundancy``, ``doctor`` and ``check conform``,
each shape asked of both. §3 holds both candidates at ``--wait-go`` and
measures each start at S (0.3, F-216).

The bus is zk2py's: R1, and where a section cuts a link R2 behind a link
the runner cuts and heals. S follows ``zk2/lab/*/@zk/**`` with a liveliness
subscriber and subscribes to ``zk2/lab/**`` and the instance keys, all on
one session that makes no read, recording each delivery's arrival on the
host's monotonic clock, and GETs an instance's descriptor the moment it
hears each of its tokens ("before the token", F-202). S2 does the same on
R2. G reads presence on a session that has never held a liveliness
subscriber. The retry interval and the grace are 2 s.

What the owner example's standard input does not show is read from the
bus for both: its puts from their stamps (its ``put`` writes ``ok``, never
a chosen value), its step-down from its tokens. zk2py's own step log adds detail where it
has it, and says so.

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
CONTRACT_DIR = REPO / "impl" / "python" / "interop" / "redundancy"
LEDGER, VALVE = "lab/ledger", "lab/valve"
A_ID, B_ID, C_ID = "00000000000000a1", "00000000000000b2", "00000000000000c3"
ALIVE = "zk2/lab/ledger/@zk/alive/ledger.v1/"
BALANCE = "zk2/lab/ledger/ledger.v1/state/balance"
APPEND = "zk2/lab/ledger/ledger.v1/@op/append"
#: the section's short lease, where a crash is a lost host (scenarios.md, "A crash")
SHORT_LEASE_MS = 2000

#: --a and --b: "zk2py", or the owner example's path
IMPL: dict[str, Any] = {"a": "zk2py", "b": "zk2py"}
#: §3 holds both candidates at their own --wait-go (0.3, F-216): each opens
#: its sessions and waits for go; the owner example reads its standard input
#: from its own start since 0.4 (F-220), so --rust-spawn-at-go is gone.
#: §12's Rust tool, zenctl (``--zenctl`` or ``ZK2PY_ZENCTL``; no default)
ZENCTL: Path | None = Path(os.environ["ZK2PY_ZENCTL"]) if os.environ.get("ZK2PY_ZENCTL") else None


def who(impl: Any) -> str:
    return "zk2py" if impl == "zk2py" else "rust"


def pairing() -> str:
    return f"A {who(IMPL['a'])}, B {who(IMPL['b'])}"


def zid_value(z: Any) -> int | None:
    try:
        return int(str(z), 16)
    except (TypeError, ValueError):
        return None


def stamp_zid(stamp: str | None) -> int | None:
    return None if not stamp or "/" not in stamp else zid_value(stamp.split("/", 1)[1])


class Inst:
    """One candidate process, zk2py's driver or the owner example, launched
    with the owner example's command line and driven by its commands."""

    def __init__(self, impl: Any, address: str, endpoint: str, *, instance: str, name: str,
                 flags: list[str] | None = None, contract: str = "ledger.v1", g: Any = None) -> None:
        self.impl, self.rust = impl, impl != "zk2py"
        self.address, self.instance, self.name = address, instance, name
        self.iface = "valve.v1" if contract.startswith("valve") else "ledger.v1"
        self.g = g
        flags = list(flags or [])
        self.wait_go = "--wait-go" in flags
        args = ["--connect", endpoint, "--standby", "--instance-id", instance, "--name", name, "--retry-ms",
                "2000", "--grace-ms", "2000", *flags, address, str(CONTRACT_DIR / f"{contract}.toml")]
        self.argv = ([str(impl)] if self.rust else [sys.executable, "-m", "zk2py.redundancy_owner"]) + args
        self.proc: subprocess.Popen | None = None
        self.lines: queue.Queue[str] = queue.Queue()
        self.stderr: list[str] = []
        self.ready: str | None = None
        self.waiting = False
        self.refused: str | None = None
        self.t_launch = 0.0
        self._spawn()
        if not self.wait_go:
            self._await_ready()
        else:
            self._await_ready(line="waiting go")

    def _spawn(self) -> None:
        env = dict(os.environ, PYTHONPATH=str(REPO / "impl" / "python"))
        self.t_launch = time.monotonic()
        self.proc = subprocess.Popen(self.argv, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                     stderr=subprocess.PIPE, text=True, env=env)
        threading.Thread(target=lambda: [self.lines.put(x.rstrip("\n")) for x in self.proc.stdout],
                         daemon=True).start()
        threading.Thread(target=lambda: [self.stderr.append(x) for x in self.proc.stderr], daemon=True).start()

    def _await_ready(self, timeout: float = 20.0, line: str = "ready") -> None:
        """Until ``ready <key>`` (or, with ``line``, that line: ``waiting
        go``), or the process's refusal."""
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline and self.proc is not None:
            try:
                got = self.lines.get(timeout=0.2)
            except queue.Empty:
                if self.proc.poll() is not None:
                    time.sleep(0.2)
                    self.refused = "".join(self.stderr).strip() or f"exit {self.proc.poll()}"
                    return
                continue
            if line == "ready" and got.startswith("ready "):
                self.ready = got[len("ready "):]
                return
            if line != "ready" and got == line:
                self.waiting = True
                return

    def go(self) -> None:
        """--wait-go's start: a go line, its sessions already open."""
        self.send("go")

    def send(self, line: str) -> None:
        try:
            assert self.proc is not None
            self.proc.stdin.write(line + "\n")
            self.proc.stdin.flush()
        except (OSError, AssertionError):
            pass

    def cmd(self, line: str, timeout: float = 15.0) -> str | None:
        if self.ready is None and self.refused is None and self.proc is not None:
            self._await_ready()
        self.send(line)
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            try:
                got = self.lines.get(timeout=0.2)
            except queue.Empty:
                if self.proc is None or self.proc.poll() is not None:
                    return None
                continue
            if got.startswith(("ready ", "connected ", "listening ")) or got in ("go", "waiting go"):
                continue
            return got
        return None

    def jcmd(self, line: str, timeout: float = 15.0) -> dict[str, Any] | None:
        got = self.cmd(line, timeout)
        try:
            j = json.loads(got) if got else None
        except ValueError:
            return None
        return j if isinstance(j, dict) else None

    # -- the owner example's commands -------------------------------------
    def standing(self) -> dict[str, Any]:
        j = self.jcmd("redundancy role") or {}
        return (j.get("interfaces") or {}).get(self.iface) or {}

    def current_id(self) -> str | None:
        return (self.jcmd("redundancy role") or {}).get("instance")

    def state(self) -> str | None:
        st = self.standing()
        if st.get("role") == "serving":
            return "serving"
        if st.get("role") == "standby" and st.get("why"):
            return "standing_by"
        return st.get("role")

    def serving(self) -> bool:
        return self.state() == "serving"

    def stands(self) -> bool:
        return self.state() == "standing_by"

    def why(self) -> tuple[str | None, str | None]:
        st = self.standing()
        return st.get("why"), st.get("instance")

    def executions(self) -> int | None:
        return (self.jcmd("redundancy calls") or {}).get("calls")

    def read_counts(self) -> dict[str, Any]:
        j = self.jcmd("redundancy reads") or {}
        return (j.get("interfaces") or {}).get(self.iface) or {}

    def zid(self) -> str | None:
        """Its claim's session's zid, the one its descriptor states as
        ``meta.zid`` (core §3.3): ``redundancy zid`` (0.3, F-216)."""
        return (self.jcmd("redundancy zid") or {}).get("zid")

    def zids(self) -> dict[str, Any]:
        """Its three sessions' zids: the claim's, the reader's and the watch's."""
        return self.jcmd("redundancy zid") or {}

    def steps(self) -> list[Any] | None:
        """zk2py's own step log (the owner example shows none)."""
        return None if self.rust else (self.jcmd("redundancy steps") or {}).get("steps")

    def reads_log(self) -> list[Any] | None:
        return None if self.rust else (self.jcmd("redundancy log") or {}).get("reads")

    def put(self) -> str | None:
        """One write of its raw state, as both drive it: ``put <iface>
        state/<resource>``, the value ``ok``."""
        res = "state/balance" if self.iface == "ledger.v1" else "state/position"
        return self.cmd(f"put {self.iface} {res}")

    def release_heal(self) -> Any:
        return self.jcmd("redundancy hold-heal off")

    def remint(self) -> str | None:
        return (self.jcmd("redundancy new-epoch") or {}).get("instance")

    def lose_lock(self) -> Any:
        return self.jcmd("redundancy lock lose")

    def wait(self, role: str, ms: int) -> bool:
        j = self.jcmd(f"redundancy wait {self.iface} {role} {ms}", timeout=ms / 1000 + 10)
        return bool(j and j.get("reached"))

    def kill(self) -> float:
        t = time.monotonic()
        if self.proc is not None and self.proc.poll() is None:
            self.proc.send_signal(signal.SIGKILL)
            self.proc.wait(10)
        return t

    def close(self) -> float:
        """A clean stop: standard input closed."""
        t = time.monotonic()
        if self.proc is None:
            return t
        try:
            self.proc.stdin.close()
        except OSError:
            pass
        try:
            self.proc.wait(timeout=15)
        except subprocess.TimeoutExpired:
            self.kill()
        return t

    def exit_code(self) -> int | None:
        return None if self.proc is None else self.proc.poll()


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


class PauseLink(SilentLink):
    """A relay that pauses: while :meth:`pause`d it reads nothing either
    way, so what is sent waits in the kernel's buffers, and on
    :meth:`resume` it flows again, nothing lost. Shorter than either
    side's lease, the transport survives it; a read made across it runs to
    its timeout (observer §3.3, possibly incomplete)."""

    def __init__(self, upstream: str) -> None:
        self.go = threading.Event()
        self.go.set()
        super().__init__(upstream)

    def _pump(self, src: Any, dst: Any) -> None:
        try:
            while True:
                self.go.wait()
                data = src.recv(65536)
                if not data:
                    break
                self.go.wait()
                dst.sendall(data)
        except OSError:
            pass
        for x in (src, dst):
            try:
                x.close()
            except OSError:
                pass

    def pause(self) -> None:
        self.go.clear()

    def resume(self) -> None:
        self.go.set()


class Watcher:
    """S (or S2): tokens, data and descriptors on one session, which makes
    no presence read (observer §3.7's watch session), so that S sees one
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

    def inst(self, impl: Any, name: str, instance: str, on: int = 1, flags: list[str] | None = None,
             address: str = LEDGER, contract: str = "ledger.v1", endpoint: str | None = None) -> Inst:
        i = Inst(impl, address, endpoint or (self.ep1 if on == 1 else self.ep2), instance=instance, name=name,
                 flags=flags, contract=contract, g=self.g if on == 1 else self.g2)
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
    # §1 and §3 Expected 1 (0.4, F-223): "unless it took over before that GET
    # was answered: its interface token at S within a round trip of the
    # answer, the two riding two priorities". The round trip is that GET's
    # own, from S hearing the instance token (pi[0], taken in the token's
    # callback before the GET's thread starts) to the GET's end (pi[2],
    # taken once its replies are in): so pi[0] < pi[2] by construction,
    # the window never 0, and no floor is needed. The GET's end comes after
    # its answer, so the window is the round trip or a little more.
    late = False
    if pi is not None and pi[1] is False and pa is not None:
        assert pi[0] <= pi[2], "S's probe: the GET ends after the token that asked it"
        late = pa[0] <= pi[2] + (pi[2] - pi[0])
    ok = (pi is not None and (pi[1] is True or late)) if claim else True
    if serving or not claim:
        ok = ok and pa is not None and pa[1] is False
    descs = bus.s.descriptors(inst.address, inst.instance, after)
    order = "; S received " + ", ".join(f"{'marked' if m else 'unmarked'} +{t - after:.4f}" for t, m in descs[:3])
    times = (f" (instance token +{pi[0] - after:.4f}, its GET answered +{pi[2] - after:.4f}" if pi else " (") + \
        (f", interface token +{pa[0] - after:.4f})" if pa else ")")
    return ok, (f"at its instance token the descriptor read {'marked' if pi and pi[1] else pi and 'unmarked'}, "
                f"at its interface token {'unmarked' if pa and pa[1] is False else pa and 'marked'}{times}{order}")


# -- helpers ----------------------------------------------------------

def _router(port: int, acl: bool = False) -> Any:
    import zenoh

    conf = zenoh.Config()
    conf.insert_json5("mode", json.dumps("router"))
    conf.insert_json5("listen/endpoints", json.dumps([f"tcp/127.0.0.1:{port}"]))
    conf.insert_json5("scouting/multicast/enabled", "false")
    conf.insert_json5("timestamping/enabled", "true")
    if acl:
        # observer §3.4 (0.8): a liveliness_query deny on the query's ingress
        conf.insert_json5("access_control", json.dumps({
            "enabled": True, "default_permission": "allow",
            "rules": [{"id": "no-presence-reads", "messages": ["liveliness_query"], "flows": ["ingress"],
                       "permission": "deny", "key_exprs": ["zk2/*/*/@zk/**"]}],
            "subjects": [{"id": "everyone"}],
            "policies": [{"id": "deny", "rules": ["no-presence-reads"], "subjects": ["everyone"]}],
        }))
    return zenoh.open(conf)


class X:
    """A test session that declares tokens under ``lab/ledger`` and serves a
    descriptor it writes (scenarios.md §12, "X")."""

    def __init__(self, endpoint: str, instance: str, *, standby: bool | None, alive: bool,
                 fingerprint: str, queryable: bool = False) -> None:
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
        if queryable:
            # §12 step 8 (0.3): a queryable on append's key, which a standby
            # never holds (§2.4)
            op = "zk2/lab/ledger/ledger.v1/@op/append"
            self.entities.append(self.session.declare_queryable(op, zenoh.handlers.Callback(
                lambda q: q.reply(op, b"x")), complete=True))

    def close(self) -> None:
        from . import live

        for e in reversed(self.entities):
            try:
                e.undeclare()
            except Exception:  # noqa: BLE001
                pass
        live.close(self.session)


def _alive_of(inst: str) -> str:
    return ALIVE + inst


def _no_read_before(cand: Inst, k: float, until: float | None, samples: list[tuple[float, int]]) -> bool:
    """§1 Expected 4: "B made no read between K and the withdrawal": its
    reads counted at K, and every sample before the withdrawal the same."""
    before = [n for t, n in samples if until is None or t < until]
    return bool(before) and len(set(before)) == 1


def _sample_reads(cand: Inst, stop: threading.Event, out: list[tuple[float, int]]) -> None:
    while not stop.is_set():
        n = cand.read_counts().get("made")
        if isinstance(n, int):
            out.append((time.monotonic(), n))
        stop.wait(0.25)


# -- §1 ------------------------------------------------------------------------------

def section1(rp: Report, runs: int) -> None:
    from . import live

    sec = f"§1 takeover after a crash [{pairing()}]"
    for silent_run in (True, False):
        bus = Bus()
        silent = SilentLink(bus.ep1) if silent_run else None
        how = "its links silent, a short lease" if silent_run else "killed as it runs"
        try:
            t0 = time.monotonic()
            a = bus.inst(IMPL["a"], "A", A_ID, endpoint=silent.endpoint if silent else None,
                         flags=["--lease-ms", str(SHORT_LEASE_MS)] if silent_run else [])
            up = _wait(a.serving, 5.0, 0.05)
            ok, why = _ordered(bus, a, t0, serving=True)
            rp.check(sec, f"1. ({how}) A claims before it reads, its descriptor marked at its instance token, "
                          "unmarked at its interface token; A serves within 3 s of its start",
                     up and ok and time.monotonic() - a.t_launch <= 3.0 + 1.5, why)
            time.sleep(2.0)
            tb = time.monotonic()
            b = bus.inst(IMPL["b"], "B", B_ID)
            _wait(b.stands, 8.0, 0.1)
            ob, wb = _ordered(bus, b, tb)
            got = [live.call(bus.g, APPEND, b"x") for _ in range(20)]
            ea, eb = a.executions(), b.executions()
            rp.check(sec, f"2-3. ({how}) B stands by, its descriptor marked at its instance token; 20 calls, "
                          "20 executions on A, none on B",
                     b.stands() and ob and all([r.payload for r in c.replies] == [b"A"] for c in got)
                     and ea == 20 and eb == 0, f"B {b.standing()}; A {ea}, B {eb}; {wb}")
            samples: list[tuple[float, int]] = []
            stop = threading.Event()
            if silent is not None:
                silent.freeze()
            threading.Thread(target=_sample_reads, args=(b, stop, samples), daemon=True).start()
            time.sleep(0.3)
            k = a.kill()
            gone = _wait(lambda: bus.s.first_token("delete", _alive_of(A_ID), k) is not None, 25.0, 0.05)
            t_w = bus.s.first_token("delete", _alive_of(A_ID), k)
            stop.set()
            took = _wait(lambda: bus.s.first_token("put", _alive_of(B_ID), k) is not None, 5.0, 0.02)
            t_b = bus.s.first_token("put", _alive_of(B_ID), k)
            seen = None if t_w is None else t_w - k
            want = (SHORT_LEASE_MS / 1000 * 0.5, SHORT_LEASE_MS / 1000 + 2.0) if silent_run else (0.0, 1.0)
            rp.check(sec, f"4. ({how}) S hears A's tokens withdrawn " + (
                f"at A's lease ({SHORT_LEASE_MS} ms), give or take a keep-alive" if silent_run else "at once"),
                gone and seen is not None and want[0] <= seen <= want[1], f"{None if seen is None else round(seen, 3)} s after K")
            ok_b, why_b = _ordered(bus, b, k, claim=False)
            gap = None if t_b is None or t_w is None else t_b - t_w
            rp.check(sec, f"4. ({how}) B takes over within 3 s of the withdrawal, its descriptor unmarked at its "
                          "interface token", took and gap is not None and gap <= 3.0 and ok_b,
                     f"+{None if gap is None else round(gap, 3)} s; {why_b}")
            if silent_run:
                rp.check(sec, "4. B made no read between K and the withdrawal, and never polled (its reads "
                              "counted every 0.25 s)", _no_read_before(b, k, t_w, samples)
                         and (b.read_counts().get("polls") or 0) == 0, f"{[n for _, n in samples][:12]}")
            c = live.call(bus.g, APPEND, b"x")
            rp.check(sec, f"5. ({how}) the call executes on B", [r.payload for r in c.replies] == [b"B"],
                     f"{[r.payload for r in c.replies]}")
        finally:
            if silent is not None:
                silent.close()
            bus.close()


# -- §2 ------------------------------------------------------------------------------

def section2(rp: Report, runs: int) -> None:
    sec = f"§2 takeover after a clean stop [{pairing()}]"
    bus = Bus()
    try:
        a = bus.inst(IMPL["a"], "A", A_ID)
        _wait(a.serving, 5.0)
        b = bus.inst(IMPL["b"], "B", B_ID)
        _wait(b.stands, 8.0)
        za = zid_value(a.zid())
        q = a.close()
        gone = _wait(lambda: bus.s.first_token("delete", _alive_of(A_ID), q) is not None, 3.0, 0.01)
        t_w = bus.s.first_token("delete", _alive_of(A_ID), q)
        took = _wait(lambda: bus.s.first_token("put", _alive_of(B_ID), q) is not None, 3.0, 0.01)
        t_b = bus.s.first_token("put", _alive_of(B_ID), q)
        ok, why = _ordered(bus, b, q, claim=False)
        rp.check(sec, "1. A's interface token is withdrawn at once, without a lease",
                 gone and t_w is not None and t_w - q < 1.0, f"{None if t_w is None else round(t_w - q, 3)} s")
        rp.check(sec, "1. B serves within 2 s of Q, in §2.6's order", took and t_b is not None and t_b - q <= 2.0
                 and ok, f"{None if t_b is None else round(t_b - q, 3)} s; {why}")
        mine = [w for w in bus.s.writes("zk2/lab/ledger/ledger.v1/", q) if za is not None
                and stamp_zid(w[4]) == za]
        rp.check(sec, "1. A puts and deletes nothing of ledger.v1's keys while it closes", za is not None and not mine,
                 f"{len(mine)} writes stamped with A's zid after Q")
    finally:
        bus.close()


# -- §3 ------------------------------------------------------------------------------

def section3(rp: Report, runs: int) -> None:
    """§3 (0.3, F-216): "A and B start within 10 ms of each other, measured at
    S as the arrival of each instance's instance token … a run whose two
    instance tokens arrive more than 10 ms apart is made again, and counted
    apart". Each holds at its own ``--wait-go``, its sessions open, and both
    are sent go together (0.4, F-220: the owner example's too)."""
    sec = f"§3 a simultaneous start [{pairing()}]"
    bus = Bus()
    healed, b_kept, fails, worst, skews, again, apart, unseen = 0, 0, [], 0.0, [], 0, [], 0
    try:
        n = 0
        tries = 0
        while n < runs and tries < runs * 6:
            tries += 1
            ids = sorted(f"{random.getrandbits(64):016x}" for _ in range(2))
            a = bus.inst(IMPL["a"], "A", ids[0], flags=["--wait-go"])
            b = bus.inst(IMPL["b"], "B", ids[1], flags=["--wait-go"])
            t = time.monotonic()
            a.go()
            b.go()

            def settled() -> bool:
                # after a heal, a step-down's stand-by names no why: its role is enough
                if a.serving() and b.standing().get("role") == "standby":
                    return True
                return b.serving() and a.stands() and a.why() == ("incumbent", ids[1])

            end = _wait(settled, 5.0 + 2 * 2.0 + 2.0, 0.1)
            dt = time.monotonic() - t
            ia = bus.s.first_token("put", f"zk2/lab/ledger/@zk/instance/{ids[0]}", t)
            ib = bus.s.first_token("put", f"zk2/lab/ledger/@zk/instance/{ids[1]}", t)
            skew = abs(ia - ib) if ia is not None and ib is not None else None
            oa, wa = _ordered(bus, a, t - 0.1, serving=a.serving())
            ob, wb = _ordered(bus, b, t - 0.1, serving=b.serving())
            first_read = True
            for inst in (a, b):
                st = inst.steps()
                if st is not None:
                    names = [x[2]["step"] for x in st]
                    first_read = first_read and "read" in names and names.index("claim") < names.index("read")
            ok = end and oa and ob and first_read
            if not ok:
                with bus.s.lock:
                    line = [(round(tt - t, 3), kk, key.rsplit("/", 2)[-2][-2:]) for tt, kk, key in bus.s.tokens
                            if tt >= t and "/@zk/alive/" in key]
                fails.append(f"run {n}: end {end} ({dt:.2f} s; A {a.standing()}, B {b.standing()}), A {wa}, B {wb}, "
                             f"first read after claim {first_read}; tokens' skew {skew}; alive tokens at S {line}; "
                             f"A's reads {a.read_counts()}, B's {b.read_counts()}")
            if skew is None:
                # a token S never saw: no start measured, made again, counted apart
                unseen += 1
            elif skew > 0.010:
                # made again, and counted apart
                again += 1
                apart.append(skew)
            else:
                n += 1
                skews.append(skew)
                worst = max(worst, dt)
                if bus.s.first_token("put", ALIVE + ids[1], t) is not None and \
                        bus.s.first_token("delete", ALIVE + ids[1], t) and a.serving():
                    healed += 1
                if b.serving():
                    b_kept += 1
            a.close()
            b.close()
            bus.insts.clear()
            time.sleep(0.3)
        rp.check(sec, f"1. {runs} runs whose instance tokens reached S within 10 ms of each other, each within 5 s "
                      "plus two graces of the later start: one serves and the other stands by: A, or B where A's "
                      "read showed B's token (§2.5 step 2); each descriptor answers marked at its instance token "
                      "(F-202); each first read follows its own claim (zk2py's step log, where there is one); and "
                      "every run made again as well",
                 not fails and n == runs, f"{n} of {runs} within 10 ms after {tries} tries; " + "; ".join(fails[:2]))
        rp.note(sec, f"of the {n} runs within 10 ms: {healed} healed (both took over, B stepped down); B kept serving "
                     f"in {b_kept}; the slowest settled in {worst:.2f} s; the tokens' skew at most "
                     f"{max(skews) * 1000 if skews else 0:.1f} ms (median "
                     f"{sorted(skews)[len(skews) // 2] * 1000 if skews else 0:.1f} ms). Made again, counted apart: "
                     f"{again} more than 10 ms apart (skews {sorted(round(x * 1000, 1) for x in apart)[:12]} ms), "
                     f"{unseen} with an instance token S never saw")
    finally:
        bus.close()


# -- §4 ------------------------------------------------------------------------------

def section4(rp: Report, runs: int) -> None:
    sec = f"§4 a live incumbent, and a lower newcomer [{pairing()}]"
    bus = Bus()
    try:
        b = bus.inst(IMPL["b"], "B", B_ID)
        _wait(b.serving, 5.0)
        t = time.monotonic()
        a = bus.inst(IMPL["a"], "A", A_ID)
        _wait(a.stands, 8.0)
        time.sleep(10.0)
        rp.check(sec, "1. A stands by on B: no interface token of A's appears at any time",
                 a.why() == ("incumbent", B_ID) and not bus.s.token_times("put", _alive_of(A_ID), t),
                 f"A {a.standing()}")
        rp.check(sec, "2. B still serves, and never stepped down (its interface token never withdrawn)",
                 b.serving() and not bus.s.token_times("delete", _alive_of(B_ID), 0.0), f"B {b.standing()}")
    finally:
        bus.close()


# -- §5 ------------------------------------------------------------------------------

def section5(rp: Report, runs: int) -> None:
    from . import live
    from .owner import free_loopback_port

    sec = f"§5 an unreadable claim set [A {who(IMPL['a'])}]"
    port = free_loopback_port()
    ep = f"tcp/127.0.0.1:{port}"
    a = Inst(IMPL["a"], LEDGER, ep, instance=A_ID, name="A", flags=["--peer"])
    bus = None
    try:
        time.sleep(4.5)
        rc = a.read_counts()
        rp.check(sec, "1. A, its sessions peers of R1 while R1 is down, makes no read and stands by: its reads not "
                      "made, never \"the only candidate\"",
                 (rc.get("not_made") or 0) >= 1 and rc.get("made") == 0 and rc.get("complete") == 0
                 and a.why()[0] == "incomplete", f"reads {rc}, {a.standing()}")
        r1 = _router(port)
        t = time.monotonic()
        bus = Bus(r1=(r1, ep))
        served = _wait(a.serving, 2.0 + 1.0 + 2.0 + 0.5, 0.05)
        dt = time.monotonic() - t
        rp.check(sec, "2. within the retry interval, its reconnect (1 s at most) and 2 s of R1's start, A's next "
                      "campaign reads complete, finds its own claim, and A serves",
                 served and dt <= 5.5 and str(a.read_counts().get("last")).startswith("complete"),
                 f"served {dt:.2f} s after R1")
        a.close()
        bus.close()
        bus = None
        port3 = free_loopback_port()
        r3 = _router(port3, acl=True)
        try:
            a3 = Inst(IMPL["a"], LEDGER, f"tcp/127.0.0.1:{port3}", instance=A_ID, name="A")
            time.sleep(10.0)
            rc = a3.read_counts()
            rp.check(sec, "3. every read through R3, whose access control denies liveliness_query, is complete and "
                          "empty, its own claim missing: A stands by, campaigns at each retry interval, never serves",
                     (rc.get("made") or 0) >= 4 and rc.get("complete") == rc.get("made") and not a3.serving()
                     and a3.why()[0] == "own_claim_missing", f"reads {rc}, {a3.standing()}")
            log = a3.reads_log()
            if log is not None:
                rp.check(sec, "3. (zk2py's log) each of them empty", all(r[3] and not r[4] for r in log), f"{len(log)}")
            holder = live.open_client(f"tcp/127.0.0.1:{port3}")
            tok = holder.liveliness().declare_token("zk2/lab/ledger/@zk/instance/" + "e" * 16)
            other = live.open_client(f"tcp/127.0.0.1:{port3}")
            time.sleep(0.3)
            own = live.list_presence(holder, "zk2/lab/ledger/@zk/**")
            far = live.list_presence(other, "zk2/lab/ledger/@zk/**")
            rp.check(sec, "3. the same read on a claim's own session returns the claim (observer §3.8, 0.40), on another "
                          "session nothing: why A reads on its reader",
                     own.kind == live.COMPLETE and len(own.instances) == 1 and far.kind == live.COMPLETE
                     and not far.instances, f"own {len(own.instances)}, other {len(far.instances)}")
            tok.undeclare()
            for x in (holder, other):
                live.close(x)
            a3.close()
        finally:
            r3.close()
        bus = Bus()
        t = time.monotonic()
        ctl = bus.inst(IMPL["a"], "A", A_ID)
        rp.check(sec, "3. the control, the same instance as a client of a router with no access control, serves "
                      "within 3 s", _wait(ctl.serving, 3.0 + 1.0, 0.05), f"{time.monotonic() - t:.2f} s")
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

    sec = f"§6 a withdrawal that is no vacancy [{pairing()}]"
    bus = Bus(cut=True)
    link = None
    try:
        a = bus.inst(IMPL["a"], "A", A_ID, on=1)
        _wait(a.serving, 5.0)
        link = Link(bus.ep2)
        b = bus.inst(IMPL["b"], "B", B_ID, endpoint=link.endpoint)
        _wait(b.stands, 10.0)
        r0 = b.read_counts()
        link.cut()
        time.sleep(1.0)
        link.heal()
        time.sleep(6.0)
        r1 = b.read_counts()
        rp.check(sec, "1. B makes no read while its session reaches no node: its reads in the cut not made",
                 (r1.get("not_made") or 0) > (r0.get("not_made") or 0), f"{r0} -> {r1}")
        rp.check(sec, "1. once restored, its read shows A's interface token: B stands by on A",
                 (r1.get("complete") or 0) > (r0.get("complete") or 0) and b.why() == ("incumbent", A_ID),
                 f"B {b.standing()}")
        rp.check(sec, "1. B never takes over: no interface token of B's at R1 (S) at any time",
                 not bus.s.token_times("put", _alive_of(B_ID), 0.0))
    finally:
        if link is not None:
            link.close()
        bus.close()


# -- §7 and §9: the split -----------------------------------------------------------------

def _split(bus: Bus, b_flags: list[str] | None = None) -> tuple[Inst, Inst, bool]:
    a = bus.inst(IMPL["a"], "A", A_ID, on=1)
    _wait(a.serving, 5.0)
    b = bus.inst(IMPL["b"], "B", B_ID, on=2, flags=b_flags)
    _wait(b.stands, 10.0)
    bus.link.cut()
    took = _wait(lambda: bus.s2.first_token("put", _alive_of(B_ID)) is not None, 25.0, 0.05)
    return a, b, took


def section7(rp: Report, runs: int) -> None:
    from . import live

    sec = f"§7 the heal of a split [{pairing()}]"
    bus = Bus(cut=True)
    try:
        t0 = time.monotonic()
        a, b, took = _split(bus)
        rp.check(sec, "1. through S2: A's tokens withdrawn, and B takes over; through S: A serves, and sees nothing "
                      "of B's interface token",
                 took and bus.s2.first_token("delete", _alive_of(A_ID), t0) is not None and a.serving()
                 and not bus.s.token_times("put", _alive_of(B_ID), 0.0))
        rp.check(sec, "2. two incumbents, one on each side", a.serving() and b.serving())
        za, zb = zid_value(a.zid()), zid_value(b.zid())
        tp = time.monotonic()
        pa, pb = a.put(), b.put()
        time.sleep(0.5)
        s_ids = {stamp_zid(w[4]) for w in bus.s.writes(BALANCE, tp)}
        s2_ids = {stamp_zid(w[4]) for w in bus.s2.writes(BALANCE, tp)}
        rp.check(sec, "3. each side's subscribers receive their own incumbent's put (by its stamp's zid)",
                 s_ids == {za} and s2_ids == {zb}, f"{pa!r} {pb!r}; S {s_ids}, S2 {s2_ids}")
        h = time.monotonic()
        bus.link.heal()
        both = _wait(lambda: bus.s.first_token("put", _alive_of(B_ID), h) is not None
                     and bus.s2.first_token("put", _alive_of(A_ID), h) is not None, 20.0, 0.05)
        down = _wait(lambda: bus.s.first_token("delete", _alive_of(B_ID), h) is not None, 2 * 2.0 + 2.0 + 8.0, 0.05)
        w_b = bus.s.first_token("delete", _alive_of(B_ID), h)
        # B's descriptor put marked is read since the heal, never against the
        # token's withdrawal: "S's arrival order of a put and a token is noted,
        # never judged" (the conventions; round 88, a Rust B's marked put once
        # reached S before its token's withdrawal did).
        _wait(lambda: any(m for t, m in bus.s.descriptors(LEDGER, B_ID, h) if m), 3.0, 0.02)
        rp.check(sec, "4. after H, S and S2 each see both interface tokens", both)
        rp.check(sec, "4. B, the higher, steps down within two graces plus 2 s of the heal's arrival",
                 down and w_b is not None, f"{None if w_b is None else round(w_b - h, 2)} s after H")
        late = [w for w in bus.s.writes("zk2/lab/ledger/ledger.v1/", h) + bus.s2.writes("zk2/lab/ledger/ledger.v1/", h)
                if stamp_zid(w[4]) == zb and w_b is not None and w[0] > w_b]
        st = live.get_state(bus.g, "zk2/lab/ledger/ledger.v1/**")
        ids = {stamp_zid(r.stamp) for r in st.replies}
        marked = [m for t, m in bus.s.descriptors(LEDGER, B_ID, h)]
        if w_b is not None:
            before = [t for t, m in bus.s.descriptors(LEDGER, B_ID, h) if m and t < w_b]
            if before:
                rp.note(sec, f"B's marked descriptor reached S {round((w_b - before[0]) * 1000, 1)} ms before its "
                             "token's withdrawal: noted, never judged (the conventions)")
        refused = b.put()
        rp.check(sec, "4. in §2.9's order: every write stamped with B's zid dated before its token's withdrawal; "
                      "then an All GET answered by A alone, B's descriptor marked, and a write by B refused (§2.4)",
                 not late and ids == {za} and bool(marked) and marked[-1] is True
                 and refused is not None and refused.startswith("refused"),
                 f"late {len(late)}, GET by {ids}, marks {marked}, B's write {refused!r}")
        time.sleep(3.0)
        rp.check(sec, "4. A never steps down, and stands alone as the incumbent",
                 a.serving() and not bus.s.token_times("delete", _alive_of(A_ID), h) and bus.holders(1)[1] == {A_ID},
                 f"holders {bus.holders(1)[1]}")
    finally:
        bus.close()


def section9(rp: Report, runs: int) -> None:
    import zenoh

    from . import live
    from .redundancy_live import Follower

    sec = f"§9 a consumer following the incumbent during a split [{pairing()}]"
    bus = Bus(cut=True)
    k = None
    try:
        a, b, took = _split(bus, b_flags=["--hold-heal"])
        za, zb = zid_value(a.zid()), zid_value(b.zid())
        seen: list[int | None] = []
        last: dict[str, Any] = {}

        def p_sample(smp: Any) -> None:
            ts = smp.timestamp
            if ts is None:
                return
            sid = stamp_zid(str(ts))
            if last.get("id") != sid or ts.get_time_as_ntp64().as_nanos() > last.get("t", 0):
                last.update(id=sid, t=ts.get_time_as_ntp64().as_nanos())
                seen.append(sid)

        ps = live.open_client(bus.ep1)
        bus.extra.append(ps)
        bus.extra.append(ps.declare_subscriber(BALANCE, zenoh.handlers.Callback(p_sample)))
        h = time.monotonic()
        bus.link.heal()
        _wait(lambda: bus.s.first_token("put", _alive_of(B_ID), h) is not None, 20.0, 0.05)
        time.sleep(0.5)
        seeded = None
        for n in range(1, 6):
            a.put()
            time.sleep(0.1)
            b.put()
            time.sleep(0.1)
            if n == 3:
                k = Follower(bus.ep1, LEDGER, "ledger.v1", "balance")
                seeded = k.seed()
        time.sleep(0.5)
        flips = sum(1 for x, y in zip(seen, seen[1:]) if x != y)
        rp.check(sec, "1. P, by core S7 alone, moves at each put between A's and B's", flips >= 6 and {za, zb} <= set(seen),
                 f"{len(seen)} values, {flips} flips")
        acc = [zid_value(z) for _, _, z in k.accepted] if k else []
        disc = [zid_value(z) for _, _, z in k.discarded] if k else []
        rp.check(sec, "1. K's value moves to A's only, and K discards every sample stamped with B's meta.zid",
                 k is not None and acc and set(acc) == {za} and disc and set(disc) == {zb},
                 f"accepted {len(acc)} ({set(acc) == {za}}), discarded {len(disc)}")
        rp.check(sec, "2. K's seed holds A's reply, or, where Latest kept B's, nothing: never B's",
                 k is not None and seeded in ("accepted", "discarded"), f"seed {seeded}")
        b.release_heal()
        # the owner example names no why after a step-down; the role is enough
        down = _wait(lambda: b.standing().get("role") == "standby", 12.0, 0.05)
        time.sleep(1.0)
        a.put()
        time.sleep(0.5)
        last_acc = zid_value(k.accepted[-1][2]) if k and k.accepted else None
        rp.check(sec, "3. B steps down; K's value stays A's", down and last_acc == za, f"B {b.standing()}")
    finally:
        if k is not None:
            k.close()
        bus.close()


# -- §8 ------------------------------------------------------------------------------

def section8(rp: Report, runs: int) -> None:
    from . import live

    sec = f"§8 a call finds no standby [{pairing()}]"
    bus = Bus()
    try:
        a = bus.inst(IMPL["a"], "A", A_ID)
        _wait(a.serving, 5.0)
        b = bus.inst(IMPL["b"], "B", B_ID)
        _wait(b.stands, 8.0)
        got = [live.call(bus.g, APPEND, b"x") for _ in range(20)]
        rp.check(sec, "1. 20 calls, BestMatching and None: 20 executions on A, none on B",
                 all([r.payload for r in c.replies] == [b"A"] for c in got) and a.executions() == 20
                 and b.executions() == 0, f"A {a.executions()}, B {b.executions()}")
        fan = live.call(bus.g, APPEND, b"x", fanout=True)
        rp.check(sec, "2. an All GET with None on the concrete key: one reply, A's",
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
    from . import redundancy as rd
    from .redundancy_live import tool_reading

    sec = f"§10 external [V1 {who(IMPL['a'])}, V2 {who(IMPL['b'])}]"
    va, vb, v3 = "0000000000000f20", "0000000000000f10", "0000000000000f30"
    alive = "zk2/lab/valve/@zk/alive/valve.v1/"
    tmp = tempfile.mkdtemp(prefix="zk2py-valve-")
    for silent_run in (True, False):
        lock = os.path.join(tmp, f"valve-{silent_run}.lock")
        bus = Bus()
        silent = SilentLink(bus.ep1) if silent_run else None
        how = "V1's links silent, a short lease" if silent_run else "V1 killed as it runs"
        stop = threading.Event()
        both: list[float] = []

        def poll() -> None:
            while not stop.wait(1.0):
                if {va, vb} <= bus.holders(1, VALVE, "valve.v1")[1]:
                    both.append(time.monotonic())

        try:
            t = time.monotonic()
            v1 = bus.inst(IMPL["a"], "V1", va, address=VALVE, contract="valve.v1",
                          endpoint=silent.endpoint if silent else None,
                          flags=["--lock-file", lock] + (["--lease-ms", str(SHORT_LEASE_MS)] if silent_run else []))
            up = _wait(v1.serving, 6.0)
            ok, why = _ordered(bus, v1, t, serving=True)
            rp.check(sec, f"1. ({how}) V1 takes the lock and serves, in §2.6's order", up and ok, why)
            threading.Thread(target=poll, daemon=True).start()
            v2 = bus.inst(IMPL["b"], "V2", vb, address=VALVE, contract="valve.v1", flags=["--lock-file", lock])
            time.sleep(6.0 if not silent_run else 10.0)
            marks = [m for _, m in bus.s.descriptors(VALVE, vb)]
            rp.check(sec, f"2. ({how}) V2 stands by, marked, no_lock: no election by ids moves it, though lower",
                     v2.why()[0] == "no_lock" and marks and all(marks), f"V2 {v2.standing()}, marks {marks}")
            if silent is not None:
                silent.freeze()
            k = v1.kill()
            w = _wait(lambda: bus.s.first_token("delete", alive + va, k) is not None, 25.0, 0.05)
            t_w = bus.s.first_token("delete", alive + va, k)
            took = _wait(lambda: bus.s.first_token("put", alive + vb, k) is not None, 5.0, 0.02)
            t_b = bus.s.first_token("put", alive + vb, k)
            lease = (SHORT_LEASE_MS / 1000 * 0.5, SHORT_LEASE_MS / 1000 + 2.0) if silent_run else (0.0, 1.0)
            rp.check(sec, f"3. ({how}) V2 takes the lock, waits for V1's interface token to go ("
                          + ("V1's lease" if silent_run else "at once") + "), and serves within 3 s of it",
                     w and took and t_w is not None and t_b is not None and lease[0] <= t_w - k <= lease[1]
                     and 0 <= t_b - t_w <= 3.0, f"V1 gone +{None if t_w is None else round(t_w - k, 2)} s, V2 "
                                               f"serving +{None if t_b is None else round(t_b - k, 2)} s")
            rp.check(sec, f"3. ({how}) at no time do two reads show V1 and V2 both holding", not both,
                     f"{len(both)} reads showed both")
        finally:
            stop.set()
            if silent is not None:
                silent.close()
            bus.close()
    # 4. two lock files, V2 first
    bus = Bus()
    try:
        v2 = bus.inst(IMPL["b"], "V2", vb, address=VALVE, contract="valve.v1",
                      flags=["--lock-file", os.path.join(tmp, "v2.lock")])
        _wait(v2.serving, 6.0)
        v1 = bus.inst(IMPL["a"], "V1", va, address=VALVE, contract="valve.v1",
                      flags=["--lock-file", os.path.join(tmp, "v1.lock")])
        time.sleep(3.0)
        rp.check(sec, "4. two lock files, V2 first: V2 serves; V1, holding its own lock, stands by on V2's token",
                 v2.serving() and v1.why() == ("incumbent", vb), f"V2 {v2.standing()}, V1 {v1.standing()}")
    finally:
        bus.close()
    # 5. two lock files behind a cut link, then the link restored (0.2, F-200)
    bus = Bus(cut=True)
    try:
        bus.link.cut()
        v1 = bus.inst(IMPL["a"], "V1", va, on=1, address=VALVE, contract="valve.v1",
                      flags=["--lock-file", os.path.join(tmp, "c1.lock")])
        v2 = bus.inst(IMPL["b"], "V2", vb, on=2, address=VALVE, contract="valve.v1",
                      flags=["--lock-file", os.path.join(tmp, "c2.lock")])
        up = _wait(lambda: v1.serving() and v2.serving(), 8.0, 0.1)
        h = time.monotonic()
        bus.link.heal()
        _wait(lambda: {va, vb} <= bus.holders(1, VALVE, "valve.v1")[1], 20.0, 0.1)
        time.sleep(10.0)
        down = bus.s.token_times("delete", alive + va, h) + bus.s.token_times("delete", alive + vb, h)
        reading, _ = tool_reading(bus.g, VALVE, "valve.v1", grace_s=2.0)
        one = rd.one_incumbent(reading)
        rp.check(sec, "5. two lock files behind a cut link: both serve; restored, neither steps down within 10 s, "
                      "and G's two reads a grace apart read a split", up and not down and v1.serving() and v2.serving()
                 and (one.answer, one.reason) == ("no", "split"), f"{one}, withdrawals {len(down)}")
    finally:
        bus.close()
    # 6. V3 refuses while V1 holds the lock
    bus = Bus()
    try:
        lock = os.path.join(tmp, "valve6.lock")
        v1 = bus.inst(IMPL["a"], "V1", va, address=VALVE, contract="valve.v1", flags=["--lock-file", lock])
        _wait(v1.serving, 6.0)
        t = time.monotonic()
        v3i = bus.inst(IMPL["a"], "V3", v3, address=VALVE, contract="valve.v1",
                       flags=["--lock-file", lock, "--refuse-without-lock"])
        ctl = bus.inst(IMPL["a"], "C", "0000000000000f40", address=VALVE, contract="valve.v1",
                       flags=["--lock-file", os.path.join(tmp, "ctl.lock")])
        time.sleep(5.0)
        rp.check(sec, "6. V3, configured to refuse, refuses while V1 holds the lock: no instance token of V3's within "
                      "5 s, against a control whose token appears",
                 v3i.refused is not None and v3i.exit_code() not in (None, 0)
                 and not bus.s.token_times("put", f"zk2/lab/valve/@zk/instance/{v3}", t)
                 and bool(bus.s.token_times("put", "zk2/lab/valve/@zk/instance/0000000000000f40", t)),
                 f"V3 {v3i.refused!r}, exit {v3i.exit_code()}")
    finally:
        bus.close()


# -- §11 -----------------------------------------------------------------------------

def section11(rp: Report, runs: int) -> None:
    sec = f"§11 the tokenless set, and an invalid declaration [A {who(IMPL['a'])}]"
    bus = Bus()
    try:
        t = time.monotonic()
        ctl = bus.inst(IMPL["a"], "A", A_ID)
        rp.check(sec, "control: A configured plainly serves within 3 s", _wait(ctl.serving, 4.0),
                 f"{time.monotonic() - t:.2f} s")
        ctl.close()
        time.sleep(0.5)
        for step, kw, why in (("1", {"flags": ["--tokenless", "ledger.v1"]}, "tokenless"),
                              ("2", {"contract": "ledger-vote.v1"}, "value")):
            t = time.monotonic()
            x = bus.inst(IMPL["a"], "A", "00000000000000a9", **kw)
            time.sleep(5.0)
            toks = bus.s.token_times("put", "zk2/lab/ledger/@zk/instance/00000000000000a9", t)
            writes = bus.s.writes("zk2/lab/ledger/", t)
            rp.check(sec, f"{step}. A refuses to start: no instance token within 5 s, nothing of lab/ledger declared "
                          f"or put",
                     x.refused is not None and x.exit_code() not in (None, 0) and not toks and not writes,
                     f"{x.refused!r}; tokens {len(toks)}, puts {len(writes)}")
            said = x.refused is not None and why in x.refused
            rp.check(sec, f"{step}. … and says why: {why} (§2.2)", said, f"{x.refused!r}")
            if step == "2":
                # 0.3 (F-214): "the owner example says not started: … invalid:
                # value, never a standby's refused write"
                text = x.refused or ""
                rp.check(sec, "2. the declaration checked before anything is written: not started: … invalid: "
                              "value, never a standby's refused write (0.3, F-214)",
                         text.startswith("not started:") and "invalid" in text and "value" in text
                         and "stands by" not in text, f"{text!r}")
    finally:
        bus.close()


#: check conform's cases other than this profile's, skipped in §12 step 8
CONFORM_OTHERS = ("contract-served", "resource-served", "payload-type", "qos", "operation", "fanout-refused",
                  "state-stamp", "state-get", "freshness", "budget", "health", "health-aggregation", "rate",
                  "budget-window", "config-projection", "config-fresh", "config-view", "config-refusal",
                  "config-dry-run", "alarm-key", "alarm-document", "telemetry-declaration", "telemetry-sample",
                  "telemetry-continuity", "desired-report", "desired-binding", "desired-author")


def zenctl(args: list[str], timeout: float = 120.0) -> tuple[int | None, Any, str]:
    """§12's Rust tool as a black box: (exit, its JSON, stderr)."""
    if ZENCTL is None:
        return None, None, "no Rust tool given"
    with tempfile.TemporaryDirectory(prefix="zk2py-zenctl-") as home:
        env = dict(os.environ, ZENKEY_EXPLORER_CONFIG_DIR=home)
        try:
            p = subprocess.run([str(ZENCTL), *args, "--format", "json"], capture_output=True, text=True, env=env,
                               timeout=timeout)
        except subprocess.TimeoutExpired:
            return None, None, "timed out"
    try:
        doc = json.loads(p.stdout)
    except ValueError:
        doc = None
    return p.returncode, doc, p.stderr


def _flat_of_zenctl(doc: Any) -> dict[str, Any] | None:
    """zenctl redundancy's interface row, as zk2py's answers are kept: each
    question's (answer, reason), the standbys' count and exactness, and each
    instance's twin answer."""
    rows = [r for r in (doc or {}).get("rows") or [] if r.get("row") == "interface"]
    if len(rows) != 1:
        return None
    r = rows[0]

    def ar(x: Any) -> Any:
        return (x.get("answer"), x.get("reason")) if isinstance(x, dict) else None

    sb = r.get("standbys") or {}
    out: dict[str, Any] = {"governance": r.get("governance"), "one": ar(r.get("one_incumbent")),
                           "vacancy": ar(r.get("vacancy")),
                           "standbys": (sb.get("answer"), sb.get("reason"), sb.get("count"), sb.get("exact"))}
    for i in r.get("instances") or []:
        out[str(i.get("instance"))] = ar(i.get("twin"))
    return out


def _flat_of_mine(reading: Any) -> dict[str, Any]:
    from . import redundancy as rd

    c = rd.conclude(reading)
    sb = c["standbys"].as_json()
    gov = {"disagree": "disagree"}.get(reading.governance, reading.governance)
    out: dict[str, Any] = {"governance": gov, "one": (c["one_incumbent"].answer, c["one_incumbent"].reason),
                           "vacancy": (c["vacancy"].answer, c["vacancy"].reason),
                           "standbys": (sb["answer"], sb["reason"], sb.get("count"), sb.get("exact"))}
    for k, v in c["twins"].items():
        out[k] = (v.answer, v.reason)
    return out


def _doctor(checks: list[str], ep: str) -> tuple[int | None, dict[str, Any], str]:
    """zenctl doctor, asked these checks: by check, its verdict."""
    args = ["doctor", "-c", ep, "--timeout", "1"]
    for c in checks:
        args += ["--check", c]
    rc, doc, err = zenctl(args)
    # each asked check's verdict, with the row's other members (its findings)
    return rc, {r.get("check"): {**(r.get("verdict") or {}), "row": r} for r in (doc or {}).get("rows") or []
                if r.get("row") == "check" and r.get("check") in checks}, err


def section12(rp: Report, runs: int) -> None:
    """§12 (0.3): T is zk2py's tool and, given (``--zenctl`` or
    ``ZK2PY_ZENCTL``), zenctl's ``redundancy``, ``doctor`` and ``check
    conform``, each holding ``ledger.v1`` offline, both asked of every
    shape, their answers compared."""
    from . import hostid_scenarios as hs, live, redundancy as rd
    from .redundancy_live import contract_at, read_tool, split_findings

    sec = f"§12 a tool's questions [{pairing()}]"
    ledger = contract_at("ledger.v1")  # T holds ledger.v1 offline (0.3)
    fp = ledger.fingerprint
    lpath = str(CONTRACT_DIR / "ledger.v1.toml")
    use_z = ZENCTL is not None
    if not use_z:
        rp.note(sec, "zk2py's tool alone: no Rust tool given (--zenctl or ZK2PY_ZENCTL)")
    elif not ZENCTL.is_file():
        rp.check(sec, f"the Rust tool is at {ZENCTL}", False, "not a file")
        return
    differ: list[str] = []

    def ask(bus: Bus, step: str, want: dict[str, Any], z_exit: int | None, address: str = LEDGER,
            probe: bool = False) -> tuple[dict[str, Any], Any]:
        t = read_tool(bus.g, address, "ledger.v1", grace_s=2.0, contracts=[ledger], with_probe=probe)
        mine = _flat_of_mine(t.reading)
        ok = all(mine.get(k) == v for k, v in want.items())
        rp.check(sec, f"{step} zk2py's tool: " + ", ".join(f"{k} {v}" for k, v in want.items()), ok, f"{mine}")
        if use_z:
            rc, doc, err = zenctl(["redundancy", address, "--iface", "ledger.v1", "--contracts", lpath,
                                   "-c", bus.ep1, "--timeout", "1"])
            z = _flat_of_zenctl(doc)
            zok = z is not None and all(z.get(k) == v for k, v in want.items())
            rp.check(sec, f"{step} zenctl redundancy: the same, exit {z_exit}", zok and rc == z_exit,
                     f"{z}, exit {rc}{(' ' + err[-300:]) if z is None else ''}")
            if z is not None:
                keys = set(mine) | set(z)
                d = {k: (mine.get(k), z.get(k)) for k in sorted(keys) if mine.get(k) != z.get(k)}
                if d:
                    differ.append(f"{step} {d}")
        return mine, t

    bus = Bus()
    try:
        a = bus.inst(IMPL["a"], "A", A_ID)
        _wait(a.serving, 5.0)
        b = bus.inst(IMPL["b"], "B", B_ID)
        _wait(b.stands, 8.0)
        ask(bus, "1.", {"governance": "claim", "one": ("yes", "one"), "vacancy": ("no", "held"),
                        "standbys": ("yes", "standing_by", 1, True), A_ID: ("no", "incumbent"),
                        B_ID: ("no", "standby")}, 0)
        # 7. the doctor on step 1's shape
        splits = split_findings(bus.g, LEDGER, ["ledger.v1"], contracts=[ledger])
        faults, _ = live.presence_faults(bus.g, "zk2/lab/*/@zk/**")
        rp.check(sec, "7. step 1's shape, zk2py's tool: no split, no exposed interface without a token for B",
                 not splits and not any(f[1] == "no token" for f in faults), f"splits {splits}; faults {faults}")
        if use_z:
            rc, v, err = _doctor(["split-brain", "token-missing", "redundancy-vacancy"], bus.ep1)
            rp.check(sec, "7. step 1's shape, zenctl doctor: no split-brain, no token-missing and no "
                          "redundancy-vacancy finding, exit 0",
                     rc == 0 and all((v.get(c) or {}).get("answer") == "not_established"
                                     for c in ("split-brain", "token-missing", "redundancy-vacancy")),
                     f"exit {rc}; {v}")
        # 8. the probe over append's wildcard
        _, t = ask(bus, "8.", {"one": ("yes", "one")}, 0, probe=True)
        a_zid = live._zid_value(((t.docs.get(A_ID) or {}).get("meta") or {}).get("zid"))
        repliers = {live._zid_value(z) for zs in (t.probed or {}).values() for z in zs}
        rp.check(sec, "8. zk2py's probe over append's wildcard: answered by A's session alone, by its zid; B "
                      "exposes nothing", repliers == {a_zid} and not t.findings,
                 f"probed {t.probed}; A's zid {a_zid}; findings {t.findings}")
        if use_z:
            skips = [x for c in CONFORM_OTHERS for x in ("--skip", c)]
            rc, doc, err = zenctl(["check", "conform", LEDGER, "ledger.v1", "--contracts", lpath, "-c", bus.ep1,
                                   "--timeout", "1", "--for", "1", *skips])
            rows = {r.get("case"): r for r in (doc or {}).get("rows") or [] if isinstance(r, dict)}
            st = rows.get("redundancy-standby") or {}
            rp.check(sec, "8. zenctl check conform: redundancy-standby passes, an operation probed",
                     rc == 0 and (st.get("verdict") or {}).get("answer") in ("no", "not_established", "pass"),
                     f"exit {rc}; {st or (doc if doc else err[-300:])}")
        x = X(bus.ep1, C_ID, standby=True, alive=False, fingerprint=fp, queryable=True)
        time.sleep(0.5)
        _, t = ask(bus, "8. (X)", {"one": ("yes", "one"), C_ID: ("no", "standby")}, 0, probe=True)
        x_zid = live._zid_value(str(x.session.zid()))
        repliers = {live._zid_value(z) for zs in (t.probed or {}).values() for z in zs}
        rp.check(sec, "8. then X's session answers the probe too: X, marked standby with no token in both reads, "
                      "holds a queryable of the interface: zk2py's finding, beside §5's answers",
                 x_zid in repliers and [f[0] for f in t.findings] == [C_ID], f"probed {t.probed}; findings {t.findings}")
        if use_z:
            rc, doc, err = zenctl(["check", "conform", LEDGER, "ledger.v1", "--contracts", lpath, "-c", bus.ep1,
                                   "--timeout", "1", "--for", "1", *skips])
            rows = {r.get("case"): r for r in (doc or {}).get("rows") or [] if isinstance(r, dict)}
            st = rows.get("redundancy-standby") or {}
            rp.check(sec, "8. zenctl check conform: redundancy-standby a violation naming X, exit 1",
                     rc == 1 and C_ID in json.dumps(st), f"exit {rc}; {st or (doc if doc else err[-300:])}")
        x.close()
        b.close()
        a.close()
        time.sleep(0.5)
        # 3. B stands by and no instance serves: X plays it, marked
        x = X(bus.ep1, B_ID, standby=True, alive=False, fingerprint=fp)
        time.sleep(0.5)
        ask(bus, "3.", {"one": ("no", "vacant"), "vacancy": ("yes", "lasting")}, 1)
        if use_z:
            # beyond §12 (whose step 7 asks the doctor of steps 1 and 2): the
            # doctor holds no contract offline, and X serves no bundle
            rc, v, err = _doctor(["redundancy-vacancy"], bus.ep1)
            rv = dict(v.get("redundancy-vacancy") or {})
            rv.pop("row", None)
            rp.note(sec, f"3. zenctl doctor's redundancy-vacancy, which takes no --contracts, over X's lasting "
                         f"vacancy (no bundle served): exit {rc}, {rv}")
        x.close()
        # 4. A serves; X lists ledger.v1 unmarked with no interface token
        a = bus.inst(IMPL["a"], "A", A_ID)
        _wait(a.serving, 5.0)
        x = X(bus.ep1, B_ID, standby=False, alive=False, fingerprint=fp)
        time.sleep(0.5)
        ask(bus, "4.", {B_ID: ("yes", "unmarked")}, 1)
        x.close()
        a.close()
        time.sleep(0.5)
        # 5. no instance; X holds an interface token, its descriptor marked
        x = X(bus.ep1, C_ID, standby=True, alive=True, fingerprint=fp)
        time.sleep(0.5)
        ask(bus, "5.", {C_ID: ("yes", "marked_holding"), "one": ("yes", "one")}, 1)
        x.close()
        # beyond §12 (§2.15 premise 3, 0.3): an instance running under an
        # invalid declaration, its contract held offline
        vote = contract_at("ledger-vote.v1")
        x = X(bus.ep1, C_ID, standby=True, alive=False, fingerprint=vote.fingerprint)
        time.sleep(0.5)
        t = read_tool(bus.g, LEDGER, "ledger.v1", grace_s=2.0, contracts=[ledger, vote])
        mine = _flat_of_mine(t.reading)
        rp.check(sec, "beyond: an invalid declaration on the bus, zk2py's tool: unobservable, no_contract, the "
                      "reasons beside (0.3)",
                 mine["one"] == ("unobservable", "no_contract") and t.reasons == ["value"],
                 f"{mine}; reasons {t.reasons}")
        if use_z:
            rc, doc, err = zenctl(["redundancy", LEDGER, "--iface", "ledger.v1", "--contracts", lpath,
                                   "--contracts", str(CONTRACT_DIR / "ledger-vote.v1.toml"), "-c", bus.ep1,
                                   "--timeout", "1"])
            z = _flat_of_zenctl(doc)
            says = next((r.get("governance_says") for r in (doc or {}).get("rows") or []), None)
            rp.check(sec, "beyond: zenctl redundancy: the same, the reasons said, exit 2",
                     z is not None and z["one"] == ("unobservable", "no_contract") and rc == 2
                     and "value" in str(says), f"{z}; says {says!r}; exit {rc} {err[-200:]}")
            if z is not None and z != mine:
                differ.append(f"beyond (invalid) {dict((k, (mine.get(k), z.get(k))) for k in mine if mine.get(k) != z.get(k))}")
        x.close()
    finally:
        bus.close()
    # 2. the split, T a client of R1 after the heal, B's step-down held
    bus = Bus(cut=True)
    try:
        a, b, took = _split(bus, b_flags=["--hold-heal"])
        h = time.monotonic()
        bus.link.heal()
        _wait(lambda: bus.s.first_token("put", ALIVE + B_ID, h) is not None, 20.0, 0.05)
        time.sleep(0.5)
        ask(bus, "2.", {"one": ("no", "split"), A_ID: ("yes", "split_lowest"), B_ID: ("yes", "split")}, 1)
        splits = split_findings(bus.g, LEDGER, ["ledger.v1"], contracts=[ledger])
        rp.check(sec, "7. step 2's shape, zk2py's tool: one split finding, this profile's, naming A and B",
                 splits == [("ledger.v1", [A_ID, B_ID], "redundancy.v1")], f"{splits}")
        if use_z:
            rc, v, err = _doctor(["split-brain"], bus.ep1)
            sb = v.get("split-brain") or {}
            text = json.dumps(sb)
            rp.check(sec, "7. step 2's shape, zenctl doctor: one split-brain finding, naming A and B, as this "
                          "profile's split, exit 1",
                     rc == 1 and sb.get("answer") == "established" and A_ID in text and B_ID in text,
                     f"exit {rc}; {sb}")
        b.release_heal()
    finally:
        bus.close()
    # 6. A serves and B stands by on a system minted by hostid.v1
    base_dir = tempfile.TemporaryDirectory(prefix="zk2py-s12-")
    root = hs.make_root(base_dir.name, {hs.ETC: hs.M1 + "\n"})
    minted = f"{hs.SYSTEM[hs.M1]}/ledger"
    bus = Bus()
    try:
        a = bus.inst(IMPL["a"], "A", A_ID, flags=["--hostid-root", root], address="@hostid.v1/ledger")
        _wait(a.serving, 5.0)
        b = bus.inst(IMPL["b"], "B", B_ID, flags=["--hostid-root", root], address="@hostid.v1/ledger")
        _wait(b.stands, 8.0)
        ask(bus, "6.", {"one": ("yes", "one"), B_ID: ("no", "standby")}, 0, address=minted)
        answer, why, _ = live.hostid_collision(bus.g, minted, grace_s=2.0)
        rp.check(sec, "6. zk2py's tool: hostid.v1 §2.12's second process is B, a recognised standby, not counted: "
                      "no finding, and B named beside the answer",
                 answer == "no" and B_ID in why and "recognised standby" in why, f"{answer}: {why}")
        if use_z:
            rc, v, err = _doctor(["hostid-duplicate"], bus.ep1)
            hd = v.get("hostid-duplicate") or {}
            rp.check(sec, "6. zenctl doctor: no hostid-duplicate finding, and B named a recognised standby",
                     rc == 0 and hd.get("answer") == "not_established" and B_ID in json.dumps(hd),
                     f"exit {rc}; {hd}")
    finally:
        bus.close()
        base_dir.cleanup()
    # 8, its ledger-appendall half (0.4, F-221): append's wildcard would
    # reach appendall, whose fan-out is allowed; neither tool asks either,
    # each says why, and appendall's handler never runs. Then F-222's shape:
    # check conform with every case skipped sends nothing
    variant = contract_at("ledger-appendall.v1")
    bus = Bus()
    try:
        a = bus.inst(IMPL["a"], "A", A_ID, contract="ledger-appendall.v1")
        _wait(a.serving, 5.0)
        b = bus.inst(IMPL["b"], "B", B_ID, contract="ledger-appendall.v1")
        _wait(b.stands, 8.0)
        before = (a.executions(), b.executions())
        t = read_tool(bus.g, LEDGER, "ledger.v1", grace_s=2.0, contracts=[variant], with_probe=True)
        time.sleep(0.5)  # a call the probe made would have run by now
        after = (a.executions(), b.executions())
        rp.check(sec, "8. on ledger-appendall, zk2py's tool asks neither append$*, which reaches appendall, nor "
                      "appendall, and says why; appendall's handler never runs (0.4, F-221)",
                 not t.probed and "@op/append" in t.not_probed and "appendall" in t.not_probed["@op/append"]
                 and "@op/appendall" in t.not_probed and after == before,
                 f"probed {t.probed}; not probed {t.not_probed}; executions A, B {before} -> {after}")
        if use_z:
            skips = [x for c in CONFORM_OTHERS for x in ("--skip", c)]
            before = (a.executions(), b.executions())
            rc, doc, err = zenctl(["check", "conform", LEDGER, "ledger.v1", "--contracts",
                                   str(CONTRACT_DIR / "ledger-appendall.v1.toml"), "-c", bus.ep1, "--timeout", "1",
                                   "--for", "1", *skips])
            time.sleep(0.5)  # a call the probe made would have run by now
            after = (a.executions(), b.executions())
            rows = {r.get("case"): r for r in (doc or {}).get("rows") or [] if isinstance(r, dict)}
            st = rows.get("redundancy-standby") or {}
            rp.check(sec, "8. on ledger-appendall, zenctl check conform's redundancy-standby: neither asked, the "
                          "same why, unobservable, exit 2; appendall's handler never runs (0.4, F-221)",
                     rc == 2 and (st.get("verdict") or {}).get("answer") == "unobservable"
                     and "appendall" in json.dumps(st) and after == before,
                     f"exit {rc}; executions A, B {before} -> {after}; {st}")
            every = [x for c in (*CONFORM_OTHERS, "redundancy-declaration", "redundancy-standby")
                     for x in ("--skip", c)]
            before = (a.executions(), b.executions())
            rc, doc, err = zenctl(["check", "conform", LEDGER, "ledger.v1", "--contracts",
                                   str(CONTRACT_DIR / "ledger-appendall.v1.toml"), "-c", bus.ep1, "--timeout", "1",
                                   "--for", "1", *every])
            time.sleep(0.5)  # a call it made would have run by now
            after = (a.executions(), b.executions())
            asked = [r.get("case") for r in (doc or {}).get("rows") or []
                     if isinstance(r, dict) and (r.get("verdict") or {}).get("answer") != "not_asked"]
            rp.check(sec, "beyond (F-222): zenctl check conform with every case skipped sends nothing: no case "
                          "asked, and no handler runs",
                     not asked and after == before, f"exit {rc}; asked {asked}; executions A, B {before} -> {after}")
    finally:
        bus.close()
    # 9 (0.5, F-226): A and B run ledger.v1, all the address's descriptors
    # name; T also holds ledger-appendall offline, a revision no instance
    # runs that adds appendall: no append$* probe, the reason naming
    # appendall and that revision, the standbys' exposure unobservable
    bus = Bus()
    try:
        a = bus.inst(IMPL["a"], "A", A_ID)
        _wait(a.serving, 5.0)
        b = bus.inst(IMPL["b"], "B", B_ID)
        _wait(b.stands, 8.0)
        t = read_tool(bus.g, LEDGER, "ledger.v1", grace_s=2.0, contracts=[ledger, variant], with_probe=True)
        why = t.not_probed.get("@op/append", "")
        rp.check(sec, "9. ledger.v1 running, ledger-appendall held offline: zk2py's tool asks no append$*, its "
                      "reason naming appendall and the revision held offline; the standbys' exposure unobservable "
                      "(0.5, F-226)",
                 not t.probed and "appendall" in why and variant.fingerprint in why
                 and ledger.fingerprint not in why and t.exposure == "unobservable",
                 f"probed {t.probed}; not probed {t.not_probed}; exposure {t.exposure}")
        if use_z:
            skips = [x for c in CONFORM_OTHERS for x in ("--skip", c)]
            rc, doc, err = zenctl(["check", "conform", LEDGER, "ledger.v1", "--contracts", lpath, "--contracts",
                                   str(CONTRACT_DIR / "ledger-appendall.v1.toml"), "-c", bus.ep1, "--timeout", "1",
                                   "--for", "1", *skips])
            rows = {r.get("case"): r for r in (doc or {}).get("rows") or [] if isinstance(r, dict)}
            st = rows.get("redundancy-standby") or {}
            text = json.dumps(st)
            rp.check(sec, "9. zenctl check conform: no append$* probe, the reason naming appendall and the revision "
                          "held offline, redundancy-standby unobservable, exit 2",
                     rc == 2 and (st.get("verdict") or {}).get("answer") == "unobservable" and "appendall" in text
                     and variant.fingerprint in text and ledger.fingerprint not in text, f"exit {rc}; {st}")
    finally:
        bus.close()
    if use_z:
        rp.note(sec, ("the two tools' answers differ: " + "; ".join(differ)) if differ
                else "the two tools answer every shape alike")


def section_remint(rp: Report, runs: int) -> None:
    """zk2py's own, beyond scenarios.md: §2.8's "A re-mint is no vacancy",
    from the incumbent's side too (§2.9, F-209): A re-mints make-before-break
    while B stands by (``redundancy new-epoch``, the election told first)."""
    sec = f"re-mint: no vacancy, and no heal of itself [{pairing()}]"
    bus = Bus()
    try:
        a = bus.inst(IMPL["a"], "A", A_ID)
        _wait(a.serving, 5.0)
        b = bus.inst(IMPL["b"], "B", B_ID)
        _wait(b.stands, 8.0)
        t = time.monotonic()
        new = a.remint()
        time.sleep(2 * 2.0 + 2.0)
        put_new = None if new is None else bus.s.first_token("put", _alive_of(new), t)
        gone_old = bus.s.first_token("delete", _alive_of(A_ID), t)
        rp.check(sec, "A's new interface token appears before its old one goes: an overlap, never a gap",
                 new is not None and put_new is not None and gone_old is not None and put_new <= gone_old,
                 f"new {new} +{None if put_new is None else round(put_new - t, 3)}, old gone +"
                 f"{None if gone_old is None else round(gone_old - t, 3)}")
        rp.check(sec, "B hears no vacancy and never takes over", b.stands()
                 and not bus.s.token_times("put", _alive_of(B_ID), t), f"B {b.standing()}")
        rp.check(sec, "A reads no heal of itself and never steps down, serving under its new id",
                 a.serving() and new is not None and bus.holders(1)[1] == {new}
                 and not bus.s.token_times("delete", _alive_of(new), t), f"holders {bus.holders(1)[1]}")
        st = a.steps()
        if st is not None:
            since = [x[2]["step"] for x in st if x[0] >= t]
            rp.check(sec, "(zk2py's log) no read, no grace, no step-down after the re-mint",
                     not ({"read", "grace", "step_down"} & set(since)), f"{since}")
    finally:
        bus.close()


def section_poll(rp: Report, runs: int) -> None:
    """zk2py's own, beyond scenarios.md: §2.7 (0.2)'s poll, where no watch
    session can be held, and what it misses: B polls every 500 ms."""
    from . import live

    sec = f"§2.7's poll [{pairing()}]"
    bus = Bus()
    try:
        a = bus.inst(IMPL["a"], "A", A_ID)
        _wait(a.serving, 5.0)
        b = bus.inst(IMPL["b"], "B", B_ID, flags=["--poll-ms", "500"])
        _wait(b.stands, 8.0)
        j = b.jcmd("redundancy reads") or {}
        rp.check(sec, "B polls (its reads count polls) and holds no watch session",
                 ((j.get("interfaces") or {}).get("ledger.v1") or {}).get("polls", 0) >= 1
                 and "watch" not in (j.get("sessions") or {}), f"{j}")
        q = a.close()
        took = _wait(lambda: bus.s.first_token("put", _alive_of(B_ID), q) is not None, 5.0, 0.02)
        t_b = bus.s.first_token("put", _alive_of(B_ID), q)
        rp.check(sec, "A's clean stop: B, polling, takes over within one period, a read and 1 s",
                 took and t_b is not None and t_b - q <= 0.5 + 1.0 + 1.0, f"+{None if t_b is None else round(t_b - q, 2)} s")
        # what comes and goes within one period: a second holder for 150 ms
        fp = (b.jcmd("redundancy role") or {}).get("instance")
        missed = 0
        tries = 6
        for n in range(tries):
            before = b.read_counts().get("made") or 0
            x = live.open_client(bus.ep1)
            tok = x.liveliness().declare_token(f"zk2/lab/ledger/@zk/alive/ledger.v1/{'d' * 15}{n}/{'0' * 16}")
            time.sleep(0.15)
            tok.undeclare()
            live.close(x)
            time.sleep(1.2)
            if (b.read_counts().get("made") or 0) == before:
                missed += 1
        rp.note(sec, f"a second holder for 150 ms, {tries} times: the polling incumbent never read for a heal in "
                     f"{missed} (§2.7: \"It misses what comes and goes within one period\"); the period 500 ms")
        rp.check(sec, "B serves throughout, its polls reading no split of its own", b.serving() and fp == B_ID)
    finally:
        bus.close()


def section_poll_miss(rp: Report, runs: int) -> None:
    """zk2py's own (§2.7, 0.2; F-215): a polling standby whose polls are not
    complete while its incumbent goes. B polls every 500 ms through a link
    paused for 6 s, longer than a read's timeout and shorter than either
    side's lease (B's 20 s, R1's 10 s), so that the pause closes nothing; A
    closes cleanly 0.3 s into the pause. Every poll across the pause runs
    to its timeout and shows nothing; the first after it is complete and
    shows no holder. Read against the last poll whatever it was, that is
    no withdrawal ("after one with some"), and B stands by on A for good;
    read against the last complete poll, it is, and B takes over."""
    sec = f"§2.7's poll across reads not complete [{pairing()}]"
    for n in range(3):
        bus = Bus()
        try:
            link = PauseLink(bus.ep1)
            bus.extra.append(link)
            a = bus.inst(IMPL["a"], "A", A_ID)
            _wait(a.serving, 5.0)
            b = bus.inst(IMPL["b"], "B", B_ID, flags=["--poll-ms", "500", "--lease-ms", "20000"],
                         endpoint=link.endpoint)
            stood = _wait(b.stands, 8.0)
            before = b.read_counts()
            link.pause()
            time.sleep(0.3)
            q = a.close()
            time.sleep(5.7)
            during = b.read_counts() if not b.rust else {}
            link.resume()
            r = time.monotonic()
            took = _wait(lambda: bus.s.first_token("put", _alive_of(B_ID), q) is not None, 6.0, 0.02)
            t_b = bus.s.first_token("put", _alive_of(B_ID), q)
            after = b.read_counts()
            polls = (after.get("polls") or 0) - (before.get("polls") or 0)
            rp.check(sec, f"run {n + 1}: B, polling across a pause in which A went, takes over once a complete poll "
                          "shows no holder (F-215: the last complete poll's holders kept)",
                     stood and took and t_b is not None and t_b >= r and b.serving()
                     # zk2py's own count: a poll across the pause was not complete
                     and (b.rust or (during.get("polls") or 0) - (during.get("polls_complete") or 0)
                          > (before.get("polls") or 0) - (before.get("polls_complete") or 0)),
                     f"stood {stood}; S heard B's interface token {None if t_b is None else round(t_b - r, 2)} s after "
                     "the resume; "
                     f"{polls} polls since the pause began (zk2py's, at the resume: {during}); reads {after}; "
                     f"standing {b.standing()}")
        finally:
            bus.close()


def section_heal_retry(rp: Report, runs: int) -> None:
    """zk2py's own (§2.9, 0.2, F-199): a heal whose first read is not made, its
    watch hearing the other holder before its reader reconnects, made again
    after the retry interval. zk2py's B only: the owner example's reader
    shares its endpoint."""
    from .budget_scenarios import Link

    sec = f"§2.9's heal retried [{pairing()}]"
    if IMPL["b"] != "zk2py":
        rp.note(sec, "B is the owner example, whose reader cannot be cut apart from its watch: not run")
        return
    bus = Bus(cut=True)
    rl = Link(bus.ep2)
    try:
        a = bus.inst(IMPL["a"], "A", A_ID, on=1)
        _wait(a.serving, 5.0)
        b = bus.inst("zk2py", "B", B_ID, on=2, flags=["--reader-connect", rl.endpoint])
        _wait(b.stands, 10.0)
        bus.link.cut()
        _wait(b.serving, 25.0, 0.05)
        rl.cut()  # B's reader cut off; its claim and its watch stay up
        time.sleep(0.5)
        h = time.monotonic()
        bus.link.heal()
        time.sleep(1.0)
        rl.heal()
        down = _wait(lambda: bus.s.first_token("delete", _alive_of(B_ID), h) is not None, 20.0, 0.05)
        steps = [x[2]["step"] for x in (b.steps() or []) if x[0] >= h]
        rp.check(sec, "B's watch hears A at the heal; its first heal read, its reader cut off, ends nothing: retry; "
                      "made again once the reader is back, B steps down to A",
                 down and "retry" in steps and "step_down" in steps and steps.index("retry") < steps.index("step_down")
                 and a.serving(), f"steps {steps}")
    finally:
        rl.close()
        bus.close()


def section_heal_retry_pause(rp: Report, runs: int) -> None:
    """zk2py's own (§2.9, 0.2, F-199), for either B: a heal read not
    complete, made again. B, holding its heal (``--hold-heal``), serves on
    R2 behind a link that pauses; at the split's heal it hears A. Its link
    paused, the heal is let go: its first heal read runs to its timeout and
    shows A not, so ends nothing. The link back 3 s later, its read made
    again finds both, and B, the higher, steps down. Under 0.1's machine
    that read went back to serving, and the split stayed."""
    sec = f"§2.9's heal retried, B's link paused [{pairing()}]"
    for n in range(3):
        bus = Bus(cut=True)
        try:
            pl = PauseLink(bus.ep2)
            bus.extra.append(pl)
            a = bus.inst(IMPL["a"], "A", A_ID, on=1)
            _wait(a.serving, 5.0)
            b = bus.inst(IMPL["b"], "B", B_ID, on=2, flags=["--hold-heal"], endpoint=pl.endpoint)
            _wait(b.stands, 10.0)
            bus.link.cut()
            took = _wait(lambda: bus.s2.first_token("put", _alive_of(B_ID)) is not None, 25.0, 0.05)
            _wait(b.serving, 5.0)
            bus.link.heal()
            healed = _wait(bus.linked, 15.0, 0.1)
            time.sleep(1.0)  # B's watch has heard A; its heal is held
            before = b.read_counts()
            pl.pause()
            h = time.monotonic()
            b.release_heal()
            time.sleep(3.0)
            during = b.read_counts()
            pl.resume()
            down = _wait(lambda: bus.s.first_token("delete", _alive_of(B_ID), h) is not None, 20.0, 0.05)
            t_d = bus.s.first_token("delete", _alive_of(B_ID), h)
            after = b.read_counts()

            def short(k: dict[str, Any]) -> int:
                return (k.get("made") or 0) - (k.get("complete") or 0) + (k.get("not_made") or 0)

            rp.check(sec, f"run {n + 1}: B's heal read across the pause is not complete and ends nothing; made again "
                          "once the link is back, B steps down to A, which serves",
                     took and healed and down and short(during) > short(before) and a.serving(),
                     f"split {took}, healed {healed}; reads not complete before {short(before)}, across the pause "
                     f"{short(during)}; B stepped down {None if t_d is None else round(t_d - h, 2)} s after the "
                     f"release; reads {before} -> {during} -> {after}; B {b.standing()}")
        finally:
            bus.close()


def section_remint_campaign(rp: Report, runs: int) -> None:
    """zk2py's own (§2.9, 0.2, F-209): B re-mints while it campaigns for A's
    seat, A killed as it runs."""
    sec = f"a re-mint during a campaign [{pairing()}]"
    fails = []
    for n in range(3):
        bus = Bus()
        try:
            a = bus.inst(IMPL["a"], "A", A_ID)
            _wait(a.serving, 5.0)
            b = bus.inst(IMPL["b"], "B", B_ID)
            _wait(b.stands, 8.0)
            a.kill()
            time.sleep(0.05 * n)
            new = b.remint()
            ok = _wait(lambda: b.serving() and bus.holders(1)[1] == {new}, 8.0, 0.1)
            time.sleep(2 * 2.0 + 1.0)
            holders = bus.holders(1)[1]
            if not (ok and holders == {new} and b.serving()):
                fails.append(f"run {n}: new {new}, holders {holders}, B {b.standing()}")
        finally:
            bus.close()
    rp.check(sec, "in 3 runs, B, re-minted while campaigning, serves alone under its new id: its former claim its "
                  "own, never a lower candidate nor a split", not fails, "; ".join(fails))


SECTIONS = {"1": section1, "2": section2, "3": section3, "4": section4, "5": section5, "6": section6,
            "7": section7, "8": section8, "9": section9, "10": section10, "11": section11, "12": section12,
            "remint": section_remint, "poll": section_poll, "heal-retry": section_heal_retry,
            "remint-campaign": section_remint_campaign, "poll-miss": section_poll_miss,
            "heal-retry-pause": section_heal_retry_pause}


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="python -m zk2py.redundancy_scenarios", description=__doc__.split("\n")[0])
    ap.add_argument("--a", default="zk2py", help="A's (and V1's) candidate: zk2py, or the owner example's path")
    ap.add_argument("--b", default="zk2py", help="B's (and V2's) candidate: zk2py, or the owner example's path")
    ap.add_argument("--owner", help="both candidates the owner example at this path")
    ap.add_argument("--only", action="append", choices=list(SECTIONS))
    ap.add_argument("--runs", type=int, default=20, help="§3's runs (20 by the text)")
    ap.add_argument("--zenctl", help="§12's Rust tool (else $ZK2PY_ZENCTL; without either, §12 runs zk2py's tool "
                                     "alone and says so)")
    a = ap.parse_args(argv)
    global ZENCTL
    if a.zenctl:
        ZENCTL = Path(a.zenctl)
    for k in ("a", "b"):
        v = a.owner or getattr(a, k)
        if v != "zk2py" and not Path(v).is_file():
            print(f"error: {v} not found", file=sys.stderr)
            return 2
        IMPL[k] = v if v == "zk2py" else Path(v)
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
    print(f"redundancy scenarios ({pairing()}): {passed} passed, {failed} failed")
    return 0 if failed == 0 else 1


if __name__ == "__main__":
    rc = main()
    sys.stdout.flush()
    os._exit(rc)

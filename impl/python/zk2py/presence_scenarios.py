"""core ``scenarios/presence.md`` §1 to §10 (core 0.41), run from Python on
zenoh-python 1.10.1 routers in process, with an owner and a reader under
test of either implementation::

    python -m zk2py.presence_scenarios [--only 1|2|3|4|5|6|7|8|9|10]
        [--owner zk2py|rust] [--reader zk2py|zenctl]
        [--owner-bin PATH] [--zenctl PATH] [--crowd N]

- ``--owner``: who holds the tokens. ``zk2py`` is zk2py's owner
  (:class:`zk2py.owner.Owner`), ``rust`` the reference's owner example as
  a black box (``--owner-bin``, a client of the section's router).
- ``--reader``: the reader under test. ``zk2py`` is zk2py's reads
  (:func:`zk2py.live.list_presence`) and follower
  (:class:`zk2py.presence.PresenceFollower`); ``zenctl`` the reference's
  tool as a black box (``--zenctl``): ``service list`` for a read,
  ``watchdog --rule 'instance-gone …'`` for a reader that follows presence.
  A section a tool's command line cannot run says so (NOT RUN), never a
  pass.

Each section observes zenoh's behaviour as the scenario states it, beside
the reader under test's verdict (0.38, F-188, F-189):
- §1: bring-up order and tokens; §2: the descriptor, and an owner that must
  not start, watched through R1 with its control; §3: a make-before-break
  re-mint, and no member token before a member; §4: the watch session's
  round trip beside 1,200 tokens; §5: a tokenless set; §6: a read refused
  by access control, and one stalled past its timeout;
- §7: a read through a session cut off, before it and while it runs;
  §8: a withdrawal heard live; §9: a read beside a liveliness subscriber;
- §10 (0.41): the round trip at a population, its GET matching no token,
  subscribers without history and a namespace's, a round trip lost with
  its transport, and a read that runs to the interests timeout.

Exit 0 when every check passes, 1 otherwise.
"""

from __future__ import annotations

import argparse
import json
import os
import queue
import socket
import subprocess
import sys
import tempfile
import threading
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Any

from .alarms_scenarios import Report, _wait

REPO = Path(__file__).resolve().parents[3]
INSTANCE = "zk2/lab/owner/@zk/instance/" + "a" * 16
SELECTOR = "zk2/*/*/@zk/**"
INSTANCES = "zk2/*/*/@zk/instance/*"
ECHO = REPO / "impl/python/interop/zk2py_echo.v1.toml"
BRINGUP = REPO / "impl/python/interop/zk2py_bringup.v1.toml"
EPOCH = REPO / "impl/python/interop/zk2py_epoch.v1.toml"
THRUSTER = REPO / "examples/zk2/walkthrough/thruster.v1.toml"
NAV = REPO / "examples/zk2/walkthrough/nav.v2.toml"
HEALTH = REPO / "spec/profiles/health/health.v1.toml"


@dataclass
class Pairing:
    """Who plays the owner and the reader under test (``--owner``,
    ``--reader``), and where the reference's binaries are."""

    owner: str = "zk2py"
    reader: str = "zk2py"
    owner_bin: Path = REPO / "target" / "debug" / "examples" / "owner"
    zenctl_bin: Path = REPO / "target" / "debug" / "zenctl"
    crowd: int = 2000
    #: §5's services
    tokenless: int = 100


P = Pairing()


class NotPairable(Exception):
    """A step the chosen implementation's interface cannot run."""


def _raw_read(session: Any, selector: str = SELECTOR, timeout: float = 1.0) -> tuple[bool, list[str], int, float]:
    """zenoh's own answer to a liveliness GET, bypassing zk2py's guards:
    (final reply came, token keys, error replies, seconds)."""
    import zenoh

    replies: list[Any] = []
    done = threading.Event()
    t0 = time.monotonic()
    session.liveliness().get(selector, zenoh.handlers.Callback(replies.append, done.set), timeout=timeout)
    finished = done.wait(timeout + 5.0)
    oks = [str(r.ok.key_expr) for r in replies if r.ok is not None]
    return finished, oks, sum(1 for r in replies if r.ok is None), time.monotonic() - t0


def _round_trip_raw(session: Any, key: str, timeout: float, on_end: Any = None,
                    direct: bool = False) -> tuple[bool, int, int, float]:
    """A round trip seen as zenoh answers it: (final reply came, tokens,
    error replies, seconds); ``on_end`` runs as the final reply arrives.
    ``direct``: its callbacks run on zenoh's own thread (``indirect=False``,
    unstable), not zenoh-python's default thread of their own."""
    import zenoh

    oks: list[Any] = []
    errs: list[Any] = []
    done = threading.Event()

    def drop() -> None:
        if on_end is not None:
            on_end()
        done.set()

    t0 = time.monotonic()
    session.liveliness().get(key, zenoh.handlers.Callback(lambda r: (errs if r.ok is None else oks).append(r), drop,
                                                          indirect=not direct), timeout=timeout)
    finished = done.wait(timeout + 5.0)
    return finished, len(oks), len(errs), time.monotonic() - t0


# -- the owner, of either implementation ---------------------------------------------------------

class Holder:
    """The section's owner: zk2py's (:class:`zk2py.owner.Owner`, a client of
    ``endpoint``) or the owner example (``owner-wb --connect``), serving
    ``contracts`` (:data:`ECHO` by default) at ``address``. ``key`` is its
    instance token. ``remint`` makes a make-before-break re-mint possible
    (the owner example's ``telemetry new-epoch``, so it implements meter.v1
    beside them)."""

    def __init__(self, endpoint: str, address: str = "lab/owner", *, contracts: list[Path] | None = None,
                 namespace: str | None = None, remint: bool = False, tokenless: set[str] | None = None,
                 flags: list[str] | None = None) -> None:
        from .contract import load_contract
        from .live_interop import Owner as RustOwner
        from .owner import Owner as PyOwner

        self.address = address
        self.system, self.service = address.split("/")
        paths = [ECHO] if contracts is None else contracts
        self.kind = P.owner
        self.proc: Any = None
        self.owner: Any = None
        if self.kind == "rust":
            if namespace is not None:
                raise NotPairable("the owner example takes no namespace option")
            extra = list(flags or [])
            if remint:
                extra.append("--telemetry")
            for iface in sorted(tokenless or ()):
                extra += ["--tokenless", iface]
            if tokenless and "health.v1" in tokenless:
                extra.append("--health")
                paths = [p for p in paths if p != HEALTH]
            self.proc = RustOwner(P.owner_bin, address, paths, connect=endpoint, flags=extra)
            ready = self.proc.wait_for("ready ", 60)
            if ready is None:
                err = " | ".join(self.proc.stderr[-2:])
                self.proc.close()
                raise RuntimeError(f"the owner example did not start: {err}")
            self.key = ready.strip()
        else:
            self.owner = PyOwner(self.system, self.service, [load_contract(p) for p in paths], connect=endpoint,
                                 namespace=namespace, tokenless=tokenless)
            self.owner.start()
            self.key = self.owner.instance_key

    @property
    def instance(self) -> str:
        return self.key.rsplit("/", 1)[1]

    def remint(self) -> str:
        """A make-before-break re-mint; the new instance token's key."""
        if self.kind == "rust":
            line = self.proc.command("telemetry new-epoch") or ""
            if not line.startswith("telemetry new-epoch "):
                raise RuntimeError(f"no re-mint: {line!r}")
            self.key = f"zk2/{self.address}/@zk/instance/{line.split()[-1]}"
        else:
            self.owner.new_epoch()
            self.key = self.owner.instance_key
        return self.key

    def withdraw(self) -> None:
        """A clean stop: every token undeclared."""
        if self.kind == "rust":
            if self.proc is not None:
                self.proc.close()
                self.proc = None
        elif self.owner is not None:
            self.owner.close()
            self.owner = None

    close = withdraw


# -- links and routers ---------------------------------------------------------------------------

class DelayLink:
    """A TCP relay that delivers every chunk, each way, ``delay_s`` after it
    arrived, in order (a long path the test can lengthen), and can be cut as
    :class:`zk2py.budget_scenarios.Link` is. For the reference's tool, a
    process of its own whose session the test cannot stall between its
    open and its first read."""

    def __init__(self, upstream: str, delay_s: float = 0.0) -> None:
        host, port = upstream.removeprefix("tcp/").rsplit(":", 1)
        self.upstream = (host, int(port))
        self.delay_s = delay_s
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
                q: queue.Queue = queue.Queue()
                threading.Thread(target=self._read, args=(a, q), daemon=True).start()
                threading.Thread(target=self._write, args=(q, b, a), daemon=True).start()

    def _read(self, src: socket.socket, q: queue.Queue) -> None:
        try:
            while True:
                data = src.recv(65536)
                if not data:
                    break
                q.put((time.monotonic() + self.delay_s, data))
        except OSError:
            pass
        q.put(None)

    @staticmethod
    def _write(q: queue.Queue, dst: socket.socket, src: socket.socket) -> None:
        while True:
            item = q.get()
            if item is None:
                break
            due, data = item
            time.sleep(max(0.0, due - time.monotonic()))
            try:
                dst.sendall(data)
            except OSError:
                break
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

    def heal(self) -> None:
        self.open = True

    def close(self) -> None:
        self.cut()
        try:
            self.sock.close()
        except OSError:
            pass


def _router(connect: str | None = None) -> tuple[Any, str]:
    """A zenoh-python router on a free loopback port, linked to ``connect``
    when given (presence.md §10's R2 → R1)."""
    import zenoh

    from .owner import free_loopback_port

    port = free_loopback_port()
    conf = zenoh.Config()
    conf.insert_json5("mode", json.dumps("router"))
    conf.insert_json5("listen/endpoints", json.dumps([f"tcp/127.0.0.1:{port}"]))
    if connect is not None:
        conf.insert_json5("connect/endpoints", json.dumps([connect]))
    conf.insert_json5("scouting/multicast/enabled", "false")
    conf.insert_json5("timestamping/enabled", "true")
    return zenoh.open(conf), f"tcp/127.0.0.1:{port}"


class Bus:
    """R1, an owner (a client of R1), and a tool that reaches R1 through a
    link the test can cut and heal; the tool's watch session goes the same
    way."""

    def __init__(self) -> None:
        from . import live
        from .budget_scenarios import Link
        from .live_interop import _r1

        self.router, self.endpoint, _ = _r1()
        self.link = Link(self.endpoint)
        self.holder: Holder | None = None
        self.tool = live.open_client(self.link.endpoint)
        self.extra: list[Any] = []

    @property
    def key(self) -> str:
        assert self.holder is not None
        return self.holder.key

    def start_owner(self, address: str = "lab/owner", **kw: Any) -> Holder:
        self.holder = Holder(self.endpoint, address, **kw)
        self.extra.append(self.holder)
        return self.holder

    def tool_reads(self, key: str | None = None) -> bool:
        from . import live

        key = key or self.key
        return any(f"zk2/{i['system']}/{i['service']}/@zk/instance/{i['instance']}" == key
                   for i in live.list_presence(self.tool).instances)

    def cut(self) -> bool:
        from . import live

        self.link.cut()
        return _wait(lambda: live.transports_held(self.tool) is False and live.links_held(self.tool) is False, 15.0)

    def heal(self) -> bool:
        from . import live

        self.link.heal()
        return _wait(lambda: live.links_held(self.tool) is True, 20.0, 0.1)

    def close(self) -> None:
        for x in reversed(self.extra):
            try:
                x.close() if hasattr(x, "close") else x.undeclare()
            except Exception:  # noqa: BLE001 - closing anyway
                pass
        self.link.close()
        try:
            self.router.close()
        except Exception:  # noqa: BLE001
            pass


# -- the reference's tool, as a black box ------------------------------------------------------

def _zenctl(args: list[str], timeout: float = 120.0) -> tuple[int, str, str]:
    """One zenctl command: (exit status, stdout, stderr)."""
    if not P.zenctl_bin.is_file():
        raise NotPairable(f"{P.zenctl_bin} not found")
    cp = subprocess.run([str(P.zenctl_bin), *args], capture_output=True, text=True, timeout=timeout)
    return cp.returncode, cp.stdout, cp.stderr


def _zenctl_config(interests_ms: int) -> str:
    """A zenoh JSON5 file setting ``routing/interests/timeout`` (ms), for
    ``--zenoh-config``."""
    fd, path = tempfile.mkstemp(suffix=".json5", prefix="zk2py-interests-")
    with os.fdopen(fd, "w") as f:
        f.write(json.dumps({"routing": {"interests": {"timeout": interests_ms}}}))
    return path


def _zenctl_list(endpoint: str, *, system: str | None = None, timeout_s: float = 1.0,
                 extra: list[str] | None = None, wait: float = 120.0) -> tuple[int, dict[str, Any] | None, str, float]:
    """``zenctl service list`` as JSON: (exit status, document or None,
    stderr's last lines, seconds)."""
    args = ["service", "list", "-c", endpoint, "--format", "json", "--timeout", f"{timeout_s:g}"]
    if system:
        args += ["--system", system]
    t0 = time.monotonic()
    rc, out, err = _zenctl(args + list(extra or []), wait)
    try:
        doc = json.loads(out) if out.strip() else None
    except ValueError:
        doc = None
    return rc, doc, " | ".join(err.strip().splitlines()[-2:]), time.monotonic() - t0


def _rows(doc: dict[str, Any] | None) -> dict[str, dict[str, Any]]:
    """``service list``'s instance rows by instance id."""
    return {r.get("instance"): r for r in (doc or {}).get("rows", []) if r.get("row") == "instance"}


def _brief(doc: dict[str, Any] | None) -> str:
    """What a ``service list`` document says of its read, beside its rows."""
    if doc is None:
        return "no document"
    keep = {k: v for k, v in doc.items() if k not in ("rows",)}
    return f"{json.dumps(keep, sort_keys=True)[:400]}, {len(doc.get('rows', []))} rows"


class Watchdog:
    """``zenctl watchdog --rule … --format ndjson``: the reference's reader
    that follows presence live, its transitions timed as they print."""

    def __init__(self, endpoint: str, rules: list[str], *, every: float = 0.5, namespace: str | None = None,
                 extra: list[str] | None = None) -> None:
        if not P.zenctl_bin.is_file():
            raise NotPairable(f"{P.zenctl_bin} not found")
        args = [str(P.zenctl_bin), "watchdog", "-c", endpoint, "--format", "ndjson", "--every", f"{every:g}"]
        for r in rules:
            args += ["--rule", r]
        if namespace is not None:
            args += ["--namespace", namespace]
        self.proc = subprocess.Popen(args + list(extra or []), stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                     text=True)
        self.t0 = time.monotonic()
        self.rows: list[tuple[float, dict[str, Any]]] = []
        self.err: list[str] = []
        self.lock = threading.Lock()
        threading.Thread(target=self._pump, daemon=True).start()
        threading.Thread(target=lambda: [self.err.append(x) for x in self.proc.stderr], daemon=True).start()

    def _pump(self) -> None:
        for line in self.proc.stdout:
            try:
                row = json.loads(line)
            except ValueError:
                continue
            with self.lock:
                self.rows.append((time.monotonic(), row))

    def transitions(self, rule: str) -> list[tuple[float, str, str]]:
        with self.lock:
            return [(t, r.get("to"), r.get("evidence", "")) for t, r in self.rows if r.get("rule") == rule]

    def state(self, rule: str) -> str | None:
        tr = self.transitions(rule)
        return tr[-1][1] if tr else None

    def close(self) -> None:
        try:
            self.proc.terminate()
            self.proc.wait(5)
        except Exception:  # noqa: BLE001
            self.proc.kill()


# -- §1 to §3 --------------------------------------------------------------------------------------

def section1(report: Report) -> None:
    """§1 Bring-up order and tokens: the owner a client of R1, a tool up
    before it."""
    import zenoh

    from . import live
    from .live_interop import _r1

    sec = f"§1 bring-up order and tokens ({P.owner} owner, {P.reader} reader)"
    system, svc = "py-site", "bringup"
    r1, ep, _ = _r1()
    tool = live.open_client(ep)
    owners: list[Holder] = []
    try:
        if P.reader == "zenctl":
            owners.append(Holder(ep, f"{system}/{svc}", contracts=[BRINGUP]))
            consumer = _pure_consumer(ep, f"{system}/consumer")
            if consumer is not None:
                owners.append(consumer)
            ok = _wait(lambda: len(_rows(_zenctl_list(ep, system=system)[1])) >= len(owners), 5.0, 0.2)
            rc, doc, err, _ = _zenctl_list(ep, system=system)
            rows = _rows(doc)
            o = rows.get(owners[0].instance, {})
            fp = (o.get("descriptor", {}).get("descriptor", {}).get("interfaces") or [{}])[0].get("contract", "")
            report.check(sec, "zenctl service list: the owner holds an instance token and one interface token, "
                              "its fingerprint prefix the descriptor's",
                         ok and rc == 0 and o.get("instance_token") is True and len(o.get("interfaces", [])) == 1
                         and o["interfaces"][0].get("token") == fp.removeprefix("sha256:")[:16],
                         f"rc {rc}, {json.dumps(o.get('interfaces'))[:200]}; {err}")
            if consumer is None:
                report.cannot(sec, "a pure consumer: the owner example starts with no contract only beside a "
                                   "profile; not asked")
            else:
                c = rows.get(consumer.instance, {})
                report.check(sec, "zenctl service list: the pure consumer holds an instance token only",
                             c.get("instance_token") is True and not c.get("interfaces"),
                             json.dumps(c.get("interfaces")))
            report.cannot(sec, "the call and the GET the moment the token appears: a zenctl command is not "
                               "launched by a token; asked of zk2py's reader")
            return
        got_instance, got_alive = threading.Event(), threading.Event()
        tokens: list[str] = []

        def on_token(smp: Any) -> None:
            k = str(smp.key_expr)
            tokens.append(k)
            if f"zk2/{system}/{svc}/@zk/instance/" in k:
                got_instance.set()
            if f"zk2/{system}/{svc}/@zk/alive/" in k:
                got_alive.set()

        puts: list[bytes] = []
        sub, placed = live.watch_or_beside(tool, SELECTOR, zenoh.handlers.Callback(on_token), history=True)
        dsub = tool.declare_subscriber(INSTANCES, zenoh.handlers.Callback(lambda s: puts.append(s.payload.to_bytes())))
        time.sleep(0.2)
        h = Holder(ep, f"{system}/{svc}", contracts=[BRINGUP])
        owners.append(h)
        base = f"zk2/{system}/{svc}/zk2py_bringup.v1"
        seen = got_instance.wait(5.0)
        d = live.get_descriptor(tool, h.key) if seen else []
        fp = live.fingerprint_of(json.loads(d[0].payload), "zk2py_bringup.v1") if len(d) == 1 and d[0].ok else None
        b = live.retrieve_bundle(tool, "zk2py_bringup.v1", fp) if fp else None
        report.check(sec, "the moment the instance token appears: a GET of the descriptor and of the bundle answer",
                     seen and len(d) == 1 and d[0].ok and b is not None and b.available,
                     f"token {seen}, descriptor {len(d)} replies, bundle {b and b.available}")
        alive = got_alive.wait(5.0)
        c = live.call(tool, f"{base}/@op/echo", b"ping") if alive else None
        st = live.get_state(tool, f"{base}/state/health") if alive else None
        report.check(sec, "the moment the interface token appears: the call succeeds (alive ⇒ callable)",
                     c is not None and [r.kind for r in c.replies] == ["value"],
                     str(c and [(r.kind, r.payload) for r in c.replies]))
        report.check(sec, "and the state GET, All + Latest, returns the value the owner started with",
                     st is not None and len(st.replies) == 1 and not st.replies[0].deleted,
                     str(st and [r.payload for r in st.replies]))
        report.check(sec, "a data subscriber up before the owner receives its first descriptor, without a GET",
                     _wait(lambda: any(h.instance.encode() in x for x in puts), 2.0), f"{len(puts)} puts")
        p = live.list_presence(tool, f"zk2/{system}/{svc}/@zk/**")
        report.check(sec, "the owner holds an instance token and one alive/zk2py_bringup.v1 token, and no member "
                          "token: its templated state has no member",
                     len(p.instances) == 1 and [a["iface"] for a in p.alive] == ["zk2py_bringup.v1"] and not p.members
                     and p.complete, p.reading)
        consumer = _pure_consumer(ep, f"{system}/consumer")
        if consumer is None:
            report.cannot(sec, "a pure consumer: the owner example starts with no contract only beside a profile")
        else:
            owners.append(consumer)
            _wait(lambda: bool(live.list_presence(tool, f"zk2/{system}/consumer/@zk/**").instances), 5.0)
            q = live.list_presence(tool, f"zk2/{system}/consumer/@zk/**")
            report.check(sec, "the pure consumer holds an instance token only",
                         len(q.instances) == 1 and not q.alive and not q.members and q.complete, q.reading)
        for x in (sub, dsub):
            x.undeclare()
    finally:
        for o in owners:
            o.close()
        live.close(tool)
        r1.close()


def _pure_consumer(endpoint: str, address: str) -> Holder | None:
    """A service implementing nothing: zk2py's owner with no contract; the
    owner example refuses a start with none (its usage asks for at least
    one contract, or a profile)."""
    try:
        return Holder(endpoint, address, contracts=[])
    except Exception:  # noqa: BLE001 - not offered
        return None


def section2(report: Report) -> None:
    """§2 steps 1, 3 and 4: the descriptor GET, and an owner whose required
    role is bound to nothing, watched through R1, with its control."""
    import zenoh

    from . import live
    from .contract import load_contract
    from .descriptor import check_descriptor
    from .live_interop import _r1

    sec = f"§2 the descriptor ({P.owner} owner, {P.reader} reader)"
    r1, ep, _ = _r1()
    tool = live.open_client(ep)
    try:
        h = Holder(ep, "lab/described", contracts=[ECHO])
        try:
            if P.reader == "zenctl":
                _wait(lambda: h.instance in _rows(_zenctl_list(ep, system="lab")[1]), 5.0, 0.2)
                rc, doc, err, _ = _zenctl_list(ep, system="lab")
                row = _rows(doc).get(h.instance, {})
                served = row.get("descriptor", {})
                d = json.dumps(served.get("descriptor", {})).encode()
                codes = check_descriptor(d, [load_contract(ECHO)])
                report.check(sec, "1. zenctl reads the descriptor at the instance key: served, valid for zk2py's "
                                  "checker", served.get("answer") == "served" and codes == [],
                             f"{served.get('answer')}, codes {codes}; {err}")
            else:
                _wait(lambda: bool(live.list_presence(tool, "zk2/lab/described/@zk/**").instances), 5.0)
                d = live.get_descriptor(tool, h.key)
                one = len(d) == 1 and d[0].ok
                codes = check_descriptor(d[0].payload, [load_contract(ECHO)]) if one else ["no reply"]
                report.check(sec, "1. one reply, application/json, no timestamp and no attachment, valid against "
                                  "the descriptor schema and the contract",
                             one and d[0].encoding.startswith("application/json") and not d[0].has_timestamp
                             and not d[0].has_attachment and codes == [],
                             f"{len(d)} replies, {d[0].encoding if d else ''}, codes {codes}")
        finally:
            h.close()
        # steps 3 and 4: thruster.v1's required role `cmd`, bound to nothing
        heard: list[str] = []
        sub, _ = live.watch_or_beside(tool, "zk2/vehicle-01/thrusters/@zk/**",
                                      zenoh.handlers.Callback(lambda s: heard.append(str(s.key_expr))), history=True)
        refused, why = None, ""
        try:
            x = Holder(ep, "vehicle-01/thrusters", contracts=[THRUSTER])
            refused = False
            x.close()
        except Exception as e:  # noqa: BLE001 - the refusal
            refused, why = True, str(e)[:200]
        time.sleep(1.0)
        after = live.list_presence(tool, "zk2/vehicle-01/thrusters/@zk/**")
        if P.reader == "zenctl":
            rc, doc, err, _ = _zenctl_list(ep, system="vehicle-01")
            gone = rc in (0, 2) and not _rows(doc)
            detail = f"zenctl rc {rc}, {len(_rows(doc))} rows; {err}"
        else:
            gone, detail = after.complete and after.count == 0, after.reading
        report.check(sec, "4. a required role bound to nothing: the owner does not start; R1's subscriber hears no "
                          "token, and a read through R1 afterwards returns none",
                     refused is True and not heard and gone, f"{why}; heard {heard}; {detail}")
        sub.undeclare()
        # the control: the same address, an owner that may start
        heard.clear()
        sub, _ = live.watch_or_beside(tool, "zk2/vehicle-01/control/@zk/**",
                                      zenoh.handlers.Callback(lambda s: heard.append(str(s.key_expr))), history=True)
        ctl = Holder(ep, "vehicle-01/control", contracts=[ECHO])
        try:
            report.check(sec, "4. its control, an owner whose roles are all bound (the echo contract, which has none: "
                              "the owner example takes no binding), shows its instance token to that subscriber "
                              "within the wait", _wait(lambda: any("/@zk/instance/" in k for k in heard), 1.0),
                         f"heard {heard}")
        finally:
            ctl.close()
            sub.undeclare()
    finally:
        live.close(tool)
        r1.close()


def section3(report: Report) -> None:
    """§3 steps 1 and 3: a make-before-break re-mint seen as 1 or 2 live
    instance tokens, never 0; an owner whose contract has an epoch template
    starts holding no member token."""
    import zenoh

    from . import live
    from .live_interop import _r1

    sec = f"§3 epochs and re-minting ({P.owner} owner, {P.reader} reader)"
    r1, ep, _ = _r1()
    tool = live.open_client(ep)
    try:
        held: set[str] = set()
        counts: list[int] = []
        lock = threading.Lock()

        def tok(s: Any) -> None:
            with lock:
                (held.discard if s.kind == zenoh.SampleKind.DELETE else held.add)(str(s.key_expr))
                counts.append(len(held))

        sub, _ = live.watch_or_beside(tool, "zk2/lab/mint/@zk/instance/*", zenoh.handlers.Callback(tok), history=True)
        h = Holder(ep, "lab/mint", remint=True)
        try:
            _wait(lambda: h.key in held, 5.0)
            old = h.key
            if P.reader == "zenctl":
                before = _rows(_zenctl_list(ep, system="lab")[1])
            new = h.remint()
            _wait(lambda: new in held and old not in held, 5.0)
            time.sleep(0.3)
            with lock:
                seen = list(counts)
            report.check(sec, "1. the re-mint: a watcher sees 1 or 2 live instance tokens throughout, never 0, and "
                              "the new one before the old one goes", seen and min(seen) >= 1 and max(seen) <= 2
                         and 2 in seen and held == {new}, f"counts {seen}, old {old[-16:]}, new {new[-16:]}")
            if P.reader == "zenctl":
                rows = _rows(_zenctl_list(ep, system="lab")[1])
                report.check(sec, "1. zenctl service list: the old instance before, the new one after, one each",
                             list(before) == [old.rsplit('/', 1)[1]] and list(rows) == [new.rsplit('/', 1)[1]],
                             f"before {list(before)}, after {list(rows)}")
        finally:
            h.close()
            sub.undeclare()
        e = Holder(ep, "lab/epoch", contracts=[EPOCH])
        try:
            _wait(lambda: bool(live.list_presence(tool, "zk2/lab/epoch/@zk/**").instances), 5.0)
            p = live.list_presence(tool, "zk2/lab/epoch/@zk/**")
            report.check(sec, "3. an owner whose contract has an epoch template starts, declares no member, and holds "
                              "no member token", len(p.instances) == 1 and not p.members and p.complete, p.reading)
        finally:
            e.close()
        report.cannot(sec, "2. a member's continuity broken: neither owner offers one from outside")
    finally:
        live.close(tool)
        r1.close()


# -- §5, §6 ----------------------------------------------------------------------------------------

def section5(report: Report, n: int | None = None) -> None:
    """§5 A tokenless set: services implementing nav.v2 with health.v1 in
    their tokenless set; a tool finds every health.v1 provider from instance
    tokens and descriptors."""
    from . import live
    from .live_interop import _r1

    n = n if n is not None else P.tokenless
    sec = f"§5 a tokenless set ({P.owner} owner, {P.reader} reader, {n} services)"
    r1, ep, _ = _r1()
    tool = live.open_client(ep)
    owners: list[Holder] = []
    try:
        for i in range(n):
            owners.append(Holder(ep, f"fleet/s{i:03d}", contracts=[NAV, HEALTH], tokenless={"health.v1"}))
        _wait(lambda: len(live.list_presence(tool, "zk2/fleet/*/@zk/instance/*").instances) == n, 10.0)
        p = live.list_presence(tool, "zk2/fleet/*/@zk/**", timeout=5.0)
        ifaces = sorted({a["iface"] for a in p.alive})
        report.check(sec, f"each instance holds an instance token and an alive/nav.v2 token, and no "
                          f"alive/health.v1 token: {2 * n} tokens, not {3 * n}",
                     len(p.instances) == n and len(p.alive) == n and ifaces == ["nav.v2"] and p.complete,
                     f"{len(p.instances)} instances, {len(p.alive)} alive {ifaces}; {p.reading}")
        if P.reader == "zenctl":
            rc, doc, err, secs = _zenctl_list(ep, system="fleet", timeout_s=5.0)
            rows = _rows(doc)

            def lists(r: dict[str, Any]) -> bool:
                es = r.get("descriptor", {}).get("descriptor", {}).get("interfaces", [])
                return any(e.get("iface") == "health.v1" and e.get("token") is False for e in es)

            found = [r for r in rows.values() if lists(r)]
            report.check(sec, "zenctl lists the health.v1 providers from instance tokens and descriptors: every "
                              "one, each descriptor marking health.v1 \"token\": false",
                         rc == 0 and len(found) == n, f"rc {rc}, {len(found)} of {len(rows)} rows, {secs:.1f} s; {err}")
            return
        marked, found = 0, 0
        for x in p.instances:
            d = live.get_descriptor(tool, f"zk2/{x['system']}/{x['service']}/@zk/instance/{x['instance']}")
            doc = json.loads(d[0].payload) if len(d) == 1 and d[0].ok else {}
            es = [e for e in doc.get("interfaces", []) if e.get("iface") == "health.v1"]
            found += bool(es)
            marked += bool(es) and all(e.get("token") is False for e in es)
        report.check(sec, "each descriptor marks health.v1 \"token\": false, and a tool lists the health.v1 "
                          "providers from instance tokens and descriptors: every one",
                     found == n and marked == n, f"{found} list it, {marked} mark it tokenless")
    finally:
        for o in owners:
            o.close()
        live.close(tool)
        r1.close()


def section6(report: Report) -> None:
    """§6 Possibly incomplete, and refused: R1 denies ``liveliness_query`` on
    ingress for ``zk2/*/*/@zk/alive/**``; the owner and a tool are its
    clients; a second tool reaches it through a link held back."""
    from . import live
    from .live_interop import _acl_router, _StallProxy

    sec = f"§6 possibly incomplete, and refused ({P.owner} owner, {P.reader} reader)"
    r1, ep = _acl_router("ingress")
    h = None
    try:
        h = Holder(ep, "py-site/guarded")
        own = None
        if _wait(lambda: bool(live.list_presence(r1, "zk2/py-site/guarded/@zk/alive/**").alive), 5.0):
            own = live.list_presence(r1, "zk2/py-site/guarded/@zk/alive/**")
        report.check(sec, "1. R1's own session reads the interface token", own is not None and len(own.alive) == 1,
                     own.reading if own else "not read")
        if P.reader == "zenctl":
            rc, doc, err, _ = _zenctl_list(ep, system="py-site")
            row = _rows(doc).get(h.instance, {})
            report.check(sec, "2. zenctl's read through R1 is of …/@zk/**, wider than the deny, which works by "
                              "inclusion (§11.3): not refused, it holds the instance token and the interface token",
                         rc == 0 and row.get("instance_token") is True and len(row.get("interfaces") or []) == 1
                         and (row["interfaces"][0].get("token") or "") != "", f"rc {rc}; {_brief(doc)}; "
                                                                             f"{json.dumps(row.get('interfaces'))[:200]}")
            report.cannot(sec, "2. the alive read alone (zk2/*/*/@zk/alive/**), refused as absence: zenctl's verbs "
                               "read …/@zk/** whole; asked of zk2py's reader")
            link = DelayLink(ep)
            try:
                rc, doc, err, secs = _zenctl_list(link.endpoint, system="py-site", timeout_s=1.0)
                report.check(sec, "3. the second tool (zenctl) through a link flowing: the owner's instance token, "
                                  "complete", rc == 0 and h.instance in _rows(doc) and (doc or {}).get("complete")
                             is True, f"rc {rc}, {secs:.1f} s; {_brief(doc)}; {err}")
                link.delay_s = 0.8
                rc, doc, err, secs = _zenctl_list(link.endpoint, system="py-site", timeout_s=1.0)
                report.check(sec, "3. its replies held back past the read's timeout (0.8 s each way, a 1 s timeout): "
                                  "possibly incomplete, never absence",
                             (doc or {}).get("complete") is False or rc == 2,
                             f"rc {rc}, {secs:.1f} s; {_brief(doc)}; {err}")
            finally:
                link.close()
            return
        tool = live.open_client(ep)
        try:
            inst = live.list_presence(tool, INSTANCES)
            alive = live.list_presence(tool, "zk2/*/*/@zk/alive/**")
        finally:
            live.close(tool)
        report.check(sec, "2. the instance read holds the owner's instance token, and no error reply",
                     [i["instance"] for i in inst.instances] == [h.instance] and inst.complete, inst.reading)
        report.check(sec, "2. the alive read is answered: a final reply, no token and no error reply, the same as "
                          "a selector no token matches", alive.complete and alive.count == 0 and not alive.errors,
                     alive.reading)
        proxy = _StallProxy(ep)
        try:
            tool2 = live.open_client(proxy.endpoint)
            try:
                flowing = live.list_presence(tool2, INSTANCES)
                proxy.flowing.clear()
                try:
                    stalled = live.list_presence(tool2, INSTANCES)
                finally:
                    proxy.flowing.set()
            finally:
                live.close(tool2)
        finally:
            proxy.close()
        report.check(sec, "3. the second tool's read with the link flowing holds the owner's instance token, no "
                          "error reply", [i["instance"] for i in flowing.instances] == [h.instance] and flowing.complete,
                     flowing.reading)
        report.check(sec, "3. held back past its timeout: the error reply Timeout and no token, read possibly "
                          "incomplete, never absence", not stalled.complete and stalled.errors == ["zenoh/string: Timeout"]
                     and stalled.count == 0 and stalled.kind == live.POSSIBLY_INCOMPLETE, stalled.reading)
    finally:
        if h is not None:
            h.close()
        r1.close()


# -- §7 to §9 --------------------------------------------------------------------------------------

def section7(report: Report) -> None:
    """§7 A read through a session cut off."""
    from . import live

    sec = f"§7 a read through a session cut off ({P.owner} owner, {P.reader} reader)"
    bus = Bus()
    try:
        bus.start_owner()
        if P.reader == "zenctl":
            _section7_zenctl(report, sec, bus)
            return
        report.check(sec, "1. the tool reads the owner's instance token", _wait(bus.tool_reads, 5.0))
        report.check(sec, "2. cut: the tool's session holds no transport, and no link", bus.cut())
        fin, oks, errs, secs = _raw_read(bus.tool)
        report.check(sec, "3. zenoh answers the GET at once: a final reply, no token and no error reply",
                     fin and not oks and errs == 0 and secs < 0.5, f"{len(oks)} tokens, {errs} errors, {secs:.3f} s")
        own = _raw_read(bus.router)
        report.check(sec, "3. R1's own session reads the owner's instance token: it never left", bus.key in own[1])
        p = live.list_presence(bus.tool)
        report.check(sec, "3. zk2py's reader makes no such read, and says why: not made, cut off",
                     p.kind == live.NOT_MADE and p.not_made == live.CUT_OFF, p.reading)
        report.check(sec, "4. heal: the tool's session holds a link again", bus.heal())
        report.check(sec, "4. the tool's read holds the owner's instance token", _wait(bus.tool_reads, 5.0))
        # Step 5 (0.38, F-185): stall the link, ask a read, cut it in flight.
        made, listed = 0, 0
        for run in range(10):
            bus.link.stall()
            got: dict[str, Any] = {}
            t0 = time.monotonic()
            th = threading.Thread(target=lambda: got.setdefault("p", live.list_presence(bus.tool, timeout=5.0)))
            th.start()
            time.sleep(0.3)
            t_cut = time.monotonic()
            bus.link.cut()
            th.join(10.0)
            p = got.get("p")
            at_cut = p is not None and p.elapsed_s < (t_cut - t0) + 0.5
            if (p is not None and at_cut and not p.instances and not p.errors and p.cut_off_during
                    and p.kind == live.POSSIBLY_INCOMPLETE and live.CUT_OFF_DURING in p.reading):
                made += 1
            if p is not None and p.cut_off_during and p.transports_at_end:
                listed += 1
            bus.heal()
            _wait(bus.tool_reads, 5.0)
        report.check(sec, "5. (0.38) a read asked while the link is stalled, and cut in flight, is answered at the "
                          "cut, no token, no error reply; the check before it passed, and the one at its end, by the "
                          "session's links, reads it possibly incomplete, cut off during it, never the owner absent",
                     made == 10, f"{made} of 10 runs")
        report.note(sec, f"5. at the read's end, the session's links were gone in each of those runs, and "
                         f"Session.info.transports() still listed the closing transport in {listed} of 10 "
                         "(core §8.1, 0.38, \"What the check reads\")")
    finally:
        bus.close()


def _section7_zenctl(report: Report, sec: str, bus: Bus) -> None:
    """§7 with the reference's tool, one process per read: it holds no
    session across the cut, so step 3's read through a session already cut
    off is its connection refused; step 5 cuts a read in flight through a
    long path (:class:`DelayLink`), at several instants."""
    link = DelayLink(bus.endpoint)
    try:
        ok = _wait(lambda: bus.holder.instance in _rows(_zenctl_list(link.endpoint, system="lab")[1]), 5.0, 0.2)
        report.check(sec, "1. zenctl reads the owner's instance token", ok)
        link.cut()
        rc, doc, err, secs = _zenctl_list(link.endpoint, system="lab")
        own = _raw_read(bus.router)
        report.check(sec, "3. through the cut link: no read made, no verdict (exit 2), never the owner absent; R1's "
                          "own session reads the token", rc == 2 and not (doc or {}).get("complete")
                     and bus.key in own[1], f"rc {rc}, {secs:.1f} s; {_brief(doc)}; {err}")
        link.heal()
        rc, doc, err, _ = _zenctl_list(link.endpoint, system="lab")
        report.check(sec, "4. healed: zenctl's read holds the owner's instance token",
                     rc == 0 and bus.holder.instance in _rows(doc), f"rc {rc}; {err}")
        outcomes = []
        wrong = 0
        for offset in (0.9, 1.3, 1.7, 2.1, 2.5, 2.9, 3.3):
            link.heal()
            link.delay_s = 0.4
            res: dict[str, Any] = {}
            th = threading.Thread(target=lambda: res.setdefault("r", _zenctl_list(link.endpoint, system="lab",
                                                                                  timeout_s=5.0)))
            th.start()
            time.sleep(offset)
            link.cut()
            th.join(60.0)
            rc, doc, err, secs = res.get("r", (None, None, "", 0.0))
            complete = (doc or {}).get("complete")
            absent = complete is True and bus.holder.instance not in _rows(doc)
            wrong += absent
            outcomes.append(f"cut at {offset:.1f} s: rc {rc}, complete {complete}, "
                            f"{len(_rows(doc))} rows" + (f", {_brief(doc)[:160]}" if complete is not True else ""))
            link.delay_s = 0.0
        report.check(sec, "5. a read cut in flight (a 0.4 s path, cut at 0.9 to 3.3 s): never complete without the "
                          "owner", wrong == 0, "; ".join(outcomes))
    finally:
        link.close()


def section8(report: Report) -> None:
    """§8 A withdrawal heard live, read by the reader under test."""
    from . import live
    from . import presence as pr

    sec = f"§8 a withdrawal heard live ({P.owner} owner, {P.reader} reader)"
    bus = Bus()
    try:
        if P.reader == "zenctl":
            _section8_zenctl(report, sec, bus)
            return
        heard: list[tuple[str, str]] = []
        watch = live.watch_session(bus.tool)
        sub = live.declare_liveliness_subscriber(watch, INSTANCES,
                                                 lambda s: heard.append((str(s.kind), str(s.key_expr))),
                                                 history=True)
        bus.extra.append(sub)
        f = pr.PresenceFollower(bus.tool, watch, INSTANCES)
        bus.extra.append(f)
        bus.start_owner()
        report.check(sec, "1. the subscriber hears the owner's instance token, and the tool reads it",
                     _wait(lambda: any("PUT" in k for k, _ in heard), 5.0) and _wait(bus.tool_reads, 5.0))
        report.check(sec, "1. zk2py's follower reads the owner up", _wait(lambda: f.address("lab", "owner")[0] == pr.UP, 5.0),
                     str(f.address("lab", "owner")))
        made_before = len(f.reads)
        bus.cut()
        report.check(sec, "2. on the cut, the subscriber hears the token withdrawn",
                     _wait(lambda: any("DELETE" in k for k, _ in heard), 5.0))
        report.check(sec, "2. R1's own session still reads it: the owner never left", bus.key in _raw_read(bus.router)[1])
        _wait(lambda: len(f.reads) > made_before, 5.0)
        st = f.address("lab", "owner")
        report.check(sec, "2. zk2py's follower reads the owner unobservable (withdrawn, unconfirmed), never down, "
                          "and its re-read is not made while cut off",
                     st == (pr.UNOBSERVABLE, pr.WITHDRAWN) and all(p.kind == live.NOT_MADE for p in f.reads[made_before:])
                     and len(f.reads) > made_before, f"{st}, {[p.kind for p in f.reads[made_before:]]}")
        bus.heal()
        report.check(sec, "2. on the heal, the subscriber hears it again",
                     _wait(lambda: [k for k, _ in heard].count("SampleKind.PUT") >= 2, 10.0))
        report.check(sec, "2. the tool's read holds it, and the follower reads the owner up",
                     _wait(bus.tool_reads, 5.0) and _wait(lambda: f.address("lab", "owner")[0] == pr.UP, 5.0),
                     str(f.address("lab", "owner")))
        report.note(sec, f"2. (0.41) the follower heard its watch session's transport move "
                         f"{[k for _, k in f.moves]}, and made {len(f.round_trips)} round trips")
        bus.holder.withdraw()
        report.check(sec, "3. the owner stops cleanly: the subscriber hears the token withdrawn",
                     _wait(lambda: [k for k, _ in heard].count("SampleKind.DELETE") >= 2, 5.0))
        report.check(sec, "3. the tool's read after it is complete and holds no instance token of the owner, and the "
                          "follower reads it down only from that read",
                     _wait(lambda: f.address("lab", "owner")[0] == pr.DOWN, 5.0) and f.reads[-1].kind == live.COMPLETE,
                     str(f.address("lab", "owner")))
        # 4. three owners on three addresses, stopped at once
        three = [Holder(bus.endpoint, f"lab/b{n}") for n in range(1, 4)]
        bus.extra += three
        up = _wait(lambda: all(f.address("lab", f"b{n}")[0] == pr.UP for n in range(1, 4)), 5.0)
        _wait(lambda: f.idle, 5.0)
        before = len(f.reads)
        ths = [threading.Thread(target=x.withdraw) for x in three]
        for t in ths:
            t.start()
        for t in ths:
            t.join()
        ok = _wait(lambda: all(f.address("lab", f"b{n}")[0] == pr.DOWN for n in range(1, 4)), 5.0)
        ok = _wait(lambda: f.idle, 5.0) and ok
        report.check(sec, "4. three owners up, stopped at once: the reader makes one read for the three withdrawals, "
                          "and reports all three down from it", up and ok and len(f.reads) - before == 1,
                     f"{len(f.reads) - before} reads, {[f.address('lab', f'b{n}') for n in range(1, 4)]}")
        # 5. a make-before-break re-mint
        m = Holder(bus.endpoint, "lab/m", remint=True)
        bus.extra.append(m)
        _wait(lambda: f.address("lab", "m")[0] == pr.UP, 5.0)
        _wait(lambda: f.idle, 5.0)
        before = len(f.reads)

        def heard_it(key: str, kind: str) -> bool:
            with f.lock:
                return any(k == key and what == kind for _, k, what in f.view.events)

        old_key = m.key
        states = []
        stop = threading.Event()

        def sample() -> None:
            while not stop.is_set():
                states.append(f.address("lab", "m")[0])
                time.sleep(0.005)

        sampler = threading.Thread(target=sample)
        sampler.start()
        new_key = m.remint()
        _wait(lambda: heard_it(new_key, "put") and heard_it(old_key, "delete"), 5.0)
        _wait(lambda: f.idle, 5.0)
        stop.set()
        sampler.join()
        report.check(sec, "5. a make-before-break re-mint: the reader makes no read, and the address stays up "
                          "throughout", len(f.reads) == before and set(states) == {pr.UP} and not f.view.owes_read,
                     f"{len(f.reads) - before} reads, {len(states)} samples, states {sorted(set(states))}")
    finally:
        bus.close()


def _section8_zenctl(report: Report, sec: str, bus: Bus) -> None:
    """§8 with ``zenctl watchdog --rule 'instance-gone …'``, the reference's
    reader that follows presence live, its sessions through the tool's link:
    ok while held, never firing on a withdrawal heard across a cut, firing
    once a complete read finds none."""
    from .budget_scenarios import Link

    link = Link(bus.endpoint)
    wd = None
    try:
        bus.start_owner()
        wd = Watchdog(link.endpoint, ["instance-gone lab/owner", "instance-gone three/*", "instance-gone mint/m"])
        r1, r3, rm = "instance-gone lab/owner", "instance-gone three/*", "instance-gone mint/m"
        three = [Holder(bus.endpoint, f"three/b{n}") for n in range(1, 4)]
        m = Holder(bus.endpoint, "mint/m", remint=True)
        bus.extra += three + [m]
        report.check(sec, "1. the watchdog reads the owner up: ok", _wait(lambda: wd.state(r1) == "ok", 10.0),
                     str(wd.transitions(r1)))
        n0 = len(wd.transitions(r1))
        link.cut()
        time.sleep(3.0)
        own = _raw_read(bus.router)
        during = [to for _, to, _ in wd.transitions(r1)[n0:]]
        report.check(sec, "2. while the link is cut: never firing (the owner never left: R1 reads it)",
                     "firing" not in during and bus.key in own[1], f"transitions {during}")
        link.heal()
        report.check(sec, "2. on the heal: ok", _wait(lambda: wd.state(r1) == "ok", 15.0), str(wd.transitions(r1)[n0:]))
        _wait(lambda: wd.state(r3) == "ok" and wd.state(rm) == "ok", 10.0)
        nm = len(wd.transitions(rm))
        bus.holder.withdraw()
        report.check(sec, "3. the owner stops cleanly: firing, from a complete read",
                     _wait(lambda: wd.state(r1) == "firing", 10.0), str(wd.transitions(r1)[-1:]))
        ths = [threading.Thread(target=x.withdraw) for x in three]
        for t in ths:
            t.start()
        for t in ths:
            t.join()
        report.check(sec, "4. three owners stopped at once: firing (one read for the three is not observable from "
                          "the tool's output)", _wait(lambda: wd.state(r3) == "firing", 10.0),
                     str(wd.transitions(r3)[-1:]))
        m.remint()
        time.sleep(3.0)
        report.check(sec, "5. a make-before-break re-mint: the rule stays ok throughout, no transition",
                     len(wd.transitions(rm)) == nm and wd.state(rm) == "ok", str(wd.transitions(rm)))
    finally:
        if wd is not None:
            wd.close()
        link.close()


def section9(report: Report) -> None:
    """§9 A read beside a liveliness subscriber."""
    import zenoh

    from . import live

    sec = f"§9 a read beside a liveliness subscriber ({P.owner} owner, {P.reader} reader)"
    if P.reader == "zenctl":
        report.cannot(sec, "zenctl's watch session's GETs, and the instant its seed is answered, are not observable "
                           "from its output: steps 4 and 5 are asked of zk2py's reader; steps 1 to 3 are zenoh's")
        return
    bus = Bus()
    try:
        bus.start_owner()
        report.check(sec, "1. the tool reads the owner's instance token", _wait(bus.tool_reads, 5.0))
        watch = live.open_client(bus.endpoint)
        heard: list[str] = []
        sub = watch.liveliness().declare_subscriber(INSTANCES,
                                                    zenoh.handlers.Callback(lambda s: heard.append(str(s.key_expr))),
                                                    history=True)
        fin, oks, errs, _ = _raw_read(watch)
        report.check(sec, "2. the watch session's GET, made at once beside its history subscriber, ends with the "
                          "final reply and no error reply, and without the owner's instance token",
                     fin and errs == 0 and bus.key not in oks, f"{len(oks)} tokens")
        report.check(sec, "2. the subscriber does hear it", _wait(lambda: bus.key in heard, 5.0))
        fin, oks, errs, _ = _raw_read(bus.tool)
        report.check(sec, "3. the tool's GET, on a session with no liveliness subscriber, holds it",
                     fin and errs == 0 and bus.key in oks)
        marked = live.open_client(bus.endpoint)
        m_sub = live.declare_liveliness_subscriber(marked, INSTANCES, lambda s: None, history=True)
        p = live.list_presence(marked)
        report.check(sec, "3. zk2py's reader makes no read on a session that holds a liveliness subscriber it "
                          "declared, and reports it not made",
                     p.kind == live.NOT_MADE and p.not_made == live.BESIDE_SUBSCRIBER, p.reading)
        for x in (sub, m_sub):
            x.undeclare()
        live.close(watch)
        live.close(marked)
        _section9_reader(report, sec, bus)
    finally:
        bus.close()


class _Counted:
    """A session seen through: every liveliness GET it is asked to make is
    counted, and ``on_answer`` runs when the first one's final reply comes,
    before the reader sees it (presence.md §9 step 5: "withdrawn as soon as
    the reader's seed is answered")."""

    def __init__(self, inner: Any, on_answer: Any = None) -> None:
        self._inner, self.on_answer = inner, on_answer
        self.gets: list[tuple[float, str]] = []

    def liveliness(self) -> Any:
        import zenoh

        lv, outer = self._inner.liveliness(), self

        class _Lv:
            def get(self, selector: str, handler: Any, **kw: Any) -> Any:
                outer.gets.append((time.monotonic(), selector))
                hook, outer.on_answer = outer.on_answer, None
                if hook is not None:
                    drop = handler.drop

                    def dropped() -> None:
                        hook()
                        drop()

                    handler = zenoh.handlers.Callback(handler.callback, dropped)
                return lv.get(selector, handler, **kw)

            def __getattr__(self, name: str) -> Any:
                return getattr(lv, name)

        return _Lv()

    def __getattr__(self, name: str) -> Any:
        return getattr(self._inner, name)


def _section9_reader(report: Report, sec: str, bus: Bus) -> None:
    """§9 steps 4 and 5 (0.38, F-189; 0.41: subscribers without history):
    the reader under test, zk2py's follower, observed."""
    from . import live
    from . import presence as pr
    from .budget_scenarios import Link

    # 4. its liveliness GETs, per session
    watch = _Counted(live.open_client(bus.endpoint))
    reading = _Counted(live.open_client(bus.endpoint))
    f = pr.PresenceFollower(reading, watch, INSTANCES)
    _wait(lambda: f.address("lab", "owner")[0] == pr.UP, 5.0)
    w_gets, r_gets = list(watch.gets), list(reading.gets)
    asked = live.list_presence(watch, INSTANCES)
    report.check(sec, "4. (0.38) the reader's watch session makes one liveliness GET, the round trip, after its "
                      "subscribers (without history, 0.41) and before the seed, and no other; the seed and every "
                      "re-read go through the reading session",
                 len(w_gets) == 1 and w_gets[0][1] == live.settle_key(INSTANCES) and r_gets
                 and w_gets[0][0] < r_gets[0][0] and f.history is False,
                 f"watch {[g[1] for g in w_gets]}, reading {len(r_gets)}, history {f.history}")
    report.check(sec, "4. a read asked on its watch session is not made, beside a liveliness subscriber",
                 asked.kind == live.NOT_MADE and asked.not_made == live.BESIDE_SUBSCRIBER
                 and len(watch.gets) == len(w_gets), asked.reading)
    f.close()
    for x in (watch._inner, reading._inner):
        live.close(x)
    # 5. the watch link stalled from before the subscribers until 150 ms later
    results = {True: [0, 0], False: [0, 0]}
    for round_trip in (True, False):
        for run in range(10):
            addr = f"lab/s9r{run}"
            holder = Holder(bus.endpoint, addr)
            _wait(lambda: holder.key in _raw_read(bus.tool, INSTANCES)[1], 5.0)
            link = Link(bus.endpoint)
            w = live.open_client(link.endpoint)
            r = _Counted(live.open_client(bus.endpoint),
                         on_answer=lambda h=holder: threading.Thread(target=h.withdraw).start())
            link.stall()
            threading.Timer(0.15, link.flowing.set).start()
            f = pr.PresenceFollower(r, w, f"zk2/{addr}/@zk/instance/*", round_trip=round_trip)
            down = _wait(lambda: f.address("lab", f"s9r{run}")[0] == pr.DOWN, 3.0)
            with f.lock:
                heard = any(k == holder.key and what == "delete" for _, k, what in f.view.events)
            results[round_trip][0] += 1
            results[round_trip][1] += 1 if (heard and down) else 0
            f.close()
            holder.close()
            live.close(w)
            live.close(r._inner)
            link.close()
    report.check(sec, "5. (0.38) a fresh reader, its watch link stalled 150 ms from before its subscribers: it hears "
                      "the token go, withdrawn as soon as its seed is answered, and reports the owner down after its "
                      "re-read, in every run",
                 results[True][1] == results[True][0], f"{results[True][1]} of {results[True][0]} runs")
    report.check(sec, "5. the control, a reader that skips the round trip, never hears it go, and goes on reading "
                      "the owner up, in every run", results[False][1] == 0,
                 f"it heard the withdrawal in {results[False][1]} of {results[False][0]} runs")


def section4(report: Report) -> None:
    """§4 (0.38, F-186): the watch session's round trip beside a liveliness
    subscriber to at least 1,200 tokens, each GET waiting 5 s; its answer
    discarded, so no token is counted. The round trip asks 0.41's key."""
    import zenoh

    from . import live
    from .live_interop import _r1

    sec = "§4 reading presence at scale"
    if P.reader == "zenctl":
        report.cannot(sec, "the pairings of a GET's handler and a subscriber's are a binding's; zenctl's are its own")
        return
    router, endpoint, _ = _r1()
    holder = live.open_client(endpoint)
    tokens = [holder.liveliness().declare_token(f"zk2/scale/t{n}/@zk/instance/" + f"{n:016x}")
              for n in range(1200)]
    time.sleep(2.0)
    key = live.settle_key(INSTANCES)
    try:
        def round_trip(sub_callback: bool, get_callback: bool) -> tuple[bool, float, int]:
            w = live.open_client(endpoint)
            heard: list[Any] = []
            if sub_callback:
                sub = w.liveliness().declare_subscriber(INSTANCES, zenoh.handlers.Callback(heard.append), history=True)
            else:
                sub = w.liveliness().declare_subscriber(INSTANCES, history=True)  # never drained
            t0 = time.monotonic()
            errors = 0
            if get_callback:
                done = threading.Event()
                errs: list[Any] = []
                w.liveliness().get(key, zenoh.handlers.Callback(
                    lambda r: errs.append(r) if r.ok is None else None, done.set), timeout=5.0)
                finished = done.wait(15.0)
                errors = len(errs)
            else:
                replies = w.liveliness().get(key, timeout=5.0)  # drained as replies arrive
                finished = True
                for rep_ in replies:
                    if rep_.ok is None:
                        errors += 1
            dt = time.monotonic() - t0
            sub.undeclare()
            live.close(w)
            return finished and errors == 0 and dt < 5.0, dt, errors

        ok, dt, errs = round_trip(True, True)
        report.check(sec, "callbacks for both: the round trip completes, at the routers' final reply, before its "
                          "timeout; its answer discarded", ok, f"{dt:.2f} s, {errs} error replies")
        ok, dt, errs = round_trip(True, False)
        report.check(sec, "a GET handler drained as replies arrive, beside a callback subscriber: it completes too",
                     ok, f"{dt:.2f} s, {errs} error replies")
        ok, dt, errs = round_trip(False, True)
        report.check(sec, "a bounded subscriber nobody drains: a callback round trip ends at its timeout, an error "
                          "reply, which a tool reads possibly incomplete",
                     not ok and errs >= 1 and dt >= 4.9, f"{dt:.2f} s, {errs} error replies")
        report.note(sec, "not run: a GET on a bounded handler beside an undrained bounded subscriber, which hangs "
                         "(round 54 measured 0 replies after 20 s), and wedges zenoh's threads in process, as the "
                         "reference found")
    finally:
        for t in tokens:
            t.undeclare()
        live.close(holder)
        router.close()


# -- §10 (0.41) --------------------------------------------------------------------------------------

class _Watch:
    """A fresh watch session for §10: a client of ``endpoint`` (a link's, to
    stall it), its subscriber's samples timed as heard."""

    def __init__(self, endpoint: str, *, namespace: str | None = None, interests_timeout_s: float | None = None) -> None:
        from . import live

        kw: dict[str, Any] = {"namespace": namespace}
        if interests_timeout_s is not None:
            kw["interests_timeout_s"] = interests_timeout_s
        self.session = live.open_client(endpoint, **kw)
        _wait(lambda: live.links_held(self.session) is True, 10.0)
        self.heard: list[tuple[float, str, str]] = []
        self.sub: Any = None

    def subscribe(self, selector: str, history: bool, direct: bool = False) -> None:
        import zenoh

        from . import live

        def got(s: Any) -> None:
            self.heard.append((time.monotonic(), "delete" if s.kind == zenoh.SampleKind.DELETE else "put",
                               str(s.key_expr)))

        self.sub = live.declare_liveliness_subscriber(self.session, selector,
                                                      zenoh.handlers.Callback(got, indirect=not direct),
                                                      history=history)

    def heard_gone(self, key: str) -> bool:
        return any(k.endswith(key) and kind == "delete" for _, kind, k in self.heard)

    def close(self) -> None:
        from . import live

        try:
            if self.sub is not None:
                self.sub.undeclare()
        except Exception:  # noqa: BLE001
            pass
        live.close(self.session)


def _reads_gone(session: Any, key: str, selector: str = INSTANCES, timeout: float = 5.0) -> bool:
    """A tool's read, complete, without ``key``: the owner read gone."""
    from . import live

    def gone() -> bool:
        p = live.list_presence(session, selector)
        return p.kind == live.COMPLETE and not any(key.endswith(f"/@zk/instance/{i['instance']}") for i in p.instances)

    return _wait(gone, timeout)


def _reads_held(session: Any, key: str, selector: str = INSTANCES, timeout: float = 5.0) -> bool:
    from . import live

    return _wait(lambda: any(key.endswith(f"/@zk/instance/{i['instance']}")
                             for i in live.list_presence(session, selector).instances), timeout)


def _s10_run(endpoint: str, tools: list[Any], address: str, *, round_trip: bool, stall_ms: float | None = 150.0,
             stall: bool = True, via: str | None = None, namespace: str | None = None, watch_ns: str | None = None,
             watch_selector: str = INSTANCES, history: bool = False) -> dict[str, Any]:
    """One run of §10 steps 1 to 3 and 7: the owner's token held and read; a
    fresh watch session (through a stallable link to ``via``, R1 by
    default) declares its subscriber, its link stalled from before the
    declaration until ``stall_ms`` later (None: until after the tool reads
    the token gone, step 2), and at once the round trip, or none; then the
    tools read the token (the seed), the owner withdraws it at once, and the
    tools read it gone. What the watch session heard, and the round trip."""
    from . import live
    from .budget_scenarios import Link

    out: dict[str, Any] = {}
    h = Holder(endpoint, address, namespace=namespace)
    link = Link(via or endpoint)
    w = None
    try:
        for t in tools:
            if not _reads_held(t, h.key):
                out["setup"] = "a tool never read the owner's token"
                return out
        w = _Watch(link.endpoint, namespace=watch_ns)
        ended: dict[str, float] = {}
        if stall:
            link.stall()
        if stall and stall_ms is not None:
            def end() -> None:
                ended["at"] = time.monotonic()
                link.flowing.set()

            threading.Timer(stall_ms / 1000.0, end).start()
        w.subscribe(watch_selector, history)
        if round_trip:
            rt = live.settle(w.session, 5.0, selector=watch_selector)
            out["rt"] = rt
            out["rt_end"] = rt.asked_ns / 1e9 + rt.elapsed_s
        out["seed"] = all(_reads_held(t, h.key, timeout=2.0) for t in tools)
        h.withdraw()
        out["gone"] = all(_reads_gone(t, h.key) for t in tools)
        if stall and stall_ms is None:
            ended["at"] = time.monotonic()
            link.flowing.set()
            if not round_trip:
                out["rt_after"] = live.settle(w.session, 5.0, selector=watch_selector)
        out["stall_end"] = ended.get("at")
        _wait(lambda: w.heard_gone(h.key), 1.5)
        out["heard"] = w.heard_gone(h.key)
        out["key"] = h.key
    finally:
        h.close()
        if w is not None:
            w.close()
        link.close()
    return out


def section10(report: Report) -> None:
    """§10 A round trip at a population, and the interests timeout (0.41)."""
    from . import live
    from .live_interop import _r1

    sec = f"§10 a round trip at a population ({P.owner} owner, {P.reader} reader)"
    r1, ep1, _ = _r1()
    r2, ep2 = _router(connect=ep1)
    tool = live.open_client(ep1)
    tool2 = live.open_client(ep2)
    crowd = [live.open_client(ep1), live.open_client(ep1)]
    held: list[Any] = []
    try:
        if not _wait(lambda: live.links_held(tool2) is True, 10.0):
            report.check(sec, "setup: the second tool reaches R2", False)
            return
        if P.reader == "zenctl":
            _section10_zenctl(report, sec, ep1, crowd, held)
            return
        _s10_steps_1_to_3(report, sec, ep1, ep2, tool, tool2)
        _s10_step_4(report, sec, ep1, tool, crowd, held)
        _s10_step_5(report, sec, ep1)
        for t in held:
            t.undeclare()
        held.clear()
        _s10_step_6(report, sec, ep1, shared=True)
        _s10_step_6(report, sec, ep1, shared=False)
        _s10_step_7(report, sec, ep1)
    finally:
        for t in held:
            try:
                t.undeclare()
            except Exception:  # noqa: BLE001
                pass
        for s in (tool, tool2, *crowd):
            live.close(s)
        r2.close()
        r1.close()


def _s10_steps_1_to_3(report: Report, sec: str, ep1: str, ep2: str, tool: Any, tool2: Any) -> None:
    def tally(runs: list[dict[str, Any]], want_heard: bool) -> tuple[int, int, list[str]]:
        good, waited, notes = 0, 0, []
        for r in runs:
            rt = r.get("rt")
            if rt is not None:
                still = r.get("stall_end") is not None and r["rt_end"] > r["stall_end"]
                waited += still and rt.finished and rt.errors == 0 and rt.tokens == 0
                notes.append(f"{rt.elapsed_s * 1000:.0f} ms")
            good += r.get("seed") and r.get("gone") and r.get("heard") is want_heard
        return good, waited, notes

    for label, via, tools in (("1.", None, [tool]), ("3. (through R2)", ep2, [tool2, tool])):
        runs = [_s10_run(ep1, tools, f"lab/s10a{n}", round_trip=True, via=via) for n in range(10)]
        good, waited, notes = tally(runs, True)
        report.check(sec, f"{label} the round trip, a GET of {INSTANCES.rsplit('/', 2)[0]}/settle made at once after a "
                          "subscriber without history on a link stalled 150 ms, is still waiting when the stall ends, "
                          "and ends with the final reply, no error reply and no token",
                     waited == 10, f"{waited} of 10; {', '.join(notes)}")
        report.check(sec, f"{label} in every run the watch session hears the token withdrawn, though its subscriber "
                          "has no history and the token was declared before it", good == 10, f"{good} of 10 runs")
        runs = [_s10_run(ep1, tools, f"lab/s10b{n}", round_trip=False, stall_ms=None, via=via) for n in range(3)]
        good, _, _ = tally(runs, False)
        rts = [r.get("rt_after") for r in runs]
        report.check(sec, f"{label.replace('1.', '2.')} without the round trip, the stall ended only after the tool "
                          "read the token gone: the watch session never hears it go; a round trip after says the "
                          "router read the subscriber", good == 3 and all(bool(x) for x in rts),
                     f"{good} of 3 never heard it; round trips after {[bool(x) for x in rts]}")


def _s10_step_4(report: Report, sec: str, ep1: str, tool: Any, crowd: list[Any], held: list[Any]) -> None:
    from . import live

    n = P.crowd
    half = n // 2
    t0 = time.monotonic()
    for i in range(n):
        held.append(crowd[i // half if i < 2 * half else 1].liveliness().declare_token(
            f"zk2/crowd/c{i}/@zk/instance/{i:016x}"))
    whole = _wait(lambda: len(live.list_presence(tool, "zk2/crowd/*/@zk/instance/*", 30.0).instances) == n, 120.0, 0.5)
    report.note(sec, f"4. the crowd: {n} instance tokens on two sessions, read whole by the tool: {whole}, "
                     f"{time.monotonic() - t0:.1f} s")
    w = _Watch(ep1)
    try:
        w.subscribe(INSTANCES, history=False)
        fin, toks, errs, secs = _round_trip_raw(w.session, live.settle_key(INSTANCES), 5.0)
        report.check(sec, f"4. beside the {n} tokens, a subscriber without history: the round trip carries no token "
                          "and ends within one round trip, under 1 s",
                     fin and toks == 0 and errs == 0 and secs < 1.0, f"{secs * 1000:.1f} ms, {toks} tokens, {errs} errors")
    finally:
        w.close()
    def with_history(direct: bool) -> tuple[bool, int | None, float, int, int]:
        w = _Watch(ep1, interests_timeout_s=60.0)

        def count() -> int:
            return len({k for _, kind, k in list(w.heard) if kind == "put" and k.startswith("zk2/crowd/")})

        try:
            at_end: dict[str, int] = {}
            w.subscribe(INSTANCES, history=True, direct=direct)
            fin, toks, errs, secs = _round_trip_raw(w.session, live.settle_key(INSTANCES), 5.0, direct=direct,
                                                    on_end=lambda: at_end.setdefault("n", count()))
            time.sleep(0.05)
            return fin and errs == 0, at_end.get("n"), secs, toks, count()
        finally:
            w.close()

    runs = [with_history(True) for _ in range(10)]
    report.check(sec, f"4. with history (its interests timeout 60 s): the round trip ends only after the history: "
                      f"by then the subscriber has heard all {n} tokens (callbacks on zenoh's thread, indirect=False), "
                      "in every run", all(ok and heard == n and toks == 0 for ok, heard, _, toks, _ in runs),
                 f"{[x[1] for x in runs]} heard by its end, {', '.join(f'{x[2] * 1000:.0f} ms' for x in runs)}")
    # The interests timeout met by a population, with no stall (core §8.1,
    # 0.41: "A reader MUST read a read whose time reached its session's
    # interests timeout as possibly incomplete"): a session at 300 ms reads
    # the crowd with a 5 s timeout; zk2py's own sessions run with 120 s.
    for its in (0.3, live.INTERESTS_TIMEOUT_S):
        s = live.open_client(ep1, interests_timeout_s=its)
        try:
            _wait(lambda: live.links_held(s) is True, 10.0)
            p = live.list_presence(s, "zk2/crowd/*/@zk/instance/*", 5.0)
        finally:
            live.close(s)
        reached = p.elapsed_s >= its
        report.check(sec, f"4. a read of the crowd (a 5 s timeout) on a session whose interests timeout is {its:g} s: "
                          "the reader's verdict follows how it ended: at the interests timeout, possibly incomplete; "
                          f"before it, complete with all {n}",
                     (p.kind == live.POSSIBLY_INCOMPLETE and p.at_interests_timeout and not p.errors) if reached
                     else (p.kind == live.COMPLETE and len(p.instances) == n),
                     f"{p.elapsed_s * 1000:.0f} ms, {len(p.instances)} of {n}, {len(p.errors)} error replies, "
                     f"{p.kind}")
    runs = [with_history(False) for _ in range(20)]
    short = [x[1] for x in runs if x[1] != n]
    late = sum(1 for x in runs if x[4] == n)
    report.note(sec, f"4. the same with zenoh-python's default callbacks, each on a thread of its own (indirect): the "
                     f"round trip's end reached the application before the subscriber's last history sample in "
                     f"{len(short)} of 20 runs ({short} heard by then); 50 ms after it, every token heard in {late} of "
                     f"20 (F-230)")


def _s10_step_5(report: Report, sec: str, ep1: str) -> None:
    from . import live
    from .budget_scenarios import Link

    link = Link(ep1)
    s = live.open_client(link.endpoint, interests_timeout_s=0.3)
    try:
        _wait(lambda: live.links_held(s) is True, 10.0)
        p = live.list_presence(s, INSTANCES, 2.0)
        name = (f"5. a session whose interests timeout is 300 ms, its link open: the read of {INSTANCES}, the crowd "
                f"of {P.crowd} under it, is complete")
        if P.crowd == 2000:
            report.check(sec, name, p.kind == live.COMPLETE, f"{p.elapsed_s * 1000:.1f} ms; {p.reading[:200]}")
        else:
            report.note(sec, f"{name}? measured beside the larger crowd (F-231): {p.kind}, "
                             f"{p.elapsed_s * 1000:.1f} ms, {p.count} tokens")
        link.stall()
        try:
            q = live.list_presence(s, INSTANCES, 2.0)
            rt = live.settle(s, 2.0, selector=INSTANCES)
        finally:
            link.flowing.set()
        report.check(sec, "5. stalled: zenoh ends the read at 300 ms with the final reply alone, no error reply",
                     not q.errors and 0.29 <= q.elapsed_s < 1.0 and q.count == 0, f"{q.elapsed_s * 1000:.1f} ms, "
                                                                                   f"{len(q.errors)} errors")
        report.check(sec, "5. zk2py's reader reads it possibly incomplete (at the interests timeout), never complete",
                     q.kind == live.POSSIBLY_INCOMPLETE and q.at_interests_timeout, q.reading)
        report.check(sec, "5. stalled: zenoh ends the round trip at 300 ms with the final reply alone, no error reply; "
                          "zk2py's reader says its round trip does not stand (not made, at the interests timeout)",
                     rt.finished and rt.errors == 0 and 0.29 <= rt.elapsed_s < 1.0 and not rt.stands and not rt.made
                     and rt.why == live.AT_INTERESTS_TIMEOUT, f"{rt.elapsed_s * 1000:.1f} ms, {rt.why}")
    finally:
        live.close(s)
        link.close()


def _s10_step_6(report: Report, sec: str, ep1: str, *, shared: bool) -> None:
    """§10 step 6: the reader under test, zk2py's follower, its watch
    session's interests timeout 300 ms, its link stalled while it starts,
    then cut and healed. ``shared``: its reading session on the same link
    (as §8's reader); otherwise straight to R1 (F-227)."""
    from . import live
    from . import presence as pr
    from .budget_scenarios import Link

    which = "its reading session on the same link" if shared else "its reading session straight to R1"
    h = Holder(ep1, "lab/s10f")
    link = Link(ep1)
    watch = live.open_client(link.endpoint, interests_timeout_s=0.3)
    reading = live.open_client(link.endpoint if shared else ep1)
    f = None
    try:
        _wait(lambda: live.links_held(watch) is True and live.links_held(reading) is True, 10.0)
        _reads_held(reading, h.key)
        link.stall()
        stall_ms = 800.0
        if shared:
            threading.Timer(stall_ms / 1000.0, link.flowing.set).start()
        f = pr.PresenceFollower(reading, watch, INSTANCES, timeout=2.0, round_trip_timeout=5.0)
        if not shared:
            time.sleep(max(0.0, stall_ms / 1000.0 - 0.35))
            link.flowing.set()
        first_rt = f.round_trips[0] if f.round_trips else None
        first = f.folds[0] if f.folds else None
        st_owner = f.address("lab", "s10f")
        st_absent = f.address("lab", "absent")
        report.check(sec, f"6. ({which}) the watch session's link stalled 0.8 s while the reader starts: its round "
                          "trip ends at the interests timeout, not made, and its seed is possibly incomplete, its "
                          "round trip not established",
                     first_rt is not None and not first_rt.stands and first_rt.why == live.AT_INTERESTS_TIMEOUT
                     and first == (live.COMPLETE, False),
                     f"round trip {first_rt and (first_rt.why[:40], round(first_rt.elapsed_s, 3))}, first fold {first}")
        report.check(sec, f"6. ({which}) the address it shows is up, one it does not is unobservable, never down",
                     st_owner[0] == pr.UP and st_absent == (pr.UNOBSERVABLE, pr.NO_ROUND_TRIP),
                     f"owner {st_owner}, absent {st_absent}")
        ok = _wait(lambda: f.address("lab", "absent")[0] == pr.DOWN, 10.0)
        whole = [i for i, x in enumerate(f.folds) if x == (live.COMPLETE, True)]
        stood = [rt for rt in f.round_trips if rt.stands]
        report.check(sec, f"6. ({which}) it reads again, making the round trip first, and once one stands a read is "
                          "complete again", ok and bool(whole) and bool(stood)
                     and stood[0].asked_ns < f.view.reads[whole[0]].asked_ns,
                     f"folds {f.folds}, round trips {[(bool(r), round(r.elapsed_s, 3)) for r in f.round_trips]}")
        n_folds, n_rt, n_moves = len(f.folds), len(f.round_trips), len(f.moves)
        states: list[tuple[str, str]] = []
        link.cut()
        _wait(lambda: len(f.moves) > n_moves and len(f.folds) > n_folds, 5.0)
        t_end = time.monotonic() + 1.5
        while time.monotonic() < t_end:
            states.append(f.address("lab", "s10f"))
            time.sleep(0.02)
        after_cut = f.folds[n_folds:]
        if shared:
            report.check(sec, f"6. ({which}) the link cut: the reader reads again at once, a read not made through a "
                              "session cut off, the owner unobservable, never down",
                         bool(after_cut) and after_cut[0][0] == live.NOT_MADE and pr.DOWN not in {s for s, _ in states}
                         and states[-1][0] == pr.UNOBSERVABLE,
                         f"moves {[k for _, k in f.moves[n_moves:]]}, folds {after_cut}, states {sorted(set(states))}")
        else:
            report.check(sec, f"6. ({which}) the watch link cut: the reader reads again at once; its read is made, "
                              "complete by its replies but no round trip stands, so it is possibly incomplete; the "
                              "owner it shows is up, never down (Expected 6's \"a read not made\" needs the reading "
                              "session cut too: F-227)",
                         bool(after_cut) and after_cut[0] == (live.COMPLETE, False)
                         and pr.DOWN not in {s for s, _ in states},
                         f"moves {[k for _, k in f.moves[n_moves:]]}, folds {after_cut}, states {sorted(set(states))}")
        time.sleep(1.5)
        after_cut = f.folds[n_folds:]
        report.check(sec, f"6. ({which}) while the watch session stays cut off, 3 s in all, the reader makes no read "
                          "beyond the one its move asked: no round trip can stand, so it waits for the transport to "
                          "open (review of PR #876)", len(after_cut) == 1 and f.waited >= 1,
                     f"{len(after_cut)} reads, {f.waited} attempts that waited")
        n_folds, n_moves = len(f.folds), len(f.moves)
        link.heal()
        opened = _wait(lambda: any(k == "open" for _, k in f.moves[n_moves:]), 20.0, 0.1)
        up = _wait(lambda: f.address("lab", "s10f")[0] == pr.UP and f.folds[-1] == (live.COMPLETE, True), 10.0)
        t_open = next((t for t, k in f.moves[n_moves:] if k == "open"), None)
        whole_after = [r for r, x in zip(f.view.reads[n_folds:], f.folds[n_folds:]) if x == (live.COMPLETE, True)]
        rt_after = [rt for rt in f.round_trips[n_rt:] if rt.stands and t_open is not None and rt.asked_ns > t_open]
        report.check(sec, f"6. ({which}) after the heal it makes the round trip again before the read that finds the "
                          "owner up", opened and up and bool(whole_after) and bool(rt_after)
                     and rt_after[0].asked_ns < whole_after[0].asked_ns,
                     f"moves {[k for _, k in f.moves]}, folds since {f.folds[n_folds:]}")
        made = [rt for rt in f.round_trips if rt.why != live.CUT_OFF]

        def how(r: Any) -> str:
            return "stood" if r else "at the interests timeout" if r.why == live.AT_INTERESTS_TIMEOUT else r.why[:30]

        report.note(sec, f"6. ({which}) round trips on the watch session: {len(made)} made "
                         f"({', '.join(how(r) for r in made)}), {len(f.round_trips) - len(made)} not made "
                         f"(cut off); reads {len(f.folds)}")
    finally:
        if f is not None:
            f.close()
        h.close()
        live.close(watch)
        live.close(reading)
        link.close()


def _s10_step_7(report: Report, sec: str, ep1: str) -> None:
    from . import live
    from . import presence as pr

    if P.owner == "rust":
        report.cannot(sec, "7. the owner example takes no namespace: step 7's owner is zk2py's only")
        return
    tool = live.open_client(ep1, namespace="ns")
    try:
        cases = (("in ns, without history", "ns", INSTANCES, False, False),
                 ("in ns, with history", "ns", INSTANCES, True, True),
                 ("in no namespace, without history, on the wire key", None, "ns/" + INSTANCES, False, True))
        for n, (label, wns, sel, history, want) in enumerate(cases):
            r = _s10_run(ep1, [tool], f"lab/s10n{n}", round_trip=True, stall=False,
                         namespace="ns", watch_ns=wns, watch_selector=sel, history=history)
            report.check(sec, f"7. a watch session {label}: " + ("hears the token go" if want else
                                                                 "never hears the token go: its router tells the "
                                                                 "withdrawal by the wire key, which it does not strip"),
                         r.get("seed") and r.get("gone") and r.get("heard") is want,
                         f"seed {r.get('seed')}, gone {r.get('gone')}, heard {r.get('heard')}")
        # the reader under test, its watch session in ns: history by default
        for history, want in ((None, True), (False, False)):
            h = Holder(ep1, "lab/s10nf", namespace="ns")
            watch = live.open_client(ep1, namespace="ns")
            f = pr.PresenceFollower(tool, watch, INSTANCES, history=history)
            try:
                _wait(lambda: f.address("lab", "s10nf")[0] == pr.UP, 5.0)
                h.withdraw()
                down = _wait(lambda: f.address("lab", "s10nf")[0] == pr.DOWN, 3.0)
                label = ("zk2py's follower, its watch session in ns: history (0.41's exception), it hears the owner "
                         "go and reads it down" if history is None else
                         "the control, a follower in ns without history: it never hears the owner go, and goes on "
                         "reading it up")
                report.check(sec, f"7. {label}", down is want and f.history is (history is None),
                             f"history {f.history}, {f.address('lab', 's10nf')}")
            finally:
                f.close()
                h.close()
                live.close(watch)
    finally:
        live.close(tool)


def _section10_zenctl(report: Report, sec: str, ep1: str, crowd: list[Any], held: list[Any]) -> None:
    """§10 with the reference's tool: what its command line lets it run.
    Steps 1 to 3 and 7's subscribers are zenoh's, measured in the zk2py
    reader's pairing; zenctl's own watch session is not observable from
    outside."""
    from . import live

    tool = live.open_client(ep1)
    h = Holder(ep1, "lab/s10z")
    try:
        n = P.crowd
        half = n // 2
        for i in range(n):
            held.append(crowd[i // half if i < 2 * half else 1].liveliness().declare_token(
                f"zk2/crowd/c{i}/@zk/instance/{i:016x}"))
        _wait(lambda: len(live.list_presence(tool, "zk2/crowd/*/@zk/instance/*", 30.0).instances) == n, 120.0, 0.5)
        if n > 2000:
            rc, doc, err, secs = _zenctl_list(ep1, system="crowd", timeout_s=2.0, wait=900.0)
            report.note(sec, f"4. zenctl's read of the {n} with --timeout 2: complete {(doc or {}).get('complete')}, "
                             f"{len(_rows(doc))} rows, {secs:.1f} s in all (its notes: "
                             f"{[x.get('text', '')[:80] for x in (doc or {}).get('notes', [])]})")
        timeout = 2.0 if n <= 2000 else 30.0
        rc, doc, err, secs = _zenctl_list(ep1, system="crowd", timeout_s=timeout, wait=900.0)
        report.check(sec, f"4. zenctl's read beside the crowd (--timeout {timeout:g}): complete, every one of the {n} "
                          "instance tokens", (doc or {}).get("complete") is True and len(_rows(doc)) == n,
                     f"rc {rc}, {secs:.1f} s in all, {len(_rows(doc))} rows; {err[:200]}")
        for t in held:
            t.undeclare()
        held.clear()
        cfg = _zenctl_config(300)
        link = DelayLink(ep1)
        try:
            rc, doc, err, secs = _zenctl_list(link.endpoint, system="lab", timeout_s=2.0, extra=["--zenoh-config", cfg])
            report.check(sec, "5. zenctl, its session's interests timeout 300 ms (--zenoh-config), the path short: "
                              "complete, the owner's instance token held",
                         (doc or {}).get("complete") is True and h.instance in _rows(doc),
                         f"rc {rc}, {secs:.1f} s; {_brief(doc)}; {err}")
            link.delay_s = 0.5
            rc, doc, err, secs = _zenctl_list(link.endpoint, system="lab", timeout_s=2.0, extra=["--zenoh-config", cfg])
            report.check(sec, "5. the same, the path 0.5 s each way, so the read runs past 300 ms: never complete "
                              "(service list is a listing: exit 0, \"complete\": false)",
                         (doc or {}).get("complete") is not True and not _rows(doc),
                         f"rc {rc}, {secs:.1f} s; {_brief(doc)}; {err}")
            rc, doc, err, secs = _zenctl_list(link.endpoint, system="lab", timeout_s=5.0)
            report.check(sec, "5. (the SHOULD) zenctl on its own interests timeout, the same long path: the read is "
                              "complete, the owner's token held", (doc or {}).get("complete") is True
                         and h.instance in _rows(doc), f"rc {rc}, {secs:.1f} s; {_brief(doc)}; {err}")
        finally:
            link.close()
        # step 6: the watchdog, its sessions behind a long path while it starts
        link = DelayLink(ep1, 0.4)
        wd = None
        rule = "instance-gone lab/s10z"
        try:
            wd = Watchdog(link.endpoint, [rule], extra=["--zenoh-config", cfg])
            time.sleep(4.0)
            early = [to for _, to, _ in wd.transitions(rule)]
            link.delay_s = 0.0
            ok = _wait(lambda: wd.state(rule) == "ok", 20.0, 0.1)
            report.check(sec, "6. zenctl watchdog, its interests timeout 300 ms, behind a 0.4 s path while it starts: "
                              "never firing while the owner holds its token; ok once the path is short",
                         "firing" not in early and ok, f"while long {early}, then {wd.transitions(rule)[len(early):]}")
            n0 = len(wd.transitions(rule))
            link.cut()
            time.sleep(3.0)
            during = [to for _, to, _ in wd.transitions(rule)[n0:]]
            link.heal()
            back = _wait(lambda: wd.state(rule) == "ok", 30.0, 0.1)
            report.check(sec, "6. its link cut, then healed: never firing (the owner never left), ok again after the "
                              "heal", "firing" not in during and back,
                         f"while cut {during}, after {wd.transitions(rule)[n0 + len(during):]}")
        finally:
            if wd is not None:
                wd.close()
            link.close()
            os.unlink(cfg)
        # step 7: a namespace
        if P.owner == "rust":
            report.cannot(sec, "7. the owner example takes no namespace")
            return
        hn = Holder(ep1, "lab/s10nz", namespace="ns")
        wd = Watchdog(ep1, ["instance-gone lab/s10nz"], namespace="ns")
        try:
            ok = _wait(lambda: wd.state("instance-gone lab/s10nz") == "ok", 15.0, 0.1)
            hn.withdraw()
            gone = _wait(lambda: wd.state("instance-gone lab/s10nz") == "firing", 15.0, 0.1)
            report.check(sec, "7. zenctl watchdog --namespace ns: the owner in ns read up, then gone once it stops: it "
                              "hears the withdrawal (history in ns, or a watch from no namespace)", ok and gone,
                         str(wd.transitions("instance-gone lab/s10nz")))
        finally:
            wd.close()
            hn.close()
    finally:
        h.close()
        live.close(tool)


SECTIONS = {"1": section1, "2": section2, "3": section3, "4": section4, "5": section5, "6": section6,
            "7": section7, "8": section8, "9": section9, "10": section10}


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="python -m zk2py.presence_scenarios", description=__doc__.split("\n")[0])
    ap.add_argument("--only", action="append", choices=sorted(SECTIONS, key=int), help="run only these sections")
    ap.add_argument("--owner", choices=("zk2py", "rust"), default="zk2py", help="who holds the tokens")
    ap.add_argument("--reader", choices=("zk2py", "zenctl"), default="zk2py", help="the reader under test")
    ap.add_argument("--owner-bin", type=Path, default=Path(os.environ.get("ZK2PY_OWNER", P.owner_bin)),
                    help="the reference's owner example")
    ap.add_argument("--zenctl", type=Path, default=Path(os.environ.get("ZK2PY_ZENCTL", P.zenctl_bin)),
                    help="the reference's zenctl")
    ap.add_argument("--crowd", type=int, default=2000, help="§10's crowd, 2,000 instance tokens by default")
    ap.add_argument("--tokenless", type=int, default=100, help="§5's services, 100 by default")
    args = ap.parse_args(argv)
    P.owner, P.reader, P.owner_bin, P.zenctl_bin, P.crowd = args.owner, args.reader, args.owner_bin, args.zenctl, \
        args.crowd
    P.tokenless = args.tokenless
    report = Report()
    for name, fn in SECTIONS.items():
        if args.only and name not in args.only:
            continue
        try:
            fn(report)
        except NotPairable as e:
            report.cannot(f"§{name}", str(e))
        except Exception as e:  # noqa: BLE001 - a section that crashes is a failure, reported
            import traceback

            traceback.print_exc()
            report.check(f"§{name}", "the section ran", False, repr(e))
    passed = sum(1 for r in report.rows if r[2])
    failed = len(report.rows) - passed
    print(f"presence scenarios §1–§10 (core 0.41), {P.owner} owner, {P.reader} reader: {passed} passed, {failed} "
          f"failed, {len(report.not_run)} not run")
    for sec, name, ok, detail in report.rows:
        if not ok:
            print(f"FAIL [{sec}] {name}: {detail}")
    for sec, why in report.not_run:
        print(f"NOT RUN [{sec}] {why}")
    return 1 if failed else 0


if __name__ == "__main__":
    rc = main()
    sys.stdout.flush()
    # zenoh-python can hang at interpreter exit with sessions open
    os._exit(rc)

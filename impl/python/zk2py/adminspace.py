"""A router's admin space beside a router that is declaring (observer §4.3 and core
Appendix B, 0.48): the processes ``scenarios/state.md`` §12 runs, and a
measurement of the hazard itself, never part of a suite.

``python -m zk2py.adminspace router …`` is a zenoh-python router in a
process of its own, its admin space on, read permitted, write refused. It
listens on port 0 and learns its endpoint from its own root document
(``@/<zid>/router``'s ``locators``, which 0.48 says is answered without the
lock held across a reply). An in-process router loads no plugin, so the
router's own session plays the storage manager: a queryable on
``@/<zid>/router/status/plugins/storage_manager/storages/<name>`` per
``--storage NAME=KEY_EXPR``, answering the storage's configuration with its
``key_expr``. Its replies carry the router's own replier id, so "Who
answered" (observer §4.2, 0.12) counts them. It prints one JSON line, ``{"endpoint",
"zid"}``, then answers each stdin line (``drop NAME``, ``add NAME
KEY_EXPR``) with ``ok …``, and closes at the end of its input.

``python -m zk2py.adminspace load …`` is the two other clients of §12's
setup in one process: one holds ``--records`` subscribers and as many
queryables on ``load/<i>/**``, and, with ``--churn``, a second declares and
undeclares a subscriber and a liveliness token, flat out, on a thread of its
own. It prints ``ready``, and at the end of its input ``churned <n>``.

``python -m zk2py.adminspace measure …`` asks whether a GET of a selector
that reaches the declaration records freezes a zenoh-python 1.10.1 router
beside a client declaring flat out (Appendix B, 0.48). Each trial has a
router process of its own, which it kills, frozen or not; every wait is
bounded. It is a measurement of zenoh, and never runs in
``just py-conformance``.

The ``acl-…`` cells (core §11.1, 0.50) run the router under
``default_permission: deny`` with the block zk2py's generator compiles for
§4's tool and service principal, 0.15's admin read (``acl-0.15``) or
0.50's (``acl-0.50``), and a principal of the measurement's own, ``bench``,
allowed everything on ``load/**``, ``churn/**`` and ``probe/**``, which the
load, the churn and the probe log in as. The reader logs in as the tool,
and the queryable on ``@/**`` is the router's own session's
(``router --record``), since no principal may declare one.

Exit 0 when the processes ran, 1 when a measurement could not be made: a
process that did not come up, or a trial that asked nothing.
"""

from __future__ import annotations

import argparse
import json
import os
import queue
import signal
import subprocess
import sys
import threading
import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

HERE = Path(__file__).resolve().parent
STORAGES = "status/plugins/storage_manager/storages"


# -- the router process -------------------------------------------------------

def node_config(mode: str = "router", block: dict[str, Any] | None = None, users: str | None = None):
    """A node listening on port 0, multicast scouting off, timestamping on,
    its admin space on, read permitted, write refused; with ``block``, under
    that ``access_control`` block, and with ``users``, a usrpwd dictionary
    file, authenticating every session."""
    import zenoh

    conf = zenoh.Config()
    conf.insert_json5("mode", json.dumps(mode))
    conf.insert_json5("listen/endpoints", json.dumps(["tcp/127.0.0.1:0"]))
    conf.insert_json5("scouting/multicast/enabled", "false")
    conf.insert_json5("timestamping/enabled", "true")
    conf.insert_json5("adminspace", json.dumps({"enabled": True, "permissions": {"read": True, "write": False}}))
    if users is not None:
        conf.insert_json5("transport/auth/usrpwd", json.dumps({"dictionary_file": users}))
    if block is not None:
        conf.insert_json5("access_control", json.dumps(block))
    return conf


def own_endpoint(session: Any, mode: str = "router") -> str | None:
    """The loopback TCP endpoint a node listening on port 0 got, from its
    own root document (``@/<zid>/<whatami>``'s ``locators``), which 0.48
    says is answered without the lock held across a reply. The node's own
    session asks it, so no access control block refuses it."""
    import zenoh

    got: list[Any] = []
    done = threading.Event()
    session.get(f"@/{session.zid()}/{mode}", zenoh.handlers.Callback(got.append, done.set), timeout=2.0)
    done.wait(5.0)
    doc = next((json.loads(x.ok.payload.to_bytes()) for x in got if x.ok is not None), {})
    return next((loc for loc in doc.get("locators", []) if loc.startswith("tcp/127.0.0.1:")), None)


def router_main(args: argparse.Namespace) -> None:
    import zenoh

    block = json.loads(Path(args.acl).read_text()) if args.acl else None
    r = zenoh.open(node_config(args.mode, block, args.users))
    zid = str(r.zid())
    endpoint = own_endpoint(r, args.mode)
    held: dict[str, Any] = {}
    recorder = Recorder(r) if args.record else None

    def add(name: str, key_expr: str) -> None:
        key = f"@/{zid}/{args.mode}/{STORAGES}/{name}"
        body = json.dumps({"key_expr": key_expr, "volume": "memory"})
        held[name] = r.declare_queryable(key, zenoh.handlers.Callback(
            lambda q: q.reply(key, body, encoding="application/json")))

    for spec in args.storage or []:
        name, _, ke = spec.partition("=")
        add(name, ke)
    print(json.dumps({"endpoint": endpoint, "zid": zid}), flush=True)
    for line in sys.stdin:
        words = line.split()
        if words[:1] == ["drop"] and len(words) == 2 and words[1] in held:
            held.pop(words[1]).undeclare()
            print(f"ok drop {words[1]}", flush=True)
        elif words[:1] == ["add"] and len(words) == 3:
            add(words[1], words[2])
            print(f"ok add {words[1]}", flush=True)
        elif words == ["heard"] and recorder is not None:
            print(json.dumps(recorder.take()), flush=True)
        else:
            print(f"refused {line.strip()}", flush=True)
    for q in held.values():
        q.undeclare()
    r.close()


# -- the load process ---------------------------------------------------------

def _client(endpoint: str, auth: tuple[str, str] | None = None):
    import zenoh

    conf = zenoh.Config()
    conf.insert_json5("mode", json.dumps("client"))
    conf.insert_json5("connect/endpoints", json.dumps([endpoint]))
    conf.insert_json5("scouting/multicast/enabled", "false")
    if auth is not None:
        conf.insert_json5("transport/auth/usrpwd", json.dumps({"user": auth[0], "password": auth[1]}))
    return zenoh.open(conf)


def _auth(text: str | None) -> tuple[str, str] | None:
    if not text:
        return None
    user, _, password = text.partition(":")
    return user, password


def load_main(args: argparse.Namespace) -> None:
    import zenoh

    auth = _auth(args.auth)
    holder = _client(args.connect, auth)
    held = []
    for i in range(args.records):
        held.append(holder.declare_subscriber(f"load/{i}/**", zenoh.handlers.Callback(lambda s: None)))
        held.append(holder.declare_queryable(f"load/{i}/**", zenoh.handlers.Callback(lambda q: None)))
    churned = [0]
    stop = threading.Event()
    churner = _client(args.connect, auth) if args.churn else None

    def churn() -> None:
        assert churner is not None
        while not stop.is_set():
            s = churner.declare_subscriber("churn/sub/**", zenoh.handlers.Callback(lambda x: None))
            t = churner.liveliness().declare_token(f"churn/token/{churned[0] % 7}")
            s.undeclare()
            t.undeclare()
            churned[0] += 1

    thread = threading.Thread(target=churn, daemon=True) if churner is not None else None
    if thread is not None:
        thread.start()
    print("ready", flush=True)
    sys.stdin.read()
    stop.set()
    print(f"churned {churned[0]}", flush=True)
    os._exit(0)  # a churner stuck on a frozen router never returns: the process ends anyway


# -- the parent's side ----------------------------------------------------------

def _python() -> str:
    return sys.executable


def _env() -> dict[str, str]:
    env = dict(os.environ)
    env["PYTHONPATH"] = str(HERE.parent) + (os.pathsep + env["PYTHONPATH"] if env.get("PYTHONPATH") else "")
    return env


class _Lines:
    """A child's stdout, read on a thread of its own, so every wait for a
    line is bounded."""

    def __init__(self, proc: subprocess.Popen) -> None:
        self.q: queue.Queue = queue.Queue()
        threading.Thread(target=self._pump, args=(proc,), daemon=True).start()

    def _pump(self, proc: subprocess.Popen) -> None:
        assert proc.stdout is not None
        for line in proc.stdout:
            self.q.put(line.rstrip("\n"))
        self.q.put(None)

    def next(self, timeout: float) -> str | None:
        try:
            return self.q.get(timeout=timeout)
        except queue.Empty:
            return None


class Child:
    """A helper process, killed whatever it is doing at :meth:`kill`."""

    def __init__(self, argv: list[str]) -> None:
        self.proc = subprocess.Popen([_python(), "-m", "zk2py.adminspace", *argv], stdin=subprocess.PIPE,
                                     stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True, env=_env(),
                                     start_new_session=True)
        self.lines = _Lines(self.proc)

    def send(self, line: str) -> None:
        assert self.proc.stdin is not None
        self.proc.stdin.write(line + "\n")
        self.proc.stdin.flush()

    def end_input(self, timeout: float = 5.0) -> list[str]:
        """Close its input and read what it says until it ends, bounded."""
        out: list[str] = []
        try:
            assert self.proc.stdin is not None
            self.proc.stdin.close()
        except OSError:
            pass
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            line = self.lines.next(max(0.0, deadline - time.monotonic()))
            if line is None:
                break
            out.append(line)
        return out

    def kill(self) -> None:
        if self.proc.poll() is None:
            try:
                os.killpg(self.proc.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
        try:
            self.proc.wait(timeout=5.0)
        except subprocess.TimeoutExpired:
            pass


class RouterProcess(Child):
    """``router``: its endpoint and zid, read from its first line. ``mode``
    ``peer`` makes it a peer, its admin space under ``@/<zid>/peer``, which
    clients may join (the measurement's node that is no router)."""

    def __init__(self, storages: dict[str, str] | None = None, timeout: float = 15.0, mode: str = "router",
                 acl: str | None = None, users: str | None = None, record: bool = False) -> None:
        argv = ["router", "--mode", mode]
        for name, ke in (storages or {}).items():
            argv += ["--storage", f"{name}={ke}"]
        if acl is not None:
            argv += ["--acl", acl]
        if users is not None:
            argv += ["--users", users]
        if record:
            argv += ["--record"]
        super().__init__(argv)
        first = self.lines.next(timeout)
        if first is None:
            self.kill()
            raise RuntimeError("the router process said nothing")
        doc = json.loads(first)
        if not doc.get("endpoint"):
            self.kill()
            raise RuntimeError(f"the router process found no endpoint of its own: {first}")
        self.endpoint: str = doc["endpoint"]
        self.zid: str = doc["zid"]

    def command(self, line: str, timeout: float = 5.0) -> str | None:
        self.send(line)
        return self.lines.next(timeout)

    def close(self) -> None:
        self.end_input()
        self.kill()


class LoadProcess(Child):
    """``load``: §12's other client and its third, ready once the records are
    declared."""

    def __init__(self, endpoint: str, records: int, churn: bool, timeout: float = 30.0,
                 auth: tuple[str, str] | None = None) -> None:
        super().__init__(["load", "--connect", endpoint, "--records", str(records)] + (["--churn"] if churn else [])
                         + (["--auth", f"{auth[0]}:{auth[1]}"] if auth else []))
        if self.lines.next(timeout) != "ready":
            self.kill()
            raise RuntimeError("the load process did not come up")
        self.churned: int | None = None

    def close(self) -> None:
        for line in self.end_input():
            if line.startswith("churned "):
                self.churned = int(line.split()[1])
        self.kill()


class Recorder:
    """§12's "session of the test's own" holding a queryable on ``@/**``
    that answers nothing and records the selector of every GET it receives.
    It is declared not complete: a complete queryable on ``@/**`` includes
    ``@/*/router``, and a ``BestMatching`` GET would go to it alone."""

    def __init__(self, session: Any) -> None:
        import zenoh

        self.heard: list[str] = []
        self._lock = threading.Lock()
        self.q = session.declare_queryable("@/**", zenoh.handlers.Callback(self._hear), complete=False)

    def _hear(self, query: Any) -> None:
        with self._lock:
            self.heard.append(str(query.selector))

    def take(self) -> list[str]:
        with self._lock:
            out, self.heard = self.heard, []
        return out

    def close(self) -> None:
        self.q.undeclare()


def close_bounded(*sessions: Any, timeout: float = 5.0) -> bool:
    """Close sessions on a thread of their own, waiting at most ``timeout``:
    a session whose router froze may not close at once."""
    t = threading.Thread(target=lambda: [s.close() for s in sessions], daemon=True)
    t.start()
    t.join(timeout)
    return not t.is_alive()


# -- whether the router still routes -------------------------------------------

@dataclass
class Probe:
    """A put that crosses the router, and a GET it routes to a queryable,
    each within ``within`` seconds: two clients of the router. It is ready
    once a put and a GET have crossed, polled for at most ``ready_s``; a
    probe that cannot come up closes what it opened and raises."""

    endpoint: str
    within: float = 2.0
    ready_s: float = 10.0
    n: int = 0
    sessions: list[Any] = field(default_factory=list)
    auth: tuple[str, str] | None = None

    def __post_init__(self) -> None:
        import zenoh

        self.arrived: dict[int, float] = {}
        self._cv = threading.Condition()
        try:
            self.pub_s = _client(self.endpoint, self.auth)
            self.sessions.append(self.pub_s)
            self.sub_s = _client(self.endpoint, self.auth)
            self.sessions.append(self.sub_s)
            self.sub = self.sub_s.declare_subscriber("probe/put/*", zenoh.handlers.Callback(self._sample))
            self.qbl = self.sub_s.declare_queryable("probe/get", zenoh.handlers.Callback(
                lambda q: q.reply("probe/get", b"here")))
            deadline = time.monotonic() + self.ready_s
            put_ok = get_ok = False
            while not (put_ok and get_ok):
                left = deadline - time.monotonic()
                if left <= 0:
                    raise RuntimeError(f"the probe did not come up within {self.ready_s:g} s "
                                       f"(a put crossed: {put_ok}; a GET was answered: {get_ok})")
                wait = min(0.2, left)
                put_ok = put_ok or self._put(wait) is not None
                get_ok = get_ok or self._get(wait) is not None
        except BaseException:
            close_bounded(*self.sessions)
            self.sessions.clear()
            raise

    def _sample(self, s: Any) -> None:
        with self._cv:
            self.arrived[int(str(s.key_expr).rsplit("/", 1)[1])] = time.monotonic()
            self._cv.notify_all()

    def put(self) -> float | None:
        """The seconds a put took to arrive, or None past ``within``."""
        return self._put(self.within)

    def get(self) -> float | None:
        """The seconds the GET's reply took, or None past ``within``."""
        return self._get(self.within)

    def _put(self, within: float) -> float | None:
        self.n += 1
        n = self.n
        t0 = time.monotonic()
        self.pub_s.put(f"probe/put/{n}", b"x")
        with self._cv:
            self._cv.wait_for(lambda: n in self.arrived, timeout=within)
            at = self.arrived.get(n)
        return None if at is None else at - t0

    def _get(self, within: float) -> float | None:
        import zenoh

        got = threading.Event()
        t0 = time.monotonic()
        at: list[float] = []

        def cb(r: Any) -> None:
            if r.ok is not None and not at:
                at.append(time.monotonic())
                got.set()

        self.pub_s.get("probe/get", zenoh.handlers.Callback(cb), target=zenoh.QueryTarget.ALL, timeout=within)
        got.wait(within)
        return at[0] - t0 if at else None

    def routes(self) -> tuple[float | None, float | None]:
        return self.put(), self.get()


# -- the measurement -------------------------------------------------------------

def _ask(session: Any, selector: str, timeout: float) -> dict[str, Any]:
    """One GET, target All, consolidation None: its value replies, its error
    replies, whether it ended within ``timeout`` plus 2 s, and when."""
    import zenoh

    ok: list[str] = []
    errs: list[str] = []
    done = threading.Event()
    t0 = time.monotonic()

    def cb(r: Any) -> None:
        if r.ok is not None:
            ok.append(str(r.ok.key_expr))
        else:
            errs.append(r.err.payload.to_string())

    session.get(selector, zenoh.handlers.Callback(cb, done.set), target=zenoh.QueryTarget.ALL,
                consolidation=zenoh.ConsolidationMode.NONE, timeout=timeout)
    ended = done.wait(timeout + 2.0)
    return {"replies": len(ok), "errors": errs, "ended": ended, "s": round(time.monotonic() - t0, 3)}


#: the measurement's own principal under access control, the tool and the
#: service principal. The passwords are fixed and test-only: every router
#: here listens on loopback alone (port 0), and reads them from a usrpwd
#: dictionary written to a temporary directory of its own (:class:`_AclFiles`),
#: removed once the trial ends.
BENCH = ("bench", "pw-bench")
TOOL = ("tool", "pw-tool")
SERVICE = ("svc", "pw-svc")


def measure_block(admin_read: list[str], bench_recorder: bool = False) -> dict[str, Any]:
    """The ``acl-…`` cells' block: zk2py's generator over security.md §4's
    two principals, a tool with ``admin_read`` that inspects every service
    and a service principal that does neither, under ``deny``, and
    ``bench``, allowed every message on both flows on ``load/**``,
    ``churn/**`` and ``probe/**``. 0.15's admin read is compiled on
    purpose (``hazard_ok``): it is what is measured. ``bench_recorder``
    also lets ``bench`` hold the queryable on ``@/**`` as a client, which
    §11.1 lets no principal of a deployment do."""
    from . import acl

    principals = [acl.Principal("tool", TOOL[0], inspects=["*/*"], admin_read=True),
                  acl.Principal("svc", SERVICE[0], service="h1/tc")]
    block = acl.generate(principals, "deny", admin_read=admin_read, hazard_ok=True).block
    block["rules"].append({"id": "bench-all", "messages": list(acl.MESSAGES), "flows": [acl.INGRESS, acl.EGRESS],
                           "permission": "allow", "key_exprs": ["load/**", "churn/**", "probe/**"]})
    names = ["bench-all"]
    if bench_recorder:
        block["rules"] += [
            {"id": "bench-recorder-in", "messages": ["declare_queryable", "reply"], "flows": [acl.INGRESS],
             "permission": "allow", "key_exprs": ["@/**"]},
            {"id": "bench-recorder-out", "messages": ["query"], "flows": [acl.EGRESS], "permission": "allow",
             "key_exprs": ["@/**"]}]
        names += ["bench-recorder-in", "bench-recorder-out"]
    block["subjects"].append({"id": "bench", "usernames": [BENCH[0]]})
    block["policies"].append({"id": "bench-policy", "rules": names, "subjects": ["bench"]})
    return block


class _AclFiles:
    """The block and the usrpwd dictionary, written to a directory of their
    own, removed at :meth:`close`."""

    def __init__(self, block: dict[str, Any]) -> None:
        import tempfile

        self.dir = tempfile.mkdtemp(prefix="zk2py-adminspace-")
        self.acl = os.path.join(self.dir, "acl.json")
        self.users = os.path.join(self.dir, "users.txt")
        Path(self.acl).write_text(json.dumps(block))
        Path(self.users).write_text("".join(f"{u}:{p}\n" for u, p in (BENCH, TOOL, SERVICE)))

    def close(self) -> None:
        import shutil

        shutil.rmtree(self.dir, ignore_errors=True)


def trial(selectors: list[str], records: int, churn: bool, timeout: float, settle: float,
          mode: str = "router", asks: int = 20, admin_read: list[str] | None = None,
          own_recorder: bool = False, client_recorder: bool = False) -> dict[str, Any]:
    """One router process (``mode`` ``peer``: a peer), the load beside it, a
    queryable on ``@/**`` recording what it hears, as §12's does, and a
    reader asking ``selectors`` in turn, up to ``asks`` times, a put across
    the node after each, stopping at the first that does not cross within
    2 s (``froze_at``); then whether the node routes: a put and a GET within
    2 s right after, and again ``settle`` seconds later, with the reader's
    links then. Frozen means neither probe passed at either time. The
    node's process is killed at the end, frozen or not.

    With ``admin_read``, the router runs :func:`measure_block` with that
    admin read: the reader logs in as the tool, the load and the probe as
    ``bench``, and the recorder is the router's own session's, read with
    ``heard``. ``own_recorder`` puts it there without access control too
    (F-254's queryable was a client's), and ``client_recorder`` puts it on
    a client logged in as ``bench`` under access control."""
    own_recorder = own_recorder or (admin_read is not None and not client_recorder)
    out: dict[str, Any] = {"mode": mode, "selectors": selectors, "records": records, "churn": churn,
                           "admin_read": admin_read, "recorder": "the router's own session" if own_recorder
                           else "a client"}
    files = _AclFiles(measure_block(admin_read, client_recorder)) if admin_read is not None else None
    bench = BENCH if files else None
    router = RouterProcess(mode=mode, acl=files.acl if files else None, users=files.users if files else None,
                           record=own_recorder)
    load = None
    sessions: list[Any] = []
    try:
        load = LoadProcess(router.endpoint, records, churn, auth=bench)
        reader = _client(router.endpoint, TOOL if files else None)
        sessions.append(reader)
        if not own_recorder:
            rec_s = _client(router.endpoint, bench)
            sessions.append(rec_s)
            recorder: Any = Recorder(rec_s)
        else:
            recorder = _RouterRecorder(router)
        probe = Probe(router.endpoint, auth=bench)
        sessions += probe.sessions
        out["before"] = probe.routes()
        time.sleep(0.5)  # the churn runs
        recorder.take()
        out["asks"] = []
        out["froze_at"] = None
        n = 0  # rounds run: none when asks is 0, a trial that measures nothing
        for n in range(1, asks + 1):
            for sel in selectors:
                out["asks"].append(_ask(reader, sel, timeout))
            if probe.put() is None:
                out["froze_at"] = n  # the round of reads after which a put no longer crossed
                break
        out["rounds"] = n
        out["after"] = probe.routes()
        time.sleep(settle)
        out["later"] = probe.routes()
        out["heard"] = recorder.take()
        out["reader_links"] = len(list(reader.info.links()))
        out["frozen"] = all(x is None for x in out["after"] + out["later"])
        out["slowed"] = not out["frozen"] and any(x is None for x in out["after"] + out["later"])
    finally:
        if load is not None:
            load.close()
            out["churned"] = load.churned
        router.kill()  # killed first: a client of a frozen router closes once its link is gone
        out["closed"] = close_bounded(*sessions)
        if files is not None:
            files.close()
    return out


class _RouterRecorder:
    """The router process's own queryable on ``@/**`` (``router --record``),
    read with ``heard``; nothing, if the process does not say within 5 s."""

    def __init__(self, router: RouterProcess) -> None:
        self.router = router

    def take(self) -> list[str]:
        line = self.router.command("heard")
        try:
            return list(json.loads(line)) if line else []
        except ValueError:
            return []


#: name -> (the selectors the reader asks, a client declaring flat out
#: beside it, the node's mode[, the tool's admin read under access control])
CELLS: dict[str, tuple] = {
    # Appendix B's fact: the queryable records, read beside the churn.
    "records+churn": (["@/*/router/queryable/**"], True, "router"),
    # the control without a writer (the CHANGELOG's "0 of 28 without a writer")
    "records": (["@/*/router/queryable/**"], False, "router"),
    # 0.11's second selector beside the churn (the CHANGELOG's 10 of 10)
    "0.11+churn": (["@/*/router/**/storage_manager/storages/**"], True, "router"),
    # 0.48's two selectors beside the churn (the CHANGELOG's 0 of 5)
    "0.48+churn": (["@/*/router", "@/*/router/status/plugins/storage_manager/storages/**"], True, "router"),
    # Appendix B states the fact of a node: a peer's queryable records, read
    # beside the churn of its clients, which observer §4.3's MUST NOT does not name
    "peer-records+churn": (["@/*/peer/queryable/**"], True, "peer"),
}
#: core §11.1 (0.50): the same GETs from a tool principal under deny, with
#: 0.15's admin read and with 0.50's, beside the churn. ``@/**`` is
#: included by neither grant; ``reads`` are the tools' three reads.
_HAZARDOUS = {"records": ["@/*/router/queryable/**"], "router**": ["@/*/router/**"],
              "0.11": ["@/*/router/**/storage_manager/storages/**"], "all": ["@/**"]}
for _grant, _keys in (("0.15", ["@/*/router", "@/*/router/**"]),
                      ("0.50", ["@/*/router", "@/*/router/status/plugins/storage_manager/storages/**", "@/*/*"])):
    for _name, _sels in _HAZARDOUS.items():
        CELLS[f"acl-{_grant}:{_name}+churn"] = (_sels, True, "router", _keys)
    CELLS[f"acl-{_grant}:reads+churn"] = (["@/*/router", "@/*/router/status/plugins/storage_manager/storages/**",
                                           "@/*/*"], True, "router", _keys)


def measure_main(args: argparse.Namespace) -> int:
    cells = args.cell or list(CELLS)
    summary: dict[str, list[bool]] = {}
    for cell in cells:
        selectors, churn, mode, *grant = CELLS[cell]
        for i in range(args.trials):
            try:
                t = trial(selectors, args.records, churn, args.timeout, args.settle, mode, args.asks,
                          grant[0] if grant else None, args.own_recorder, args.client_recorder)
            except RuntimeError as e:
                print(json.dumps({"cell": cell, "trial": i, "could not run": str(e)}), flush=True)
                return 1
            asks = t.pop("asks", [])
            heard = t.pop("heard", [])
            # F-254, F-257: whether the queryable on @/** heard every GET
            # asked, the last one, which froze the node in a frozen trial,
            # included. A trial that asked nothing has no answer (None), and
            # is reported apart, never as heard.
            heard_all = len(heard) >= len(asks) if asks else None
            summary.setdefault(cell, []).append((t["frozen"], heard_all, len(asks)))
            t["asks"] = {"n": len(asks), "replies": [a["replies"] for a in asks][-3:],
                         "answered": sum(1 for a in asks if a["replies"]),
                         "errors": sorted({e for a in asks for e in a["errors"]}),
                         "slowest_s": max((a["s"] for a in asks), default=None)}
            t["heard"] = {"n": len(heard), "selectors": sorted(set(heard)), "every_get": heard_all}
            print(json.dumps({"cell": cell, "trial": i, **t}), flush=True)
    unasked = 0
    for cell, rows in summary.items():
        asked = [r for r in rows if r[2]]
        frozen = [r for r in asked if r[0]]
        line = f"{cell}: froze in {len(frozen)} of {len(asked)} trials, {sum(r[2] for r in asked)} GETs; "
        line += (f"the queryable on @/** heard the freezing GET in {sum(1 for r in frozen if r[1])} of {len(frozen)}"
                 if frozen else "nothing froze, so no freezing GET to hear")
        if len(asked) < len(rows):
            line += f"; {len(rows) - len(asked)} trial(s) asked nothing, and measured nothing"
            unasked += len(rows) - len(asked)
        print(line, flush=True)
    return 1 if unasked else 0


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="python -m zk2py.adminspace", description=__doc__.split("\n")[0])
    sub = ap.add_subparsers(dest="cmd", required=True)
    r = sub.add_parser("router", help="a router in a process of its own, its admin space on, read-only")
    r.add_argument("--mode", choices=("router", "peer"), default="router")
    r.add_argument("--storage", action="append", metavar="NAME=KEY_EXPR", help="a storage document to play")
    r.add_argument("--acl", help="a JSON file holding an access_control block")
    r.add_argument("--users", help="a usrpwd dictionary file")
    r.add_argument("--record", action="store_true", help="a queryable on @/** on the router's own session")
    lo = sub.add_parser("load", help="the records' holder and, with --churn, a client declaring flat out")
    lo.add_argument("--connect", required=True)
    lo.add_argument("--auth", metavar="USER:PASSWORD")
    lo.add_argument("--records", type=int, default=200)
    lo.add_argument("--churn", action="store_true")
    m = sub.add_parser("measure", help="does a GET of the declaration records freeze the router? (never a suite)")
    m.add_argument("--cell", action="append", choices=sorted(CELLS))
    m.add_argument("--trials", type=int, default=10)
    m.add_argument("--records", type=int, default=200)
    m.add_argument("--timeout", type=float, default=3.0, help="each GET's timeout")
    m.add_argument("--settle", type=float, default=12.0,
                   help="seconds before the second look, past zenoh's 10 s lease")
    m.add_argument("--asks", type=int, default=20, help="the most rounds of the cell's reads in one trial")
    m.add_argument("--own-recorder", action="store_true",
                   help="the queryable on @/** on the node's own session, as the acl-… cells have it")
    m.add_argument("--client-recorder", action="store_true",
                   help="acl-… cells: the queryable on @/** on a client, bench, allowed to declare it")
    args = ap.parse_args(argv)
    if args.cmd == "router":
        router_main(args)
        return 0
    if args.cmd == "load":
        load_main(args)
        return 0
    return measure_main(args)


if __name__ == "__main__":
    rc = main()
    sys.stdout.flush()
    os._exit(rc)

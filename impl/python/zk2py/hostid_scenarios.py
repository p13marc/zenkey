"""The ``hostid.v1`` scenarios (``spec/profiles/hostid/scenarios.md``) in
temporary roots: ``python -m zk2py.hostid_scenarios [--only 1 …]``.

- **The root** is :class:`zk2py.hostid.Runtime`'s seam, a directory standing
  in for ``/`` (scenarios.md, "A root"). Paths resolve in it as a chroot
  would (§2.4, 0.2). Nothing under the real ``/etc`` or ``/var/lib`` is
  read or written.
- **Cases a runner cannot cause as root** need no privilege here:
  - the runner is no root, so ``chmod`` makes a file unreadable and a
    directory unwritable;
  - ``link(2)`` refused is the runtime's ``link`` seam.
- **The bus**, where a section expects observations on it, is an in-process
  zenoh-python router R1 with zk2py's own owners and tool as its clients.
  There is no Rust owner here: the owner example mints in ``just py-live``'s
  run ``hostid``.
- **A process** is one :class:`zk2py.hostid.Runtime`:
  - §2's racers are separate OS processes;
  - §5's restart is a new runtime;
  - §6's two hosts are two runtimes on two roots.
- **A re-mint** (§5) is approximated: zk2py's owner has no re-mint (core
  §8.1). A new owner of the same address in the same process stands in,
  and the old one then stops.

Exit 0 when every check passes, 1 when any fails.
"""

from __future__ import annotations

import argparse
import json
import multiprocessing
import os
import shutil
import stat
import sys
import tempfile
import time
from pathlib import Path
from typing import Any

from . import hostid
from .contract import load_contract

REPO = Path(__file__).resolve().parents[3]
SYSINFO = REPO / "impl" / "python" / "interop" / "zk2py_sysinfo.v1.toml"
SYSINFO_X = REPO / "impl" / "python" / "interop" / "zk2py_sysinfo_x.v1.toml"
TRACKER = REPO / "impl" / "python" / "interop" / "zk2py_tracker.v1.toml"
#: scenarios.md, "Machine ids".
M1, M2, M3 = ("b642b4217b34b1e8d3bd915fc65c4452", "0123456789abcdef0123456789abcdef",
              "ffffffffffffffffffffffffffffffff")
SYSTEM = {M1: "h-bbd1aa1db10b", M2: "h-3f6d94515669", M3: "h-504c6767c349"}
ETC, DBUS, SHARED = "etc/machine-id", "var/lib/dbus/machine-id", "var/lib/zk2/hostid"


class Report:
    def __init__(self) -> None:
        self.rows: list[tuple[str, str, bool, str]] = []

    def check(self, section: str, name: str, ok: bool, detail: str = "") -> bool:
        self.rows.append((section, name, ok, detail))
        print(f"{'PASS' if ok else 'FAIL'} [{section}] {name}" + (f": {detail}" if detail else ""), flush=True)
        return ok

    def info(self, section: str, text: str) -> None:
        print(f"info [{section}] {text}", flush=True)


# -- roots -------------------------------------------------------------------------

def make_root(base: str, files: dict[str, str] | None = None, *, var_lib: bool = True,
              dirs: tuple[str, ...] = (), links: dict[str, str] | None = None) -> str:
    """A fresh root: ``var/lib`` exists unless told otherwise, each file of
    ``files`` holds its content, and each of ``links`` is a symbolic link to
    its target, as written (an absolute one included)."""
    root = tempfile.mkdtemp(prefix="root-", dir=base)
    if var_lib:
        os.makedirs(os.path.join(root, "var", "lib"))
    for d in dirs:
        os.makedirs(os.path.join(root, d), exist_ok=True)
    for rel, content in (files or {}).items():
        path = os.path.join(root, rel)
        os.makedirs(os.path.dirname(path), exist_ok=True)
        with open(path, "w", encoding="utf-8") as f:
            f.write(content)
    for rel, target in (links or {}).items():
        path = os.path.join(root, rel)
        os.makedirs(os.path.dirname(path), exist_ok=True)
        os.symlink(target, path)
    return root


def tree(root: str) -> list[tuple[str, int, int]]:
    """Every entry under ``root``: (path, size, mode), for "nothing is
    written"."""
    out = []
    for d, subdirs, files in os.walk(root):
        for name in subdirs + files:
            p = os.path.join(d, name)
            st = os.lstat(p)
            out.append((os.path.relpath(p, root), st.st_size, stat.S_IMODE(st.st_mode)))
    return sorted(out)


def temps(root: str) -> list[str]:
    d = os.path.join(root, "var", "lib", "zk2")
    return sorted(n for n in os.listdir(d) if n.startswith(".hostid.")) if os.path.isdir(d) else []


def read_shared(root: str) -> str | None:
    try:
        return Path(root, SHARED).read_text(encoding="utf-8")
    except OSError:
        return None


def outcomes(err_or_minted: Any) -> list[tuple[str, str]]:
    return [(o.path, o.outcome) for o in err_or_minted.outcomes]


def chmod_back(root: str) -> None:
    """Make a root removable again after a section made parts unreadable."""
    for d, subdirs, files in os.walk(root):
        for name in subdirs + files:
            path = os.path.join(d, name)
            try:
                if not os.path.islink(path):  # a link's own mode is not settable on Linux
                    os.chmod(path, 0o755)
            except OSError:
                pass


# -- the bus ----------------------------------------------------------------------

class Bus:
    """R1, a tool, and a watcher, all zenoh-python, in this process."""

    def __init__(self) -> None:
        import zenoh

        from . import live
        from .live_interop import _r1

        self.zenoh = zenoh
        self.router, self.endpoint, self.zid = _r1()
        self.tool = live.open_client(self.endpoint)
        self.received: list[tuple[str, bytes]] = []
        self.tokens: list[tuple[str, str]] = []
        self._subs = [
            self.tool.declare_subscriber("zk2/**", zenoh.handlers.Callback(
                lambda s: self.received.append((str(s.key_expr), s.payload.to_bytes())))),
            self.tool.declare_subscriber("zk2/*/*/@zk/instance/*", zenoh.handlers.Callback(
                lambda s: self.received.append((str(s.key_expr), s.payload.to_bytes())))),
            self.tool.liveliness().declare_subscriber("zk2/*/*/@zk/**", zenoh.handlers.Callback(
                lambda s: self.tokens.append((str(s.kind), str(s.key_expr))))),
        ]

    def owner(self, address: str, runtime: hostid.Runtime | None, contracts=None, **kw):
        from .contract import load_contract
        from .owner import Owner

        system, service = address.split("/")
        contracts = [load_contract(SYSINFO)] if contracts is None else contracts
        return Owner(system, service, contracts, connect=self.endpoint, hostid=runtime, **kw)

    def wait_instance(self, owner, timeout: float = 3.0) -> bool:
        from . import live

        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            pres = live.list_presence(self.tool, f"zk2/{owner.system}/{owner.service}/@zk/instance/*")
            if any(i["instance"] == owner.instance for i in pres.instances):
                return True
            time.sleep(0.05)
        return False

    def descriptor(self, owner) -> dict[str, Any] | None:
        from . import live

        got = [a for a in live.get_descriptor(self.tool, owner.instance_key) if a.ok]
        if len(got) == 1:
            self.received.append((owner.instance_key, got[0].payload))
            return json.loads(got[0].payload)
        return None

    def close(self) -> None:
        for s in self._subs:
            s.undeclare()
        self.tool.close()
        self.router.close()


def leaks(seen: list[tuple[str, bytes]], secrets: list[str]) -> list[str]:
    """hostid.v1 §2.10: "the hex text in either case, the 16 bytes, or a UUID
    spelling"."""
    found = []
    for secret in secrets:
        raw = bytes.fromhex(secret)
        uuid = f"{secret[:8]}-{secret[8:12]}-{secret[12:16]}-{secret[16:20]}-{secret[20:]}"
        forms = [secret.encode(), secret.upper().encode(), raw, uuid.encode(), uuid.upper().encode()]
        for key, payload in seen:
            blob = key.encode() + b"\0" + payload
            if any(f in blob for f in forms):
                found.append(f"{secret[:6]}… in {key}")
    return found


# -- §1 the input order -------------------------------------------------------------

def section1(report: Report, base: str) -> None:
    sec = "§1 input order"
    bus = Bus()
    try:
        steps = [
            ("1", {ETC: M1 + "\n", DBUS: M2 + "\n"}, "@hostid.v1/sysinfo"),
            ("2", {ETC: "uninitialized\n", DBUS: M2 + "\n"}, "@hostid.v1/sysinfo"),
            ("3", {ETC: "", SHARED: M3 + "\n"}, "@hostid.v1/sysinfo"),
            ("4", {ETC: "0" * 32 + "\n", DBUS: "not-a-machine-id\n"}, "@hostid.v1/sysinfo"),
            ("5", {ETC: M1 + "\n", DBUS: M2 + "\n"}, "h-bbd1aa1db10b/sysinfo"),
            ("6", {SHARED: M3 + "\n"}, "@hostid.v1/sysinfo"),
            ("7", {ETC: "uninitialized\n", "srv/machine-id": M2 + "\n"}, "@hostid.v1/sysinfo"),
        ]
        #: 0.2's steps 6 and 7: var/lib/dbus/machine-id as an absolute link.
        links = {"6": {DBUS: "/etc/machine-id"}, "7": {DBUS: "/srv/machine-id"}}
        secrets = [M1, M2, M3]
        for step, files, address in steps:
            root = make_root(base, files, links=links.get(step))
            before = read_shared(root)
            runtime = hostid.Runtime(root)
            o = bus.owner(address, runtime)
            o.start()
            try:
                up = bus.wait_instance(o)
                doc = bus.descriptor(o) or {}
                listed = "hostid.v1" in (doc.get("profiles") or [])
                shared = read_shared(root)
                if shared:
                    secrets.append(shared.strip())
                if step == "1":
                    ok = (o.system == SYSTEM[M1] and up and listed and shared is None)
                    detail = f"system {o.system}, token {up}, profiles {doc.get('profiles')}, shared {shared!r}"
                    report.check(sec, "step 1: M1's system, its token under it, hostid.v1 listed, no shared "
                                      "file created", ok, detail)
                elif step == "2":
                    report.check(sec, "step 2: uninitialized is skipped: M2's system",
                                 o.system == SYSTEM[M2] and up and listed, f"system {o.system}")
                elif step == "3":
                    report.check(sec, "step 3: M3's system, from the shared file, left unchanged",
                                 o.system == SYSTEM[M3] and up and listed and shared == before,
                                 f"system {o.system}, shared {shared!r}")
                elif step == "4":
                    mode = stat.S_IMODE(os.stat(os.path.join(root, SHARED)).st_mode) if shared else None
                    ok = (shared is not None and len(shared) == 33 and shared.endswith("\n")
                          and all(c in "0123456789abcdef" for c in shared[:32]) and mode == 0o644
                          and not temps(root) and o.system == hostid.derive(shared) and up and listed)
                    report.check(sec, "step 4: neither file yields an id; the shared file is created (32 lowercase "
                                      "hex digits and a newline, mode 0644, no temporary file), and the system "
                                      "is its derivation", ok,
                                 f"shared {shared!r}, mode {oct(mode) if mode is not None else None}, temps "
                                 f"{temps(root)}, system {o.system}")
                elif step == "6":
                    dbus = next((x for x in runtime.minted.outcomes if x.path == "/var/lib/dbus/machine-id"), None)
                    report.check(sec, "step 6 (0.2): var/lib/dbus/machine-id links to the absolute /etc/machine-id, "
                                      "absent in the root: both machine-id files are absent, and the system is M3's, "
                                      "whatever the host running the scenario holds",
                                 o.system == SYSTEM[M3] and dbus is not None and dbus.outcome == "absent" and listed,
                                 f"system {o.system}; {[str(x) for x in runtime.minted.outcomes]}")
                elif step == "7":
                    report.check(sec, "step 7 (0.2): the link to the absolute /srv/machine-id resolves in the root: "
                                      "M2's system", o.system == SYSTEM[M2] and listed, f"system {o.system}")
                else:
                    report.check(sec, "step 5 (control): the literal h-bbd1aa1db10b/sysinfo is literal: hostid.v1 "
                                      "not listed, and no input read",
                                 o.system == SYSTEM[M1] and up and not listed and not runtime.reads,
                                 f"profiles {doc.get('profiles')}, reads {runtime.reads}")
            finally:
                o.close()
            time.sleep(0.2)
        found = leaks(bus.received + [(k, b"") for _, k in bus.tokens], secrets)
        report.check(sec, "every step: nothing the tool received (samples, replies, tokens, descriptors) holds "
                          "M1, M2, M3 or the shared file's content, in any form (§2.10)",
                     not found and len(bus.received) > 0,
                     f"{len(bus.received)} samples and replies, {len(bus.tokens)} tokens; leaks {found}")
    finally:
        bus.close()


# -- §2 the shared file under racers ------------------------------------------------

def _racer(index: int, inbox, outbox, barrier) -> None:
    """One racer, a process of its own: each round, a fresh runtime (nothing
    cached), released by the barrier."""
    while True:
        root = inbox.get()
        if root is None:
            return
        try:
            barrier.wait(30)
            rt = hostid.Runtime(root)
            r = rt.configure({"address": f"@hostid.v1/svc-{index}"})
            last = rt.minted.outcomes[-1] if rt.minted else None
            outbox.put((index, r.system, last.outcome if last else None, (last.note or "") if last else ""))
        except Exception as e:  # noqa: BLE001 - reported to the parent
            outbox.put((index, None, None, repr(e)))


def section2(report: Report, base: str, rounds: int = 100, racers: int = 16) -> None:
    sec = "§2 racers"
    ctx = multiprocessing.get_context("spawn")
    barrier = ctx.Barrier(racers)
    inboxes = [ctx.Queue() for _ in range(racers)]
    outbox = ctx.Queue()
    procs = [ctx.Process(target=_racer, args=(i, inboxes[i], outbox, barrier), daemon=True) for i in range(racers)]
    for p in procs:
        p.start()
    try:
        for variant, dirs in (("no var/lib/zk2", ()), ("var/lib/zk2 present and empty", ("var/lib/zk2",))):
            systems, bad = [], []
            winners: dict[int, int] = {}
            eexist = 0
            for n in range(rounds):
                root = make_root(base, {}, dirs=dirs)
                for q in inboxes:
                    q.put(root)
                got = [outbox.get(timeout=60) for _ in range(racers)]
                content = read_shared(root)
                same = {g[1] for g in got}
                created = sum(1 for g in got if g[2] == hostid.CREATED)
                eexist += sum(1 for g in got if g[3] == "written by another racer")
                winners[created] = winners.get(created, 0) + 1
                ok = (len(same) == 1 and None not in same and content is not None and len(content) == 33
                      and content.endswith("\n") and all(c in "0123456789abcdef" for c in content[:32])
                      and hostid.derive(content) in same and not temps(root) and created == 1)
                if not ok:
                    bad.append((n, sorted(same, key=str), content, temps(root), [g[3] for g in got if g[3]][:2]))
                systems.append(next(iter(same)))
                shutil.rmtree(root)
            report.check(sec, f"{variant}: {rounds} rounds of {racers} processes released by a barrier: every racer "
                              "has the shared file's system, one file of 32 hex digits and a newline, one winner, "
                              "no temporary file", not bad,
                         f"failed rounds {bad[:3]}; winners per round {winners}; "
                         f"{eexist} racers lost at link(2) (EEXIST)")
            report.check(sec, f"{variant}: rounds on different roots have different systems",
                         len(set(systems)) == len(systems), f"{len(set(systems))} distinct of {len(systems)}")
    finally:
        for q in inboxes:
            q.put(None)
        for p in procs:
            p.join(10)


# -- §3 fail closed ---------------------------------------------------------------

def section3(report: Report, base: str) -> None:
    sec = "§3 fail closed"
    bus = Bus()
    roots = []
    try:
        def unwritable_zk2() -> str:
            r = make_root(base, {}, dirs=("var/lib/zk2",))
            os.chmod(os.path.join(r, "var", "lib", "zk2"), 0o555)
            return r

        def watch(root: str, runtime: hostid.Runtime, address: str = "@hostid.v1/sysinfo"):
            n_tokens, n_received = len(bus.tokens), len(bus.received)
            o = bus.owner(address, runtime)
            try:
                o.start()
                started = True
            except (hostid.HostidError, hostid.ConfigError) as e:
                started, o = e, None
            time.sleep(1.0)
            new = [k for _, k in bus.tokens[n_tokens:]] + [k for k, _ in bus.received[n_received:]]
            return started, o, new

        # Step 1.
        r1 = unwritable_zk2()
        roots.append(r1)
        err, _, new = watch(r1, hostid.Runtime(r1))
        ok = (isinstance(err, hostid.HostidError)
              and outcomes(err) == [("/etc/machine-id", "absent"), ("/var/lib/dbus/machine-id", "absent"),
                                    ("/var/lib/zk2/hostid", "not created")]
              and err.outcomes[-1].error and "EACCES" in err.outcomes[-1].error and not new)
        report.check(sec, "step 1: an unwritable var/lib/zk2: the service does not start; the error names "
                          "/etc/machine-id (absent), /var/lib/dbus/machine-id (absent), /var/lib/zk2/hostid (not "
                          "created, EACCES); no token or descriptor appears through R1", ok,
                     f"{err}; new on the bus {new}")
        # Step 5, the control: the root of step 1 with M1.
        r5 = unwritable_zk2()
        roots.append(r5)
        Path(r5, ETC).parent.mkdir(parents=True, exist_ok=True)
        Path(r5, ETC).write_text(M1 + "\n")
        started, o, new = watch(r5, hostid.Runtime(r5))
        try:
            report.check(sec, "step 5 (control): with M1, the service starts as h-bbd1aa1db10b and its token "
                              "appears through R1",
                         started is True and o.system == SYSTEM[M1]
                         and any(f"zk2/{SYSTEM[M1]}/sysinfo/@zk/instance/" in k for k in new),
                         f"started {started}, new {new[:3]}")
        finally:
            if o is not None:
                o.close()
        # Step 2.
        r2 = make_root(base, {SHARED: "garbage"})
        roots.append(r2)
        err, _, _ = watch(r2, hostid.Runtime(r2))
        report.check(sec, "step 2: the shared file holds garbage: no start; it is named refused by §2.1, and still "
                          "holds garbage",
                     isinstance(err, hostid.HostidError) and outcomes(err)[-1] == ("/var/lib/zk2/hostid", "refused")
                     and read_shared(r2) == "garbage", f"{err}")
        # Step 3.
        r3 = make_root(base, {ETC: M1 + "\n"}, dirs=("var/lib/zk2",))
        roots.append(r3)
        os.chmod(os.path.join(r3, ETC), 0o000)
        err, _, _ = watch(r3, hostid.Runtime(r3))
        report.check(sec, "step 3: /etc/machine-id holds M1 but cannot be read: no start; named unreadable with "
                          "EACCES; no shared file created",
                     isinstance(err, hostid.HostidError)
                     and outcomes(err) == [("/etc/machine-id", "unreadable")]
                     and "EACCES" in (err.outcomes[0].error or "") and read_shared(r3) is None, f"{err}")
        # Step 4: link(2) refused, through the runtime's seam.
        def refuse_link(src: str, dst: str) -> None:
            raise PermissionError(1, "Operation not permitted", dst)

        r4 = make_root(base, {}, dirs=("var/lib/zk2",))
        roots.append(r4)
        err, _, new = watch(r4, hostid.Runtime(r4, link=refuse_link))
        report.check(sec, "step 4: link(2) fails with EPERM: as step 1; no temporary file remains, and no final "
                          "file was made by other means",
                     isinstance(err, hostid.HostidError) and outcomes(err)[-1] == ("/var/lib/zk2/hostid", "not created")
                     and "EPERM" in (err.outcomes[-1].error or "") and not temps(r4) and read_shared(r4) is None
                     and not new, f"{err}; temps {temps(r4)}")
    finally:
        for r in roots:
            chmod_back(r)
        bus.close()


# -- §4 ephemeral -----------------------------------------------------------------

def section4(report: Report, base: str) -> None:
    sec = "§4 ephemeral"
    bus = Bus()
    roots = []
    try:
        def root_of_3_1() -> str:
            r = make_root(base, {}, dirs=("var/lib/zk2",))
            os.chmod(os.path.join(r, "var", "lib", "zk2"), 0o555)
            roots.append(r)
            return r

        # Step 1: two runs. hostid.v1 0.3: the log goes "wherever the
        # process's operational logs go, at its warning level", and "a
        # scenario reads it there, through a log capture in process".
        import logging

        captured: list[logging.LogRecord] = []

        class Capture(logging.Handler):
            def emit(self, record: logging.LogRecord) -> None:
                captured.append(record)

        handler = Capture(level=logging.DEBUG)
        logging.getLogger("zk2py.hostid").addHandler(handler)
        r = root_of_3_1()
        before = tree(r)
        runs = []
        for _ in range(2):
            rt = hostid.Runtime(r)
            o = bus.owner("@hostid.v1/sysinfo", rt, hostid_ephemeral=True)
            o.start()
            try:
                bus.wait_instance(o)
                doc = bus.descriptor(o) or {}
                runs.append((o.system, "hostid.v1" in (doc.get("profiles") or []), rt.logs))
            finally:
                o.close()
        logging.getLogger("zk2py.hostid").removeHandler(handler)
        logs_ok = all(len(logs) == 1 and "ephemeral" in logs[0] and all(p in logs[0] for p in
                      ("/etc/machine-id", "/var/lib/dbus/machine-id", "/var/lib/zk2/hostid")) for _, _, logs in runs)
        warned = [rec.getMessage() for rec in captured if rec.levelno == logging.WARNING]
        logs_ok = logs_ok and warned == [logs[0] for _, _, logs in runs]
        report.check(sec, "step 1: both runs start, each system in the minted shape, the two differ, each lists "
                          "hostid.v1, each start logs the ephemeral system with the three paths, at WARNING through "
                          "the process's logging (0.3), and nothing is written under the root",
                     all(hostid.is_minted_shape(s) and listed for s, listed, _ in runs) and runs[0][0] != runs[1][0]
                     and logs_ok and tree(r) == before,
                     f"systems {[s for s, _, _ in runs]}; log {runs[0][2][:1]}; {len(warned)} WARNING records")
        # Step 2: one process, two ephemeral services.
        rt = hostid.Runtime(root_of_3_1())
        a = rt.configure({"address": "@hostid.v1/a", "hostid": {"ephemeral": True}})
        b = rt.configure({"address": "@hostid.v1/b", "hostid": {"ephemeral": True}})
        report.check(sec, "step 2: one process, a and b both ephemeral: the same system",
                     a.system == b.system and rt.minted.ephemeral, f"{a.system}, {b.system}")
        # Step 3: a with ephemeral, then b without.
        rt = hostid.Runtime(root_of_3_1())
        oa = bus.owner("@hostid.v1/a", rt, hostid_ephemeral=True)
        oa.start()
        n_tokens = len(bus.tokens)
        ob = bus.owner("@hostid.v1/b", rt)
        try:
            ob.start()
            b_outcome = "started"
            ob.close()
        except hostid.ConfigError as e:
            b_outcome = f"configuration error: {e}"
        time.sleep(0.5)
        b_tokens = [k for _, k in bus.tokens[n_tokens:] if "/b/@zk/" in k]
        oa.close()
        report.check(sec, "step 3: a (ephemeral) starts; b, without it, does not, as a configuration error, and "
                          "declares nothing", b_outcome.startswith("configuration error") and not b_tokens,
                     f"{b_outcome}; b's tokens {b_tokens}")
        # Step 4: M1 present wins.
        r4 = make_root(base, {ETC: M1 + "\n", DBUS: M2 + "\n"})
        rt = hostid.Runtime(r4)
        s = rt.configure({"address": "@hostid.v1/sysinfo", "hostid": {"ephemeral": True}})
        report.check(sec, "step 4: with M1 present, ephemeral is a fallback, not a mode: h-bbd1aa1db10b",
                     s.system == SYSTEM[M1] and not rt.minted.ephemeral, s.system)
        # Step 5: the roots of §3 steps 2 and 3.
        r52 = make_root(base, {SHARED: "garbage"})
        r53 = make_root(base, {ETC: M1 + "\n"}, dirs=("var/lib/zk2",))
        roots += [r52, r53]
        os.chmod(os.path.join(r53, ETC), 0o000)
        refused = []
        for r in (r52, r53):
            try:
                hostid.Runtime(r).configure({"address": "@hostid.v1/sysinfo", "hostid": {"ephemeral": True}})
                refused.append("started")
            except hostid.HostidError as e:
                refused.append(outcomes(e)[-1][1])
        report.check(sec, "step 5: on the roots of §3 steps 2 and 3, ephemeral or not, the service does not start",
                     refused == ["refused", "unreadable"], str(refused))
        # Step 6 (0.2): link(2) fails with EEXIST, and the final file is
        # absent when read: a racer's file removed in between, through the
        # runtime's seam.
        def raced(src: str, dst: str) -> None:
            raise FileExistsError(17, "File exists", dst)

        r6 = make_root(base, {}, dirs=("var/lib/zk2",))
        try:
            hostid.Runtime(r6, link=raced).configure({"address": "@hostid.v1/sysinfo",
                                                      "hostid": {"ephemeral": True}})
            step6 = None
        except hostid.HostidError as e:
            step6 = e
        report.check(sec, "step 6 (0.2): EEXIST, then the shared file absent when read: the service does not "
                          "start, even ephemeral; the error names /var/lib/zk2/hostid as absent; no temporary file "
                          "remains",
                     step6 is not None and outcomes(step6)[-1] == ("/var/lib/zk2/hostid", "absent")
                     and not temps(r6), f"{step6}; temps {temps(r6)}")
    finally:
        for r in roots:
            chmod_back(r)
        bus.close()


# -- §5 minted once per run ---------------------------------------------------------

def section5(report: Report, base: str) -> None:
    from . import live

    sec = "§5 once per run"
    bus = Bus()
    roots = []
    owners: list[Any] = []
    try:
        def instances(system: str, service: str) -> list[str]:
            return [i["instance"] for i in live.list_presence(bus.tool, f"zk2/{system}/{service}/@zk/instance/*").instances]

        def remint(old, rt):
            new = bus.owner(f"@hostid.v1/{old.service}", rt, hostid_ephemeral=old.hostid_ephemeral)
            new.start()
            owners.append(new)
            bus.wait_instance(new)
            both = instances(new.system, new.service)
            old.close()
            time.sleep(0.5)
            return new, both

        r = make_root(base, {ETC: M1 + "\n"})
        roots.append(r)
        rt = hostid.Runtime(r)
        s1 = bus.owner("@hostid.v1/sysinfo", rt)
        s1.start()
        owners.append(s1)
        bus.wait_instance(s1)
        # Step 1.
        s2, both = remint(s1, rt)
        after = instances(SYSTEM[M1], "sysinfo")
        report.check(sec, "step 1: a re-mint (a new instance of the address in the same process): a new instance "
                          "token under zk2/h-bbd1aa1db10b/sysinfo/, then the old one goes; the system is unchanged",
                     s2.system == s1.system == SYSTEM[M1] and set(both) == {s1.instance, s2.instance}
                     and after == [s2.instance], f"both {both}, after {after}")
        # Step 2.
        Path(r, ETC).write_text(M2 + "\n")
        s3, _ = remint(s2, rt)
        logger = bus.owner("@hostid.v1/logger", rt, contracts=[load_contract(TRACKER)],
                           bindings={"sources": ["self.system/sysinfo"]})
        logger.start()
        owners.append(logger)
        bus.wait_instance(logger)
        ldoc = bus.descriptor(logger) or {}
        lbind = [r.get("bindings") for r in ldoc.get("requires", [])]
        report.check(sec, "step 2: with etc/machine-id now M2, the re-mint and a second service, logger, keep "
                          "h-bbd1aa1db10b: the process minted once; logger's descriptor lists its binding "
                          "self.system/sysinfo resolved (core R1, 0.20)",
                     s3.system == logger.system == SYSTEM[M1] and lbind == [[f"{SYSTEM[M1]}/sysinfo"]],
                     f"sysinfo {s3.system}, logger {logger.system}, bindings {lbind}")
        # Step 3: restart.
        for o in owners:
            o.close()
        owners.clear()
        time.sleep(0.5)
        rt = hostid.Runtime(r)
        a = bus.owner("@hostid.v1/sysinfo", rt)
        b = bus.owner("@hostid.v1/logger", rt)
        for o in (a, b):
            o.start()
            owners.append(o)
            bus.wait_instance(o)
        stale = live.list_presence(bus.tool, f"zk2/{SYSTEM[M1]}/*/@zk/**").count
        report.check(sec, "step 3: after a restart, both services are under zk2/h-3f6d94515669/, and no token "
                          "remains under zk2/h-bbd1aa1db10b/",
                     a.system == b.system == SYSTEM[M2] and stale == 0, f"{a.system}, {b.system}; stale {stale}")
        for o in owners:
            o.close()
        owners.clear()
        # Step 4: steps 1 and 3 with an ephemeral service, on the root of §3 step 1.
        re = make_root(base, {}, dirs=("var/lib/zk2",))
        os.chmod(os.path.join(re, "var", "lib", "zk2"), 0o555)
        roots.append(re)
        rt = hostid.Runtime(re)
        e1 = bus.owner("@hostid.v1/sysinfo", rt, hostid_ephemeral=True)
        e1.start()
        owners.append(e1)
        bus.wait_instance(e1)
        e2, _ = remint(e1, rt)
        first = e2.system
        e2.close()
        rt2 = hostid.Runtime(re)
        e3 = bus.owner("@hostid.v1/sysinfo", rt2, hostid_ephemeral=True)
        e3.start()
        owners.append(e3)
        report.check(sec, "step 4: ephemeral: the re-mint keeps the ephemeral system, and the restart mints "
                          "another", e1.system == first and e3.system != first and hostid.is_minted_shape(e3.system),
                     f"{e1.system} → {first}; restart {e3.system}")
        for o in owners:
            o.close()
        owners.clear()
        # Step 5: a literal-only process on the root of §3 step 3.
        rl = make_root(base, {ETC: M1 + "\n"}, dirs=("var/lib/zk2",))
        os.chmod(os.path.join(rl, ETC), 0o000)
        roots.append(rl)
        rt = hostid.Runtime(rl)
        lit = bus.owner("vehicle-01/sysinfo", rt)
        lit.start()
        owners.append(lit)
        up = bus.wait_instance(lit)
        report.check(sec, "step 5: a process whose only service is literal starts on a root whose "
                          "etc/machine-id cannot be read: it reads no input", up and not rt.reads,
                     f"started {up}, reads {rt.reads}")
        # Step 6 (0.2): on the root of §3 step 1, a fails; the host is fixed;
        # b starts; c, with ephemeral, is a configuration error.
        r6 = make_root(base, {}, dirs=("var/lib/zk2",))
        os.chmod(os.path.join(r6, "var", "lib", "zk2"), 0o555)
        roots.append(r6)
        rt = hostid.Runtime(r6)
        got = []
        for name, eph in (("a", None), ("b", None), ("c", True)):
            if name == "b":
                Path(r6, ETC).parent.mkdir(parents=True, exist_ok=True)
                Path(r6, ETC).write_text(M1 + "\n")
            o = bus.owner(f"@hostid.v1/{name}", rt, hostid_ephemeral=eph)
            try:
                o.start()
                owners.append(o)
                got.append((name, "started", o.system))
            except hostid.HostidError:
                got.append((name, "failed closed", None))
            except hostid.ConfigError:
                got.append((name, "configuration error", None))
        report.check(sec, "step 6 (0.2): a does not start; the host fixed, b starts with h-bbd1aa1db10b (a failure "
                          "mints nothing, the inputs are read again); c, with ephemeral, is a configuration error, "
                          "a having fixed the setting although it did not start",
                     got == [("a", "failed closed", None), ("b", "started", SYSTEM[M1]),
                             ("c", "configuration error", None)], str(got))
    finally:
        for o in owners:
            o.close()
        for r in roots:
            chmod_back(r)
        bus.close()


# -- §6 what a tool concludes ---------------------------------------------------------

def section6(report: Report, base: str) -> None:
    from . import live

    sec = "§6 a tool"
    bus = Bus()
    owners: list[Any] = []
    try:
        def host(mid: str, name: str, contracts=None, **kw):
            rt = hostid.Runtime(make_root(base, {ETC: mid + "\n"}))
            o = bus.owner("@hostid.v1/sysinfo", rt, contracts=contracts or [], meta_host=name, **kw)
            o.start()
            owners.append(o)
            bus.wait_instance(o)
            return o

        def stop_all():
            for o in owners:
                o.close()
            owners.clear()
            time.sleep(0.5)

        a, b = host(M1, "host-a"), host(M1, "host-b")
        answer, why, reads = live.hostid_collision(bus.tool, f"{SYSTEM[M1]}/sysinfo", grace_s=2.0)
        hosts = sorted({(d or {}).get("meta", {}).get("host") for _, rows in reads for _, d, _ in rows} - {None})
        report.check(sec, "step 1: two hosts from one image with M1: two instances of h-bbd1aa1db10b/sysinfo with "
                          "different meta.zid in both reads: the finding, its cause undecided, naming none",
                     a.system == b.system == SYSTEM[M1] and answer == "finding" and "undecided" in why,
                     f"{answer}: {why}; meta.host {hosts}")
        stop_all()
        a, b = host(M1, "host-a"), host(M2, "host-b")
        on_a = live.hostid_collision(bus.tool, f"{SYSTEM[M1]}/sysinfo", grace_s=2.0)[0]
        on_b = live.hostid_collision(bus.tool, f"{SYSTEM[M2]}/sysinfo", grace_s=2.0)[0]
        report.check(sec, "step 2 (control): B with M2: two systems, and no finding",
                     a.system == SYSTEM[M1] and b.system == SYSTEM[M2] and on_a == on_b == "no",
                     f"{a.system}: {on_a}; {b.system}: {on_b}")
        stop_all()
        host(M1, "host-a")
        host(M1, "host-b", state_zid=False)
        answer, why, _ = live.hostid_collision(bus.tool, f"{SYSTEM[M1]}/sysinfo", grace_s=2.0)
        report.check(sec, "step 3: B states no meta.zid: the address is undecided: unobservable, never clean",
                     answer == "unobservable", f"{answer}: {why}")
        stop_all()
        # Step 5 (0.2): as step 1, each service also an owner of a contract
        # that lists hostid.v1 in uses, held in its tokenless set.
        x = [load_contract(SYSINFO_X)]
        host(M1, "host-a", contracts=x, tokenless={"zk2py_sysinfo_x.v1"})
        host(M1, "host-b", contracts=x, tokenless={"zk2py_sysinfo_x.v1"})
        answer, why, reads = live.hostid_collision(bus.tool, f"{SYSTEM[M1]}/sysinfo", grace_s=2.0)
        firsts = sorted({first for _, rows in reads for _, _, first in rows})
        report.check(sec, "step 5 (0.2): both list hostid.v1, but a contract each implements does too: neither is "
                          "counted, and the address is unobservable, never clean and never the finding",
                     answer == "unobservable" and firsts == ["unobservable"], f"{answer}: {why}; first {firsts}")
        stop_all()
        rt = hostid.Runtime(make_root(base, {ETC: M1 + "\n"}))
        logger = bus.owner("h-504c6767c349/logger", rt, contracts=[])
        logger.start()
        owners.append(logger)
        bus.wait_instance(logger)
        doc = bus.descriptor(logger)
        answer, why = hostid.minted_by_listing(doc, set())
        report.check(sec, "step 4: a literal h-504c6767c349/logger: not minted, whatever the shape of its system",
                     answer == "no" and hostid.is_minted_shape(logger.system), f"{answer}: {why}")
    finally:
        for o in owners:
            o.close()
        bus.close()


SECTIONS = {"1": section1, "2": section2, "3": section3, "4": section4, "5": section5, "6": section6}


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="python -m zk2py.hostid_scenarios", description=__doc__.split("\n")[0])
    ap.add_argument("--only", action="append", choices=sorted(SECTIONS), help="run only these sections")
    ap.add_argument("--rounds", type=int, default=100, help="§2's rounds per variant")
    args = ap.parse_args(argv)
    report = Report()
    base = tempfile.mkdtemp(prefix="zk2py-hostid-")
    try:
        for name, fn in SECTIONS.items():
            if args.only and name not in args.only:
                continue
            if name == "2":
                fn(report, base, rounds=args.rounds)
            else:
                fn(report, base)
    finally:
        chmod_back(base)
        shutil.rmtree(base, ignore_errors=True)
    passed = sum(1 for r in report.rows if r[2])
    failed = len(report.rows) - passed
    print(f"hostid scenarios: {passed} passed, {failed} failed")
    for sec, name, ok, detail in report.rows:
        if not ok:
            print(f"FAIL [{sec}] {name}: {detail}")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())

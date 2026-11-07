"""The ``config.v2`` scenarios (``spec/profiles/config/scenarios-v2.md``,
text 0.1, #985 G1; #609, round 102) on in-process zenoh-python routers, in
three pairings: ``python -m zk2py.config_v2_scenarios [--pairing …]
[--only 1 …]``.

- **py-py:** zk2py's owner (:class:`zk2py.config_v2.ConfigV2Owner`, its
  device :class:`zk2py.config.SimDevice`) and zk2py's caller
  (:class:`zk2py.config_v2.Caller`), zk2py's tool for §14;
- **rust-py:** the owner example's ``--config-v2`` (a black box, driven by its
  ``config`` commands) and zk2py's caller and tool;
- **py-zenctl:** zk2py's owner and ``zenctl config`` (a black box) as C and,
  for §14, as T: ``set`` sent with ``--no-precheck`` where the owner's own
  answer is the observable, ``get --for`` or ``--clocks-synced`` as the tool.

Either owner:
- **The bus** is the text's: R1, a router with timestamping on; the owner,
  C, S and T its clients, each with a session of its own. Timeouts 1 s, a
  call's 2 s. No catalog, so the epoch is 0.
- **The owner** is ``lab/rf0`` on ``rfcfg.v1``: the startup of
  ``lifecycle.json``, floor 10 s, ``apply_at`` 1 s after a reach ``set``, a
  persisted layer, ``queue`` outside the allowlist.
- **S** records every delivery on ``zk2/lab/<svc>/rfcfg.v1/**`` and
  ``zk2/lab/<svc>/@zk/**``, and follows the heartbeat key covering
  ``rfcfg.v1`` (:class:`zk2py.freshness_v2.Subscriber`).
- **§8** runs C's router with usrpwd and access control, as config.v1's §8:
  zk2py's owner authenticates, and the owner example sits on an open router
  R0 linked to R1, R1 under ``allow`` with ``ops``'s ``persist`` denied.
- **§10** drops step 1's reply at zk2py's owner; the owner example cannot,
  so C sets its first reply aside unread. Its 300 s are the owner's clock
  moved (zk2py's ``offset_ms``, the owner example's ``config offset``).
- **§12**'s document reaches either owner directly (zk2py's
  :meth:`ConfigV2Owner.desire`, the owner example's ``config desired``): the
  binding is ``desired.v1``'s, which config.v1's scenarios run.
- **§13**'s face is config.v1's §13's: C a client of a router whose access
  control denies ``zk2/**/@zk/**`` to C's principal both ways, C's batch at
  1 KB. C binds statically, holding the contract.

Exit 0 when every check passes, 1 when any fails."""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path
from typing import Any

from . import config as v1
from . import config_v2 as c2
from . import freshness as fr
from . import zk2c

REPO = Path(__file__).resolve().parents[3]
LIFECYCLE = REPO / "spec" / "profiles" / "config" / "conformance" / "v2" / "lifecycle.json"
NAV = REPO / "examples" / "zk2" / "walkthrough" / "nav.v2.toml"
BLACKBOX = Path(os.environ.get("ZK2PY_BLACKBOX", "/srv/dev/scratch/zenkey/bin/current"))
IFACE = "rfcfg.v1"
NS = 1_000_000_000
PAIRINGS = ("py-py", "rust-py", "py-zenctl")
_ANSI = re.compile(r"\x1b\[[0-9;]*m")


def setup() -> dict[str, Any]:
    return json.loads(LIFECYCLE.read_text())["cases"][0]["setup"]


def contract() -> Any:
    from .contract import load_contract

    return load_contract(c2.TEST_CONTRACT, check_file_name=True)


def layout() -> c2.Layout:
    return c2.Layout.of_contract(contract())


class Report:
    def __init__(self, label: str = "") -> None:
        self.rows: list[tuple[str, str, bool, str]] = []
        self.lock = threading.Lock()
        self.label = label
        self.not_run: list[tuple[str, str]] = []

    def check(self, section: str, name: str, ok: bool, detail: str = "") -> bool:
        section = f"{self.label}{section}"
        with self.lock:
            self.rows.append((section, name, bool(ok), detail))
            print(f"{'PASS' if ok else 'FAIL'} [{section}] {name}" + (f": {detail}" if detail else ""), flush=True)
        return bool(ok)

    def cannot(self, section: str, why: str) -> None:
        with self.lock:
            self.not_run.append((f"{self.label}{section}", why))
            print(f"NOT RUN [{self.label}{section}] {why}", flush=True)


def _wait(pred: Any, timeout: float, step: float = 0.05) -> bool:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if pred():
            return True
        time.sleep(step)
    return bool(pred())


# -- the owners, behind one adapter --------------------------------------------------------

class PyOwner:
    """zk2py's :class:`ConfigV2Owner`; a restart is a new process's: a new
    owner with the same persisted layer, device and event record."""

    rust = False

    def __init__(self, endpoint: str, service: str, *, persist: bool = True, auth: tuple[str, str] | None = None):
        self.endpoint, self.service, self.persist, self.auth = endpoint, service, persist, auth
        self.o = self._make(None, None, None, 0)

    def _make(self, overlay: Any, device: Any, events_at: Any, offset: int) -> c2.ConfigV2Owner:
        s = setup()
        kw: dict[str, Any] = {"auth": self.auth} if self.auth else {}
        o = c2.ConfigV2Owner("lab", self.service, contract(), startup=s["startup"], overlay=overlay,
                             floor_s=10, apply_after_ms=1000, persist=self.persist, writable=s["writable"],
                             device=device, events_at=events_at, offset_ms=offset, connect=self.endpoint, **kw)
        o.start()
        time.sleep(0.3)
        return o

    @property
    def instance(self) -> str:
        return self.o.instance

    @property
    def instance_key(self) -> str:
        return self.o.owner.instance_key

    def device(self) -> dict[str, Any]:
        d = self.o.device
        return {"held": json.loads(json.dumps(d.values)), "applies": len(d.acts("apply")),
                "undos": len(d.acts("undo")), "undo_at": [t for t, *_ in d.acts("undo")]}

    def fail_apply(self, words: str) -> None:
        self.o.device.next_apply.append({"refused": words})

    def fail_undo(self) -> None:
        self.o.device.next_undo.append({"failed": "the radio did not answer", "readback": {}})

    def restart(self) -> None:
        old = self.o
        old.close()
        self.o = self._make(old.core.overlay, old.device, old.core.events_at, old.offset_ms)

    def stop(self) -> None:
        self.o.close()

    def log(self) -> list[str]:
        return list(self.o.core.log)

    def drop_next_reply(self) -> None:
        self.o.core.drop_replies += 1

    def dropped(self) -> list[bytes]:
        return list(self.o.core.dropped)

    def move_clock(self, ms: int) -> None:
        self.o.offset_ms += ms

    def desire(self, doc: dict[str, Any]) -> None:
        self.o.desire(doc)

    def counter(self, n: int) -> str:
        """scenarios §7 step 5's hook, as the owner example's ``config
        counter``: a new run, its descriptor put, whose counter starts at
        ``n``."""
        o = self.o
        with o.lock:
            with o.owner._writes:  # noqa: SLF001
                o.owner.take_run()
                o.owner._put_descriptor()  # noqa: SLF001
            o.core.counter, o.core._counter_run = n, o.owner.run  # noqa: SLF001
        return o.owner.run_hex()

    def remint(self) -> str:
        return self.o.remint()

    def new_run(self) -> str:
        """A new run within the instance (core §4.3), its descriptor put."""
        o = self.o
        with o.lock:
            with o.owner._writes:  # noqa: SLF001
                o.owner.take_run()
                o.owner._put_descriptor()  # noqa: SLF001
        return o.owner.run_hex()


class RustOwner:
    """The owner example's ``--config-v2``, a black box."""

    rust = True

    def __init__(self, endpoint: str, service: str, *, persist: bool = True, frames: bool = False, **_: Any):
        from .live_interop import Owner

        flags = ["--config-v2", "--config-floor", "10", "--config-apply-after-ms", "1000"]
        if not persist:
            flags.append("--config-no-persist")
        if frames:
            # its `frame new-epoch` (a re-mint) and `frame new-run` (core §4.3)
            flags.append("--frames")
        self.p = Owner(BLACKBOX / "owner", f"lab/{service}", [], connect=endpoint, flags=flags)
        key = self.p.wait_for("ready ", 120)
        if key is None:
            raise RuntimeError(f"the owner example did not start: {self.p.stderr[-3:]}")
        self.instance_key = key
        time.sleep(0.3)

    @property
    def instance(self) -> str:
        return self.instance_key.rsplit("/", 1)[1]

    def _cmd(self, line: str, expect: str) -> str:
        got = self.p.command(line, timeout=10.0)
        if got is None or not got.startswith(expect):
            raise RuntimeError(f"{line!r} answered {got!r}")
        return got

    def device(self) -> dict[str, Any]:
        got = self._cmd("config device", "config device ")
        m = re.match(r"config device held=(.*) applies=(\d+) undos=(\d+) persisted=(.*) next=(.*)$", got)
        if m is None:
            raise RuntimeError(f"cannot read {got!r}")
        return {"held": json.loads(m.group(1)), "applies": int(m.group(2)), "undos": int(m.group(3)),
                "persisted": m.group(4)}

    def fail_apply(self, words: str) -> None:
        self._cmd(f"config fail-apply {words}", "config device ")

    def fail_undo(self) -> None:
        self._cmd("config fail-undo unknown", "config device ")

    def restart(self) -> None:
        got = self._cmd("config restart", "config restarted ")
        self.instance_key = got.split()[-1]
        time.sleep(0.5)

    def stop(self) -> None:
        self.p.close()

    def log(self) -> list[str]:
        return [_ANSI.sub("", line) for line in list(self.p.stderr)]

    def move_clock(self, ms: int) -> None:
        self._cmd(f"config offset {ms}", "config offset")

    def desire(self, doc: dict[str, Any]) -> None:
        self._cmd(f"config desired {json.dumps(doc, separators=(',', ':'))}", "config desired")

    def counter(self, n: int) -> str:
        return self._cmd(f"config counter {n}", "config counter ").split()[-1]

    def record(self) -> dict[str, Any]:
        return json.loads(self._cmd("config record", "config record ").split(" ", 2)[2])

    def remint(self) -> str:
        got = json.loads(self._cmd("frame new-epoch", "{"))
        self.instance_key = f"{self.instance_key.rsplit('/', 1)[0]}/{got['instance']}"
        return got["instance"]

    def new_run(self) -> str:
        return json.loads(self._cmd("frame new-run", "{"))["run"]


# -- the callers -----------------------------------------------------------------------------

class PyC:
    """zk2py's caller, per address."""

    name = "zk2py's caller"

    def __init__(self, session: Any, lay: c2.Layout):
        self.session, self.layout = session, lay
        self.by: dict[str, c2.Caller] = {}

    def of(self, address: str) -> c2.Caller:
        if address not in self.by:
            self.by[address] = c2.Caller(self.session, address, self.layout, IFACE)
        return self.by[address]

    def call(self, address: str, op: str, fields: dict[str, Any], *, request_id: str | None = None,
             actor: str | None = "ops", timeout: float = 2.0, **_: Any) -> c2.Called:
        return self.of(address).call(op, fields, actor=actor, request_id=request_id, timeout=timeout)


class ZenctlC:
    """``zenctl config`` as C, a black box: each call one invocation, its
    JSON report read back into a :class:`zk2py.config_v2.Called`."""

    name = "zenctl config"

    def __init__(self, endpoint: str, extra: list[str] | None = None):
        self.endpoint, self.extra = endpoint, list(extra or [])
        self.runs: list[dict[str, Any]] = []

    def run(self, *args: str, timeout_s: float = 2.0) -> dict[str, Any]:
        cmd = [str(BLACKBOX / "zenctl"), "config", *args, "-c", self.endpoint, "--contracts", str(c2.TEST_CONTRACT),
               "--format", "json", "--color", "never", "--timeout", f"{timeout_s:g}", *self.extra]
        p = subprocess.run(cmd, capture_output=True, text=True, timeout=120)
        try:
            doc: Any = json.loads(p.stdout) if p.stdout.strip() else None
        except ValueError:
            doc = {"unparsed": p.stdout[-2000:]}
        out = {"exit": p.returncode, "out": doc, "err": p.stderr[-1500:], "args": list(args)}
        self.runs.append(out)
        return out

    def call(self, address: str, op: str, fields: dict[str, Any], *, request_id: str | None = None,
             actor: str | None = "ops", timeout: float = 2.0, precheck: bool = False, **_: Any) -> c2.Called:
        args: list[str]
        f = dict(fields)
        if op.startswith("set/"):
            group = op[4:]
            args = ["set", address, group]
            g = layout().decl.group(group)
            for k in list(f):
                if k in ("confirm_s", "token", "expected_revision", "idempotency_key", "dry_run"):
                    continue
                prm = None if g is None else g.param(k)
                if prm is not None and prm.sensitive:
                    # zenctl takes a sensitive value from a file alone, never
                    # inline (config.v1 §2.15)
                    fd, path = tempfile.mkstemp(prefix="zk2py-secret-")
                    with os.fdopen(fd, "w") as fh:
                        fh.write(json.dumps(f.pop(k)))
                    args.append(f"{k}=@{path}")
                    continue
                args.append(f"{k}={json.dumps(f.pop(k))}")
            if "confirm_s" in f:
                args += ["--confirm-s", str(f.pop("confirm_s"))]
            if "token" in f:
                args += ["--join", str(f.pop("token"))]
            if "expected_revision" in f:
                args += ["--expected-revision", str(f.pop("expected_revision"))]
            if "idempotency_key" in f:
                args += ["--idempotency-key", str(f.pop("idempotency_key"))]
            if f.pop("dry_run", False):
                args.append("--dry-run")
            args.append("--i-know")
            if not precheck:
                args.append("--no-precheck")
        else:
            args = [op, address, str(f.pop("token", 0))]
            if "confirm_s" in f:
                args += ["--confirm-s", str(f.pop("confirm_s"))]
        if actor is not None:
            args += ["--actor", actor]
        if request_id is not None:
            args += ["--request-id", request_id]
        z = self.run(*args, timeout_s=timeout)
        doc = z["out"] if isinstance(z["out"], dict) else {}

        def mark_of(m: Any) -> zk2c.InBand | None:
            if not isinstance(m, dict) or not isinstance(m.get("run"), str):
                return None
            return zk2c.InBand(len(m["run"]) * 4, int(m["run"], 16), int(m.get("seq", 0)))

        req_bytes = b"\x00" * int(doc.get("request_bytes") or 0)
        if doc.get("answer") == "record":
            st = doc.get("record")
            raw = b"\x00" * int(doc.get("size") or 0)
            return c2.Called("value", state=st, mark=mark_of(doc.get("mark")), raw=raw, request=req_bytes,
                             at_ns=time.time_ns())
        if doc.get("answer") == "refused":
            r = doc.get("refusal") or {}
            det = r.get("error", r.get("detail"))
            app = det.get("code") if isinstance(det, dict) else det
            return c2.Called("error", error={"code": r.get("code"), "cause": r.get("cause"), "app": app,
                                             "message": r.get("message")}, mark=mark_of(r.get("mark")),
                             request=req_bytes, at_ns=time.time_ns())
        return c2.Called("silent", request=req_bytes, at_ns=time.time_ns(),
                         refusal=f"exit {z['exit']}: {z['err'][-300:]}")


# -- the bus ---------------------------------------------------------------------------------

class Bus:
    """R1, and C, S and T as client sessions of it; S's recordings made before
    any owner starts."""

    def __init__(self, pairing: str, addresses: tuple[str, ...] = ("lab/rf0",), *, router: Any = None,
                 sessions: dict[str, Any] | None = None, owner_endpoint: str | None = None,
                 zenctl_extra: list[str] | None = None, selectors: list[str] | None = None) -> None:
        from . import freshness_v2, live
        from .live_interop import _r1

        self.pairing = pairing
        if router is None:
            self.router, self.endpoint, _ = _r1()
        else:
            self.router, self.endpoint = router
        self.owner_endpoint = owner_endpoint or self.endpoint
        sessions = sessions or {}
        self.c_session = sessions.get("C") or live.open_client(self.endpoint)
        self.s_session = sessions.get("S") or live.open_client(self.endpoint)
        self.t = sessions.get("T") or live.open_client(self.endpoint)
        self.layout = layout()
        self.k = contract()
        sel = list(selectors or [])
        if selectors is None:
            for a in addresses:
                sel += [f"zk2/{a}/{IFACE}/**", f"zk2/{a}/@zk/**"]
        self.s = c2.Recorder(self.s_session, sel)
        self.hb = {a: freshness_v2.Subscriber(self.s_session, a, self.k.canonical, self.k.fingerprint, wait_s=0.0)
                   for a in addresses}
        from .config_scenarios import Tokens

        self.tokens = Tokens(self.s_session, "zk2/lab/*/@zk/**")
        if pairing == "py-zenctl":
            self.C: Any = ZenctlC(self.endpoint, zenctl_extra)
        else:
            self.C = PyC(self.c_session, self.layout)
        self.owners: list[Any] = []
        self.extra: list[Any] = []
        time.sleep(0.3)

    @property
    def rust(self) -> bool:
        return self.pairing == "rust-py"

    def owner(self, service: str = "rf0", **kw: Any) -> Any:
        o: Any = RustOwner(self.owner_endpoint, service, **kw) if self.rust else \
            PyOwner(self.owner_endpoint, service, **kw)
        self.owners.append(o)
        return o

    def drop(self, o: Any) -> None:
        o.stop()
        if o in self.owners:
            self.owners.remove(o)

    def call(self, op: str, fields: dict[str, Any] | None = None, address: str = "lab/rf0", **kw: Any) -> c2.Called:
        return self.C.call(address, op, fields or {}, **kw)

    def records(self, address: str = "lab/rf0", since: int = 0) -> list[tuple[int, Any, dict[str, Any]]]:
        out = []
        for d in self.s.of(f"zk2/{address}/{IFACE}/state/config", since):
            if d.kind != "put":
                out.append((d.at_ns, None, {"deleted": True}))
                continue
            try:
                st, rec = c2.read_put(self.layout, d.payload)
            except Exception as e:  # noqa: BLE001 - a record S cannot read is named
                out.append((d.at_ns, None, {"refused": str(e)}))
                continue
            out.append((d.at_ns, st, rec))
        return out

    def raw_records(self, address: str = "lab/rf0", since: int = 0) -> list[bytes]:
        return [d.payload for d in self.s.of(f"zk2/{address}/{IFACE}/state/config", since) if d.kind == "put"]

    def events(self, address: str = "lab/rf0", since: int = 0) -> list[tuple[int, Any, dict[str, Any]]]:
        out = []
        for d in self.s.of(f"zk2/{address}/{IFACE}/events/change/", since):
            env = zk2c.parse(d.payload, "put")
            out.append((d.at_ns, env.stamp, json.loads(env.record or b"{}")))
        return out

    def get(self, address: str = "lab/rf0", session: Any = None) -> tuple[Any, dict[str, Any] | None, bytes]:
        from . import live

        st = live.get_state(session or self.c_session, f"zk2/{address}/{IFACE}/state/config")
        mine = [r for r in st.replies if not r.deleted and r.raw]
        if not mine:
            return None, None, b""
        stamp, rec = c2.read_put(self.layout, mine[-1].raw)
        return stamp, rec, mine[-1].raw

    def close(self) -> None:
        self.s.close()
        for h in self.hb.values():
            h.close()
        self.tokens.close()
        for o in list(self.owners):
            try:
                o.stop()
            except Exception:  # noqa: BLE001 - closing anyway
                pass
        for e in self.extra:
            try:
                e.close()
            except Exception:  # noqa: BLE001
                pass
        for x in (self.c_session, self.s_session, self.t, self.router):
            try:
                x.close()
            except Exception:  # noqa: BLE001
                pass


def _p(rec: dict[str, Any] | None, group: str, name: str) -> dict[str, Any]:
    if not rec:
        return {}
    return ((rec.get("groups") or {}).get(group) or {}).get("params", {}).get(name) or {}


def _pending(c: c2.Called) -> dict[str, Any] | None:
    return (c.state or {}).get("pending") if c.kind == "value" else None


def _held_applied(bus: Bus, since: int, address: str = "lab/rf0") -> bool:
    return any((r.get("pending") or {}).get("applied") for _, _, r in bus.records(address, since))


# -- §1 ------------------------------------------------------------------------------------

def section1(report: Report, pairing: str) -> None:
    from . import live

    sec = "§1 a hot set, read back, and a dry run"
    bus = Bus(pairing)
    try:
        seen: list[tuple[str, Any]] = []

        def got(key: str) -> None:
            if "lab/rf0/@zk/instance/" in key:
                try:
                    seen.append((key, bus.get(session=bus.t)))
                except Exception as e:  # noqa: BLE001
                    seen.append((key, (None, {"error": str(e)}, b"")))

        from .config_scenarios import Tokens

        bus.extra.append(Tokens(live.open_client(bus.endpoint), "zk2/lab/*/@zk/**",
                                on_put=lambda k: threading.Thread(target=got, args=(k,), daemon=True).start()))
        t_start = time.time_ns()
        bus.owner()
        _wait(lambda: bool(seen), 3.0)
        rec = seen[0][1][1] if seen else None
        rec_put = [x for x in bus.records(since=t_start)]
        tok_at = [r[0] for r in bus.tokens.of("lab/rf0/@zk/instance/") if r[1] == "put"]
        groups = (rec or {}).get("groups", {})
        ok1 = (rec is not None and rec.get("revision") == 0 and rec.get("pending") is None and rec.get("last") is None
               and rec.get("floor_s") == 10
               and all(groups[g].get("writable") == (g != "queue") for g in ("duty", "radio", "link", "sim", "queue"))
               and _p(rec, "sim", "pin") == {"source": "file"}
               and bool(rec_put) and bool(tok_at) and rec_put[0][0] <= min(tok_at))
        report.check(sec, "step 1: T's GET at the owner's instance token answers the record: revision 0, nothing "
                          "pending, no last change, floor_s 10, every settable group's writable bit but queue's, pin "
                          "with its source alone; put before the tokens (§2.3)",
                     ok1, f"record {rec and {k: rec[k] for k in ('revision', 'floor_s', 'pending', 'last')}}; "
                          f"pin {_p(rec, 'sim', 'pin')}; put at {rec_put and rec_put[0][0]}, token at {tok_at[:1]}")
        t2 = time.time_ns()
        r = bus.call("set/duty", {"fraction_ppm": 20000}, request_id="r1")
        time.sleep(0.5)
        puts = bus.records(since=t2)
        evs = bus.events(since=t2)
        st = r.state or {}
        p = _p(st, "duty", "fraction_ppm")
        tok = (st.get("last") or {}).get("token")
        ok2 = (r.kind == "value" and p == {"source": "runtime", "differs": True, "value": 20000}
               and st.get("revision") == 1 and (st.get("last") or {}).get("groups") == ["duty"]
               and st["groups"]["duty"].get("writable") is True and len(puts) == 1 and r.mark is not None
               and puts[0][1] is not None and puts[0][1].seq == r.mark.seq and puts[0][2] == st)
        ok2b = (len(evs) == 1 and evs[0][2]["outcome"] == "applied" and evs[0][2]["token"] == tok
                and evs[0][2]["edits"] == [{"group": "duty", "parameter": "fraction_ppm", "old": 10000, "new": 20000}]
                and evs[0][2].get("actor") == "ops" and r.mark is not None and evs[0][1].seq == r.mark.seq + 1
                and evs[0][0] >= puts[0][0])
        report.check(sec, "step 2: the value form: fraction_ppm 20000 at runtime, differs set, revision 1, a last "
                          "token, duty's last bit; its seq the put's S received before it; then one event applied, "
                          "that token, the edit 10000 -> 20000, actor ops, its seq one past the put's (§2.9, §2.17)",
                     ok2 and ok2b, f"{r.short()}; puts {[(s and s.seq) for _, s, _ in puts]}; events "
                                   f"{[(s.seq, e['outcome'], e.get('actor')) for _, s, e in evs]}")
        _, g, raw = bus.get()
        same = (pairing == "py-zenctl" and g == st) or (raw != b"" and r.raw != b"" and raw[1:] == r.raw[1:]
                                                       and g == st)
        report.check(sec, "step 3: Cfg answers the reply's record, byte for byte (the put's mutation is the reply's "
                          "value with its CH's form)" + (" (zenctl: its record read back)" if pairing == "py-zenctl"
                                                         else ""),
                     same, f"GET {raw.hex()} vs reply {r.raw.hex()}")
        t4 = time.time_ns()
        r4 = bus.call("set/duty", {"period_s": 60, "dry_run": True})
        time.sleep(0.4)
        held = bus.C.of("lab/rf0").held if isinstance(bus.C, PyC) else None
        ok4 = (r4.kind == "value" and (r4.state or {}).get("would_be") is True
               and _p(r4.state, "duty", "period_s") == {"source": "runtime", "differs": True, "value": 60}
               and r4.mark is not None and evs and r4.mark.seq == evs[0][1].seq and not bus.records(since=t4)
               and (held is None or held[1] == st))
        report.check(sec, "step 4: the dry run's record has would_be set and period_s 60 at runtime, under step 2's "
                          "event's seq; S receives no put; C still holds step 2's record (§2.9)",
                     ok4, f"{r4.short()}; held {None if held is None else held[0]}")
        mark = time.monotonic_ns()
        time.sleep(65.0)
        at = time.monotonic_ns()
        sub = bus.hb["lab/rf0"]
        beats = [h for h in sub.answers(mark)]
        arrivals = [mark] + [h.arrival_ns for h in beats]
        gaps = [(b - a) / NS for a, b in zip(arrivals, arrivals[1:])]
        key = f"zk2/lab/rf0/{IFACE}/state/config"
        later = [d for d in bus.s.of(key, time.time_ns() - (at - mark))]
        vouched = bool(beats) and all(key in ((h.answer.get("read") or {}).get("sections", {}).get("state/config")
                                              or {}).get("confirmed", []) for h in beats)
        v = sub.judge(key, at)
        report.check(sec, "step 5: in 65 s with no change, S receives no put or delete of Cfg, a heartbeat at least "
                          "every 30 s vouching for it, and judges it fresh past the horizon (§2.3, freshness.v2)",
                     not later and vouched and bool(gaps) and max(gaps) <= 30.5 and v.verdict == fr.FRESH,
                     f"{len(later)} puts or deletes, {len(beats)} heartbeats, vouched {vouched}, largest gap "
                     f"{max(gaps, default=0):.1f} s, judged {v.pair()}")
    finally:
        bus.close()


# -- §2 ------------------------------------------------------------------------------------

def section2(report: Report, pairing: str) -> None:
    sec = "§2 a reach set, confirmed"
    bus = Bus(pairing)
    try:
        o = bus.owner()
        before = o.device()["applies"]
        t0 = time.time_ns()
        r1 = bus.call("set/radio", {"frequency_hz": 869525000, "confirm_s": 120}, request_id="r1")
        at_reply = o.device()["applies"]
        pend = _pending(r1) or {}
        token = pend.get("token")
        ok = False
        if pend:
            aa, dl = pend["apply_at"], pend["deadline"]
            recv_ms = t0 // 1_000_000
            ok = (aa % 1000 == 0 and dl % 1000 == 0 and recv_ms + 1000 <= aa < recv_ms + 2000 + 500
                  and recv_ms + 120_000 <= dl < recv_ms + 121_000 + 500 and pend["applied"] is False
                  and pend["groups"] == ["radio"] and (r1.state or {})["groups"]["radio"]["writable"] is True)
        r2 = bus.call("confirm", {"token": token}, request_id="r2")
        puts0 = bus.records(since=t0)
        report.check(sec, "step 2: the confirm before apply_at is the error form, busy, under the first put's seq: "
                          "not yet applied (§2.9)",
                     r2.code == "busy" and r2.mark is not None and bool(puts0) and puts0[0][1] is not None
                     and r2.mark.seq == puts0[0][1].seq, f"{r2.short()}; first put {puts0 and puts0[0][1]}")
        _wait(lambda: _held_applied(bus, t0), 5.0)
        puts = bus.records(since=t0)
        dev = o.device()
        seq_ok = (len(puts) >= 2 and (puts[0][2].get("pending") or {}).get("apply_at") is not None
                  and (puts[1][2].get("pending") or {}).get("apply_at") is None
                  and _p(puts[1][2], "radio", "frequency_hz").get("value") == 869525000
                  and r1.mark is not None and puts[0][1].seq == r1.mark.seq)
        report.check(sec, "step 1: the record: the change pending, radio's pending bit, apply_at and the deadline in "
                          "whole seconds, 1 s and 120 s after the receipt, rounded up; before the device held the new "
                          "frequency; S receives the record put before the reply, then one with no apply_at and "
                          "frequency_hz 869525000",
                     ok and at_reply == before and dev["applies"] == before + 1
                     and dev["held"]["radio"]["frequency_hz"] == 869525000 and seq_ok,
                     f"{r1.short()}; applies {before}/{at_reply}/{dev['applies']}; puts "
                     f"{[(s and s.seq, (x.get('pending') or {}).get('apply_at')) for _, s, x in puts]}")
        t3 = time.time_ns()
        r3 = bus.call("confirm", {"token": token}, request_id="r3")
        time.sleep(0.5)
        evs = bus.events(since=t3)
        report.check(sec, "step 3: the record: nothing pending, the last token the change's; S receives one event, "
                          "confirmed, after the reply, actor ops and step 1's request_id (v1 §2.17)",
                     r3.kind == "value" and (r3.state or {}).get("pending") is None
                     and ((r3.state or {}).get("last") or {}).get("token") == token and len(evs) == 1
                     and evs[0][2]["outcome"] == "confirmed" and evs[0][2].get("actor") == "ops"
                     and evs[0][2].get("request_id") == "r1" and r3.mark is not None
                     and evs[0][1].seq == r3.mark.seq + 1, f"{r3.short()}; events {[e for _, _, e in evs]}")
        r4 = bus.call("confirm", {"token": token}, request_id="r4")
        report.check(sec, "step 4: the error form, not_found", r4.code == "not_found", r4.short())
    finally:
        bus.close()


# -- §3 ------------------------------------------------------------------------------------

def section3(report: Report, pairing: str) -> None:
    sec = "§3 a reach set rolled back at its deadline"
    bus = Bus(pairing, ("lab/rf0", "lab/rf1"))
    try:
        o = bus.owner()
        o1 = bus.owner("rf1")
        o1.fail_apply("the PLL did not lock")
        # 300 ms into a second
        while (time.time_ns() // 1_000_000) % 1000 > 300 or (time.time_ns() // 1_000_000) % 1000 < 280:
            time.sleep(0.002)
        t0u = time.time_ns()
        r1 = bus.call("set/radio", {"frequency_hz": 869525000, "confirm_s": 5})
        r1b = bus.call("set/radio", {"frequency_hz": 869525000, "confirm_s": 5}, address="lab/rf1")
        pend = _pending(r1) or {}
        recv = t0u // 1_000_000
        dl = pend.get("deadline", 0)
        report.check(sec, "step 1: the deadline is the next whole second after the receipt plus 10 s: 10.7 s after "
                          "it, never 10.3 s (§2.8)",
                     dl % 1000 == 0 and 10_000 < dl - recv <= 11_000, f"{r1.short()}; deadline - receipt "
                                                                       f"{dl - recv} ms")
        t_undo_due = dl / 1000
        while time.time() < t_undo_due + 1.5:
            time.sleep(0.05)
        evs = bus.events(since=t0u)
        puts = bus.records(since=t0u)
        _, g, _ = bus.get()
        r2 = bus.call("confirm", {"token": pend.get("token")})
        dev = o.device()
        undo_ok = True
        if not bus.rust and dev.get("undo_at"):
            undo_ok = abs(dev["undo_at"][0] / 1e9 - t_undo_due) <= 0.1
        last = puts[-1] if puts else (0, None, {})
        report.check(sec, "step 2: the undo at that instant (within 0.1 s, zk2py's device timing it); then S receives "
                          "the record, frequency_hz 868100000 again at file, revision 2, then one event rolled-back; "
                          "the confirm is not_found; the device saw exactly one undo",
                     undo_ok and len(evs) == 1 and evs[0][2]["outcome"] == "rolled-back"
                     and _p(last[2], "radio", "frequency_hz") == {"source": "file", "differs": False,
                                                                  "value": 868100000}
                     and last[2].get("revision") == 2 and last[0] <= evs[0][0] and g == last[2]
                     and r2.code == "not_found" and dev["undos"] == 1
                     and dev["held"]["radio"]["frequency_hz"] == 868100000,
                     f"undo at {dev.get('undo_at')} vs {t_undo_due}; events {[e['outcome'] for _, _, e in evs]}; "
                     f"last record revision {last[2].get('revision')}; confirm {r2.short()}; undos {dev['undos']}")
        evs1 = bus.events("lab/rf1", since=t0u)
        p1 = _pending(r1b) or {}
        aa = p1.get("apply_at", 0) * 1_000_000
        dev1 = o1.device()
        report.check(sec, "step 3: lab/rf1, its device refusing the apply at apply_at, publishes one event "
                          "rolled-back at apply_at, with no undo made (v1 §2.11)",
                     len(evs1) == 1 and evs1[0][2]["outcome"] == "rolled-back" and abs(evs1[0][0] - aa) < NS
                     and dev1["undos"] == 0 and evs1[0][2]["revision"] == 0,
                     f"{[(round((t - aa) / NS, 3), e['outcome'], e['revision']) for t, _, e in evs1]}, "
                     f"undos {dev1['undos']}")
    finally:
        bus.close()


# -- §4 ------------------------------------------------------------------------------------

def section4(report: Report, pairing: str) -> None:
    sec = "§4 an undo that fails"
    bus = Bus(pairing)
    try:
        o = bus.owner()
        o.fail_undo()
        t0 = time.monotonic()
        t0u = time.time_ns()
        r1 = bus.call("set/radio", {"frequency_hz": 869525000, "confirm_s": 10})
        dl = (_pending(r1) or {}).get("deadline", 0) * 1_000_000
        _wait(lambda: bool(bus.events(since=t0u)), 15.0)
        time.sleep(0.2)
        evs = bus.events(since=t0u)
        puts = bus.records(since=t0u)
        report.check(sec, "step 1: at the deadline S receives the record, then one event, partial",
                     len(evs) == 1 and evs[0][2]["outcome"] == "partial" and abs(evs[0][0] - dl) < NS
                     and bool(puts) and puts[-1][0] <= evs[0][0] and puts[-1][2].get("pending") is None,
                     f"{[(round((t - dl) / NS, 3), e['outcome']) for t, _, e in evs]}")
        while time.monotonic() < t0 + 20:
            time.sleep(0.1)
        _, g, _ = bus.get()
        dev = o.device()
        report.check(sec, "step 2: 20 s after the call, the record shows frequency_hz with neither source nor value, "
                          "the device reporting none; nothing pending; exactly one undo, none in the 10 s after it",
                     g is not None and _p(g, "radio", "frequency_hz") == {"source": None, "differs": False,
                                                                          "value": None}
                     and g.get("pending") is None and dev["undos"] == 1,
                     f"frequency_hz {_p(g, 'radio', 'frequency_hz')}, undos {dev['undos']}")
    finally:
        bus.close()


# -- §5 ------------------------------------------------------------------------------------

def section5(report: Report, pairing: str) -> None:
    from . import live

    sec = "§5 a join, and a second writer"
    bus = Bus(pairing)
    c2sess = live.open_client(bus.endpoint)
    bus.extra.append(c2sess)
    second = PyC(c2sess, bus.layout) if pairing != "py-zenctl" else bus.C
    try:
        bus.owner()
        t0u = time.time_ns()
        r1 = bus.call("set/radio", {"frequency_hz": 869525000, "confirm_s": 120}, request_id="r1")
        p1 = _pending(r1) or {}
        token = p1.get("token")
        report.check(sec, "step 1: the record, pending", token is not None, r1.short())
        r2 = second.call("lab/rf0", "set/link", {"net_id": 7, "confirm_s": 60}, actor="ops2", request_id="r2")
        _, g, _ = bus.get(session=c2sess)
        report.check(sec, "step 2: C2's set is the error form, busy; C2 reads the token from the record it GETs",
                     r2.code == "busy" and ((g or {}).get("pending") or {}).get("token") == token,
                     f"{r2.short()}; GET pending {(g or {}).get('pending')}")
        r3 = bus.call("set/link", {"net_id": 7, "token": token}, request_id="r3")
        r3b = bus.call("set/duty", {"fraction_ppm": 20000, "token": token}, request_id="r4")
        p3, p3b = _pending(r3) or {}, _pending(r3b) or {}
        report.check(sec, "step 3: two records: the same token and deadline, link's apply_at; then duty, radio and "
                          "link's pending bits, in the contract's order",
                     p3.get("token") == token and p3.get("deadline") == p1.get("deadline")
                     and p3.get("apply_at") is not None and p3.get("apply_at") >= p1.get("apply_at", 0)
                     and p3.get("groups") == ["radio", "link"]
                     and p3b.get("groups") == ["duty", "radio", "link"] and p3b.get("token") == token,
                     f"{r3.short()}; {r3b.short()}")
        _wait(lambda: _held_applied(bus, t0u), 5.0)
        t4 = time.time_ns()
        r4 = bus.call("confirm", {"token": token}, request_id="r5")
        time.sleep(0.5)
        evs = bus.events(since=t0u)
        edits = evs[0][2]["edits"] if evs else []
        report.check(sec, "step 4: one event, confirmed, its edits duty's, radio's, then link's: the contract's "
                          "order, not the join's; none at either join (§2.17)",
                     r4.kind == "value" and len(evs) == 1 and evs[0][0] >= t4 and evs[0][2]["outcome"] == "confirmed"
                     and [e["group"] for e in edits] == ["duty", "radio", "link"],
                     f"{r4.short()}; events {[e for _, _, e in evs]}")
    finally:
        bus.close()


# -- §6 ------------------------------------------------------------------------------------

def section6(report: Report, pairing: str) -> None:
    sec = "§6 an extend"
    bus = Bus(pairing)
    try:
        bus.owner()
        t0u = time.time_ns()
        r1 = bus.call("set/duty", {"fraction_ppm": 20000, "confirm_s": 30})
        p1 = _pending(r1) or {}
        recv = t0u // 1_000_000
        report.check(sec, "step 1: the record, the change applied and pending, the deadline 30 s after the call",
                     p1.get("applied") is True and 30_000 <= p1.get("deadline", 0) - recv <= 31_500, r1.short())
        while time.time_ns() < t0u + 20_400_000_000:
            time.sleep(0.01)
        t2u = time.time_ns()
        r2 = bus.call("extend", {"token": p1.get("token"), "confirm_s": 60})
        p2 = _pending(r2) or {}
        time.sleep(0.3)
        dl2 = p2.get("deadline", 0)
        report.check(sec, "step 2: the record, its deadline the extend's receipt plus 60 s, rounded up (§2.8); no "
                          "event",
                     dl2 % 1000 == 0 and 60_000 <= dl2 - t2u // 1_000_000 < 61_000 and not bus.events(since=t0u),
                     f"{r2.short()}; deadline - step 1 {(dl2 - recv) / 1000:.3f} s")
        _wait(lambda: bool(bus.events(since=t0u)), 75.0, 0.1)
        evs = bus.events(since=t0u)
        at = (evs[0][0] - t0u) / NS if evs else None
        # Text 0.2 (F-490): "That is 80.4 s to 81.4 s after step 1, by its
        # phase", and the event "at that deadline, ± 0.5 s"; the extend's
        # receipt comes a few ms after C sends it.
        report.check(sec, "step 3: the undo at the new deadline, not the first: the rolled-back event at the record's "
                          "deadline, ± 0.5 s, that deadline 80.4 s to 81.4 s after step 1 by its phase",
                     len(evs) == 1 and evs[0][2]["outcome"] == "rolled-back" and at is not None
                     and abs(at - (dl2 - recv) / 1000) <= 0.5 and 80.4 <= (dl2 - recv) / 1000 <= 81.6,
                     f"event at {at and round(at, 3)} s; deadline - step 1 {(dl2 - recv) / 1000:.3f} s")
    finally:
        bus.close()


# -- §7 ------------------------------------------------------------------------------------

def section7(report: Report, pairing: str) -> None:
    sec = "§7 tokens, and a restart during a pending change"
    bus = Bus(pairing)
    try:
        o = bus.owner()
        t0 = time.time_ns()
        r1 = bus.call("set/radio", {"frequency_hz": 869525000, "confirm_s": 120})
        p1 = _pending(r1) or {}
        token = p1.get("token")
        _wait(lambda: _held_applied(bus, t0), 5.0)
        report.check(sec, "step 1: the record, pending; the device holds 869525000; the token's high 32 bits are the "
                          "record's instance",
                     token is not None and o.device()["held"]["radio"]["frequency_hz"] == 869525000
                     and token >> 16 == (r1.state or {}).get("instance"), r1.short())
        old = o.instance
        t2 = time.time_ns()
        o.restart()
        new = o.instance
        time.sleep(0.6)
        withdrawn = [r for r in bus.tokens.of(f"/@zk/instance/{old}") if r[1] == "delete"]
        declared = [r for r in bus.tokens.of(f"/@zk/instance/{new}") if r[1] == "put"]
        recs = bus.records(since=t2)
        first = recs[0] if recs else (0, None, {})
        rec = first[2]
        tok_at = [r[0] for r in declared]
        report.check(sec, "step 2: S sees the old instance token withdrawn and a new one declared; the new record, "
                          "put before its tokens, another instance number, revision 0, nothing pending, no last "
                          "change, frequency_hz 868100000 at file; the device holds the startup frequency; no event "
                          "for the old change (§2.12)",
                     bool(withdrawn) and bool(declared) and new != old and rec.get("instance") not in (
                         None, (r1.state or {}).get("instance")) and rec.get("revision") == 0
                     and rec.get("pending") is None and rec.get("last") is None
                     and _p(rec, "radio", "frequency_hz") == {"source": "file", "differs": False,
                                                              "value": 868100000}
                     and bool(tok_at) and first[0] <= min(tok_at)
                     and o.device()["held"]["radio"]["frequency_hz"] == 868100000 and not bus.events(since=t2),
                     f"withdrawn {len(withdrawn)}, declared {len(declared)}, record instance {rec.get('instance')}, "
                     f"events {len(bus.events(since=t2))}")
        r3 = bus.call("confirm", {"token": token})
        report.check(sec, "step 3: the error form, not_found: a token the new instance never minted (§2.6)",
                     r3.code == "not_found", r3.short())
        counters = []
        for _ in range(2):
            o.restart()
            time.sleep(0.3)
            r = bus.call("set/duty", {"fraction_ppm": 11000 + len(counters)})
            counters.append(((r.state or {}).get("last") or {}).get("token", 0) & 0xFFFF)
        r = bus.call("set/duty", {"fraction_ppm": 12000})
        counters.insert(0, ((r.state or {}).get("last") or {}).get("token", 0) & 0xFFFF)
        report.check(sec, "step 4: the token counters after three starts are not all equal: each run's counter "
                          "starts at random (§2.6)", len(set(counters)) > 1, f"counters {counters}")
        t5 = time.time_ns()
        run_hex = o.counter(65534)
        toks = []
        marks = []
        for i in range(3):
            r = bus.call("set/duty", {"fraction_ppm": 13000 + i})
            toks.append(((r.state or {}).get("last") or {}).get("token"))
            marks.append(r.mark)
        time.sleep(0.4)
        descs = [d for d in bus.s.of("lab/rf0/@zk/instance/", t5) if d.kind == "put"]
        runs = []
        for d in descs:
            try:
                runs.append(json.loads(d.payload).get("run"))
            except ValueError:
                pass
        recs = bus.records(since=t5)
        ok = (all(t is not None for t in toks) and toks[0] >> 16 == toks[1] >> 16 and toks[0] & 0xFFFF == 65534
              and toks[1] & 0xFFFF == 65535 and toks[2] >> 16 != toks[0] >> 16
              and marks[2] is not None and marks[2].run == toks[2] >> 16 and f"{toks[2] >> 16:08x}" in runs
              and len({r.get("instance") for _, _, r in recs}) == 1)
        report.check(sec, "step 5: the hook starts the counter at 65,534: the first two tokens share a run, counters "
                          "65,534 and 65,535; the third's run is another, which S sees the descriptor state, and the "
                          "third reply's run is it; every record keeps one instance",
                     ok, f"hook run {run_hex}; tokens {[hex(t) if t else t for t in toks]}; marks "
                         f"{[str(m) for m in marks]}; descriptor runs {runs}; instances "
                         f"{sorted({r.get('instance') for _, _, r in recs})}")
    finally:
        bus.close()


# -- §8 ------------------------------------------------------------------------------------

def section8(report: Report, pairing: str) -> None:
    from . import acl, live
    from .config_scenarios import _auth_router
    from .live_interop import _r1

    sec = "§8 persist, granted and not"
    k = contract()
    both = ["lab/rf0", "lab/rf2"]
    reads = acl.Use(IFACE, both, ["state/config"])
    acts = ["@op/set/duty", "@op/set/radio", "@op/confirm", "@op/cancel", "@op/extend"]
    principals = [
        acl.Principal("ops", "ops", consumes=[reads], calls=[acl.Use(IFACE, both, acts)]),
        acl.Principal("admin", "admin", consumes=[reads], calls=[acl.Use(IFACE, both, [*acts, "@op/persist"])]),
        acl.Principal("S", "s", consumes=[acl.Use(IFACE, ["lab/rf0"], ["state/config", "events/change"])]),
    ]
    r0 = None
    if pairing != "rust-py":
        principals += [acl.Principal("rf0", "rf0", service="lab/rf0", implements=[k]),
                       acl.Principal("rf2", "rf2", service="lab/rf2", implements=[k])]
        block = acl.generate(principals, "deny").block
        router, ep = _auth_router({p.user: f"{p.user}-pw" for p in principals}, block)
        owner_ep = ep
    else:
        r0, owner_ep, _ = _r1()
        block = {"enabled": True, "default_permission": "allow",
                 "rules": [{"id": "no-persist", "messages": ["query"], "flows": ["ingress"], "permission": "deny",
                            "key_exprs": [f"zk2/lab/*/{IFACE}/@op/persist"]}],
                 "subjects": [{"id": "ops", "usernames": ["ops"]}],
                 "policies": [{"id": "ops-policy", "rules": ["no-persist"], "subjects": ["ops"]}]}
        router, ep = _auth_router({p.user: f"{p.user}-pw" for p in principals}, block, connect=owner_ep)
    admin = live.open_client(ep, ("admin", "admin-pw"))
    ops = live.open_client(ep, ("ops", "ops-pw"))
    s_sess = live.open_client(ep, ("s", "s-pw"))
    tmp = tempfile.mkdtemp(prefix="zk2py-cfgv2-")
    zconf = {}
    for who in ("admin", "ops"):
        path = Path(tmp) / f"{who}.json5"
        path.write_text(json.dumps({"transport": {"auth": {"usrpwd": {"user": who, "password": f"{who}-pw"}}}}))
        zconf[who] = str(path)
    base = f"zk2/lab/rf0/{IFACE}"
    bus = Bus(pairing, router=(router, ep), owner_endpoint=owner_ep,
              sessions={"C": admin, "S": s_sess, "T": live.open_client(ep, ("ops", "ops-pw"))},
              zenctl_extra=["--zenoh-config", zconf["admin"]],
              selectors=[f"{base}/state/config", f"{base}/events/change/*"])
    bus.extra.append(ops)
    if r0 is not None:
        bus.extra.append(r0)
    as_ops = PyC(ops, bus.layout) if pairing != "py-zenctl" else ZenctlC(ep, ["--zenoh-config", zconf["ops"]])
    try:
        auth = {} if bus.rust else {"auth": ("rf0", "rf0-pw")}
        o = bus.owner(**auth)
        bus.owner("rf2", persist=False, **({} if bus.rust else {"auth": ("rf2", "rf2-pw")}))
        time.sleep(0.5)
        t0 = time.time_ns()
        r1 = bus.call("set/duty", {"fraction_ppm": 20000}, actor="admin")
        tok = ((r1.state or {}).get("last") or {}).get("token")
        time.sleep(0.3)
        t1 = time.time_ns()
        r1p = bus.call("persist", {"token": tok}, actor="admin")
        time.sleep(0.5)
        report.check(sec, "step 1: as admin, both answered with the record; the persist's shows fraction_ppm at "
                          "overlay, differs clear; no event for the persist",
                     r1.kind == "value" and r1p.kind == "value"
                     and _p(r1p.state, "duty", "fraction_ppm") == {"source": "overlay", "differs": False,
                                                                   "value": 20000}
                     and len(bus.events(since=t0)) == 1 and not bus.events(since=t1),
                     f"{r1.short()}; {r1p.short()}; events {len(bus.events(since=t0))}")
        r2 = as_ops.call("lab/rf0", "set/duty", {"fraction_ppm": 30000})
        tok2 = ((r2.state or {}).get("last") or {}).get("token")
        r2p = as_ops.call("lab/rf0", "persist", {"token": tok2})
        report.check(sec, "step 2: as ops, the set answered; the persist draws nothing before its timeout: silence "
                          "(core O5), never not_found",
                     r2.kind == "value" and r2p.kind == "silent", f"{r2.short()}; persist {r2p.short()}")
        bus.call("set/duty", {"fraction_ppm": 20000}, address="lab/rf2", actor="admin")
        _, g2, _ = bus.get("lab/rf2", session=admin)
        tok3 = ((g2 or {}).get("last") or {}).get("token", 1)
        r3 = bus.call("persist", {"token": tok3}, address="lab/rf2", actor="admin")
        ds = [d for d in bus.s.rows if False]
        desc = None
        rf2 = [x for x in bus.owners if getattr(x, "service", None) == "rf2" or (
            getattr(x, "instance_key", "").startswith("zk2/lab/rf2/"))]
        if rf2:
            d = live.get_descriptor(admin, rf2[0].instance_key)
            desc = json.loads(d[0].payload) if d and d[0].ok else None
        entry = next((e for e in (desc or {}).get("interfaces") or [] if e.get("iface") == IFACE), {})
        report.check(sec, "step 3: as admin on lab/rf2, the error form, unavailable, cause config; its descriptor "
                          "lists persist unavailable with cause config (core §3.3)",
                     r3.code == "unavailable" and (r3.error or {}).get("cause") == "config"
                     and any(u.get("resource") == "@op/persist" and u.get("cause") == "config"
                             for u in entry.get("unavailable", [])),
                     f"{r3.short()}; descriptor unavailable {entry.get('unavailable')} {len(ds)}")
        o.restart()
        _, g, _ = bus.get(session=admin)
        report.check(sec, "step 4: fraction_ppm is 20000 at overlay: step 1's change survived, step 2's did not",
                     _p(g, "duty", "fraction_ppm") == {"source": "overlay", "differs": False, "value": 20000},
                     f"fraction_ppm {_p(g, 'duty', 'fraction_ppm')}")
    finally:
        bus.close()


# -- §9 ------------------------------------------------------------------------------------

def section9(report: Report, pairing: str) -> None:
    sec = "§9 a group outside the allowlist"
    bus = Bus(pairing)
    try:
        bus.owner()
        _, g, _ = bus.get()
        groups = (g or {}).get("groups", {})
        report.check(sec, "step 1: every settable group but queue has its writable bit set",
                     bool(groups) and all(groups[n].get("writable") == (n != "queue")
                                          for n in ("duty", "radio", "link", "sim", "queue"))
                     and "writable" not in groups.get("modulation", {"writable": 1}),
                     f"{[(n, x.get('writable')) for n, x in groups.items()]}")
        t0 = time.time_ns()
        r2 = bus.call("set/queue", {"tx_queue_len": 10})
        time.sleep(0.3)
        _, g2, _ = bus.get()
        report.check(sec, "step 2: the error form, unavailable, cause config; nothing changed; no event",
                     r2.code == "unavailable" and (r2.error or {}).get("cause") == "config" and g2 == g
                     and not bus.events(since=t0), r2.short())
        if pairing == "py-zenctl":
            z = bus.C.run("set", "lab/rf0", "modulation", "spreading_factor=8", "--i-know", "--no-precheck",
                          timeout_s=1.0)
            report.check(sec, "step 3: zenctl refuses set modulation before sending: the contract has no "
                              "set/modulation (exit 2, nothing sent)", z["exit"] == 2, z["err"][-200:])
        else:
            r3 = bus.C.of("lab/rf0").call("set/modulation", raw=b"\x40", timeout=1.0)
            report.check(sec, "step 3: nothing before the timeout: no set/modulation queryable; C reports silence "
                              "(§2.4, core O5)", r3.kind == "silent", r3.short())
    finally:
        bus.close()


# -- §10 -----------------------------------------------------------------------------------

def section10(report: Report, pairing: str) -> None:
    sec = "§10 an idempotent retry"
    bus = Bus(pairing)
    try:
        o = bus.owner()
        req = {"frequency_hz": 869525000, "confirm_s": 60, "idempotency_key": 1001}
        t0 = time.monotonic()
        t0u = time.time_ns()
        first: bytes | None = None
        if bus.rust:
            r1 = bus.call("set/radio", req)
            first = r1.raw or None
            armed = (bus.get()[1] or {}).get("pending") is not None
            report.check(sec, "step 1: the owner arms the change (the owner example cannot drop a reply: C sets it "
                              "aside unread); S receives its record", r1.kind == "value" and armed
                         and bool(bus.records(since=t0u)), f"{r1.short()}, pending {armed}")
        else:
            o.drop_next_reply()
            r1 = bus.call("set/radio", req)
            armed = (bus.get()[1] or {}).get("pending") is not None
            first = (o.dropped() or [None])[0]
            report.check(sec, "step 1: the owner arms the change; C receives nothing; S receives its record",
                         r1.kind == "silent" and armed and bool(bus.records(since=t0u)), f"{r1.short()}, armed {armed}")
        while time.monotonic() < t0 + 5:
            time.sleep(0.05)
        r2 = bus.call("set/radio", req)
        time.sleep(0.3)
        applies = o.device()["applies"]
        puts = bus.records(since=t0u)
        applied_seen = any((r.get("pending") or {}).get("applied") for _, _, r in puts)
        p2 = _pending(r2) or {}
        same = (r2.raw == first) if pairing != "py-zenctl" else None
        fst = None
        if first is not None and pairing != "py-zenctl":
            env = zk2c.parse(first, "reply")
            fst = env.stamp
        report.check(sec, "step 2: the reply is step 1's, byte for byte: its run and seq the first put's, the change "
                          "pending and not applied though S has received the applied record since; one apply, nothing "
                          "new published (§2.14)" + (" (zenctl: its record and mark compared)" if same is None else ""),
                     r2.kind == "value" and p2.get("applied") is False and applied_seen and applies == 1
                     and not bus.events(since=t0u) and (same is True or (same is None and r2.mark is not None
                                                                          and r2.mark.seq == puts[0][1].seq))
                     and (fst is None or (puts and puts[0][1] is not None and fst.seq == puts[0][1].seq)),
                     f"{r2.short()}; first {None if first is None else first.hex()}")
        r3 = bus.call("set/radio", {**req, "frequency_hz": 869000000})
        report.check(sec, "step 3: the error form, invalid_request, under the current mark",
                     r3.code == "invalid_request" and r3.mark is not None and r3.mark.seq >= puts[-1][1].seq,
                     f"{r3.short()}; last put {puts and puts[-1][1]}")
        bus.call("cancel", {"token": p2.get("token")})
        o.move_clock(300_000)
        r4 = bus.call("set/radio", req)
        p4 = _pending(r4) or {}
        report.check(sec, "step 4: after a cancel, 300 s on (the owner's clock moved), step 1's request draws a new "
                          "pending record with a new token: the key forgotten at 300 s",
                     bool(p4) and p4.get("token") not in (None, p2.get("token")), r4.short())
    finally:
        bus.close()


# -- §11 -----------------------------------------------------------------------------------

def section11(report: Report, pairing: str) -> None:
    sec = "§11 sensitive values"
    bus = Bus(pairing)
    try:
        o = bus.owner()
        t0 = time.time_ns()
        r1 = bus.call("set/sim", {"pin": 1234})
        tok1 = ((r1.state or {}).get("last") or {}).get("token")
        time.sleep(0.2)
        t1 = time.time_ns()
        r1b = bus.call("set/sim", {"pin": 9999})
        tok2 = ((r1b.state or {}).get("last") or {}).get("token")
        time.sleep(0.4)
        evs = bus.events(since=t0)
        raws = bus.raw_records(since=t0)
        recs = bus.records(since=t0)
        diff_ok = False
        if len(recs) >= 2:
            a, b = dict(recs[0][2]), dict(recs[1][2])
            a.pop("revision"), b.pop("revision"), a.pop("last"), b.pop("last")
            diff_ok = a == b
        report.check(sec, "step 1: both applied; the two records S receives differ only in their revision and last "
                          "token; each event's edit is {sim, pin, redacted}",
                     r1.kind == "value" and r1b.kind == "value" and diff_ok and len(evs) == 2
                     and all(e[2]["edits"] == [{"group": "sim", "parameter": "pin", "redacted": True}] for e in evs),
                     f"{r1.short()}; {r1b.short()}; edits {[e['edits'] for _, _, e in evs]}; {len(raws)} puts")
        o.fail_apply("PIN 4321 rejected by the SIM")
        r2 = bus.call("set/sim", {"pin": 4321, "attempts": 5})
        report.check(sec, "step 2: the error form, app, an Error detail device_refused, with no message",
                     r2.code == "app" and (r2.error or {}).get("app") == "device_refused"
                     and not (r2.error or {}).get("message"), r2.short())
        _, g, graw = bus.get()
        r3a = bus.call("persist", {"token": tok1})
        r3b = bus.call("persist", {"token": tok2})
        report.check(sec, "step 3: the record shows pin with source runtime and nothing more; the first persist is "
                          "not_found, the last change being the second; the second is forbidden",
                     _p(g, "sim", "pin") == {"source": "runtime"} and r3a.code == "not_found"
                     and r3b.code == "forbidden", f"pin {_p(g, 'sim', 'pin')}; {r3a.short()}; {r3b.short()}")
        log = o.log()
        heard = [d.payload for d in bus.s.rows]
        replies = [x.raw for x in (r1, r1b, r2, r3a, r3b)] + [graw]
        bad = [w for w in (b"1234", b"9999", b"4321") if any(w in x for x in heard + replies)]
        sim_lines = [line for line in log if "sim" in line and ("request" in line or "pin" in line)]
        report.check(sec, "step 4: no log line holds 1234, 9999 or 4321; a logged request writes \"<redacted>\"; no "
                          "delivery S recorded and no reply C received holds them",
                     not any(w in line for line in log for w in ("1234", "9999", "4321")) and not bad
                     and any(c2.REDACTED in line for line in sim_lines),
                     f"{len(log)} log lines, {len(sim_lines)} of sim; in the wire: {bad}")
    finally:
        bus.close()


# -- §12 -----------------------------------------------------------------------------------

def section12(report: Report, pairing: str) -> None:
    sec = "§12 desired state refused"
    bus = Bus(pairing)
    try:
        o = bus.owner()
        t0 = time.time_ns()
        o.desire({"revision": 1, "values": {"duty": {"fraction_ppm": 20000}}})
        _wait(lambda: any((r.get("desired") or {}).get("observed_revision") == 1 for _, _, r in bus.records(since=t0)),
              3.0)
        time.sleep(0.3)
        recs = bus.records(since=t0)
        evs = bus.events(since=t0)
        conv = next((x for x in recs if (x[2].get("desired") or {}).get("observed_revision") == 1), None)
        val = next((x for x in recs if _p(x[2], "duty", "fraction_ppm").get("value") == 20000), None)
        # Text 0.2 (F-482): §2.18 "After the acts. The report changes in a put
        # of its own … Each converging change's put and its event come
        # before it", and §12 expected 1: the record, the event, then the
        # next record with the report.
        order = (val is not None and conv is not None and len(evs) == 1 and val[1] is not None
                 and conv[1] is not None and evs[0][1] is not None
                 and val[1].seq < evs[0][1].seq < conv[1].seq
                 and [x for x in recs if x[1] is not None and val[1].seq < x[1].seq < conv[1].seq] == [])
        report.check(sec, "step 1: S receives the record, fraction_ppm 20000, then one event, applied; then the next "
                          "record, its report desired converged at revision 1 (§2.18)",
                     order and (val[2].get("desired") or {}).get("observed_revision") != 1
                     and _p(conv[2], "duty", "fraction_ppm").get("value") == 20000
                     and conv[2]["desired"] == {"outcome": "converged", "observed_revision": 1, "refusal": None}
                     and evs[0][2]["outcome"] == "applied",
                     f"records {[(st and st.seq, _p(r, 'duty', 'fraction_ppm').get('value'), r.get('desired')) for _, st, r in recs]}; "
                     f"events {[(st and st.seq, e['outcome']) for _, st, e in evs]}")
        before = o.device()["applies"]
        t2 = time.time_ns()
        o.desire({"revision": 2, "values": {"duty": {"fraction_ppm": 30000}, "radio": {"frequency_hz": 869525000}}})
        _wait(lambda: any((r.get("desired") or {}).get("observed_revision") == 2 for _, _, r in bus.records(since=t2)),
              3.0)
        rec2 = next((r for _, _, r in bus.records(since=t2) if (r.get("desired") or {}).get("observed_revision") == 2),
                    None)
        f2 = c2.state_fields(bus.layout, rec2) if rec2 else {}
        report.check(sec, "step 2: no act and no event; desired refused at revision 2, desired_code reach, "
                          "desired_group 1 (radio), no parameter index; fraction_ppm still 20000",
                     rec2 is not None and f2.get("desired") == "refused" and f2.get("desired_code") == "reach"
                     and f2.get("desired_group") == 1 and "desired_parameter" not in f2
                     and _p(rec2, "duty", "fraction_ppm").get("value") == 20000
                     and o.device()["applies"] == before and not bus.events(since=t2),
                     f"{ {k: v for k, v in f2.items() if k.startswith('desired')} }")
        t3 = time.time_ns()
        o.desire({"revision": 3, "values": {"sim": {"pin": 1234}}})
        _wait(lambda: any((r.get("desired") or {}).get("observed_revision") == 3 for _, _, r in bus.records(since=t3)),
              3.0)
        rec3 = next((r for _, _, r in bus.records(since=t3) if (r.get("desired") or {}).get("observed_revision") == 3),
                    None)
        f3 = c2.state_fields(bus.layout, rec3) if rec3 else {}
        report.check(sec, "step 3: desired_code sensitive, desired_group 3 and desired_parameter 0, sim's pin; no act",
                     f3.get("desired_code") == "sensitive" and f3.get("desired_group") == 3
                     and f3.get("desired_parameter") == 0 and o.device()["applies"] == before,
                     f"{ {k: v for k, v in f3.items() if k.startswith('desired')} }")
    finally:
        bus.close()


# -- §13 -----------------------------------------------------------------------------------

def section13(report: Report, pairing: str) -> None:
    import zenoh

    from . import live
    from .config_scenarios import Tokens, _auth_router
    from .live_interop import _r1

    sec = "§13 the record across a constrained face"
    every = ["put", "delete", "declare_subscriber", "query", "declare_queryable", "reply", "liveliness_token",
             "declare_liveliness_subscriber", "liveliness_query"]
    # The face: C's principal denied, both ways, every @zk key and every key of
    # the service but the record, its heartbeat and the four operations (a
    # deny list under allow, as config.v1's §13; the heartbeat key is
    # another interface's, freshness.v2's, so it is not denied).
    denied = ["zk2/**/@zk/**", f"zk2/lab/rf0/{IFACE}/events/**", f"zk2/lab/rf0/{IFACE}/@op/persist",
              *(f"zk2/lab/rf0/{IFACE}/@op/set/{g}" for g in ("duty", "link", "sim", "queue"))]
    block = {"enabled": True, "default_permission": "allow",
             "rules": [{"id": "face", "messages": every, "flows": ["ingress", "egress"], "permission": "deny",
                        "key_exprs": denied}],
             "subjects": [{"id": "far", "usernames": ["far"]}],
             "policies": [{"id": "the-face", "rules": ["face"], "subjects": ["far"]}]}
    r0 = None
    if pairing != "rust-py":
        router, ep = _auth_router({"far": "far-pw", "owner": "owner-pw"}, block)
        owner_ep = ep
        near = live.open_client(ep, ("owner", "owner-pw"))
        t_sess = live.open_client(ep, ("owner", "owner-pw"))
    else:
        r0, owner_ep, _ = _r1()
        router, ep = _auth_router({"far": "far-pw", "owner": "owner-pw"}, block, connect=owner_ep)
        near = live.open_client(owner_ep)
        t_sess = live.open_client(owner_ep)
    cconf = zenoh.Config()
    cconf.insert_json5("mode", json.dumps("client"))
    cconf.insert_json5("connect/endpoints", json.dumps([ep]))
    cconf.insert_json5("scouting/multicast/enabled", "false")
    cconf.insert_json5("transport/auth/usrpwd", json.dumps({"user": "far", "password": "far-pw"}))
    cconf.insert_json5("transport/link/tx/batch_size", "1024")
    far = zenoh.open(cconf)
    tmp = tempfile.mkdtemp(prefix="zk2py-cfgv2-face-")
    zpath = Path(tmp) / "far.json5"
    zpath.write_text(json.dumps({"transport": {"auth": {"usrpwd": {"user": "far", "password": "far-pw"}},
                                               "link": {"tx": {"batch_size": 1024}}}}))
    bus = Bus(pairing, router=(router, ep), owner_endpoint=owner_ep, sessions={"C": far, "S": near, "T": t_sess},
              zenctl_extra=["--zenoh-config", str(zpath), "--across-face", "--iface", IFACE])
    if r0 is not None:
        bus.extra.append(r0)
    try:
        crossed = Tokens(far, "zk2/lab/rf0/@zk/**")
        bus.extra.append(crossed)
        far_rec = c2.Recorder(far, [f"zk2/lab/rf0/{IFACE}/state/config"])
        bus.extra.append(far_rec)
        o = bus.owner(**({} if bus.rust else {"auth": ("owner", "owner-pw")}))
        time.sleep(0.6)
        if pairing == "py-zenctl":
            z = bus.C.run("get", "lab/rf0", timeout_s=2.0)
            doc = z["out"] if isinstance(z["out"], dict) else {}
            rec1 = (doc.get("record") or {}).get("record")
            st1 = None
        else:
            st1, rec1, _ = bus.get(session=far)
        time.sleep(0.5)
        report.check(sec, "step 1: the record across the face, with floor_s; no @zk delivery crosses"
                          + (" (zenctl config get --across-face --iface)" if pairing == "py-zenctl" else ""),
                     rec1 is not None and rec1.get("floor_s") == 10 and not crossed.rows,
                     f"floor_s {(rec1 or {}).get('floor_s')}; {len(crossed.rows)} @zk deliveries")
        rev = (rec1 or {}).get("revision", 0)
        t2 = time.time_ns()
        gets_before = len(bus.s.rows)
        r2 = bus.call("set/radio", {"frequency_hz": 869525000, "confirm_s": 120, "expected_revision": rev,
                                    "idempotency_key": 0x3D2C1B0A})
        p2 = _pending(r2) or {}
        _wait(lambda: any((r.get("pending") or {}).get("applied") for r in
                          [c2.read_put(bus.layout, d.payload)[1] for d in far_rec.of("state/config", t2)]), 5.0)
        pushes = [c2.read_put(bus.layout, d.payload)[1] for d in far_rec.of("state/config", t2)]
        applied = next((x for x in pushes if (x.get("pending") or {}).get("applied")), None)
        report.check(sec, "step 2: the request at most 16 bytes of payload, the reply at most 64, the record pending; "
                          "the applied record by the push, no GET after the reply",
                     r2.kind == "value" and len(r2.request) <= 16 and 0 < len(r2.raw) <= 64 and bool(p2)
                     and applied is not None, f"{r2.short()}; request {len(r2.request)} B, reply {len(r2.raw)} B; "
                                              f"pushes "
                                              f"{[(x.get('pending') or {}).get('applied') for x in pushes]}; "
                                              f"{len(bus.s.rows) - gets_before} deliveries at S")
        r3 = bus.call("confirm", {"token": p2.get("token", 0)})
        report.check(sec, "step 3: the record, at most 64 bytes, nothing pending, the last token the change's",
                     r3.kind == "value" and 0 < len(r3.raw) <= 64 and (r3.state or {}).get("pending") is None
                     and ((r3.state or {}).get("last") or {}).get("token") == p2.get("token"),
                     f"{len(r3.raw)} B: {r3.short()}")
        time.sleep(0.5)  # the confirm's own push, put before its reply, delivered first
        t4 = time.time_ns()
        o.restart()
        _wait(lambda: bool(far_rec.of("state/config", t4)), 5.0)
        nxt = [c2.read_put(bus.layout, d.payload)[1] for d in far_rec.of("state/config", t4)]
        report.check(sec, "step 4: the next push's instance differs from step 1's: C learns of the restart without "
                          "presence (§2.12)",
                     bool(nxt) and rec1 is not None and nxt[0].get("instance") != rec1.get("instance")
                     and not crossed.rows, f"{(rec1 or {}).get('instance')} -> {nxt and nxt[0].get('instance')}")
        del st1
    finally:
        bus.close()


# -- §14 -----------------------------------------------------------------------------------

def _tool_read(bus: Bus, address: str, presence: bool = True) -> dict[str, Any]:
    """zk2py's tool, config.v2 §5 (round 102): the premise (a descriptor
    listing config.v2, by presence and the descriptor), a fresh record (the
    heartbeat-following subscriber's judgement, or a reply's record), and the
    change events S heard."""
    from . import live

    out: dict[str, Any] = {"premise": None, "record": None, "fresh": False, "events": []}
    if presence:
        alive: dict[str, bool] = {}
        for _, kind, key in bus.tokens.of(f"{address}/@zk/instance/"):
            alive[key] = kind == "put"
        insts = [k for k, v in alive.items() if v]
        if not insts:
            out["premise"] = "absent"
            return out
        docs = []
        for k in insts:
            key = k if isinstance(k, str) and k.startswith("zk2/") else f"zk2/{address}/@zk/instance/{k}"
            d = live.get_descriptor(bus.t, key)
            if d and d[0].ok:
                docs.append(json.loads(d[0].payload))
        out["premise"] = "listed" if any("config.v2" in (x.get("profiles") or []) for x in docs) else "not_listed"
        if out["premise"] != "listed":
            return out
    sub = bus.hb.get(address)
    key = f"zk2/{address}/{IFACE}/state/config"
    recs = bus.records(address)
    if recs:
        out["record"] = recs[-1][2]
    if sub is not None:
        out["fresh"] = sub.judge(key).verdict == fr.FRESH
    out["events"] = [e for _, _, e in bus.events(address)]
    return out


def section14(report: Report, pairing: str) -> None:
    from .contract import load_contract
    from .owner import Owner

    sec = "§14 a tool's answers"
    bus = Bus(pairing, ("lab/rf0", "lab/plain"))
    plain = None
    try:
        o = bus.owner()
        plain = Owner("lab", "plain", [load_contract(NAV)], connect=bus.endpoint)
        plain.start()
        time.sleep(0.8)

        def answers(address: str, presence: bool = True, window_s: float | None = None, **q: Any) -> dict[str, Any]:
            if pairing == "py-zenctl":
                args = ["get", address, "--clocks-synced"]
                if window_s is not None:
                    # its events are read where a window hears them (zenctl
                    # config get --for)
                    args += ["--for", f"{window_s:g}"]
                if "token" in q:
                    args += ["--token", str(q["token"])]
                if "instance" in q:
                    args += ["--instance", str(q["instance"])]
                if not presence:
                    args += ["--across-face", "--iface", IFACE]
                z = bus.C.run(*args, timeout_s=2.0)
                return {"zenctl": z["out"] if isinstance(z["out"], dict) else {"exit": z["exit"], "err": z["err"]},
                        "exit": z["exit"]}
            return c2.tool_answers(_tool_read(bus, address, presence), bus.layout, **q)

        def a(ans: dict[str, Any], q: str, **of: str) -> Any:
            if "zenctl" in ans:
                z = ans["zenctl"]
                if q in ("landed", "undo_clean"):
                    return ((z.get("change") or {}).get(q) or {}).get("answer")
                if q in ("writable", "survives"):
                    row = "group" if q == "writable" else "parameter"
                    hit = [r for r in z.get("rows") or [] if r.get("row") == row
                           and all(r.get(k) == v for k, v in of.items())]
                    return hit[0].get("answer") if hit else None
                return (z.get(q) or {}).get("answer")
            return ans.get(q, (None,))[0]

        a_rf, a_pl = answers("lab/rf0"), answers("lab/plain")
        report.check(sec, "step 1: is a change pending? lab/rf0: no; lab/plain: not asked, its descriptor not "
                          "listing config.v2",
                     a(a_rf, "pending") == "no" and a(a_pl, "pending") in ("not_asked", None)
                     and (pairing != "py-zenctl" or a_pl["exit"] == 2 or a(a_pl, "pending") == "not_asked"),
                     f"rf0 {a(a_rf, 'pending')}; plain {a(a_pl, 'pending') if 'zenctl' not in a_pl else a_pl['exit']}")
        r = bus.call("set/radio", {"frequency_hz": 869525000, "confirm_s": 10})
        p = _pending(r) or {}
        token, inst = p.get("token"), (r.state or {}).get("instance")
        time.sleep(0.4)
        a2 = answers("lab/rf0", token=token, instance=inst)
        if "zenctl" in a2:
            pc = a2["zenctl"].get("pending_change") or {}
            pend2 = {"token": int(pc["token"]) if str(pc.get("token", "")).isdigit() else None,
                     "deadline": v1.parse_time(pc["deadline"]) // 1_000_000 if pc.get("deadline") else None}
        else:
            pend2 = a2.get("pending", (None, None))[1]
        report.check(sec, "step 2: yes, with the token and the deadline; \"did it land?\": not yet",
                     a(a2, "pending") == "yes" and a(a2, "landed") == "not_yet"
                     and (pend2 or {}).get("token") == token and (pend2 or {}).get("deadline") == p.get("deadline"),
                     f"pending {a(a2, 'pending')} {pend2 if 'zenctl' not in a2 else ''}; landed {a(a2, 'landed')}")
        held3: dict[str, Any] = {}
        win = None
        if pairing == "py-zenctl":
            # T's window opened now, to hear the rolled-back event at the deadline
            left = max(1.0, (p.get("deadline", 0) - time.time_ns() // 1_000_000) / 1000 + 2.0)
            win = threading.Thread(target=lambda: held3.update(answers("lab/rf0", window_s=left, token=token,
                                                                         instance=inst)), daemon=True)
            win.start()
        _wait(lambda: any(e[2]["token"] == token for e in bus.events()), 14.0)
        time.sleep(0.4)
        if win is not None:
            win.join(timeout=30)
            a3 = held3
        else:
            a3 = answers("lab/rf0", token=token, instance=inst)
        report.check(sec, "step 3: after the rollback at its deadline: \"did it land?\" no, rolled-back; \"was its "
                          "undo clean?\" yes",
                     a(a3, "landed") == "no" and a(a3, "undo_clean") == "yes",
                     f"landed {a(a3, 'landed')}, undo {a(a3, 'undo_clean')}"
                     + (f"; {a3['zenctl'].get('landed')}" if "zenctl" in a3 else ""))
        bus.call("set/duty", {"fraction_ppm": 20000})
        time.sleep(0.4)
        a4 = answers("lab/rf0", group="queue", survives=("duty", "fraction_ppm"))
        s_ans = a(a4, "survives", group="duty", parameter="fraction_ppm")
        w_ans = a(a4, "writable", group="queue")
        report.check(sec, "step 4: \"will fraction_ppm survive a restart?\" no, its differs set; \"is queue "
                          "writable?\" no, its bit clear though the contract declares it hot",
                     s_ans == "no" and w_ans == "no", f"survives {s_ans}; writable {w_ans}"
                     + (f"; keys {sorted(a4['zenctl'])}" if "zenctl" in a4 else ""))
        last_put = max(d.at_ns for d in bus.s.of(f"zk2/lab/rf0/{IFACE}/state/config"))
        bus.drop(o)
        time.sleep(1.5)
        a5 = answers("lab/rf0")
        within = (time.time_ns() - last_put) / NS
        report.check(sec, "step 5a: within 60 s, \"is a change pending?\": not asked, the owner absent (presence's "
                          "word)", a(a5, "pending") in ("not_asked",) or ("zenctl" in a5 and a5["exit"] == 2),
                     f"{a(a5, 'pending')} (exit {a5.get('exit')}) at {within:.1f} s")
        while (time.time_ns() - last_put) / NS < 70:
            time.sleep(0.5)
        a6 = answers("lab/rf0", presence=False)
        report.check(sec, "step 5b: 70 s after the last put, a reader that does not read presence: unobservable, the "
                          "record stale; never no (v1 §5, core O5)", a(a6, "pending") == "unobservable",
                     f"{a(a6, 'pending')}" + (f" (exit {a6['exit']})" if "zenctl" in a6 else ""))
    finally:
        if plain is not None:
            plain.close()
        bus.close()


# -- the re-mint (config.v1 §2.12, #1055, #1064) -------------------------------------------------

def section_remint(report: Report, pairing: str) -> None:
    """config.v1 §2.12 "A re-mint is a restart", by config.v2's reference
    (§0, §2.12): a running owner's re-mint while a reach change is pending
    and applied, then a new run within the instance (core §4.3). zk2py's
    owner re-mints through :meth:`ConfigV2Owner.remint`; the owner example
    with ``--frames``, ``frame new-epoch`` and ``frame new-run``."""
    sec = "re-mint (v1 §2.12)"
    if pairing == "py-zenctl":
        report.cannot(sec, "a re-mint is the owner's: zk2py's owner is run in py-py")
        return
    bus = Bus(pairing)
    try:
        from . import live
        from .config_scenarios import Tokens

        seen: list[tuple[str, Any]] = []

        def got(key: str) -> None:
            # T's GET the moment it sees an instance token (§2.3, as §1 step 1)
            try:
                seen.append((key, bus.get(session=bus.t)[1]))
            except Exception as e:  # noqa: BLE001
                seen.append((key, {"error": str(e)}))

        o = bus.owner(**({"frames": True} if bus.rust else {}))
        time.sleep(0.3)
        bus.extra.append(Tokens(live.open_client(bus.endpoint), "zk2/lab/rf0/@zk/instance/*",
                                on_put=lambda k: threading.Thread(target=got, args=(k,), daemon=True).start()))
        time.sleep(0.3)
        t0 = time.time_ns()
        r1 = bus.call("set/radio", {"frequency_hz": 869525000, "confirm_s": 120})
        p1 = _pending(r1) or {}
        old_inst, old_number = o.instance, (r1.state or {}).get("instance")
        _wait(lambda: _held_applied(bus, t0), 5.0)
        t1 = time.time_ns()
        new_inst = o.remint()
        time.sleep(0.8)
        recs = bus.records(since=t1)
        evs = bus.events(since=t1)
        new_recs = [x for x in recs if x[2].get("instance") != old_number]
        declared = [r[0] for r in bus.tokens.of(f"/@zk/instance/{new_inst}") if r[1] == "put"]
        withdrawn = [r for r in bus.tokens.of(f"/@zk/instance/{old_inst}") if r[1] == "delete"]
        nr = new_recs[0] if new_recs else (0, None, {})
        at_token = next((r for k, r in seen if k.endswith(new_inst)), None)
        dev = o.device()
        report.check(sec, "a re-mint with a reach change pending and applied: the device back at its startup "
                          "frequency; a new instance (the old token withdrawn) whose record, its instance number the "
                          "new run, is revision 0, nothing pending, no last change, frequency_hz at file, put before "
                          "its instance token: T's GET the moment it appears answers it (§2.3)",
                     new_inst != old_inst and bool(withdrawn) and bool(declared) and nr[1] is not None
                     and nr[2].get("instance") == nr[1].run and nr[2].get("revision") == 0
                     and nr[2].get("pending") is None and nr[2].get("last") is None
                     and _p(nr[2], "radio", "frequency_hz") == {"source": "file", "differs": False, "value": 868100000}
                     and at_token is not None and at_token.get("instance") == nr[2].get("instance")
                     and dev["held"]["radio"]["frequency_hz"] == 868100000,
                     f"records {[(st and str(st), r.get('instance'), r.get('revision'), bool(r.get('pending'))) for _, st, r in recs]}; "
                     f"T's GET at the new token: instance {(at_token or {}).get('instance')}; S heard the new record "
                     f"{(nr[0] - min(declared)) / 1e6 if declared and nr[0] else None} ms after the token; "
                     f"withdrawn {len(withdrawn)}; held {dev['held']['radio']}; undos {dev['undos']}")
        # config.v1 0.11 §2.12 (F-481): "A pending change ends first, in the
        # old instance, as cancel ends it … its read-back is put", then "Its
        # event is published, rolled-back or partial"; config.v2 0.2 §2.12:
        # "That change's record and event carry the old instance number."
        old_recs = [x for x in recs if x[2].get("instance") == old_number and x[1] is not None]
        ended = next((x for x in old_recs if x[2].get("pending") is None), None)
        ev_old = [x for x in evs if x[2].get("instance") == old_number]
        ev_new = [x for x in evs if x[2].get("instance") != old_number]
        report.check(sec, "the pending change ends first in the old instance (config.v1 0.11 §2.12): its record, "
                          "nothing pending, then one event, rolled-back, each under the old instance number and run, "
                          "both before the new instance's record; none under the new",
                     ended is not None and len(ev_old) == 1 and ev_old[0][2]["outcome"] == "rolled-back"
                     and ev_old[0][2].get("token") == p1.get("token") and ev_old[0][1] is not None
                     and ended[1].run == ev_old[0][1].run and ended[1].seq < ev_old[0][1].seq
                     and nr[0] is not None and ended[0] <= ev_old[0][0] <= nr[0] and not ev_new,
                     f"events {[(str(st), e['outcome'], e['instance']) for _, st, e in evs]}; records under the old "
                     f"instance number {[(str(st), r.get('revision'), bool(r.get('pending'))) for _, st, r in old_recs]}")
        r3 = bus.call("confirm", {"token": p1.get("token", 0)})
        report.check(sec, "a late confirm with the old token: not_found", r3.code == "not_found", r3.short())
        r4 = bus.call("set/duty", {"fraction_ppm": 1})
        t5 = time.time_ns()
        run = o.new_run()
        time.sleep(0.4)
        quiet = not bus.records(since=t5)
        r5 = bus.call("set/duty", {"fraction_ppm": 2})
        tok = ((r5.state or {}).get("last") or {}).get("token", 0)
        report.check(sec, "a new run within the instance (core §4.3): no put of its own; the next token's high 32 "
                          "bits the new run, its counter afresh; the record's instance number unchanged",
                     quiet and r5.kind == "value" and f"{tok >> 16:08x}" == run
                     and (r5.state or {}).get("instance") == (r4.state or {}).get("instance") == nr[2].get("instance"),
                     f"run {run}; token {hex(tok)}; instance {(r5.state or {}).get('instance')}")
    finally:
        bus.close()


SECTIONS = {"1": section1, "2": section2, "3": section3, "4": section4, "5": section5, "6": section6,
            "7": section7, "8": section8, "9": section9, "10": section10, "11": section11, "12": section12,
            "13": section13, "14": section14, "remint": section_remint}
LONG = ("1", "6", "14")


def run(report: Report, pairing: str, only: list[str] | None = None) -> None:
    chosen = [n for n in SECTIONS if not only or n in only]

    def one(name: str) -> None:
        try:
            SECTIONS[name](report, pairing)
        except Exception as e:  # noqa: BLE001 - a section that cannot run is a failure
            import traceback

            traceback.print_exc()
            report.check(f"§{name}", "the section ran", False, f"{type(e).__name__}: {e}")

    threads = [threading.Thread(target=one, args=(n,)) for n in chosen if n in LONG]
    for t in threads:
        t.start()
    for n in chosen:
        if n not in LONG:
            one(n)
    for t in threads:
        t.join()


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="python -m zk2py.config_v2_scenarios", description=__doc__.split("\n")[0])
    ap.add_argument("--pairing", action="append", choices=PAIRINGS)
    ap.add_argument("--only", action="append", choices=list(SECTIONS))
    ap.add_argument("--out", default=None)
    args = ap.parse_args(argv)
    pairings = args.pairing or list(PAIRINGS)
    if any(p != "py-py" for p in pairings) and not (BLACKBOX / "owner").is_file():
        print(f"config_v2_scenarios: the black boxes are not in {BLACKBOX}", file=sys.stderr)
        return 2
    rows = []
    for p in pairings:
        rep = Report(f"{p} ")
        run(rep, p, args.only)
        rows += [(p, *r) for r in rep.rows]
    by: dict[tuple[str, str], list[bool]] = {}
    for p, sec, _, ok, _ in rows:
        by.setdefault((p, sec.split(" ", 1)[1].split(" ")[0]), []).append(ok)
    for (p, s), oks in sorted(by.items(), key=lambda x: (x[0][0], int(x[0][1].strip("§")) if x[0][1].strip(
            "§").isdigit() else 0)):
        print(f"{p:10} {s:5} {sum(oks)}/{len(oks)}")
    if args.out:
        Path(args.out).write_text(json.dumps([{"pairing": p, "section": s, "check": n, "ok": ok, "detail": d}
                                              for p, s, n, ok, d in rows], indent=1) + "\n")
    passed = sum(1 for r in rows if r[3])
    print(f"config_v2_scenarios: {passed}/{len(rows)}")
    for p, sec, name, ok, d in rows:
        if not ok:
            print(f"FAIL [{sec}] {name}: {d}")
    return 0 if passed == len(rows) else 1


if __name__ == "__main__":
    sys.exit(main())

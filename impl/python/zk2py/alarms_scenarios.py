"""The ``alarms.v1`` scenarios (``spec/profiles/alarms/scenarios.md``, text
0.2) on in-process zenoh-python routers: ``python -m zk2py.alarms_scenarios
[--only 1 …] [--owner <exe>] [--ttl <s>] [--full]``.

Each owner section runs against one owner implementation, through an
adapter:
- **zk2py's owner** (the default): :class:`zk2py.alarms.AlarmsOwner`;
- **the Rust owner example** (``--owner <exe>``, or ``ZK2PY_OWNER``), a
  black box driven by its ``--alarms`` stdin commands (:class:`RustAlarms`).

Either way:
- **The bus** is the text's: R1, a router with timestamping on; the owner,
  S, G and T its clients, each with a session of its own. Timeouts are 1 s.
- **The contract** is the standard one, 900 s, for §1, §3, §5 and §7 to
  §9. §2, §6 and §10 wait out the horizon, and §4's extra check waits for
  the re-raise's confirmations, so by default they run on a copy
  of the standard contract with ``freshness.ttl_s = --ttl`` (4 s), made in
  memory and never published, as the reference's default run does
  (scenarios.md's conventions, 0.2). Their waits scale with it: §2's 455 s
  are ttl/2 + 5 s of margin … here three confirmation periods; §6's 905 s
  are ttl + 1.5 s; §10's 905 s are ttl + 1 s. ``--full`` runs them at
  900 s: §2 then takes about 8 minutes, and §6 and §10 about 15 each.
- **S** subscribes to ``zk2/lab/svc/alarms.v1/**`` and holds a liveliness
  subscriber on the owner's instance tokens before the owner starts, both
  on S's monotonic clock. "Reads" is :class:`zk2py.alarms.Transitions` over
  S's deliveries of a key; "judges" is :func:`zk2py.alarms.read_owner` and
  :func:`zk2py.alarms.judge`, on the deployment's word for the clock.
- **§8 to §10** are a tool's: T is zk2py's reader. §8's archive is the
  stand-in of ``health.v1``'s scenarios §6 (core §4.4's form). §9's liar is
  a zk2py owner putting raw documents, since the text gives no owner a way
  to put a document on another key.

Exit 0 when every check passes, 1 when any fails, 2 when a check or a
section did not run, unless ``--only`` or ``--quick`` asked for that subset.
"""

from __future__ import annotations

import argparse
import json
import os
import queue
import re
import sys
import tempfile
import threading
import time
from pathlib import Path
from typing import Any

from . import alarms as al
from . import freshness as fr

JITTER_S = 0.5
A = ("link_down", {"peer": "r2", "port": "eth0"})
A_KEY = "a659f813308ad1da"
B = ("disk_full", {"mount": "/var"})
C = ("link_down", {"peer": "r3", "port": "eth1"})
ADDRESS = "lab/svc"
_ANSI = re.compile(r"\x1b\[[0-9;]*m")


class Report:
    def __init__(self) -> None:
        self.rows: list[tuple[str, str, bool, str]] = []
        self.lock = threading.Lock()
        self.not_run: list[tuple[str, str]] = []

    def check(self, section: str, name: str, ok: bool, detail: str = "") -> bool:
        with self.lock:
            self.rows.append((section, name, bool(ok), detail))
            print(f"{'PASS' if ok else 'FAIL'} [{section}] {name}" + (f": {detail}" if detail else ""), flush=True)
        return bool(ok)

    def note(self, section: str, text: str) -> None:
        """A measurement printed and not counted."""
        with self.lock:
            print(f"NOTE [{section}] {text}", flush=True)

    def cannot(self, section: str, why: str) -> None:
        with self.lock:
            self.not_run.append((section, why))
            print(f"NOT RUN [{section}] {why}", flush=True)


def _wait(pred: Any, timeout: float, step: float = 0.05) -> bool:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if pred():
            return True
        time.sleep(step)
    return bool(pred())


# -- the owners, behind one adapter ------------------------------------------------------

class PyAlarms:
    """zk2py's :class:`zk2py.alarms.AlarmsOwner`. ``crash`` ends it
    without resolving anything; a restart is a new owner with the same
    record."""

    rust = False

    def __init__(self, endpoint: str, address: str, *, contract: Any, cardinality: int | None = None,
                 tokenless: bool = False, record: al.Record | None = None, rules: set[str] | None = None) -> None:
        self.endpoint, self.address, self.contract = endpoint, address, contract
        self.kw: dict[str, Any] = {}
        if cardinality is not None:
            self.kw["cardinality"] = {al.PROFILE: {f"{al.TOKEN}/{al.TEMPLATE}": cardinality}}
        if tokenless:
            self.kw["tokenless"] = {al.PROFILE}
        self.record = record if record is not None else al.Record()
        self.rules = rules
        self.o = self._make()

    def _make(self) -> al.AlarmsOwner:
        system, service = self.address.split("/")
        o = al.AlarmsOwner(system, service, contract=self.contract, record=self.record, rules=self.rules,
                           connect=self.endpoint, **self.kw)
        o.start()
        time.sleep(0.3)
        return o

    @property
    def instance(self) -> str:
        return self.o.owner.instance

    def raise_(self, alarm: tuple[str, dict[str, str]], severity: str = "warning", summary: str | None = None,
               labels: dict[str, str] | None = None) -> str:
        from .owner import BudgetExceeded

        rule, lab = alarm
        try:
            return self.o.raise_alarm(rule, labels if labels is not None else lab, severity,
                                      summary or f"{rule} {json.dumps(lab, sort_keys=True)}")
        except BudgetExceeded as e:
            return f"refused: {e}"

    def resolve(self, alarm: tuple[str, dict[str, str]]) -> bool:
        return self.o.resolve(*alarm)

    def stop_confirming(self, alarm: tuple[str, dict[str, str]]) -> None:
        self.o.stop_evaluating(*alarm)

    def crash(self) -> None:
        self.o.owner.close()

    def restart(self) -> None:
        self.o = self._make()

    def close(self) -> None:
        self.o.close()

    def summary_of(self, alarm: tuple[str, dict[str, str]]) -> str:
        return self.o.held[self.o.key_for(*alarm)].summary

    def adoption(self) -> list[tuple[str, str]]:
        return list(self.o.adoption)


class RustAlarms:
    """The owner example's ``--alarms``, a black box: a child process driven
    by its stdin commands. Its ``--help`` gives the flags (``--alarms``,
    ``--alarms-record <file>``, ``--alarms-archive``, ``--alarms-rules``,
    ``--cardinality``, ``--tokenless``) and nothing of stdin; the command
    ``alarm`` answers its own usage (``alarm raise <json> | resolve
    <alert_key> | confirm <alert_key>|all on|off | list | adoption``), found
    by trying (SPEC-FINDINGS F-133).
    - A short-ttl copy is passed as an ``alarms.v1`` contract file, written
      with its schema into a temporary directory.
    - It always keeps a record file, so a restart is a new process on it.
      Its format, read from the file: ``{"profile": "alarms.v1", "alarms":
      {<alert key>: <document>}}``.
    - ``crash`` kills the process; ``close`` closes its stdin, a clean
      close."""

    rust = True

    def __init__(self, exe: Path, endpoint: str, address: str, *, contract: Any, cardinality: int | None = None,
                 tokenless: bool = False, record: Any = None, rules: set[str] | None = None) -> None:
        self.exe, self.endpoint, self.address = exe, endpoint, address
        self.tmp = tempfile.TemporaryDirectory(prefix="zk2py-alarms-rust-")
        d = Path(self.tmp.name)
        self.record_path = d / "record.json"
        self.contracts: list[Path] = []
        ttl = al.horizon_of(contract).ttl_s
        if ttl != al.TTL_S:
            (d / "schemas").mkdir()
            (d / "schemas" / "alarms.v1.json").write_bytes(al.SCHEMA_PATH.read_bytes())
            text = al.CONTRACT.read_text(encoding="utf-8").replace(f'"freshness.ttl_s" = {al.TTL_S}',
                                                                   f'"freshness.ttl_s" = {ttl}')
            (d / "alarms.v1.toml").write_text(text, encoding="utf-8")
            self.contracts.append(d / "alarms.v1.toml")
        self.flags = ["--alarms", "--alarms-record", str(self.record_path)]
        if cardinality is not None:
            self.flags += ["--cardinality", f"{al.PROFILE}:{al.TOKEN}/{al.TEMPLATE}={cardinality}"]
        if tokenless:
            self.flags += ["--tokenless", al.PROFILE]
        if rules is not None:
            self.flags += ["--alarms-rules", ",".join(sorted(rules))]
        self._start()

    def _start(self) -> None:
        from .live_interop import Owner

        self.p = Owner(self.exe, self.address, self.contracts, connect=self.endpoint, flags=self.flags)
        k = self.p.wait_for("ready ", 120)
        if k is None:
            raise RuntimeError(f"the owner example did not start: {self.p.stderr[-3:]}")
        self.instance_key = k
        time.sleep(0.3)

    @property
    def instance(self) -> str:
        return self.instance_key.rsplit("/", 1)[1]

    def _cmd(self, line: str) -> str:
        got = self.p.command(line, timeout=10.0)
        if got is None:
            raise RuntimeError(f"{line!r}: no answer")
        return got

    def raise_(self, alarm: tuple[str, dict[str, str]], severity: str = "warning", summary: str | None = None,
               labels: dict[str, str] | None = None) -> str:
        rule, lab = alarm
        cond = {"rule": rule, "labels": labels if labels is not None else lab, "severity": severity,
                "summary": summary or f"{rule} {json.dumps(lab, sort_keys=True)}"}
        got = self._cmd(f"alarm raise {json.dumps(cond, separators=(',', ':'))}")
        if got.startswith("refused"):
            return got
        words = got.split()
        if len(words) < 2 or words[0] != "alarm":
            raise RuntimeError(f"alarm raise: cannot read the answer {got!r}")
        act = words[1]
        return "none" if act == "unchanged" else act

    def resolve(self, alarm: tuple[str, dict[str, str]]) -> bool:
        return self._cmd(f"alarm resolve {al.derive(*alarm).key}").startswith("alarm resolved")

    def stop_confirming(self, alarm: tuple[str, dict[str, str]]) -> None:
        self._cmd(f"alarm confirm {al.derive(*alarm).key} off")

    def crash(self) -> None:
        self.p.proc.kill()
        self.p.proc.wait()

    def restart(self) -> None:
        self._start()

    def close(self) -> None:
        self.p.close()
        self.tmp.cleanup()

    def _record(self) -> dict[str, Any]:
        return json.loads(self.record_path.read_text(encoding="utf-8"))

    def record_doc(self, full_key: str) -> dict[str, Any]:
        return self._record()["alarms"][full_key.rsplit("/", 1)[1]]

    def write_record(self, docs: dict[str, dict[str, Any]]) -> None:
        rec = self._record()
        rec["alarms"].update({k.rsplit("/", 1)[1]: d for k, d in docs.items()})
        self.record_path.write_text(json.dumps(rec), encoding="utf-8")

    def adoption(self) -> str:
        return self._cmd("alarm adoption")


class Bus:
    """R1; S, G and T as client sessions of it; S's subscription and token
    watch declared before any owner starts."""

    def __init__(self, mode: Any, selectors: list[str] | None = None, address: str = ADDRESS) -> None:
        import zenoh

        from . import live
        from .live_interop import _r1

        self.mode = mode
        self.router, self.endpoint, _ = _r1()
        self.s_session = live.open_client(self.endpoint)
        self.g_session = live.open_client(self.endpoint)
        self.t_session = live.open_client(self.endpoint)
        self.trust = fr.ClockTrust(word=True)
        self.s = fr.Subscriber(self.s_session, selectors or [f"zk2/{address}/{al.PROFILE}/**"])
        self.tokens: list[tuple[int, str, str]] = []
        self.tlock = threading.Lock()

        #: called, on a thread of its own, with each instance token's key
        #: the moment S sees it appear
        self.on_token: Any = None

        def got(smp: Any) -> None:
            put = smp.kind == zenoh.SampleKind.PUT
            with self.tlock:
                self.tokens.append((time.monotonic_ns(), "put" if put else "delete", str(smp.key_expr)))
            if put and self.on_token is not None:
                threading.Thread(target=self.on_token, args=(str(smp.key_expr),), daemon=True).start()

        # Observer §3.7 (0.37): S's token subscription on a watch session of its
        # own, never beside the presence reads made on s_session.
        self.token_sub, _ = live.watch_or_beside(self.s_session, "zk2/lab/*/@zk/instance/*",
                                                 zenoh.handlers.Callback(got), history=True)
        self.owners: list[Any] = []
        self.extra: list[Any] = []
        time.sleep(0.2)

    def owner(self, address: str = ADDRESS, **kw: Any) -> Any:
        if self.mode == "zk2py":
            o: Any = PyAlarms(self.endpoint, address, **kw)
        else:
            o = RustAlarms(self.mode, self.endpoint, address, **kw)
        self.owners.append(o)
        return o

    def deliveries(self, key: str, since_ns: int = 0) -> list[fr.Received]:
        return [x for x in self.s.of(key) if x.arrival_ns >= since_ns]

    def reads(self, key: str) -> list[str]:
        """S's readings of a key (§2.6), over every delivery so far, in
        arrival order. On one router a key's deliveries arrive in stamp
        order."""
        return al.transitions(al.DELETE if x.kind == "delete" else (al.decode(x.payload) or {"x": 1})
                              for x in self.s.of(key))

    def token_puts(self, address: str = ADDRESS) -> list[tuple[int, str]]:
        with self.tlock:
            return [(t, k) for t, kind, k in self.tokens if kind == "put" and f"zk2/{address}/" in k]

    def read(self, session: Any, address: str = ADDRESS, *, subscriber: fr.Subscriber | None = None,
             get: bool = True, horizon: fr.Horizon = al.HORIZON, window: Any = None,
             keys_known: tuple[str, ...] = ()) -> al.OwnerReading:
        return al.read_owner(session, address, subscriber=subscriber, window=window, trust=self.trust, get=get,
                             horizon=horizon, keys_known=keys_known)

    def close(self) -> None:
        self.s.close()
        try:
            self.token_sub.undeclare()
        except Exception:  # noqa: BLE001 - closing anyway
            pass
        for o in list(self.owners):
            try:
                o.close()
            except Exception:  # noqa: BLE001 - closing anyway
                pass
        for e in self.extra:
            try:
                e.close()
            except Exception:  # noqa: BLE001
                pass
        from . import live

        for x in (self.s_session, self.g_session, self.t_session, self.router):
            live.close(x)  # and the watch session zk2py opened for it


def key(alarm: tuple[str, dict[str, str]], address: str = ADDRESS) -> str:
    system, service = address.split("/")
    k = al.key_of(system, service, al.derive(*alarm).key)
    assert k is not None
    return k


def _doc(x: fr.Received) -> al.Alarm | None:
    return al.decode(x.payload) if x.kind == "put" else None


def _pair(a: al.Answer) -> str:
    return f"{a.answer}, {a.reason}"


def _zid_eq(a: Any, b: Any) -> bool:
    return al._zid(a) is not None and al._zid(a) == al._zid(b)


# -- §1 Raise ------------------------------------------------------------------------------

def section1(report: Report, mode: Any, ctx: dict[str, Any]) -> None:
    sec = "§1"
    bus = Bus(mode)
    try:
        o = bus.owner(contract=al.load())
        g = bus.read(bus.g_session)
        report.check(sec, "1. G's GET ends at its final reply with no value (core S2)",
                     g.seed is not None and g.seed.complete and g.seed.members == 0 and not g.readings,
                     f"seed {g.seed}, keys {sorted(g.readings)}")
        t0 = time.time_ns() // 1_000_000
        mark = time.monotonic_ns()
        act = o.raise_(A, "warning", summary="link to r2 down on eth0")
        k = key(A)
        report.check(sec, "2. the key is the test vector's", k.endswith(f"/{A_KEY}"), k)
        _wait(lambda: bus.deliveries(k, mark), 3.0)
        time.sleep(0.3)
        got = bus.deliveries(k, mark)
        d = _doc(got[0]) if got else None
        g = bus.read(bus.g_session)
        report.check(sec, "2. S receives one put on the key, stamped by the owner (core S1)",
                     len(got) == 1 and got[0].kind == "put" and _zid_eq(got[0].stamp_id, g.meta_zid),
                     f"{len(got)} deliveries, stamp {got[0].stamp_id if got else None}, meta.zid {g.meta_zid}, "
                     f"act {act}")
        report.check(sec, "2. A firing, its timestamp within 1 s of the step, no observed_at_ms",
                     d is not None and d.firing and d.severity == "warning" and abs(d.timestamp - t0) <= 1000
                     and d.observed_at_ms is None, f"{d}")
        report.check(sec, "2. S reads it raised", bus.reads(k) == [al.RAISED], f"{bus.reads(k)}")
        reply = g.stamps.get(k)
        f1, _ = g.answers()[k] if k in g.readings else (None, None)
        report.check(sec, "2. G's reply holds the same document under the same stamp (core S2)",
                     bool(got) and reply == got[0].stamp_id and g.documents.get(k) == d,
                     f"reply stamp {reply}, document {g.documents.get(k)}")
        report.check(sec, "2. G judges yes, firing", f1 is not None and _pair(f1) == "yes, firing",
                     f1 and _pair(f1))
        mark = time.monotonic_ns()
        act = o.raise_(A, "warning", summary="link to r2 down on eth0", labels={"port": "eth0", "peer": "r2"})
        time.sleep(1.0)
        got = bus.deliveries(k, mark)
        same = all(x.kind == "put" and _doc(x) == d for x in got)
        report.check(sec, "3. nothing changes: no put, or the same document, which S reads confirmed",
                     same and bus.reads(k)[1:] == [al.CONFIRMED] * len(got),
                     f"act {act}, {len(got)} puts, reads {bus.reads(k)}")
        with bus.s.lock:
            ks = sorted(x for x in bus.s.keys() if "/state/alarms/" in x)
        report.check(sec, "3. no second key appears, whatever the order of the labels (§2.2)", ks == [k], f"{ks}")
    finally:
        bus.close()


# -- §2 Escalate, refresh and confirm -----------------------------------------------------

def section2(report: Report, mode: Any, ctx: dict[str, Any]) -> None:
    ttl = ctx["ttl"]
    sec = f"§2 (ttl {ttl} s)"
    contract = al.load(None if ttl == al.TTL_S else ttl)
    bus = Bus(mode)
    try:
        o = bus.owner(contract=contract)
        k = key(A)
        o.raise_(A, "warning", summary="link to r2 down on eth0")
        _wait(lambda: bus.s.of(k), 3.0)
        t0 = _doc(bus.s.of(k)[-1]).timestamp  # type: ignore[union-attr]
        time.sleep(0.2)
        mark = time.monotonic_ns()
        o.raise_(A, "critical", summary="link to r2 down on eth0")
        _wait(lambda: bus.deliveries(k, mark), 3.0)
        e = _doc(bus.deliveries(k, mark)[0]) if bus.deliveries(k, mark) else None
        report.check(sec, "1. a put on the same key: critical, timestamp later than t0, no observed_at_ms",
                     e is not None and e.severity == "critical" and e.timestamp > t0 and e.observed_at_ms is None,
                     f"{e}, t0 {t0}")
        report.check(sec, "1. S reads it escalated", bus.reads(k)[-1:] == [al.ESCALATED], f"{bus.reads(k)}")
        time.sleep(0.2)
        mark = time.monotonic_ns()
        act = o.raise_(A, "critical", summary="link to r2 down on eth0 for 5 min",
                       labels={**A[1], "host.id": "h-3fa9c2d41b7e"})
        _wait(lambda: bus.deliveries(k, mark), 3.0)
        got = bus.deliveries(k, mark)
        r = _doc(got[0]) if got else None
        report.check(sec, "2. a put on the same key: the escalation's timestamp kept, observed_at_ms at least it",
                     e is not None and r is not None and r.timestamp == e.timestamp and r.observed_at_ms is not None
                     and r.observed_at_ms >= r.timestamp and r.labels.get("host.id") == "h-3fa9c2d41b7e",
                     f"act {act}, {r}")
        report.check(sec, "2. the alert key unchanged: host.id is host-scoped (§2.2)",
                     r is not None and r.derive().key == A_KEY, r and r.derive().key)
        report.check(sec, "2. S reads it refreshed", bus.reads(k)[-1:] == [al.REFRESHED], f"{bus.reads(k)}")
        period = ttl / 2
        wait = 455.0 if ttl == al.TTL_S else 3 * period + 0.5
        mark = time.monotonic_ns()
        last_refresh = got[-1] if got else None
        time.sleep(wait)
        conf = bus.deliveries(k, mark)
        arrivals = [last_refresh.arrival_ns] + [x.arrival_ns for x in conf] if last_refresh else []
        gaps = [(b - a) / fr.NS for a, b in zip(arrivals, arrivals[1:])]
        report.check(sec, f"3. over {wait:.1f} s, S receives the refreshed document again, unchanged, "
                          "observed_at_ms included", bool(conf) and all(_doc(x) == r for x in conf),
                     f"{len(conf)} puts")
        report.check(sec, f"3. no two deliveries more than ttl/2 ({period:g} s) apart, jitter {JITTER_S} s",
                     bool(gaps) and max(gaps) <= period + JITTER_S, f"gaps {[round(x, 3) for x in gaps]}")
        stamps = [(x.stamp_ns, x.stamp_id) for x in ([last_refresh] if last_refresh else []) + conf]
        report.check(sec, "3. each under a later stamp of the same id (freshness.v1 §2.4)",
                     len(stamps) > 1 and all(b[0] > a[0] and _zid_eq(a[1], b[1]) for a, b in zip(stamps, stamps[1:])),
                     f"{len(stamps)} stamps")
        n = len(conf)
        report.check(sec, "3. S reads each confirmed", n > 0 and bus.reads(k)[-n:] == [al.CONFIRMED] * n,
                     f"{bus.reads(k)[-n - 1:]}")
    finally:
        bus.close()


# -- §3 Resolve, then delete, and §4 Re-raise -----------------------------------------

def section3(report: Report, mode: Any, ctx: dict[str, Any], reraise: bool = False) -> None:
    # §4 runs on the short-ttl copy unless --full, so that its re-raise's
    # confirmations can be waited for (review of PR #807).
    ttl = ctx["ttl"] if reraise else al.TTL_S
    sec = f"§4 (ttl {ttl} s)" if reraise else "§3"
    bus = Bus(mode)
    try:
        o = bus.owner(contract=al.load(None if ttl == al.TTL_S else ttl))
        k = key(A)
        # Beyond scenarios.md, for 0.3's §2.14 step 5: a second reader R,
        # whose subscription hears A firing and closes before the resolve.
        # R is an AlarmsReader; its subscription closes before the resolve,
        # and it notes the loss that took A's resolve and delete.
        r3 = None if reraise else al.AlarmsReader(bus.s_session, f"zk2/{ADDRESS}/{al.PROFILE}/state/alarms/*",
                                                  (ADDRESS,), trust=bus.trust)
        if r3 is not None:
            bus.extra.append(r3)
        r_sub = None if r3 is None else r3.subscriber
        o.raise_(A, "warning")
        _wait(lambda: bus.s.of(k), 3.0)
        fired = _doc(bus.s.of(k)[-1])
        if r3 is not None and r_sub is not None:
            _wait(lambda: r_sub.of(k), 3.0)
            r_sub.close()
            r3.note_loss()
            o.raise_(B, "warning")
        mark = time.monotonic_ns()
        o.resolve(A)
        _wait(lambda: any(x.kind == "delete" for x in bus.deliveries(k, mark)), 5.0)
        got = bus.deliveries(k, mark)
        put = next((x for x in got if x.kind == "put"), None)
        dele = next((x for x in got if x.kind == "delete"), None)
        rd = _doc(put) if put else None
        if not reraise:
            report.check(sec, "1. a put of A resolved, a new timestamp, no observed_at_ms",
                         rd is not None and rd.state == al.RESOLVED and fired is not None
                         and rd.timestamp > fired.timestamp and rd.observed_at_ms is None, f"{rd}")
            gap = None if put is None or dele is None else (dele.arrival_ns - put.arrival_ns) / fr.NS
            report.check(sec, "1. then a delete of the key, within 60 s and, for the reference, within 1 s",
                         gap is not None and 0 <= gap <= 1.0, f"{gap} s")
            report.check(sec, "1. S reads resolved, then retired",
                         bus.reads(k) == [al.RAISED, al.RESOLVED, al.RETIRED], f"{bus.reads(k)}")
            g = bus.read(bus.g_session)
            report.check(sec, "2. within 1 s of the delete, G's reply holds a reply_del with the delete's stamp",
                         k in g.readings and g.readings[k].latest == "deleted" and dele is not None
                         and g.stamps.get(k) == dele.stamp_id, f"latest {g.readings.get(k)}, stamp {g.stamps.get(k)}")
            f1 = g.answers()[k][0] if k in g.readings else None
            report.check(sec, "2. G judges no, deleted", f1 is not None and _pair(f1) == "no, deleted", f1 and _pair(f1))
            if ctx.get("quick"):
                report.cannot(sec, "3. the 65 s wait for the tombstone window: skipped by --quick")
            else:
                time.sleep(65.0)
                g = bus.read(bus.g_session)
                report.check(sec, "3. 65 s after the delete, G's GET draws nothing for the key (core S3)",
                             g.seed is not None and g.seed.complete and k not in g.readings, f"{sorted(g.readings)}")
                # R holds A firing and lost the resolve and delete; its GET,
                # asked after the loss, establishes the owner's alarms (it
                # answers B) and leaves A out, nothing of A arriving since:
                # step 4 healed, A holds none (0.5, F-136).
                assert r3 is not None and r_sub is not None
                rr = r3.read(ADDRESS)
                held = [x.kind for x in r_sub.of(k)]
                f1, f4 = rr.answers().get(k, (None, None))
                report.check(sec, "extra (§2.14 steps 4 and 5, 0.5): a reader that held A firing and lost its resolve "
                                  "and delete reads no, not_held",
                             held == ["put"] and rr.verdict.reason == "answered" and f1 is not None
                             and _pair(f1) == "no, not_held" and _pair(f4) == "not_asked, not_firing",
                             f"held {held}, owner {rr.verdict.expect()}, {f1 and _pair(f1)}; {f4 and _pair(f4)}")
        else:
            mark = time.monotonic_ns()
            o.raise_(A, "warning")
            _wait(lambda: bus.deliveries(k, mark), 3.0)
            got = bus.deliveries(k, mark)
            rr = _doc(got[0]) if got else None
            report.check(sec, "a put of A firing on the same key, its timestamp later than the resolve's",
                         rr is not None and rr.firing and rd is not None and rr.timestamp > rd.timestamp,
                         f"{rr}, resolve {rd and rd.timestamp}")
            report.check(sec, "S reads raised", bus.reads(k)[-1:] == [al.RAISED], f"{bus.reads(k)}")
            # Beyond scenarios.md: the resolve closed the alarm's writer
            # (§2.7, a resolved alarm is never confirmed), and the re-raise
            # reopens it, so the re-raised alarm is confirmed (§2.7).
            if ttl == al.TTL_S:
                report.cannot(sec, "extra: the re-raise's confirmations at 900 s: run §4 without --full")
            else:
                _wait(lambda: len(bus.deliveries(k, mark)) >= 3, 2 * ttl + 2.0)
                again = bus.deliveries(k, mark)
                gaps = [(b.arrival_ns - a.arrival_ns) / fr.NS for a, b in zip(again, again[1:])]
                n = len(again) - 1
                report.check(sec, "extra (§2.7): the re-raised alarm is confirmed, unchanged, within ttl/2",
                             n >= 2 and all(_doc(x) == rr for x in again[1:]) and max(gaps) <= ttl / 2 + JITTER_S
                             and bus.reads(k)[-n:] == [al.CONFIRMED] * n,
                             f"{n} confirmations, gaps {[round(x, 2) for x in gaps]}, reads {bus.reads(k)[-n - 1:]}")
    finally:
        bus.close()


def section4(report: Report, mode: Any, ctx: dict[str, Any]) -> None:
    section3(report, mode, ctx, reraise=True)


# -- §5 A restart mid-incident --------------------------------------------------------

def section5(report: Report, mode: Any, ctx: dict[str, Any]) -> None:
    sec = "§5"
    bus = Bus(mode)
    tmp = tempfile.TemporaryDirectory(prefix="zk2py-alarms-")
    try:
        record = al.Record(Path(tmp.name) / "record.json")
        o = bus.owner(contract=al.load(), record=record)
        ka, kb = key(A), key(B)
        o.raise_(A, "critical", summary="link to r2 down on eth0")
        o.raise_(B, "warning", summary="/var full")
        _wait(lambda: bus.s.of(ka) and bus.s.of(kb), 3.0)
        t0 = _doc(bus.s.of(ka)[-1]).timestamp  # type: ignore[union-attr]
        old_instance = o.instance
        # The test writes R and P into the record (scenarios.md §5 setup).
        r_doc = {"timestamp": t0 - 1000, "rule": "fan_failed", "severity": "warning", "state": "resolved",
                 "summary": "fan back"}
        kr = key(("fan_failed", {}))
        kp = ka.rsplit("/", 1)[0] + "/0000000000000001"
        mark = time.monotonic_ns()
        o.crash()
        time.sleep(3.0)
        got = [x for k in (ka, kb) for x in bus.deliveries(k, mark)]
        report.check(sec, "1. the process ends without closing: S receives no delete and no resolved put in 3 s",
                     not any(x.kind == "delete" or (_doc(x) and not _doc(x).firing) for x in got),  # type: ignore[union-attr]
                     f"{[(x.kind, _doc(x) and _doc(x).state) for x in got]}")
        # The test writes R and P into the record while the owner is down.
        if o.rust:
            o.write_record({kr: r_doc, kp: o.record_doc(ka)})
        else:
            record.put(kr, al.document_bytes(r_doc))
            record.put(kp, record.docs[ka])
        # Core §8.2's observation of "before the tokens" (presence.md §1,
        # health.v1's scenarios §1): a GET made the moment the new instance
        # token appears already answers what was put before it.
        at_token: dict[str, Any] = {}

        def get_at_token(k: str) -> None:
            from . import live

            if k.endswith(old_instance) or "st" in at_token:
                return
            at_token["st"] = live.get_state(bus.g_session, f"zk2/{ADDRESS}/{al.PROFILE}/state/alarms/*")

        bus.on_token = get_at_token
        mark = time.monotonic_ns()
        o.restart()
        new_instance = o.instance
        # 0.5: "G GETs the moment S sees the new instance token. Then the
        # owner evaluates its rules once": the GET first, B still firing.
        seen = _wait(lambda: "st" in at_token, 5.0)
        if not seen:
            new_tokens = [k for _, k in bus.token_puts() if not k.endswith(old_instance)]
            report.check(sec, "3. G's GET at the new instance token, before the owner's evaluation", False,
                         "no GET within 5 s: " + ("S saw no new instance token" if not new_tokens
                                                  else f"the GET did not return ({new_tokens})"))
        o.resolve(B)  # its first evaluation finds B's condition clear
        time.sleep(1.0)
        tok = [t for t, k in bus.token_puts() if k.endswith(new_instance)]
        report.check(sec, "3. a new instance", new_instance != old_instance and bool(tok),
                     f"{old_instance} -> {new_instance}")
        g = bus.read(bus.g_session)
        a_new = bus.deliveries(ka, mark)
        first_a = a_new[0] if a_new else None
        report.check(sec, "3. A firing, unchanged, timestamp t0 kept, under the new instance's stamp id",
                     first_a is not None and _doc(first_a) is not None and _doc(first_a).firing  # type: ignore[union-attr]
                     and _doc(first_a).timestamp == t0 and _zid_eq(first_a.stamp_id, g.meta_zid),  # type: ignore[union-attr]
                     f"{first_a and _doc(first_a)}, stamp {first_a and first_a.stamp_id}, meta.zid {g.meta_zid}")
        st = at_token.get("st")
        rep_at = {} if st is None else {r.key: r for r in st.replies}
        a_at, b_at = rep_at.get(ka), rep_at.get(kb)
        a_doc = None if a_at is None or a_at.deleted else al.decode(a_at.payload)
        problems = []
        if not seen:
            new_tokens = [k for _, k in bus.token_puts() if not k.endswith(old_instance)]
            problems.append("no GET at the token within 5 s: " + (
                "S saw no new instance token" if not new_tokens else f"the GET did not return ({new_tokens})"))
        elif st is not None and st.errors:
            problems.append(f"the GET's errors {st.errors}")
        if seen and (a_doc is None or not a_doc.firing or a_doc.timestamp != t0
                     or not _zid_eq(a_at.stamp_id if a_at else None, g.meta_zid)):
            problems.append(f"A not firing under the new instance's stamp with t0 kept: {a_doc}, "
                            f"stamp {a_at and a_at.stamp_id}")
        if seen and (b_at is None or b_at.deleted or not _zid_eq(b_at.stamp_id, g.meta_zid)):
            problems.append(f"B not put by the new instance: {b_at}")
        for name, k in (("R", kr), ("P", kp)):
            if seen and not (k in rep_at and rep_at[k].deleted):
                problems.append(f"no reply_del for {name}'s key")
            elif seen and not _zid_eq(rep_at[k].stamp_id, g.meta_zid):
                problems.append(f"{name}'s reply_del not stamped by the new instance: {rep_at[k].stamp_id}")
        report.check(sec, "3. before the new instance token appears (a SHOULD): a GET made the moment S sees it "
                          "answers A firing (t0 kept) and B under the new instance's stamps, and a reply_del "
                          "for each of R's and P's keys",
                     not problems, "; ".join(problems) or
                     f"{sorted((k.rsplit('/', 1)[1], 'del' if r.deleted else 'put') for k, r in rep_at.items())}")
        if first_a is not None and tok:
            report.note(sec, f"A's put reached S {(tok[0] - first_a.arrival_ns) / 1e6:+.3f} ms before the new "
                             "instance token did: arrival order at a subscriber, which zenoh does not keep "
                             "between a put and a liveliness declaration (SPEC-FINDINGS F-135)")
        report.check(sec, "3. S reads A confirmed", bus.reads(ka)[-1:] == [al.CONFIRMED], f"{bus.reads(ka)}")
        for name, k in (("R", kr), ("P", kp)):
            d = bus.deliveries(k, mark)
            report.check(sec, f"3. a delete of {name}'s key, which S reads retired (0.2)",
                         [x.kind for x in d] == ["delete"] and bus.reads(k) == [al.RETIRED],
                         f"{[x.kind for x in d]}, {bus.reads(k)}")
        b = bus.deliveries(kb, mark)
        report.check(sec, "3. B: a put unchanged, then a resolved put and a delete; S reads confirmed, resolved, "
                          "retired (0.2)",
                     [x.kind for x in b] == ["put", "put", "delete"] and bus.reads(kb)[-3:] == [
                         al.CONFIRMED, al.RESOLVED, al.RETIRED], f"{[x.kind for x in b]}, {bus.reads(kb)}")
        f1 = g.answers()[ka][0] if ka in g.readings else None
        report.check(sec, "4. G's reply holds A stamped by the new instance, whose meta.zid is the stamp's id",
                     _zid_eq(g.stamps.get(ka), g.meta_zid) and not g.readings[ka].previous_instance
                     and not g.foreign,
                     f"stamp {g.stamps.get(ka)}, meta.zid {g.meta_zid}")
        report.check(sec, "4. G judges A yes, firing, never previous_instance",
                     f1 is not None and _pair(f1) == "yes, firing", f1 and _pair(f1))
        if o.rust:
            got = o.adoption()
            m = re.match(r"alarm adoption source=(\S+) adopted=(\S+) retired=(\S+) resolved=(\S+) over_bound=(\S+)",
                         got)
            ok = m is not None and m.group(1) == "record" and sorted(m.group(2).split(",")) == sorted(
                [A_KEY, al.derive(*B).key]) and sorted(x.split(":")[0] for x in m.group(3).split(",")) == sorted(
                [kr.rsplit("/", 1)[1], "0000000000000001"])
            report.check(sec, "the owner's adoption: A and B adopted, R and P retired", ok, got)
        else:
            done = [w.split(":")[0] for _, w in o.adoption()]
            first_adopt = done.index("adopted") if "adopted" in done else len(done)
            report.check(sec, "the owner deletes and resolves before it adopts (§2.9, 0.2)",
                         sorted(done) == ["adopted", "adopted", "deleted", "deleted"]
                         and "deleted" not in done[first_adopt:], f"{o.adoption()}")
    finally:
        bus.close()
        tmp.cleanup()


# -- §6 A hung owner goes stale -------------------------------------------------------

def section6(report: Report, mode: Any, ctx: dict[str, Any]) -> None:
    ttl = ctx["ttl"]
    sec = f"§6 (ttl {ttl} s)"
    contract = al.load(None if ttl == al.TTL_S else ttl)
    h = al.horizon_of(contract)
    bus = Bus(mode)
    try:
        o = bus.owner(contract=contract)
        k = key(A)
        o.raise_(A, "warning")
        _wait(lambda: len(bus.s.of(k)) >= 2, ttl / 2 + 3.0)  # a confirmation
        o.stop_confirming(A)
        time.sleep(0.3)
        last = bus.s.of(k)[-1]
        wait = 905.0 if ttl == al.TTL_S else ttl + 1.5
        while (time.monotonic_ns() - last.arrival_ns) / fr.NS < wait:
            time.sleep(0.2)
        after = [x for x in bus.s.of(k) if x.arrival_ns > last.arrival_ns]
        report.check(sec, "1. S receives nothing more of A", not after, f"{len(after)} more")
        s_read = bus.read(bus.s_session, subscriber=bus.s, get=False, horizon=h)
        report.check(sec, "2. the owner's instance token is present", s_read.presence == "present", s_read.presence)
        f1, f4 = s_read.answers(h).get(k, (None, None))
        report.check(sec, "2. S judges the first question unobservable, stale, and its last yes, stale",
                     f1 is not None and _pair(f1) == "unobservable, stale" and _pair(f4) == "yes, stale",
                     f"{f1 and _pair(f1)}; {f4 and _pair(f4)}")
        g = bus.read(bus.g_session, horizon=h)
        rep = g.readings.get(k)
        age = None
        if rep is not None:
            gets = [x for x in rep.observations if isinstance(x, fr.Get)]
            age = gets[0].reply.stamp_age_ns / fr.NS if gets and gets[0].reply and gets[0].reply.stamp_age_ns else None
        report.check(sec, f"2. G's reply holds A firing under a stamp more than {h.ttl_s}.5 s old",
                     g.documents.get(k) is not None and g.documents[k].firing and age is not None
                     and age > h.ttl_s + 0.5, f"age {age}")  # type: ignore[union-attr]
        g1, g4 = g.answers(h).get(k, (None, None))
        report.check(sec, "2. G judges the same: neither firing now nor resolved (§2.7)",
                     g1 is not None and _pair(g1) == "unobservable, stale" and _pair(g4) == "yes, stale",
                     f"{g1 and _pair(g1)}; {g4 and _pair(g4)}")
        # Beyond scenarios.md: §2.7 (0.2), "An evaluation that finds the
        # condition again confirms it again", the same observation as at the
        # raise, which §2.5 makes no act.
        mark = time.monotonic_ns()
        o.raise_(A, "warning")
        _wait(lambda: len(bus.deliveries(k, mark)) >= 2, ttl / 2 + 3.0)
        again = bus.deliveries(k, mark)
        gaps = [(b.arrival_ns - a.arrival_ns) / fr.NS for a, b in zip(again, again[1:])]
        report.check(sec, "extra (§2.7, 0.2): the condition found again, its confirmations resume, unchanged",
                     len(again) >= 2 and all(_doc(x) == _doc(last) for x in again)
                     and (again[0].arrival_ns - mark) / fr.NS <= 1.0 and max(gaps) <= ttl / 2 + JITTER_S,
                     f"{len(again)} puts, first {(again[0].arrival_ns - mark) / fr.NS if again else None} s, "
                     f"gaps {[round(x, 2) for x in gaps]}")
    finally:
        bus.close()


# -- §7 At the bound ----------------------------------------------------------------------

def section7(report: Report, mode: Any, ctx: dict[str, Any]) -> None:
    sec = "§7"
    bus = Bus(mode)
    try:
        o = bus.owner(contract=al.load(), cardinality=2)
        ka, kb, kc = key(A), key(B), key(C)
        o.raise_(A, "warning")
        o.raise_(B, "warning")
        _wait(lambda: bus.s.of(ka) and bus.s.of(kb), 3.0)
        report.check(sec, "1. S receives A and B, firing",
                     all(_doc(bus.s.of(k)[0]) and _doc(bus.s.of(k)[0]).firing for k in (ka, kb)), "")  # type: ignore[union-attr]
        g = bus.read(bus.g_session)
        mark = time.monotonic_ns()
        act = o.raise_(C, "warning")
        time.sleep(1.0)
        report.check(sec, "2. the owner's writer is told the raise is refused (core §2.7)",
                     act.startswith("refused"), act)
        report.check(sec, "2. S receives no put of C, and no resolve or delete of A or B",
                     not bus.deliveries(kc, mark) and not bus.deliveries(ka, mark) and not bus.deliveries(kb, mark),
                     "")
        report.check(sec, "the descriptor states the bound of 2 (core §3.3)", g.descriptor is not None, "")
        mark = time.monotonic_ns()
        o.resolve(B)
        act = o.raise_(C, "warning")
        _wait(lambda: bus.deliveries(kc, mark), 3.0)
        b, c = bus.deliveries(kb, mark), bus.deliveries(kc, mark)
        report.check(sec, "3. S receives B resolved and deleted, then C firing",
                     [x.kind for x in b] == ["put", "delete"] and bool(c) and _doc(c[0]) is not None
                     and _doc(c[0]).firing and c[0].arrival_ns > b[-1].arrival_ns,  # type: ignore[union-attr]
                     f"B {[x.kind for x in b]}, C {[x.kind for x in c]}, act {act}")
    finally:
        bus.close()


# -- §8 An absent owner, last-known, for a tool ----------------------------------------

def section8(report: Report, mode: Any, ctx: dict[str, Any]) -> None:
    """Two tools T, as the reference's §8 runs both (0.3): a live reader,
    which knows of the owner from its step 1, and a one-shot tool, a reader
    made at step 3, which knows of it by the archive's keys (§2.15)."""
    from . import live
    from .health_scenarios import StandInArchive

    sec = "§8"
    bus = Bus(mode)
    try:
        o = bus.owner(contract=al.load())
        ka = key(A)
        o.raise_(A, "warning")
        _wait(lambda: bus.s.of(ka), 3.0)
        identity = {"type": "json:alarms.v1.Alarm", "contract": al.load().fingerprint}
        arch = StandInArchive(bus.endpoint, "lab/archive", f"zk2/{ADDRESS}/{al.PROFILE}/state/**", identity)
        bus.extra.append(arch)
        time.sleep(0.5)
        live_t = al.AlarmsReader(bus.t_session, f"zk2/lab/*/{al.PROFILE}/state/alarms/*", trust=bus.trust)
        bus.extra.append(live_t)
        pres = live.list_presence(bus.t_session, "zk2/lab/*/@zk/**")
        for a in sorted({f"{p['system']}/{p['service']}" for p in pres.instances}):
            live_t.add_owner(a)
        t1 = live_t.read(ADDRESS)
        f1 = t1.answers()[ka][0] if ka in t1.readings else None
        report.check(sec, "1. T judges A yes, firing", f1 is not None and _pair(f1) == "yes, firing", f1 and _pair(f1))
        mark = time.monotonic_ns()
        o.close()
        bus.owners.remove(o)
        time.sleep(1.0)
        got = bus.deliveries(ka, mark)
        report.check(sec, "2. a clean close: S receives no resolved put and no delete of A (§2.9)",
                     not any(x.kind == "delete" or not _doc(x).firing for x in got),  # type: ignore[union-attr]
                     f"{[x.kind for x in got]}")
        # The archive's form of the owner's alarms (core §4.4: `*` kept).
        st = live.get_state(bus.t_session, al.archive_form("lab/archive", f"zk2/{ADDRESS}/{al.PROFILE}/state/alarms/*"))
        arch_doc = next((al.decode(r.payload) for r in st.replies if not r.deleted), None)
        stamp = next((r.stamp for r in st.replies if not r.deleted), None)
        report.check(sec, "3. the archive's form of the owner's alarms holds A, firing, with its stamp (core S6)",
                     arch_doc is not None and arch_doc.firing and stamp is not None, f"{arch_doc}, {stamp}")

        def step3(name: str, t: al.AlarmsReader, scope: al.ScopeRead) -> None:
            r3 = t.read(ADDRESS, keys_known=(ka,))
            report.check(sec, f"3 ({name}). the owner's tokens are gone, a complete read; its GET draws no reply",
                         r3.presence == "absent" and not r3.stamps.get(ka), f"{r3.presence}")
            rd = r3.readings[ka]
            rd.observations = [*rd.observations, fr.Archive()]
            f3 = al.judge(rd)
            report.check(sec, f"3 ({name}). T judges A not asked, absent; the archived document is last-known",
                         _pair(f3) == "not_asked, absent", _pair(f3))
            owners = [t.read(a, keys_known=(ka,) if a == ADDRESS else ()).owner_read() for a in scope.owners]
            ans = al.any_firing("warning", owners, scope.whole)
            report.check(sec, f"3 ({name}). anything firing at or above warning over zk2/lab/*: the owner counted "
                              "not asked, and not yes",
                         ADDRESS in scope.owners and ans["answer"] != al.YES and ans["owners"][al.NOT_ASKED] >= 1,
                         f"{ans}, scope {scope}")

        found = al.owners_by_presence(bus.t_session, "lab")
        # The live reader: the owners it found (step 1), and presence now.
        step3("a live reader", live_t, found.union(al.ScopeRead(sorted(live_t.token_events))))
        # A one-shot tool: presence now, and the archive's keys (0.3).
        once_session = live.open_client(bus.endpoint)
        bus.extra.append(once_session)
        once = al.AlarmsReader(once_session, f"zk2/lab/*/{al.PROFILE}/state/alarms/*", trust=bus.trust)
        bus.extra.append(once)
        by_archive = al.owners_from_archive(once_session, "lab/archive", "lab")
        report.check(sec, "3 (a one-shot tool). it knows of the owner by the archive's keys, the read not short "
                          "(§2.15, 0.3)", by_archive.owners == [ADDRESS] and not by_archive.archive_short,
                     f"{by_archive}")
        # 0.5: the archive's read plays no part in whether the scope is whole.
        step3("a one-shot tool", once, found.union(by_archive))
    finally:
        bus.close()


# -- §9 A key that does not re-derive, for a tool ---------------------------------------

def section9(report: Report, mode: Any, ctx: dict[str, Any]) -> None:
    sec = "§9"
    address = "lab/liar"
    bus = Bus("zk2py", address=address)
    try:
        liar = al.AlarmsOwner("lab", "liar", connect=bus.endpoint)
        liar.start()
        bus.extra.append(liar)
        base = f"zk2/{address}/{al.PROFILE}/state/alarms/"
        a_doc = {"timestamp": 1760950000000, "rule": A[0], "labels": A[1], "severity": "warning",
                 "state": "firing", "summary": "link to r2 down on eth0"}
        bad = {**a_doc, "rule": "link\ndown", "labels": {"port": "22"}}
        puts = [(base + "0000000000000001", a_doc, "no, mismatch"),
                (base + "16991d91fb818ca6", bad, "yes, sanitized"),
                (base + A_KEY, a_doc, "yes, strict")]
        for k, d, _ in puts:
            liar.owner.set_state(k, al.document_bytes(d))
        time.sleep(0.5)
        t = bus.read(bus.t_session, address)
        for k, d, want in puts:
            doc = t.documents.get(k)
            ref = al.parse_key(k)
            got = al.rederives(ref["alert_key"], doc if doc is not None else "delete") if ref else None
            report.check(sec, f"{k.rsplit('/', 1)[1]}: {want}", got is not None and _pair(got) == want,
                         got and _pair(got))
    finally:
        bus.close()


# -- §10 Anything firing, for a tool ------------------------------------------------------

def section10(report: Report, mode: Any, ctx: dict[str, Any]) -> None:
    """T is :class:`zk2py.alarms.AlarmsReader`: one subscription, then a GET
    per owner at each re-read, its window as §2.15 (0.3) counts one. Step 2
    listens until the window, from the declaration plus the reply timeout to
    the watch after it before the instant T judges (observer §5.2: the
    longer of the lease and wait_before_close, plus the reply timeout), is longer
    than the horizon. Then, beyond scenarios.md, 0.3's two breaks of a
    window: a gap in lab/b's presence, and a loss."""
    from . import live

    ttl = ctx["ttl"]
    sec = f"§10 (ttl {ttl} s)"
    contract = al.load(None if ttl == al.TTL_S else ttl)
    h = al.horizon_of(contract)
    bus = Bus(mode, selectors=["zk2/lab/none"])
    try:
        oa = bus.owner("lab/a", contract=contract)
        ob = bus.owner("lab/b", contract=contract, tokenless=True)
        oa.raise_(A, "warning")
        time.sleep(0.5)
        t = al.AlarmsReader(bus.t_session, f"zk2/lab/*/{al.PROFILE}/state/alarms/*", ("lab/a", "lab/b"),
                            trust=bus.trust)
        bus.extra.append(t)

        scope_now: list[al.ScopeRead | None] = [None]

        def scope_whole() -> bool:
            # the presence read scope() made, asked at the question (0.6)
            sr = scope_now[0]
            assert sr is not None, "no presence read of the scope: scope() was not called"
            return t.scope_whole(sr)
        reporter = al.ChangeReporter()

        def scope(get: bool = True) -> tuple[list[al.OwnerReading], list[str]]:
            # The owners in scope by presence (§2.13), and whether that read
            # is whole (§2.15, 0.5).
            sr = t.scope_read("lab")
            scope_now[0] = sr
            pres = live.list_presence(bus.t_session, "zk2/lab/*/@zk/alive/**")
            alive = sorted({f"{p['system']}/{p['service']}" for p in pres.alive if p["iface"] == al.PROFILE})
            return [t.read(a, horizon=h, get=get) for a in sr.owners], alive

        def report_changes(reads: list[al.OwnerReading],
                           partial: bool = False) -> list[tuple[str | None, str, str, str | None]]:
            # §2.16 (0.4): each key's first answer, after its change; a
            # partial reading is one owner's keys
            feed = {}
            for r in reads:
                for k, (f1, _) in r.answers(h).items():
                    d = r.documents.get(k)
                    feed[k] = (f1, None if d is None else d.severity, None)
            return [(x.key, x.answer, x.reason, x.severity) for x in reporter.update(feed, partial=partial)]

        reads, alive = scope()
        by = {r.address: r for r in reads}
        ka = key(A, "lab/a")
        kb = key(B, "lab/b")
        got = report_changes(reads)
        report.check(sec, "extra (§2.16, 0.4): at the first reading, the alarm firing is reported, with its answer",
                     got == [(ka, al.YES, "firing", "warning")], f"{got}")
        report.check(sec, "1. T finds lab/b by its descriptor, though it holds no alive/alarms.v1 token (§2.13)",
                     "lab/b" in by and by["lab/b"].descriptor == {"lists": True, "token": False} and "lab/b" not in alive,
                     f"{by.get('lab/b') and by['lab/b'].descriptor}, alive {alive}")
        w = al.any_firing("warning", [r.owner_read(h) for r in reads], scope_whole())
        report.check(sec, "1. at warning: yes, any_firing, 1", (w["answer"], w["reason"], w["firing"]) ==
                     (al.YES, "any_firing", 1), f"{w}")
        c = al.any_firing("critical", [r.owner_read(h) for r in reads], scope_whole())
        report.check(sec, "1. at critical: unobservable, owner_unobservable",
                     (c["answer"], c["reason"]) == (al.UNOBSERVABLE, "owner_unobservable"), f"{c}")
        report.check(sec, "1. lab/a established (answered), lab/b not (window_short)",
                     by.get("lab/a") is not None and by["lab/a"].verdict.expect() == {"reason": "answered", "verdict": "established"}
                     and by.get("lab/b") is not None and by["lab/b"].verdict.expect() ==
                     {"reason": "window_short", "verdict": "unobservable"},
                     f"{[(r.address, r.verdict.expect()) for r in reads]}")
        # The horizon's window (plus a second's margin) after the
        # declaration plus the reply timeout, and the watch after it, as
        # observer §5.2 says: the longer of the lease and wait_before_close,
        # plus the reply timeout (§2.15, 0.3; scenarios §10 step 2 as read
        # against observer.md 0.6). The reply timeout is counted twice, as
        # the old sum's two reply timeouts were: once explicitly, for the
        # window's first instant (the declaration plus the reply timeout),
        # and once inside watch_after_ns, for the withdrawal's transit.
        need_ns = int((h.ttl_s + 1.0) * fr.NS) + t.reply_timeout_ns + t.watch_after_ns
        # A bound on the wait: the length itself from now, plus 30 s. A
        # declaration instant that leaves the length unreached by then (one
        # in the future, or a clock that does not advance) is the harness's
        # failure, never the reader's.
        bound_ns = time.monotonic_ns() + need_ns + 30 * fr.NS
        while time.monotonic_ns() - t.declared_ns < need_ns and time.monotonic_ns() < bound_ns:
            time.sleep(0.2)
        if time.monotonic_ns() - t.declared_ns < need_ns:
            report.check(sec, "harness: step 2's listening reached its length within its bound", False,
                         f"declared at {t.declared_ns} ns, now {time.monotonic_ns()} ns, need {need_ns} ns")
            return
        reads, _ = scope()
        by = {r.address: r for r in reads}
        c = al.any_firing("critical", [r.owner_read(h) for r in reads], scope_whole())
        report.check(sec, f"2. a window over {h.ttl_s} s, lossless, both present: at critical no, none_firing",
                     (c["answer"], c["reason"]) == (al.NO, "none_firing"), f"{c}, window {by['lab/b'].window}")
        report.check(sec, "2. lab/b established by its window",
                     by.get("lab/b") is not None and by["lab/b"].verdict.reason == "window",
                     f"{by.get('lab/b') and by['lab/b'].verdict.expect()}")
        got = report_changes(reads)
        report.check(sec, "extra (§2.16): A's confirmations since are no change to report", got == [], f"{got}")
        ob.raise_(B, "critical")
        time.sleep(0.5)
        reads, _ = scope()
        c = al.any_firing("critical", [r.owner_read(h) for r in reads], scope_whole())
        report.check(sec, "3. lab/b raises B at critical: yes, any_firing, 1",
                     (c["answer"], c["reason"], c["firing"]) == (al.YES, "any_firing", 1), f"{c}")
        got = report_changes(reads)
        report.check(sec, "extra (§2.16): B's raise is reported as the answer after it, at its severity",
                     got == [(kb, al.YES, "firing", "critical")], f"{got}")
        # Beyond scenarios.md (0.3): a gap in lab/b's presence breaks the
        # window until a complete GET after it; so does a loss.
        ob.resolve(B)
        ob.crash()
        ob.restart()
        _wait(lambda: len(t.gaps("lab/b")) >= 2, 5.0)
        time.sleep(0.5)
        rb = t.read("lab/b", horizon=h, get=False)
        report.check(sec, "extra (0.3): a gap in lab/b's presence, no GET since: owner_gap",
                     rb.verdict.expect() == {"reason": "owner_gap", "verdict": "unobservable"},
                     f"{rb.verdict.expect()}, gaps {len(t.gaps('lab/b'))}")
        rb = t.read("lab/b", horizon=h)
        report.check(sec, "extra (0.3): a complete GET after it heals the gap, and the window runs again from it",
                     rb.verdict.expect() == {"reason": "window_short", "verdict": "unobservable"}
                     and rb.window is not None and rb.window.present_throughout,
                     f"{rb.verdict.expect()}, window {rb.window}")
        got = report_changes([rb], partial=True)
        report.check(sec, "extra (§2.16): B's resolve and delete are reported as the answer after them, no",
                     len(got) == 1 and got[0][:2] == (kb, al.NO), f"{got}")
        t.note_loss()
        ra = t.read("lab/a", horizon=h, get=False)
        ka = key(A, "lab/a")
        fa = ra.answers(h).get(ka, (None, None))[0]
        report.check(sec, "extra (0.3): a loss, no GET since: window_lost, and A unobservable, incomplete "
                          "(§2.14 step 4)",
                     ra.verdict.expect() == {"reason": "window_lost", "verdict": "unobservable"}
                     and fa is not None and _pair(fa) == "unobservable, incomplete",
                     f"{ra.verdict.expect()}, A {fa and _pair(fa)}")
        got = report_changes([ra], partial=True)
        report.check(sec, "extra (§2.16): the loss is reported once for the scope, and no key for it (the MAY)",
                     got == [(None, al.UNOBSERVABLE, "incomplete", None)], f"{got}")
        ra = t.read("lab/a", horizon=h)
        fa = ra.answers(h).get(ka, (None, None))[0]
        report.check(sec, "extra (0.3): a complete GET after the loss heals it: lab/a answered, A yes, firing",
                     ra.verdict.expect() == {"reason": "answered", "verdict": "established"}
                     and fa is not None and _pair(fa) == "yes, firing",
                     f"{ra.verdict.expect()}, A {fa and _pair(fa)}, window {ra.window}")
        got = report_changes([ra], partial=True)
        report.check(sec, "extra (§2.16): healed, A still firing at warning: no key to report, the scope "
                          "observable again", got == [(None, "observable", "healed", None)], f"{got}")
    finally:
        bus.close()


SECTIONS = {"1": section1, "2": section2, "3": section3, "4": section4, "5": section5, "6": section6,
            "7": section7, "8": section8, "9": section9, "10": section10}


def run(report: Report, mode: Any, only: list[str] | None, ctx: dict[str, Any]) -> None:
    for name in [n for n in SECTIONS if not only or n in only]:
        try:
            SECTIONS[name](report, mode, ctx)
        except NotImplementedError as e:
            report.cannot(f"§{name}", str(e))
        except Exception as e:  # noqa: BLE001 - a section that cannot run is a failure
            import traceback

            traceback.print_exc()
            report.check(f"§{name}", "the section ran", False, f"{type(e).__name__}: {e}")


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="python -m zk2py.alarms_scenarios", description=__doc__.split("\n")[0])
    ap.add_argument("--only", action="append", choices=sorted(SECTIONS, key=int), help="run only these sections")
    ap.add_argument("--owner", type=Path, default=os.environ.get("ZK2PY_OWNER") or None,
                    help="run against the Rust owner example at this path (--alarms)")
    ap.add_argument("--ttl", type=int, default=4, help="the short-ttl copy's horizon for §2, §4, §6 and §10")
    ap.add_argument("--full", action="store_true", help="run §2, §4, §6 and §10 at the contract's 900 s")
    ap.add_argument("--quick", action="store_true", help="skip §3 step 3's 65 s wait")
    args = ap.parse_args(argv)
    mode: Any = "zk2py"
    if args.owner is not None:
        exe = Path(args.owner)
        if not exe.is_file():
            print(f"error: {exe} not found", file=sys.stderr)
            return 2
        mode = exe
    ctx = {"ttl": al.TTL_S if args.full else args.ttl, "quick": args.quick}
    report = Report()
    run(report, mode, args.only, ctx)
    passed = sum(1 for r in report.rows if r[2])
    failed = len(report.rows) - passed
    who = "zk2py's owner" if mode == "zk2py" else "the owner example"
    print(f"alarms scenarios ({who}): {passed} passed, {failed} failed, {len(report.not_run)} not run")
    for sec, name, ok, detail in report.rows:
        if not ok:
            print(f"FAIL [{sec}] {name}: {detail}")
    for sec, why in report.not_run:
        print(f"NOT RUN [{sec}] {why}")
    # A NOT RUN is a partial run, which a gate must not read as green,
    # unless the caller asked for a subset (--only, --quick).
    subset = bool(args.only) or args.quick
    return 1 if failed else 2 if report.not_run and not subset else 0


if __name__ == "__main__":
    rc = main()
    sys.stdout.flush()
    sys.stderr.flush()
    # zenoh-python can hang tearing down at interpreter exit; every session
    # here is already closed.
    os._exit(rc)

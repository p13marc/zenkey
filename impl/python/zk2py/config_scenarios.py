"""The ``config.v1`` scenarios (``spec/profiles/config/scenarios.md``, text
0.1) against zk2py's owner, on in-process zenoh-python routers: ``python -m
zk2py.config_scenarios [--only 1 …]``.

- **The bus** is the text's: R1, a router with timestamping on; the owner,
  the caller C, the subscriber S and the tool T are its clients, each with
  a session of its own. Timeouts are 1 s, a call's 2 s.
- **The owner** is :class:`zk2py.config.ConfigOwner`, ``lab/rf0``, with the
  schema of ``conformance/lifecycle.json``'s ``rf``, its startup values, a
  persisted layer, a floor of 10 s and ``apply_at`` 1 s after it receives a
  reach ``set``. Its device is :class:`zk2py.config.SimDevice`: a test sets
  each act's result, and reads what the device holds and every act it saw.
- **C** calls through zk2py's client (``live.call``: one concrete key,
  ``BestMatching``, the first value or envelope), with the call metadata
  ``{"actor": "ops", "request_id": …}`` as its attachment (core O7).
- **S** subscribes to ``zk2/lab/rf0/config.v1/**`` and holds a liveliness
  subscriber on ``zk2/lab/rf0/@zk/**`` before the owner starts.
- **§8** runs R1 with zk2py's generated ``access_control`` block under
  ``deny`` (``zk2py.acl``) and usrpwd.
- **§10** drops step 1's reply at the owner, which keeps it as the key's
  first answer: C's call draws nothing.
- **§12**'s author is a zk2py owner of ``interop/config/zk2py_author.v1``,
  whose ``state/desired`` the deployment binds to ``lab/rf0`` (the owner's
  ``desired_key``).
- **§13**'s face is ``constrained.md`` §1 step 2's attachment, as
  ``health.v1``'s scenarios §8 run it: C a client of the owner's router,
  whose access control denies ``zk2/**/@zk/**`` and the subscription to
  ``zk2/lab/rf0/*/state/**`` to C's principal, both ways, with C's link
  batch at 1 KB. The link is not shaped to a serial line.
- **§14**'s tool is zk2py's reader of §5 (:func:`zk2py.config.read`).

The long sections (§1, §6, §10, §14) run beside the others, each on a bus
of its own. Exit 0 when every check passes, 1 when any fails.
"""

from __future__ import annotations

import argparse
import json
import os
import queue
import sys
import tempfile
import threading
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Any

from . import config as cf
from . import freshness as fr

REPO = Path(__file__).resolve().parents[3]
LIFECYCLE = REPO / "spec" / "profiles" / "config" / "conformance" / "lifecycle.json"
AUTHOR = REPO / "impl" / "python" / "interop" / "config" / "zk2py_author.v1.toml"
NAV = REPO / "examples" / "zk2" / "walkthrough" / "nav.v2.toml"
JITTER_S = 0.5
NS = cf.NS
STARTUP = {"duty": {"fraction_ppm": {"value": 10000, "source": "file"}},
           "radio": {"frequency_hz": {"value": 868100000, "source": "file"}},
           "link": {"net_id": {"value": 1, "source": "default"}},
           "sim": {"pin": {"value": "0000", "source": "file"}},
           "queue": {"tx_queue_len": {"value": 100, "source": "default"}}}


def rf_schema() -> list[dict[str, Any]]:
    return json.loads(LIFECYCLE.read_text())["schemas"]["rf"]


class Report:
    def __init__(self) -> None:
        self.rows: list[tuple[str, str, bool, str]] = []
        self.lock = threading.Lock()

    def check(self, section: str, name: str, ok: bool, detail: str = "") -> bool:
        with self.lock:
            self.rows.append((section, name, ok, detail))
            print(f"{'PASS' if ok else 'FAIL'} [{section}] {name}" + (f": {detail}" if detail else ""), flush=True)
        return ok


@dataclass
class Called:
    """One call as C saw it: a value, a refusal, or nothing (core O5)."""

    kind: str                          # "value", "refused", "silent"
    doc: Any = None                    # the value, decoded
    envelope: dict[str, Any] | None = None
    raw: bytes = b""
    at_ns: int = 0                     # C's clock when the call returned

    @property
    def code(self) -> str | None:
        return None if self.envelope is None else self.envelope["code"]

    def short(self) -> str:
        if self.kind == "refused":
            e = self.envelope or {}
            return f"refused {e.get('code')}" + (f" ({e.get('cause')})" if e.get("cause") else "") + \
                (f" {e.get('detail')}" if e.get("detail") is not None else "") + f": {e.get('message')}"
        if self.kind == "value":
            return json.dumps(cf.project_reply(self.doc) if isinstance(self.doc, dict) and len(self.doc) == 1
                              else self.doc, sort_keys=True)
        return self.kind


def call(session: Any, address: str, op: str, request: Any, *, actor: str | None = "ops",
         request_id: str | None = None, timeout: float = 2.0) -> Called:
    from . import live
    from .slug import slug

    key = f"zk2/{address}/{cf.IFACE}/@op/{op}" if not op.startswith("set/") else \
        f"zk2/{address}/{cf.IFACE}/@op/set/{slug(op[4:])}"
    meta = {k: v for k, v in (("actor", actor), ("request_id", request_id)) if v is not None}
    res = live.call(session, key, cf.encode_json(request), encoding="application/json", timeout=timeout, first=True,
                    attachment=json.dumps(meta).encode() if meta else None)
    at = time.time_ns()
    for r in res.replies:
        if r.kind == "value":
            return Called("value", json.loads(r.payload), raw=r.payload, at_ns=at)
        if r.kind == "envelope":
            return Called("refused", envelope=r.envelope, raw=r.payload, at_ns=at)
    return Called("silent", at_ns=at)


def _wait(pred: Any, timeout: float, step: float = 0.05) -> bool:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if pred():
            return True
        time.sleep(step)
    return bool(pred())


class Tokens:
    """A liveliness subscriber with history, recording (utc ns, kind, key)."""

    def __init__(self, session: Any, selector: str, on_put: Any = None):
        import zenoh

        self.rows: list[tuple[int, str, str]] = []
        self.lock = threading.Lock()
        self.on_put = on_put
        self.sub = session.liveliness().declare_subscriber(selector, zenoh.handlers.Callback(self._got),
                                                           history=True)

    def _got(self, smp: Any) -> None:
        import zenoh

        kind = "put" if smp.kind == zenoh.SampleKind.PUT else "delete"
        with self.lock:
            self.rows.append((time.time_ns(), kind, str(smp.key_expr)))
        if kind == "put" and self.on_put is not None:
            self.on_put(str(smp.key_expr))

    def of(self, part: str) -> list[tuple[int, str, str]]:
        with self.lock:
            return [r for r in self.rows if part in r[2]]

    def close(self) -> None:
        try:
            self.sub.undeclare()
        except Exception:  # noqa: BLE001 - closing anyway
            pass


class Bus:
    """R1, and C, S and T as client sessions of it; S's subscriptions made
    before any owner starts."""

    def __init__(self, addresses: tuple[str, ...] = ("lab/rf0",), *, router: Any = None,
                 sessions: dict[str, Any] | None = None, selectors: list[str] | None = None) -> None:
        from . import live
        from .live_interop import _r1

        if router is None:
            self.router, self.endpoint, _ = _r1()
        else:
            self.router, self.endpoint = router
        sessions = sessions or {}
        self.c = sessions.get("C") or live.open_client(self.endpoint)
        self.s_session = sessions.get("S") or live.open_client(self.endpoint)
        self.t = sessions.get("T") or live.open_client(self.endpoint)
        self.s = fr.Subscriber(self.s_session, selectors or [f"zk2/{a}/{cf.IFACE}/**" for a in addresses])
        self.tokens = Tokens(self.s_session, "zk2/lab/*/@zk/**")
        self.owners: list[cf.ConfigOwner] = []
        self.extra: list[Any] = []
        time.sleep(0.2)

    def owner(self, service: str = "rf0", start: bool = True, **kw: Any) -> cf.ConfigOwner:
        kw.setdefault("schema", rf_schema())
        kw.setdefault("startup", STARTUP)
        kw.setdefault("floor_s", 10)
        kw.setdefault("apply_after_s", 1.0)
        kw.setdefault("connect", self.endpoint)
        o = cf.ConfigOwner("lab", service, **kw)
        if start:
            o.start()
            time.sleep(0.3)
        self.owners.append(o)
        return o

    def drop(self, o: cf.ConfigOwner) -> None:
        o.close()
        if o in self.owners:
            self.owners.remove(o)

    def events(self, address: str = "lab/rf0", since_utc: int = 0) -> list[tuple[int, dict[str, Any]]]:
        prefix = f"zk2/{address}/{cf.IFACE}/events/change/"
        with self.s.lock:
            keys = [k for k in self.s.deliveries if k.startswith(prefix)]
        out = [(x.receipt_utc_ns, json.loads(x.payload)) for k in keys for x in self.s.of(k)
               if x.kind == "put" and x.receipt_utc_ns >= since_utc]
        return sorted(out, key=lambda e: e[0])

    def docs(self, which: str, address: str = "lab/rf0", since_utc: int = 0) -> list[tuple[int, dict[str, Any]]]:
        key = f"zk2/{address}/{cf.IFACE}/state/{which}"
        return [(x.receipt_utc_ns, json.loads(x.payload)) for x in self.s.of(key)
                if x.kind == "put" and x.receipt_utc_ns >= since_utc]

    def get(self, which: str, address: str = "lab/rf0", session: Any = None) -> dict[str, Any] | None:
        from . import live

        st = live.get_state(session or self.c, f"zk2/{address}/{cf.IFACE}/state/{which}")
        mine = [r for r in st.replies if not r.deleted]
        return json.loads(mine[-1].payload) if mine else None

    def close(self) -> None:
        self.s.close()
        self.tokens.close()
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
        for x in (self.c, self.s_session, self.t, self.router):
            try:
                x.close()
            except Exception:  # noqa: BLE001
                pass


def _param(view: dict[str, Any] | None, group: str, name: str) -> dict[str, Any] | None:
    if view is None:
        return None
    g = cf.find_group(view["groups"], group)
    return None if g is None else cf.find_param(g, name)


def _val(doc: dict[str, Any] | None, group: str, name: str) -> Any:
    if doc is None:
        return None
    if "groups" in doc:
        p = _param(doc, group, name)
        return None if p is None else p.get("value")
    return doc.get("values", {}).get(group, {}).get(name)


def _applied_doc(c: Called) -> dict[str, Any] | None:
    return c.doc.get("applied") if c.kind == "value" and isinstance(c.doc, dict) else None


# -- §1 ------------------------------------------------------------------------------

def section1(report: Report) -> None:
    from . import live

    sec = "§1 a hot set, read back"
    bus = Bus()
    try:
        seen: list[tuple[dict | None, dict | None]] = []
        work: queue.Queue = queue.Queue()

        def worker() -> None:
            while True:
                key = work.get()
                if key is None:
                    return
                v = live.get_state(bus.t, "zk2/lab/rf0/config.v1/state/view")
                s = live.get_state(bus.t, "zk2/lab/rf0/config.v1/state/status")
                seen.append((json.loads(v.replies[0].payload) if v.replies else None,
                             json.loads(s.replies[0].payload) if s.replies else None))

        threading.Thread(target=worker, daemon=True).start()
        t_tokens = Tokens(bus.t, "zk2/lab/*/@zk/**",
                          on_put=lambda k: work.put(k) if "/@zk/instance/" in k and "lab/rf0" in k else None)
        bus.extra.append(t_tokens)
        o = bus.owner()
        _wait(lambda: bool(seen), 3.0)
        v, s = seen[0] if seen else (None, None)
        ok1 = (v is not None and s is not None and v["revision"] == 0 and s["revision"] == 0
               and "pending" not in v and "last_change" not in v and v["confirm_floor_s"] == 10
               and s["confirm_floor_s"] == 10 and all("writable" in g for g in v["groups"])
               and "value" not in (_param(v, "sim", "pin") or {"value": 1}) and cf.status_of(v) == s)
        report.check(sec, "step 1: T's GETs at the owner's instance token answer the view and the status: revision "
                          "0, nothing pending, no last change, confirm_floor_s 10, every group with writable, pin "
                          "without a value: put before the tokens (§2.3)",
                     ok1, f"view revision {v and v['revision']}, status {s}")
        t2 = time.time_ns()
        r = call(bus.c, "lab/rf0", "set/duty", {"values": {"fraction_ppm": 20000}}, request_id="r1")
        a = _applied_doc(r)
        p = _param(a, "duty", "fraction_ppm") or {}
        ok2 = (a is not None and p.get("value") == 20000 and p.get("source") == "runtime" and p.get("startup") == 10000
               and a["revision"] == 1 and a.get("last_change", {}).get("groups") == ["duty"])
        token = (a or {}).get("last_change", {}).get("token")
        time.sleep(0.5)
        evs = bus.events(since_utc=t2)
        vs, ss = bus.docs("view", since_utc=t2), bus.docs("status", since_utc=t2)
        ok2b = (len(evs) == 1 and evs[0][1]["outcome"] == "applied" and evs[0][1]["token"] == token
                and evs[0][1]["edits"] == [{"group": "duty", "parameter": "fraction_ppm", "old": 10000, "new": 20000}]
                and evs[0][1].get("actor") == "ops" and len(vs) == 1 and len(ss) == 1
                and vs[0][1]["revision"] == 1 and ss[0][1]["revision"] == 1)
        report.check(sec, "step 2: {applied: <view>}: fraction_ppm 20000 at runtime, startup 10000, revision 1, "
                          "last_change {token, [duty]}; S receives one event applied with that token, the edit "
                          "10000 -> 20000 and actor ops, and V and St once each at revision 1",
                     ok2 and ok2b, f"{r.short()}; events {[e for _, e in evs]}; V {len(vs)}, St {len(ss)}")
        gv, gs = bus.get("view"), bus.get("status")
        report.check(sec, "step 3: V and St answer revision 1, and agree (§2.19)",
                     gv is not None and gs is not None and gv["revision"] == 1 and cf.status_of(gv) == gs,
                     f"view revision {gv and gv['revision']}, status {gs}")
        time.sleep(35.0)
        key_v, key_s = o.view_key, o.status_key
        ok4 = True
        detail = []
        for key in (key_v, key_s):
            recs = bus.s.of(key)
            after = [x for x in recs if x.receipt_utc_ns >= t2]
            gaps = [(b.arrival_ns - a_.arrival_ns) / NS for a_, b in zip(after, after[1:])]
            reput = len(after) >= 2 and len({x.payload for x in after}) == 1 and max(gaps) <= 30 + JITTER_S
            deletes = [x for x in recs if x.kind == "delete"]
            ok4 = ok4 and reput and not deletes
            detail.append(f"{key.rsplit('/', 1)[-1]}: {len(after)} deliveries, gaps {[round(g, 1) for g in gaps]} s, "
                          f"{len(deletes)} deletes")
        report.check(sec, "step 4: in 35 s with no change, S receives V and St again within 30 s, unchanged "
                          "(freshness.v1 §2.4), and no delete of either", ok4, "; ".join(detail))
        work.put(None)
    finally:
        bus.close()


# -- §2 ------------------------------------------------------------------------------

def section2(report: Report) -> None:
    sec = "§2 a reach set, confirmed"
    bus = Bus()
    try:
        o = bus.owner()
        t0 = time.time_ns()
        r1 = call(bus.c, "lab/rf0", "set/radio", {"values": {"frequency_hz": 869525000}, "confirm_s": 120},
                  request_id="r1")
        pend = r1.doc.get("pending") if r1.kind == "value" else None
        ok = pend is not None
        if ok:
            apply_at, dl = cf.parse_time(pend["apply_at"]), cf.parse_time(pend["deadline"])
            ok = (dl - apply_at == 119 * NS and t0 - NS // 2 <= apply_at - NS <= r1.at_ns)
        token = (pend or {}).get("token")
        r2 = call(bus.c, "lab/rf0", "confirm", {"token": token}, request_id="r2")
        report.check(sec, "step 2: a confirm before apply_at is busy: not yet applied (§2.9)",
                     r2.code == "busy", r2.short())
        _wait(lambda: any(d.get("pending", {}).get("applied") for _, d in bus.docs("status")), 5.0)
        applies = o.device.acts("apply")
        sts = [d for _, d in bus.docs("status", since_utc=t0)]
        seq_ok = any(d.get("pending", {}).get("applied") is False for d in sts) and any(
            d.get("pending", {}).get("applied") is True and _val(d, "radio", "frequency_hz") == 869525000 for d in sts)
        first_true = next((i for i, d in enumerate(sts) if d.get("pending", {}).get("applied") is True), -1)
        last_false = max((i for i, d in enumerate(sts) if d.get("pending", {}).get("applied") is False), default=-1)
        report.check(sec, "step 1: {pending: {token, apply_at, deadline}}, apply_at 1 s and the deadline 120 s after "
                          "the owner received the call; the device's apply is later than the reply's arrival; S "
                          "receives St with pending.applied false, then true with frequency_hz 869525000",
                     ok and len(applies) == 1 and applies[0][0] > r1.at_ns and seq_ok and last_false < first_true,
                     f"{r1.short()}; device apply at +{(applies[0][0] - r1.at_ns) / NS:.3f} s after the reply"
                     if applies else f"{r1.short()}; no apply")
        t3 = time.time_ns()
        r3 = call(bus.c, "lab/rf0", "confirm", {"token": token}, request_id="r3")
        v3 = r3.doc.get("view") if r3.kind == "value" else None
        time.sleep(0.5)
        evs = bus.events(since_utc=t3)
        report.check(sec, "step 3: {view: <view>}: nothing pending, last_change the token; S receives one event "
                          "confirmed, actor ops and the request_id of step 1's call, not step 3's (§2.17)",
                     v3 is not None and "pending" not in v3 and v3.get("last_change", {}).get("token") == token
                     and len(evs) == 1 and evs[0][1]["outcome"] == "confirmed" and evs[0][1].get("actor") == "ops"
                     and evs[0][1].get("request_id") == "r1",
                     f"{r3.short()}; events {[e for _, e in evs]}")
        r4 = call(bus.c, "lab/rf0", "confirm", {"token": token}, request_id="r4")
        report.check(sec, "step 4: the same confirm again is not_found (§2.10)", r4.code == "not_found", r4.short())
    finally:
        bus.close()


# -- §3 ------------------------------------------------------------------------------

def section3(report: Report) -> None:
    sec = "§3 a reach set rolled back at its deadline"
    bus = Bus(("lab/rf0", "lab/rf1"))
    try:
        o = bus.owner()
        o1 = bus.owner("rf1")
        o1.device.next_apply.append({"refused": "the PLL did not lock"})
        t0 = time.monotonic()
        t0u = time.time_ns()
        r1 = call(bus.c, "lab/rf0", "set/radio", {"values": {"frequency_hz": 869525000}, "confirm_s": 5})
        r1b = call(bus.c, "lab/rf1", "set/radio", {"values": {"frequency_hz": 869525000}, "confirm_s": 5})
        pend = r1.doc.get("pending") if r1.kind == "value" else {}
        dl, apply_at = cf.parse_time(pend["deadline"]), cf.parse_time(pend["apply_at"])
        report.check(sec, "step 1: the window is the floor: the reply's deadline is 10 s after the call (§2.8)",
                     dl - apply_at == 9 * NS and abs(dl - 10 * NS - t0u) < NS, r1.short())
        time.sleep(max(0.0, t0 + 10.5 - time.monotonic()))
        evs = bus.events(since_utc=t0u)
        st = bus.get("status")
        v = bus.get("view")
        r2 = call(bus.c, "lab/rf0", "confirm", {"token": pend["token"]})
        undos = o.device.acts("undo")
        report.check(sec, "step 2: before the GET, S received one event rolled-back; St's frequency_hz is 868100000 "
                          "again, at source file, revision 2; the device holds the startup frequency; the confirm "
                          "is not_found; the device saw exactly one undo",
                     len(evs) == 1 and evs[0][1]["outcome"] == "rolled-back" and _val(st, "radio", "frequency_hz")
                     == 868100000 and (_param(v, "radio", "frequency_hz") or {}).get("source") == "file"
                     and st is not None and st["revision"] == 2
                     and o.device.values["radio"]["frequency_hz"] == 868100000 and r2.code == "not_found"
                     and len(undos) == 1,
                     f"events {[e['outcome'] for _, e in evs]}, St {st}, confirm {r2.short()}, undos {len(undos)}")
        evs1 = bus.events("lab/rf1", since_utc=t0u)
        p1 = r1b.doc.get("pending") if r1b.kind == "value" else {}
        at1 = cf.parse_time(p1["apply_at"]) if p1 else 0
        report.check(sec, "step 3: lab/rf1, whose device refuses the apply at apply_at, publishes one event "
                          "rolled-back at apply_at, with no undo made: nothing was applied (§2.9, §2.11)",
                     len(evs1) == 1 and evs1[0][1]["outcome"] == "rolled-back" and abs(evs1[0][0] - at1) < NS
                     and not o1.device.acts("undo") and evs1[0][1]["revision"] == 0,
                     f"events {[(round((t - at1) / NS, 3), e['outcome'], e['revision']) for t, e in evs1]}, "
                     f"undos {len(o1.device.acts('undo'))}")
    finally:
        bus.close()


# -- §4 ------------------------------------------------------------------------------

def section4(report: Report) -> None:
    sec = "§4 an undo that fails"
    bus = Bus()
    try:
        o = bus.owner()
        o.device.next_undo.append({"failed": "the radio did not answer", "readback": {}})
        t0 = time.monotonic()
        t0u = time.time_ns()
        r1 = call(bus.c, "lab/rf0", "set/radio", {"values": {"frequency_hz": 869525000}, "confirm_s": 10})
        dl = cf.parse_time(r1.doc["pending"]["deadline"]) if r1.kind == "value" else 0
        _wait(lambda: bool(bus.events(since_utc=t0u)), 15.0)
        evs = bus.events(since_utc=t0u)
        report.check(sec, "step 1: at the deadline, S receives one event, partial",
                     len(evs) == 1 and evs[0][1]["outcome"] == "partial" and abs(evs[0][0] - dl) < NS,
                     f"{[(round((t - dl) / NS, 3), e['outcome']) for t, e in evs]}")
        time.sleep(max(0.0, t0 + 20 - time.monotonic()))
        v = bus.get("view")
        p = _param(v, "radio", "frequency_hz") or {}
        undos = o.device.acts("undo")
        report.check(sec, "step 2: V shows frequency_hz without a value, the device reporting none; nothing "
                          "pending; the device saw exactly one undo, and no second in the 10 s after it (§2.11)",
                     v is not None and "value" not in p and "pending" not in v and len(undos) == 1
                     and undos[0][0] <= time.time_ns() - 9 * NS,
                     f"frequency_hz {p}, pending {v and v.get('pending')}, undos {len(undos)}")
    finally:
        bus.close()


# -- §5 ------------------------------------------------------------------------------

def section5(report: Report) -> None:
    from . import live

    sec = "§5 a join, and a second writer"
    bus = Bus()
    c2 = live.open_client(bus.endpoint)
    bus.extra.append(c2)
    try:
        bus.owner()
        t0u = time.time_ns()
        r1 = call(bus.c, "lab/rf0", "set/radio", {"values": {"frequency_hz": 869525000}, "confirm_s": 120},
                  request_id="r1")
        token = r1.doc["pending"]["token"] if r1.kind == "value" else None
        report.check(sec, "step 1: a pending reply", token is not None, r1.short())
        r2 = call(c2, "lab/rf0", "set/link", {"values": {"net_id": 7}, "confirm_s": 60}, actor="ops2",
                  request_id="r2")
        st = bus.get("status", session=c2)
        report.check(sec, "step 2: C2's set is busy, its message naming the pending token; C2 reads the token from "
                          "St's pending (§2.7)",
                     r2.code == "busy" and token in (r2.envelope or {}).get("message", "")
                     and (st or {}).get("pending", {}).get("token") == token,
                     f"{r2.short()}; St pending {(st or {}).get('pending')}")
        r3 = call(bus.c, "lab/rf0", "set/link", {"values": {"net_id": 7}, "token": token}, request_id="r3")
        r3b = call(bus.c, "lab/rf0", "set/duty", {"values": {"fraction_ppm": 20000}, "token": token},
                   request_id="r4")
        p3 = r3.doc.get("pending") if r3.kind == "value" else {}
        a = _applied_doc(r3b)
        report.check(sec, "step 3: a pending reply with the same token and deadline, its own apply_at; then "
                          "{applied: <view>} with pending.groups [radio, link, duty]",
                     p3.get("token") == token and p3.get("deadline") == r1.doc["pending"]["deadline"]
                     and p3.get("apply_at") != r1.doc["pending"]["apply_at"]
                     and a is not None and a.get("pending", {}).get("groups") == ["radio", "link", "duty"],
                     f"{r3.short()}; {r3b.short()}")
        _wait(lambda: (bus.get("status") or {}).get("pending", {}).get("applied") is True, 5.0)
        t4 = time.time_ns()
        r4 = call(bus.c, "lab/rf0", "confirm", {"token": token}, request_id="r5")
        time.sleep(0.5)
        evs = bus.events(since_utc=t0u)
        edits = evs[0][1]["edits"] if evs else []
        report.check(sec, "step 4: one event, confirmed, with the three groups' edits in that order; no event at "
                          "either join (§2.17)",
                     r4.kind == "value" and len(evs) == 1 and evs[0][0] >= t4 and evs[0][1]["outcome"] == "confirmed"
                     and [e["group"] for e in edits] == ["radio", "link", "duty"],
                     f"{r4.short()}; events {[e for _, e in evs]}")
    finally:
        bus.close()


# -- §6 ------------------------------------------------------------------------------

def section6(report: Report) -> None:
    sec = "§6 an extend"
    bus = Bus()
    try:
        bus.owner()
        t0 = time.monotonic()
        t0u = time.time_ns()
        r1 = call(bus.c, "lab/rf0", "set/duty", {"values": {"fraction_ppm": 20000}, "confirm_s": 30})
        a = _applied_doc(r1)
        dl1 = cf.parse_time(a["pending"]["deadline"]) if a and "pending" in a else 0
        report.check(sec, "step 1: {applied: <view>} with the change pending, its deadline 30 s after the call",
                     a is not None and abs(dl1 - 30 * NS - t0u) < NS, r1.short())
        time.sleep(max(0.0, t0 + 20 - time.monotonic()))
        t2u = time.time_ns()
        r2 = call(bus.c, "lab/rf0", "extend", {"token": a["pending"]["token"], "confirm_s": 60, "answer": "status"})
        st = r2.doc.get("status") if r2.kind == "value" else None
        dl2 = cf.parse_time(st["pending"]["deadline"]) if st and "pending" in st else 0
        time.sleep(0.3)
        report.check(sec, "step 2: {status: <status>} with the deadline 60 s after the extend; no event",
                     st is not None and "groups" not in st and abs(dl2 - 60 * NS - t2u) < NS
                     and not bus.events(since_utc=t0u), r2.short())
        _wait(lambda: bool(bus.events(since_utc=t0u)), 70.0, 0.1)
        evs = bus.events(since_utc=t0u)
        at = (evs[0][0] - t0u) / NS if evs else None
        report.check(sec, "step 3: the undo runs at the new deadline, not the first: S receives the rolled-back "
                          "event 80 s after step 1, ± 0.5 s",
                     len(evs) == 1 and evs[0][1]["outcome"] == "rolled-back" and at is not None and abs(at - 80) <= 0.5,
                     f"event at {at and round(at, 3)} s: {[e['outcome'] for _, e in evs]}")
    finally:
        bus.close()


# -- §7 ------------------------------------------------------------------------------

def _restart(bus: Bus, o: cf.ConfigOwner, service: str = "rf0", **kw: Any) -> cf.ConfigOwner:
    """The owner's process stops and starts again: a new instance with the
    same persisted layer, device and published events (§2.12)."""
    overlay, device, events_at = o.core.overlay, o.device, o.core.events_at
    bus.drop(o)
    return bus.owner(service, overlay=overlay, device=device, events_at=events_at, **kw)


def section7(report: Report) -> None:
    from . import live

    sec = "§7 a restart during a pending change"
    bus = Bus()
    try:
        seen: list[tuple[str, dict | None, dict | None]] = []
        work: queue.Queue = queue.Queue()

        def worker() -> None:
            while True:
                key = work.get()
                if key is None:
                    return
                v = live.get_state(bus.t, "zk2/lab/rf0/config.v1/state/view")
                s = live.get_state(bus.t, "zk2/lab/rf0/config.v1/state/status")
                seen.append((key, json.loads(v.replies[0].payload) if v.replies else None,
                             json.loads(s.replies[0].payload) if s.replies else None))

        threading.Thread(target=worker, daemon=True).start()
        bus.extra.append(Tokens(bus.t, "zk2/lab/*/@zk/**",
                                on_put=lambda k: work.put(k) if "/@zk/instance/" in k else None))
        o = bus.owner()
        r1 = call(bus.c, "lab/rf0", "set/radio", {"values": {"frequency_hz": 869525000}, "confirm_s": 120})
        token = r1.doc["pending"]["token"] if r1.kind == "value" else None
        _wait(lambda: (bus.get("status") or {}).get("pending", {}).get("applied") is True, 5.0)
        report.check(sec, "step 1: a pending reply; the device holds 869525000",
                     token is not None and o.device.values["radio"]["frequency_hz"] == 869525000, r1.short())
        old = o.instance
        t2 = time.time_ns()
        o2 = _restart(bus, o)
        _wait(lambda: any(o2.instance in k for k, _, _ in seen), 3.0)
        time.sleep(0.5)
        withdrawn = [r for r in bus.tokens.of(f"/@zk/instance/{old}") if r[1] == "delete"]
        declared = [r for r in bus.tokens.of(f"/@zk/instance/{o2.instance}") if r[1] == "put"]
        at_token = next(((v, s) for k, v, s in seen if o2.instance in k), (None, None))
        v, s = at_token
        p = _param(v, "radio", "frequency_hz") or {}
        report.check(sec, "step 2: S sees the old instance token withdrawn and a new one declared; the new "
                          "instance's V and St, put before its tokens, show revision 0, nothing pending, no last "
                          "change, frequency_hz 868100000 at source file; the device holds the startup frequency; "
                          "St's instance is the new one; no event for the old change (§2.12)",
                     bool(withdrawn) and bool(declared) and v is not None and s is not None and v["revision"] == 0
                     and "pending" not in v and "last_change" not in v and p.get("value") == 868100000
                     and p.get("source") == "file" and o2.device.values["radio"]["frequency_hz"] == 868100000
                     and s["instance"] == o2.instance != old and not bus.events(since_utc=t2),
                     f"withdrawn {len(withdrawn)}, declared {len(declared)}, St {s}, events {bus.events(since_utc=t2)}")
        r3 = call(bus.c, "lab/rf0", "confirm", {"token": token})
        report.check(sec, "step 3: the late confirm is not_found: the token is not one the new instance minted "
                          "(§2.6)", r3.code == "not_found", r3.short())
        work.put(None)
    finally:
        bus.close()


# -- §8 ------------------------------------------------------------------------------

def section8(report: Report) -> None:
    import zenoh

    from . import acl, live
    from .contract import load_contract
    from .owner import free_loopback_port

    sec = "§8 persist, granted and not"
    workdir = tempfile.mkdtemp(prefix="zk2py-config-")
    contract = load_contract(cf.CONTRACT)
    both = ["lab/rf0", "lab/rf2"]
    reads = acl.Use(cf.IFACE, both, ["state/view", "state/status"])
    acts = ["@op/set/{group}", "@op/confirm", "@op/cancel", "@op/extend"]
    principals = [
        acl.Principal("rf0", "rf0", service="lab/rf0", implements=[contract]),
        acl.Principal("rf2", "rf2", service="lab/rf2", implements=[contract]),
        acl.Principal("ops", "ops", consumes=[reads], calls=[acl.Use(cf.IFACE, both, acts)]),
        acl.Principal("admin", "admin", consumes=[reads], calls=[acl.Use(cf.IFACE, both, [*acts, "@op/persist"])]),
        acl.Principal("S", "s", consumes=[acl.Use(cf.IFACE, ["lab/rf0"], ["state/view", "state/status",
                                                                           "events/change"])]),
    ]
    block = acl.generate(principals, "deny").block
    users = os.path.join(workdir, "users.txt")
    with open(users, "w") as f:
        for p in principals:
            f.write(f"{p.user}:{p.user}-pw\n")
    port = free_loopback_port()
    conf = zenoh.Config()
    conf.insert_json5("mode", json.dumps("router"))
    conf.insert_json5("listen/endpoints", json.dumps([f"tcp/127.0.0.1:{port}"]))
    conf.insert_json5("scouting/multicast/enabled", "false")
    conf.insert_json5("timestamping/enabled", "true")
    conf.insert_json5("transport/auth/usrpwd", json.dumps({"dictionary_file": users}))
    conf.insert_json5("access_control", json.dumps(block))
    router = zenoh.open(conf)
    ep = f"tcp/127.0.0.1:{port}"
    admin = live.open_client(ep, ("admin", "admin-pw"))
    ops = live.open_client(ep, ("ops", "ops-pw"))
    s_sess = live.open_client(ep, ("s", "s-pw"))
    # S subscribes to what its grants name, under deny: a wider selector
    # is refused.
    base = f"zk2/lab/rf0/{cf.IFACE}"
    bus = Bus(router=(router, ep), sessions={"C": admin, "S": s_sess, "T": live.open_client(ep, ("ops", "ops-pw"))},
              selectors=[f"{base}/state/view", f"{base}/state/status", f"{base}/events/change/*"])
    bus.extra.append(ops)
    try:
        o = bus.owner(auth=("rf0", "rf0-pw"))
        o2 = bus.owner("rf2", persist=False, auth=("rf2", "rf2-pw"))
        t0 = time.time_ns()
        r1 = call(admin, "lab/rf0", "set/duty", {"values": {"fraction_ppm": 20000}}, actor="admin")
        tok = (_applied_doc(r1) or {}).get("last_change", {}).get("token")
        time.sleep(0.3)
        t1 = time.time_ns()
        r1p = call(admin, "lab/rf0", "persist", {"token": tok}, actor="admin")
        pv = r1p.doc.get("view") if r1p.kind == "value" else None
        time.sleep(0.5)
        report.check(sec, "step 1: as admin, the set and the persist are both answered; the persist's reply shows "
                          "fraction_ppm at source overlay; no event for the persist",
                     _applied_doc(r1) is not None and (_param(pv, "duty", "fraction_ppm") or {}).get("source")
                     == "overlay" and len(bus.events(since_utc=t0)) == 1 and not bus.events(since_utc=t1),
                     f"{r1.short()}; {r1p.short()}; events {[e['outcome'] for _, e in bus.events(since_utc=t0)]}")
        r2 = call(ops, "lab/rf0", "set/duty", {"values": {"fraction_ppm": 30000}})
        tok2 = (_applied_doc(r2) or {}).get("last_change", {}).get("token")
        r2p = call(ops, "lab/rf0", "persist", {"token": tok2})
        report.check(sec, "step 2: as ops, the set is answered; the persist draws nothing before its timeout: a "
                          "refusal by access control is silent (core O5), never not_found",
                     _applied_doc(r2) is not None and r2p.kind == "silent", f"{r2.short()}; persist {r2p.short()}")
        call(admin, "lab/rf2", "set/duty", {"values": {"fraction_ppm": 20000}}, actor="admin")
        tok3 = (bus.get("status", "lab/rf2", session=admin) or {}).get("last_change", {}).get("token", "x")
        r3 = call(admin, "lab/rf2", "persist", {"token": tok3}, actor="admin")
        d = live.get_descriptor(admin, o2.owner.instance_key)
        doc = json.loads(d[0].payload) if d and d[0].ok else {}
        entry = next((e for e in doc.get("interfaces") or [] if e.get("iface") == cf.IFACE), {})
        report.check(sec, "step 3: as admin on lab/rf2, persist is unavailable, cause config; its descriptor lists "
                          "persist unavailable with cause config (core §3.3)",
                     r3.code == "unavailable" and (r3.envelope or {}).get("cause") == "config"
                     and {"resource": "@op/persist", "cause": "config"} in entry.get("unavailable", []),
                     f"{r3.short()}; descriptor unavailable {entry.get('unavailable')}")
        o3 = _restart(bus, o, auth=("rf0", "rf0-pw"))
        v = bus.get("view", session=admin)
        p = _param(v, "duty", "fraction_ppm") or {}
        report.check(sec, "step 4: after the restart, fraction_ppm is 20000 at source overlay: step 1's change "
                          "survived, step 2's did not (§2.12)",
                     p.get("value") == 20000 and p.get("source") == "overlay" and v is not None
                     and v["instance"] == o3.instance, f"fraction_ppm {p}")
    finally:
        bus.close()


# -- §9 ------------------------------------------------------------------------------

def section9(report: Report) -> None:
    sec = "§9 a group outside the allowlist"
    bus = Bus()
    try:
        bus.owner()
        v = bus.get("view")
        report.check(sec, "step 1: every group but queue has writable true",
                     v is not None and [g["name"] for g in v["groups"] if g["writable"]]
                     == ["duty", "radio", "link", "sim"] and not cf.find_group(v["groups"], "queue")["writable"],
                     f"{[(g['name'], g['writable']) for g in (v or {}).get('groups', [])]}")
        t0 = time.time_ns()
        r2 = call(bus.c, "lab/rf0", "set/queue", {"values": {"tx_queue_len": 10}})
        r3 = call(bus.c, "lab/rf0", "set/queue", {"values": {"nope": 1}})
        time.sleep(0.3)
        v2 = bus.get("view")
        report.check(sec, "step 2: unavailable, cause config; nothing changed; no event",
                     r2.code == "unavailable" and (r2.envelope or {}).get("cause") == "config"
                     and v2 is not None and v2["revision"] == 0 and not bus.events(since_utc=t0), r2.short())
        report.check(sec, "step 3: the same: the allowlist comes before the values (§2.5)",
                     r3.code == "unavailable" and (r3.envelope or {}).get("cause") == "config", r3.short())
    finally:
        bus.close()


# -- §10 -----------------------------------------------------------------------------

def section10(report: Report) -> None:
    sec = "§10 an idempotent retry"
    bus = Bus()
    try:
        o = bus.owner()
        req = {"values": {"frequency_hz": 869525000}, "confirm_s": 60, "idempotency_key": "k1"}
        t0 = time.monotonic()
        t0u = time.time_ns()
        o.drop_replies = 1
        r1 = call(bus.c, "lab/rf0", "set/radio", req)
        armed = o.core.pending is not None
        report.check(sec, "step 1: the owner arms the change; C receives nothing (the reply dropped at the owner)",
                     r1.kind == "silent" and armed, f"{r1.short()}, pending {armed}")
        time.sleep(max(0.0, t0 + 5 - time.monotonic()))
        r2 = call(bus.c, "lab/rf0", "set/radio", req)
        first = json.loads(o.dropped[0]) if o.dropped else None
        time.sleep(0.3)
        report.check(sec, "step 2: the reply is step 1's, token and instants unchanged; the device saw one apply, and "
                          "the owner published nothing new (§2.14)",
                     r2.kind == "value" and r2.doc == first and len(o.device.acts("apply")) == 1
                     and not bus.events(since_utc=t0u),
                     f"{r2.short()} vs dropped {first}; applies {len(o.device.acts('apply'))}")
        r3 = call(bus.c, "lab/rf0", "set/radio", {**req, "values": {"frequency_hz": 869000000}})
        report.check(sec, "step 3: the same key with another frequency_hz is invalid_request",
                     r3.code == "invalid_request", r3.short())
        token = (first or {}).get("pending", {}).get("token")
        call(bus.c, "lab/rf0", "cancel", {"token": token})
        time.sleep(max(0.0, t0 + 300.2 - time.monotonic()))
        r4 = call(bus.c, "lab/rf0", "set/radio", req)
        p4 = r4.doc.get("pending") if r4.kind == "value" else {}
        report.check(sec, "step 4: after a cancel, 300 s after step 1's call, step 1's request draws a new pending "
                          "reply with a new token: the key was forgotten at 300 s",
                     bool(p4) and p4.get("token") not in (None, token), r4.short())
    finally:
        bus.close()


# -- §11 -----------------------------------------------------------------------------

def section11(report: Report) -> None:
    sec = "§11 sensitive values"
    bus = Bus()
    try:
        o = bus.owner()
        t0 = time.time_ns()
        r1 = call(bus.c, "lab/rf0", "set/sim", {"values": {"pin": "1234"}})
        tok = (_applied_doc(r1) or {}).get("last_change", {}).get("token")
        r1b = call(bus.c, "lab/rf0", "set/sim", {"values": {"pin": 1234}})
        time.sleep(0.4)
        evs = bus.events(since_utc=t0)
        msg = (r1b.envelope or {}).get("message", "")
        report.check(sec, "step 1: the first is applied, its event's edit {group sim, parameter pin, redacted true}; "
                          "the second is invalid_request, its message naming pin and holding no 1234",
                     _applied_doc(r1) is not None and len(evs) == 1
                     and evs[0][1]["edits"] == [{"group": "sim", "parameter": "pin", "redacted": True}]
                     and r1b.code == "invalid_request" and "pin" in msg and "1234" not in msg,
                     f"{r1.short()}; {r1b.short()}; edits {[e['edits'] for _, e in evs]}")
        o.device.next_apply.append({"refused": "PIN 1234 rejected by the SIM"})
        r2 = call(bus.c, "lab/rf0", "set/sim", {"values": {"pin": "5678"}})
        report.check(sec, "step 2: the device refusing a set of pin with words holding the PIN: app, "
                          "ConfigError{code: device-refused}, with no detail",
                     r2.code == "app" and (r2.envelope or {}).get("detail") == {"code": "device-refused"},
                     r2.short())
        v, st = bus.get("view"), bus.get("status")
        p = _param(v, "sim", "pin") or {}
        r3 = call(bus.c, "lab/rf0", "persist", {"token": tok})
        report.check(sec, "step 3: V shows pin with source runtime and no value or startup; St has no sim; the "
                          "persist of the first change is forbidden",
                     p.get("source") == "runtime" and "value" not in p and "startup" not in p
                     and st is not None and "sim" not in st["values"] and r3.code == "forbidden",
                     f"pin {p}; St groups {sorted((st or {}).get('values', {}))}; persist {r3.short()}")
        logged = [json.loads(line)["request"] for line in o.core.log if json.loads(line)["group"] == "sim"]
        heard = [x.payload for k in list(bus.s.deliveries) for x in bus.s.of(k)]
        replies = [r.raw for r in (r1, r1b, r2, r3)]
        report.check(sec, "step 4: no line of the owner's log holds 1234, a logged request writing \"<redacted>\"; "
                          "no delivery S recorded, and no reply C received, holds 1234",
                     not any("1234" in line for line in o.core.log)
                     and all(x["values"].get("pin") == cf.REDACTED for x in logged)
                     and not any(b"1234" in x for x in heard + replies),
                     f"{len(o.core.log)} log lines, sim requests logged {logged}; {len(heard)} deliveries")
    finally:
        bus.close()


# -- §12 -----------------------------------------------------------------------------

def section12(report: Report) -> None:
    from .contract import load_contract
    from .owner import Owner

    sec = "§12 desired state refused"
    bus = Bus()
    author = None
    try:
        contract = load_contract(AUTHOR)
        author = Owner("lab", "author", [contract], connect=bus.endpoint)
        author.start()
        key = f"zk2/lab/author/{contract.interface}/state/desired"
        o = bus.owner(desired_key=key)
        t0 = time.time_ns()
        author.set_state(key, cf.encode_json({"revision": 1, "values": {"duty": {"fraction_ppm": 20000}}}))
        _wait(lambda: (bus.get("status") or {}).get("desired", {}).get("observed_revision") == 1, 3.0)
        time.sleep(0.3)
        st = bus.get("status")
        evs = bus.events(since_utc=t0)
        report.check(sec, "step 1: S receives one event, applied, and St with fraction_ppm 20000 and desired "
                          "{observed_revision 1, converged}",
                     len(evs) == 1 and evs[0][1]["outcome"] == "applied" and _val(st, "duty", "fraction_ppm") == 20000
                     and (st or {}).get("desired") == {"observed_revision": 1, "outcome": "converged"},
                     f"events {[e['outcome'] for _, e in evs]}; St desired {(st or {}).get('desired')}")
        acts_before = len(o.device.log)
        t2 = time.time_ns()
        author.set_state(key, cf.encode_json({"revision": 2, "values": {"duty": {"fraction_ppm": 30000},
                                                                         "radio": {"frequency_hz": 869525000}}}))
        _wait(lambda: (bus.get("status") or {}).get("desired", {}).get("observed_revision") == 2, 3.0)
        st = bus.get("status")
        report.check(sec, "step 2: no act and no event; St's desired is {2, refused, {reach, radio}}, and "
                          "fraction_ppm is still 20000: a document is refused whole",
                     (st or {}).get("desired") == {"observed_revision": 2, "outcome": "refused",
                                                   "refusal": {"code": "reach", "group": "radio"}}
                     and _val(st, "duty", "fraction_ppm") == 20000 and len(o.device.log) == acts_before
                     and not bus.events(since_utc=t2), f"St desired {(st or {}).get('desired')}")
        author.set_state(key, cf.encode_json({"revision": 3, "values": {"sim": {"pin": "1234"}}}))
        _wait(lambda: (bus.get("status") or {}).get("desired", {}).get("observed_revision") == 3, 3.0)
        st = bus.get("status")
        report.check(sec, "step 3: refusal {sensitive, sim, pin}, and no act",
                     (st or {}).get("desired", {}).get("refusal") == {"code": "sensitive", "group": "sim",
                                                                       "parameter": "pin"}
                     and len(o.device.log) == acts_before, f"St desired {(st or {}).get('desired')}")
    finally:
        if author is not None:
            author.close()
        bus.close()


# -- §13 -----------------------------------------------------------------------------

def section13(report: Report) -> None:
    import zenoh

    from . import live
    from .owner import free_loopback_port

    sec = "§13 the compact status across a constrained face"
    workdir = tempfile.mkdtemp(prefix="zk2py-config-face-")
    users = os.path.join(workdir, "users.txt")
    with open(users, "w") as f:
        f.write("far:far-pw\nowner:owner-pw\n")
    port = free_loopback_port()
    conf = zenoh.Config()
    conf.insert_json5("mode", json.dumps("router"))
    conf.insert_json5("listen/endpoints", json.dumps([f"tcp/127.0.0.1:{port}"]))
    conf.insert_json5("scouting/multicast/enabled", "false")
    conf.insert_json5("timestamping/enabled", "true")
    conf.insert_json5("transport/auth/usrpwd", json.dumps({"dictionary_file": users}))
    every = ["put", "delete", "declare_subscriber", "query", "declare_queryable", "reply", "liveliness_token",
             "declare_liveliness_subscriber", "liveliness_query"]
    conf.insert_json5("access_control", json.dumps({
        "enabled": True, "default_permission": "allow",
        "rules": [{"id": "zk", "messages": every, "flows": ["ingress", "egress"], "permission": "deny",
                   "key_exprs": ["zk2/**/@zk/**"]},
                  {"id": "sub", "messages": ["declare_subscriber"], "flows": ["ingress", "egress"],
                   "permission": "deny", "key_exprs": ["zk2/lab/rf0/*/state/**"]}],
        "subjects": [{"id": "far", "usernames": ["far"]}],
        "policies": [{"id": "the-face", "rules": ["zk", "sub"], "subjects": ["far"]}],
    }))
    router = zenoh.open(conf)
    ep = f"tcp/127.0.0.1:{port}"
    cconf = zenoh.Config()
    cconf.insert_json5("mode", json.dumps("client"))
    cconf.insert_json5("connect/endpoints", json.dumps([ep]))
    cconf.insert_json5("scouting/multicast/enabled", "false")
    cconf.insert_json5("transport/auth/usrpwd", json.dumps({"user": "far", "password": "far-pw"}))
    cconf.insert_json5("transport/link/tx/batch_size", "1024")
    far = zenoh.open(cconf)
    near = live.open_client(ep, ("owner", "owner-pw"))
    bus = Bus(router=(router, ep), sessions={"C": far, "S": near, "T": live.open_client(ep, ("owner", "owner-pw"))})
    try:
        crossed = Tokens(far, "zk2/lab/rf0/@zk/**")
        bus.extra.append(crossed)
        o = bus.owner(auth=("owner", "owner-pw"))
        st1 = bus.get("status", session=far)
        time.sleep(0.5)
        report.check(sec, "step 1: St answers across the face, with confirm_floor_s; no @zk delivery crosses",
                     st1 is not None and st1.get("confirm_floor_s") == 10 and not crossed.rows,
                     f"St {st1}; {len(crossed.rows)} @zk deliveries")
        r2 = call(far, "lab/rf0", "set/radio", {"values": {"frequency_hz": 869525000}, "confirm_s": 120})
        report.check(sec, "step 2: the pending reply, at most 160 bytes of payload",
                     r2.kind == "value" and "pending" in r2.doc and len(r2.raw) <= 160,
                     f"{len(r2.raw)} bytes: {r2.short()}")
        _wait(lambda: (bus.get("status", session=far) or {}).get("pending", {}).get("applied") is True, 5.0)
        r3 = call(far, "lab/rf0", "confirm", {"token": r2.doc["pending"]["token"], "answer": "status"})
        report.check(sec, "step 3: {status: <status>}, at most 400 bytes for this schema: the status, not the view",
                     r3.kind == "value" and "status" in r3.doc and len(r3.raw) <= 400,
                     f"{len(r3.raw)} bytes (the view would be {len(cf.encode_json({'view': o.core.view()}))})")
        o2 = _restart(bus, o, auth=("owner", "owner-pw"))
        st4 = bus.get("status", session=far)
        report.check(sec, "step 4: St's instance differs from step 1's: C learns of the restart without presence "
                          "(§2.12)",
                     st4 is not None and st1 is not None and st4["instance"] != st1["instance"] == o.instance
                     and st4["instance"] == o2.instance and not crossed.rows,
                     f"{st1 and st1['instance']} -> {st4 and st4['instance']}")
    finally:
        bus.close()


# -- §14 -----------------------------------------------------------------------------

def section14(report: Report) -> None:
    from .contract import load_contract
    from .owner import Owner

    sec = "§14 a tool's answers"
    bus = Bus()
    plain = None
    try:
        t_sub = fr.Subscriber(bus.t, "zk2/lab/rf0/config.v1/**")
        bus.extra.append(t_sub)
        o = bus.owner()
        plain = Owner("lab", "plain", [load_contract(NAV)], connect=bus.endpoint)
        plain.start()
        time.sleep(0.5)
        a_rf = cf.answer_pending(cf.read(bus.t, "lab/rf0", subscriber=t_sub))
        a_pl = cf.answer_pending(cf.read(bus.t, "lab/plain"))
        report.check(sec, "step 1: is a change pending? lab/rf0: no; lab/plain: not asked, its descriptor not "
                          "listing config.v1",
                     a_rf[0] == "no" and a_pl[0] == cf.NOT_ASKED, f"rf0 {a_rf}; plain {a_pl}")
        r = call(bus.c, "lab/rf0", "set/radio", {"values": {"frequency_hz": 869525000}, "confirm_s": 10})
        token = r.doc["pending"]["token"] if r.kind == "value" else None
        time.sleep(0.3)
        rd = cf.read(bus.t, "lab/rf0", subscriber=t_sub)
        a2, l2 = cf.answer_pending(rd), cf.answer_landed(rd, token, o.instance)
        report.check(sec, "step 2: yes, with the token and the deadline; \"did it land?\": not yet",
                     a2[0] == "yes" and a2[1]["token"] == token and a2[1]["deadline"] == r.doc["pending"]["deadline"]
                     and l2[0] == "not_yet", f"{a2}; {l2}")
        _wait(lambda: any(e[1]["token"] == token for e in bus.events()), 12.0)
        time.sleep(0.3)
        rd = cf.read(bus.t, "lab/rf0", subscriber=t_sub)
        l3, u3 = cf.answer_landed(rd, token, o.instance), cf.answer_undo_clean(rd, token)
        report.check(sec, "step 3: after the rollback at its deadline, \"did it land?\": no, rolled-back; \"was its "
                          "undo clean?\": yes", l3 == ("no", "rolled-back") and u3[0] == "yes", f"{l3}; {u3}")
        call(bus.c, "lab/rf0", "set/duty", {"values": {"fraction_ppm": 20000}})
        time.sleep(0.3)
        s4 = cf.answer_survives(cf.read(bus.t, "lab/rf0", subscriber=t_sub), "duty", "fraction_ppm")
        report.check(sec, "step 4: \"will fraction_ppm survive a restart?\": no, it runs a value with startup 10000",
                     s4 == ("no", {"startup": 10000}), str(s4))
        last_put = max(x.arrival_ns for x in t_sub.of(o.status_key))
        bus.drop(o)
        time.sleep(1.0)
        a5 = cf.answer_pending(cf.read(bus.t, "lab/rf0", subscriber=t_sub))
        within = (time.monotonic_ns() - last_put) / NS
        report.check(sec, "step 5a: within 60 s of the owner's stop, \"is a change pending?\": not asked, the owner "
                          "absent (presence's word)", a5[0] == cf.NOT_ASKED and within < 60,
                     f"{a5} at {within:.1f} s after the last put")
        while (time.monotonic_ns() - last_put) / NS < 70:
            time.sleep(0.5)
        a6 = cf.answer_pending(cf.read(bus.t, "lab/rf0", subscriber=t_sub, presence=False, lists=True))
        report.check(sec, "step 5b: 70 s after the owner's last put, a reader that does not read presence: "
                          "unobservable, the status stale; never no (§5, core O5)", a6[0] == cf.UNOBSERVABLE, str(a6))
    finally:
        if plain is not None:
            plain.close()
        bus.close()


SECTIONS = {"1": section1, "2": section2, "3": section3, "4": section4, "5": section5, "6": section6,
            "7": section7, "8": section8, "9": section9, "10": section10, "11": section11, "12": section12,
            "13": section13, "14": section14}
LONG = ("1", "6", "10", "14")


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="python -m zk2py.config_scenarios", description=__doc__.split("\n")[0])
    ap.add_argument("--only", action="append", choices=sorted(SECTIONS, key=int), help="run only these sections")
    args = ap.parse_args(argv)
    report = Report()
    chosen = [n for n in SECTIONS if not args.only or n in args.only]

    def run(name: str) -> None:
        try:
            SECTIONS[name](report)
        except Exception as e:  # noqa: BLE001 - a section that cannot run is a failure
            import traceback

            traceback.print_exc()
            report.check(f"§{name}", "the section ran", False, f"{type(e).__name__}: {e}")

    threads = [threading.Thread(target=run, args=(n,)) for n in chosen if n in LONG]
    for t in threads:
        t.start()
    for n in chosen:
        if n not in LONG:
            run(n)
    for t in threads:
        t.join()
    passed = sum(1 for r in report.rows if r[2])
    failed = len(report.rows) - passed
    print(f"config scenarios: {passed} passed, {failed} failed")
    for sec, name, ok, detail in report.rows:
        if not ok:
            print(f"FAIL [{sec}] {name}: {detail}")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())

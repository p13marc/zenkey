"""The ``health.v1`` scenarios (``spec/profiles/health/scenarios.md``, text
0.2) on
in-process zenoh-python routers: ``python -m zk2py.health_scenarios
[--only 1 …]``.

- **The bus** is the text's: R1, a router with timestamping on and without
  ``drop_future_timestamp``; owners, the subscriber S, the GET reader G and
  the tool T are its clients, each with a session of its own. Timeouts are
  1 s.
- **The contract** is the standard one, ``spec/profiles/health/
  health.v1.toml``, with protobuf payloads, served by
  :class:`zk2py.health.HealthOwner`.
- **S** subscribes to the owner's ``health.v1/**`` before it starts. **G**
  GETs ``health.v1/state/**`` and measures its clock from S's deliveries
  (freshness.v1 §2.6, ground 2). "Judges" is :func:`zk2py.health.judge`
  over :func:`zk2py.health.read_near`.
- **The horizon** is 60 s, so §2, §4 and §8 wait it out: about 70 s, 100 s
  and 200 s (§8's two runs side by side). §4's [moved clock] tier waits
  seconds.
- **A clock ahead** (§4) is the owner's ``clock``, offset 2 s, in both of
  0.2's tiers. ``--only 4r`` (round 98, opt-in) runs §4's steps 1 to 3
  against the owner example, a black box, S zk2py's. Without root (``--only 4``), on a session whose HLC runs: the
  fault's stamp is not checked. [moved clock] (``--only 4m``): an owner
  client whose session has no HLC (``hlc=False``), stamping its puts and its
  faults from the offset clock, so they reach R1 dated ahead, and step 4
  under ``drop_future_timestamp``.
- **The tool T** of §5 to §7 reads by presence and GET alone, and takes the
  deployment's word for its clock (freshness.v1 §2.6, ground 1; 0.2).
- **§6's archive** is a stand-in that records the owner's
  ``health.v1/state/**`` and answers its archive form with core §4.4's
  attachment. It is not ``archive.v1``, which no text defines.
- **§8's face** is a router V with usrpwd and access control: the ground
  session G is a client of V, with ``zk2/**/@zk/**`` denied to it (run A),
  and ``zk2/vehicle-01/*/*/state/**`` too (run B).

§1 to §5 are the runtime's, §6 to §8 a tool's (zk2py's reader).

Exit 0 when every check passes, 1 when any fails.
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
from pathlib import Path
from typing import Any

from . import heartbeat, zk2c
from . import freshness as fr
from . import health as h

REPO = Path(__file__).resolve().parents[3]
NAV = REPO / "examples" / "zk2" / "walkthrough" / "nav.v2.toml"
JITTER_S = 0.2


class Report:
    def __init__(self) -> None:
        self.rows: list[tuple[str, str, bool, str]] = []
        self.lock = threading.Lock()

    def check(self, section: str, name: str, ok: bool, detail: str = "") -> bool:
        with self.lock:
            self.rows.append((section, name, ok, detail))
            print(f"{'PASS' if ok else 'FAIL'} [{section}] {name}" + (f": {detail}" if detail else ""), flush=True)
        return ok


def _zid(text: Any) -> int | None:
    from .live import _zid_value

    return _zid_value(text)


class Offset:
    """An owner's clock offset, which a scenario can set right (§4)."""

    def __init__(self, ns: int = 0):
        self.ns = ns

    def clock(self, owner: Any):
        import zenoh

        def now():
            ts = owner.session.new_timestamp()
            n = ts.get_time_as_ntp64().as_nanos() + self.ns
            return zenoh.Timestamp(zenoh.NTP64(n // fr.NS, n % fr.NS), ts.get_id())

        return now

    def ns_now(self) -> int:
        return time.time_ns() + self.ns


class Bus:
    """R1, and S, G and T as client sessions of it."""

    def __init__(self) -> None:
        from . import live
        from .live_interop import _r1

        self.router, self.endpoint, self.r1_zid = _r1()
        self.s_session = live.open_client(self.endpoint)
        self.g_session = live.open_client(self.endpoint)
        self.t_session = live.open_client(self.endpoint)
        self.trust = fr.ClockTrust()
        self.subs: list[fr.Subscriber] = []
        self.owners: list[Any] = []
        self.extra: list[Any] = []

    def subscriber(self, address: str) -> fr.Subscriber:
        from .contract import load_contract

        s = fr.Subscriber(self.s_session, f"zk2/{address}/health.v1/**", self.trust)
        # 0.8 (#978 F4): S hears the heartbeat key covering health.v1 too,
        # and reads each heartbeat as freshness.v2 §2.5 says.
        hc = load_contract(h.CONTRACT)
        s.follow(address, hc.canonical, hc.fingerprint)
        self.subs.append(s)
        time.sleep(0.2)
        return s

    def owner(self, address: str, start: bool = True, offset: Offset | None = None, **kw: Any) -> h.HealthOwner:
        system, service = address.split("/")
        if offset is not None:
            kw["clock_ns"] = offset.ns_now
        o = h.HealthOwner(system, service, connect=self.endpoint, **kw)
        if offset is not None:
            o.owner.clock = offset.clock(o.owner)
        if start:
            o.start()
        self.owners.append(o)
        return o

    def read(self, address: str, s: fr.Subscriber | None = None, get: bool = True,
             trust: fr.ClockTrust | None = None):
        """S's reading (``s``) or G's. 0.8: "G and T take the deployment's
        word, freshness.v1 §2.6's ground 1": a status put carries no Zenoh
        timestamp of the owner's, so G can no longer measure it from S's
        deliveries."""
        return h.read_near(self.t_session if s is None else self.g_session, address, subscriber=s,
                           trust=fr.ClockTrust(word=True) if trust is None else trust, get=get)

    def t_read(self, address: str):
        """T in §5 to §7 (0.2): presence and GET, on the deployment's word
        for its clock (freshness.v1 §2.6, ground 1). Every session here runs
        on one host, whose one clock keeps the word."""
        return h.read_near(self.t_session, address, trust=fr.ClockTrust(word=True))

    def close(self) -> None:
        for s in self.subs:
            s.close()
        for o in self.owners:
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

        # live.close: each session with the watch session zk2py opened for it
        for x in (self.s_session, self.g_session, self.t_session, self.router):
            live.close(x)


def _puts(recs: list[fr.Received]) -> list[fr.Received]:
    """What the owner put, a heartbeat's confirmation set aside (0.8)."""
    return [r for r in recs if not r.confirmation]


def _beats(s: fr.Subscriber, address: str, since: int) -> list[tuple[int, dict[str, Any]]]:
    """The heartbeats covering health.v1 S read since ``since`` (0.8)."""
    with s.lock:
        return [(t, a) for t, a in s.beats.get((address, h.IFACE), []) if t >= since]


def _vouches(a: dict[str, Any], key: str) -> bool:
    sec = ((a.get("read") or {}).get("sections") or {}).get("state/status")
    return isinstance(sec, dict) and key in sec.get("confirmed", [])


def _status(payload: bytes) -> dict[str, Any] | None:
    return h.decode_status(payload)


def _since(recs: list[fr.Received], t_ns: int) -> list[fr.Received]:
    return [r for r in recs if r.arrival_ns >= t_ns]


def _sleep_until(mono_ns: int) -> None:
    d = (mono_ns - time.monotonic_ns()) / fr.NS
    if d > 0:
        time.sleep(d)


def _answer(a: h.Answer) -> str:
    return f"{a.verdict}/{a.reason}" + (f" at {a.level}" if a.level else "")


# -- §1 ------------------------------------------------------------------------------

def section1(report: Report) -> None:
    from . import live

    sec = "§1 bring-up"
    bus = Bus()
    try:
        s = bus.subscriber("lab/svc")
        seen: queue.Queue = queue.Queue()
        # Observer §3.7 (0.37): on T's watch session, never beside T's reads.
        lv, _ = live.watch_or_beside(bus.t_session, "zk2/lab/*/@zk/**", h_callback(seen), history=True)
        bus.extra.append(lv)
        answers: dict[str, list] = {}

        def worker() -> None:
            while True:
                item = seen.get()
                if item is None:
                    return
                kind, key = item
                if "PUT" not in kind:
                    continue
                parts = key.split("/")
                address = "/".join(parts[1:3])
                if parts[4] == "instance":
                    st = live.get_state(bus.t_session, f"zk2/{address}/health.v1/state/**")
                    d = live.get_descriptor(bus.t_session, key)
                    answers.setdefault(f"{address} instance", []).append((st, d))
                elif parts[4] == "alive" and parts[5] == "health.v1":
                    st = live.get_state(bus.t_session, f"zk2/{address}/health.v1/state/**")
                    answers.setdefault(f"{address} alive", []).append((st, None))

        th = threading.Thread(target=worker, daemon=True)
        th.start()
        o = bus.owner("lab/svc", level=h.UNSPECIFIED, reason="starting")
        zid = _zid(str(o.owner.session.zid()))
        first_since = o.since_ns
        time.sleep(1.0)

        def status_of(st) -> list[tuple[dict | None, int | None]]:
            return [(_status(x.payload), _zid(x.stamp_id)) for x in st.replies if x.key.endswith("/state/status")]

        # core 0.100 (round 105): status, a freshness.v2 state, is answered by
        # snapshot, its entry with no Zenoh timestamp; "stamped by the owner's
        # session" is read by its in-band run, the one the owner's descriptor
        # states (F-503: the scenario reads a stamp's session)
        run = o.owner.run_hex()
        got = [[(_status(x.payload), x.inband.run_hex() if x.inband is not None else None, x.stamp_id)
                for x in st.replies if x.key.endswith("/state/status")]
               for k in ("lab/svc instance", "lab/svc alive") for st, _ in answers.get(k, [])]
        report.check(sec, "step 1: both of T's GETs, at the instance token and at the alive/health.v1 token, "
                          "answer the status UNSPECIFIED \"starting\", a snapshot's entry under the owner's run "
                          "(its in-band stamp; no Zenoh timestamp, F-503): put before the tokens (§2.3)",
                     len(got) == 2 and all(len(x) == 1 and x[0][0] is not None and x[0][0]["level"] == h.UNSPECIFIED
                                           and x[0][0]["reason"] == "starting" and x[0][1] == run
                                           and x[0][2] is None for x in got),
                     f"{got}; run {run}")
        # Step 2.
        bus.owner("lab/quiet", level=h.OK, reason="serving", tokenless={"health.v1", "freshness.v2"})
        time.sleep(1.0)
        pres = live.list_presence(bus.t_session, "zk2/lab/quiet/@zk/**")
        qa = answers.get("lab/quiet instance", [])
        doc = json.loads(qa[0][1][0].payload) if qa and qa[0][1] and qa[0][1][0].ok else {}
        entry = next((e for e in doc.get("interfaces") or [] if e.get("iface") == "health.v1"), {})
        st_q = status_of(qa[0][0]) if qa else []
        report.check(sec, "step 2: lab/quiet holds an instance token and no alive/health.v1 or alive/freshness.v2 "
                          "token, its descriptor lists health.v1 with \"token\": false, and T's GET at the instance "
                          "token answers its status OK",
                     len(pres.instances) == 1 and not [a for a in pres.alive if a["iface"] in ("health.v1",
                                                                                            "freshness.v2")]
                     and entry.get("token") is False and len(st_q) == 1 and st_q[0][0]["level"] == h.OK,
                     f"{len(pres.instances)} instance, alive {[a['iface'] for a in pres.alive]}, entry {entry}, "
                     f"status {st_q}")
        # Step 3.
        t3 = time.monotonic_ns()
        o.set_status(h.OK, "serving")
        time.sleep(0.3)
        st = live.get_state(bus.g_session, "zk2/lab/svc/health.v1/state/status")
        g = [_status(x.payload) for x in st.replies]
        report.check(sec, "step 3: G's reply is OK, \"serving\", with since_ns later than the first status's",
                     len(g) == 1 and g[0]["level"] == h.OK and g[0]["reason"] == "serving"
                     and g[0]["since_ns"] > first_since, f"{g}, first since {first_since}")
        # Step 4 (0.8): "The owner puts nothing for 65 s, and S judges."
        mark = time.monotonic_ns()
        time.sleep(65.0)
        sk = "zk2/lab/svc/health.v1/state/status"
        after = _puts(_since(s.of(sk), mark))
        beats = _beats(s, "lab/svc", mark)
        arrivals = [mark] + [t for t, _ in beats]
        gaps = [(b - a) / fr.NS for a, b in zip(arrivals, arrivals[1:])]
        tag = heartbeat.layout_of(o.contract.canonical, o.contract.fingerprint).tag
        r4, _ = bus.read("lab/svc", s, get=False)
        a4 = h.judge(r4)
        report.check(sec, "step 4: in 65 s S receives no put of the status (freshness.v2 §2.4), a heartbeat at least "
                          "every 30 s vouching for it, under health.v1 1.1's tag, and judges healthy, ok, past the "
                          "horizon",
                     not after and bool(beats) and max(gaps) <= 30 + JITTER_S
                     and all(_vouches(a, sk) for _, a in beats) and o.contract.fingerprint.startswith("sha256:60a43745")
                     and a4.expect() == {"verdict": "healthy", "reason": "ok", "level": "ok"},
                     f"{len(after)} puts, {len(beats)} heartbeats (tag {tag}), gaps {[round(x, 2) for x in gaps]} s, "
                     f"{_answer(a4)}")
        # Step 5.
        o.close()
        bus.owners.remove(o)
        time.sleep(3.0)
        deletes = [r for r in s.of(sk) if r.kind == "delete"]
        report.check(sec, "step 5: S receives no delete of the status, before or after the close (§2.3)",
                     not deletes, f"{len(deletes)} deletes of {len(s.of(sk))} deliveries")
        seen.put(None)
    finally:
        bus.close()


def h_callback(q: queue.Queue):
    import zenoh

    return zenoh.handlers.Callback(lambda smp: q.put((str(smp.kind), str(smp.key_expr))))


# -- §2 ------------------------------------------------------------------------------

def section2(report: Report) -> None:
    sec = "§2 a stale status"
    bus = Bus()
    try:
        s = bus.subscriber("lab/svc")
        o = bus.owner("lab/svc", level=h.OK, reason="serving")
        t1 = time.monotonic_ns()
        # 0.8 step 1: "The owner puts status OK, and goes on confirming it
        # for 65 s. S and G judge."
        _sleep_until(t1 + 65 * fr.NS)
        r_s, _ = bus.read("lab/svc", s, get=False)
        r_g, info_g = bus.read("lab/svc", None, get=True)
        a_s, a_g = h.judge(r_s), h.judge(r_g)
        puts = _puts(s.of(o.status_key))
        rd = info_g.get("status_reading")
        report.check(sec, "step 1: past the horizon S and G both judge healthy, ok; S has received no put of the "
                          "status since step 1's; G's entry carries its confirmation (core 0.100, freshness.v2 0.7 §2.4)",
                     a_s.expect() == a_g.expect() == {"verdict": "healthy", "reason": "ok", "level": "ok"}
                     and len(puts) == 1 and rd is not None and rd.confirmed_ns is not None
                     and (time.time_ns() - rd.confirmed_ns) / fr.NS < 31,
                     f"S {_answer(a_s)}, G {_answer(a_g)}, {len(puts)} puts, reply aged "
                     f"{None if rd is None or not rd.confirmed_ns else round((time.time_ns() - rd.confirmed_ns) / fr.NS, 2)} s")
        # step 2: the owner stops vouching (zk2py: its writer closed, its bit clear)
        o.owner.close_writer(o.status_key)
        t2 = time.monotonic_ns()
        last = max(t for t, a in _beats(s, "lab/svc", 0) if _vouches(a, o.status_key))
        _sleep_until(last + 65 * fr.NS)
        beats = _beats(s, "lab/svc", t2 + int(0.2 * fr.NS))
        report.check(sec, "step 2: S receives heartbeats, each with the status's bit clear, and no put",
                     bool(beats) and not any(_vouches(a, o.status_key) for _, a in beats)
                     and len(_puts(s.of(o.status_key))) == 1, f"{len(beats)} heartbeats")
        r_s, info_s = bus.read("lab/svc", s, get=False)
        r_g, info_g = bus.read("lab/svc", None, get=True)
        a_s, a_g = h.judge(r_s), h.judge(r_g)
        pres = info_s["presence"]
        reading = info_g.get("status_reading")
        age = (time.time_ns() - reading.confirmed_ns) / fr.NS if reading is not None and reading.confirmed_ns else None
        report.check(sec, "step 3: the owner's instance and interface tokens are present; 65 s after the last "
                          "heartbeat that vouched for the status S judges stale, beyond_horizon, and G stale, "
                          "beyond_horizon, its entry's confirmation more than 60.5 s old; neither unhealthy, FAILED or OK "
                          "as current",
                     len(pres.instances) == 1 and len(pres.alive) >= 1
                     and a_s.expect() == a_g.expect() == {"verdict": "stale", "reason": "beyond_horizon", "level": None}
                     and age is not None and age > 60.5,
                     f"tokens {len(pres.instances)}+{len(pres.alive)}; S {_answer(a_s)}, G {_answer(a_g)}, "
                     f"reply aged {age if age is None else round(age, 2)} s")
    finally:
        bus.close()


# -- §3 ------------------------------------------------------------------------------

def section3(report: Report) -> None:
    sec = "§3 aggregation"
    bus = Bus()
    try:
        s = bus.subscriber("lab/svc")
        o = bus.owner("lab/svc", level=h.OK, reason="serving")
        o.set_check("disk", h.OK)
        o.set_check("net", h.OK)
        time.sleep(0.5)
        disk_k, net_k = o.check_key("disk"), o.check_key("net")

        def merged(t0: int) -> list[fr.Received]:
            recs = [(k, r) for k in (o.status_key, disk_k, net_k) for r in s.of(k) if r.arrival_ns >= t0]
            recs.sort(key=lambda kr: kr[1].arrival_ns)
            return recs

        def safe(recs) -> bool:
            st = dk = None
            for k, r in recs:
                if k == o.status_key and r.kind == "put":
                    st = h.status_level(r.payload)
                elif k == disk_k and r.kind == "put":
                    dk = h.check_level(r.payload)
                if isinstance(st, str) and isinstance(dk, str) and h.worse(dk, st):
                    return False
            return True

        def g_reading():
            r, info = bus.read("lab/svc", s, get=True)
            levels = {x.key.rsplit("/", 1)[1]: ("deleted" if x.deleted else
                                                 h.status_level(x.payload) if x.key.endswith("/status")
                                                 else h.check_level(x.payload)) for x in info["replies"]}
            return r, levels

        t2 = time.monotonic_ns()
        o.set_check("disk", h.FAILED, "the disk is full")
        time.sleep(0.5)
        recs = merged(t2)
        order = [(k.rsplit("/", 1)[1], h.status_level(r.payload) if k == o.status_key else h.check_level(r.payload))
                 for k, r in recs]
        r2, levels = g_reading()
        a2 = h.judge(r2)
        report.check(sec, "step 2: S receives the status FAILED before checks/disk FAILED, and at no delivery is "
                          "its latest status better than its latest disk; G holds status FAILED, disk FAILED, net "
                          "OK, and judges unhealthy, failed",
                     order == [("status", "failed"), ("disk", "failed")] and safe(merged(0))
                     and levels == {"status": "failed", "disk": "failed", "net": "ok"}
                     and a2.expect() == {"verdict": "unhealthy", "reason": "failed", "level": "failed"},
                     f"order {order}; G {levels}; {_answer(a2)}")
        t3 = time.monotonic_ns()
        o.set_check("disk", h.OK)
        o.set_status(h.OK, "serving")
        time.sleep(0.5)
        order = [(k.rsplit("/", 1)[1], h.status_level(r.payload) if k == o.status_key else h.check_level(r.payload))
                 for k, r in merged(t3)]
        r3, _ = g_reading()
        a3 = h.judge(r3)
        report.check(sec, "step 3: S receives checks/disk OK before the status OK; G judges healthy, ok",
                     order == [("disk", "ok"), ("status", "ok")] and safe(merged(0))
                     and a3.expect() == {"verdict": "healthy", "reason": "ok", "level": "ok"},
                     f"order {order}; {_answer(a3)}")
        t4 = time.monotonic_ns()
        o.retire_check("net")
        time.sleep(0.5)
        recs4 = [(k.rsplit("/", 1)[1], r.kind) for k, r in merged(t4)]
        _, levels = g_reading()
        disk_seen = [h.check_level(r.payload) for r in s.of(disk_k)]
        net_seen = [r.kind if r.kind == "delete" else h.check_level(r.payload) for r in s.of(net_k)]
        report.check(sec, "step 4: S receives a delete of checks/net and no status put for it; G holds the status "
                          "and disk, and a reply_del for checks/net (core S2, S3); no check was re-put: S received "
                          "each only when it changed (§2.3)",
                     recs4 == [("net", "delete")] and levels == {"status": "ok", "disk": "ok", "net": "deleted"}
                     and disk_seen == ["ok", "failed", "ok"] and net_seen == ["ok", "delete"],
                     f"after the retire {recs4}; G {levels}; disk {disk_seen}, net {net_seen}")
        # §2.2: a status better than the worst current check is refused.
        o.set_check("disk", h.FAILED, "full again")
        try:
            o.set_status(h.OK, "fine?")
            refused = None
        except h.HealthRuleError as e:
            refused = str(e)
        report.check(sec, "§2.2: the owner refuses to put a status better than its worst current check",
                     refused is not None, str(refused))
    finally:
        bus.close()


# -- §4 ------------------------------------------------------------------------------

def section4(report: Report) -> None:
    from . import live

    sec = "§4 clock ahead"
    bus = Bus()
    beat = "zk2py/health/heartbeat"
    stop_beat = threading.Event()
    try:
        s = bus.subscriber("lab/ahead")
        off = Offset(2 * fr.NS)
        o = bus.owner("lab/ahead", level=h.OK, reason="serving", heartbeat=beat, offset=off)
        zid = _zid(str(o.owner.session.zid()))
        time.sleep(0.5)
        sk, fk = o.status_key, o.key("stream/faults")
        report.check(sec, "step 1: S receives the status put before any heartbeat",
                     any(r.kind == "put" for r in s.of(sk)), f"{len(s.of(sk))} deliveries")
        detected: list[int] = []

        def beats() -> None:
            while not stop_beat.wait(0.3):
                bus.g_session.put(beat, b"beat")  # no stamp of its own: R1 stamps it
                if o.owner.ahead and not detected:
                    detected.append(time.monotonic_ns())

        th = threading.Thread(target=beats, daemon=True)
        th.start()
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline and not o.owner.ahead:
            time.sleep(0.05)
        t_det = time.monotonic_ns()
        if not report.check(sec, "step 2: the owner reports itself ahead from the router-stamped heartbeat",
                            o.owner.ahead, str(o.owner.events)):
            return
        time.sleep(1.0)
        faults = [(r, h.decode_fault(r.payload)) for r in s.of(fk)]
        first = [(r, f) for r, f in faults if f and f["code"] == "clock_ahead" and r.arrival_ns <= t_det + fr.NS]
        report.check(sec, "step 2: within 1 s of the detection S receives a faults sample, code clock_ahead, level "
                          "FAILED, its detail naming the offset",
                     len(first) >= 1 and first[0][1]["level"] == h.FAILED and "2." in first[0][1]["detail"],
                     str([f for _, f in faults]))
        # 0.9 (#978 F6): "The owner then puts status OK with the reason
        # "ahead"": the guard holds no status, which carries no Zenoh
        # timestamp (core §4.3, 0.88).
        put_ok = "ok"
        try:
            o.set_status(h.OK, "ahead")
        except Exception as e:  # noqa: BLE001 - named
            put_ok = f"{type(e).__name__}: {e}"
        t_put = time.monotonic_ns()
        # Without root (0.2), "the fault's stamp is the owner's honest one,
        # set or re-stamped by its own session (v1.md §2.5), and is not
        # checked": S judges nothing by it.
        _sleep_until(t_det + 65 * fr.NS)
        puts_after = _puts([r for r in s.of(sk) if r.kind == "put" and r.arrival_ns > t_det])
        ahead_put = [r for r in puts_after if (_status(r.payload) or {}).get("reason") == "ahead"]
        ahead_faults = [r for r, f in ((r, h.decode_fault(r.payload)) for r in s.of(fk)) if f and f["code"] == "clock_ahead"]
        gap = (ahead_faults[1].arrival_ns - ahead_faults[0].arrival_ns) / fr.NS if len(ahead_faults) > 1 else None
        pres = live.list_presence(bus.t_session, "zk2/lab/ahead/@zk/**")
        r, _ = bus.read("lab/ahead", s, get=False)
        a = h.judge(r)
        q = h.clock_from(s, "lab/ahead")
        # every heartbeat from the detection on (one already on its way excepted)
        held_beats = _beats(s, "lab/ahead", t_det + 200_000_000)
        all_held = bool(held_beats) and all(b.get("time") is None and _vouches(b, sk) for _, b in held_beats)
        report.check(sec, "step 2: the guard does not hold the status: the owner's put succeeds, and S receives it; "
                          "the fault again within 31 s; the tokens stay present",
                     put_ok == "ok" and len(ahead_put) == 1 and gap is not None and gap <= 31
                     and len(pres.instances) == 1,
                     f"put {put_ok}, {len(ahead_put)} 'ahead' put(s) received of {len(puts_after)} after the "
                     f"detection, fault gap {gap if gap is None else round(gap, 2)} s, tokens "
                     f"{len(pres.instances)}+{len(pres.alive)}")
        report.check(sec, "step 2: every heartbeat from the detection on vouches for the status, its time unknown, "
                          "all ones (freshness.v2 §2.10); at 65 s S judges healthy, ok; the faults and the heartbeats "
                          "answer \"is its clock ahead?\" yes",
                     all_held and a.expect() == {"verdict": "healthy", "reason": "ok", "level": "ok"} and q == "yes",
                     f"{len(held_beats)} heartbeats, each held and vouching {all_held}; {_answer(a)}, clock ahead: {q}"
                     f" ({h.heard_from(s, 'lab/ahead')[-4:]})")
        # Step 3.
        off.ns = 0
        t_fix = time.monotonic_ns()
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline and o.owner.ahead:
            time.sleep(0.05)
        t_rel = time.monotonic_ns()
        _sleep_until(t_fix + 35 * fr.NS)
        reput = _puts([x for x in s.of(sk) if x.kind == "put" and x.arrival_ns > t_put + fr.NS])
        known = [t for t, b in _beats(s, "lab/ahead", t_fix) if b.get("time") is not None and _vouches(b, sk)]
        first = (known[0] - t_fix) / fr.NS if known else None
        faults_after = [x for x, f in ((x, h.decode_fault(x.payload)) for x in s.of(fk))
                        if f and f["code"] == "clock_ahead" and known and x.arrival_ns > known[0]]
        r, _ = bus.read("lab/ahead", s, get=False)
        a = h.judge(r)
        q = h.clock_from(s, "lab/ahead")
        report.check(sec, "step 3: the clock set right, the guard releases; within 30 s S receives a heartbeat whose "
                          "time is known again (freshness.v2 §2.10), no put of the status after step 2's, and no "
                          "clock_ahead fault after that heartbeat; S judges healthy, ok, and the clock question "
                          "answers no",
                     not o.owner.ahead and first is not None and first <= 30 + JITTER_S and not reput
                     and not faults_after and a.expect() == {"verdict": "healthy", "reason": "ok", "level": "ok"}
                     and q == "no",
                     f"a known time {first if first is None else round(first, 2)} s after the correction "
                     f"(released at {round((t_rel - t_fix) / fr.NS, 2)} s), {len(reput)} puts, {len(faults_after)} "
                     f"faults after, {_answer(a)}, clock ahead: {q}; events {o.owner.events}")
    finally:
        stop_beat.set()
        bus.close()


def section4r(report: Report) -> None:
    """§4 steps 1 to 3 against the owner example, a black box (round 98, core
    0.88): ``--health --clock-reference <key> --clock-offset-ms 2000``, its
    ``status`` and ``clock-offset`` commands, S zk2py's subscriber following
    its heartbeats. The detection is the first ``clock_ahead`` fault S
    hears (its report, core §4.3)."""
    from . import live
    from .live_interop import Owner as Exe

    sec = "§4 clock ahead (the owner example)"
    from .blackbox import BLACKBOX

    exe = BLACKBOX / "owner"
    if not exe.is_file():
        report.check(sec, "the section ran", False, f"{exe} not found")
        return
    bus = Bus()
    beat = "zk2py/health/heartbeat"
    stop_beat = threading.Event()
    p = None
    try:
        s = bus.subscriber("lab/ahead")
        p = Exe(exe, "lab/ahead", [], connect=bus.endpoint,
                flags=["--health", "--health-status", "OK:serving", "--clock-reference", beat,
                       "--clock-offset-ms", "2000"])
        if p.wait_for("ready ", 120) is None:
            report.check(sec, "the section ran", False, f"the owner example did not start: {p.stderr[-3:]}")
            return
        time.sleep(0.5)
        sk, fk = "zk2/lab/ahead/health.v1/state/status", "zk2/lab/ahead/health.v1/stream/faults"
        report.check(sec, "step 1: S receives the status put before any heartbeat",
                     any(r.kind == "put" for r in s.of(sk)), f"{len(s.of(sk))} deliveries")

        def beats() -> None:
            while not stop_beat.wait(0.3):
                bus.g_session.put(beat, b"beat")

        threading.Thread(target=beats, daemon=True).start()

        def ahead_faults() -> list[fr.Received]:
            return [r for r, f in ((r, h.decode_fault(r.payload)) for r in s.of(fk)) if f and f["code"] == "clock_ahead"]

        deadline = time.monotonic() + 15
        while time.monotonic() < deadline and not ahead_faults():
            time.sleep(0.05)
        if not report.check(sec, "step 2: the owner reports itself ahead: S receives a faults sample, code "
                                 "clock_ahead, level FAILED", bool(ahead_faults())
                            and h.decode_fault(ahead_faults()[0].payload)["level"] == h.FAILED,
                            f"{[h.decode_fault(r.payload) for r in s.of(fk)]}"):
            return
        t_det = ahead_faults()[0].arrival_ns
        got = p.command("status OK ahead") or ""
        t_put = time.monotonic_ns()
        _sleep_until(t_det + 65 * fr.NS)
        puts_after = _puts([r for r in s.of(sk) if r.kind == "put" and r.arrival_ns > t_det])
        ahead_put = [r for r in puts_after if (_status(r.payload) or {}).get("reason") == "ahead"]
        af = ahead_faults()
        gap = (af[1].arrival_ns - af[0].arrival_ns) / fr.NS if len(af) > 1 else None
        pres = live.list_presence(bus.t_session, "zk2/lab/ahead/@zk/**")
        r, _ = bus.read("lab/ahead", s, get=False)
        a = h.judge(r)
        q = h.clock_from(s, "lab/ahead")
        held_beats = _beats(s, "lab/ahead", t_det + 200_000_000)
        all_held = bool(held_beats) and all(b.get("time") is None and _vouches(b, sk) for _, b in held_beats)
        report.check(sec, "step 2: the guard does not hold the status: the owner's put succeeds, and S receives it; "
                          "the fault again within 31 s; the tokens stay present",
                     not got.startswith("refused") and len(ahead_put) == 1 and gap is not None and gap <= 31
                     and len(pres.instances) == 1,
                     f"put {got!r}, {len(ahead_put)} 'ahead' put(s) of {len(puts_after)} after the detection, fault "
                     f"gap {gap if gap is None else round(gap, 2)} s, tokens {len(pres.instances)}+{len(pres.alive)}")
        report.check(sec, "step 2: every heartbeat from the detection on vouches for the status, its time unknown, "
                          "all ones; at 65 s S judges healthy, ok; the clock question answers yes",
                     all_held and a.expect() == {"verdict": "healthy", "reason": "ok", "level": "ok"} and q == "yes",
                     f"{len(held_beats)} heartbeats, each held and vouching {all_held}; {_answer(a)}, clock ahead: {q}")
        t_fix = time.monotonic_ns()
        fixed = p.command("clock-offset 0") or ""
        _sleep_until(t_fix + 35 * fr.NS)
        reput = _puts([x for x in s.of(sk) if x.kind == "put" and x.arrival_ns > t_put + fr.NS])
        known = [t for t, b in _beats(s, "lab/ahead", t_fix) if b.get("time") is not None and _vouches(b, sk)]
        first = (known[0] - t_fix) / fr.NS if known else None
        faults_after = [x for x in ahead_faults() if known and x.arrival_ns > known[0]]
        r, _ = bus.read("lab/ahead", s, get=False)
        a = h.judge(r)
        q = h.clock_from(s, "lab/ahead")
        report.check(sec, "step 3: the clock set right; within 30 s S receives a heartbeat whose time is known again, "
                          "no put of the status after step 2's, no clock_ahead fault after that heartbeat; S judges "
                          "healthy, ok, and the clock question answers no",
                     first is not None and first <= 30 + JITTER_S and not reput and not faults_after
                     and a.expect() == {"verdict": "healthy", "reason": "ok", "level": "ok"} and q == "no",
                     f"{fixed!r}; a known time {first if first is None else round(first, 2)} s after the correction, "
                     f"{len(reput)} puts, {len(faults_after)} faults after, {_answer(a)}, clock ahead: {q}")
    finally:
        stop_beat.set()
        if p is not None:
            p.close()
        bus.close()


def section4m(report: Report) -> None:
    """§4's [moved clock] expectations (0.2): an owner that stamps from its
    offset clock on a session without an HLC, as a client's is by default
    (core §4.1), so its puts reach R1 dated 2 s ahead."""
    from . import live
    from .live_interop import _r1

    sec = "§4 clock ahead [moved clock]"
    beat = "zk2py/health/heartbeat"

    def run(drop: bool):
        r1, ep, r1_zid = _r1(drop_future=drop)
        s_sess, g_sess = live.open_client(ep), live.open_client(ep)
        s = fr.Subscriber(s_sess, "zk2/lab/ahead/health.v1/**")
        from .contract import load_contract

        hc = load_contract(h.CONTRACT)
        s.follow("lab/ahead", hc.canonical, hc.fingerprint)
        time.sleep(0.2)
        off = Offset(2 * fr.NS)
        o = h.HealthOwner("lab", "ahead", connect=ep, level=h.OK, reason="serving", heartbeat=beat, hlc=False,
                          clock_ns=off.ns_now)
        o.owner.clock = off.clock(o.owner)
        stop = threading.Event()
        try:
            o.start()
            time.sleep(0.5)
            got_status = [r for r in s.of(o.status_key) if r.kind == "put"]

            def beats() -> None:
                while not stop.wait(0.3):
                    g_sess.put(beat, b"beat")

            threading.Thread(target=beats, daemon=True).start()
            deadline = time.monotonic() + 10
            while time.monotonic() < deadline and not o.owner.ahead:
                time.sleep(0.05)
            t_det = time.monotonic_ns()
            o.set_status(h.OK, "ahead")  # step 2 (0.9): not held
            time.sleep(1.0 if not drop else 3.0)
            if drop:
                # the next heartbeat covering health.v1, its time unknown: the
                # heartbeat beats at 28.5 s in revision 1.1
                deadline = time.monotonic() + 31
                while time.monotonic() < deadline and not _beats(s, "lab/ahead", t_det + 200_000_000):
                    time.sleep(0.2)
            faults = [(r, h.decode_fault(r.payload)) for r in s.of(o.key("stream/faults"))]
            st = live.get_state(g_sess, o.status_key)
            return dict(ahead=o.owner.ahead, status=got_status, faults=faults, t_det=t_det, r1=r1_zid,
                        owner=str(o.owner.session.zid()), get=st.replies,
                        puts=[r for r in _puts(s.of(o.status_key)) if r.kind == "put"],
                        clock=h.clock_question("present", {"lists": True}, h.heard_from(s, "lab/ahead")),
                        heard=h.heard_from(s, "lab/ahead"))
        finally:
            stop.set()
            o.close()
            s.close()
            s_sess.close()
            g_sess.close()
            r1.close()

    a = run(drop=False)
    report.check(sec, "steps 1 and 2: S receives the status, the owner reports itself ahead, and within 1 s S "
                      "receives clock_ahead at FAILED whose stamp is not the owner's: R1 re-stamped a future-dated "
                      "put (core §4.1)",
                 a["ahead"] and a["status"] and len(a["faults"]) >= 1 and a["faults"][0][1]["code"] == "clock_ahead"
                 and a["faults"][0][1]["level"] == h.FAILED
                 and a["faults"][0][0].arrival_ns <= a["t_det"] + fr.NS
                 and _zid(a["faults"][0][0].stamp_id) == _zid(a["r1"]) != _zid(a["owner"]),
                 f"status {len(a['status'])}, fault stamp id "
                 f"{a['faults'][0][0].stamp_id if a['faults'] else None}, R1 {a['r1']}, owner {a['owner']}")
    b = run(drop=True)
    gets = [(x.stamp_id, h.decode_status(x.payload)) for x in b["get"]]
    report.check(sec, "step 4: under drop_future_timestamp, S receives no fault: R1 dropped it; the status puts "
                      "carry no Zenoh timestamp of the owner's (freshness.v2 §2.4), so R1 stamps them and S receives "
                      "them, step 2's after the detection included; G's reply holds the status under the owner's "
                      "stamp, since a router never drops or re-stamps a reply (core §4.1); its readers have only "
                      "the heartbeat's time unknown to go on, which answers no clock question alone (0.9)",
                 b["ahead"] and not b["faults"] and len(b["puts"]) == 2
                 and all(_zid(x.stamp_id) == _zid(b["r1"]) for x in b["puts"]) and len(gets) == 1
                 # core 0.100 (round 105): the status is a snapshot's entry, under
                 # the owner's run in band, with no Zenoh timestamp (F-503)
                 and gets[0][0] is None and b["get"][0].snapshot_key is not None and b["get"][0].inband is not None
                 and gets[0][1] is not None and gets[0][1]["level"] == h.OK
                 and b["clock"] == ("unobservable", "time_unknown"),
                 f"ahead {b['ahead']}, {len(b['faults'])} faults, {len(b['puts'])} status puts; GET {gets}; owner "
                 f"{b['owner']}; clock question {b['clock']} over {b['heard'][-4:]}")


# -- §5 ------------------------------------------------------------------------------

def section5(report: Report, n: int = 100) -> None:
    from . import live
    from .contract import load_contract

    sec = f"§5 a tokenless set of {n}"
    bus = Bus()
    try:
        nav = load_contract(NAV)
        for i in range(n):
            bus.owner(f"p5/dev{i}", level=h.OK, reason="serving", contracts=[nav],
                      tokenless={"health.v1", "freshness.v2"})
        time.sleep(1.5)
        pres = live.list_presence(bus.t_session, "zk2/p5/*/@zk/**")
        alive_health = live.list_presence(bus.t_session, "zk2/p5/*/@zk/alive/health.v1/**")
        alive_fv2 = live.list_presence(bus.t_session, "zk2/p5/*/@zk/alive/freshness.v2/**")
        report.check(sec, f"{2 * n} tokens: an instance token and an alive/nav.v2 token each, no alive/health.v1 "
                          "or alive/freshness.v2 token (0.8); the second reads find nothing",
                     len(pres.instances) == n and len(pres.alive) == n
                     and all(a["iface"] == "nav.v2" for a in pres.alive) and pres.complete
                     and alive_health.count == 0 and alive_health.complete
                     and alive_fv2.count == 0 and alive_fv2.complete,
                     f"{len(pres.instances)} instance, {len(pres.alive)} alive "
                     f"({sorted({a['iface'] for a in pres.alive})}), second read {alive_health.count}")
        answers = []
        tokenless = 0
        for i in range(n):
            r, info = bus.t_read(f"p5/dev{i}")
            tokenless += int(r.descriptor == {"lists": True, "token": False})
            answers.append(h.judge(r))
        healthy = sum(1 for a in answers if a.expect() == {"verdict": "healthy", "reason": "ok", "level": "ok"})
        report.check(sec, f"every descriptor lists health.v1 with \"token\": false; the tool finds all {n} providers "
                          "and judges each healthy, ok: none reported as not implementing health.v1",
                     tokenless == n and healthy == n and not any(a.reason == "not_listed" for a in answers),
                     f"{tokenless} tokenless, {healthy} healthy; roll-up {h.rollup(answers)}")
    finally:
        bus.close()


# -- §6 ------------------------------------------------------------------------------

class StandInArchive:
    """§6's archive, a stand-in for ``archive.v1`` (core §4.4's
    requirements, not the profile): it records the owner's mutations of
    ``origin`` with their stamps, aligns once from the owner at start, and
    answers GETs on its archive form, each reply carrying core §4.4's
    attachment."""

    def __init__(self, endpoint: str, address: str, origin: str, type_identity: dict[str, Any]):
        import zenoh

        from . import live

        self.session = live.open_client(endpoint)
        self.address, self.identity = address, type_identity
        self.held: dict[str, tuple[bytes | None, str, Any, bool]] = {}
        self.lock = threading.Lock()
        self.entities = [
            self.session.declare_subscriber(origin, zenoh.handlers.Callback(self._record)),
            self.session.declare_queryable(f"zk2/{address}/archive.v1/@state/**",
                                           zenoh.handlers.Callback(self._answer), complete=False),
        ]
        time.sleep(0.2)
        # Alignment (core §4.4, 0.100): the owner's collection, read as S4
        # says, consolidation None, by snapshot; a value read from the owner
        # is confirmed, "a snapshot's entry included", and "From a snapshot's
        # entry it keeps the put the entry stands for: CH of form 0 with the
        # snapshot's tag and the run's width, the entry's run and seq, then
        # its record" (live.StateReply.raw).
        st = live.get_state(self.session, origin, timeout=1.0)
        for r_ in st.replies:
            if r_.refusal is not None:
                continue
            with self.lock:
                # core §4.4 (0.73): "A put's bytes are kept as the owner wrote
                # them, its envelope included, and a reply gives them back
                # verbatim."
                self.held[r_.key] = (None if r_.deleted else r_.raw, r_.encoding, None, True)

    def form(self, key: str) -> str:
        from .slug import slug

        return f"zk2/{self.address}/archive.v1/@state/" + "/".join(slug(c) for c in key.split("/")[1:])

    def _record(self, sample: Any) -> None:
        import zenoh

        key = str(sample.key_expr)
        with self.lock:
            self.held[key] = (None if sample.kind == zenoh.SampleKind.DELETE else sample.payload.to_bytes(),
                              str(sample.encoding), sample.timestamp, True)

    def _answer(self, query: Any) -> None:
        import zenoh

        asked = zenoh.KeyExpr(str(query.key_expr))
        with self.lock:
            held = dict(self.held)
        for key, (payload, enc, ts, confirmed) in held.items():
            form = self.form(key)
            if not asked.intersects(zenoh.KeyExpr(form)):
                continue
            att = json.dumps({**self.identity, "confirmed": confirmed}).encode()
            if payload is None:
                query.reply_del(form, timestamp=ts, attachment=att)
            else:
                query.reply(form, payload, encoding=enc, timestamp=ts, attachment=att)

    def close(self) -> None:
        for e in self.entities:
            e.undeclare()
        self.session.close()


def section6(report: Report) -> None:
    import zenoh

    sec = "§6 an absent owner"
    bus = Bus()
    try:
        o = bus.owner("lab/svc", level=h.DEGRADED, reason="upstream lost")
        status_r = next(r for r in o.contract.canonical["resources"] if r["template"] == "status")
        archive = StandInArchive(bus.endpoint, "lab/archive", "zk2/lab/svc/health.v1/state/**",
                                 {"iface": "health.v1", "contract": o.contract.fingerprint,
                                  "type": status_r["type"]})
        bus.extra.append(archive)
        time.sleep(0.5)
        r1, _ = bus.t_read("lab/svc")
        a1 = h.judge(r1)
        report.check(sec, "step 1: unhealthy, degraded",
                     a1.expect() == {"verdict": "unhealthy", "reason": "degraded", "level": "degraded"}, _answer(a1))
        o.close()
        bus.owners.remove(o)
        time.sleep(1.0)
        r3, info = bus.t_read("lab/svc")
        form = archive.form(o.status_key)
        got: list[Any] = []
        done = threading.Event()
        bus.t_session.get(form, zenoh.handlers.Callback(got.append, done.set),
                          target=zenoh.QueryTarget.ALL, consolidation=zenoh.ConsolidationMode.NONE, timeout=1.0)
        done.wait(3.0)
        shown = []
        for rep in got:
            if rep.ok is None:
                continue
            smp = rep.ok
            att = json.loads(smp.attachment.to_bytes()) if smp.attachment is not None else {}
            st = h.decode_status(zk2c.payload_of(smp))
            shown.append({"level": h.NAMES.get(st["level"]) if st else None, "stamp": str(smp.timestamp),
                          "confirmed": att.get("confirmed")})
        # The archive's answer joins the reading as last-known: it plays no part.
        r3 = h.Reading(r3.presence, r3.descriptor, None, [*(r3.observations or []), fr.Archive()], r3.level, r3.checks)
        a3 = h.judge(r3)
        status_replies = [x for x in info.get("replies", []) if x.key == o.status_key]
        report.check(sec, "step 3: the tokens are gone and the owner's GET draws no reply; T judges not asked, "
                          "absent, never FAILED or stale, and shows the archived status, DEGRADED, as last-known "
                          "with its stamp and confirmed (core S6)",
                     r3.presence == "absent" and not status_replies
                     and a3.expect() == {"verdict": "not_asked", "reason": "absent", "level": None}
                     and len(shown) == 1 and shown[0]["level"] == "degraded" and shown[0]["confirmed"] is True,
                     f"{_answer(a3)}; last-known {shown}")
    finally:
        bus.close()


# -- §7 ------------------------------------------------------------------------------

def section7(report: Report) -> None:
    sec = "§7 an inconsistent status, for a tool"
    bus = Bus()
    try:
        liar = bus.owner("lab/liar", level=h.OK, reason="fine")
        # It breaks §2.2 on purpose, past HealthOwner's own refusal.
        liar.owner.set_state(liar.check_key("disk"), h.check(h.FAILED, "full", time.time_ns()))
        frank = bus.owner("lab/frank", level=h.FAILED, reason="the job cannot run")
        frank.set_check("disk", h.OK)
        time.sleep(0.5)
        rows: dict[str, list[tuple[h.Answer, str]]] = {"lab/liar": [], "lab/frank": []}
        for i in range(2):
            if i:
                time.sleep(2.0)
            for addr in ("lab/liar", "lab/frank"):
                r, _ = bus.t_read(addr)
                rows[addr].append((h.judge(r), h.status_agrees(r)))
        liar_ok = all(a.expect() == {"verdict": "unhealthy", "reason": "inconsistent", "level": "failed"} and ag == "no"
                      for a, ag in rows["lab/liar"])
        report.check(sec, "lab/liar: each reading unhealthy, inconsistent, at FAILED, never healthy; the break of "
                          "§2.2 reported as a finding about the owner, seen in both readings",
                     liar_ok, str([(_answer(a), ag) for a, ag in rows["lab/liar"]]))
        frank_ok = all(a.expect() == {"verdict": "unhealthy", "reason": "failed", "level": "failed"} and ag == "yes"
                       for a, ag in rows["lab/frank"])
        report.check(sec, "lab/frank: unhealthy, failed, and no break: a status may be worse than its checks",
                     frank_ok, str([(_answer(a), ag) for a, ag in rows["lab/frank"]]))
    finally:
        bus.close()


# -- §8 ------------------------------------------------------------------------------

def _face_router(workdir: str, denied: list[str]):
    """V: a router with usrpwd, and access control under ``allow`` denying
    ``denied`` to the ground principal, both ways, every message."""
    import zenoh

    from .owner import free_loopback_port

    users = os.path.join(workdir, f"users-{len(denied)}.txt")
    with open(users, "w") as f:
        f.write("ground:ground-pw\nvehicle:vehicle-pw\n")
    port = free_loopback_port()
    conf = zenoh.Config()
    conf.insert_json5("mode", json.dumps("router"))
    conf.insert_json5("listen/endpoints", json.dumps([f"tcp/127.0.0.1:{port}"]))
    conf.insert_json5("scouting/multicast/enabled", "false")
    conf.insert_json5("timestamping/enabled", "true")
    conf.insert_json5("transport/auth/usrpwd", json.dumps({"dictionary_file": users}))
    messages = ["put", "delete", "declare_subscriber", "query", "declare_queryable", "reply",
                "liveliness_token", "declare_liveliness_subscriber", "liveliness_query"]
    conf.insert_json5("access_control", json.dumps({
        "enabled": True, "default_permission": "allow",
        "rules": [{"id": "face", "messages": messages, "flows": ["ingress", "egress"], "permission": "deny",
                   "key_exprs": denied}],
        "subjects": [{"id": "ground", "usernames": ["ground"]}],
        "policies": [{"id": "the-face", "rules": ["face"], "subjects": ["ground"]}],
    }))
    return zenoh.open(conf), f"tcp/127.0.0.1:{port}"


def _face_run(report: Report, workdir: str, run: str, crosses: bool, wait_s: float) -> None:
    from . import live

    sec = f"§8 across a constrained face, run {run}"
    denied = ["zk2/**/@zk/**"] + ([] if crosses else ["zk2/vehicle-01/*/*/state/**"])
    v, ep = _face_router(workdir, denied)
    ground = live.open_client(ep, ("ground", "ground-pw"))
    o = None
    s = None
    try:
        addr = "vehicle-01/nav"
        s = fr.Subscriber(ground, f"zk2/{addr}/health.v1/state/status")  # statically, without presence
        from .contract import load_contract

        hc = load_contract(h.CONTRACT)
        s.follow(addr, hc.canonical, hc.fingerprint)  # 0.8: the heartbeat crosses with the status
        time.sleep(0.2)
        t0 = time.monotonic_ns()
        _sleep_until(t0 + int(wait_s * fr.NS))
        a1 = h.judge(h.read_across(s, addr, crosses))
        want1 = ({"verdict": "unobservable", "reason": "nothing_crossed", "level": None} if crosses
                 else {"verdict": "unobservable", "reason": "face_closed", "level": None})
        report.check(sec, f"step 1: before the owner starts, G receives nothing and judges {want1['reason']}",
                     not s.of(f"zk2/{addr}/health.v1/state/status") and a1.expect() == want1, _answer(a1))
        o = h.HealthOwner("vehicle-01", "nav", level=h.OK, reason="serving", connect=ep,
                          auth=("vehicle", "vehicle-pw"))
        o.start()
        t1 = time.monotonic_ns()
        _sleep_until(t1 + int(wait_s * fr.NS))
        got = s.of(o.status_key)
        tokens = live.list_presence(ground, "zk2/vehicle-01/*/@zk/**")
        a2 = h.judge(h.read_across(s, addr, crosses))
        if crosses:
            vouching = [t for t, a in _beats(s, addr, t1) if _vouches(a, o.status_key)]
            report.check(sec, "step 2: G receives the status, and the heartbeats that vouch for it (link.v1 §2.9), "
                              "sees no token across the face, and judges healthy, ok",
                         len(_puts(got)) == 1 and bool(vouching) and tokens.count == 0
                         and a2.expect() == {"verdict": "healthy", "reason": "ok", "level": "ok"},
                         f"{len(_puts(got))} puts, {len(vouching)} vouching heartbeats, {tokens.count} tokens seen; "
                         f"{_answer(a2)}")
            o.owner.close_writer(o.status_key)
            last = s.of(o.status_key)[-1].arrival_ns
            _sleep_until(max(last, time.monotonic_ns()) + int(wait_s * fr.NS))
            a3 = h.judge(h.read_across(s, addr, crosses))
            report.check(sec, "step 3: the owner stops vouching, G judges stale: never down, never FAILED, never "
                              "absent",
                         a3.verdict == "stale" and a3.level is None, _answer(a3))
        else:
            report.check(sec, "step 2: nothing of the status crosses; G judges face_closed, never unhealthy, failed "
                              "or down",
                         not got and a2.expect() == {"verdict": "unobservable", "reason": "face_closed", "level": None},
                         f"{len(got)} deliveries; {_answer(a2)}")
    finally:
        if s is not None:
            s.close()
        if o is not None:
            o.close()
        ground.close()
        v.close()


def section8(report: Report, wait_s: float = 65.0) -> None:
    workdir = tempfile.mkdtemp(prefix="zk2py-health-")
    runs = [threading.Thread(target=_face_run, args=(report, workdir, name, crosses, wait_s))
            for name, crosses in (("A", True), ("B", False))]
    for t in runs:
        t.start()
    for t in runs:
        t.join()


SECTIONS = {"1": section1, "2": section2, "3": section3, "4": section4, "4r": section4r, "4m": section4m, "5": section5,
            "6": section6, "7": section7, "8": section8}
#: run only when asked by --only: §4 against the owner example, a black box
OPT_IN = {"4r"}


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="python -m zk2py.health_scenarios", description=__doc__.split("\n")[0])
    ap.add_argument("--only", action="append", choices=sorted(SECTIONS), help="run only these sections")
    args = ap.parse_args(argv)
    report = Report()
    for name, fn in SECTIONS.items():
        if args.only and name not in args.only:
            continue
        if not args.only and name in OPT_IN:
            continue
        try:
            fn(report)
        except Exception as e:  # noqa: BLE001 - a section that cannot run is a failure
            report.check(f"§{name}", "the section ran", False, f"{type(e).__name__}: {e}")
    passed = sum(1 for r in report.rows if r[2])
    failed = len(report.rows) - passed
    print(f"health scenarios: {passed} passed, {failed} failed")
    for sec, name, ok, detail in report.rows:
        if not ok:
            print(f"FAIL [{sec}] {name}: {detail}")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())

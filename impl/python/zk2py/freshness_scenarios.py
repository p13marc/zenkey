"""The ``freshness.v1`` scenarios (``spec/profiles/freshness/scenarios.md``)
on an in-process zenoh-python router: ``python -m zk2py.freshness_scenarios
[--only 1 …]``.

- **The bus** is the text's: R1, a router with timestamping on, and the
  owner, the subscriber S and the GET reader G as its clients, each with a
  session of its own. Timeouts are 1 s.
- **The contract** is ``beacon.v1``, copied from the text into
  ``impl/python/interop/freshness/beacon.v1.toml``; the owner is
  ``lab/beacon``, zk2py's :class:`zk2py.owner.Owner`.
- **S** is :class:`zk2py.freshness.Subscriber` on the state and stream
  keys. **G** GETs as core S4 says, with the delta at 500 ms, and measures
  its clock from S's deliveries (§2.6, ground 2).
- **Jitter:** a gap between deliveries is allowed 200 ms on loopback, as
  the reference allows.
- **A skewed clock** is the owner's: its ``clock`` is offset (§4, and §2
  step 5's owner ahead). zenoh-python's own session replaces a stamp more
  than its HLC delta ahead of its own clock, so the owner ahead puts with
  its session's stamps until its guard trips. What §2 step 5 checks, the
  guard and what S receives after it, does not depend on them.

Sections §1 to §5 are the runtime's, as the text says. §6, a tool's
verdict on a resource, runs zk2py's tool, :func:`zk2py.freshness.read_service`.

Exit 0 when every check passes, 1 when any fails.
"""

from __future__ import annotations

import argparse
import sys
import threading
import time
from pathlib import Path
from typing import Any

from . import freshness as fr

REPO = Path(__file__).resolve().parents[3]
BEACON = REPO / "impl" / "python" / "interop" / "freshness" / "beacon.v1.toml"
BASE = "zk2/lab/beacon/beacon.v1"
STATUS, INTENT, NOTE, LEVEL = (f"{BASE}/state/status", f"{BASE}/state/intent", f"{BASE}/state/note",
                               f"{BASE}/stream/level")
JITTER_S = 0.2
DELTA_NS = fr.DEFAULT_DELTA_NS


class Report:
    def __init__(self) -> None:
        self.rows: list[tuple[str, str, bool, str]] = []

    def check(self, section: str, name: str, ok: bool, detail: str = "") -> bool:
        self.rows.append((section, name, ok, detail))
        print(f"{'PASS' if ok else 'FAIL'} [{section}] {name}" + (f": {detail}" if detail else ""), flush=True)
        return ok


def _zid(text: Any) -> int | None:
    from .live import _zid_value

    return _zid_value(text)


def offset_clock(owner: Any, ns: int):
    """The owner's clock, ``ns`` off its session's (the reference's
    ``simulate_offset``)."""
    import zenoh

    def clock():
        ts = owner.session.new_timestamp()
        n = ts.get_time_as_ntp64().as_nanos() + ns
        return zenoh.Timestamp(zenoh.NTP64(n // fr.NS, n % fr.NS), ts.get_id())

    return clock


class Bus:
    """R1, and S and G as two client sessions of it. G's trust is measured
    from S's deliveries."""

    def __init__(self) -> None:
        from . import live
        from .contract import load_contract
        from .live_interop import _r1

        self.router, self.endpoint, self.r1_zid = _r1()
        self.s_session = live.open_client(self.endpoint)
        self.g_session = live.open_client(self.endpoint)
        self.trust = fr.ClockTrust(DELTA_NS)
        self.contract = load_contract(BEACON)
        assert self.contract.valid, self.contract.codes
        self.subs: list[fr.Subscriber] = []
        self.owners: list[Any] = []

    def subscriber(self, service: str = "beacon") -> fr.Subscriber:
        s = fr.Subscriber(self.s_session, [f"zk2/lab/{service}/beacon.v1/state/*",
                                           f"zk2/lab/{service}/beacon.v1/stream/*"], self.trust)
        self.subs.append(s)
        time.sleep(0.2)  # the declaration reaches R1 before the owner puts
        return s

    def owner(self, service: str = "beacon", **kw: Any):
        from .owner import Owner

        o = Owner("lab", service, [self.contract], connect=self.endpoint, **kw)
        return o

    def start(self, o: Any, clock_offset_ns: int = 0) -> Any:
        if clock_offset_ns:
            o.clock = offset_clock(o, clock_offset_ns)
        o.start()
        self.owners.append(o)
        return o

    def g(self, key: str, trust: fr.ClockTrust | None = None, at_utc_ns: int | None = None):
        reading = fr.get_reading(self.g_session, key)
        return reading, reading.observation(self.trust if trust is None else trust, at_utc_ns)

    def close(self) -> None:
        for s in self.subs:
            s.close()
        for o in self.owners:
            o.close()
        self.s_session.close()
        self.g_session.close()
        self.router.close()


def _sleep_until(mono_ns: int) -> None:
    d = (mono_ns - time.monotonic_ns()) / fr.NS
    if d > 0:
        time.sleep(d)


def _since(recs: list[fr.Received], t_ns: int) -> list[fr.Received]:
    return [r for r in recs if r.arrival_ns >= t_ns]


def _gaps(recs: list[fr.Received]) -> list[float]:
    return [(b.arrival_ns - a.arrival_ns) / fr.NS for a, b in zip(recs, recs[1:])]


def _h(owner: Any, key: str) -> fr.Horizon:
    for c in owner.contracts:
        for r in c.canonical["resources"]:
            if key.endswith(f"/{c.interface}/{r['token']}/{r['template']}"):
                return fr.horizon(r["kind"], r["annotations"])
    raise KeyError(key)


# -- §1 ------------------------------------------------------------------------------

def section1(report: Report) -> None:
    sec = "§1 the re-put cadence"
    bus = Bus()
    try:
        s = bus.subscriber()
        o = bus.start(bus.owner())
        zid = _zid(str(o.session.zid()))
        time.sleep(0.3)
        # Step 1.
        t1 = time.monotonic_ns()
        o.set_state(STATUS, b"up")
        time.sleep(5.0)
        got = _since(s.of(STATUS), t1)
        gaps = _gaps(got)
        report.check(sec, "step 1: at least 4 re-puts of up in the 5 s after the first, no two deliveries more "
                          "than 1 s apart (ttl/2, with 200 ms of jitter); every one up, text/plain, stamped by the "
                          "owner's zid, each stamp above the one before",
                     len(got) >= 5 and max(gaps) <= 1.0 + JITTER_S and all(r.payload == b"up" for r in got)
                     and all(r.encoding == "text/plain" for r in got)
                     and all(_zid(r.stamp_id) == zid for r in got)
                     and all(b.stamp_ns > a.stamp_ns for a, b in zip(got, got[1:])),
                     f"{len(got)} deliveries, largest gap {max(gaps, default=0):.3f} s, "
                     f"payloads {sorted({r.payload for r in got})}, encodings {sorted({r.encoding for r in got})}")
        # Step 2.
        t2 = time.monotonic_ns()
        change = o.set_state(STATUS, b"down")
        time.sleep(2.0)
        got = _since(s.of(STATUS), t2)
        report.check(sec, "step 2: every delivery from the change on carries down, none up: the change started a "
                          "new interval",
                     len(got) >= 2 and all(r.payload == b"down" for r in got),
                     f"{[r.payload for r in got]}")
        # Step 3.
        before = s.of(STATUS)[-1]
        reading, _ = bus.g(STATUS)
        delivered = {r.stamp for r in s.of(STATUS)}
        report.check(sec, "step 3: G's reply carries down and the stamp of S's latest delivery, a re-put's, not "
                          "the change's (core S2, 0.21)",
                     reading.payload == b"down" and reading.stamp in delivered and reading.stamp != str(change)
                     and reading.stamp_ns is not None and before.stamp_ns is not None
                     and reading.stamp_ns >= before.stamp_ns,
                     f"reply {reading.payload!r} {reading.stamp}; latest before {before.stamp}; change {change}")
        # Step 4.
        t4 = time.monotonic_ns()
        o.set_state(NOTE, b"n")
        time.sleep(3.0)
        got = _since(s.of(NOTE), t4)
        report.check(sec, "step 4: S receives note once: a member with no horizon is not re-put",
                     [r.payload for r in got] == [b"n"], f"{[r.payload for r in got]}")
        # Step 5.
        t5 = time.monotonic_ns()
        o.delete_state(STATUS)
        time.sleep(3.0)
        got = _since(s.of(STATUS), t5)
        report.check(sec, "step 5: S receives the delete, and nothing more on status",
                     [r.kind for r in got] == ["delete"], f"{[(r.kind, r.payload) for r in got]}")
        # Step 6.
        t6 = time.monotonic_ns()
        o.set_state(STATUS, b"up")
        o.close_writer(STATUS)
        time.sleep(3.0)
        got = _since(s.of(STATUS), t6)
        reading, _ = bus.g(STATUS)
        report.check(sec, "step 6: S receives the put, at most one re-put already under way, then nothing; G's "
                          "reply carries up and the stamp of the last delivery: the owner holds the value, and "
                          "nobody confirms it",
                     1 <= len(got) <= 2 and all(r.payload == b"up" for r in got)
                     and reading.payload == b"up" and reading.stamp == got[-1].stamp,
                     f"{len(got)} deliveries; reply {reading.payload!r} {reading.stamp}, last "
                     f"{got[-1].stamp if got else None}")
        # Step 7.
        o.set_state(STATUS, b"up")
        o.close()
        bus.owners.remove(o)
        t7 = time.monotonic_ns()
        time.sleep(3.0)
        got = _since(s.of(STATUS), t7)
        report.check(sec, "step 7: a new writer's put, then the service closes: no delivery on status once it has "
                          "closed", got == [], f"{len(got)} deliveries after the close")
    finally:
        bus.close()


# -- §2 ------------------------------------------------------------------------------

def section2(report: Report) -> None:
    from . import live

    sec = "§2 a stopped refresher goes stale"
    bus = Bus()
    try:
        s = bus.subscriber()
        o = bus.start(bus.owner())
        h = _h(o, STATUS)
        zid_text = str(o.session.zid())
        time.sleep(0.3)
        # Step 1.
        o.set_state(STATUS, b"up")
        time.sleep(2.5)
        v_s = fr.judge_observation("state", h, s.observation(STATUS))
        _, g_obs = bus.g(STATUS)
        v_g = fr.judge_observation("state", h, g_obs)
        measured = bus.trust.measured(next(k for k in bus.trust.measurements if _zid(k) == _zid(zid_text)))
        report.check(sec, "step 1: S and G both judge status fresh; G trusts its clock by a measurement of S's "
                          "deliveries (§2.6, ground 2)",
                     v_s.verdict == fr.FRESH and v_g.verdict == fr.FRESH and measured,
                     f"S {v_s.pair()}, G {v_g.pair()}, measured {measured}")
        # Step 2.
        o.close_writer(STATUS)
        t_close = time.monotonic_ns()
        # Step 3: G at 0.2 s after the close.
        _sleep_until(t_close + 200_000_000)
        _, g1 = bus.g(STATUS)
        v_g1 = fr.judge_observation("state", h, g1)
        last = s.of(STATUS)[-1].arrival_ns
        _sleep_until(last + 1 * fr.NS)
        v_s1 = fr.judge_observation("state", h, s.observation(STATUS))
        _sleep_until(last + 3 * fr.NS)
        v_s3 = fr.judge_observation("state", h, s.observation(STATUS))
        _sleep_until(t_close + 3_500_000_000)
        _, g35 = bus.g(STATUS)
        v_g35 = fr.judge_observation("state", h, g35)
        age1 = None if g1.reply is None else g1.reply.stamp_age_ns
        report.check(sec, "step 3: S fresh 1 s after its last delivery, stale 3 s after it",
                     v_s1.verdict == fr.FRESH and v_s3.verdict == fr.STALE, f"{v_s1.pair()} then {v_s3.pair()}")
        report.check(sec, "step 3: G fresh 0.2 s after the close (the reply at most 1.2 s old, within ttl - "
                          "delta), stale 3.5 s after it (above ttl + delta)",
                     v_g1.verdict == fr.FRESH and v_g35.verdict == fr.STALE and age1 is not None
                     and age1 <= 1_200_000_000,
                     f"{v_g1.pair()} aged {None if age1 is None else age1 / fr.NS:.3f} s, then {v_g35.pair()}")
        # Step 4.
        pres = live.list_presence(bus.g_session, "zk2/lab/beacon/@zk/**")
        present = len(pres.instances) == 1 and len(pres.alive) == 1 and pres.complete
        report.check(sec, "step 4: the owner's instance and interface tokens are present, and status is stale: "
                          "both reported, neither folded into the other (§2.11)",
                     present and v_s3.verdict == fr.STALE,
                     f"presence: {len(pres.instances)} instance, {len(pres.alive)} interface tokens; status "
                     f"{v_s3.verdict}")
        # Step 5: clock ahead.
        ahead_key = "zk2/lab/ahead/beacon.v1/state/status"
        sa = bus.subscriber("ahead")
        beat = "zk2py/freshness/heartbeat"
        oa = bus.start(bus.owner("ahead", heartbeat=beat), clock_offset_ns=2 * fr.NS)
        time.sleep(0.3)
        oa.set_state(ahead_key, b"up")
        time.sleep(0.3)
        deadline = time.monotonic() + 5.0
        while time.monotonic() < deadline and not oa.ahead:
            bus.g_session.put(beat, b"beat")  # no timestamp: R1 stamps it
            time.sleep(0.2)
        t_ahead = time.monotonic_ns()
        refused = None
        try:
            oa.set_state(ahead_key, b"again")
        except Exception as e:  # noqa: BLE001 - the guard's refusal
            refused = type(e).__name__
        time.sleep(3.0)
        after = _since(sa.of(ahead_key), t_ahead)
        last_a = sa.of(ahead_key)[-1].arrival_ns if sa.of(ahead_key) else None
        verdict = None
        if last_a is not None:
            _sleep_until(last_a + 2_100_000_000)
            verdict = fr.judge_observation("state", h, sa.observation(ahead_key))
        pres = live.list_presence(bus.g_session, "zk2/lab/ahead/@zk/**")
        report.check(sec, "step 5: the owner 2 s ahead reports itself ahead from the router-stamped heartbeat; "
                          "from then S receives nothing more on status (re-puts held, a put refused), judges it "
                          "stale 2 s after its last delivery, and the owner's tokens stay present",
                     oa.ahead and not after and refused == "ClockAhead" and verdict is not None
                     and verdict.verdict == fr.STALE and len(pres.instances) == 1 and len(pres.alive) == 1,
                     f"events {oa.events}; {len(after)} deliveries after; put {refused}; "
                     f"verdict {verdict.pair() if verdict else None}; tokens {len(pres.instances)}+{len(pres.alive)}")
    finally:
        bus.close()


# -- §3 ------------------------------------------------------------------------------

def section3(report: Report) -> None:
    sec = "§3 never stale"
    bus = Bus()
    try:
        s = bus.subscriber()
        o = bus.start(bus.owner())
        h = _h(o, INTENT)
        time.sleep(0.3)
        t1 = time.monotonic_ns()
        put_utc = time.time_ns()
        o.set_state(INTENT, b"i")
        time.sleep(3.0)
        got = _since(s.of(INTENT), t1)
        v_s = fr.judge_observation("state", h, s.observation(INTENT))
        report.check(sec, "step 1: S receives intent once, no re-put, and judges it fresh at 3 s: never stale",
                     [r.payload for r in got] == [b"i"] and v_s.pair() == {"verdict": "fresh", "reason": "never_stale"},
                     f"{[r.payload for r in got]}; {v_s.pair()}")
        untrusted = fr.ClockTrust(DELTA_NS)
        reading, g = bus.g(INTENT, trust=untrusted)
        v_g = fr.judge_observation("state", h, g)
        report.check(sec, "step 2: G, 3 s after the put, with no measurement and no word, judges intent fresh: "
                          "ttl 0 needs no clock",
                     reading.payload == b"i" and v_g.pair() == {"verdict": "fresh", "reason": "never_stale"}
                     and not untrusted.trusted(reading.stamp_id),
                     f"{v_g.pair()}, {(time.time_ns() - put_utc) / fr.NS:.1f} s after the put")
    finally:
        bus.close()


# -- §4 ------------------------------------------------------------------------------

def section4(report: Report) -> None:
    sec = "§4 a skewed GET reader"
    bus = Bus()
    try:
        s = bus.subscriber()
        o = bus.start(bus.owner(), clock_offset_ns=-5 * fr.NS)
        h = _h(o, STATUS)
        sid = str(o.session.zid())
        time.sleep(0.3)
        o.set_state(STATUS, b"up")
        time.sleep(2.5)
        v_s = fr.judge_observation("state", h, s.observation(STATUS))
        offs = [off for k, v in bus.trust.measurements.items() if _zid(k) == _zid(sid) for off in v]
        failed = bool(offs) and all(abs(off - 5 * fr.NS) < 300_000_000 for off in offs) \
            and not any(bus.trust.measured(k) for k in bus.trust.measurements if _zid(k) == _zid(sid))
        report.check(sec, "step 1: S judges status fresh on its receive clock; G's measurement fails, each stamp "
                          "about 5 s behind its clock at receipt",
                     v_s.verdict == fr.FRESH and failed,
                     f"S {v_s.pair()}; {len(offs)} measurements, {sorted({round(x / fr.NS, 1) for x in offs})} s")
        reading, g = bus.g(STATUS)
        v_g = fr.judge_observation("state", h, g)
        report.check(sec, "step 2: G judges the reply unobservable, its clock untrusted, although the stamp reads "
                          "about 5 s old",
                     v_g.pair() == {"verdict": "unobservable", "reason": "clock_untrusted"},
                     f"{v_g.pair()}, aged {g.reply.stamp_age_ns / fr.NS if g.reply and g.reply.stamp_age_ns else None}")
        word = fr.ClockTrust(DELTA_NS, word=True)
        v_w = fr.judge_observation("state", h, reading.observation(word))
        report.check(sec, "step 3: on the deployment's word (which the clocks do not keep), G judges the same "
                          "reply stale: the error is the deployment's",
                     v_w.verdict == fr.STALE, str(v_w.pair()))
    finally:
        bus.close()


# -- §5 ------------------------------------------------------------------------------

def section5(report: Report) -> None:
    sec = "§5 the stream case"
    bus = Bus()
    try:
        s = bus.subscriber()
        o = bus.start(bus.owner())
        h = _h(o, LEVEL)
        time.sleep(0.3)
        t1 = time.monotonic_ns()
        for i in range(10):
            o.publish(LEVEL, str(i).encode())
            time.sleep(0.2)
        t_stop = time.monotonic_ns()
        time.sleep(0.1)
        got = _since(s.of(LEVEL), t1)
        last = got[-1].arrival_ns if got else t_stop
        _sleep_until(last + 500_000_000)
        v05 = fr.judge_observation("stream", h, s.observation(LEVEL))
        _sleep_until(last + 1_500_000_000)
        v15 = fr.judge_observation("stream", h, s.observation(LEVEL))
        time.sleep(0.5)
        after = [r for r in s.of(LEVEL) if r.arrival_ns > last]
        gaps = _gaps(got)
        report.check(sec, "step 1: a sample every 200 ms, within the 0.5 s (ttl/2) bound, and nothing after the "
                          "owner stops: no stream sample is published on its behalf",
                     len(got) == 10 and max(gaps) <= 0.5 + JITTER_S and not after,
                     f"{len(got)} samples, largest gap {max(gaps, default=0):.3f} s, {len(after)} after the stop")
        report.check(sec, "step 2: fresh 0.5 s after the last sample, stale 1.5 s after it",
                     v05.verdict == fr.FRESH and v15.verdict == fr.STALE, f"{v05.pair()} then {v15.pair()}")
    finally:
        bus.close()


# -- §6 ------------------------------------------------------------------------------

def section6(report: Report) -> None:
    sec = "§6 a tool's verdict on a resource"
    bus = Bus()
    try:
        o = bus.start(bus.owner())
        time.sleep(0.3)
        o.set_state(STATUS, b"up")
        o.set_state(INTENT, b"i")
        o.set_state(NOTE, b"n")
        stop = threading.Event()

        def levels() -> None:
            i = 0
            while not stop.is_set():
                o.publish(LEVEL, str(i).encode())
                i += 1
                time.sleep(0.2)

        th = threading.Thread(target=levels, daemon=True)
        th.start()
        want1 = {"state/status": fr.FRESH, "state/intent": fr.FRESH, "state/note": fr.NOT_ASKED,
                 "stream/level": fr.FRESH}
        got1 = fr.read_service(bus.g_session, "lab/beacon", [bus.contract.canonical], 4.0, fr.ClockTrust(DELTA_NS))
        v1 = {k: v.verdict for k, (v, _) in got1.items()}
        report.check(sec, "step 1: status fresh, intent fresh (never stale), note not asked (no horizon), level "
                          "fresh", v1 == want1, str({k: (v.pair(), m) for k, (v, m) in got1.items()}))

        def on_subscribed() -> None:
            def later() -> None:
                time.sleep(0.5)
                o.close_writer(STATUS)
                stop.set()
            threading.Thread(target=later, daemon=True).start()

        got2 = fr.read_service(bus.g_session, "lab/beacon", [bus.contract.canonical], 4.0, fr.ClockTrust(DELTA_NS),
                               on_subscribed=on_subscribed)
        v2 = {k: v.verdict for k, (v, _) in got2.items()}
        want2 = {"state/status": fr.STALE, "state/intent": fr.FRESH, "state/note": fr.NOT_ASKED,
                 "stream/level": fr.STALE}
        finding = any(v.verdict == fr.STALE for v, _ in got2.values())
        report.check(sec, "step 2: status stale, level stale, intent fresh, note not asked: the run's verdict is "
                          "the finding", v2 == want2 and finding,
                     str({k: (v.pair(), m) for k, (v, m) in got2.items()}))
        stop.set()
    finally:
        bus.close()


SECTIONS = {"1": section1, "2": section2, "3": section3, "4": section4, "5": section5, "6": section6}


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="python -m zk2py.freshness_scenarios", description=__doc__.split("\n")[0])
    ap.add_argument("--only", action="append", choices=sorted(SECTIONS), help="run only these sections")
    args = ap.parse_args(argv)
    report = Report()
    for name, fn in SECTIONS.items():
        if args.only and name not in args.only:
            continue
        fn(report)
    passed = sum(1 for r in report.rows if r[2])
    failed = len(report.rows) - passed
    print(f"freshness scenarios: {passed} passed, {failed} failed")
    for sec, name, ok, detail in report.rows:
        if not ok:
            print(f"FAIL [{sec}] {name}: {detail}")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())

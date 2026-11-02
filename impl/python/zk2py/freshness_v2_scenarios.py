"""The ``freshness.v2`` scenarios (``spec/profiles/freshness/scenarios-v2.md``,
text 0.1, core 0.82; #978 F3) on in-process zenoh-python routers:
``python -m zk2py.freshness_v2_scenarios [--pairing …] [--only 1 …]``.

**The pairings** (#609, round 96), each section run in each:
- ``py-py``: zk2py's owner (:class:`zk2py.owner.Owner`) and zk2py's readers:
  H and S are :class:`zk2py.freshness_v2.Subscriber`, G is
  :func:`zk2py.freshness.get_reading`;
- ``rust-py``: the Rust owner example, a black box (``put``, ``delete``,
  ``vouch``, ``--clock-reference`` and ``--clock-offset-ms``), with zk2py's
  readers;
- ``py-zenctl``: zk2py's owner, with ``zenctl watch`` as S (its ``sample``
  and ``confirmation`` rows, each dated at its line's arrival) and H (its
  rows on ``freshness.v2``'s ``heartbeat/{iface}``), and ``zenctl get
  state`` as G (its ``last_confirmation``). zenctl judges no freshness, so
  "S judges" applies v2.md §2.5 to zenctl's rows, and "G judges" §2.6 to its
  reply's stamp.

**The bus** is the text's: R1, a router with timestamping on; the owner,
S, H and G its clients, the owner's session with timestamping off (core
§8.2). Timeouts are 1 s. Jitter: 200 ms on loopback, as the reference
allows. §4's R2 is a router linked to R1 through a TCP relay the run cuts
(:class:`zk2py.budget_scenarios.Link`); §7's R1 also listens on a
``unixsock-stream`` path, the face, runs the plan's downsampling block,
and F is its client there.

**The contract** is ``beacon.v2``, copied from the text into
``impl/python/interop/freshness/beacon.v2.toml``. Since ``freshness.v2``
0.2 it marks ``status`` and ``level`` ``link.exposure = "link"`` and lists
``link.v1`` (F-443's answer), so §7's face runs it too, and round 96's
``beacon-face.v2.toml`` is gone (round 97).

Exit 0 when every check passes, 1 when any fails, 2 when a pairing cannot
run (a black box missing).
"""

from __future__ import annotations

import argparse
import json
import os
import queue
import subprocess
import sys
import tempfile
import threading
import time
from datetime import datetime
from pathlib import Path
from typing import Any

from . import freshness as fr
from . import freshness_v2 as fv2
from . import heartbeat, zk2c

REPO = Path(__file__).resolve().parents[3]
BEACON = REPO / "impl" / "python" / "interop" / "freshness" / "beacon.v2.toml"
#: §7's contract: beacon.v2 itself since freshness.v2 0.2 (F-443's answer)
BEACON_FACE = BEACON
BLACKBOX = Path(os.environ.get("ZK2PY_BLACKBOX", "/srv/dev/scratch/zenkey/bin/current"))
IFACE = "beacon.v2"
BASE = f"zk2/lab/beacon/{IFACE}"
STATUS, PEER_A, PEER_B = f"{BASE}/state/status", f"{BASE}/state/peer/a", f"{BASE}/state/peer/b"
JITTER_S = 0.2
NS = fr.NS
PAIRINGS = ("py-py", "rust-py", "py-zenctl")


class Report:
    def __init__(self) -> None:
        self.rows: list[tuple[str, str, str, bool, str]] = []

    def check(self, pairing: str, section: str, name: str, ok: bool, detail: str = "") -> bool:
        self.rows.append((pairing, section, name, ok, detail))
        print(f"{'PASS' if ok else 'FAIL'} [{pairing}] [{section}] {name}" + (f": {detail}" if detail else ""),
              flush=True)
        return ok

    def note(self, pairing: str, section: str, text: str) -> None:
        print(f"NOTE [{pairing}] [{section}] {text}", flush=True)


def _contract(path: Path = BEACON) -> Any:
    from .contract import load_contract

    c = load_contract(path)
    assert c.valid, c.codes
    return c


def _sleep_until(mono_ns: int) -> None:
    d = (mono_ns - time.monotonic_ns()) / NS
    if d > 0:
        time.sleep(d)


# -- the owners ---------------------------------------------------------------------------

class PyOwner:
    """zk2py's owner of ``beacon.v2`` at ``lab/<service>``, a client of R1;
    it holds no capability, so ``route`` is not exposed."""

    impl = "zk2py"

    def __init__(self, ep: str, service: str = "beacon", contract: Path = BEACON, *, ahead_beat: str | None = None):
        from .owner import Owner

        self.c = _contract(contract)
        self.base = f"zk2/lab/{service}/{IFACE}"
        kw: dict[str, Any] = {}
        if ahead_beat is not None:
            kw.update(heartbeat=ahead_beat, delta_s=0.5)
        self.o = Owner("lab", service, [self.c], connect=ep, **kw)
        if ahead_beat is not None:
            from .freshness_scenarios import offset_clock

            self.o.open_session()
            self.o.clock = offset_clock(self.o, 2 * NS)
        self.o.start()
        self.instance_key = self.o.instance_key
        self.closed_at: int | None = None

    def key(self, rel: str) -> str:
        return f"{self.base}/state/{rel}"

    def put(self, rel: str, value: bytes = b"1") -> None:
        self.o.set_state(self.key(rel), value)

    def delete(self, rel: str) -> None:
        self.o.delete_state(self.key(rel))

    def vouch(self, rel: str, on: bool) -> None:
        self.o.vouch(self.key(rel), on)

    def ahead(self) -> bool:
        return self.o.ahead

    def zid(self) -> str:
        return str(self.o.session.zid())

    def close(self) -> None:
        if self.closed_at is None:
            self.o.close()
            self.closed_at = time.monotonic_ns()


class RustOwner:
    """The owner example, a black box, of ``beacon.v2`` at ``lab/<service>``:
    each put puts ``ok``. It exposes ``route``: it takes no capabilities
    argument (its usage), so §1's map reads 1 1 1 against it."""

    impl = "rust"

    def __init__(self, ep: str, service: str = "beacon", contract: Path = BEACON, *, ahead_beat: str | None = None):
        from .live_interop import Owner as Exe

        flags: list[str] = []
        if ahead_beat is not None:
            flags += ["--clock-reference", ahead_beat, "--clock-offset-ms", "2000"]
        self.base = f"zk2/lab/{service}/{IFACE}"
        self.p = Exe(BLACKBOX / "owner", f"lab/{service}", [contract], connect=ep, flags=flags)
        got = self.p.wait_for("ready ", 120)
        if got is None:
            raise RuntimeError(f"the owner example did not start: {self.p.stderr[-3:]}")
        self.instance_key = got.strip()
        self.ep = ep
        self.closed_at: int | None = None
        self._zid: str | None = None
        self._ahead = False

    def key(self, rel: str) -> str:
        return f"{self.base}/state/{rel}"

    def _resource(self, rel: str) -> str:
        if rel.startswith("peer/"):
            return f"state/peer/{{peer}} {rel.split('/', 1)[1]}"
        return f"state/{rel}"

    def command(self, line: str) -> str:
        got = self.p.command(line)
        if got is None or got.startswith("refused"):
            raise RuntimeError(f"{line!r}: {got!r}")
        return got

    def put(self, rel: str, value: bytes = b"1") -> None:
        self.command(f"put {IFACE} {self._resource(rel)}")

    def try_put(self, rel: str) -> str | None:
        return self.p.command(f"put {IFACE} {self._resource(rel)}")

    def delete(self, rel: str) -> None:
        self.command(f"delete {IFACE} {self._resource(rel)}")

    def vouch(self, rel: str, on: bool) -> None:
        self.command(f"vouch {IFACE} {self._resource(rel)} {'on' if on else 'off'}")

    def ahead(self) -> bool:
        """Whether its guard holds its writes: a put refused for it."""
        if not self._ahead:
            got = self.try_put("status") or ""
            self._ahead = got.startswith("refused")
        return self._ahead

    def zid(self) -> str:
        return self._zid or ""

    def close(self) -> None:
        if self.closed_at is None:
            self.p.close()
            self.closed_at = time.monotonic_ns()


def make_owner(impl: str, ep: str, **kw: Any) -> Any:
    return RustOwner(ep, **kw) if impl == "rust" else PyOwner(ep, **kw)


# -- zenctl as a reader ---------------------------------------------------------------------

class ZWatch:
    """``zenctl watch`` on one resource, its ndjson rows each dated at its
    line's arrival on this process's monotonic clock."""

    def __init__(self, ep: str, address: str, iface: str, resource: str, for_s: float, contract: Path = BEACON):
        exe = BLACKBOX / "zenctl"
        # The revisions it holds offline (``--contracts``), so a watch
        # declared before the owner is up resolves (zenctl refuses one with
        # no revision of the interface, exit 2).
        held = contract if iface == IFACE else fv2.CONTRACT
        self.proc = subprocess.Popen([str(exe), "watch", address, iface, resource, "--for", f"{for_s:g}",
                                      "--format", "ndjson", "-c", ep, "--contracts", str(held)],
                                     stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        self.rows: list[tuple[int, dict[str, Any]]] = []
        self.err: list[str] = []
        self._t = threading.Thread(target=self._pump, daemon=True)
        self._t.start()
        threading.Thread(target=lambda: self.err.extend(self.proc.stderr), daemon=True).start()

    def _pump(self) -> None:
        for line in self.proc.stdout:
            try:
                self.rows.append((time.monotonic_ns(), json.loads(line)))
            except ValueError:
                pass

    def wait(self, timeout: float = 60.0) -> None:
        try:
            self.proc.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            self.proc.kill()
            self.proc.wait()
        self._t.join(timeout=2.0)

    def samples(self, key: str | None = None) -> list[tuple[int, dict[str, Any]]]:
        return [(t, r) for t, r in self.rows if r.get("row") == "sample" and (key is None or r.get("key") == key)]

    def confirmations(self, key: str) -> list[int]:
        return [t for t, r in self.rows if r.get("row") == "confirmation" and key in r.get("confirmed", [])]

    def kinds(self) -> list[str]:
        return sorted({r.get("row", "?") for _, r in self.rows})


def zget(ep: str, rel: str, extra: list[str] | None = None) -> dict[str, Any] | None:
    """``zenctl get state lab/beacon beacon.v2 <rel>`` as JSON, or None."""
    exe = BLACKBOX / "zenctl"
    r = subprocess.run([str(exe), "get", "state", "lab/beacon", IFACE, rel, "--format", "json", "-c", ep,
                        "--timeout", "1", *(extra or [])],
                       capture_output=True, text=True, timeout=60)
    try:
        return json.loads(r.stdout)
    except ValueError:
        return None


def _rfc3339_ns(text: str) -> int:
    """An RFC 3339 UTC time with nanoseconds, as zenctl writes a stamp."""
    main, _, frac = text.rstrip("Z").partition(".")
    dt = datetime.strptime(main, "%Y-%m-%dT%H:%M:%S")
    from calendar import timegm

    return timegm(dt.timetuple()) * NS + int((frac + "000000000")[:9])


def zget_reading(ep: str, key: str) -> tuple[fr.GetReading, dict[str, Any] | None]:
    """G through ``zenctl get state``: the row of ``key``, its stamp the
    ``last_confirmation`` (else its ``timestamp``)."""
    rel = key.split(f"/{IFACE}/state/", 1)[1]
    doc = zget(ep, rel.split("/", 1)[0] if "/" in rel else rel,
               ["--param", f"peer={rel.split('/', 1)[1]}"] if "/" in rel else None)
    rows = [] if doc is None else [r for r in doc.get("rows", []) if r.get("key") == key]
    if not rows:
        return fr.GetReading(None), doc
    row = rows[0]
    if row.get("state") == "deleted":
        return fr.GetReading("delete"), row
    st = row.get("last_confirmation") or row.get("timestamp")
    if not st:
        return fr.GetReading("put"), row
    return fr.GetReading("put", _rfc3339_ns(st["time"]), st["clock"], f"{st['time']}/{st['clock']}"), row


# -- zk2py as a reader ----------------------------------------------------------------------

def subscriber(session: Any, follows: list[str], seed: bool = False, service: str = "beacon",
               contract: Path = BEACON) -> fv2.Subscriber:
    c = _contract(contract)
    return fv2.Subscriber(session, f"lab/{service}", c.canonical, c.fingerprint, follows=follows, seed=seed)


def _judge_rows(w: ZWatch, key: str, ttl_s: int, at: int, since: int) -> fr.Verdict:
    """v2.md §2.5 over zenctl's rows: the member's last confirmation the
    later of its last put row and its last confirmation row, at ``at``."""
    puts = [t for t, r in w.samples(key) if t <= at]
    kinds = [r.get("kind") for t, r in w.samples(key) if t <= at]
    confs = [t for t in w.confirmations(key) if t <= at]
    if kinds and kinds[-1] == "delete":
        return fr.judge_subscription(fr.Horizon("within", ttl_s),
                                     fr.Subscription(at - since, True, fr.Delivery("delete", 0)))
    last = max(puts + confs, default=None)
    return fr.judge_subscription(fr.Horizon("within", ttl_s),
                                 fr.Subscription(at - since, True,
                                                 None if last is None else fr.Delivery("put", at - last)))


def _echo(ep: str, contract: Path | None) -> subprocess.Popen:
    """zk2py's echo tool on the owner's heartbeat key for 9 s, given
    ``contract`` or nothing."""
    env = dict(os.environ, PYTHONPATH=str(REPO / "impl" / "python"))
    return subprocess.Popen([sys.executable, "-m", "zk2py.echo_tool", "-c", ep, "--for", "9",
                             *(["--contracts", str(contract)] if contract is not None else []),
                             f"zk2/lab/beacon/{fv2.PROFILE}/stream/heartbeat/*"],
                            stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, env=env)


def _echo_rows(p: subprocess.Popen) -> list[dict[str, Any]]:
    try:
        out, _ = p.communicate(timeout=60)
    except subprocess.TimeoutExpired:
        p.kill()
        out, _ = p.communicate()
    try:
        return json.loads(out)["samples"]
    except (ValueError, KeyError):
        return []


# -- the bus ---------------------------------------------------------------------------------

class Bus:
    def __init__(self) -> None:
        from . import live
        from .live_interop import _r1

        self.router, self.endpoint, self.r1_zid = _r1()
        self.sessions = [live.open_client(self.endpoint) for _ in range(3)]
        self.s_session, self.h_session, self.g_session = self.sessions
        self.closers: list[Any] = []

    def close(self) -> None:
        for x in reversed(self.closers):
            try:
                x.close()
            except Exception:  # noqa: BLE001 - closing anyway
                pass
        for s in self.sessions:
            s.close()
        self.router.close()


class TokenWatch:
    """When the owner's instance token went: a liveliness subscriber on its
    ``@zk/instance/**``."""

    def __init__(self, session: Any, service: str = "beacon"):
        import zenoh

        self.gone: int | None = None
        self.sub = session.liveliness().declare_subscriber(f"zk2/lab/{service}/@zk/instance/*",
                                                           zenoh.handlers.Callback(self._heard))

    def _heard(self, sample: Any) -> None:
        import zenoh

        if sample.kind == zenoh.SampleKind.DELETE and self.gone is None:
            self.gone = time.monotonic_ns()

    def close(self) -> None:
        try:
            self.sub.undeclare()
        except Exception:  # noqa: BLE001
            pass


def _cadence(beats: list[tuple[int, zk2c.InBand | None, int | None]]) -> tuple[bool, str]:
    """Each heartbeat at most 1 s plus jitter after the one before, its seqs
    consecutive within one run, every CH beacon.v2's tag."""
    gaps = [(b[0] - a[0]) / NS for a, b in zip(beats, beats[1:])]
    seqs_ok = all(b[1] is not None and a[1] is not None and (not a[1].same_run(b[1]) or b[1].seq == a[1].seq + 1)
                  for a, b in zip(beats, beats[1:]))
    tag = heartbeat.layout_of(_contract().canonical, _contract().fingerprint).tag
    tags = {t for _, _, t in beats}
    ok = len(beats) >= 2 and max(gaps, default=0) <= 1.0 + JITTER_S and seqs_ok and tags <= {tag}
    return ok, f"{len(beats)} heartbeats, largest gap {max(gaps, default=0):.3f} s, seqs consecutive {seqs_ok}, " \
               f"tags {sorted(t for t in tags if t is not None)} (beacon.v2's {tag})"


def _h_beats(h: fv2.Subscriber, since: int = 0) -> list[tuple[int, zk2c.InBand | None, int | None]]:
    return [(x.arrival_ns, x.stamp, x.tag) for x in h.answers(since)]


def _z_beats(w: ZWatch, since: int = 0) -> list[tuple[int, zk2c.InBand | None, int | None]]:
    tag = heartbeat.layout_of(_contract().canonical, _contract().fingerprint).tag
    out = []
    for t, r in w.samples():
        if t < since:
            continue
        ib = r.get("inband")
        st = None if not ib else zk2c.InBand(len(ib["run"]) * 4, int(ib["run"], 16), ib["seq"])
        # zenctl shows no CH: its declared reading names the covered revision
        out.append((t, st, tag if "beacon.v2" in json.dumps(r.get("conformance", {})) else None))
    return out


# -- §1 ---------------------------------------------------------------------------------------

def section1(report: Report, pairing: str) -> None:
    from . import live

    sec = "§1 the cadence, and the bits"
    owner_impl, reader = pairing.split("-")
    bus = Bus()
    try:
        tw = TokenWatch(bus.g_session)
        bus.closers.append(tw)
        if reader == "py":
            h = subscriber(bus.h_session, [])
            s = subscriber(bus.s_session, ["state/status", "state/peer/{peer}"])
            bus.closers += [h, s]
            # observer §7.3 (0.20): zk2py's echo tool on the heartbeat key,
            # given beacon.v2 and given nothing.
            echoes = [_echo(bus.endpoint, BEACON), _echo(bus.endpoint, None)]
            time.sleep(2.0)
        else:
            zh = ZWatch(bus.endpoint, "lab/beacon", fv2.PROFILE, "heartbeat/{iface}", 14)
            zs = ZWatch(bus.endpoint, "lab/beacon", IFACE, "state/status", 14)
            zp = ZWatch(bus.endpoint, "lab/beacon", IFACE, "state/peer/{peer}", 14)
            time.sleep(1.0)
        o = make_owner(owner_impl if owner_impl == "rust" else "py", bus.endpoint)
        bus.closers.append(o)
        d = live.get_descriptor(bus.g_session, o.instance_key)
        doc = json.loads(d[0].payload) if d and d[0].ok else {}
        ifaces = sorted(i["iface"] for i in doc.get("interfaces", []))
        report.check(pairing, sec, "the descriptor lists beacon.v2 and freshness.v2",
                     {"beacon.v2", "freshness.v2"} <= set(ifaces), f"interfaces {ifaces}")
        time.sleep(0.3)
        t1 = time.monotonic_ns()
        o.put("status", b"up")
        o.put("peer/a", b"1")
        o.put("peer/b", b"1")
        time.sleep(5.0)
        t1_end = time.monotonic_ns()
        if reader == "py":
            beats = _h_beats(h, t1)
            ok, detail = _cadence(beats)
            report.check(pairing, sec, "step 1: H hears a heartbeat at least every 1 s plus jitter, each form 6 "
                                       "under beacon.v2's tag, its seqs consecutive within one run", ok, detail)
            maps = {tuple(sorted(x.value["sections"])) for x in h.answers(t1) if x.value}
            want_map = {("state/peer/{peer}", "state/status")} if owner_impl == "py" else \
                {("state/peer/{peer}", "state/route", "state/status")}
            report.check(pairing, sec, "step 1: each body's map is 1 0 1, route not exposed" if owner_impl == "py"
                         else "step 1: each body's map is 1 1 1: the owner example exposes route (it takes no "
                              "capability argument), the map its exposure's", maps == want_map, f"maps {maps}")
            sm = {k: [x for x in s.samples if x[1] == k] for k in (STATUS, PEER_A, PEER_B)}
            last = h.answers(t1)[-1].value if h.answers(t1) else None
            stamp = {k: (v[-1][3] if v else None) for k, v in sm.items()}
            want_status = None if stamp[STATUS] is None else \
                f"{heartbeat.digest([(STATUS, stamp[STATUS].width, stamp[STATUS].run, stamp[STATUS].seq)]):08x}"
            want_peer = None if None in (stamp[PEER_A], stamp[PEER_B]) else f"{heartbeat.digest([(k, stamp[k].width, stamp[k].run, stamp[k].seq) for k in (PEER_A, PEER_B)]):08x}"
            got_s = None if last is None else last["sections"].get("state/status")
            got_p = None if last is None else last["sections"].get("state/peer/{peer}")
            report.check(pairing, sec, "step 1: status's section vouches for it, its digest status's stamp's; "
                                       "peer/{peer}'s holds 2 live, both vouched for, its digest over both stamps",
                         got_s == {"digest": want_status, "vouched": True}
                         and got_p is not None and got_p["live"] == 2 and got_p["vouched"] == [0, 1]
                         and got_p["digest"] == want_peer, f"status {got_s} (want {want_status}), peer {got_p} "
                                                           f"(want {want_peer})")
            puts = {k: [x for x in v if x[0] >= t1] for k, v in sm.items()}
            report.check(pairing, sec, "step 1: no re-put: S hears each put once, nothing else on status or peer/* "
                                       "in 5 s",
                         all(len(v) == 1 for v in puts.values()), f"{ {k.rsplit('/', 2)[-2] + '/' + k.rsplit('/', 1)[-1]: len(v) for k, v in puts.items()} }")
            v = s.judge(STATUS, t1_end)
            report.check(pairing, sec, "step 1: S judges status fresh 5 s after its put, beyond its horizon: the "
                                       "heartbeats confirmed it", v.verdict == fr.FRESH, f"{v.pair()}")
            held, bare = (_echo_rows(e) for e in echoes)
            hb_h = [r.get("heartbeat") or {} for r in held]
            hb_b = [r.get("heartbeat") or {} for r in bare]
            report.check(pairing, sec, "observer §7.3: zk2py's echo tool never reads a heartbeat's body as a record: "
                                       "given beacon.v2 it shows what each vouches for; given nothing, its form, "
                                       "the interface it covers and its size; never sniffed",
                         bool(held) and bool(bare) and all(r.get("sniffed") is None for r in held + bare)
                         and all(isinstance(x.get("vouches"), dict) and "state/status" in x["vouches"]["sections"]
                                 for x in hb_h)
                         and all(x.get("form") == "heartbeat" and x.get("covers") == IFACE and x.get("bytes")
                                 and x.get("vouches") is None for x in hb_b),
                         f"{len(held)} and {len(bare)} rows; {hb_h[:1]} {hb_b[:1]}")
        else:
            beats = _z_beats(zh, t1)
            ok, detail = _cadence(beats)
            report.check(pairing, sec, "step 1: zenctl watch hears a heartbeat at least every 1 s plus jitter, its "
                                       "seqs consecutive within one run, each read as beacon.v2's (zenctl shows "
                                       "no CH)", ok, detail)
            report.note(pairing, sec, f"zenctl watch on heartbeat/{{iface}} reads each sample "
                                      f"{zh.samples()[0][1].get('conformance') if zh.samples() else None}")
            puts = {k: [x for x in w.samples(k) if x[0] >= t1] for k, w in ((STATUS, zs), (PEER_A, zp), (PEER_B, zp))}
            report.check(pairing, sec, "step 1: no re-put: zenctl watch shows each put once, nothing else on status "
                                       "or peer/* in 5 s", all(len(v) == 1 for v in puts.values()),
                         f"{ {k.rsplit('/', 1)[-1]: len(v) for k, v in puts.items()} }")
            confs = {k: [t for t in w.confirmations(k) if t >= t1] for k, w in
                     ((STATUS, zs), (PEER_A, zp), (PEER_B, zp))}
            report.check(pairing, sec, "step 1: its confirmation rows confirm status, peer/a and peer/b at each "
                                       "heartbeat (the bits and the digests, as zenctl reads them)",
                         all(len(v) >= 4 for v in confs.values()), f"{ {k.rsplit('/', 1)[-1]: len(v) for k, v in confs.items()} }")
            v = _judge_rows(zs, STATUS, 2, t1_end, t1)
            report.check(pairing, sec, "step 1: status judged fresh 5 s after its put from zenctl's rows",
                         v.verdict == fr.FRESH, f"{v.pair()}")
        # Step 2.
        o.vouch("peer/b", False)
        t2 = time.monotonic_ns()
        time.sleep(2.0)
        if reader == "py":
            got = [x.value["sections"].get("state/peer/{peer}") for x in h.answers(t2 + 1_200_000_000) if x.value]
            report.check(pairing, sec, "step 2: the next heartbeats vouch for rank 0, peer/a, and not for peer/b, "
                                       "still live", bool(got) and all(g and g["live"] == 2 and g["vouched"] == [0]
                                                                       for g in got), f"{got}")
        else:
            a = [t for t in zp.confirmations(PEER_A) if t >= t2 + 1_200_000_000]
            b = [t for t in zp.confirmations(PEER_B) if t >= t2 + 1_200_000_000]
            report.check(pairing, sec, "step 2: zenctl's next confirmation rows confirm peer/a, not peer/b",
                         bool(a) and not b, f"peer/a {len(a)}, peer/b {len(b)}")
        # Step 3.
        o.delete("peer/a")
        t3 = time.monotonic_ns()
        time.sleep(2.0)
        if reader == "py":
            got = [x.value["sections"].get("state/peer/{peer}") for x in h.answers(t3 + 1_200_000_000) if x.value]
            sb = [x for x in s.samples if x[1] == PEER_B][-1][3]
            want = f"{heartbeat.digest([(PEER_B, sb.width, sb.run, sb.seq)]):08x}"
            report.check(pairing, sec, "step 3: the next heartbeats hold 1 live member, peer/b, the digest its "
                                       "stamp's alone", bool(got) and all(g and g["live"] == 1 and g["digest"] == want
                                                                         for g in got), f"{got}, want {want}")
        else:
            dels = [r for t, r in zp.samples(PEER_A) if t >= t3]
            b = [t for t in zp.confirmations(PEER_B) if t >= t3]
            report.check(pairing, sec, "step 3: zenctl watch shows peer/a's delete, and confirms neither peer since "
                                       "(peer/b's bit is clear)",
                         [r.get("kind") for r in dels] == ["delete"] and not b, f"{[r.get('kind') for r in dels]}, "
                                                                                f"peer/b {len(b)}")
        # Step 4.
        o.close()
        time.sleep(2.0)
        gone = tw.gone
        if reader == "py":
            after = [] if gone is None else _h_beats(h, gone)
        else:
            zh.wait()
            zs.wait()
            zp.wait()
            after = [] if gone is None else _z_beats(zh, gone)
        report.check(pairing, sec, "step 4: no heartbeat after the owner's tokens went",
                     gone is not None and not after, f"token gone {gone is not None}, {len(after)} after")
    finally:
        bus.close()


# -- §2 ---------------------------------------------------------------------------------------

def section2(report: Report, pairing: str) -> None:
    sec = "§2 a vouch withdrawn goes stale"
    owner_impl, reader = pairing.split("-")
    impl = "rust" if owner_impl == "rust" else "py"
    bus = Bus()
    try:
        if reader == "py":
            s = subscriber(bus.s_session, ["state/status"])
            bus.closers.append(s)
        else:
            zs = ZWatch(bus.endpoint, "lab/beacon", IFACE, "state/status", 40)
            time.sleep(1.0)
        since = time.monotonic_ns()
        o = make_owner(impl, bus.endpoint)
        bus.closers.append(o)
        time.sleep(0.3)

        def judge(at: int) -> fr.Verdict:
            return s.judge(STATUS, at) if reader == "py" else _judge_rows(zs, STATUS, 2, at, since)

        def confs(lo: int, hi: int) -> list[int]:
            if reader == "py":
                return [x.arrival_ns for x in s.answers(lo) if x.arrival_ns <= hi and isinstance(
                    x.answer.get("read", {}).get("sections", {}).get("state/status"), dict)
                    and STATUS in x.answer["read"]["sections"]["state/status"]["confirmed"]]
            return [t for t in zs.confirmations(STATUS) if lo <= t <= hi]

        # Step 1.
        t1 = time.monotonic_ns()
        o.put("status", b"up")
        _sleep_until(t1 + 3 * NS)
        v1 = judge(time.monotonic_ns())
        report.check(pairing, sec, "step 1: S judges status fresh at 3 s, past its horizon", v1.verdict == fr.FRESH,
                     f"{v1.pair()}")
        # Step 2.
        o.vouch("status", False)
        t2 = time.monotonic_ns()
        v_just = judge(t2 + 50_000_000)
        time.sleep(4.0)
        last = max(confs(t1, t2 + 1_000_000_000), default=None)
        v_after = None if last is None else judge(last + 2_100_000_000)
        report.check(pairing, sec, "step 2: fresh just after the bit cleared, never stale at once; stale more than "
                                   "2 s after the last heartbeat that vouched",
                     v_just.verdict == fr.FRESH and v_after is not None and v_after.verdict == fr.STALE,
                     f"{v_just.pair()}, then {None if v_after is None else v_after.pair()}")
        # Step 3.
        o.vouch("status", True)
        t3 = time.monotonic_ns()
        time.sleep(2.0)
        again = confs(t3, time.monotonic_ns())
        v3 = None if not again else judge(again[0] + 100_000_000)
        report.check(pairing, sec, "step 3: fresh again from the next heartbeat",
                     v3 is not None and v3.verdict == fr.FRESH and again[0] - t3 <= int((1.0 + JITTER_S) * NS),
                     f"first confirmation {(again[0] - t3) / NS if again else None} s after; {None if v3 is None else v3.pair()}")
        # Step 4: a second owner, its clock 2 s ahead.
        beat = "zk2py/freshness-v2/clock-reference"
        ahead_key = "zk2/lab/ahead/beacon.v2/state/status"
        if reader == "py":
            s2 = subscriber(bus.s_session, ["state/status"], service="ahead")
            h2 = subscriber(bus.h_session, [], service="ahead")
            bus.closers += [s2, h2]
        else:
            zs2 = ZWatch(bus.endpoint, "lab/ahead", IFACE, "state/status", 14)
            zh2 = ZWatch(bus.endpoint, "lab/ahead", fv2.PROFILE, "heartbeat/{iface}", 14)
            time.sleep(1.0)
        since2 = time.monotonic_ns()
        oa = make_owner(impl, bus.endpoint, service="ahead", ahead_beat=beat)
        bus.closers.append(oa)
        time.sleep(0.3)
        try:
            oa.put("status", b"up")
        except Exception:  # noqa: BLE001 - the guard may hold already
            pass
        deadline = time.monotonic() + 8.0
        while time.monotonic() < deadline and not oa.ahead():
            bus.g_session.put(beat, b"beat")  # no timestamp: R1 stamps it
            time.sleep(0.2)
        held = time.monotonic_ns()
        stop = threading.Event()

        def beats() -> None:
            while not stop.wait(0.3):
                bus.g_session.put(beat, b"beat")

        threading.Thread(target=beats, daemon=True).start()
        time.sleep(4.0)
        stop.set()
        if reader == "py":
            vals = [x.value for x in h2.answers(held + 1_100_000_000) if x.value is not None]
            clear = bool(vals) and all(x["time"] is None and all(
                (not sec_["vouched"]) if isinstance(sec_["vouched"], bool) else not sec_["vouched"]
                for sec_ in x["sections"].values()) for x in vals)
            report.check(pairing, sec, "step 4: H2 still hears heartbeats while the guard holds, each bit clear and "
                                       "the time unknown, all 42 bits set",
                         oa.ahead() and clear, f"guard {oa.ahead()}, {len(vals)} heartbeats: "
                                               f"{vals[-1] if vals else None}")
            v4 = s2.judge(ahead_key, held + 2_100_000_000)
            report.check(pairing, sec, "step 4: S2 judges status stale more than 2 s after the guard held",
                         v4.verdict == fr.STALE, f"{v4.pair()}")
        else:
            zh2.wait()
            zs2.wait()
            n = len(_z_beats(zh2, held + 1_100_000_000))
            conf = [t for t in zs2.confirmations(ahead_key) if t >= held + 1_100_000_000]
            report.check(pairing, sec, "step 4: zenctl watch still hears heartbeats while the guard holds, and no "
                                       "confirmation row (zenctl shows no body)", oa.ahead() and n >= 2 and not conf,
                         f"guard {oa.ahead()}, {n} heartbeats, {len(conf)} confirmations")
            v4 = _judge_rows(zs2, ahead_key, 2, held + 2_100_000_000, since2)
            report.check(pairing, sec, "step 4: status of lab/ahead judged stale more than 2 s after the guard held, "
                                       "from zenctl's rows", v4.verdict == fr.STALE, f"{v4.pair()}")
        # Step 5.
        o.close()
        t5 = time.monotonic_ns()
        time.sleep(3.0)
        last = max(confs(t1, time.monotonic_ns()), default=None)
        v5 = None if last is None else judge(last + 2_100_000_000)
        report.check(pairing, sec, "step 5: a silent heartbeat: stale more than 2 s after the last; never reported "
                                   "down (a freshness verdict, no presence read)",
                     v5 is not None and v5.verdict == fr.STALE and last is not None and last <= t5,
                     f"{None if v5 is None else v5.pair()}")
        if reader != "py":
            zs.wait()
    finally:
        bus.close()


# -- §3 ---------------------------------------------------------------------------------------

def section3(report: Report, pairing: str) -> None:
    from .live import _zid_value

    sec = "§3 a GET reader's age"
    owner_impl, reader = pairing.split("-")
    impl = "rust" if owner_impl == "rust" else "py"
    bus = Bus()
    trust = fr.ClockTrust(fr.DEFAULT_DELTA_NS, word=True)
    h = fr.Horizon("within", 2)
    try:
        if reader == "py":
            s = subscriber(bus.s_session, ["state/status"])
            bus.closers.append(s)
        else:
            zs = ZWatch(bus.endpoint, "lab/beacon", IFACE, "state/status", 14)
            time.sleep(1.0)
        o = make_owner(impl, bus.endpoint)
        bus.closers.append(o)
        if impl == "rust":
            from . import live

            d = live.get_descriptor(bus.g_session, o.instance_key)
            o._zid = json.loads(d[0].payload).get("meta", {}).get("zid") if d and d[0].ok else None
        time.sleep(0.3)
        t_put_utc = time.time_ns()
        o.put("status", b"up")
        time.sleep(0.5)
        time.sleep(2.5)

        def g() -> tuple[fr.GetReading, Any]:
            if reader == "py":
                return fr.get_reading(bus.g_session, STATUS), None
            return zget_reading(bus.endpoint, STATUS)

        r1, row1 = g()
        now = time.time_ns()
        obs = r1.observation(trust, now)
        v1 = fr.judge_get(h, obs)
        age = None if r1.stamp_ns is None else (now - r1.stamp_ns) / NS
        report.check(pairing, sec, "step 1: G's reply carries a Zenoh timestamp of the owner's zid, later than the "
                                   "put and within 1 s plus jitter of G's clock; G judges status fresh 3 s after "
                                   "its put",
                     r1.stamp_id is not None and _zid_value(r1.stamp_id) == _zid_value(o.zid())
                     and r1.stamp_ns is not None and r1.stamp_ns > t_put_utc and age is not None
                     and age <= 1.0 + JITTER_S and v1.verdict == fr.FRESH,
                     f"stamp {r1.stamp} (owner zid {o.zid()}), aged {age} s, {v1.pair()}")
        # The put reaches S stamped by R1's clock, if at all.
        if reader == "py":
            ids = {x[4].split("/", 1)[1] if x[4] else None for x in s.samples if x[1] == STATUS}
        else:
            rows = [r for _, r in zs.samples(STATUS)]
            ids = {(r.get("timestamp") or {}).get("clock") for r in rows}
        if True:
            report.check(pairing, sec, "the put reaches S stamped by R1's clock, if at all: the owner set no Zenoh "
                                       "timestamp (core §4.3's table)",
                         all(i is None or _zid_value(i) == _zid_value(bus.r1_zid) for i in ids),
                         f"clocks {ids}, R1 {bus.r1_zid}")
        # Step 2.
        o.vouch("status", False)
        t2 = time.monotonic_ns()
        last_before = r1.stamp_ns
        time.sleep(3.5)
        r2, _ = g()
        now = time.time_ns()
        v2 = fr.judge_get(h, r2.observation(trust, now))
        report.check(pairing, sec, "step 2: the reply's stamp is the last confirmation before the bit cleared; G "
                                   "judges status stale, its age above ttl + δ",
                     r2.stamp_ns is not None and last_before is not None and r2.stamp_ns >= last_before
                     and (now - r2.stamp_ns) / NS > 2.5 and v2.verdict == fr.STALE,
                     f"stamp {r2.stamp}, aged {None if r2.stamp_ns is None else (now - r2.stamp_ns) / NS} s, "
                     f"{v2.pair()}")
        if reader != "py":
            zs.wait()
    finally:
        bus.close()


# -- §4 ---------------------------------------------------------------------------------------

def section4(report: Report, pairing: str) -> None:
    import zenoh

    from . import live
    from .budget_scenarios import Link
    from .owner import free_loopback_port

    sec = "§4 a lost change, detected and re-read"
    owner_impl, reader = pairing.split("-")
    impl = "rust" if owner_impl == "rust" else "py"
    bus = Bus()
    try:
        link = Link(bus.endpoint)
        bus.closers.append(link)
        port = free_loopback_port()
        conf = zenoh.Config()
        conf.insert_json5("mode", json.dumps("router"))
        conf.insert_json5("listen/endpoints", json.dumps([f"tcp/127.0.0.1:{port}"]))
        conf.insert_json5("connect/endpoints", json.dumps([link.endpoint]))
        conf.insert_json5("scouting/multicast/enabled", "false")
        r2 = zenoh.open(conf)
        bus.closers.append(r2)
        r2_ep = f"tcp/127.0.0.1:{port}"
        o = make_owner(impl, bus.endpoint)
        bus.closers.append(o)
        o.put("peer/a", b"1")
        o.put("peer/b", b"1")
        time.sleep(0.3)
        if reader == "py":
            ss = live.open_client(r2_ep)
            bus.closers.append(ss)
            s = subscriber(ss, ["state/peer/{peer}"], seed=True)
            bus.closers.append(s)
        else:
            zp = ZWatch(r2_ep, "lab/beacon", IFACE, "state/peer/{peer}", 18)
        # zenctl seeds by no GET of its own: its first heartbeats diverge
        time.sleep(2.0 if reader == "py" else 3.5)
        if reader == "py":
            ok1 = any(isinstance(x.answer.get("read", {}).get("sections", {}).get("state/peer/{peer}"), dict)
                      and len(x.answer["read"]["sections"]["state/peer/{peer}"]["confirmed"]) == 2
                      for x in s.answers())
        else:
            ok1 = bool(zp.confirmations(PEER_A)) and bool(zp.confirmations(PEER_B))
        report.check(pairing, sec, "step 1: S's heartbeats confirm both members" + (" (S seeds by its own GET)"
                                                                                   if reader == "py" else ""), ok1)
        # Step 2.
        link.cut()
        time.sleep(1.5)
        o.put("peer/a", b"2")
        time.sleep(1.5)
        link.heal()
        healed = time.monotonic_ns()
        time.sleep(6.0)
        if reader == "py":
            after = s.answers(healed)
            first = after[0] if after else None
            gap = first is not None and first.answer.get("read", {}).get("gap") is True
            v_gap = None if first is None else s.judge(PEER_A, first.arrival_ns + 1)
            report.check(pairing, sec, "expected 1: the first heartbeat after the heal follows a gap in its key's "
                                       "seqs", gap, f"{None if first is None else first.answer.get('read', {}).get('gap')}")
            report.note(pairing, sec, f"peer/a judged at that heartbeat: {None if v_gap is None else v_gap.pair()}")
            div = [x.answer["read"]["sections"]["state/peer/{peer}"] for x in after if "read" in x.answer]
            reads = [r for r in s.reads if r[1] >= healed]
            report.check(pairing, sec, "expected 2: no heartbeat confirms peer/{peer} until S re-reads: it diverges",
                         len(div) >= 2 and div[0] == "diverged" and div[1] == "diverged", f"{div[:4]}")
            report.check(pairing, sec, "expected 3: after two in a row diverge, S re-reads once; the answer holds "
                                       "peer/a's new stamp, and the next heartbeat confirms both",
                         len(reads) == 1 and any(isinstance(d, dict) and len(d["confirmed"]) == 2 for d in div[2:]),
                         f"reads {len(reads)}, then {div[2:5]}")
        else:
            zp.wait()
            puts_a = [r for t, r in zp.samples(PEER_A)]
            confs = [t for t in zp.confirmations(PEER_A) if t >= healed]
            report.check(pairing, sec, "zenctl watch never heard peer/a = 2 (nor its first put, before it "
                                       "subscribed), and confirms peer/a again after the heal: its re-read, as its "
                                       "rows show it",
                         not puts_a and bool(confs), f"{len(puts_a)} put rows, {len(confs)} confirmations "
                                                     f"after the heal; rows {zp.kinds()}")
            report.note(pairing, sec, f"zenctl's row kinds: {zp.kinds()}")
    finally:
        bus.close()


# -- §5 ---------------------------------------------------------------------------------------

def section5(report: Report, pairing: str) -> None:
    sec = "§5 a late joiner by S4"
    owner_impl, reader = pairing.split("-")
    impl = "rust" if owner_impl == "rust" else "py"
    bus = Bus()
    try:
        o = make_owner(impl, bus.endpoint)
        bus.closers.append(o)
        t_put = time.monotonic_ns()
        o.put("peer/a", b"1")
        o.put("peer/b", b"1")
        time.sleep(1.0)
        if reader == "py":
            s = subscriber(bus.s_session, ["state/peer/{peer}"], seed=True)
            sp = subscriber(bus.h_session, ["state/peer/{peer}"], seed=False)
            bus.closers += [s, sp]
        else:
            zp = ZWatch(bus.endpoint, "lab/beacon", IFACE, "state/peer/{peer}", 6)
            g = zget(bus.endpoint, "peer/{peer}")
            report.note(pairing, sec, f"zenctl get state answered {len((g or {}).get('rows', []))} rows")
        time.sleep(5.0)
        if reader == "py":
            first = s.answers()[0] if s.answers() else None
            fsec = None if first is None else first.answer.get("read", {}).get("sections", {}).get("state/peer/{peer}")
            v = [s.judge(k, t_put + 6 * NS).verdict for k in (PEER_A, PEER_B)]
            report.check(pairing, sec, "S confirms both members at its first heartbeat, with no re-read, and judges "
                                       "both fresh 6 s after their puts",
                         isinstance(fsec, dict) and len(fsec["confirmed"]) == 2 and not s.reads and v == [fr.FRESH] * 2,
                         f"first {fsec}, reads {len(s.reads)}, verdicts {v}")
            secs = [x.answer.get("read", {}).get("sections", {}).get("state/peer/{peer}") for x in sp.answers()]
            report.check(pairing, sec, "S′ holds no member, so its first heartbeats diverge; it re-reads once, and "
                                       "its next heartbeat confirms both",
                         secs[:2] == ["diverged", "diverged"] and len(sp.reads) == 1
                         and any(isinstance(d, dict) and len(d["confirmed"]) == 2 for d in secs[2:]),
                         f"{secs[:4]}, reads {len(sp.reads)}")
        else:
            zp.wait()
            a, b = zp.confirmations(PEER_A), zp.confirmations(PEER_B)
            report.check(pairing, sec, "zenctl watch, a late joiner, confirms both members from its heartbeats",
                         bool(a) and bool(b), f"peer/a {len(a)}, peer/b {len(b)} confirmations; rows {zp.kinds()}")
    finally:
        bus.close()


# -- §6 ---------------------------------------------------------------------------------------

def section6(report: Report, pairing: str) -> None:
    sec = "§6 a restart, a new run"
    owner_impl, reader = pairing.split("-")
    impl = "rust" if owner_impl == "rust" else "py"
    bus = Bus()
    try:
        if reader == "py":
            s = subscriber(bus.s_session, ["state/peer/{peer}"])
            bus.closers.append(s)
        else:
            zp = ZWatch(bus.endpoint, "lab/beacon", IFACE, "state/peer/{peer}", 14)
            time.sleep(1.0)
        o = make_owner(impl, bus.endpoint)
        bus.closers.append(o)
        o.put("peer/a", b"1")
        o.put("peer/b", b"1")
        time.sleep(2.0)
        o.close()
        o2 = make_owner(impl, bus.endpoint)
        bus.closers.append(o2)
        restarted = time.monotonic_ns()
        o2.put("peer/a", b"1")
        time.sleep(6.0)
        if reader == "py":
            after = s.answers(restarted)
            gaps = [x.answer.get("read", {}).get("gap") for x in after if "read" in x.answer]
            secs = [x.answer["read"]["sections"]["state/peer/{peer}"] for x in after if "read" in x.answer]
            report.check(pairing, sec, "no gap: the new instance's heartbeats are of another run", not any(gaps),
                         f"gaps {gaps[:4]}")
            report.check(pairing, sec, "the leftover: S's view holds peer/b of the old run, so its heartbeats "
                                       "diverge; the re-read replaces the view, peer/b leaves it, and the next "
                                       "heartbeat confirms peer/a alone",
                         "diverged" in secs and any(isinstance(d, dict) and d["confirmed"] == [PEER_A] for d in secs)
                         and PEER_B not in s.view.members and len([r for r in s.reads if r[1] >= restarted]) == 1,
                         f"{secs[:5]}, reads {len(s.reads)}, view {sorted(s.view.members)}")
            last_b = s.heard[PEER_B].last_confirmation if PEER_B in s.heard else None
            v = None if last_b is None else s.judge(PEER_B, last_b + 4_100_000_000)
            later_b = [x for x in after if isinstance(x.answer.get("read", {}).get("sections", {}).get(
                "state/peer/{peer}"), dict) and PEER_B in x.answer["read"]["sections"]["state/peer/{peer}"]["confirmed"]]
            report.check(pairing, sec, "no heartbeat confirms peer/b again, and S judges it stale more than 4 s after "
                                       "its last confirmation",
                         not later_b and v is not None and v.verdict == fr.STALE, f"{None if v is None else v.pair()}")
        else:
            zp.wait()
            b = [t for t in zp.confirmations(PEER_B) if t >= restarted]
            a = [t for t in zp.confirmations(PEER_A) if t >= restarted]
            report.check(pairing, sec, "zenctl watch confirms peer/a after the restart, and never peer/b again",
                         bool(a) and not b, f"peer/a {len(a)}, peer/b {len(b)}; rows {zp.kinds()}")
    finally:
        bus.close()


# -- §7 ---------------------------------------------------------------------------------------

FACE_POLICY = """system   = "lab"
services = ["beacon"]

[face.rf]
far          = "ground"
attach       = "client"
select       = { link_protocols = ["unixsock-stream"] }
class        = "constrained"
expose       = "link"
interval_s   = 2
max_outage_s = 60
batch_bytes  = 1000
link_bps     = 2400
"""


def section7(report: Report, pairing: str) -> None:
    import zenoh

    from . import link, live
    from .contract import load_contract
    from .owner import free_loopback_port

    sec = "§7 across a face"
    owner_impl, reader = pairing.split("-")
    impl = "rust" if owner_impl == "rust" else "py"
    face_c = load_contract(BEACON_FACE)
    std = fv2.standard_contract()
    plan = link.plan(FACE_POLICY, {"beacon": [IFACE, fv2.PROFILE]},
                     {IFACE: {"revisions": [link.revision_of(face_c)]},
                      fv2.PROFILE: {"revisions": [link.revision_of(std)]}})
    face = plan["faces"][0]
    rules = [r["key_expr"] for r in (face["downsampling"] or {}).get("rules", [])]
    hb_key = heartbeat.heartbeat_key("lab", "beacon", IFACE)
    denied = [k for d in face["deny"] for k in d["key_exprs"]
              if zenoh.KeyExpr(k).intersects(zenoh.KeyExpr(hb_key))]
    ev = [e for e in face["liveness"] if e["service"] == "lab/beacon"]
    report.check(pairing, sec, "the plan: one downsampling rule, level's, none for the heartbeat key; no deny "
                               "reaches it; lab/beacon's evidence lists freshness.v2/stream/heartbeat/beacon.v2",
                 rules == [f"{BASE}/stream/level"] and not denied
                 and ev and "freshness.v2/stream/heartbeat/beacon.v2" in ev[0]["evidence"],
                 f"rules {rules}, denies reaching it {denied}, evidence {ev}")
    workdir = tempfile.mkdtemp(prefix="zk2py-fv2-face-")
    sock = os.path.join(workdir, "face.sock")
    port = free_loopback_port()
    near = f"tcp/127.0.0.1:{port}"
    conf = zenoh.Config()
    conf.insert_json5("mode", json.dumps("router"))
    conf.insert_json5("listen/endpoints", json.dumps([near, f"unixsock-stream/{sock}"]))
    conf.insert_json5("scouting/multicast/enabled", "false")
    conf.insert_json5("timestamping/enabled", "true")
    for k, v in link.router_extras(face).items():
        conf.insert_json5(k, json.dumps(v))
    r1 = zenoh.open(conf)
    closers: list[Any] = []
    try:
        far_ep = f"unixsock-stream/{sock}"
        if reader == "py":
            fs = live.open_client(far_ep)
            closers.append(fs)
            f_hb = fv2.Subscriber(fs, "lab/beacon", face_c.canonical, face_c.fingerprint, follows=["state/status"])
            levels: list[int] = []
            closers.append(f_hb)
            closers.append(fs.declare_subscriber(f"{BASE}/stream/level",
                                                 zenoh.handlers.Callback(lambda s: levels.append(time.monotonic_ns()))))
        else:
            zh = ZWatch(far_ep, "lab/beacon", fv2.PROFILE, "heartbeat/{iface}", 7)
            zl = ZWatch(far_ep, "lab/beacon", IFACE, "stream/level", 7, contract=BEACON_FACE)
            time.sleep(1.0)
        o = make_owner(impl, near, contract=BEACON_FACE)
        closers.append(o)
        o.put("status", b"up")
        stop = threading.Event()

        def levels_loop() -> None:
            n = 0
            while not stop.wait(0.2):
                n += 1
                if impl == "py":
                    o.o.publish(f"{BASE}/stream/level", str(n).encode())
                else:
                    o.p.command(f"put {IFACE} stream/level")

        threading.Thread(target=levels_loop, daemon=True).start()
        t0 = time.monotonic_ns()
        time.sleep(5.0)
        end = time.monotonic_ns()
        stop.set()
        if reader == "py":
            beats = _h_beats(f_hb, t0)
            lv = [t for t in levels if t >= t0]
            last_hb = beats[-1][0] if beats else None
        else:
            zh.wait()
            zl.wait()
            beats = _z_beats(zh, t0)
            lv = [t for t, _ in zl.samples() if t >= t0]
            last_hb = beats[-1][0] if beats else None
        gaps = [(b[0] - a[0]) / NS for a, b in zip(beats, beats[1:])]
        lgaps = [(b - a) / NS for a, b in zip(lv, lv[1:])]
        report.check(pairing, sec, "no downsampling: F hears a heartbeat at least every 1 s plus jitter",
                     len(beats) >= 4 and max(gaps, default=9) <= 1.0 + JITTER_S,
                     f"{len(beats)} heartbeats, largest gap {max(gaps, default=0):.3f} s")
        report.check(pairing, sec, "level at most once every 2 s (the face's downsampling)",
                     len(lv) >= 1 and min(lgaps, default=2.0) >= 2.0 - JITTER_S,
                     f"{len(lv)} level samples, smallest gap {min(lgaps, default=0):.3f} s")
        hb_age = None if last_hb is None else (end - last_hb) / NS
        report.check(pairing, sec, "liveness: F judges the heartbeat key fresh at the end, by its horizon of 2 s, so "
                                   "lab/beacon live across the face",
                     hb_age is not None and hb_age <= 2.0, f"last heartbeat {hb_age} s before the end")
    finally:
        for c in reversed(closers):
            try:
                c.close()
            except Exception:  # noqa: BLE001
                pass
        r1.close()


SECTIONS = {1: section1, 2: section2, 3: section3, 4: section4, 5: section5, 6: section6, 7: section7}


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="python -m zk2py.freshness_v2_scenarios")
    ap.add_argument("--pairing", action="append", choices=PAIRINGS)
    ap.add_argument("--only", type=int, action="append", choices=sorted(SECTIONS))
    args = ap.parse_args(argv)
    pairings = args.pairing or list(PAIRINGS)
    if any(p != "py-py" for p in pairings):
        need = ["owner"] if "rust-py" in pairings else []
        need += ["zenctl"] if "py-zenctl" in pairings else []
        missing = [n for n in need if not (BLACKBOX / n).is_file()]
        if missing:
            print(f"error: the black boxes {missing} are not in {BLACKBOX}", file=sys.stderr)
            return 2
    report = Report()
    for p in pairings:
        for n in args.only or sorted(SECTIONS):
            try:
                SECTIONS[n](report, p)
            except Exception as e:  # noqa: BLE001 - a section that crashes is a failure, named
                import traceback

                traceback.print_exc()
                report.check(p, f"§{n}", "the section ran", False, f"{type(e).__name__}: {e}")
    failed = [r for r in report.rows if not r[3]]
    print(f"\n{len(report.rows) - len(failed)} of {len(report.rows)} checks pass")
    by = {}
    for p, s, _, ok, _ in report.rows:
        by.setdefault((p, s.split(" ")[0]), []).append(ok)
    for (p, s), oks in sorted(by.items()):
        print(f"  {p:10s} {s:4s} {sum(oks)}/{len(oks)}")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())

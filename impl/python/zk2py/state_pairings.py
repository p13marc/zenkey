"""``spec/scenarios/state.md`` at core 0.73 (#976), live, in each pairing
(#609, round 93): ``python -m zk2py.state_pairings [--pairing P]… [--only N]…``.

**The pairings.**
- ``py-py``: zk2py's owner (:class:`zk2py.owner.Owner`), zk2py's readers: a
  raw subscriber for the wire, S4's GET (:func:`zk2py.live.get_state`), and a
  consumer's and a tool's late joiner (:class:`zk2py.seed.LateJoiner`);
- ``rust-py``: the Rust owner example (a black box, ``--run-file``,
  ``seq <n>``, ``--standby``, ``--timing``), zk2py's readers;
- ``py-rust``: zk2py's owner, the Rust readers (black boxes): ``zenctl
  watch`` (the runtime's consumer, one ndjson row per sample with its
  ``inband`` run and seq), ``zenctl get state`` (S4's GET, each row's
  ``inband``), ``zenctl get <key> --raw`` (a reply's bytes), ``zenctl echo
  --fmt`` (each sample's Zenoh timestamp), and ``consume`` (one state GET,
  its reply's Zenoh timestamp).

**The contract** is zk2py's ``interop/state/zk2py_ground.v1.toml``: the
common setup's ``plans/{vehicle}`` (no ``freshness.ttl_s``, off core §4.3's
table), §1's ``status`` (``freshness.ttl_s`` 60, on it), §13's streams, and
an operation for ``consume``. §13 step 3's governed interface is
``interop/state/zk2py_relay.v1.toml``, an external election, so that one
candidate steps down and takes over again as its lock goes and comes back;
the reference's templated stream member is ``pace.v1``'s ``axis/{axis}``,
removed and added by ``timing remove|add``.

**Sections.** §1, §2, §7 (steps 1 to 6), §10 and §13 are here; §3, §11 and
§12 are :mod:`zk2py.state_scenarios`', which runs them for ``py-py`` and, with
zenctl's readers, ``py-rust``. §4, §5, §6 and §9 need an archive, and §8 a
union storage: zk2py runs neither, a zenoh-python router loads no plugin,
and no black box is one. Each section that a pairing cannot run says why
(:data:`NOT_RUNNABLE`).

Every listener is on a port the OS picked (port 0). Exit 0 when every check
passes, 1 when any fails.
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path
from typing import Any

from . import seed as sd
from . import zk2c

REPO = Path(__file__).resolve().parents[3]
GROUND = REPO / "impl" / "python" / "interop" / "state" / "zk2py_ground.v1.toml"
RELAY = REPO / "impl" / "python" / "interop" / "state" / "zk2py_relay.v1.toml"
PACE = REPO / "impl" / "python" / "interop" / "timing" / "pace.v1.toml"
IFACE = "zk2py_ground.v1"
BLACKBOX = Path(os.environ.get("ZK2PY_BLACKBOX", "/srv/dev/scratch/zenkey/bin/current"))
PAIRINGS = ("py-py", "rust-py", "py-rust")

#: Each section a pairing cannot run, and why.
NOT_RUNNABLE = {
    "4": "an archive (core §4.4): zk2py runs none, and no black box is one",
    "5": "an archive's alignment (core §4.4): zk2py runs none, and no black box is one",
    "6": "an archive's alignment (core §4.4): zk2py runs none, and no black box is one",
    "8": "a union storage on zk2/*/*/*/events/**: a zenoh-python router loads no plugin",
    "9": "an archive's backend (core §4.4): zk2py runs none, and no black box is one",
}


class Report:
    def __init__(self) -> None:
        self.rows: list[tuple[str, str, bool, str]] = []
        self.sections: dict[tuple[str, str], str] = {}

    def check(self, section: str, name: str, ok: bool, detail: str = "") -> bool:
        self.rows.append((section, name, ok, detail))
        print(f"{'PASS' if ok else 'FAIL'} [{section}] {name}" + (f": {detail}" if detail else ""), flush=True)
        return ok

    def note(self, section: str, text: str) -> None:
        print(f"NOTE [{section}] {text}", flush=True)

    def outcome(self, pairing: str, number: str, word: str) -> None:
        self.sections[(pairing, number)] = word
        print(f"SECTION {pairing} §{number}: {word}", flush=True)


def router(timestamping: bool) -> tuple[Any, str, str]:
    """A router of the runner's own (state.md §1's R1 or R2)."""
    from .live_interop import _r1

    return _r1(timestamping=timestamping)


class Tap:
    """A raw subscriber that records every sample as it arrived: key, kind,
    payload, attachment, Encoding and Zenoh timestamp.

    Its callbacks are direct (``indirect=False``), run on zenoh's own thread
    as each sample arrives: zenoh-python's default runs each subscriber's
    on a thread of its own, so two selectors' samples, a descriptor's and
    a state put's, could land in ``got`` out of their arrival order (§7
    step 3's order, the mend after round 93)."""

    def __init__(self, session: Any, selectors: list[str]) -> None:
        import zenoh

        self.lock = threading.Lock()
        self.got: list[dict[str, Any]] = []
        self.subs = [session.declare_subscriber(s, zenoh.handlers.Callback(self._take, indirect=False))
                     for s in selectors]

    def _take(self, s: Any) -> None:
        import zenoh

        ts = s.timestamp
        row = {"key": str(s.key_expr), "delete": s.kind == zenoh.SampleKind.DELETE,
               "payload": s.payload.to_bytes(), "encoding": str(s.encoding),
               "attachment": None if s.attachment is None else s.attachment.to_bytes(),
               "ts": None if ts is None else str(ts), "read": zk2c.read_sample(s)}
        with self.lock:
            self.got.append(row)

    def of(self, suffix: str) -> list[dict[str, Any]]:
        with self.lock:
            return [r for r in self.got if r["key"].endswith(suffix)]

    def close(self) -> None:
        for s in self.subs:
            try:
                s.undeclare()
            except Exception:  # noqa: BLE001 - closing anyway
                pass


def _zid_of(ts: str | None) -> str | None:
    return None if ts is None else ts.split("/", 1)[1].lower().lstrip("0")


def _time_of(ts: str | None) -> int | None:
    return None if ts is None else int(ts.split("/", 1)[0])


def _same_zid(a: str | None, b: str | None) -> bool:
    return a is not None and b is not None and a.lower().lstrip("0") == b.lower().lstrip("0")


# -- the owners -------------------------------------------------------------------------------

class PyOwner:
    """zk2py's owner of ``zk2py_ground.v1`` at ``ground/fleet-mgr``, a client
    of ``ep``."""

    impl = "zk2py"

    def __init__(self, ep: str, *, record: str | None = None, hlc: bool | None = None,
                 offset_s: float = 0.0, service: str = "fleet-mgr", contracts: list[Path] | None = None,
                 **kw: Any) -> None:
        import zenoh

        from .contract import load_contract
        from .owner import Owner

        self.contracts = [load_contract(p) for p in (contracts or [GROUND])]
        self.o = Owner("ground", service, self.contracts, connect=ep, run_record=record, hlc=hlc, **kw)
        if offset_s:
            def shifted(o: Any = self.o) -> Any:
                ts = o.session.new_timestamp()
                n = ts.get_time_as_ntp64().as_nanos() + int(offset_s * 1e9)
                return zenoh.Timestamp(zenoh.NTP64(n // 1_000_000_000, n % 1_000_000_000), ts.get_id())

            self.o.clock = shifted
        self.o.start()
        self.base = f"zk2/ground/{service}/{IFACE}"
        self.address = f"ground/{service}"

    @property
    def instance_key(self) -> str:
        return self.o.instance_key

    def put(self, member: str, value: bytes = b"ok") -> Any:
        return self.o.set_state(f"{self.base}/state/plans/{member}", value)

    def delete(self, member: str) -> Any:
        return self.o.delete_state(f"{self.base}/state/plans/{member}")

    def status(self, value: bytes = b"up") -> Any:
        return self.o.set_state(f"{self.base}/state/status", value)

    def seq(self, n: int) -> None:
        self.o.set_next_seq(n)

    def zid(self) -> str:
        return str(self.o.session.zid())

    def close(self) -> None:
        self.o.close()


class RustOwner:
    """The Rust owner example (a black box) of ``zk2py_ground.v1`` at
    ``ground/fleet-mgr``, a client of ``ep``. Its writes put ``ok``."""

    impl = "rust"

    def __init__(self, ep: str, *, record: str | None = None, hlc: bool | None = None,
                 offset_s: float = 0.0, service: str = "fleet-mgr", contracts: list[Path] | None = None,
                 flags: list[str] | None = None) -> None:
        from .live_interop import Owner as Exe

        f = list(flags or [])
        if record is not None:
            f += ["--run-file", record]
        if hlc:
            f += ["--hlc"]
        if offset_s:
            f += ["--clock-offset-ms", str(int(offset_s * 1000))]
        self.p = Exe(BLACKBOX / "owner", f"ground/{service}", list(contracts or [GROUND]), connect=ep, flags=f)
        got = self.p.wait_for("ready ", 120)
        if got is None:
            raise RuntimeError(f"the owner example did not start: {self.p.stderr[-3:]}")
        self.instance_key = got.strip()
        self.base = f"zk2/ground/{service}/{IFACE}"
        self.address = f"ground/{service}"
        self._zid: str | None = None

    def command(self, line: str) -> str:
        got = self.p.command(line)
        if got is None or got.startswith("refused"):
            raise RuntimeError(f"{line!r}: {got!r}")
        return got

    def put(self, member: str, value: bytes = b"ok") -> Any:
        return self.command(f"put {IFACE} state/plans/{{vehicle}} {member}")

    def delete(self, member: str) -> Any:
        return self.command(f"delete {IFACE} state/plans/{{vehicle}} {member}")

    def status(self, value: bytes = b"up") -> Any:
        return self.command(f"put {IFACE} state/status")

    def seq(self, n: int) -> None:
        self.command(f"seq {n}")

    def zid(self) -> str:
        return self._zid or ""

    def close(self) -> None:
        self.p.close()


def make_owner(impl: str, ep: str, **kw: Any) -> Any:
    o = PyOwner(ep, **kw) if impl == "zk2py" else RustOwner(ep, **kw)
    return o


def descriptor(session: Any, owner: Any) -> dict[str, Any]:
    from . import live

    end = time.monotonic() + 10.0
    while True:
        for a in live.get_descriptor(session, owner.instance_key, timeout=2.0):
            if a.ok:
                doc = json.loads(a.payload)
                if isinstance(owner, RustOwner):
                    owner._zid = (doc.get("meta") or {}).get("zid")
                return doc
        if time.monotonic() > end:
            return {}
        time.sleep(0.2)


# -- the Rust readers -------------------------------------------------------------------------

def zenctl(*args: str, timeout: float = 60.0) -> subprocess.CompletedProcess:
    return subprocess.run([str(BLACKBOX / "zenctl"), *args], capture_output=True, text=True, timeout=timeout)


class Watch:
    """``zenctl watch`` of one resource, ndjson, for ``for_s`` seconds: the
    runtime's consumer, each sample's row with its in-band stamp."""

    def __init__(self, ep: str, address: str, iface: str, resource: str, for_s: float,
                 contracts: Path = GROUND) -> None:
        self.p = subprocess.Popen([str(BLACKBOX / "zenctl"), "watch", "-c", ep, "--format", "ndjson",
                                   "--contracts", str(contracts), "--for", str(for_s), address, iface, resource],
                                  stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)

    def rows(self, timeout: float = 60.0) -> list[dict[str, Any]]:
        out, _ = self.p.communicate(timeout=timeout)
        return [json.loads(x) for x in out.splitlines() if x.strip().startswith("{")]


class Echo:
    """``zenctl echo`` of a wire selector, ndjson: each sample's key and
    Zenoh timestamp (its ``timestamp``, absent when it carries none)."""

    def __init__(self, ep: str, selector: str, for_s: float, count: int = 0) -> None:
        self.p = subprocess.Popen([str(BLACKBOX / "zenctl"), "echo", "-c", ep, "--format", "ndjson",
                                   "--contracts", str(GROUND), "--count", str(count), selector],
                                  stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        self.for_s = for_s

    def rows(self) -> list[tuple[str, str]]:
        time.sleep(0)
        try:
            out, _ = self.p.communicate(timeout=self.for_s)
        except subprocess.TimeoutExpired:
            self.p.terminate()
            out, _ = self.p.communicate(timeout=10)
        rows = [json.loads(x) for x in out.splitlines() if x.strip().startswith("{")]
        return [(x.get("key", ""), x.get("timestamp") or "-") for x in rows]


def consume_state(ep: str, address: str, resource: str) -> tuple[int, list[str]]:
    p = subprocess.run([str(BLACKBOX / "consume"), ep, address, str(GROUND), resource, "@op/poke"],
                       capture_output=True, text=True, timeout=60)
    return p.returncode, p.stdout.splitlines()


# -- §1 -------------------------------------------------------------------------------------

def section1(report: Report, pairing: str) -> None:
    """§1 In-band stamps (S1, S2, §4.3): through R1 (timestamping off) and R2
    (on)."""
    import zenoh

    from . import live

    impl = "rust" if pairing == "rust-py" else "zk2py"
    for ts_on, rname in ((False, "R1"), (True, "R2")):
        sec = f"{pairing} §1 through {rname} (timestamping {'on' if ts_on else 'off'})"
        r, ep, rzid = router(ts_on)
        tool = live.open_client(ep)
        third = live.open_client(ep)
        owner = None
        tap = echo = watch = None
        try:
            if pairing == "py-rust":
                echo = Echo(ep, "zk2/ground/**", 20.0, count=7)
                watch = Watch(ep, "ground/fleet-mgr", IFACE, "state/plans/{vehicle}", 10.0)
            tap = Tap(tool, ["zk2/ground/fleet-mgr/**", "control/s1"])
            time.sleep(2.0 if pairing == "py-rust" else 0.3)
            owner = make_owner(impl, ep)
            doc = descriptor(tool, owner)
            run = doc.get("run")
            # step 1
            owner.put("a", b"v1")
            owner.put("a", b"v2")
            time.sleep(0.3)
            st = live.get_state(tool, f"{owner.base}/state/plans/a", timeout=5.0)
            raw_get = None
            if pairing == "py-rust":
                p = zenctl("get", "-c", ep, "--raw", "--format", "json", "--no-decode", f"{owner.base}/state/plans/a")
                raw_get = p.stdout
            owner.delete("a")
            # step 2: the control
            third.put("control/s1", b"c")
            # step 3
            owner.put("c", b"v3")
            owner.put("c", b"v4")
            # step 4
            owner.status(b"s1")
            owner.status(b"s2")
            time.sleep(0.5)
            sst = live.get_state(tool, f"{owner.base}/state/status", timeout=5.0)
            cons: list[str] = []
            if pairing == "py-rust":
                _, cons = consume_state(ep, owner.address, "state/status")
            time.sleep(0.3)
            zid = owner.zid() or (doc.get("meta") or {}).get("zid")
            a = tap.of("/state/plans/a")
            cc = tap.of("/state/plans/c")
            stat = tap.of("/state/status")
            ctl = tap.of("control/s1")
            if pairing == "py-rust":
                rows = watch.rows()
                wa = [x for x in rows if x.get("row") == "sample" and x["key"].endswith("/plans/a")]
                wc = [x for x in rows if x.get("row") == "sample" and x["key"].endswith("/plans/c")]
                report.check(sec, "expected 1 (zenctl watch): every put an envelope whose run is the descriptor's, "
                                  "the delete one of the same run, each seq above the one before",
                             [x["kind"] for x in wa] == ["put", "put", "delete"]
                             and all(x.get("inband", {}).get("run") == run for x in wa + wc)
                             and [x["inband"]["seq"] for x in wa + wc] == sorted({x["inband"]["seq"] for x in wa + wc})
                             and len(wa + wc) == 5,
                             f"run {run}; {[(x['kind'], x.get('inband')) for x in wa + wc]}")
                want = owner.o._held[f"{owner.base}/state/plans/c"].wire  # noqa: SLF001 - the owner's own bytes
                v2 = [r_ for r_ in tap.of("/state/plans/a") if not r_["delete"]]
                report.check(sec, "expected 2 (zenctl get --raw): the GET after v2 returns v2's bytes, its envelope "
                                  "included",
                             bool(v2) and v2[-1]["payload"].hex() in (raw_get or "").replace(" ", ""),
                             f"want {v2[-1]['payload'].hex() if v2 else None} in {raw_get!r}"[:400])
                er = echo.rows()
                plan_ts = [t for k, t in er if "/state/plans/" in k]
                stat_ts = [t for k, t in er if k.endswith("/state/status")]
                report.check(sec, "expected 3 (zenctl echo %T): the plans' puts carry "
                                  + ("R2's zid" if ts_on else "no Zenoh timestamp"),
                             bool(plan_ts) and (all(rzid.lower().lstrip("0") in t.lower() for t in plan_ts[:2])
                                                if ts_on else all(t.strip() in ("", "-") for t in plan_ts)),
                             f"router {rzid}; {plan_ts}")
                report.check(sec, "expected 4 (zenctl watch): v4's seq is v3's plus 1",
                             len(wc) == 2 and wc[1]["inband"]["seq"] == wc[0]["inband"]["seq"] + 1,
                             str([x.get("inband") for x in wc]))
                cl = [x for x in cons if x.startswith("state ")]
                report.check(sec, "expected 5 (consume, zenctl echo): status's reply and puts carry a Zenoh "
                                  "timestamp whose id is the owner session's zid",
                             len(cl) == 1 and _same_zid(cl[0].split()[2].split("/", 1)[1], zid)
                             and len(stat_ts) >= 2 and all(_same_zid(t.split("/", 1)[1] if "/" in t else None, zid)
                                                           for t in stat_ts[-2:]),
                             f"zid {zid}; consume {cl}; echo {stat_ts}")
                continue
            reads = [x["read"] for x in a + cc]
            report.check(sec, "expected 1: every put an envelope of the mutation form whose run is the "
                              "descriptor's run; the delete's attachment a delete envelope of the same run; each "
                              "seq above the one before",
                         len(a) == 3 and [x["delete"] for x in a] == [False, False, True] and len(cc) == 2
                         and all(x["read"].enveloped and not x["read"].refused and x["read"].stamp is not None
                                 and x["read"].stamp.run_hex() == run for x in a + cc)
                         and all(x["encoding"] == "zenoh/bytes" for x in a + cc)
                         and [r_.stamp.seq for r_ in reads] == sorted({r_.stamp.seq for r_ in reads}),
                         f"run {run}; {[(x['delete'], str(x['read'].stamp), x['read'].refusal) for x in a + cc]}")
            rep = [x for x in st.replies if x.key.endswith("/plans/a")]
            v2 = [x for x in a if not x["delete"]][-1:] if a else []
            report.check(sec, "expected 2: the GET after v2 returns v2's bytes, its envelope included",
                         len(rep) == 1 and bool(v2) and rep[0].raw == v2[0]["payload"]
                         and rep[0].encoding == "zenoh/bytes",
                         f"{[(x.raw.hex(), x.encoding) for x in rep]} against {[x['payload'].hex() for x in v2]}")
            if ts_on:
                ok3 = (all(_same_zid(_zid_of(x["ts"]), rzid) for x in a + cc if not x["delete"])
                       and all(x["ts"] is None for x in a + cc if x["delete"])
                       and all(x.stamp is None for x in rep)
                       and len(ctl) == 1 and _same_zid(_zid_of(ctl[0]["ts"]), rzid))
            else:
                ok3 = (all(x["ts"] is None for x in a + cc) and all(x.stamp is None for x in rep)
                       and len(ctl) == 1 and ctl[0]["ts"] is None)
            report.check(sec, "expected 3: " + ("each put with R2's zid, its delete and its reply with none; the "
                                                "control's put with R2's zid" if ts_on else
                                                "no put, delete or reply of the plans carries a Zenoh timestamp; "
                                                "nor does the control's put"),
                         ok3, f"router {rzid}; puts {[x['ts'] for x in a + cc]}; reply {[x.stamp for x in rep]}; "
                              f"control {[x['ts'] for x in ctl]}")
            report.check(sec, "expected 4: v4's seq is v3's plus 1",
                         len(cc) == 2 and cc[1]["read"].stamp.seq == cc[0]["read"].stamp.seq + 1,
                         str([str(x["read"].stamp) for x in cc]))
            srep = [x for x in sst.replies if x.key.endswith("/state/status")]
            two = stat[-2:]
            report.check(sec, "expected 5: each put and the reply of status carry a Zenoh timestamp whose id is the "
                              "owner session's zid, the second put's above the first's",
                         len(two) == 2 and all(_same_zid(_zid_of(x["ts"]), zid) for x in two)
                         and _time_of(two[1]["ts"]) > _time_of(two[0]["ts"])
                         and len(srep) == 1 and _same_zid(srep[0].stamp_id, zid) and srep[0].inband is not None,
                         f"zid {zid}; puts {[x['ts'] for x in two]}; reply {[(x.stamp, str(x.inband)) for x in srep]}")
        except Exception as e:  # noqa: BLE001 - a step that could not run is a failure
            report.check(sec, "the section ran", False, f"{type(e).__name__}: {e}")
        finally:
            for x in (tap,):
                if x is not None:
                    x.close()
            for p in (echo, watch):
                if p is not None and p.p.poll() is None:
                    p.p.kill()
            if owner is not None:
                owner.close()
            third.close()
            tool.close()
            r.close()
    report.outcome(pairing, "1", "run")


# -- §2 -------------------------------------------------------------------------------------

def section2(report: Report, pairing: str) -> None:
    """§2 Deletes inside the window, and the collection (S2, S3)."""
    from . import live

    impl = "rust" if pairing == "rust-py" else "zk2py"
    sec = f"{pairing} §2"
    r, ep, _ = router(False)
    tool = live.open_client(ep)
    owner = None
    try:
        tap = Tap(tool, ["zk2/ground/fleet-mgr/**"])
        time.sleep(0.3)
        owner = make_owner(impl, ep)
        owner.put("a", b"va")
        owner.put("b", b"vb")
        owner.delete("b")
        time.sleep(0.4)
        dele = [x for x in tap.of("/plans/b") if x["delete"]]
        puta = [x for x in tap.of("/plans/a") if not x["delete"]]
        if pairing == "py-rust":
            p = zenctl("get", "state", "-c", ep, "--format", "json", "--contracts", str(GROUND), owner.address, IFACE,
                       "plans/{vehicle}")
            rows = {x["key"].rsplit("/", 1)[1]: x for x in json.loads(p.stdout or "{}").get("rows", [])}
            d = dele[0]["read"].stamp if dele else None
            report.check(sec, "zenctl get state, the selector plans/*: a with the bytes the owner put, and b a "
                              "deletion whose in-band stamp is its delete envelope's",
                         p.returncode == 0 and "a" in rows and rows["a"].get("state") != "deleted"
                         and rows.get("b", {}).get("state") == "deleted" and d is not None
                         and rows["b"].get("inband") == {"run": d.run_hex(), "seq": d.seq},
                         f"{p.returncode} {rows}; the delete's {d}")
            code, lines = consume_state(ep, owner.address, "state/plans/{vehicle}")
            report.note(sec, f"consume over plans/{{vehicle}} prints one reply: {lines}")
        else:
            one = live.get_state(tool, f"{owner.base}/state/plans/b", timeout=5.0)
            report.check(sec, "for plans/b: a reply_del whose attachment is the delete's envelope",
                         len(one.replies) == 1 and one.replies[0].deleted and bool(dele)
                         and one.replies[0].attachment == dele[0]["attachment"]
                         and one.replies[0].inband is not None,
                         f"{[(x.deleted, x.attachment and x.attachment.hex()) for x in one.replies]}; the delete's "
                         f"{dele[0]['attachment'].hex() if dele else None}")
            coll = live.get_state(tool, f"{owner.base}/state/plans/*", timeout=5.0)
            got = {x.key.rsplit("/", 1)[1]: x for x in coll.replies}
            report.check(sec, "for plans/*: plans/a with the bytes the owner put, and a reply_del for plans/b",
                         set(got) == {"a", "b"} and not got["a"].deleted and bool(puta)
                         and got["a"].raw == puta[-1]["payload"] and got["b"].deleted,
                         f"{[(k, x.deleted, x.raw.hex()) for k, x in got.items()]}")
        tap.close()
    except Exception as e:  # noqa: BLE001
        report.check(sec, "the section ran", False, f"{type(e).__name__}: {e}")
    finally:
        if owner is not None:
            owner.close()
        tool.close()
        r.close()
    report.outcome(pairing, "2", "run")


# -- §7 -------------------------------------------------------------------------------------

def _wait(cond: Any, limit: float = 5.0) -> bool:
    end = time.monotonic() + limit
    while time.monotonic() < end:
        if cond():
            return True
        time.sleep(0.05)
    return bool(cond())


def section7(report: Report, pairing: str) -> None:
    """§7 Runs (S7): steps 1 to 6."""
    from . import live

    impl = "rust" if pairing == "rust-py" else "zk2py"
    key = "r"
    # Steps 1 to 3, one owner restarting, each run beside a consumer.
    for step in ("1", "2", "2-spent", "3"):
        sec = f"{pairing} §7 step {step}"
        r, ep, _ = router(False)
        tool = live.open_client(ep)
        owners: list[Any] = []
        lc = None
        watch = None
        tmp = tempfile.mkdtemp(prefix="zk2py-s7-")
        rec = os.path.join(tmp, "run")
        try:
            if step == "2-spent":
                if impl == "rust":
                    Path(rec).write_bytes(b"\xff\xff")
                else:
                    Path(rec).write_text(json.dumps({"run": 0xFFFF}))
            record = None if step == "2" else rec
            base = "zk2/ground/fleet-mgr/zk2py_ground.v1/state/plans"
            tap = Tap(tool, [f"{base}/*", "zk2/ground/fleet-mgr/@zk/instance/*"])
            if pairing == "py-rust":
                watch = Watch(ep, "ground/fleet-mgr", IFACE, "state/plans/{vehicle}", 9.0)
                time.sleep(2.0)
            else:
                lc = sd.LateJoiner(tool, f"{base}/*", consumer=True, timeout=5.0)
                lc.start()
                lc.wait()
            o1 = make_owner(impl, ep, record=record)
            owners.append(o1)
            d1 = descriptor(tool, o1)
            if step == "3":
                o1.seq(16_777_214)
                for i in range(3):
                    o1.put(key, f"rev{i}".encode())
                time.sleep(0.5)
                d2 = descriptor(tool, o1)
                got = [x["read"].stamp for x in tap.of(f"/plans/{key}")]
                report.check(sec, "the first two at seqs 16,777,214 and 16,777,215 of run n, the third under a new "
                                  "run at seq 0, which the descriptor states",
                             len(got) == 3 and all(g is not None for g in got)
                             and [g.seq for g in got] == [16_777_214, 16_777_215, 0]
                             and got[0].same_run(got[1]) and not got[2].same_run(got[1])
                             and d1.get("run") == got[0].run_hex() and d2.get("run") == got[2].run_hex(),
                             f"{[str(g) for g in got]}; descriptor {d1.get('run')} then {d2.get('run')}")
                with tap.lock:
                    order = [("descriptor", json.loads(x["payload"]).get("run")) if "/@zk/instance/" in x["key"]
                             else ("put", str(x["read"].stamp)) for x in tap.got]
                new = got[2].run_hex() if len(got) == 3 and got[2] is not None else None
                at_desc = next((i for i, x in enumerate(order) if x == ("descriptor", new)), None)
                at_third = next((i for i, x in enumerate(order) if x[0] == "put" and new and new in x[1]), None)
                report.check(sec, "the owner puts its descriptor stating the new run before it puts the third",
                             at_desc is not None and at_third is not None and at_desc < at_third, str(order))
                # core 0.74 (F-415): "Each put of it (below) carries a Zenoh
                # timestamp the owner sets" (§3.3), the first and the wrap's.
                with tap.lock:
                    stamps = [x["ts"] for x in tap.got if "/@zk/instance/" in x["key"]]
                report.check(sec, "each descriptor put carries a Zenoh timestamp the owner sets (core §3.3), its "
                                  "session's zid (the first, and the one stating the new run)",
                             len(stamps) >= 2 and all(_same_zid(_zid_of(t), o1.zid()) for t in stamps),
                             f"{stamps}; zid {o1.zid()}")
                o1.close()
                owners.remove(o1)
            else:
                o1.put(key, b"rev10")
                time.sleep(0.3)
                o1.close()
                owners.remove(o1)
                kw: dict[str, Any] = {"record": record, "offset_s": -5.0}
                o2 = make_owner(impl, ep, **kw)
                owners.append(o2)
                d2 = descriptor(tool, o2)
                o2.put(key, b"rev11")
                time.sleep(0.5)
                got = [x for x in tap.of(f"/plans/{key}")]
                s10, s11 = (got[0]["read"].stamp, got[1]["read"].stamp) if len(got) == 2 else (None, None)
                if step == "1":
                    on_disk = Path(rec).read_bytes()
                    want = (s10.run + 1) if s10 is not None else None
                    report.check(sec, "rev 11 comes under run n + 1, which the record holds and the new descriptor "
                                      "states",
                                 s10 is not None and s11 is not None and s10.width == s11.width == 16
                                 and s11.run == want and d2.get("run") == s11.run_hex()
                                 and (on_disk == want.to_bytes(2, "big") if impl == "rust"
                                      else json.loads(on_disk).get("run") == want),
                                 f"rev 10 {s10}, rev 11 {s11}; descriptor {d2.get('run')}; record {on_disk!r}")
                else:
                    raws = [x["payload"][0] for x in got]
                    report.check(sec, "each start draws a 32-bit run: CH 0x10, the descriptor's run 8 hex digits, "
                                      "the two starts' runs different",
                                 len(got) == 2 and raws == [0x10, 0x10] and s10.width == s11.width == 32
                                 and len(d1.get("run", "")) == 8 and len(d2.get("run", "")) == 8
                                 and not s10.same_run(s11),
                                 f"CH {[hex(x) for x in raws]}; {s10}, {s11}; descriptors {d1.get('run')}, "
                                 f"{d2.get('run')}")
                    if step == "2-spent":
                        left = Path(rec).read_bytes()
                        report.check(sec, "a record at 0xFFFF is spent: the owner draws a 32-bit run, and leaves "
                                          "the record as it is",
                                     left == (b"\xff\xff" if impl == "rust" else json.dumps({"run": 0xFFFF}).encode()),
                                     repr(left))
            if pairing == "py-rust":
                rows = [x for x in watch.rows() if x.get("row") == "sample" and x["key"].endswith(f"/plans/{key}")]
                report.check(sec, "zenctl watch, a consumer, delivers every value, the later run's included",
                             len(rows) in (2, 3) and all("inband" in x for x in rows),
                             str([x.get("inband") for x in rows]))
            else:
                v = lc.view().get(f"{base}/{key}")
                last = [x for x in lc.applied if x.key.endswith(f"/plans/{key}")]
                want = b"rev2" if step == "3" else b"rev11"
                report.check(sec, "the consumer accepts the last write as another run's, and orders by seq within "
                                  "it from then on",
                             v is not None and (v.payload == want or impl == "rust")
                             and all(x.outcome == "kept" for x in last) and len(last) == (3 if step == "3" else 2),
                             f"{_show(v)}; {[(str(x.inband), x.outcome) for x in last]}")
            tap.close()
        except Exception as e:  # noqa: BLE001
            report.check(sec, "the step ran", False, f"{type(e).__name__}: {e}")
        finally:
            if lc is not None:
                lc.close()
            if watch is not None and watch.p.poll() is None:
                watch.p.kill()
            for o in owners:
                o.close()
            tool.close()
            r.close()
    if pairing == "py-rust":
        report.note(f"{pairing} §7", "steps 4 and 5 need a consumer and a tool that are late joiners; neither "
                                    "black box seeds (zenctl watch subscribes, zenctl get state and consume GET)")
    else:
        section7_late(report, pairing, impl)
    section7_ahead(report, pairing, impl)
    report.outcome(pairing, "7", "run" if pairing != "py-rust" else "steps 1 to 3 and 6 run; 4 and 5 not runnable")


def _show(e: Any) -> str:
    if e is None:
        return "missing"
    return f"{'deleted' if e.deleted else e.payload!r} at {e.mark} via {e.via}"


def section7_late(report: Report, pairing: str, impl: str) -> None:
    """§7 steps 4 and 5: late joiners across a restart, after the seed and
    inside it."""
    import zenoh

    from . import live
    from .checks import HeldQueries

    base = "zk2/ground/fleet-mgr/zk2py_ground.v1/state/plans"
    for inside in (False, True):
        sec = f"{pairing} §7 step {'5, inside the seed' if inside else '4, across a restart'}"
        r, ep, _ = router(False)
        tool, helper = live.open_client(ep), live.open_client(ep)
        owners: list[Any] = []
        joiners: list[sd.LateJoiner] = []
        held: HeldQueries | None = None
        q = None
        try:
            o1 = make_owner(impl, ep)
            owners.append(o1)
            o1.put("r", b"rev10")
            time.sleep(0.3)
            if inside:
                held = HeldQueries()
                q = helper.declare_queryable(f"{base}/*", zenoh.handlers.Callback(held.take), complete=False)
                time.sleep(0.3)
            lc = sd.LateJoiner(tool, f"{base}/*", consumer=True, timeout=10.0)
            lt = sd.LateJoiner(tool, f"{base}/*", consumer=False, timeout=10.0)
            joiners += [lc, lt]
            for j in joiners:
                j.start()
            if not inside:
                ok = all(j.wait() for j in joiners) and all(
                    (j.view().get(f"{base}/r") is not None) for j in joiners)
                report.check(sec, "both seeded rev 10", ok, str([_show(j.view().get(f"{base}/r")) for j in joiners]))
            else:
                report.check(sec, "both GETs open on the second queryable",
                             _wait(lambda: len(held.queries) >= 2, 3.0) and not any(j.seed.ended for j in joiners),
                             f"held {len(held.queries)}")
                time.sleep(0.4)  # the owner's replies on their way
            o1.close()
            owners.remove(o1)
            o2 = make_owner(impl, ep)
            owners.append(o2)
            o2.put("r", b"rev11")
            time.sleep(0.5)
            if inside:
                first = [[(a.via, str(a.inband)) for a in j.applied] for j in joiners]
                report.check(sec, "rev 11 arrives live inside both seeds, before any reply (Latest delivers at the "
                                  "GET's end, core §4.1)",
                             all(len(f) == 1 and f[0][0] == "live" for f in first), str(first))
                held.release()
                ended = all(j.wait(12.0) for j in joiners)
                c = lc.view().get(f"{base}/r")
                t = lt.view().get(f"{base}/r")
                lcs = [(a.via, a.outcome) for a in lc.applied]
                lts = [(a.via, a.outcome) for a in lt.applied]
                report.check(sec, "the consumer accepts rev 11, then the reply, which carries the old run: it holds "
                                  "rev 10",
                             ended and lcs == [("live", "kept"), ("reply", "kept")] and c is not None
                             and (c.payload == b"rev10" or impl == "rust") and c.via == "reply",
                             f"{_show(c)}; {lcs}")
                report.check(sec, "the tool keeps the later arrival too: it holds rev 10, the reply that arrived last",
                             lts == [("live", "kept"), ("reply", "kept")] and t is not None and t.via == "reply",
                             f"{_show(t)}; {lts}")
            o2.put("r", b"rev12")
            time.sleep(0.5)
            c = lc.view().get(f"{base}/r")
            t = lt.view().get(f"{base}/r")
            lcs = [(a.via, a.outcome) for a in lc.applied]
            lts = [(a.via, a.outcome) for a in lt.applied]
            tail = [("live", "kept"), ("live", "kept")] if not inside else [("live", "kept")]
            report.check(sec, ("the consumer, by S7 from its first value, accepts rev 11, then rev 12" if not inside
                               else "the consumer holds rev 10 until rev 12, which it accepts"),
                         c is not None and (c.payload == b"rev12" or impl == "rust") and lcs[-len(tail):] == tail
                         and c.inband is not None and c.via == "live", f"{_show(c)}; {lcs}")
            report.check(sec, ("the tool keeps the later arrival across two runs: rev 11, then rev 12" if not inside
                               else "the tool holds rev 10 until rev 12"),
                         t is not None and (t.payload == b"rev12" or impl == "rust") and lts[-len(tail):] == tail,
                         f"{_show(t)}; {lts}; {lt.bound_note()}")
        except Exception as e:  # noqa: BLE001
            report.check(sec, "the step ran", False, f"{type(e).__name__}: {e}")
        finally:
            if held is not None:
                held.release()
            if q is not None:
                try:
                    q.undeclare()
                except Exception:  # noqa: BLE001
                    pass
            for j in joiners:
                j.close()
            for o in owners:
                o.close()
            helper.close()
            tool.close()
            r.close()


def section7_ahead(report: Report, pairing: str, impl: str) -> None:
    """§7 step 6, "Ahead" (core 0.88, #978 F6, round 98): the owner's clock
    2 s ahead of a router-stamped heartbeat, serving §1's ``status``, which
    core §4.3's table lists (``freshness.v1``, ttl 60). It detects the
    drift, then tries a put and a delete of ``status``, and of a plan:
    ``status``'s are refused, each would carry a Zenoh timestamp, and the
    plan's go on under the in-band stamp alone (``plans/{vehicle}`` is on no
    row of the table). The heartbeat is put by a client of R2, whose
    timestamping is on, so R2 stamps it, and every unstamped put it routes:
    a plan's stamp on the tap is R2's, never the owner's."""
    from . import live

    sec = f"{pairing} §7 step 6, ahead"
    r, ep, r2_zid = router(True)
    beat = "lab/clock/beat"
    pub = live.open_client(ep)
    tool = live.open_client(ep)
    owner = None
    stop = threading.Event()
    try:
        def beating() -> None:
            while not stop.wait(0.2):
                pub.put(beat, b"tick")

        threading.Thread(target=beating, daemon=True).start()
        time.sleep(0.5)
        tap = Tap(tool, ["zk2/ground/fleet-mgr/**"])
        answers: dict[str, str] = {}
        if impl == "zk2py":
            owner = PyOwner(ep, offset_s=2.0, heartbeat=beat, delta_s=0.5)
            ahead = _wait(lambda: owner.o.ahead, 5.0)
            time.sleep(0.3)
            before = len(tap.of("/state/status"))  # the status put at start, before the detection
            base = owner.base
            for name, fn in (("status put", lambda: owner.status(b"up")),
                             ("status delete", lambda: owner.o.delete_state(f"{base}/state/status")),
                             ("plan put", lambda: owner.put("a", b"x")),
                             ("plan delete", lambda: owner.delete("a"))):
                try:
                    fn()
                    answers[name] = "ok"
                except Exception as e:  # noqa: BLE001
                    answers[name] = type(e).__name__
            reported = bool(owner.o.events)
            owner_zid = owner.zid()
        else:
            owner = RustOwner(ep, offset_s=2.0, flags=["--clock-reference", beat])
            time.sleep(1.5)
            ahead = True
            before = len(tap.of("/state/status"))  # the status put at start, before the detection
            for name, line in (("status put", f"put {IFACE} state/status"),
                               ("status delete", f"delete {IFACE} state/status"),
                               ("plan put", f"put {IFACE} state/plans/{{vehicle}} a"),
                               ("plan delete", f"delete {IFACE} state/plans/{{vehicle}} a")):
                got = owner.p.command(line) or ""
                answers[name] = "refused" if got.startswith("refused") else "ok"
                if got.startswith("refused"):
                    answers[name + " why"] = got[-160:]
            reported = any("ahead" in x.lower() for x in owner.p.stderr)
            owner_zid = ""
        time.sleep(0.5)
        refused = "ClockAhead" if impl == "zk2py" else "refused"
        status_rows = tap.of("/state/status")[before:]
        plan_rows = tap.of("/plans/a")
        report.check(sec, "it detects the drift; status's put and delete are refused, each would carry a Zenoh "
                          "timestamp (core §4.3's table), and nothing of status reaches the wire after the detection",
                     ahead and answers.get("status put") == refused and answers.get("status delete") == refused
                     and not status_rows,
                     f"ahead {ahead}; {answers}; status on the wire {len(status_rows)}")
        stamps = [x["ts"].split("/", 1)[1] if x["ts"] else None for x in plan_rows]
        report.check(sec, "the plan's put and delete go on, under the in-band stamp alone: plans/{vehicle} is on no "
                          "row of the table (the stamp the tap sees is R2's, set as it routes, never the owner's)",
                     answers.get("plan put") == "ok" and answers.get("plan delete") == "ok"
                     and [x["delete"] for x in plan_rows] == [False, True]
                     and all(x["read"].stamp is not None for x in plan_rows)
                     and all(z is None or z != owner_zid for z in stamps),
                     f"{[(x['delete'], x['read'].stamp, x['ts']) for x in plan_rows]}; R2 {r2_zid}")
        report.check(sec, "it reports the drift (a SHOULD)", reported,
                     f"{owner.o.events if impl == 'zk2py' else [x for x in owner.p.stderr if 'ahead' in x.lower()][-1:]}")
        tap.close()
    except Exception as e:  # noqa: BLE001
        report.check(sec, "the step ran", False, f"{type(e).__name__}: {e}")
    finally:
        stop.set()
        if owner is not None:
            owner.close()
        pub.close()
        tool.close()
        r.close()


# -- §10 ------------------------------------------------------------------------------------

def section10(report: Report, pairing: str) -> None:
    """§10 A late joiner's seed, by seq: the owner holds its answer to the
    GET (zk2py's owner, ``Owner.state_hold``)."""
    from . import live
    from .state_scenarios import Hold

    sec = f"{pairing} §10"
    if pairing != "py-py":
        report.outcome(pairing, "10", "not runnable: " + (
            "the test holds the owner's answer to a GET, and the owner example answers at once with no hold"
            if pairing == "rust-py" else "its reader is a tool's late joiner, and no black box seeds"))
        return
    r, ep, _ = router(False)
    tool = live.open_client(ep)
    owners: list[Any] = []
    try:
        o = PyOwner(ep)
        owners.append(o)
        base = f"{o.base}/state/plans"
        o.put("b", b"v1")      # seq 0
        o.put("a", b"v1")      # seq 1, as the setup has it
        t1 = o.o._held[f"{base}/a"].inband  # noqa: SLF001
        o.delete("b")          # seq 2
        t2 = o.o._held[f"{base}/b"].inband  # noqa: SLF001
        hold = Hold()
        o.o.state_hold = hold
        time.sleep(0.3)
        lj = sd.LateJoiner(tool, f"{base}/*", consumer=False, timeout=5.0)
        lj.start()
        if not report.check(sec, "the owner takes the late joiner's GET and holds it", hold.taken.wait(5.0)):
            return
        o.put("d", b"v1")
        t3 = o.o._held[f"{base}/d"].inband  # noqa: SLF001
        answer = o.o.state_answer(hold.queries[0])
        o.put("a", b"v2")
        t4 = o.o._held[f"{base}/a"].inband  # noqa: SLF001
        o.put("c", b"v1")
        t5 = o.o._held[f"{base}/c"].inband  # noqa: SLF001
        time.sleep(0.5)
        o.o.answer_state(hold.queries[0], answer)
        ended = lj.wait()
        time.sleep(0.3)
        v = lj.view()

        def e(k: str) -> Any:
            return v.get(f"{base}/{k}")

        report.check(sec, "every seq below one run's: the answer holds a (seq 1), b's reply_del (seq 2) and d (seq 3)",
                     sorted((k.rsplit("/", 1)[1], str(h.inband)) for k, h in answer)
                     == sorted([("a", str(t1)), ("b", str(t2)), ("d", str(t3))])
                     and [t.seq - t1.seq + 1 for t in (t1, t2, t3, t4, t5)] == [1, 2, 3, 4, 5],
                     str([(k.rsplit("/", 1)[1], str(h.inband)) for k, h in answer]))
        a_reply = [x for x in lj.applied if x.key.endswith("/a") and x.via == "reply"]
        report.check(sec, "expected 1: a as v2 with seq 4's stamp; the reply (seq 1) older, dropped",
                     e("a") is not None and e("a").payload == b"v2" and e("a").inband == t4
                     and [x.outcome for x in a_reply] == ["dropped"], f"{_show(e('a'))}; {[x.outcome for x in a_reply]}")
        report.check(sec, "expected 2: c as v1 with seq 5's, through the subscription",
                     e("c") is not None and e("c").inband == t5 and e("c").via == "live", _show(e("c")))
        report.check(sec, "expected 3: b as a deletion with seq 2's, the reply_del merged as a deletion",
                     e("b") is not None and e("b").deleted and e("b").inband == t2, _show(e("b")))
        d_all = [x for x in lj.applied if x.key.endswith("/d")]
        report.check(sec, "expected 4: d as v1 with seq 3's, once: the live put and the reply one stamp",
                     e("d") is not None and e("d").inband == t3 and sorted(x.outcome for x in d_all) == ["kept", "same"],
                     f"{_show(e('d'))}; {[(x.via, x.outcome) for x in d_all]}")
        report.check(sec, "expected 5: the seed ran to its final reply, with no error reply: complete",
                     ended and lj.seed.complete and not lj.seed.errors, str(lj.seed.errors))
        lj.close()
        o.o.state_hold = None
        oc = PyOwner(ep, service="fleet-mgr-control")
        owners.append(oc)
        basec = f"{oc.base}/state/plans"
        oc.put("a", b"v1")
        oc.put("b", b"v1")
        oc.delete("b")
        holdc = Hold()
        oc.o.state_hold = holdc
        time.sleep(0.3)
        ljc = sd.LateJoiner(tool, f"{basec}/*", consumer=False, timeout=5.0, order="get-first")
        ljc.start()
        holdc.taken.wait(5.0)
        answer_c = oc.o.state_answer(holdc.queries[0])
        oc.put("c", b"v1")
        time.sleep(0.3)
        oc.o.answer_state(holdc.queries[0], answer_c)
        ljc.wait()
        ljc.subscribe()
        time.sleep(0.5)
        vc = ljc.view()
        report.check(sec, "expected 6, the control: c is missing", f"{basec}/c" not in vc and f"{basec}/a" in vc,
                     str(sorted(k.rsplit("/", 1)[1] for k in vc)))
        ljc.close()
        # What a seed cannot know (observer §4.1): a GET that ran to its
        # timeout, and an owner absent at the GET.
        holdt = Hold()
        o.o.state_hold = holdt
        ljt = sd.LateJoiner(tool, f"{base}/*", consumer=False, timeout=1.0)
        ljt.start()
        ljt.wait()
        report.check(sec, "a GET held past its timeout ends with an error reply: the seed is possibly incomplete "
                          "(observer §4.1)", ljt.seed.ended and not ljt.seed.complete and bool(ljt.seed.errors),
                     str(ljt.seed.errors))
        for q in holdt.queries:
            q.drop()
        ljt.close()
        o.o.state_hold = None
        lja = sd.seed(tool, "zk2/ground/nobody/zk2py_ground.v1/state/plans/*", consumer=False)
        report.check(sec, "an owner absent at the GET seeds nothing: silence (O5), never \"no state\"",
                     lja.seed.silent and not lja.view(), f"replies {lja.seed.replies}, errors {lja.seed.errors}")
        lja.close()
    except Exception as ex:  # noqa: BLE001
        report.check(sec, "the section ran", False, f"{type(ex).__name__}: {ex}")
    finally:
        for x in owners:
            x.close()
        tool.close()
        r.close()
    report.outcome(pairing, "10", "run")


# -- §13 ------------------------------------------------------------------------------------

def _play(session: Any, key: str, stamps: list[tuple[int, int, int]]) -> None:
    for w, run, seq in stamps:
        session.put(key, zk2c.mutation(zk2c.InBand(w, run, seq), f"s{seq}".encode()), encoding=zk2c.ENCODING)
        time.sleep(0.05)


def section13(report: Report, pairing: str) -> None:
    """§13 A stream key's seq wraps (§4.3)."""
    from . import live

    sec = f"{pairing} §13"
    r, ep, _ = router(False)
    tool = live.open_client(ep)
    test = live.open_client(ep)
    owners: list[Any] = []
    try:
        base = "zk2/lab/plain/zk2py_ground.v1/stream"
        if pairing != "py-rust":
            lj = sd.LateJoiner(tool, f"{base}/*", consumer=False, timeout=3.0)
            lj.start()
            lj.wait()
            # Steps 1 and 2: a session of the test's own plays the owner.
            _play(test, f"{base}/pose", [(16, 7, s) for s in (16_777_214, 16_777_215, 0, 1)])
            _play(test, f"{base}/heading", [(16, 7, 5), (16, 7, 8_388_613)])
            time.sleep(0.4)
            pose = [x.outcome for x in lj.applied if x.key.endswith("/pose")]
            head = [x.outcome for x in lj.applied if x.key.endswith("/heading")]
            report.check(sec, "expected 1: the tool's merge keeps all four samples of pose across the wrap",
                         pose == ["kept"] * 4, str(pose))
            report.check(sec, "expected 2: it keeps both samples of heading, 2^23 apart, unordered: the later "
                              "arrival", head == ["kept", "kept"], str(head))
            lj.close()
        else:
            report.note(sec, "steps 1 and 2 need a tool's merge, and no black box merges (zenctl echo and watch show "
                             "every sample)")
        # Step 3: a candidate steps down and takes over again within its instance.
        tap = Tap(tool, ["zk2/ground/**"])
        relay_key = "zk2/ground/relay/zk2py_relay.v1/stream/entries"
        if pairing == "rust-py":
            c = RustOwner(ep, service="relay", contracts=[RELAY], flags=["--standby"])
            owners.append(c)
            for line in ("redundancy lock hold", f"redundancy wait zk2py_relay.v1 serving 5000"):
                c.command(line)
            for _ in range(3):
                c.command("put zk2py_relay.v1 stream/entries")
            for line in ("redundancy lock lose", "redundancy wait zk2py_relay.v1 standby 5000",
                         "redundancy lock hold", "redundancy wait zk2py_relay.v1 serving 5000"):
                c.command(line)
            c.command("put zk2py_relay.v1 stream/entries")
        else:
            c = PyOwner(ep, service="relay", contracts=[RELAY], standby={"zk2py_relay.v1"})
            owners.append(c)
            c.o.take_over("zk2py_relay.v1")
            for i in range(3):
                c.o.publish(relay_key, f"e{i}".encode())
            c.o.step_down("zk2py_relay.v1")
            time.sleep(0.2)
            c.o.take_over("zk2py_relay.v1")
            c.o.publish(relay_key, b"e3")
        time.sleep(0.5)
        d = descriptor(tool, c)
        got = [x["read"].stamp for x in tap.of("/stream/entries")]
        report.check(sec, "expected 3: the sample after the takeover comes at seq 3 of that run, the run unchanged, as "
                          "the descriptor says",
                     len(got) == 4 and all(g is not None for g in got) and [g.seq for g in got] == [0, 1, 2, 3]
                     and all(g.same_run(got[0]) for g in got) and d.get("run") == got[0].run_hex()
                     and got[0].width == 32,
                     f"{[str(g) for g in got]}; descriptor {d.get('run')}")
        # A templated stream member removed, then added again within the run.
        if pairing == "rust-py":
            t = RustOwner(ep, service="pace", contracts=[PACE], flags=["--timing"])
            owners.append(t)
            t.command("timing add x")
            time.sleep(0.45)
            t.command("timing remove x")
            time.sleep(0.3)
            t.command("timing add x")
            time.sleep(0.45)
            t.command("timing remove x")
            time.sleep(0.3)
            seqs = [x["read"].stamp.seq for x in tap.of("/stream/axis/x") if x["read"].stamp is not None]
        else:
            m = PyOwner(ep, service="tracks")
            owners.append(m)
            k = f"{m.base}/stream/track/x"
            for i in range(3):
                m.o.publish(k, f"t{i}".encode())
            m.o.remove_member(k)
            m.o.publish(k, b"t3")
            time.sleep(0.4)
            seqs = [x["read"].stamp.seq for x in tap.of("/stream/track/x") if x["read"].stamp is not None]
        gaps = [b - a for a, b in zip(seqs, seqs[1:])]
        report.check(sec, "expected 3: the re-added member's sample comes at the seq after its last, never at 0",
                     len(seqs) >= 2 and seqs[0] == 0 and all(g == 1 for g in gaps), str(seqs))
        tap.close()
    except Exception as e:  # noqa: BLE001
        report.check(sec, "the section ran", False, f"{type(e).__name__}: {e}")
    finally:
        for o in owners:
            o.close()
        test.close()
        tool.close()
        r.close()
    report.outcome(pairing, "13", "run" if pairing != "py-rust" else "step 3 run; steps 1 and 2 not runnable")


def section13_rust_readers(report: Report) -> None:
    """``py-rust`` §13 step 3: zk2py's candidate read by ``zenctl watch``."""
    from . import live

    sec = "py-rust §13 step 3 (zenctl watch)"
    r, ep, _ = router(False)
    owners: list[Any] = []
    try:
        w = Watch(ep, "ground/relay", "zk2py_relay.v1", "stream/entries", 8.0, contracts=RELAY)
        time.sleep(2.0)
        c = PyOwner(ep, service="relay", contracts=[RELAY], standby={"zk2py_relay.v1"})
        owners.append(c)
        key = "zk2/ground/relay/zk2py_relay.v1/stream/entries"
        c.o.take_over("zk2py_relay.v1")
        for i in range(3):
            c.o.publish(key, f"e{i}".encode())
        c.o.step_down("zk2py_relay.v1")
        time.sleep(0.2)
        c.o.take_over("zk2py_relay.v1")
        c.o.publish(key, b"e3")
        rows = [x for x in w.rows() if x.get("row") == "sample"]
        report.check(sec, "the sample after the takeover at seq 3 of the same run",
                     [x.get("inband", {}).get("seq") for x in rows] == [0, 1, 2, 3]
                     and len({x["inband"]["run"] for x in rows}) == 1, str([x.get("inband") for x in rows]))
    except Exception as e:  # noqa: BLE001
        report.check(sec, "the step ran", False, f"{type(e).__name__}: {e}")
    finally:
        for o in owners:
            o.close()
        r.close()


SECTIONS = {"1": section1, "2": section2, "7": section7, "10": section10, "13": section13}


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="python -m zk2py.state_pairings", description=__doc__.split("\n")[0])
    ap.add_argument("--pairing", action="append", choices=PAIRINGS)
    ap.add_argument("--only", action="append", choices=sorted(SECTIONS, key=int))
    args = ap.parse_args(argv)
    report = Report()
    for pairing in args.pairing or PAIRINGS:
        for n, fn in SECTIONS.items():
            if args.only and n not in args.only:
                continue
            try:
                fn(report, pairing)
            except Exception as e:  # noqa: BLE001
                report.check(f"{pairing} §{n}", "the section ran", False, f"{type(e).__name__}: {e}")
        if pairing == "py-rust" and (not args.only or "13" in args.only):
            section13_rust_readers(report)
        if not args.only:
            for n, why in NOT_RUNNABLE.items():
                report.outcome(pairing, n, f"not runnable: {why}")
    passed = sum(1 for r in report.rows if r[2])
    failed = len(report.rows) - passed
    print(f"state pairings: {passed} passed, {failed} failed")
    for sec, name, ok, detail in report.rows:
        if not ok:
            print(f"FAIL [{sec}] {name}: {detail}")
    return 1 if failed else 0


if __name__ == "__main__":
    rc = main()
    sys.stdout.flush()
    os._exit(rc)

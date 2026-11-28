"""``redundancy.v2`` ``scenarios-v2.md`` §1, §9, §13, §14 and §15 (text
0.1), live, in both pairings (#609, round 128; #987)::

    python -m zk2py.redundancy_v2_scenarios --pairing zk2py-cand|ref-cand
        [--only 1|9|13|14|15|hb|tool]... [--bin DIR]

- ``zk2py-cand``: every candidate is zk2py's (``python -m
  zk2py.redundancy_owner``), and K is the reference's ``consume --follow``.
- ``ref-cand``: every candidate is the reference's owner example, and K is
  zk2py's follower (``python -m zk2py.redundancy_v2_live --follow``), which
  takes ``consume --follow``'s command line and prints its lines.

The black boxes are run from ``--bin`` (``$ZK2PY_BIN``), never read. The
bus is :class:`zk2py.redundancy_scenarios.Bus`: R1, and R2 behind a link
the runner cuts, each a zenoh-python router stamping what arrives
unstamped (scenarios-v2.md, "The bus"). S is its watcher, which records each
delivery's Zenoh timestamp. The contracts are the reference's own files,
``zenkey/tests/contracts/ledger-run.v1.toml`` and ``ledger.v1.toml``, read
as TOML (the barrier allows them).

``hb`` reads ``consume --follow``'s heartbeat lines beside zk2py's
follower's, both following one ``beacon-governed-run.v2`` address at once.
``tool`` reads ``zenctl redundancy`` and the doctor's ``split-brain``,
``redundancy-vacancy`` and ``hostid-duplicate`` on a v2 address, beside
zk2py's own reading of §2.15 and §5.

Exit 0 when every check passes.
"""

from __future__ import annotations

import argparse
import json
import os
import queue
import secrets
import subprocess
import sys
import threading
import time
from pathlib import Path
from typing import Any

from . import live, redundancy as rd, redundancy_scenarios as rs, redundancy_v2 as r2, zk2c
from .alarms_scenarios import Report, _wait

REPO = Path(__file__).resolve().parents[3]
CONTRACTS = REPO / "zenkey" / "tests" / "contracts"
LEDGER_RUN = str(CONTRACTS / "ledger-run.v1.toml")
LEDGER_V1 = str(CONTRACTS / "ledger.v1.toml")
BEACON_RUN = str(CONTRACTS / "beacon-governed-run.v2.toml")
LEDGER = rs.LEDGER
A_ID, B_ID, C_ID = rs.A_ID, rs.B_ID, rs.C_ID
BALANCE = rs.BALANCE
BIN = Path(os.environ.get("ZK2PY_BIN", "/srv/dev/scratch/zenkey/bin/91c62f0e"))
PAIRING = {"name": "zk2py-cand"}


def cand_impl() -> Any:
    return "zk2py" if PAIRING["name"] == "zk2py-cand" else BIN / "owner"


def k_impl() -> str:
    return "rust" if PAIRING["name"] == "zk2py-cand" else "zk2py"


def label() -> str:
    return (f"candidates {'zk2py' if PAIRING['name'] == 'zk2py-cand' else 'the reference'}, "
            f"K {'the reference' if k_impl() == 'rust' else 'zk2py'}")


# -- K, either follower, behind one adapter --------------------------------------------------

class KProc:
    """One follower process: the reference's ``consume --follow`` or
    zk2py's, with the same command line and lines."""

    def __init__(self, impl: str, endpoint: str, contract: str = LEDGER_RUN, resource: str = "state/balance",
                 delay_ms: int = 0, address: str = LEDGER) -> None:
        self.impl = impl
        extra = ["--descriptor-delay-ms", str(delay_ms)] if delay_ms else []
        tail = [*extra, endpoint, address, contract, resource]
        argv = ([str(BIN / "consume"), "--follow", *tail] if impl == "rust"
                else [sys.executable, "-m", "zk2py.redundancy_v2_live", "--follow", *tail])
        env = dict(os.environ, PYTHONPATH=str(REPO / "impl" / "python"))
        self.proc = subprocess.Popen(argv, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                     text=True, env=env)
        self.lock = threading.Lock()
        self.lines: list[tuple[float, dict[str, Any]]] = []
        self.answers: queue.Queue[dict[str, Any]] = queue.Queue()
        self.stderr: list[str] = []
        threading.Thread(target=self._read, daemon=True).start()
        threading.Thread(target=lambda: [self.stderr.append(x) for x in self.proc.stderr], daemon=True).start()
        _wait(lambda: self.of("started"), 10.0, 0.02)

    def _read(self) -> None:
        for x in self.proc.stdout:
            try:
                j = json.loads(x)
            except ValueError:
                continue
            if not isinstance(j, dict):
                continue
            if j.get("follow") in ("values", "counts", "incumbent?", "runs"):
                self.answers.put(j)
                continue
            with self.lock:
                self.lines.append((time.monotonic(), j))

    def of(self, *kinds: str, after: float = 0.0) -> list[dict[str, Any]]:
        with self.lock:
            return [j for t, j in self.lines if j.get("follow") in kinds and t >= after]

    def timed(self, *kinds: str, after: float = 0.0) -> list[tuple[float, dict[str, Any]]]:
        with self.lock:
            return [(t, j) for t, j in self.lines if j.get("follow") in kinds and t >= after]

    def ask(self, word: str, timeout: float = 5.0) -> dict[str, Any]:
        while not self.answers.empty():
            self.answers.get_nowait()
        try:
            self.proc.stdin.write(word + "\n")
            self.proc.stdin.flush()
            return self.answers.get(timeout=timeout)
        except (OSError, queue.Empty):
            return {}

    def value(self, key: str = BALANCE) -> dict[str, Any] | None:
        return (self.ask("values").get("values") or {}).get(key)

    def close(self) -> None:
        try:
            self.proc.stdin.close()
        except OSError:
            pass
        try:
            self.proc.wait(10)
        except subprocess.TimeoutExpired:
            self.proc.kill()

    def brief(self, after: float = 0.0) -> str:
        out = []
        for _, j in self.timed("accepted", "discarded", "held", "superseded", "seed_discarded", "omitted", "seeded",
                               "incumbent", after=after):
            f = j["follow"]
            if f == "incumbent":
                out.append(f"incumbent {j.get('incumbent')}")
            elif f == "seeded":
                out.append(f"seeded({j.get('snapshots')},{j.get('complete')})")
            elif f == "omitted":
                out.append("omitted")
            else:
                out.append(f"{f}:{j.get('record')}@{j.get('run')}/{j.get('seq')}"
                           + (f"({j.get('reason')})" if j.get("reason") else "")
                           + ("s" if j.get("seeded") else ""))
        return "; ".join(out)


def inst(bus: rs.Bus, name: str, instance: str, on: int = 1, flags: list[str] | None = None,
         contract: str = LEDGER_RUN, impl: Any = None) -> rs.Inst:
    return bus.inst(cand_impl() if impl is None else impl, name, instance, on=on, flags=flags, contract=contract)


def run_of(i: rs.Inst) -> str | None:
    for _ in range(20):
        j = i.jcmd("redundancy run")
        if j and j.get("run"):
            return str(j["run"])
        time.sleep(0.1)
    return None


def put_value(i: rs.Inst, value: str) -> str | None:
    """A put of ``balance``: zk2py's driver takes the value; the owner
    example's ``put`` writes its raw value, ``ok``."""
    if i.rust:
        return i.cmd(f"put {i.iface} state/balance")
    return i.cmd(f"put balance {value}")


def stamp_id(stamp: str | None) -> int | None:
    return rs.stamp_zid(stamp)


def env_of(payload: bytes) -> zk2c.Envelope | None:
    try:
        return zk2c.parse(payload, "put")
    except zk2c.Refused:
        return None


class RunWatcher(rs.Watcher):
    """S, its probe also recording the run its GET's answer states."""

    def __init__(self, endpoint: str) -> None:
        self.probe_runs: list[tuple[float, str, str, bool | None, str | None]] = []
        self.raw: list[tuple[float, str, bytes, str | None]] = []
        super().__init__(endpoint)

    def _probe(self, t: float, key: str) -> None:
        chunks = key.split("/")
        at = chunks.index("@zk")
        kind, instance = chunks[at + 1], chunks[at + 2] if chunks[at + 1] == "instance" else chunks[at + 3]
        ikey = "/".join(chunks[:at]) + f"/@zk/instance/{instance}"
        got = [a for a in live.get_descriptor(self.session, ikey) if a.ok]
        mark, run = None, None
        if len(got) == 1:
            try:
                doc = json.loads(got[0].payload)
                e = (doc.get("interfaces") or [{}])[0]
                mark, run = e.get("standby") is True, doc.get("run")
            except (ValueError, AttributeError, IndexError):
                pass
        with self.lock:
            self.probes.append((t, kind, instance, mark, time.monotonic()))
            self.probe_runs.append((t, kind, instance, mark, run))

    def _data(self, smp: Any) -> None:
        super()._data(smp)
        ts = smp.timestamp
        with self.lock:
            self.raw.append((time.monotonic(), str(smp.key_expr), smp.payload.to_bytes(),
                             None if ts is None else str(ts)))

    def desc_runs(self, instance: str, after: float = 0.0) -> list[tuple[float, bool | None, str | None]]:
        key = f"zk2/{LEDGER}/@zk/instance/{instance}"
        out = []
        with self.lock:
            rows = [(t, p) for t, k, p, _ in self.raw if k == key and t >= after]
        for t, p in rows:
            try:
                doc = json.loads(p)
                out.append((t, ((doc.get("interfaces") or [{}])[0]).get("standby") is True, doc.get("run")))
            except (ValueError, AttributeError, IndexError):
                out.append((t, None, None))
        return out

    def balance(self, after: float = 0.0) -> list[tuple[float, zk2c.Envelope | None, str | None]]:
        with self.lock:
            rows = [(t, p, s) for t, k, p, s in self.raw if k == BALANCE and t >= after]
        return [(t, env_of(p), s) for t, p, s in rows]


class V2Bus(rs.Bus):
    """:class:`zk2py.redundancy_scenarios.Bus` with :class:`RunWatcher`s."""

    def __init__(self, cut: bool = False) -> None:
        orig = rs.Watcher
        rs.Watcher = RunWatcher  # type: ignore[misc]
        try:
            super().__init__(cut=cut)
        finally:
            rs.Watcher = orig  # type: ignore[misc]
        self.ks: list[KProc] = []

    def k(self, **kw: Any) -> KProc:
        k = KProc(k_impl(), self.ep1, **kw)
        self.ks.append(k)
        return k

    def close(self) -> None:
        for k in self.ks:
            try:
                k.close()
            except Exception:  # noqa: BLE001
                pass
        super().close()


# -- §1 ------------------------------------------------------------------------------------------

def section1(rp: Report) -> None:
    """§1, "As v1's, with these added to its Expected". v1's section is run
    first, on ``ledger-run.v1`` (its checks are v1's); then one run more for
    the three additions, read off S."""
    sec = f"§1 takeover after a crash [{label()}]"
    orig_bus, orig_inst = rs.Bus, rs.Bus.inst
    rs.IMPL["a"] = rs.IMPL["b"] = cand_impl()

    def inst_v2(self: Any, impl: Any, name: str, instance: str, on: int = 1, flags: list[str] | None = None,
                address: str = LEDGER, contract: str = "ledger.v1", endpoint: str | None = None) -> rs.Inst:
        return orig_inst(self, impl, name, instance, on, flags, address,
                         LEDGER_RUN if contract == "ledger.v1" else contract, endpoint)

    rs.Bus.inst = inst_v2  # type: ignore[method-assign]
    try:
        rs.section1(rp, 1)
    finally:
        rs.Bus.inst = orig_inst  # type: ignore[method-assign]
        rs.Bus = orig_bus  # type: ignore[misc]
    bus = V2Bus()
    try:
        t0 = time.monotonic()
        a = inst(bus, "A", A_ID)
        _wait(a.serving, 6.0)
        ra = run_of(a)
        b = inst(bus, "B", B_ID)
        _wait(b.stands, 8.0)
        rb = run_of(b)
        time.sleep(0.5)
        with bus.s.lock:
            pr = [x for x in bus.s.probe_runs if x[2] == A_ID and x[0] >= t0]
        at_inst = [(m, r) for _, k, _, m, r in pr if k == "instance"]
        at_alive = [(m, r) for _, k, _, m, r in pr if k == "alive"]
        rp.check(sec, "Expected 1 added: S's GET at A's instance token answers A's run, and its GET at A's interface "
                      "token the same run, unmarked",
                 bool(at_inst) and at_inst[0][1] == ra and bool(at_alive) and at_alive[0] == (False, ra),
                 f"A's run {ra}; at the instance token {at_inst[:1]}, at the interface token {at_alive[:1]}")
        k_t = a.kill()
        _wait(lambda: bus.s.first_token("put", rs._alive_of(B_ID), k_t) is not None, 8.0, 0.02)
        time.sleep(0.8)
        descs = bus.s.desc_runs(B_ID)
        marked = [r for _, m, r in descs if m is True]
        unmarked = [(t, r) for t, m, r in descs if m is False]
        rp.check(sec, "Expected 2 added: every descriptor of B's that S receives, marked, states one run, B's",
                 bool(marked) and set(marked) == {rb}, f"B's run {rb}; marked descriptors' runs {marked}")
        t_un = unmarked[0][0] if unmarked else None
        puts = [(t, e, s) for t, e, s in bus.s.balance(k_t) if e is not None]
        first = puts[0] if puts else None
        r1z = rs.zid_value(str(bus.r1.zid()))
        bz = rs.zid_value(b.zid())
        rp.check(sec, "Expected 4 added: B's descriptor put unmarked at its takeover states the run it stated while "
                      "it stood by; its put of balance at its takeover is under that run, at seq 0, and S records "
                      "R1's id on it, never B's",
                 t_un is not None and unmarked[0][1] == rb and first is not None and first[1].stamp is not None
                 and first[1].stamp.run_hex() == rb and first[1].seq == 0 and stamp_id(first[2]) == r1z
                 and stamp_id(first[2]) != bz,
                 f"unmarked {unmarked[:1]}; first put {None if first is None else (first[1].stamp, first[1].record)} "
                 f"stamp {None if first is None else first[2]}; R1 {bus.r1.zid()}, B {b.zid()}")
    finally:
        bus.close()


# -- §9 ------------------------------------------------------------------------------------------

def _split(bus: rs.Bus, b_flags: list[str] | None = None) -> tuple[rs.Inst, rs.Inst, bool]:
    a = inst(bus, "a0", A_ID, on=1)
    _wait(a.serving, 6.0)
    b = inst(bus, "b0", B_ID, on=2, flags=b_flags)
    _wait(b.stands, 10.0)
    bus.link.cut()
    took = _wait(lambda: bus.s2.first_token("put", rs._alive_of(B_ID)) is not None, 25.0, 0.05)
    return a, b, took


def section9(rp: Report) -> None:
    import zenoh

    sec = f"§9 a follower during a split [{label()}]"
    bus = V2Bus(cut=True)
    k = None
    try:
        a, b, took = _split(bus, ["--hold-heal"])
        rp.check(sec, "setup: the split established, A serving behind R1 and B behind R2", took and a.serving()
                 and b.serving(), f"A {a.standing()}, B {b.standing()}")
        ra, rb = run_of(a), run_of(b)
        za, zb = rs.zid_value(a.zid()), rs.zid_value(b.zid())
        seen: list[str | None] = []
        held: dict[str, Any] = {}

        def p_sample(smp: Any) -> None:
            # P reads by core S7 alone: within one run the higher seq, the
            # first value under a new run
            e = env_of(smp.payload.to_bytes())
            if e is None or e.stamp is None:
                return
            cur = held.get("ib")
            if cur is not None and cur.same_run(e.stamp) and e.stamp.seq <= cur.seq:
                return
            held["ib"] = e.stamp
            seen.append(e.stamp.run_hex())

        ps = live.open_client(bus.ep1)
        bus.extra.append(ps)
        bus.extra.append(ps.declare_subscriber(BALANCE, zenoh.handlers.Callback(p_sample)))
        h = time.monotonic()
        bus.link.heal()
        _wait(lambda: bus.s.first_token("put", rs._alive_of(B_ID), h) is not None, 20.0, 0.05)
        time.sleep(0.5)
        k_start = None
        for n in range(1, 6):
            put_value(a, f"a{n}")
            time.sleep(0.1)
            put_value(b, f"b{n}")
            time.sleep(0.1)
            if n == 2:
                # "K restarts during step 1, and seeds (core S4)"
                k_start = time.monotonic()
                k = bus.k()
        time.sleep(1.0)
        assert k is not None and k_start is not None
        flips = sum(1 for x, y in zip(seen, seen[1:]) if x != y)
        rp.check(sec, "1. P's value moves at each put, between A's and B's runs (core S7)",
                 flips >= 6 and {ra, rb} <= set(seen), f"{len(seen)} values, {flips} flips; A {ra}, B {rb}")
        acc = [j for j in k.of("accepted") if not j.get("seeded")]
        disc = [j for j in k.of("discarded")]
        rp.check(sec, "1. K's value moves to A's values only, in A's order; K discards every sample under B's run, "
                      "not_incumbent",
                 bool(acc) and {j["run"] for j in acc} == {ra} and [j["seq"] for j in acc] == sorted(j["seq"] for j in acc)
                 and bool(disc) and all(j["run"] == rb and j.get("reason") == "not_incumbent" for j in disc),
                 k.brief())
        own = [(t, s) for t, e, s in bus.s.balance(h) if e is not None and stamp_id(s) in (za, zb)]
        routers = {rs.zid_value(str(bus.r1.zid())), rs.zid_value(str(bus.r2.zid()))}
        ids = {stamp_id(s) for _, e, s in bus.s.balance(h) if e is not None}
        rp.check(sec, "2. no put of A's or B's carries a Zenoh timestamp of its own: S records each with a "
                      "router's id, never A's meta.zid nor B's", not own and ids <= routers and bool(ids),
                 f"{len(own)} stamped with A's or B's zid; ids {sorted(x for x in ids if x is not None)[:3]}")
        seeded = k.of("seeded")
        sacc = [j for j in k.of("accepted") if j.get("seeded")]
        sdisc = k.of("seed_discarded")
        a_arr = bool(sacc) and all(j["run"] == ra for j in sacc)
        rp.check(sec, "3. K's seed merges A's snapshot and discards B's, not_incumbent; where A's did not arrive, "
                      "K holds no value until A's next put: never B's",
                 bool(seeded) and all(j["run"] != rb for j in sacc)
                 and all(j.get("reason") == "not_incumbent" and j.get("run") == rb for j in sdisc)
                 and (a_arr or not sacc),
                 f"seeded {seeded[:1]}; seed accepted {[(j['run'], j['seq']) for j in sacc]}; seed discarded "
                 f"{[(j.get('run'), j.get('snapshot'), j.get('reason')) for j in sdisc]}")
        rp.note(sec, f"3. of S4's snapshots, A's arrived {bool(sacc)} and B's {bool(sdisc)}")
        b.release_heal()
        down = _wait(lambda: b.standing().get("role") == "standby", 12.0, 0.05)
        t_down = time.monotonic()
        time.sleep(1.0)
        put_value(a, "a6")
        time.sleep(0.6)
        v = k.value()
        late = [j for j in k.of("discarded", after=t_down - 0.5)]
        rp.check(sec, "4. B steps down; K's value stays A's, and a sample of B's after is discarded, not_incumbent",
                 down and v is not None and v.get("run") == ra and all(j.get("reason") == "not_incumbent" for j in late),
                 f"B {b.standing()}; K's value {v}; late discards {[(j['run'], j.get('reason')) for j in late]}")
    finally:
        bus.close()


# -- §13 -----------------------------------------------------------------------------------------

def section13(rp: Report) -> None:
    sec = f"§13 a takeover seen by a follower [{label()}]"
    for second in (False, True):
        bus = V2Bus()
        tag = "run 2, B puts nothing" if second else "run 1"
        try:
            a = inst(bus, "a1", A_ID)
            _wait(a.serving, 6.0)
            ra = run_of(a)
            b = inst(bus, "b1", B_ID, flags=["--takeover-puts-nothing"] if second else [])
            _wait(b.stands, 8.0)
            rb = run_of(b)
            k = bus.k()
            _wait(lambda: bool(k.of("seeded")), 5.0, 0.05)
            v0 = k.value()
            rp.check(sec, f"setup ({tag}): K has seeded and holds A's balance", v0 is not None and v0["run"] == ra,
                     f"{v0}; {k.brief()}")
            put_value(a, "a2")
            time.sleep(0.5)
            v1 = k.value()
            rp.check(sec, f"1. ({tag}) K holds a2, under A's run", v1 is not None and v1["run"] == ra
                     and v0 is not None and v1["seq"] > v0["seq"], f"{v1}")
            t_k = a.kill()
            _wait(lambda: bus.s.first_token("put", rs._alive_of(B_ID), t_k) is not None, 8.0, 0.02)
            t_b = bus.s.first_token("put", rs._alive_of(B_ID), t_k)
            time.sleep(1.5)
            inc = [(t, j.get("incumbent")) for t, j in k.timed("incumbent", after=t_k)]
            none_first = bool(inc) and inc[0][1] is None and any(x == B_ID for _, x in inc)
            seeds = k.timed("seeded", after=t_k)
            v2 = k.value()
            b_lines = [j for j in k.of("accepted", "discarded", after=t_k) if j.get("run") == rb]
            if not second:
                rp.check(sec, f"2. ({tag}) K hears A's tokens withdrawn and sees no incumbent, then B's",
                         none_first, f"{inc}")
                rp.check(sec, f"2. ({tag}) B's put of b1 is discarded no_incumbent before B's token, or accepted under "
                              "B's run after it", all((j["follow"] == "discarded" and j.get("reason") == "no_incumbent")
                                                      or (j["follow"] == "accepted") for j in b_lines) and bool(b_lines),
                         k.brief(t_k))
                rp.check(sec, f"2. ({tag}) K seeds again when B's token appears: one GET, B's one snapshot of balance "
                              "under B's run, never a reply per key",
                         len(seeds) == 1 and seeds[0][1].get("snapshots") == 1
                         and any(j.get("seeded") and j["run"] == rb for j in k.of("accepted", after=t_k)),
                         f"{[j for _, j in seeds]}")
                within = seeds and t_b is not None and seeds[0][0] - t_b < 1.0
                rp.check(sec, f"2. ({tag}) K holds b1 either way, within a round trip of B's token",
                         v2 is not None and v2["run"] == rb and v2["record"] == "b1" and bool(within),
                         f"{v2}; seeded {None if not seeds or t_b is None else round(seeds[0][0] - t_b, 3)} s after "
                         f"B's token at S")
            else:
                om = k.of("omitted", after=t_k)
                rp.check(sec, f"3. ({tag}) B's snapshot of balance is complete and lists no entry; K applies the "
                              "omission rule: it drops a2, and holds no value",
                         len(seeds) == 1 and seeds[0][1].get("snapshots") == 1 and bool(om) and v2 is None,
                         f"seeded {[j for _, j in seeds]}; omitted {om}; value {v2}")
        finally:
            bus.close()


# -- §14 -----------------------------------------------------------------------------------------

class XDesc:
    """§14's X: a test session declaring an instance token under
    ``lab/ledger`` and serving a descriptor that marks ``ledger.v1``
    standby, whose run is ``run``."""

    def __init__(self, endpoint: str, instance: str, run: str, fingerprint: str) -> None:
        import zenoh

        self.session = live.open_client(endpoint)
        key = f"zk2/{LEDGER}/@zk/instance/{instance}"
        doc = json.dumps({"format": "zk2-descriptor/0.1", "service": LEDGER, "instance": instance, "run": run,
                          "interfaces": [{"iface": "ledger.v1", "contract": fingerprint, "minor": 1, "token": True,
                                          "standby": True}],
                          "profiles": ["redundancy.v2"], "meta": {"zid": str(self.session.zid())}}).encode()
        self.entities = [self.session.declare_queryable(key, zenoh.handlers.Callback(
            lambda q: q.reply(key, doc, encoding="application/json")), complete=True)]
        self.session.put(key, doc, encoding="application/json")
        self.token = self.session.liveliness().declare_token(key)

    def withdraw(self) -> None:
        self.token.undeclare()

    def close(self) -> None:
        for e in reversed(self.entities):
            try:
                e.undeclare()
            except Exception:  # noqa: BLE001
                pass
        live.close(self.session)


def section14(rp: Report) -> None:
    from .contract import load_contract

    sec = f"§14 an incumbent's new run, and a run another instance states [{label()}]"
    bus = V2Bus()
    try:
        a = inst(bus, "a1", A_ID)
        _wait(a.serving, 6.0)
        k = bus.k(delay_ms=500)
        _wait(lambda: bool(k.of("seeded")), 5.0, 0.05)
        v0 = k.value()
        rp.check(sec, "setup: K, its descriptor subscriber delayed 500 ms, holds A's balance", v0 is not None, f"{v0}")
        t1 = time.monotonic()
        new = (a.jcmd("redundancy new-run") or {}).get("run")
        put_value(a, "a2")
        time.sleep(1.2)
        lines = k.timed("held", "accepted", "discarded", after=t1)
        held = [(t, j) for t, j in lines if j["follow"] == "held" and j.get("run") == new]
        acc = [(t, j) for t, j in lines if j["follow"] == "accepted" and j.get("run") == new]
        disc = [(t, j) for t, j in lines if j["follow"] == "discarded" and j.get("run") == new]
        d_put = [t for t, m, r in bus.s.desc_runs(A_ID, t1) if r == new]
        before_put = bool(acc) and bool(d_put) and acc[0][0] < d_put[0] + 0.5
        rp.check(sec, "1. K receives a2 before A's delayed descriptor put: it holds it, unattributed, asks one read, "
                      "and accepts a2 under A's new run when the read ends, before the delayed put arrives; it never "
                      "discards it",
                 bool(held) and bool(acc) and not disc and before_put,
                 f"new run {new}; held {len(held)}, accepted +{None if not acc else round(acc[0][0] - t1, 3)} s, "
                 f"descriptor put at S +{None if not d_put else round(d_put[0] - t1, 3)} s (+0.5 s at K); "
                 f"{k.brief(t1)}")
        # step 2: X puts balance x1 under a run no descriptor states
        x = live.open_client(bus.ep1)
        bus.extra.append(x)
        xrun = secrets.randbits(32)
        t2 = time.monotonic()
        x.put(BALANCE, zk2c.mutation(zk2c.InBand(32, xrun, 0), b"x1"), encoding=zk2c.ENCODING)
        time.sleep(1.5)
        xh = [j for j in k.of("held", after=t2) if j.get("run") == f"{xrun:08x}"]
        xd = [j for j in k.of("discarded", after=t2) if j.get("run") == f"{xrun:08x}"]
        v2 = k.value()
        rp.check(sec, "2. K holds x1, asks a read, and discards x1 when that read ends, unattributed; K's value stays "
                      "a2", bool(xh) and len(xd) == 1 and xd[0].get("reason") == "unattributed" and v2 is not None
                 and v2.get("run") == new, f"held {len(xh)}, discarded {[j.get('reason') for j in xd]}; value {v2}")
        counts = k.ask("counts")
        rp.note(sec, f"K's counts after step 2: {counts}")
        a.kill()
        time.sleep(1.0)
    finally:
        bus.close()
    # step 3: separately, X below C with C's first run
    bus = V2Bus()
    xd_: XDesc | None = None
    try:
        fp = load_contract(Path(LEDGER_RUN), spec_dir=REPO / "spec").fingerprint
        r = f"{secrets.randbits(32):08x}"
        x_id = "0000000000000001"
        xd_ = XDesc(bus.ep1, x_id, r, fp)
        time.sleep(0.3)
        t3 = time.monotonic()
        c = inst(bus, "c1", C_ID, flags=["--first-run", r])
        _wait(c.stands, 8.0)
        time.sleep(0.5)
        descs = bus.s.desc_runs(C_ID, t3)
        runs = [run for _, _, run in descs]
        st = c.standing()
        cur = run_of(c)
        rp.check(sec, "3. C's first campaign reads X's descriptor and finds its own run there: C draws a new run and "
                      "puts its descriptor stating it, before it serves; it stands by, X a lower candidate",
                 bool(runs) and runs[0] == r and cur is not None and cur != r and cur in runs
                 and st.get("role") == "standby" and st.get("why") == "lower_candidate",
                 f"R {r}; C's descriptor puts' runs {runs}; its run now {cur}; {st}")
        t4 = time.monotonic()
        xd_.withdraw()
        served = _wait(c.serving, 10.0, 0.05)
        time.sleep(0.5)
        after = [run for _, m, run in bus.s.desc_runs(C_ID, t4)]
        puts = [e.stamp.run_hex() for _, e, _ in bus.s.balance(t4) if e is not None and e.stamp is not None]
        rp.check(sec, "4. X withdraws its instance token: C takes over under its new run, never under R",
                 served and r not in after and r not in puts and bool(puts) and set(puts) == {cur},
                 f"served {served}; descriptor runs {after}; balance puts' runs {puts}")
    finally:
        if xd_ is not None:
            xd_.close()
        bus.close()


# -- §15 -----------------------------------------------------------------------------------------

def section15(rp: Report) -> None:
    from .redundancy_live import Follower as V1Follower

    sec = f"§15 both majors side by side [{label()}]"
    bus = V2Bus()
    k1 = None
    try:
        k = bus.k()
        # K1, built for redundancy.v1 alone: zk2py's v1 follower (by meta.zid)
        k1 = V1Follower(bus.ep1, LEDGER, "ledger.v1", "balance")
        a = inst(bus, "a1", A_ID, contract=LEDGER_V1)
        _wait(a.serving, 6.0)
        b = inst(bus, "b1", B_ID)
        _wait(b.stands, 8.0)
        za, zb = rs.zid_value(a.zid()), rs.zid_value(b.zid())
        time.sleep(0.5)
        from .redundancy_live import read_tool
        tr = read_tool(bus.g, LEDGER, "ledger.v1", grace_s=1.0)
        rp.check(sec, "1. A serves and B stands by: one election; zk2py's tool reads the address governed, claim, "
                      "never revisions_disagree", a.serving() and b.stands() and tr.reading.governance == "claim",
                 f"A {a.standing()}, B {b.standing()}; governance {tr.reading.governance}, majors {tr.majors}")
        if rs.ZENCTL is not None or (BIN / "zenctl").is_file():
            rc, doc, err = _zenctl(["redundancy", LEDGER, "--format", "json", "--grace", "1", "-c", bus.ep1])
            row = next((x for x in (doc or {}).get("rows") or [] if x.get("iface") == "ledger.v1"), {})
            rp.check(sec, "1. zenctl redundancy reads it governed, claim, both majors, never revisions_disagree",
                     row.get("governance") == "claim" and sorted(row.get("majors") or []) == ["redundancy.v1",
                                                                                              "redundancy.v2"],
                     f"exit {rc}; governance {row.get('governance')}, majors {row.get('majors')}")
        ta = time.monotonic()
        rp.note(sec, f"K at A's (redundancy.v1) token, before step 1's put: {k.brief()}")
        put_value(a, "a1x")
        time.sleep(0.6)
        a_puts = [(e, s) for _, e, s in bus.s.balance(ta) if e is not None]
        rp.check(sec, "1. A's put carries A's Zenoh timestamp, its id A's meta.zid",
                 bool(a_puts) and all(stamp_id(s) == za for _, s in a_puts), f"{[s for _, s in a_puts]}; A {a.zid()}")
        acc = [j for j in k.of("accepted", after=ta) if not j.get("seeded")]
        k1v = k1.value
        rp.check(sec, "1. K follows A by meta.zid and accepts a1; so does K1",
                 bool(acc) and k1v is not None and bool(k1.accepted), f"K {k.brief(ta)}; K1 {k1v}")
        t2 = a.kill()
        _wait(b.serving, 10.0, 0.05)
        time.sleep(1.2)
        b_puts = [(e, s) for _, e, s in bus.s.balance(t2) if e is not None]
        rp.check(sec, "2. B's puts carry no Zenoh timestamp of B's", bool(b_puts)
                 and all(stamp_id(s) != zb for _, s in b_puts), f"{[s for _, s in b_puts]}; B {b.zid()}")
        v = k.value()
        rb = run_of(b)
        seeded = k.of("seeded", after=t2)
        rp.check(sec, "2. K follows B by run from B's token on: it seeds again, and holds b1",
                 v is not None and v.get("run") == rb and bool(seeded), f"{v}; {k.brief(t2)}")
        disc1 = [x for x in k1.discarded if x[0] >= t2]
        rp.check(sec, "2. K1 finds no stamp of B's meta.zid on any of B's samples: it discards each, and holds a1",
                 k1.value is not None and k1v == k1.value and bool(disc1),
                 f"K1 value {k1.value}; discarded since the kill {len(disc1)}")
        t3 = time.monotonic()
        a2 = inst(bus, "a3", "00000000000000a0", contract=LEDGER_V1)
        _wait(a2.stands, 8.0)
        time.sleep(0.8)
        st = a2.standing()
        v3 = k.value()
        rp.check(sec, "3. the new A stands by on B's token; B keeps serving, and K's value stays B's",
                 st.get("role") == "standby" and st.get("why") == "incumbent" and b.serving() and v3 is not None
                 and v3.get("run") == rb, f"new A {st}; B {b.standing()}; K {v3}; {k.brief(t3)}")
    finally:
        if k1 is not None:
            k1.close()
        bus.close()


# -- the heartbeats --------------------------------------------------------------------------------

def section_hb(rp: Report) -> None:
    """``consume --follow``'s heartbeat lines against zk2py's follower's: both
    follow one ``beacon-governed-run.v2`` address at once. A serves and B
    stands by; X puts a heartbeat under B's run, then one under a run no
    descriptor states; A is killed and B takes over."""
    import zenoh

    from .contract import load_contract

    sec = f"heartbeats, consume --follow beside zk2py's follower [{label()}]"
    bus = V2Bus()
    try:
        addr = "lab/beacon"
        a = bus.inst(cand_impl(), "a", A_ID, address=addr, contract=BEACON_RUN)
        _wait(a.serving, 6.0)
        b = bus.inst(cand_impl(), "b", B_ID, address=addr, contract=BEACON_RUN)
        _wait(b.stands, 8.0)
        ra, rb = run_of(a), run_of(b)
        ks = {"rust": KProc("rust", bus.ep1, BEACON_RUN, "state/status", address=addr),
              "zk2py": KProc("zk2py", bus.ep1, BEACON_RUN, "state/status", address=addr)}
        bus.ks += list(ks.values())
        t0 = time.monotonic()
        time.sleep(2.5)
        hb_key = f"zk2/{addr}/freshness.v2/stream/heartbeat/beacon.v2"
        x = live.open_client(bus.ep1)
        bus.extra.append(x)
        body = b""
        t1 = time.monotonic()
        x.put(hb_key, zk2c.heartbeat(zk2c.InBand(32, int(rb, 16), 900), body, 0), encoding=zk2c.ENCODING)
        time.sleep(0.6)
        xr = secrets.randbits(32)
        t2 = time.monotonic()
        x.put(hb_key, zk2c.heartbeat(zk2c.InBand(32, xr, 901), body, 0), encoding=zk2c.ENCODING)
        time.sleep(1.5)
        tk = a.kill()
        _wait(b.serving, 10.0, 0.05)
        time.sleep(2.5)
        for who, k in ks.items():
            hb_a = [j for j in k.of("heartbeat", after=t0) if j.get("run") == ra]
            dis_b = [j for j in k.of("heartbeat_discarded", after=t1) if j.get("run") == rb and j.get("seq") == 900]
            held_x = [j for j in k.of("heartbeat_held", after=t2) if j.get("run") == f"{xr:08x}"]
            dis_x = [j for j in k.of("heartbeat_discarded", after=t2) if j.get("run") == f"{xr:08x}"]
            hb_b = [j for j in k.of("heartbeat", after=tk) if j.get("run") == rb]
            rp.check(sec, f"{who}'s follower: A's heartbeats read under A's run; X's under B's run discarded "
                          "not_incumbent; X's under no descriptor's run held, then discarded unattributed; B's read "
                          "under B's run after its takeover",
                     bool(hb_a) and len(dis_b) == 1 and dis_b[0].get("reason") == "not_incumbent" and bool(held_x)
                     and len(dis_x) == 1 and dis_x[0].get("reason") == "unattributed" and bool(hb_b),
                     f"A's {len(hb_a)}, X under B's {[j.get('reason') for j in dis_b]}, X unknown held {len(held_x)} "
                     f"discarded {[j.get('reason') for j in dis_x]}, B's {len(hb_b)}")
        def shape(j: dict[str, Any]) -> tuple[Any, ...]:
            return (j["follow"], j.get("key"), j.get("run"), j.get("seq"), j.get("reason"))

        r = [shape(j) for j in ks["rust"].of("heartbeat", "heartbeat_discarded", "heartbeat_held", after=t1)]
        z = [shape(j) for j in ks["zk2py"].of("heartbeat", "heartbeat_discarded", "heartbeat_held", after=t1)]
        rp.check(sec, "the two followers' heartbeat lines from X's first put on: the same kinds, keys, runs and "
                      "reasons, seqs aside", sorted({x[:3] + x[4:] for x in r}) == sorted({x[:3] + x[4:] for x in z}),
                 f"reference {sorted(set(x[:3] + x[4:] for x in r))}; zk2py {sorted(set(x[:3] + x[4:] for x in z))}")
        for who, k in ks.items():
            at_take = [(round(t - tk, 3), j["follow"], j.get("seq")) for t, j in
                       k.timed("heartbeat", "heartbeat_held", "heartbeat_discarded", after=tk) if j.get("run") == rb][:4]
            rp.note(sec, f"{who}'s follower at B's takeover, B's first heartbeats (s after the kill, line, seq): "
                         f"{at_take}; its incumbent lines {[(round(t - tk, 3), j.get('incumbent')) for t, j in k.timed('incumbent', after=tk)]}")
        rp.note(sec, f"line names: reference {sorted({j['follow'] for j in ks['rust'].of('heartbeat', 'heartbeat_discarded', 'heartbeat_held')})}"
                     f"; keys {sorted({j.get('key') for j in ks['rust'].of('heartbeat')})}")
    finally:
        bus.close()


def section_hb_takeover(rp: Report) -> None:
    """§2.10 "How often. Never at a takeover, since a standby states its run
    from its claim on (Appendix B)": both followers start after B's claim,
    read nothing of B before its takeover, and B's first heartbeats after
    its interface token are read. Run three times."""
    sec = f"heartbeats at a takeover, followers started after B's claim [{label()}]"
    for n in range(3):
        bus = V2Bus()
        try:
            addr = "lab/beacon"
            a = bus.inst(cand_impl(), "a", A_ID, address=addr, contract=BEACON_RUN)
            _wait(a.serving, 6.0)
            b = bus.inst(cand_impl(), "b", B_ID, address=addr, contract=BEACON_RUN)
            _wait(b.stands, 8.0)
            rb = run_of(b)
            ks = {"rust": KProc("rust", bus.ep1, BEACON_RUN, "state/status", address=addr),
                  "zk2py": KProc("zk2py", bus.ep1, BEACON_RUN, "state/status", address=addr)}
            bus.ks += list(ks.values())
            time.sleep(1.5)
            tk = a.kill()
            _wait(b.serving, 10.0, 0.05)
            time.sleep(2.5)
            for who, k in ks.items():
                first = [(round(t - tk, 3), j["follow"], j.get("seq"), j.get("reason")) for t, j in
                         k.timed("heartbeat", "heartbeat_held", "heartbeat_discarded", after=tk)
                         if j.get("run") == rb][:3]
                inc = [(round(t - tk, 3), j.get("incumbent")) for t, j in k.timed("incumbent", after=tk)]
                held = [x for x in first if x[1] == "heartbeat_held"]
                rp.note(sec, f"run {n + 1}, {who}'s follower: B's first heartbeats {first}; incumbent {inc}")
                # rule step 1: one that overtakes B's token is no_incumbent;
                # "Never at a takeover": none is held
                rp.check(sec, f"run {n + 1}, {who}'s follower reads B's heartbeats under B's run after its takeover: "
                              "none held, and none discarded but no_incumbent before B's token reached it",
                         bool(first) and not held and any(x[1] == "heartbeat" for x in first)
                         and all(x[3] == "no_incumbent" and bool(inc) and inc[-1][0] >= x[0] - 0.002
                                 for x in first if x[1] == "heartbeat_discarded"),
                         f"held {len(held)}")
        finally:
            bus.close()


# -- the tools ------------------------------------------------------------------------------------

def _zenctl(args: list[str], timeout: float = 90.0) -> tuple[int | None, Any, str]:
    exe = rs.ZENCTL or (BIN / "zenctl")
    try:
        p = subprocess.run([str(exe), *args], capture_output=True, text=True, timeout=timeout)
    except subprocess.TimeoutExpired:
        return None, None, "timeout"
    try:
        doc = json.loads(p.stdout)
    except ValueError:
        doc = None
    return p.returncode, doc, p.stderr


def section_tool(rp: Report) -> None:
    """``zenctl redundancy`` and the doctor on a v2 address against zk2py's
    reading of §2.15 and §5: A serving and B standing by; a split held at
    its heal; a vacancy under ``external`` with no lock (``valve-run.v1``);
    and hostid.v1 scenarios §6 step 7's recognised standby of either
    major."""
    from . import hostid_scenarios as hs
    from .contract import load_contract
    from .redundancy_live import read_tool, split_findings

    sec = f"tools on a redundancy.v2 address [{label()}]"
    held = [load_contract(Path(LEDGER_RUN), spec_dir=REPO / "spec")]
    bus = V2Bus(cut=True)
    try:
        a = inst(bus, "a", A_ID, on=1)
        _wait(a.serving, 6.0)
        b = inst(bus, "b", B_ID, on=2, flags=["--hold-heal"])
        _wait(b.stands, 8.0)
        ra, rb = run_of(a), run_of(b)
        time.sleep(0.5)
        rc, doc, err = rs.zenctl(["redundancy", LEDGER, "--grace", "1", "-c", bus.ep1, "--contracts", LEDGER_RUN])
        row = next((x for x in (doc or {}).get("rows") or [] if x.get("row") == "interface"), {})
        insts = {i.get("instance"): i for i in row.get("instances") or []}
        tr = read_tool(bus.g, LEDGER, "ledger.v1", grace_s=1.0, contracts=held)
        rp.check(sec, "A serving, B standing by: zenctl redundancy exits 0, governed claim, majors redundancy.v2, "
                      "each candidate's run beside its instance (§2.15)",
                 rc == 0 and row.get("governance") == "claim" and row.get("majors") == ["redundancy.v2"]
                 and insts.get(A_ID, {}).get("run") == ra and insts.get(B_ID, {}).get("run") == rb
                 and insts.get(A_ID, {}).get("role") == "incumbent" and insts.get(B_ID, {}).get("role") == "standby",
                 f"exit {rc}; {row.get('governance')} {row.get('majors')}; A {insts.get(A_ID, {}).get('run')} "
                 f"{insts.get(A_ID, {}).get('role')}, B {insts.get(B_ID, {}).get('run')} "
                 f"{insts.get(B_ID, {}).get('role')}; runs {ra} {rb}; {err.strip()[:200]}")
        zf, mf = rs._flat_of_zenctl(doc), rs._flat_of_mine(tr.reading)
        rp.check(sec, "A serving, B standing by: zk2py's §5 answers are zenctl's, and zk2py's tool shows each run",
                 zf == mf and tr.runs().get(A_ID) == ra and tr.runs().get(B_ID) == rb and tr.majors == ["redundancy.v2"],
                 f"zenctl {zf}; zk2py {mf}; zk2py's runs {tr.runs()}, majors {tr.majors}")
        drc, dv, _ = rs._doctor(["split-brain", "redundancy-vacancy"], bus.ep1)
        rp.check(sec, "A serving, B standing by: the doctor's split-brain and redundancy-vacancy are not established",
                 drc == 0 and all((dv.get(c) or {}).get("answer") == "not_established"
                                  for c in ("split-brain", "redundancy-vacancy")),
                 f"exit {drc}; {[(c, (dv.get(c) or {}).get('answer')) for c in ('split-brain', 'redundancy-vacancy')]}")
        bus.link.cut()
        _wait(lambda: bus.s2.first_token("put", rs._alive_of(B_ID)) is not None, 25.0, 0.05)
        h = time.monotonic()
        bus.link.heal()
        _wait(bus.linked, 10.0, 0.2)
        time.sleep(1.0)
        rc2, doc2, _ = rs.zenctl(["redundancy", LEDGER, "--grace", "1", "-c", bus.ep1, "--contracts", LEDGER_RUN])
        tr2 = read_tool(bus.g, LEDGER, "ledger.v1", grace_s=1.0, contracts=held)
        zf2, mf2 = rs._flat_of_zenctl(doc2), rs._flat_of_mine(tr2.reading)
        rp.check(sec, "the split held at its heal: zenctl exits 1, one incumbent no, split; zk2py's answers are "
                      "zenctl's", rc2 == 1 and zf2 is not None and zf2.get("one") == ("no", "split") and zf2 == mf2,
                 f"exit {rc2}; zenctl {zf2}; zk2py {mf2}")
        drc2, dv2, _ = rs._doctor(["split-brain"], bus.ep1)
        sb = dv2.get("split-brain") or {}
        text = json.dumps(sb)
        splits = split_findings(bus.g, LEDGER, ["ledger.v1"], grace_s=1.0, contracts=held)
        rp.check(sec, "the split: the doctor's split-brain finds it, naming A and B, exit 1; so does zk2py's, as "
                      "redundancy.v2's", drc2 == 1 and sb.get("answer") == "established" and A_ID in text
                 and B_ID in text and splits == [("ledger.v1", [A_ID, B_ID], "redundancy.v2")],
                 f"exit {drc2}; {sb.get('answer')}; zk2py {splits}")
        b.release_heal()
        _wait(lambda: b.standing().get("role") == "standby", 12.0, 0.05)
    finally:
        bus.close()
    # a vacancy: valve-run.v1 under external, no lock held
    bus = V2Bus()
    try:
        valve = str(CONTRACTS / "valve-run.v1.toml")
        v = bus.inst(cand_impl(), "v", A_ID, address="lab/valve", contract=valve, flags=["--election", "external"])
        _wait(v.stands, 8.0)
        time.sleep(0.5)
        rc, doc, err = rs.zenctl(["redundancy", "lab/valve", "--grace", "1", "-c", bus.ep1, "--contracts", valve])
        tr = read_tool(bus.g, "lab/valve", "valve.v1", grace_s=1.0,
                       contracts=[load_contract(Path(valve), spec_dir=REPO / "spec")])
        zf, mf = rs._flat_of_zenctl(doc), rs._flat_of_mine(tr.reading)
        rp.check(sec, "a vacancy under external, no lock: zenctl redundancy says vacancy yes, exit 1; zk2py's "
                      "answers are zenctl's", rc == 1 and zf is not None and zf.get("vacancy", (None,))[0] == "yes"
                 and zf == mf, f"{v.standing()}; exit {rc}; zenctl {zf}; zk2py {mf}")
        drc, dv, _ = rs._doctor(["redundancy-vacancy"], bus.ep1)
        rv = dv.get("redundancy-vacancy") or {}
        rp.check(sec, "a vacancy: the doctor's redundancy-vacancy is established, a warning, exit 1",
                 drc == 1 and rv.get("answer") == "established", f"exit {drc}; {rv}")
    finally:
        bus.close()
    # hostid.v1 scenarios §6 step 7: B on ledger-run.v1, A on ledger.v1, then both on ledger-run.v1
    import tempfile

    for pair in ((LEDGER_V1, LEDGER_RUN), (LEDGER_RUN, LEDGER_RUN)):
        base_dir = tempfile.TemporaryDirectory(prefix="zk2py-r128-")
        root = hs.make_root(base_dir.name, {hs.ETC: hs.M1 + "\n"})
        minted = f"{hs.SYSTEM[hs.M1]}/ledger"
        bus = V2Bus()
        tag = "A on ledger.v1, B on ledger-run.v1" if pair[0] == LEDGER_V1 else "both on ledger-run.v1"
        try:
            a = bus.inst(cand_impl(), "A", A_ID, flags=["--hostid-root", root], address="@hostid.v1/ledger",
                         contract=pair[0])
            _wait(a.serving, 6.0)
            b = bus.inst(cand_impl(), "B", B_ID, flags=["--hostid-root", root], address="@hostid.v1/ledger",
                         contract=pair[1])
            _wait(b.stands, 8.0)
            time.sleep(0.5)
            answer, why, _ = live.hostid_collision(bus.g, minted, grace_s=1.0)
            rp.check(sec, f"hostid.v1 0.10, its scenarios §6 step 7 ({tag}): zk2py's tool recognises B a standby: "
                          "no finding, B named", answer == "no" and B_ID in why and "recognised standby" in why,
                     f"{a.standing()}, {b.standing()}; {answer}: {why}")
            drc, dv, _ = rs._doctor(["hostid-duplicate"], bus.ep1)
            hd = dv.get("hostid-duplicate") or {}
            rp.check(sec, f"hostid.v1 0.10 ({tag}): zenctl doctor finds no hostid-duplicate, and names B a "
                          "recognised standby", drc == 0 and hd.get("answer") == "not_established"
                     and B_ID in json.dumps(hd), f"exit {drc}; {json.dumps(hd)[:300]}")
        finally:
            bus.close()
            base_dir.cleanup()


def _doctor_brief(doc: Any) -> str:
    if not isinstance(doc, dict):
        return "no document"
    out = []
    for c in doc.get("checks") or doc.get("results") or []:
        if isinstance(c, dict):
            out.append(f"{c.get('id') or c.get('check')}={c.get('verdict') or c.get('status') or c.get('state')}")
    return ", ".join(out) or json.dumps(doc)[:300]


SECTIONS = {"1": section1, "9": section9, "13": section13, "14": section14, "15": section15, "hb": section_hb,
            "hb-takeover": section_hb_takeover,
            "tool": section_tool}


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="python -m zk2py.redundancy_v2_scenarios", description=__doc__.split("\n")[0])
    ap.add_argument("--pairing", choices=("zk2py-cand", "ref-cand"), required=True)
    ap.add_argument("--only", action="append", choices=list(SECTIONS))
    ap.add_argument("--bin", help="the black boxes' directory (else $ZK2PY_BIN)")
    a = ap.parse_args(argv)
    global BIN
    if a.bin:
        BIN = Path(a.bin)
    PAIRING["name"] = a.pairing
    rs.ZENCTL = rs.ZENCTL or (BIN / "zenctl")
    rp = Report()
    for name, fn in SECTIONS.items():
        if a.only and name not in a.only:
            continue
        try:
            fn(rp)
        except Exception as e:  # noqa: BLE001 - a section that crashes fails, the rest run
            import traceback

            traceback.print_exc()
            rp.check(f"§{name}", "the section ran to its end", False, f"{type(e).__name__}: {e}")
    passed = sum(1 for r in rp.rows if r[2])
    failed = len(rp.rows) - passed
    print(f"redundancy.v2 scenarios ({label()}): {passed} passed, {failed} failed", flush=True)
    return 0 if failed == 0 else 1


if __name__ == "__main__":
    rc = main()
    sys.stdout.flush()
    os._exit(rc)

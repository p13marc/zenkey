"""``spec/scenarios/state.md`` §16 at core 0.110 (#986; #609, round 116),
live: a conditional read. ``python -m zk2py.conditional_live [--pairing P]…
[--old DIR]``.

**The pairings.**
- ``py-py``: zk2py's owner (:class:`zk2py.owner.Owner`) and zk2py's reader:
  steps 1 to 8 by P, a session with no subscription, each GET made twice,
  raw for its bytes and through :func:`zk2py.live.get_state` for the
  reader's verdict; steps 9 and 10 by zk2py's consumer
  (:class:`zk2py.seed.LateJoiner`) against a played owner, then by a tool's
  late joiner; and a reader's merge of each answer
  (:meth:`zk2py.seed.LateJoiner.conditional_read`);
- ``rust-py``: the reference's owner example (a black box) read by zk2py's
  conditional read, steps 1 to 8 and the reader's merge;
- ``py-rust``: zk2py's owner read by the reference, ``zenctl get <key>
  --body @FILE``, whose body is the held stamp, sent under ``zenoh/bytes``;
  steps 9 and 10's held stamps played to it too;
- ``old-py``: zk2py's conditional read against the owner example built
  before core 0.110 (``--old``, a directory of black boxes): "An owner built
  before this version reads no payload on a state GET. It answers every
  conditional read whole", and the reader "MUST accept a snapshot in
  answer"; ``--old`` is also read by its ``zenctl``, a reader built before
  this version, of a held stamp: "the same bytes are a body cut ``short``".

**The contract** is ``interop/state/zk2py_ground.v1.toml``: ``plans/{vehicle}``,
a collection its owner answers by snapshot, and ``status``, a per-key
resource (``freshness.ttl_s`` 60 under ``freshness.v1``).

**Windows.** zk2py's owner runs §16's W, 2 s. The owner example keeps its
60 s, so its step 7 waits it out once.

**Records.** zk2py's owner puts ``v1``, ``v2`` and ``v3``. The owner example
puts its own record, ``ok``, each time, at a new seq.

Every listener is on a port the OS picked. Exit 0 when every check passes, 1
when any fails.
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
import threading
import time
from pathlib import Path
from typing import Any

from . import held, seed_held, zk2c
from . import seed as sd
from . import seed_snapshot as ss
from .blackbox import BLACKBOX, workdir
from .snapshot_live import Report
from .zk2c import InBand

REPO = Path(__file__).resolve().parents[3]
GROUND = REPO / "impl" / "python" / "interop" / "state" / "zk2py_ground.v1.toml"
IFACE = "zk2py_ground.v1"
ADDRESS = "ground/fleet-mgr"
BASE = f"zk2/{ADDRESS}/{IFACE}/state"
PLAN = f"{BASE}/plans/vehicle-01"
STATUS = f"{BASE}/status"
PAIRINGS = ("py-py", "rust-py", "py-rust", "old-py")
#: §16's W for zk2py's owner, and the owner example's
PY_WINDOW_S, RUST_WINDOW_S = 2.0, 60.0
#: the older black boxes, where ``--old`` is not given
OLD_DEFAULT = BLACKBOX.parent / "547d202a"


def _contract() -> Any:
    from .contract import load_contract

    return load_contract(GROUND)


class Ground:
    """One instance of ``ground/fleet-mgr``, zk2py's or an owner example's,
    a client of ``ep``, its runs from the record ``record``."""

    def __init__(self, impl: str, ep: str, record: str, box: Path = BLACKBOX, conditional: bool = True) -> None:
        self.impl, self.ep, self.record, self.box = impl, ep, record, box
        self.contract = _contract()
        if impl == "zk2py":
            from .owner import Owner

            self.o: Any = Owner("ground", "fleet-mgr", [self.contract], connect=ep, tombstone_s=PY_WINDOW_S,
                                run_record=record)
            self.o.conditional = conditional
            self.o.start()
            self.window_s = PY_WINDOW_S
        else:
            from .live_interop import Owner as Exe

            self.p = Exe(box / "owner", ADDRESS, [GROUND], connect=ep, flags=["--run-file", record])
            if self.p.wait_for("ready ", 120) is None:
                raise RuntimeError(f"the owner example did not start: {self.p.stderr[-3:]}")
            self.window_s = RUST_WINDOW_S

    def put(self, record: bytes) -> None:
        if self.impl == "zk2py":
            self.o.set_state(PLAN, record)
        else:
            got = self.p.command(f"put {IFACE} state/plans/{{vehicle}} vehicle-01")
            if got is None or got.startswith("refused"):
                raise RuntimeError(f"put: {got!r}")

    def put_status(self) -> None:
        """``status``'s last put: zk2py's owner puts one; the owner example
        holds its own from its start."""
        if self.impl == "zk2py":
            self.o.set_state(STATUS, b"ok")

    def delete(self) -> None:
        if self.impl == "zk2py":
            self.o.delete_state(PLAN)
        else:
            got = self.p.command(f"delete {IFACE} state/plans/{{vehicle}} vehicle-01")
            if got is None or got.startswith("refused"):
                raise RuntimeError(f"delete: {got!r}")

    def record_of(self, name: bytes) -> bytes:
        return name if self.impl == "zk2py" else b"ok"

    def close(self) -> None:
        if self.impl == "zk2py":
            self.o.close()
        else:
            self.p.close()


def raw_get(session: Any, selector: str, payload: bytes | None = None, encoding: str = zk2c.ENCODING,
            timeout: float = 5.0) -> list[tuple[str, bytes, str, bool]]:
    """Every reply as it arrived: its key, its payload, its Encoding and
    whether it carries a Zenoh timestamp."""
    import zenoh

    kw: dict[str, Any] = {} if payload is None else {"payload": payload, "encoding": encoding}
    out = []
    for rep in session.get(selector, target=zenoh.QueryTarget.ALL, consolidation=zenoh.ConsolidationMode.NONE,
                           timeout=timeout, **kw):
        if rep.ok is not None:
            out.append((str(rep.ok.key_expr), rep.ok.payload.to_bytes(), str(rep.ok.encoding),
                        rep.ok.timestamp is not None))
    return out


def _page(reply: tuple[str, bytes, str, bool]) -> dict[str, Any] | None:
    from . import snapshot

    if held.read_reply_on(reply[0], reply[2], reply[1]).reads != "page":
        return None
    try:
        return ss.read_page_of(reply[0], reply[1])
    except (snapshot.Refused, zk2c.Refused):
        return None


def _entry(replies: list[tuple[str, bytes, str, bool]], key: str = PLAN) -> tuple[str, InBand | None, bytes | None]:
    """What one answer is: ``held`` with its stamp; ``live`` or ``tomb`` with
    the one entry's stamp (and a live one's record), for one page on ``key``
    whose one entry has the empty id; ``none`` for one page of no entry;
    else ``other``."""
    if len(replies) != 1 or replies[0][0] != key or replies[0][2] != zk2c.ENCODING:
        return "other", None, None
    r = held.read_reply_on(*replies[0][:1], replies[0][2], replies[0][1])
    if r.held:
        return "held", r.stamp, None
    page = _page(replies[0])
    if page is None or page["index"] != 0 or page["more"]:
        return "other", None, None
    entries = [(False, e) for e in page["live"]] + [(True, e) for e in page["tombs"]]
    if not entries:
        return "none", InBand(page["run_width"], page["run"], page["asof"]), None
    if len(entries) != 1 or entries[0][1]["id"] != "":
        return "other", None, None
    tomb, e = entries[0]
    stamp = seed_held.entry_stamp(page, e)
    return ("tomb", stamp, None) if tomb else ("live", stamp, bytes.fromhex(e["record"]))


class P:
    """§16's reader P: "a session of the test's own, with no subscription".
    Each GET is made raw, for its bytes, and through zk2py's conditional read
    (:func:`zk2py.live.get_state`), for the reader's verdict."""

    def __init__(self, session: Any) -> None:
        self.session = session

    def ask(self, selector: str, stamp: InBand | None = None, payload: bytes | None = None,
            encoding: str = zk2c.ENCODING) -> tuple[list[tuple[str, bytes, str, bool]], Any]:
        from . import live

        sent = held.write(stamp) if stamp is not None else payload
        raw = raw_get(self.session, selector, sent, encoding)
        if stamp is not None and held.one_concrete_key(selector):
            reading = live.get_state(self.session, selector, timeout=5.0, held=stamp)
        else:
            reading = live.get_state(self.session, selector, timeout=5.0, payload=sent, encoding=encoding)
        return raw, reading


class ZenctlP:
    """The reference as §16's P: ``zenctl get <selector> [--body @FILE]
    --raw --no-decode``, its body sent as typed, and its decoded row beside
    it for what the tool says of a held stamp."""

    def __init__(self, ep: str, box: Path, tmp: str) -> None:
        self.ep, self.box, self.tmp, self.n = ep, box, Path(tmp), 0
        self.said: list[str] = []
        self.rows: list[dict[str, Any]] = []

    def _run(self, *a: str) -> tuple[int, dict[str, Any], str]:
        p = subprocess.run([str(self.box / "zenctl"), *a], capture_output=True, text=True, timeout=60)
        try:
            doc = json.loads(p.stdout or "{}")
        except ValueError:
            doc = {"raw": p.stdout[-400:]}
        return p.returncode, doc, p.stderr[-300:]

    def ask(self, selector: str, stamp: InBand | None = None, payload: bytes | None = None,
            encoding: str = zk2c.ENCODING) -> tuple[list[tuple[str, bytes, str, bool]], Any]:
        if encoding != zk2c.ENCODING:
            raise NotImplementedError("zenctl get sends its body under zenoh/bytes alone")
        sent = held.write(stamp) if stamp is not None else payload
        body: list[str] = []
        if sent is not None:
            self.n += 1
            f = self.tmp / f"body-{self.n}.bin"
            f.write_bytes(sent)
            body = ["--body", f"@{f}"]
        rc, doc, err = self._run("get", "-c", self.ep, "--format", "json", "--raw", "--no-decode", *body, selector)
        rows = [x for x in doc.get("rows", []) if x.get("row") == "answer"]
        raw = [(x["key"], bytes.fromhex(x["hex"].replace(" ", "")), x.get("encoding", ""), False) for x in rows]
        rc2, doc2, _ = self._run("get", "-c", self.ep, "--format", "json", "--contracts", str(GROUND), *body, selector)
        said = [str(x.get("decode_error") or x.get("type")) for x in doc2.get("rows", []) if x.get("row") == "answer"]
        self.said = said
        #: the decoded rows whole (round 125: observer 0.36 §4.1, a held
        #: stamp that answers the tool's own body shown as unchanged)
        self.rows = [x for x in doc2.get("rows", []) if x.get("row") == "answer"]
        return raw, said


def steps_1_to_8(report: Report, sec: str, impl: str, ep: str, p: Any, record: str, box: Path = BLACKBOX,
                 whole: bool = False, py_reader: bool = True) -> None:
    """§16's steps 1 to 8 against one owner. ``whole``: the owner predates
    core 0.110, so each answer §16 expects as a held stamp is the member's
    snapshot, which the reader accepts. ``py_reader``: P's second GET is
    zk2py's conditional read, whose verdict is checked beside the bytes."""
    g = Ground(impl, ep, record, box)
    owners = [g]
    try:
        g.put(b"v1")
        g.put_status()
        time.sleep(0.6)

        def answer(sel: str, stamp: InBand | None = None, payload: bytes | None = None,
                   encoding: str = zk2c.ENCODING) -> tuple[Any, Any]:
            return p.ask(sel, stamp, payload, encoding)

        def verdict(reading: Any, want: str) -> bool:
            """zk2py's reader on the same GET: ``unchanged``, or a snapshot
            merged and no held stamp."""
            if not py_reader:
                return True
            if want == "unchanged":
                return reading.unchanged and not reading.snapshots
            return not reading.held and len(reading.snapshots) >= 1 and all(h.done for h in reading.snapshots)

        def unchanged(n: str, what: str, stamp: InBand) -> None:
            """An Expected that is the held stamp alone, or, for an owner
            built before 0.110, the member's snapshot at that stamp."""
            raw, reading = answer(PLAN, stamp)
            kind, got, _ = _entry(raw)
            if whole:
                report.check(sec, f"expected {n}: {what}: an owner built before 0.110 answers whole, the entry at the "
                                  "stamp sent, and the reader accepts the snapshot",
                             kind in ("live", "tomb") and held.same(got, stamp) and verdict(reading, "whole"),
                             f"{kind} {got}")
                return
            report.check(sec, f"expected {n}: {what}: one reply on the plan's key under zenoh/bytes, its payload the "
                              f"{held.size(stamp.width)} bytes sent, 0xe0, the run and the seq; no snapshot",
                         kind == "held" and len(raw) == 1 and raw[0][1] == held.write(stamp)
                         and raw[0][1][0] == 0xE0 and verdict(reading, "unchanged"),
                         f"{kind} {raw[0][1].hex() if raw else None}" + (f"; the tool: {p.said}" if not py_reader
                                                                         else ""))
            if not py_reader and not whole and getattr(p, "rows", None) is not None and not getattr(p, "old", False):
                # observer 0.36 §4.1 (F-572): "A held stamp that is the
                # payload its GET carried, on its one concrete key, is the
                # owner's answer. The tool shows it as unchanged: the
                # member's last mutation is that stamp. It never shows a
                # value."
                rows = p.rows
                report.check(sec, f"expected {n}: zenctl get, whose body was that held stamp, shows the reply as "
                                  "unchanged at the stamp's run and seq, and shows no value (observer §4.1, 0.36)",
                             len(rows) == 1 and rows[0].get("unchanged") == {"run": format(stamp.run, "04x"),
                                                                             "seq": stamp.seq}
                             and "value" not in rows[0] and "decode_error" not in rows[0],
                             json.dumps(rows)[:400])

        # -- step 1: plain -------------------------------------------------------------
        raw, reading = answer(PLAN)
        kind, s1, rec = _entry(raw)
        report.check(sec, "expected 1: one snapshot on the plan's key, one page whose one entry has the empty id, seq "
                          "s1 and v1's record",
                     kind == "live" and rec == g.record_of(b"v1") and s1 is not None and s1.width == 16
                     and verdict(reading, "whole"),
                     f"{kind} {s1} {rec!r}")
        if s1 is None:
            raise RuntimeError("no stamp to go on with")
        if impl == "zk2py":
            mine = g.o._held[PLAN].inband  # noqa: SLF001 - the owner's own record of its put
            report.check(sec, "expected 1: s1 is the put's seq, as the owner minted it", held.same(mine, s1),
                         f"{mine} and {s1}")
        # -- step 2: unchanged -----------------------------------------------------------
        unchanged("2", "with (R, s1)", s1)
        # -- step 3: changed ---------------------------------------------------------------
        g.put(b"v2")
        time.sleep(0.4)
        raw, reading = answer(PLAN, s1)
        kind, s2, rec = _entry(raw)
        report.check(sec, "expected 3: with (R, s1) after v2, the plan's snapshot, its entry at s2 with v2's record",
                     kind == "live" and s2 is not None and s2.same_run(s1) and s2.seq > s1.seq
                     and rec == g.record_of(b"v2") and verdict(reading, "whole"),
                     f"{kind} {s2} {rec!r}")
        assert s2 is not None
        unchanged("3", "with (R, s2)", s2)
        # -- step 4: another stamp ---------------------------------------------------------
        others = {"a run other than R at seq s2": InBand(16, (s2.run + 1) % 65536, s2.seq),
                  "R's value as a 32-bit run at seq s2": InBand(32, s2.run, s2.seq),
                  "(R, s2 + 1)": InBand(16, s2.run, s2.seq + 1)}
        got4 = {}
        for name, st in others.items():
            raw, reading = answer(PLAN, st)
            kind, e, _ = _entry(raw)
            got4[name] = (kind, str(e), verdict(reading, "whole"))
        report.check(sec, "expected 4: each GET draws the plan's snapshot, its entry at s2: another run, another "
                          "width and another seq each name another mutation",
                     all(k == "live" and e == str(s2) and v for k, e, v in got4.values()), str(got4))
        # -- step 5: no held stamp ---------------------------------------------------------
        hs = held.write(s2)
        payloads: dict[str, tuple[bytes, str]] = {
            "the held stamp and one byte after it": (hs + b"\x00", zk2c.ENCODING),
            "the held stamp with tag 5 in its CH": (bytes([hs[0] | 5]) + hs[1:], zk2c.ENCODING),
            "a mutation's envelope under (R, s2), with no record": (zk2c.mutation(s2, b""), zk2c.ENCODING),
            "the first 5 bytes of the held stamp": (hs[:5], zk2c.ENCODING),
            "the held stamp under application/json": (hs, "application/json"),
            "a payload of no byte": (b"", zk2c.ENCODING),
        }
        want5 = {"the held stamp and one byte after it": "trailing", "the held stamp with tag 5 in its CH": "tag",
                 "a mutation's envelope under (R, s2), with no record": "sample_kind",
                 "the first 5 bytes of the held stamp": "short", "the held stamp under application/json": "encoding",
                 "a payload of no byte": None}
        got5, skipped = {}, []
        for name, (payload, enc) in payloads.items():
            try:
                raw, reading = answer(PLAN, None, payload, enc)
            except NotImplementedError:
                skipped.append(name)
                continue
            kind, e, _ = _entry(raw)
            got5[name] = (kind, str(e), verdict(reading, "whole"), held.get_token(enc, payload))
        report.check(sec, f"expected 5: each of the {len(got5)} GETs draws the plan's snapshot, its entry at s2; none "
                          "is refused, and none is left unanswered"
                          + (f" (not sent by this reader: {skipped})" if skipped else ""),
                     all(k == "live" and e == str(s2) and v and t == want5[n] for n, (k, e, v, t) in got5.items()),
                     str(got5))
        if impl == "zk2py":
            seen = [t for _, t in g.o.held_refused]
            report.check(sec, "step 5: zk2py's owner names each refusal, §7.5's token, and answers whole",
                         all(seen.count(t) >= 1 for n, t in want5.items() if t and n in got5), str(seen))
        # -- step 6: other GETs --------------------------------------------------------------
        raw, _ = answer(f"{BASE}/plans/*", s2)
        kind, e, _ = _entry(raw)
        ok_star = kind == "live" and held.same(e, s2)
        raw, _ = answer(f"{BASE}/plans/**", s2)
        pages = [(r[0], _page(r)) for r in raw]
        ok_coll = len(pages) == 1 and pages[0][0] == f"{BASE}/plans" and pages[0][1] is not None \
            and [x["id"] for x in pages[0][1]["live"]] == ["vehicle-01"]
        raw0 = raw_get(p.session, STATUS) if hasattr(p, "session") else p.ask(STATUS)[0]
        st_env = zk2c.parse(raw0[0][1], "put") if len(raw0) == 1 else None
        st_stamp = None if st_env is None else st_env.stamp
        raw, _ = answer(STATUS, st_stamp)
        ok_status = len(raw) == 1 and raw[0][0] == STATUS and raw[0][1] == raw0[0][1] \
            and zk2c.parse(raw[0][1], "put").form == "mutation" and (raw[0][3] or not py_reader)
        report.check(sec, "expected 6: plans/* draws a snapshot of the plan on its own key, plans/** the collection's "
                          "snapshot on …/state/plans, and status its per-key reply, the put's bytes with their Zenoh "
                          "timestamp; no held stamp answers any of the three",
                     ok_star and ok_coll and ok_status,
                     f"plans/* {ok_star}; plans/** {ok_coll} {[k for k, _ in pages]}; status {ok_status} "
                     f"{[(r[0][len(BASE):], r[1].hex(), r[3]) for r in raw]}")
        # -- step 7: a delete -------------------------------------------------------------------
        g.delete()
        t_del = time.monotonic()
        time.sleep(0.3)
        raw, reading = answer(PLAN, s2)
        kind, s3, _ = _entry(raw)
        report.check(sec, "expected 7: with (R, s2) within W, a snapshot whose one entry is a tombstone at s3",
                     kind == "tomb" and s3 is not None and s3.same_run(s2) and s3.seq > s2.seq
                     and verdict(reading, "whole"), f"{kind} {s3}")
        assert s3 is not None
        unchanged("7", "with (R, s3), within W", s3)
        time.sleep(max(0.0, g.window_s + 1.0 - (time.monotonic() - t_del)))
        raw, reading = answer(PLAN, s3)
        kind, _, _ = _entry(raw)
        report.check(sec, f"expected 7: with (R, s3) after W ({g.window_s:g} s), a snapshot of no entry",
                     kind == "none" and verdict(reading, "whole"), kind)
        # -- step 8: a restart --------------------------------------------------------------------
        g.put(b"v3")
        time.sleep(0.4)
        raw, _ = answer(PLAN)
        kind, s4, rec = _entry(raw)
        report.check(sec, "expected 8: the plain GET draws v3 at s4",
                     kind == "live" and rec == g.record_of(b"v3") and s4 is not None and s4.same_run(s3)
                     and s4.seq > s3.seq, f"{kind} {s4} {rec!r}")
        assert s4 is not None
        g.close()
        owners.remove(g)
        time.sleep(0.5)
        g2 = Ground(impl, ep, record, box)
        owners.append(g2)
        g2.put(b"v3")
        time.sleep(0.6)
        raw, reading = answer(PLAN, s4)
        kind, t1, rec = _entry(raw)
        page = _page(raw[0]) if len(raw) == 1 else None
        report.check(sec, "expected 8: with (R, s4), the second instance answers a snapshot under R2, whose entry is "
                          "v3 at t1: the stamp sent names another run",
                     kind == "live" and t1 is not None and not t1.same_run(s4) and rec == g2.record_of(b"v3")
                     and page is not None and (page["run_width"], page["run"]) == (t1.width, t1.run)
                     and verdict(reading, "whole"), f"{kind} {t1} {rec!r}")
        assert t1 is not None
        g2.close()
        owners.remove(g2)
        time.sleep(0.5)
        g3 = Ground(impl, ep, record, box)
        owners.append(g3)
        time.sleep(0.6)
        raw, reading = answer(PLAN, t1)
        kind, r3, _ = _entry(raw)
        report.check(sec, "expected 8: with (R2, t1), the third instance, which put nothing, answers a snapshot of no "
                          "entry under R3",
                     kind == "none" and r3 is not None and not r3.same_run(t1) and not r3.same_run(s4)
                     and verdict(reading, "whole"), f"{kind} {r3}")
    finally:
        for o in owners:
            o.close()


def readers_merge(report: Report, sec: str, impl: str, ep: str, session: Any, record: str, box: Path = BLACKBOX,
                  whole: bool = False) -> None:
    """A reader's merge of each answer, on a bus: zk2py's late joiner with
    no subscription holds the plan from a plain GET, then reads it
    conditionally after each of the owner's acts."""
    g = Ground(impl, ep, record, box)
    lj = None
    try:
        g.put(b"v1")
        time.sleep(0.6)
        lj = sd.LateJoiner(session, PLAN, consumer=True, timeout=5.0, order="get",
                           collections=ss.ByDescriptor([g.contract]))
        lj.start()
        lj.wait()

        def holds() -> tuple[bool | None, InBand | None, bytes | None]:
            e = lj.view().get(PLAN)
            return (None, None, None) if e is None else (e.deleted, e.inband, e.payload)

        first = holds()
        got = lj.conditional_read(PLAN)
        last = [a for a in lj.applied if a.key == PLAN][-1]
        want = "whole" if whole else "unchanged"
        report.check(sec, f"the reader holds v1, reads it with the stamp it holds, and is answered {want}: it keeps "
                          "what it holds" + (", the entry the mutation it held, kept once" if whole else
                                             ", and counts nothing undecodable"),
                     got == want and holds() == first and first[1] is not None and not lj.undecodable
                     and last.outcome == ("same" if whole else "unchanged")
                     and lj.held_asked[-1] == (PLAN, first[1]),
                     f"{got}; holds {holds()[1]}; last {last.outcome}; undecodable {lj.undecodable}")
        g.put(b"v2")
        time.sleep(0.4)
        got = lj.conditional_read(PLAN)
        second = holds()
        report.check(sec, "after the owner's put it reads with the stamp of v1 and is answered the snapshot, which it "
                          "merges as any member read's: it holds v2",
                     got == "whole" and second[2] == g.record_of(b"v2") and second[1] is not None
                     and first[1] is not None and second[1].seq > first[1].seq, f"{got}; holds {second[1]}")
        got = lj.conditional_read(PLAN)
        report.check(sec, f"it reads with the stamp of v2, the snapshot entry's own, and is answered {want}",
                     got == want and holds() == second and lj.held_asked[-1] == (PLAN, second[1]),
                     f"{got}; sent {lj.held_asked[-1][1]}")
        g.delete()
        t_del = time.monotonic()
        time.sleep(0.3)
        got = lj.conditional_read(PLAN)
        third = holds()
        report.check(sec, "after the delete it is answered a snapshot whose tombstone it merges as a delete",
                     got == "whole" and third[0] is True and third[1] is not None and second[1] is not None
                     and third[1].seq > second[1].seq, f"{got}; holds {third[:2]}")
        got = lj.conditional_read(PLAN)
        report.check(sec, f"within W it reads with the deletion's stamp, the tombstone's, and is answered {want}: the "
                          "member is still deleted",
                     got == want and holds() == third and lj.held_asked[-1] == (PLAN, third[1]), f"{got}")
        time.sleep(max(0.0, g.window_s + 1.0 - (time.monotonic() - t_del)))
        got = lj.conditional_read(PLAN)
        report.check(sec, "after W it is answered a snapshot of no entry, the owner's word: the omission rule drops the "
                          "deletion it held, and it holds nothing",
                     got == "whole" and holds() == (None, None, None) and (PLAN, PLAN) in lj.omitted,
                     f"{got}; holds {holds()[:2]}; omitted {lj.omitted}")
        n = len(lj.held_asked)
        got = lj.conditional_read(PLAN)
        report.check(sec, "holding nothing for the member, it sends no held stamp",
                     got == "none" and len(lj.held_asked) == n, f"{got}; {len(lj.held_asked) - n} sent")
    finally:
        if lj is not None:
            lj.close()
        g.close()


class Played:
    """A session of the test's own playing the owner (§16 steps 9 and 10): a
    queryable on the plan's key that answers each GET with the next payload
    it was given, and keeps what each GET carried."""

    def __init__(self, session: Any) -> None:
        import zenoh

        self.lock = threading.Lock()
        #: each GET's answer in turn: one payload, or a list of them, the
        #: replies of two instances to the one GET, in that order
        self.next: list[Any] = []
        self.carried: list[tuple[bytes | None, str | None]] = []
        self.q = session.declare_queryable(PLAN, zenoh.handlers.Callback(self._take), complete=False)

    def _take(self, query: Any) -> None:
        with self.lock:
            self.carried.append((None if query.payload is None else query.payload.to_bytes(),
                                 None if query.encoding is None else str(query.encoding)))
            reply = self.next.pop(0) if self.next else None
        for one in ([] if reply is None else reply if isinstance(reply, list) else [reply]):
            query.reply(PLAN, one, encoding=zk2c.ENCODING)

    def then(self, *payloads: Any) -> None:
        with self.lock:
            self.next += list(payloads)

    def close(self) -> None:
        try:
            self.q.undeclare()
        except Exception:  # noqa: BLE001 - closing anyway
            pass


def _member_snapshot(stamp: InBand, record: bytes | None) -> bytes:
    """A member read's snapshot as an owner writes it: one page on the
    member's key, whose one entry has the empty id; no entry for None."""
    from . import snapshot

    live = [] if record is None else [{"id": "", "run_width": None, "run": None, "seq": stamp.seq,
                                       "confirmation": "none", "record": record.hex()}]
    snap = {"tag": 0, "run_width": stamp.width, "run": stamp.run, "asof": stamp.seq, "time": None, "coll": 0,
            "live": live, "tombs": []}
    pages = snapshot.cut(snap, 1024)
    assert len(pages) == 1
    return pages[0]


def steps_9_and_10(report: Report, sec: str, ep: str, consumer: bool = True) -> None:
    """§16's steps 9 and 10: zk2py's consumer against a played owner, or,
    with ``consumer`` false, a tool's late joiner, bound by the same three
    rules ("A consumer or a tool")."""
    from . import live

    if not consumer:
        sec += ", a tool"

    owner_s, cons_s = live.open_client(ep), live.open_client(ep)
    played = Played(owner_s)
    lj = None
    try:
        time.sleep(0.4)
        r7, r9, r8 = InBand(16, 0x0102, 7), InBand(16, 0x0102, 9), InBand(16, 0x0102, 8)
        lj = sd.LateJoiner(cons_s, PLAN, consumer=consumer, timeout=3.0,
                           collections=ss.collections_from(_contract(), f"zk2/{ADDRESS}/{IFACE}"))
        played.then(_member_snapshot(r7, b"v1"))
        lj.start()
        lj.wait()

        def holds() -> tuple[InBand | None, bytes | None]:
            e = lj.view().get(PLAN)
            return (None, None) if e is None else (e.inband, e.payload)

        seeded = holds()
        # -- step 9 ----------------------------------------------------------------------
        played.then(held.write(r7))
        got1 = lj.conditional_read(PLAN)
        report.check(sec, "expected 9: after the first read, answered the held stamp (R, 7), the consumer holds v1 at "
                          "(R, 7), and counts nothing undecodable",
                     seeded == (r7, b"v1") and got1 == "unchanged" and holds() == (r7, b"v1") and not lj.undecodable
                     and played.carried[-1] == (held.write(r7), zk2c.ENCODING),
                     f"{got1}; holds {holds()}; the GET carried {played.carried[-1]}")
        played.then(_member_snapshot(r7, b"v1"))
        got2 = lj.conditional_read(PLAN)
        same = [a.outcome for a in lj.applied if a.key == PLAN][-1]
        report.check(sec, "expected 9: after the second, answered the plan's snapshot, v1 at seq 7, it holds v1 at "
                          "(R, 7) once: the entry is the mutation it held",
                     got2 == "whole" and holds() == (r7, b"v1") and same == "same", f"{got2}; {same}; holds {holds()}")
        played.then(_member_snapshot(r9, b"v2"))
        got3 = lj.conditional_read(PLAN)
        report.check(sec, "expected 9: after the third, answered the snapshot v2 at seq 9, it holds v2 at (R, 9)",
                     got3 == "whole" and holds() == (r9, b"v2") and played.carried[-1][0] == held.write(r7),
                     f"{got3}; holds {holds()}")
        # -- step 10 ---------------------------------------------------------------------
        played.then(held.write(r8))
        got4 = lj.conditional_read(PLAN)
        und1 = list(lj.undecodable)
        played.then(held.write(r9))
        lj.get(PLAN)
        lj.wait()
        und2 = list(lj.undecodable)
        report.check(sec, "expected 10: the consumer counts each held stamp undecodable, refused as `held`: (R, 8) in "
                          "answer to (R, 9), and (R, 9) in answer to a GET with no payload; it merges nothing for "
                          "either, and holds v2 at (R, 9)",
                     got4 == "refused" and und1 == [(PLAN, "reply", "held")] and und2 == [(PLAN, "reply", "held")] * 2
                     and holds() == (r9, b"v2") and played.carried[-2] == (held.write(r9), zk2c.ENCODING)
                     and played.carried[-1] == (None, None) and len(lj.unchanged) == 1,
                     f"{got4}; undecodable {und2}; holds {holds()}; carried {played.carried[-2:]}")
        # -- beyond §16: on a subscription, and the answer after a later value -----------
        owner_s.put(PLAN, held.write(r9), encoding=zk2c.ENCODING)
        time.sleep(0.4)
        report.check(sec, "a held stamp heard on the subscription, though it names the mutation held, is no owner's "
                          "answer: undecodable, `held`",
                     lj.undecodable[-1] == (PLAN, "live", "held") and holds() == (r9, b"v2"),
                     str(lj.undecodable[-1]))
        # -- two instances answer the one GET (seed/held.json's two orders, live) ---------
        r2 = InBand(16, 0x0103, 0)
        none2 = _member_snapshot(r2, None)
        played.then([held.write(r9), none2])
        got5 = lj.conditional_read(PLAN)
        report.check(sec, "two instances answer, unchanged first: the mutation counts as heard again by the GET, so the "
                          "other run's snapshot of no entry spares it",
                     got5 == "unchanged" and holds() == (r9, b"v2") and lj.unchanged[-1] == (PLAN, r9, "held"),
                     f"{got5}; holds {holds()}; omitted {lj.omitted}")
        played.then([none2, held.write(r9)], _member_snapshot(r9, b"v2"))
        dropped = len(lj.omitted)
        got6 = lj.conditional_read(PLAN)
        report.check(sec, "the same two answers in the other order: the snapshot of no entry drops the value, held from "
                          "before the GET; the answer then finds nothing held, and the reader reads again, with no held "
                          "stamp",
                     got6 == "again" and len(lj.omitted) == dropped + 1 and lj.unchanged[-1] == (PLAN, r9, "nothing")
                     and played.carried[-2] == (held.write(r9), zk2c.ENCODING) and played.carried[-1] == (None, None)
                     and holds() == (r9, b"v2"),
                     f"{got6}; unchanged {lj.unchanged[-1][2]}; carried {played.carried[-2:]}; holds {holds()}")
    finally:
        if lj is not None:
            lj.close()
        played.close()
        owner_s.close()
        cons_s.close()


def tool_reads_played(report: Report, sec: str, ep: str, box: Path, tmp: str, old: bool = False) -> None:
    """Steps 9 and 10's held stamps played to ``zenctl get``: what the tool
    says of each. An observation of the reference, judged on one thing: it
    shows no value for a held stamp."""
    from . import live

    owner_s = live.open_client(ep)
    played = Played(owner_s)
    z = ZenctlP(ep, box, tmp)
    try:
        time.sleep(0.4)
        r9, r8 = InBand(16, 0x0102, 9), InBand(16, 0x0102, 8)
        said = {}
        rows: dict[str, Any] = {}
        for name, sent, back in (("(R, 9) in answer to (R, 9)", r9, r9), ("(R, 8) in answer to (R, 9)", r9, r8),
                                 ("(R, 9) in answer to no payload", None, r9)):
            played.then(held.write(back), held.write(back))  # the tool GETs twice: raw, then decoded
            raw, _ = z.ask(PLAN, sent)
            said[name] = (raw[0][1].hex() if raw else None, z.said)
            rows[name] = z.rows
        report.note(sec, f"zenctl get, a held stamp in answer: {said}")
        report.note(sec, f"zenctl get's decoded rows: {json.dumps(rows, sort_keys=True)[:1500]}")
        if old:
            # §7.5's `short`, in the tool's words: "the body ends before a
            # byte a field needs"
            report.check(sec, "a reader built before 0.110 reads a held stamp as a page, a body cut short, and merges "
                              "nothing (§7.5, \"Read as a page\")",
                         all(h is not None and any("short" in s or "body ends before a byte a field needs" in s
                                                   for s in ss_) for h, ss_ in said.values()), str(said))
        else:
            # observer 0.36 §4.1 (F-572): the held stamp its body carried is
            # shown as unchanged; "Any other held stamp is no answer (held)"
            own = rows["(R, 9) in answer to (R, 9)"]
            others = [r for k, rs in rows.items() if k != "(R, 9) in answer to (R, 9)" for r in rs]
            report.check(sec, "zenctl get shows the held stamp its own body carried as unchanged at that run and seq, "
                              "with no value; each other held stamp as a held stamp, no answer: held; never as a page "
                              "cut short (observer §4.1, 0.36)",
                         len(own) == 1 and own[0].get("unchanged") == {"run": "0102", "seq": 9} and "value" not in own[0]
                         and len(others) == 2 and all("no answer here: held" in str(r.get("decode_error"))
                                                      and "unchanged" not in r for r in others),
                         str(said))
    finally:
        played.close()
        owner_s.close()


def run(report: Report, pairing: str, old: Path) -> None:
    from . import live
    from .live_interop import _r1

    sec = f"{pairing} §16"
    r, ep, _ = _r1(timestamping=False)
    sessions: list[Any] = []
    try:
        with workdir("zk2py-cond-") as tmp:
            def record(name: str) -> str:
                return str(Path(tmp) / f"run-{name}.json")

            if pairing == "py-py":
                s = live.open_client(ep)
                sessions.append(s)
                steps_1_to_8(report, sec, "zk2py", ep, P(s), record("a"))
                readers_merge(report, sec, "zk2py", ep, s, record("b"))
                steps_9_and_10(report, sec, ep)
                steps_9_and_10(report, sec, ep, consumer=False)
            elif pairing == "rust-py":
                s = live.open_client(ep)
                sessions.append(s)
                steps_1_to_8(report, sec, "rust", ep, P(s), record("a"))
                readers_merge(report, sec, "rust", ep, s, record("b"))
            elif pairing == "py-rust":
                steps_1_to_8(report, sec, "zk2py", ep, ZenctlP(ep, BLACKBOX, tmp), record("a"), py_reader=False)
                tool_reads_played(report, sec, ep, BLACKBOX, tmp)
            else:
                if not (old / "owner").is_file():
                    report.check(sec, f"the older black boxes are at {old}", False, "no owner there")
                    return
                s = live.open_client(ep)
                sessions.append(s)
                steps_1_to_8(report, sec, "rust", ep, P(s), record("a"), box=old, whole=True)
                readers_merge(report, sec, "rust", ep, s, record("b"), box=old, whole=True)
                tool_reads_played(report, sec, ep, old, tmp, old=True)
    except Exception as e:  # noqa: BLE001 - a step that could not run is a failure
        import traceback

        report.check(sec, "the section ran", False, f"{type(e).__name__}: {e} {traceback.format_exc()[-900:]}")
    finally:
        for s in sessions:
            s.close()
        r.close()


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="python -m zk2py.conditional_live", description=__doc__.split("\n")[0])
    ap.add_argument("--pairing", action="append", choices=PAIRINGS)
    ap.add_argument("--old", type=Path, default=OLD_DEFAULT,
                    help="the black boxes built before core 0.110 (default: 547d202a beside the current ones)")
    args = ap.parse_args(argv)
    report = Report()
    for p in args.pairing or list(PAIRINGS):
        run(report, p, args.old)
    failed = [r for r in report.rows if not r[2]]
    print(f"{len(report.rows) - len(failed)} passed, {len(failed)} failed", flush=True)
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())

"""``scenarios/state.md`` §4, §5 and §6, live: the Rust ``archive`` black box
(an ``archive.v1`` service, core §4.4) recording zk2py's owner, and aligning
from it after a reconnect (#977's last done-when row; #609, round 107).
``python -m zk2py.archive_live [--only 4|5|6]``.

**The deployment.** Two routers of the runner's own: R1, the ground's, and
R2, the vehicle's, a client of R1 through a :class:`zk2py.budget_scenarios.Link`
that the runner cuts and heals: "The link between the vehicle's router and
the ground's is cut." zk2py's owner ``ground/fleet-mgr`` serves
``zk2py_ground.v1`` (``interop/state/zk2py_ground.v1.toml``: ``plans/{vehicle}``
and §1's ``status``, a per-key resource) as a client of R1, its tombstone
window W the default 60 s. The archive ``vehicle-01/archive`` (the black box)
is a client of R2 and records ``plans/vehicle-01`` (and ``status``, for §6).
Where a step has an archive on the owner's side, it is the black box too,
``ground/archive`` on R1, and the vehicle's archive names it ``--peer``: zk2py
runs no archive of its own (README, "What it does not cover").

**What zk2py reads.** The archive's own key, as a consumer reads last-known
state (core S5): :func:`zk2py.live.last_known`, its reply's bytes and its
attachment's type identity and ``confirmed``; the archive's ``held``,
``gone`` and ``aligned`` lines, and its ``last-known`` answers; and what
zk2py's owner answered (:attr:`zk2py.owner.Owner.snapshots_sent`).

**Step 2's refusal.** zk2py's routers run no access control here, so "the
access control refuses the archive's GETs" is played by the owner dropping
every state query unanswered (``Owner.state_hold``): every reply set is
empty, which is what the step's expected rests on.

Exit 0 when every check passes, 1 when any fails.
"""

from __future__ import annotations

import argparse
import json
import queue
import subprocess
import sys
import threading
import time
from pathlib import Path
from typing import Any

from .blackbox import BLACKBOX

REPO = Path(__file__).resolve().parents[3]
GROUND = REPO / "impl" / "python" / "interop" / "state" / "zk2py_ground.v1.toml"
OWNER = "ground/fleet-mgr"
BASE = "zk2/ground/fleet-mgr/zk2py_ground.v1/state"
PLAN = f"{BASE}/plans/vehicle-01"
STATUS = f"{BASE}/status"

Result = tuple[str, bool, str]


class Archive:
    """The ``archive`` black box as a child process, its standard output
    kept line by line with the instant each came."""

    def __init__(self, connect: str, system: str, service: str, records: list[str], peers: list[str] = (),
                 window: int | None = None, flags: list[str] = ()) -> None:
        args = [str(BLACKBOX / "archive"), "--connect", connect, "--system", system, "--service", service,
                *flags]
        if window is not None:
            args += ["--window", str(window)]
        for p in peers:
            args += ["--peer", p]
        for k in records:
            args += ["--record", OWNER, k, str(GROUND)]
        self.address = f"{system}/{service}"
        self.proc = subprocess.Popen(args, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                     text=True)
        self.lines: list[tuple[float, str]] = []
        self.lock = threading.Lock()
        self.new = queue.Queue()
        self.stderr: list[str] = []
        threading.Thread(target=self._pump, daemon=True).start()
        threading.Thread(target=lambda: [self.stderr.append(x) for x in self.proc.stderr], daemon=True).start()

    def _pump(self) -> None:
        for line in self.proc.stdout:
            with self.lock:
                self.lines.append((time.monotonic(), line.rstrip("\n")))
            self.new.put(None)

    def mark(self) -> int:
        with self.lock:
            return len(self.lines)

    def since(self, mark: int) -> list[str]:
        with self.lock:
            return [x for _, x in self.lines[mark:]]

    def wait(self, pred: Any, timeout: float, mark: int = 0) -> str | None:
        """The first line from ``mark`` on that ``pred`` accepts."""
        deadline = time.monotonic() + timeout
        while True:
            for x in self.since(mark):
                if pred(x):
                    return x
            left = deadline - time.monotonic()
            if left <= 0 or self.proc.poll() is not None:
                return None
            try:
                self.new.get(timeout=min(left, 0.2))
            except queue.Empty:
                pass

    def command(self, line: str, word: str, timeout: float = 15.0) -> str | None:
        """One command, and the first line after it whose first word is
        ``word``: "match an answer by its first word … never by its
        position"."""
        m = self.mark()
        self.proc.stdin.write(line + "\n")
        self.proc.stdin.flush()
        return self.wait(lambda x: x.split(" ", 1)[0] == word, timeout, m)

    def close(self) -> None:
        try:
            self.proc.stdin.write("quit\n")
            self.proc.stdin.close()
        except (OSError, ValueError):
            pass
        try:
            self.proc.wait(15)
        except subprocess.TimeoutExpired:
            self.proc.kill()
            self.proc.wait()


class Deployment:
    """R1, R2 behind a link, zk2py's owner on R1, a consumer on R2."""

    def __init__(self) -> None:
        import zenoh

        from . import live
        from .budget_scenarios import Link
        from .contract import load_contract
        from .live_interop import _r1
        from .owner import Owner, free_loopback_port

        self.r1, self.ep1, _ = _r1(timestamping=True)
        self.link = Link(self.ep1)
        port = free_loopback_port()
        conf = zenoh.Config()
        conf.insert_json5("mode", json.dumps("router"))
        conf.insert_json5("listen/endpoints", json.dumps([f"tcp/127.0.0.1:{port}"]))
        conf.insert_json5("connect/endpoints", json.dumps([self.link.endpoint]))
        conf.insert_json5("scouting/multicast/enabled", "false")
        conf.insert_json5("timestamping/enabled", "true")
        self.r2 = zenoh.open(conf)
        self.ep2 = f"tcp/127.0.0.1:{port}"
        self.contract = load_contract(GROUND)
        self.owner: Any = Owner("ground", "fleet-mgr", [self.contract], connect=self.ep1)
        self.owner.start()
        self.c = live.open_client(self.ep2)
        self.archives: list[Archive] = []

    def archive(self, *a: Any, **kw: Any) -> Archive:
        x = Archive(*a, **kw)
        self.archives.append(x)
        if x.wait(lambda s: s.startswith("ready "), 120) is None:
            raise RuntimeError(f"the archive did not start: {x.stderr[-3:]}")
        return x

    def owner_seen(self) -> bool:
        from . import live

        return bool(live.list_presence(self.c, f"zk2/{OWNER}/@zk/instance/*").instances)

    def cut(self) -> bool:
        self.link.cut()
        return _until(lambda: not self.owner_seen(), 20.0)

    def heal(self) -> None:
        self.link.open = True
        self.link.heal()

    def close(self) -> None:
        for x in self.archives:
            x.close()
        try:
            self.owner.close()
        except Exception:  # noqa: BLE001 - closing anyway
            pass
        self.c.close()
        self.link.cut()
        self.r2.close()
        self.r1.close()


def _until(pred: Any, timeout: float, step: float = 0.1) -> bool:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if pred():
            return True
        time.sleep(step)
    return pred()


def _held(origin: str, confirmed: bool | None = None, tombstone: bool | None = None) -> Any:
    def pred(x: str) -> bool:
        parts = x.split()
        if len(parts) < 3 or parts[0] != "held" or parts[1] != origin:
            return False
        if confirmed is not None and parts[2] != f"confirmed={str(confirmed).lower()}":
            return False
        return tombstone is None or (("tombstone" in parts[3:]) == tombstone)
    return pred


def _gone(origin: str) -> Any:
    return lambda x: x == f"gone {origin}"


def _dropped(origin: str) -> Any:
    """The archive's drop of ``origin`` on positive evidence: the deletion it
    keeps within its window (``held … tombstone``), or the key gone."""
    held = _held(origin, None, True)
    return lambda x: held(x) or x == f"gone {origin}"


#: The vehicle's archive reads its owners' presence every 500 ms
#: (``--no-watch``): with its watch session, the default, it re-reads no owner
#: whose instance token comes back after an outage of the link (a deviation
#: of the reference, #1101, reported by :func:`watch_deviation`).
POLL = ["--no-watch"]


def section4_5(only: set[str]) -> list[Result]:
    """§4, then §5 steps 1 to 3, on one deployment."""
    from . import live

    out: list[Result] = []
    d = Deployment()
    try:
        d.owner.set_state(PLAN, b"v3")
        ground_side = d.archive(d.ep1, "ground", "archive", [PLAN]) if "5" in only else None
        a = d.archive(d.ep2, "vehicle-01", "archive", [PLAN], peers=["ground/archive"] if ground_side else [],
                      flags=POLL)
        ok = a.wait(_held(PLAN, True), 20)
        out.append(("setup: the vehicle's archive records plans/vehicle-01 from zk2py's owner, confirmed, aligned by "
                    "a member read of its key",
                    ok is not None and any(x.startswith("aligned from=owner:ground/fleet-mgr") for x in a.since(0)),
                    f"{a.since(0)[:4]}"))
        put_wire = d.owner._held[PLAN].wire  # noqa: SLF001 - the bytes zk2py's owner put
        # -- §4 ------------------------------------------------------------
        if not d.cut():
            return out + [("§4: the link is cut, the owner absent from R2", False, "")]
        st = live.get_state(d.c, PLAN, timeout=2.0)
        lk = live.last_known(d.c, "vehicle-01/archive", PLAN)
        line = a.command(f"last-known {PLAN}", "last-known")
        out.append(("§4: the owner's GET returns no reply within the timeout",
                    st.silent, f"{st.arrivals}, errors {st.errors}"))
        ctype = next(r["type"] for r in d.contract.canonical["resources"] if r["template"] == "plans/{vehicle}")
        out.append(("§4: the consumer reads the archive's key for the plan: v3's bytes as the owner put them, its "
                    "envelope included, its type identity in the attachment (iface, contract, and type, the "
                    "canonical form's type object, core §4.4), and its confirmed, a boolean: core §4.4 has it "
                    "false while alignment has not confirmed the key, and a cut alone does not unconfirm one "
                    "aligned before it, so neither value is judged here",
                    lk is not None and lk.raw == put_wire and lk.attachment.get("iface") == "zk2py_ground.v1"
                    and lk.attachment.get("contract") == d.contract.fingerprint
                    and lk.attachment.get("type") == ctype and isinstance(lk.attachment.get("confirmed"), bool)
                    and lk.record == b"v3",
                    f"{lk and (lk.raw.hex(), lk.attachment)}; owner put {put_wire.hex()}; {line}"))
        out.append(("§4: the consumer presents v3 as last-known, never as current (live.LastKnown.shown)",
                    lk is not None and lk.shown().startswith("last-known") and "current" not in lk.shown(),
                    lk.shown() if lk else ""))
        if "5" not in only:
            d.heal()
            return out
        # -- §5 step 1 -----------------------------------------------------
        sent = len(d.owner.snapshots_sent)
        d.owner.delete_state(PLAN)
        m = a.mark()
        d.heal()
        # review-bot on #1100, 4: the text's expected alone, "The owner's
        # snapshot of the plan holds its tombstone, and the archive drops
        # it": its deletion held within W (``held … tombstone``), never the
        # key gone, and read back as a deletion, never as no reply
        held = a.wait(_held(PLAN, None, True), 30, m)
        reads = [x for x in d.owner.snapshots_sent[sent:] if x[0] == PLAN]
        lk = live.last_known(d.c, "vehicle-01/archive", PLAN)
        line = a.command(f"last-known {PLAN}", "last-known")
        out.append(("§5 step 1: deleted within W while cut, healed within W: the archive re-reads the plan by a member "
                    "read of its key (a snapshot on the plan's own key), whose tombstone it keeps: the plan dropped, "
                    "its deletion held within the window, which zk2py reads back from the archive's key as a "
                    "deletion",
                    held is not None and bool(reads) and lk is not None and lk.deleted and line is not None
                    and line.rstrip().endswith("tombstone") and f"gone {PLAN}" not in a.since(m),
                    f"{a.since(m)}; the owner's member-read snapshots {reads[:3]}; {line}; zk2py reads "
                    f"{lk and ('deleted' if lk.deleted else lk.record)}"))
        # -- §5 step 2 -----------------------------------------------------
        d.owner.set_state(PLAN, b"v4")
        m = a.mark()
        a.wait(_held(PLAN, True), 20, m)
        if not d.cut():
            return out + [("§5 step 2: the link is cut", False, "")]
        d.owner.delete_state(PLAN)
        d.owner.state_hold = lambda q: q.drop()
        m = a.mark()
        d.heal()
        _until(d.owner_seen, 20)
        time.sleep(6.0)
        aligned = a.command("align", "aligned")
        line = a.command(f"last-known {PLAN}", "last-known")
        lk = live.last_known(d.c, "vehicle-01/archive", PLAN)
        out.append(("§5 step 2: every reply set empty (the owner drops each GET unanswered), its retries included: "
                    "nothing dropped, and the archive keeps serving the plan with confirmed: false",
                    not any(x == f"gone {PLAN}" for x in a.since(m)) and line is not None
                    and "confirmed=false" in line and lk is not None and lk.attachment.get("confirmed") is False
                    and lk.record == b"v4",
                    f"{aligned}; {line}; {lk and lk.attachment}"))
        d.owner.state_hold = None
        m = a.mark()
        aligned = a.command("align", "aligned")
        out.append(("§5 step 2, the owner answering again within W: the next alignment drops the plan on its "
                    "tombstone",
                    aligned is not None and "dropped=1" in aligned and a.wait(_dropped(PLAN), 10, m) is not None,
                    f"{aligned}; {a.since(m)}"))
        # -- §5 step 3 -----------------------------------------------------
        d.owner.set_state(PLAN, b"v5")
        m = a.mark()
        a.wait(_held(PLAN, True), 20, m)
        gm = ground_side.mark()
        ground_side.wait(_held(PLAN, True), 20, 0)
        if not d.cut():
            return out + [("§5 step 3: the link is cut", False, "")]
        d.owner.delete_state(PLAN)
        tomb = ground_side.wait(_dropped(PLAN), 15, gm)
        d.owner.close()
        time.sleep(1.0)
        m = a.mark()
        d.heal()
        gone = a.wait(_dropped(PLAN), 30, m)
        out.append(("§5 step 3: the owner stopped behind the cut; at the heal the vehicle's archive aligns from the "
                    "owner-side archive through the collection's archive form, whose reply_del is positive "
                    "evidence, and drops the plan",
                    tomb is not None and gone is not None and not d.owner_seen(),
                    f"owner-side {tomb}; vehicle {a.since(m)}"))
    except Exception as e:  # noqa: BLE001 - a run that could not finish is a failure
        out.append(("§4/§5: the run finished", False, f"{type(e).__name__}: {e}"))
    finally:
        d.close()
    return out


def section6(step: int) -> list[Result]:
    """§6, one step: the link cut for longer than 60 s, the owner deleting
    the plan and ``status`` while it is cut."""
    out: list[Result] = []
    d = Deployment()
    tag = f"§6 step {step}"
    try:
        d.owner.set_state(PLAN, b"p1")
        d.owner.set_state(STATUS, b"s1")
        peer = None
        if step >= 2:
            peer = d.archive(d.ep1, "ground", "archive", [PLAN, STATUS], window=60 if step == 2 else 120)
        a = d.archive(d.ep2, "vehicle-01", "archive", [PLAN, STATUS], peers=["ground/archive"] if peer else [],
                      window=60 if step < 3 else 120, flags=POLL)
        a.wait(_held(PLAN, True), 20)
        a.wait(_held(STATUS, True), 20)
        if peer is not None:
            peer.wait(_held(PLAN, True), 20)
            peer.wait(_held(STATUS, True), 20)
        if not d.cut():
            return [(f"{tag}: the link is cut", False, "")]
        d.owner.delete_state(PLAN)
        d.owner.delete_state(STATUS)
        if step >= 2:
            time.sleep(2.0)
            d.owner.close()
        time.sleep(66.0)
        m = a.mark()
        d.heal()
        time.sleep(10.0)
        lines = a.since(m)
        plan = a.command(f"last-known {PLAN}", "last-known")
        status = a.command(f"last-known {STATUS}", "last-known")
        if step == 1:
            out.append((f"{tag}: W 60 s, the owner back at the heal: its snapshot of no entry on the plan's key is "
                        "positive evidence, and the archive drops the plan; status's tombstone is gone, so the archive "
                        "keeps serving it, confirmed: false",
                        any(_dropped(PLAN)(x) for x in lines) and plan is not None
                        and (plan.endswith(" none") or plan.rstrip().endswith("tombstone"))
                        and status is not None and "confirmed=false" in status and "tombstone" not in status
                        and f"gone {STATUS}" not in lines,
                        f"{lines}; {plan}; {status}"))
        elif step == 2:
            out.append((f"{tag}: W 60 s, the owner stopped before the heal: the owner-side archive answers per key, "
                        "its tombstones gone with W, so the vehicle's archive keeps serving the plan and status, "
                        "confirmed: false",
                        plan is not None and "confirmed=false" in plan and status is not None
                        and "confirmed=false" in status and not any(x.startswith("gone ") for x in lines),
                        f"{lines}; {plan}; {status}"))
        else:
            out.append((f"{tag}: W 120 s, the face's maximum outage, at the owner-side archive: it answers each with "
                        "its reply_del, and the vehicle's archive drops both",
                        any(_dropped(PLAN)(x) for x in lines) and any(_dropped(STATUS)(x) for x in lines),
                        f"{lines}; {plan}; {status}"))
    except Exception as e:  # noqa: BLE001 - a run that could not finish is a failure
        out.append((f"{tag}: the run finished", False, f"{type(e).__name__}: {e}"))
    finally:
        d.close()
    return out


def watch_deviation() -> list[Result]:
    """§5 step 1 with the vehicle's archive on its default watch session:
    reported, never failed. Core §4.4: "When an owner becomes reachable
    again, the archive MUST re-read the owner's recorded collection …
    Reachable is judged by presence: the owner's instance token". A
    liveliness subscriber on R2 hears the owner's token deleted at the cut
    and put again at the heal; the black box's watch session re-reads
    nothing then."""
    d = Deployment()
    try:
        d.owner.set_state(PLAN, b"w1")
        a = d.archive(d.ep2, "vehicle-01", "archive", [PLAN])
        a.wait(_held(PLAN, True), 20)
        time.sleep(2.0)
        d.cut()
        d.owner.delete_state(PLAN)
        time.sleep(5.0)
        m = a.mark()
        sent = len(d.owner.snapshots_sent)
        d.heal()
        seen = _until(d.owner_seen, 30)
        dropped = a.wait(_dropped(PLAN), 30, m)
        reads = [x for x in d.owner.snapshots_sent[sent:] if x[0] == PLAN]
        return [("DEVIATION", dropped is not None,
                 f"§5 step 1 under the archive's watch session (its default): the owner seen again "
                 f"{seen}; the archive re-read {len(reads)} time(s) and printed {a.since(m)} within 30 s "
                 "(re-read and dropped under --no-watch; #1101)")]
    finally:
        d.close()


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="python -m zk2py.archive_live", description=__doc__.split("\n")[0])
    ap.add_argument("--only", action="append", choices=("4", "5", "6"))
    args = ap.parse_args(argv)
    only = set(args.only or ["4", "5", "6"])
    results: list[Result] = []
    if only & {"4", "5"}:
        results += section4_5(only)
    if "6" in only:
        for step in (1, 2, 3):
            results += section6(step)
    failed = 0
    for what, ok, detail in results:
        print(f"{'PASS' if ok else 'FAIL'} {what}" + (f": {detail}" if detail else ""), flush=True)
        failed += not ok
    if "5" in only:
        for _, ok, detail in watch_deviation():
            print(f"{'NOTE' if ok else 'DEVIATION'} the reference's archive: {detail}", flush=True)
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())

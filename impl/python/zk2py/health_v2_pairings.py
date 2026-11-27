"""``health.v2``'s scenarios (``spec/profiles/health/scenarios-v2.md``, text
0.2) between zk2py and the reference, live (#609, round 123)::

    python -m zk2py.health_v2_scenarios --owner rust [--only 1|…|9|epoch|epoch-oh]...
    python -m zk2py.health_v2_pairings --owner zk2py|rust [--only 1|…|9|epoch|epoch-oh]...

The black boxes are ``owner``, ``archive`` and ``zenctl`` of ``bin/current``,
run and never read; their words are their ``--help``'s. Every router is
zenoh-python's.

**The two cross pairings.**
- **The reference's owner, zk2py's reader:** ``health_v2_scenarios --owner
  rust``. Each section of that runner starts the owner example
  (:class:`RustProc`, ``owner --health-v2``) where it started zk2py's owner
  process, and reads it with zk2py's S, G and T, unchanged.
- **zk2py's owner, the reference's reader:** this module, ``--owner zk2py``.
  Each section starts zk2py's owners as ``health_v2_scenarios`` does, and the
  tool of the section is ``zenctl health``, beside the doctor's ``health-*``
  checks and ``check conform``'s ``health`` cases where a section's state is
  theirs to judge. ``--owner rust`` runs the same against the owner example:
  the reference against itself, as a check of this runner.

**What the owner example cannot play,** by its usage: a test owner that
breaks §2.2 (``lab/liar``, §7: ``status`` and ``check`` answer
``raised_by=<check>``), one that answers two snapshots at two asofs
(``lab/two``), and 100 services in one process (§5 starts 100 processes).
``lab/liar`` and ``lab/two`` are zk2py's in every pairing, and named so.

Exit 0 when every check passes, 1 when any fails.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
import threading
import time
from pathlib import Path
from typing import Any

from .blackbox import BLACKBOX

REPO = Path(__file__).resolve().parents[3]
ZENCTL = BLACKBOX / "zenctl"
ANSI = re.compile(r"\x1b\[[0-9;]*m")


class CannotPlay(RuntimeError):
    """A part the owner example's usage has no word for."""


# -- the owner example, in zk2py's owner process's place ----------------------------------

class RustProc:
    """The reference's owner example (``owner --health-v2``), driven by the
    words of :class:`zk2py.health_v2_scenarios.Proc`: each flag and command
    of zk2py's owner process is said in the example's usage.

    - ``--level L --reason R`` is ``--health-status L:R``, ``--check
      n=l:d`` ``--health-check n=l:d``, the levels upper-cased as its usage
      spells them;
    - ``--run-record`` is ``--run-file``, ``--reference`` and
      ``--offset-ms`` ``--clock-reference`` and ``--clock-offset-ms``,
      ``--contract f`` a contract file after the address, and ``--catalog``
      and ``--tokenless`` its own; ``--owner-heartbeat`` passes through;
    - ``--many n --nav f`` starts ``<system>/dev0`` … ``dev<n-1>``, each a
      process of its own with ``--health-status OK:serving``;
    - ``--v1`` is ``--health`` in place of ``--health-v2``: ``lab/old`` of
      §1 step 5;
    - ``--liar`` has no word in its usage, and is refused (:class:`CannotPlay`).

    Commands: ``status``, ``check``, ``retire`` and ``fault`` as its usage
    words them; ``clock <ms>`` is ``clock-offset <ms>``; ``unvouch`` and
    ``vouch`` are ``confirm off`` and ``confirm on``; ``flip <ms>`` sets
    ``disk`` to ``FAILED`` and back every ``<ms>``, then puts the status
    ``OK serving``, from this process; ``state`` is what its answers and its
    standard error say: its guard (``clock-guard``), the holds this process
    saw begin, and the words it logged out of range."""

    impl = "rust"

    def __init__(self, endpoint: str, address: str, *flags: str):
        from .live_interop import Owner as Exe

        self.address = address
        args = list(flags)
        out: list[str] = []
        contracts: list[str] = []
        many, nav, v1 = 0, None, False
        level, reason = None, None
        i = 0
        while i < len(args):
            a = args[i]
            nxt = args[i + 1] if i + 1 < len(args) else None
            if a == "--level":
                level, i = nxt, i + 2
            elif a == "--reason":
                reason, i = nxt, i + 2
            elif a == "--check":
                name, _, rest = nxt.partition("=")
                lv, _, detail = rest.partition(":")
                out += ["--health-check", f"{name}={lv.upper()}" + (f":{detail}" if detail else "")]
                i += 2
            elif a == "--run-record":
                out += ["--run-file", nxt]
                i += 2
            elif a == "--reference":
                out += ["--clock-reference", nxt]
                i += 2
            elif a == "--offset-ms":
                out += ["--clock-offset-ms", nxt]
                i += 2
            elif a == "--contract":
                contracts.append(nxt)
                i += 2
            elif a in ("--catalog", "--tokenless"):
                out += [a, nxt]
                i += 2
            elif a == "--owner-heartbeat":
                out.append(a)
                i += 1
            elif a == "--many":
                many, i = int(nxt), i + 2
            elif a == "--nav":
                nav, i = nxt, i + 2
            elif a == "--v1":
                v1, i = True, i + 1
            elif a == "--liar":
                raise CannotPlay("--liar: the owner example's usage has no owner that breaks §2.2; its status and "
                                 "check answer raised_by=<check>")
            elif a == "--auth":
                i += 2          # the face selects the ground by its link protocol (§8), never by a password
            else:
                raise CannotPlay(f"{a}: no word for it in the owner example's usage")
        status = [] if level is None else ["--health-status", level.upper() + (f":{reason}" if reason else "")]
        self.flags = (["--health"] if v1 else ["--health-v2"]) + status + out
        self.procs: list[Any] = []
        system = address.split("/")[0]
        if many:
            if nav is None:
                raise CannotPlay("--many needs --nav")
            for n in range(many):
                self.procs.append(Exe(BLACKBOX / "owner", f"{system}/dev{n}", [nav], connect=endpoint,
                                      flags=["--health-v2", "--health-status", "OK:serving", *out]))
            ready = [p.wait_for("ready ", 120) for p in self.procs]
            if any(r is None for r in ready):
                raise RuntimeError(f"{address}: {sum(r is None for r in ready)} of {many} did not start")
            self.ready = f"ready {many} owners"
            self.p = self.procs[0]
        else:
            self.p = Exe(BLACKBOX / "owner", address, contracts, connect=endpoint, flags=self.flags)
            self.procs.append(self.p)
            got = self.p.wait_for("ready ", 120)
            if got is None:
                raise RuntimeError(f"{address}: the owner example did not start: {self.p.stderr[-3:]}")
            self.ready = "ready " + got.strip()
        #: the owner example's ready line states no run: a reader takes it
        #: from the descriptor (core §3.3)
        self.run: str | None = None
        self._held = False
        self.holds = 0
        self._flip: dict[str, Any] = {}
        self._lock = threading.Lock()

    def _say(self, line: str) -> str:
        with self._lock:
            got = self.p.command(line)
        return "" if got is None else got

    def guard(self) -> bool:
        held = self._say("clock-guard").strip() == "clock-guard held"
        if held and not self._held:
            self.holds += 1
        self._held = held
        return held

    def log(self) -> list[str]:
        """"The words stay in the owner's log": its standard error's
        ``table=… words=…`` lines, as zk2py's process words them."""
        kinds = {"reasons": "reason", "details": "detail", "codes": "code"}
        out = []
        for line in self.p.stderr:
            m = re.search(r'table="(\w+)" words="(.*)"', ANSI.sub("", line))
            if m:
                out.append(f"{kinds.get(m.group(1), m.group(1))}: {m.group(2)}")
        return out

    def cmd(self, line: str) -> str:
        w = line.split()
        if w[0] == "status":
            said = f"status {w[1].upper()}" + (" " + " ".join(w[2:]) if len(w) > 2 else "")
        elif w[0] == "check":
            # health.v2's `unknown` is the usage's UNSPECIFIED: "one it cannot run" (v2 §2.6: UNSPECIFIED reads
            # unknown)
            lv = "UNSPECIFIED" if w[2] == "unknown" else w[2].upper()
            said = f"check {w[1]} {lv}" + (" " + " ".join(w[3:]) if len(w) > 3 else "")
        elif w[0] == "fault":
            said = f"fault {w[1]} {w[2].upper()}" + (f" {w[3]}" if len(w) > 3 else "")
        elif w[0] == "retire":
            said = line
        elif w[0] == "clock":
            self.guard()
            said = f"clock-offset {w[1]}"
        elif w[0] == "unvouch":
            said = "confirm off"
        elif w[0] == "vouch":
            said = "confirm on"
        elif w[0] == "flip":
            return self._flip_cmd(w[1])
        elif w[0] == "state":
            return json.dumps(self.state())
        else:
            return f"refused {line}: unknown command"
        got = self._say(said)
        return got if got.startswith("refused") or not got else f"ok {got}"

    def _flip_cmd(self, arg: str) -> str:
        if arg == "stop":
            self._flip["stop"].set()
            self._flip["thread"].join()
            return f"ok flip stopped after {self._flip['n']}"
        period = int(arg) / 1000
        self._flip.update(stop=threading.Event(), n=0)

        def run() -> None:
            while not self._flip["stop"].wait(period):
                self._say("check disk FAILED timeout")       # the runtime raises the status: raised_by=disk
                self._flip["stop"].wait(period)
                self._say("check disk OK ok")
                self._say("status OK serving")
                self._flip["n"] += 1

        self._flip["thread"] = threading.Thread(target=run, daemon=True)
        self._flip["thread"].start()
        return f"ok flip {arg}"

    def state(self) -> dict[str, Any]:
        return {"ahead": self.guard(), "holds": self.holds, "log": self.log(),
                "events": [ANSI.sub("", x) for x in self.p.stderr[-4:]]}

    def close(self) -> None:
        if self._flip.get("stop") is not None:
            self._flip["stop"].set()
        for p in self.procs:
            p.close()

    def end(self) -> None:
        """The process ends, with no word (scenarios §6 step 2)."""
        for p in self.procs:
            p.proc.kill()
            p.proc.wait(10)


# -- zenctl as the section's tool ---------------------------------------------------------

def zenctl(*argv: str, timeout: float = 240.0) -> tuple[int, Any, str]:
    """``zenctl <argv> --format json``: its exit, its JSON document (None
    where stdout held none), and its standard error's tail."""
    p = subprocess.run([str(ZENCTL), *argv, "--format", "json", "--color", "never"], capture_output=True, text=True,
                       timeout=timeout)
    try:
        doc = json.loads(p.stdout) if p.stdout.strip() else None
    except ValueError:
        doc = {"stdout": p.stdout[-2000:]}
    return p.returncode, doc, p.stderr.strip()[-1500:]


class Background:
    """A ``zenctl`` run started now and read when it ends: a window that
    spans a section's steps ("T and T2 each subscribe … before step 1")."""

    def __init__(self, *argv: str, timeout: float = 240.0):
        self.argv = argv
        self.result: tuple[int, Any, str] | None = None
        self.listening = threading.Event()
        self._t = threading.Thread(target=self._run, args=(timeout,), daemon=True)
        self._t.start()

    def _run(self, timeout: float) -> None:
        p = subprocess.Popen([str(ZENCTL), *self.argv, "--format", "json", "--color", "never"], stdout=subprocess.PIPE,
                             stderr=subprocess.PIPE, text=True)
        err: list[str] = []

        def pump() -> None:
            for line in p.stderr:
                err.append(line)
                if "listening" in line:
                    self.listening.set()

        chunks: list[str] = []
        t = threading.Thread(target=pump, daemon=True)
        t.start()
        t_out = threading.Thread(target=lambda: chunks.append(p.stdout.read()), daemon=True)
        t_out.start()
        try:
            p.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            p.kill()
            p.wait()
        t.join(5)
        t_out.join(5)
        out = "".join(chunks)
        try:
            doc = json.loads(out) if out.strip() else None
        except ValueError:
            doc = {"stdout": out[-2000:]}
        self.result = (p.returncode, doc, "".join(err).strip()[-1500:])

    def wait(self, timeout: float = 300.0) -> tuple[int, Any, str]:
        self._t.join(timeout)
        return self.result if self.result is not None else (-1, None, "did not end")


def row(doc: Any, address: str) -> dict[str, Any]:
    """The row of ``address`` in a ``zenctl health`` document."""
    for r in (doc or {}).get("rows", []) if isinstance(doc, dict) else []:
        if r.get("address") == address:
            return r
    return {}


def verdict(r: dict[str, Any]) -> tuple[Any, Any, Any]:
    return r.get("verdict"), r.get("reason"), r.get("level")


def said(r: dict[str, Any], *keys: str) -> Any:
    x: Any = r
    for k in keys:
        x = x.get(k) if isinstance(x, dict) else None
    return x


def brief(r: dict[str, Any]) -> str:
    """One row in a line, for a check's detail."""
    st = r.get("status") or {}
    return json.dumps({"verdict": verdict(r), "status": [st.get("level"), st.get("reason"), st.get("via"),
                                                          said(st, "word", "read")],
                       "agrees": said(r, "agrees", "answer"), "clock": [said(r, "clock_ahead", "answer"),
                                                                       said(r, "clock_ahead", "reason")],
                       "checks": [[c.get("check"), c.get("level"), c.get("detail")] for c in r.get("checks") or []],
                       "faults": r.get("faults"), "last_fault": r.get("last_fault"), "readings": r.get("readings"),
                       "last_known": r.get("last_known")},
                      default=str)[:1400]


def health(ep: str, *argv: str, timeout: float = 240.0) -> tuple[int, Any, str]:
    return zenctl("health", *argv, "-c", ep, timeout=timeout)


def doctor(ep: str, *argv: str) -> tuple[int, dict[str, Any], str]:
    """The doctor's four ``health-*`` checks: their verdicts by id."""
    code, doc, err = zenctl("doctor", "--check", "health-failed", "--check", "health-degraded", "--check",
                            "health-stale", "--check", "health-inconsistent", *argv, "-c", ep, timeout=120)
    return code, doc if isinstance(doc, dict) else {}, err


def doctor_rows(doc: dict[str, Any]) -> dict[str, Any]:
    """Each ``health-*`` check's answer and the subjects of its findings:
    ``established`` is a finding (its usage: "one verdict per check")."""
    out: dict[str, Any] = {}
    for r in doc.get("rows") or []:
        cid = r.get("check")
        if cid and cid.startswith("health"):
            out[cid] = {"answer": said(r, "verdict", "answer"),
                        "subjects": sorted({f.get("subject") for f in r.get("findings") or []}),
                        "section": r.get("section"), "reason": said(r, "verdict", "reason")}
    return out


def finds(doc: dict[str, Any], check: str) -> list[str]:
    """The subjects ``check`` found, [] when it found none."""
    r = doctor_rows(doc).get(check) or {}
    return r.get("subjects") or [] if r.get("answer") == "established" else []


def conform(ep: str, address: str, *argv: str) -> tuple[int, dict[str, Any], str]:
    """``check conform <address> health.v2``: its cases' verdicts by case,
    the health cases' rows whole."""
    code, doc, err = zenctl("check", "conform", address, "health.v2", *argv, "-c", ep, timeout=120)
    return code, doc if isinstance(doc, dict) else {}, err


def conform_cases(doc: dict[str, Any]) -> dict[str, list[Any]]:
    """Each case's answers: ``established`` is a violation, and a pass is
    ``not_established`` (its rows' verdicts)."""
    out: dict[str, list[Any]] = {}
    for r in doc.get("rows") or []:
        case = r.get("case")
        if case:
            out.setdefault(case, []).append(said(r, "verdict", "answer"))
    return out


PASS, FOUND = "not_established", "established"


HEALTHY = ("healthy", "ok", "ok")


def _hs() -> Any:
    from . import health_v2_scenarios as hs

    return hs


def _named(r: dict[str, Any], key: str = "status") -> Any:
    w = said(r, key, "word") or {}
    return w.get("name") if w.get("name") is not None else w.get("number")


# -- §1 -----------------------------------------------------------------------------------

def zsection1(report: Any) -> None:
    hs = _hs()
    sec = "§1 bring-up, and both majors (zenctl as T)"
    bus = hs.Bus()
    try:
        o = bus.owner("lab/svc", "--run-record", os.path.join(bus.tmp, "svc.run"))
        bus.owner("lab/quiet", "--level", "ok", "--reason", "serving", "--tokenless", "health.v2",
                  "--tokenless", "freshness.v2")
        time.sleep(1.0)
        c1, d1, e1 = health(bus.endpoint, "lab/svc", "--clocks-synced")
        r1 = row(d1, "lab/svc")
        st1 = r1.get("status") or {}
        report.check(sec, "step 1: zenctl health reads lab/svc's status unknown, the reason 1 shown by its "
                          "profile name starting; never ok",
                     st1.get("level") == "unknown" and said(st1, "word", "number") == 1
                     and said(st1, "word", "name") == "starting" and r1.get("verdict") != "healthy",
                     f"exit {c1}; {brief(r1)}; {e1[-200:] if not r1 else ''}")
        c2, d2, e2 = health(bus.endpoint, "lab/quiet", "--clocks-synced")
        r2 = row(d2, "lab/quiet")
        report.check(sec, "step 2: lab/quiet, tokenless, is found by its descriptor and read under health.v2: "
                          "healthy, ok",
                     r2.get("profile") == "health.v2" and verdict(r2) == HEALTHY and c2 == 0,
                     f"exit {c2}; {brief(r2)}")
        time.sleep(1.1)
        o.cmd("status ok serving")
        time.sleep(0.3)
        c3, d3, _ = health(bus.endpoint, "lab/svc", "--clocks-synced")
        r3 = row(d3, "lab/svc")
        st3 = r3.get("status") or {}
        report.check(sec, "step 3: zenctl shows the status ok, serving (the reason 2), its since not before the "
                          "first status's",
                     st3.get("level") == "ok" and said(st3, "word", "number") == 2
                     and _named(r3) == "serving" and (st3.get("since_s") or 0) >= (st1.get("since_s") or 1 << 40),
                     f"{brief(r3)}; since {st1.get('since_s')} then {st3.get('since_s')}")
        c4, d4, e4 = health(bus.endpoint, "lab/svc", "--for", "65", timeout=200)
        r4 = row(d4, "lab/svc")
        report.check(sec, "step 4: a 65 s window with no status put: zenctl judges healthy, ok, past the horizon, "
                          "its heartbeats confirming the status, and the clock question no, confirmed",
                     verdict(r4) == HEALTHY and c4 == 0
                     and (said(r4, "clock_ahead", "answer"), said(r4, "clock_ahead", "reason")) == ("no", "confirmed"),
                     f"exit {c4}; {brief(r4)}")
        if hs.OWNER == "rust":
            bus.owner("lab/old", "--v1", "--level", "ok", "--reason", "serving")
        else:
            from . import health as v1

            old = v1.HealthOwner("lab", "old", level=v1.OK, reason="serving", connect=bus.endpoint)
            old.start()
            bus.extra.append(old)
        time.sleep(1.0)
        c5, d5, e5 = health(bus.endpoint, "--clocks-synced")
        rows = {a: row(d5, a) for a in ("lab/svc", "lab/quiet", "lab/old")}
        roll = (d5 or {}).get("rollup") or {}
        report.check(sec, "step 5: zenctl reads lab/svc and lab/quiet under health.v2 and lab/old under health.v1, "
                          "each healthy, ok; its roll-up counts three healthy services",
                     {a: r.get("profile") for a, r in rows.items()}
                     == {"lab/svc": "health.v2", "lab/quiet": "health.v2", "lab/old": "health.v1"}
                     and all(verdict(r)[:2] == HEALTHY[:2] for r in rows.values()) and roll.get("healthy") == 3
                     and c5 == 0,
                     f"exit {c5}; {[(a, r.get('profile'), verdict(r)) for a, r in rows.items()]}; roll-up {roll}")
        cd, dd, ed = doctor(bus.endpoint, "--clocks-synced")
        rows_d = doctor_rows(dd)
        report.check(sec, "step 5, the doctor: its four health-* checks over the three services are clean, exit 0",
                     cd == 0 and all(r["answer"] == PASS for r in rows_d.values()) and len(rows_d) == 4,
                     f"exit {cd}; {json.dumps(rows_d)[:900]}; {ed[-300:]}")
        cc, dc, ec = conform(bus.endpoint, "lab/svc", "--for", "3", "--clocks-synced")
        cases = conform_cases(dc)
        report.check(sec, "step 5, check conform lab/svc health.v2: its health and health-aggregation cases pass",
                     all(v and all(x == PASS for x in v) for k, v in cases.items() if k.startswith("health"))
                     and {"health", "health-aggregation"} <= set(cases),
                     f"exit {cc}; {json.dumps(cases)[:900]}; {ec[-300:]}")
    finally:
        bus.close()


# -- §2 -----------------------------------------------------------------------------------

def zsection2(report: Any) -> None:
    hs = _hs()
    sec = "§2 a stale status (zenctl as S and G)"
    bus = hs.Bus()
    try:
        # zenctl reads presence before its window opens, so its window
        # starts once the owner's tokens are up: one started before them
        # reads the service absent at both readings (round 123)
        o = bus.owner("lab/svc", "--level", "ok", "--reason", "serving")
        window = Background("health", "lab/svc", "--for", "65", "-c", bus.endpoint)
        c1, d1, _ = window.wait(200)
        r1 = row(d1, "lab/svc")
        c1g, d1g, _ = health(bus.endpoint, "lab/svc", "--clocks-synced")
        r1g = row(d1g, "lab/svc")
        report.check(sec, "step 1: over 65 s, zenctl's window judges healthy, ok, past the horizon; its GET reading, "
                          "on the deployment's word for the clock, healthy, ok",
                     verdict(r1) == HEALTHY and verdict(r1g) == HEALTHY and c1 == 0 and c1g == 0,
                     f"window exit {c1}: {brief(r1)}; GET exit {c1g}: {brief(r1g)}")
        o.cmd("unvouch")
        t2 = time.monotonic()
        w2 = Background("health", "lab/svc", "--for", "66", "-c", bus.endpoint)
        c2, d2, _ = w2.wait(200)
        r2 = row(d2, "lab/svc")
        time.sleep(max(0.0, 31 - (time.monotonic() - t2) + 35))     # 65 s past the last vouching heartbeat
        c3, d3, _ = health(bus.endpoint, "lab/svc", "--clocks-synced")
        r3 = row(d3, "lab/svc")
        report.check(sec, "step 3: the owner present, zenctl's window after the unvouching judges stale, "
                          "beyond_horizon, and its GET reading stale too: never unhealthy, never failed, never ok as "
                          "current",
                     r2.get("verdict") == "stale" and r3.get("verdict") == "stale" and c2 == 1 and c3 == 1
                     and r2.get("level") in (None, "stale") and r3.get("level") in (None, "stale"),
                     f"window exit {c2}: {brief(r2)}; GET exit {c3}: {brief(r3)}")
        cd, dd, ed = doctor(bus.endpoint, "--clocks-synced")
        report.check(sec, "step 3, the doctor: health-stale finds lab/svc, health-failed and health-degraded do not",
                     cd == 1 and finds(dd, "health-stale") == ["lab/svc"] and not finds(dd, "health-failed")
                     and not finds(dd, "health-degraded"),
                     f"exit {cd}; {json.dumps(doctor_rows(dd))[:1200]}")
    finally:
        bus.close()


# -- §3 -----------------------------------------------------------------------------------

def zsection3(report: Any) -> None:
    hs = _hs()
    sec = "§3 aggregation, by seq (zenctl as G)"
    bus = hs.Bus()
    try:
        o = bus.owner("lab/svc", "--level", "ok", "--reason", "serving")
        o.cmd("check disk ok ok")
        o.cmd("check net ok ok")
        time.sleep(0.5)
        o.cmd("check disk failed timeout")
        time.sleep(0.5)
        c2, d2, _ = health(bus.endpoint, "lab/svc", "--clocks-synced")
        r2 = row(d2, "lab/svc")
        checks = {c.get("check"): (c.get("level"), c.get("detail")) for c in r2.get("checks") or []}
        report.check(sec, "step 2: zenctl reads one instant in each reading, status failed with the reason check, "
                          "disk failed (timeout) and net ok, and judges unhealthy, failed",
                     verdict(r2) == ("unhealthy", "failed", "failed") and _named(r2) == "check"
                     and checks == {"disk": ("failed", "timeout"), "net": ("ok", "ok")}
                     and all(x.get("one_instant") for x in r2.get("readings") or [{}]) and c2 == 1,
                     f"exit {c2}; {brief(r2)}")
        cc, dc, _ = conform(bus.endpoint, "lab/svc", "--for", "3", "--clocks-synced")
        cd, dd, _ = doctor(bus.endpoint, "--clocks-synced")
        cases = conform_cases(dc)
        report.check(sec, "step 2, the doctor and check conform: health-failed finds lab/svc; conform's health case "
                          "fails it, its health-aggregation case passes",
                     cd == 1 and finds(dd, "health-failed") == ["lab/svc"]
                     and cases.get("health") == [FOUND] and cases.get("health-aggregation") == [PASS],
                     f"doctor exit {cd}: {json.dumps(doctor_rows(dd))[:700]}; conform exit {cc}: {json.dumps(cases)}")
        o.cmd("check disk ok ok")
        o.cmd("status ok serving")
        time.sleep(0.5)
        c3, d3, _ = health(bus.endpoint, "lab/svc", "--clocks-synced")
        r3 = row(d3, "lab/svc")
        report.check(sec, "step 3: zenctl judges healthy, ok", verdict(r3) == HEALTHY and c3 == 0, brief(r3))
        o.cmd("retire net")
        time.sleep(0.5)
        c4, d4, _ = health(bus.endpoint, "lab/svc", "--clocks-synced")
        r4 = row(d4, "lab/svc")
        report.check(sec, "step 4: net retired: zenctl's reading holds the status and disk, and no net",
                     [c.get("check") for c in r4.get("checks") or []] == ["disk"] and verdict(r4) == HEALTHY,
                     brief(r4))
    finally:
        bus.close()


# -- §4 -----------------------------------------------------------------------------------

def _zclock(report: Any, drop_future: bool) -> None:
    hs = _hs()
    sec = "§4 clock ahead (zenctl as S and L)" + (", step 6 under drop_future_timestamp" if drop_future else "")
    bus = hs.Bus(drop_future=drop_future)
    ref = "zk2py/health-v2/reference"
    stop = threading.Event()
    try:
        o = bus.owner("lab/ahead", "--level", "ok", "--reason", "serving", "--reference", ref, "--offset-ms", "2000",
                      "--run-record", os.path.join(bus.tmp, "ahead.run"))
        time.sleep(1.0)
        if not drop_future:
            c1, d1, _ = health(bus.endpoint, "lab/ahead", "--for", "31", timeout=120)
            r1 = row(d1, "lab/ahead")
            report.check(sec, "step 1: before any put of the reference, zenctl's 31 s window reads heartbeats with "
                              "an instant: the clock question no, confirmed; healthy, ok",
                         said(r1, "clock_ahead", "answer") == "no" and verdict(r1) == HEALTHY, brief(r1))
        s = Background("health", "lab/ahead", "--for", "65" if not drop_future else "35", "-c", bus.endpoint)
        s.listening.wait(20)

        def reference() -> None:
            while not stop.wait(0.3):
                bus.g.put(ref, b"beat")

        threading.Thread(target=reference, daemon=True).start()
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline and not o.state()["ahead"]:
            time.sleep(0.05)
        o.cmd("status ok fallback")
        late = None
        if not drop_future:
            time.sleep(35.0)
            late = Background("health", "lab/ahead", "--for", "31", "-c", bus.endpoint)
        c2, d2, e2 = s.wait(200)
        r2 = row(d2, "lab/ahead")
        tag = "step 6" if drop_future else "step 2"
        report.check(sec, f"{tag}: zenctl's window over the hold answers the clock question yes, ahead, judges "
                          f"healthy, ok, and shows the status ok, fallback; the fault heard answers nothing",
                     (said(r2, "clock_ahead", "answer"), said(r2, "clock_ahead", "reason")) == ("yes", "ahead")
                     and verdict(r2)[:2] == ("healthy", "ok") and _named(r2) == "fallback",
                     f"exit {c2}; {brief(r2)}")
        lf = r2.get("last_fault") or {}
        report.check(sec, f"{tag}: what zenctl shows of the faults heard in the window: one, clock_ahead, at "
                          f"failed, the argument 2 or 3",
                     r2.get("faults") == 1 and lf.get("code") == "clock_ahead" and lf.get("level") == "failed"
                     and lf.get("argument") in (2, 3), f"faults {r2.get('faults')}, last {lf}")
        if drop_future:
            return
        assert late is not None
        c3, d3, _ = late.wait(200)
        r3 = row(d3, "lab/ahead")
        report.check(sec, "step 3: L, zenctl started 35 s after the detection, hears no fault and answers yes, "
                          "ahead, from the heartbeats it reads",
                     (said(r3, "clock_ahead", "answer"), said(r3, "clock_ahead", "reason")) == ("yes", "ahead")
                     and not r3.get("faults"),
                     brief(r3))
        o.cmd("clock 0")
        c4, d4, _ = health(bus.endpoint, "lab/ahead", "--for", "35", timeout=120)
        r4 = row(d4, "lab/ahead")
        report.check(sec, "step 4: the clock set right: zenctl's 35 s window answers no, confirmed, and judges "
                          "healthy, ok",
                     said(r4, "clock_ahead", "answer") == "no" and verdict(r4)[:2] == ("healthy", "ok"), brief(r4))
        s5 = Background("health", "lab/ahead", "--for", "35", "-c", bus.endpoint)
        s5.listening.wait(20)
        o.cmd("clock 2000")
        c5, d5, _ = s5.wait(200)
        r5 = row(d5, "lab/ahead")
        report.check(sec, "step 5: a new hold: zenctl's window hears one more clock_ahead fault, and answers yes, "
                          "ahead",
                     said(r5, "clock_ahead", "answer") == "yes" and r5.get("faults") == 1
                     and said(r5, "last_fault", "code") == "clock_ahead", brief(r5))
    finally:
        stop.set()
        bus.close()


def zsection4(report: Any) -> None:
    _zclock(report, False)
    _zclock(report, True)


# -- §5 -----------------------------------------------------------------------------------

def zsection5(report: Any, n: int = 100) -> None:
    hs = _hs()
    sec = f"§5 a tokenless set of {n} (zenctl as T)"
    bus = hs.Bus()
    try:
        bus.owner("p5/x", "--many", str(n), "--nav", str(hs.NAV), "--tokenless", "health.v2",
                  "--tokenless", "freshness.v2")
        time.sleep(2.0)
        c, d, e = health(bus.endpoint, "--clocks-synced", "--timeout", "5", timeout=600)
        rows = [r for r in (d or {}).get("rows", []) if str(r.get("address", "")).startswith("p5/")]
        roll = (d or {}).get("rollup") or {}
        report.check(sec, f"zenctl finds all {n} providers by their descriptors, none by a token, reads each under "
                          "health.v2, and judges each healthy, ok",
                     len(rows) == n and all(r.get("profile") == "health.v2" and verdict(r) == HEALTHY for r in rows)
                     and roll.get("healthy") == n and c == 0,
                     f"exit {c}; {len(rows)} rows; roll-up {roll}; {e[-300:]}")
    finally:
        bus.close()


# -- §6 -----------------------------------------------------------------------------------

def zsection6(report: Any) -> None:
    hs = _hs()
    sec = "§6 an absent owner (zenctl as T, the reference's archive)"
    bus = hs.Bus()
    try:
        o = bus.owner("lab/svc", "--level", "degraded", "--reason", "upstream_lost")
        time.sleep(0.5)
        archive = hs.ReferenceArchive(bus.endpoint, "lab/archive", "lab/svc", "zk2/lab/svc/health.v2/state/**")
        bus.extra.append(archive)
        time.sleep(1.0)
        c1, d1, _ = health(bus.endpoint, "lab/svc", "--clocks-synced")
        r1 = row(d1, "lab/svc")
        report.check(sec, "step 1: unhealthy, degraded; zenctl shows the reason as upstream_lost",
                     verdict(r1) == ("unhealthy", "degraded", "degraded") and _named(r1) == "upstream_lost", brief(r1))
        cd, dd, _ = doctor(bus.endpoint, "--clocks-synced")
        report.check(sec, "step 1, the doctor: health-degraded finds lab/svc",
                     cd == 1 and finds(dd, "health-degraded") == ["lab/svc"] and not finds(dd, "health-failed"),
                     f"exit {cd}; {json.dumps(doctor_rows(dd))[:900]}")
        o.end()
        time.sleep(3.0)
        c3, d3, e3 = health(bus.endpoint, "lab/svc", "--clocks-synced")
        r3 = row(d3, "lab/svc")
        report.check(sec, "step 3: zenctl judges not asked, absent: never failed, never stale; it shows the "
                          "archived status, degraded, upstream_lost, as last-known, never as current",
                     verdict(r3)[:2] == ("not_asked", "absent") and "upstream_lost" in json.dumps(r3)
                     and said(r3, "status", "via") != "get",
                     f"exit {c3}; {json.dumps(r3, default=str)[:1500]}; {e3[-300:]}")
    finally:
        bus.close()


# -- §7 -----------------------------------------------------------------------------------

def zsection7(report: Any) -> None:
    hs = _hs()
    sec = "§7 an inconsistent status (zenctl as T)"
    bus = hs.Bus()
    try:
        bus.owner("lab/liar", "--level", "ok", "--reason", "serving", "--liar", "--check", "disk=failed:timeout",
                  impl="zk2py")
        bus.owner("lab/frank", "--level", "failed", "--reason", "misconfigured", "--check", "disk=ok:ok")
        flip = bus.owner("lab/flip", "--level", "ok", "--reason", "serving", "--check", "disk=ok:ok")
        bus.owner("lab/two", "--level", "ok", "--reason", "serving", "--liar", "--two-asofs",
                  "--check", "disk=failed:timeout", impl="zk2py")
        time.sleep(0.5)
        c1, d1, _ = health(bus.endpoint, "lab/liar", "--clocks-synced")
        r1 = row(d1, "lab/liar")
        report.check(sec, "step 1: lab/liar (zk2py's): each reading one instant; unhealthy, inconsistent, at "
                          "failed; zenctl reports the break as a finding",
                     verdict(r1) == ("unhealthy", "inconsistent", "failed")
                     and all(x.get("one_instant") for x in r1.get("readings") or [{}])
                     and said(r1, "agrees", "answer") == "no" and c1 == 1, brief(r1))
        c1f, d1f, _ = health(bus.endpoint, "lab/frank", "--clocks-synced")
        r1f = row(d1f, "lab/frank")
        report.check(sec, "step 1: lab/frank: unhealthy, failed, and no break",
                     verdict(r1f) == ("unhealthy", "failed", "failed") and said(r1f, "agrees", "answer") == "yes",
                     brief(r1f))
        cd, dd, _ = doctor(bus.endpoint, "--clocks-synced")
        inc = finds(dd, "health-inconsistent")
        report.check(sec, "step 1, the doctor: health-inconsistent finds lab/liar and lab/two, and not lab/frank or "
                          "lab/flip",
                     inc == ["lab/liar", "lab/two"], f"exit {cd}; {json.dumps(doctor_rows(dd))[:900]}")
        cc, dc, _ = conform(bus.endpoint, "lab/liar", "--for", "3", "--clocks-synced")
        cases = conform_cases(dc)
        report.check(sec, "step 1, check conform lab/liar health.v2: its health-aggregation case fails",
                     cases.get("health-aggregation") == [FOUND],
                     f"exit {cc}; {json.dumps(cases)}")
        flip.cmd("flip 100")
        rows = []
        for _ in range(50):
            t = time.monotonic()
            c, d, _ = health(bus.endpoint, "lab/flip", "--clocks-synced", "--grace", "0.1")
            r = row(d, "lab/flip")
            rows.append((said(r, "agrees", "answer"), [x.get("one_instant") for x in r.get("readings") or []],
                         r.get("verdict")))
            time.sleep(max(0.0, 0.037 - (time.monotonic() - t)))
        flip.cmd("flip stop")
        report.check(sec, "step 2: 50 readings of lab/flip: every reading one instant, and no break reported",
                     all(len(x) == 2 and all(x) for _, x, _ in rows) and all(a != "no" for a, _, _ in rows)
                     and all(v is not None for _, _, v in rows),
                     f"agrees {sorted({str(a) for a, _, _ in rows})}; one instant "
                     f"{sum(all(x) for _, x, _ in rows)} of 50; verdicts {sorted({str(v) for _, _, v in rows})}")
        c3, d3, _ = health(bus.endpoint, "lab/two", "--clocks-synced", "--grace", "2")
        r3 = row(d3, "lab/two")
        rd = r3.get("readings") or []
        report.check(sec, "step 3: lab/two (zk2py's): no reading is one instant; unhealthy, inconsistent, at "
                          "failed; the break reported from two readings a grace apart",
                     len(rd) == 2 and not any(x.get("one_instant") for x in rd)
                     and verdict(r3) == ("unhealthy", "inconsistent", "failed")
                     and said(r3, "agrees", "answer") == "no", brief(r3))
    finally:
        bus.close()


# -- §8 -----------------------------------------------------------------------------------

def _zface(report: Any, workdir: str, run: str, wait_s: float) -> None:
    from . import live

    hs = _hs()
    sec = f"§8 across a constrained face (zenctl as G), run {run}"
    crosses = run == "A"
    denied = ["zk2/**/@zk/**"] + hs.FACES["listed"][run]
    v, ep, face_ep = hs.face_router(os.path.join(workdir, run), denied)
    addr = "vehicle-01/nav"
    ref = "zk2py/health-v2/reference"
    stop = threading.Event()
    o = None
    across = ["--across-face", "crosses" if crosses else "denied", "--iface", "health.v2"]
    try:
        g1 = Background("health", addr, *across, "--for", str(int(wait_s)), "-c", face_ep)
        c1, d1, e1 = g1.wait(wait_s + 60)
        r1 = row(d1, addr)
        want = ("unobservable", "nothing_crossed" if crosses else "face_closed")
        report.check(sec, f"step 1: before the owner starts zenctl judges unobservable, {want[1]}",
                     verdict(r1)[:2] == want, f"exit {c1}; {brief(r1)}; {e1[-300:] if not r1 else ''}")
        g2 = Background("health", addr, *across, "--for", str(int(wait_s)), "-c", face_ep)
        g2.listening.wait(15)
        o = hs.make_owner(ep, addr, "--level", "ok", "--reason", "serving", "--reference", ref,
                          "--run-record", os.path.join(workdir, f"{run}.run"))
        c2, d2, _ = g2.wait(wait_s + 60)
        r2 = row(d2, addr)
        if crosses:
            report.check(sec, "step 2: zenctl judges healthy, ok, from the status and the heartbeats that cross",
                         verdict(r2) == HEALTHY, f"exit {c2}; {brief(r2)}")
        else:
            report.check(sec, "step 2: zenctl judges unobservable, face_closed, and the clock question "
                              "unobservable, nothing_heard",
                         verdict(r2)[:2] == ("unobservable", "face_closed")
                         and (said(r2, "clock_ahead", "answer"), said(r2, "clock_ahead", "reason"))
                         == ("unobservable", "nothing_heard"), f"exit {c2}; {brief(r2)}")
        vehicle = live.open_client(ep)

        def reference() -> None:
            while not stop.wait(0.3):
                vehicle.put(ref, b"beat")

        g3 = Background("health", addr, *across, "--for", "37", "-c", face_ep)
        g3.listening.wait(15)
        threading.Thread(target=reference, daemon=True).start()
        o.cmd("clock 2000")
        c3, d3, _ = g3.wait(120)
        r3 = row(d3, addr)
        ca = (said(r3, "clock_ahead", "answer"), said(r3, "clock_ahead", "reason"))
        nf, lcode = r3.get("faults") or 0, said(r3, "last_fault", "code")
        if run == "A":
            report.check(sec, "step 3: no fault crosses; zenctl reads a heartbeat whose time is ahead, and answers "
                              "yes, ahead", ca == ("yes", "ahead") and not nf, brief(r3))
        elif run == "B":
            report.check(sec, "step 3: nothing crosses; the clock question unobservable, nothing_heard",
                         ca == ("unobservable", "nothing_heard") and not nf, brief(r3))
        else:
            report.check(sec, "step 3: zenctl shows one clock_ahead fault as an occurrence; the clock question "
                              "unobservable, nothing_heard; health unobservable, face_closed",
                         nf == 1 and lcode == "clock_ahead" and ca == ("unobservable", "nothing_heard")
                         and verdict(r3)[:2] == ("unobservable", "face_closed"), brief(r3))
        if run == "A":
            o.cmd("clock 0")
            o.cmd("unvouch")
            c4, d4, _ = health(face_ep, addr, *across, "--for", str(int(wait_s) + 2), timeout=wait_s + 60)
            r4 = row(d4, addr)
            report.check(sec, "step 4: zenctl judges stale: never down, never failed, never absent; the clock "
                              "question no",
                         r4.get("verdict") == "stale" and said(r4, "clock_ahead", "answer") == "no", brief(r4))
        vehicle.close()
    finally:
        stop.set()
        if o is not None:
            o.close()
        v.close()


def zsection8(report: Any, wait_s: float = 65.0) -> None:
    import tempfile

    workdir = tempfile.mkdtemp(prefix="zk2py-hv2-zface-", dir=os.environ.get("ZK2PY_SCRATCH"))
    threads = []
    for run in ("A", "B", "C"):
        os.makedirs(os.path.join(workdir, run), exist_ok=True)
        t = threading.Thread(target=_zface, args=(report, workdir, run, wait_s))
        t.start()
        threads.append(t)
    for t in threads:
        t.join()


# -- §9 -----------------------------------------------------------------------------------

def zsection9(report: Any) -> None:
    """T is ``zenctl health --contracts tc.v1``; T2 ``zenctl health
    --contracts`` the earlier revision alone, its session on a face that
    denies it the contract keys, so that no bundle is retrieved (§8.4)."""
    hs = _hs()
    sec = "§9 words, by table (zenctl as T and T2)"
    bus = hs.Bus()
    import tempfile

    wd = tempfile.mkdtemp(prefix="zk2py-hv2-z9-", dir=os.environ.get("ZK2PY_SCRATCH"))
    v2r, _, t2_ep = None, None, None
    try:
        import zenoh

        from .owner import free_loopback_port

        sock = os.path.join(wd, "t2.sock")
        port = free_loopback_port()
        conf = zenoh.Config()
        conf.insert_json5("mode", json.dumps("router"))
        conf.insert_json5("listen/endpoints", json.dumps([f"tcp/127.0.0.1:{port}", f"unixsock-stream/{sock}"]))
        conf.insert_json5("connect/endpoints", json.dumps([bus.endpoint]))
        conf.insert_json5("scouting/multicast/enabled", "false")
        conf.insert_json5("access_control", json.dumps({
            "enabled": True, "default_permission": "allow",
            "rules": [{"id": "no-bundles", "messages": ["query", "reply", "declare_queryable"],
                       "flows": ["ingress", "egress"], "permission": "deny", "key_exprs": ["zk2/@zk/contract/**"]}],
            "subjects": [{"id": "t2", "link_protocols": ["unixsock-stream"]}],
            "policies": [{"id": "t2", "rules": ["no-bundles"], "subjects": ["t2"]}]}))
        v2r = zenoh.open(conf)
        t2_ep = f"unixsock-stream/{sock}"
        tc_now, tc_old = str(hs.INTEROP / "tc.v1.toml"), str(hs.INTEROP / "earlier" / "tc.v1.toml")
        tc = bus.owner("lab/tc", "--level", "ok", "--reason", "serving", "--contract", tc_now,
                       "--run-record", os.path.join(bus.tmp, "tc.run"))
        plain = bus.owner("lab/plain", "--level", "ok", "--reason", "serving")
        time.sleep(1.0)
        t = Background("health", "--contracts", tc_now, "--contracts", str(REPO / "spec" / "profiles" / "health"
                                                                            / "health.v2.toml"),
                       "--for", "8", "-c", bus.endpoint)
        t2 = Background("health", "--contracts", tc_old, "--for", "8", "-c", t2_ep)
        echo_out: list[str] = []
        ep_ = subprocess.Popen([str(ZENCTL), "echo", "zk2/lab/tc/health.v2/**", "-c", bus.endpoint, "--contracts",
                                tc_now, "--contracts", str(REPO / "spec" / "profiles" / "health" / "health.v2.toml"),
                                "--format", "ndjson", "--color", "never"], stdout=subprocess.PIPE,
                               stderr=subprocess.DEVNULL, text=True)
        threading.Thread(target=lambda: [echo_out.append(x) for x in ep_.stdout], daemon=True).start()
        t.listening.wait(20)
        t2.listening.wait(20)
        time.sleep(1.0)
        tc.cmd("status degraded tc_netns_gone")
        tc.cmd("fault tc_netns_restarted degraded 3")
        time.sleep(0.5)
        tc.cmd("status degraded kernel too old for this qdisc")
        plain.cmd("status degraded tc_netns_gone")
        ct, dt, et = t.wait(120)
        ct2, dt2, et2 = t2.wait(120)
        ep_.terminate()
        ep_.wait(10)
        echo_t = "".join(echo_out)
        rt, rt2 = row(dt, "lab/tc"), row(dt2, "lab/tc")
        pt, pt2 = row(dt, "lab/plain"), row(dt2, "lab/plain")
        lf, lf2 = rt.get("last_fault") or {}, rt2.get("last_fault") or {}
        report.check(sec, "step 4, T: the fault heard shows its code as tc_netns_restarted, the service's word 32, "
                          "with the argument 3; the status read, step 2's, shows as a word no table names (63, "
                          "out_of_range), never a name, never no word",
                     rt.get("faults") == 1 and lf.get("code") == "tc_netns_restarted" and lf.get("argument") == 3
                     and said(lf, "word", "number") == 32 and said(lf, "word", "read") == "service"
                     and said(rt, "status", "word") == {"number": 63, "read": "out_of_range"},
                     f"exit {ct}; faults {rt.get('faults')}, last {lf}; status {rt.get('status')}; "
                     f"{et[-300:] if not rt else ''}")
        echo = [json.loads(x) for x in echo_t.splitlines() if x.startswith("{")]
        st1 = [e for e in echo if str(e.get("key", "")).endswith("lab/tc/health.v2/state/status")]
        report.check(sec, "step 4, T: what the reference shows of step 1's status, heard: zenctl health shows the "
                          "last status alone; zenctl echo, beside it, shows step 1's put, its reason as "
                          "tc_netns_gone",
                     "tc_netns_gone" in json.dumps(rt) or "tc_netns_gone" in json.dumps(st1[:1]),
                     f"health shows {said(rt, 'status', 'reason')!r}; echo's first status row "
                     f"{json.dumps(st1[:1])[:700]}")
        report.check(sec, "step 4, T2 (the earlier revision alone, no bundle retrieved): it shows the fault's code "
                          "as the number 32, never as a name, and no name of its own revision",
                     rt2.get("faults") == 1 and lf2.get("code") == "32" and said(lf2, "word", "read") == "number"
                     and "tc_qdisc_missing" not in json.dumps(rt2) and "tc_netns" not in json.dumps(rt2),
                     f"exit {ct2}; faults {rt2.get('faults')}, last {lf2}; status {rt2.get('status')}; {et2[-400:]}")
        report.check(sec, "step 4: both show lab/plain's reason as a word no table names; each judges both "
                          "services unhealthy, degraded",
                     said(pt, "status", "word", "number") == 63 and said(pt2, "status", "word", "number") == 63
                     and all(verdict(r) == ("unhealthy", "degraded", "degraded") for r in (rt, rt2, pt, pt2)),
                     f"{[verdict(r) for r in (rt, rt2, pt, pt2)]}; plain {said(pt, 'status', 'word')} "
                     f"{said(pt2, 'status', 'word')}")
    finally:
        bus.close()
        if v2r is not None:
            v2r.close()


SECTIONS = {"1": zsection1, "2": zsection2, "3": zsection3, "4": zsection4, "5": zsection5, "6": zsection6,
            "7": zsection7, "8": zsection8, "9": zsection9}


def main(argv: list[str] | None = None) -> int:
    hs = _hs()
    p = argparse.ArgumentParser(prog="zk2py.health_v2_pairings")
    p.add_argument("--owner", choices=("zk2py", "rust"), default="zk2py")
    p.add_argument("--only", action="append", choices=sorted(SECTIONS))
    args = p.parse_args(argv)
    hs.OWNER = args.owner
    hs.ARCHIVE = "rust"
    print(f"NOTE the owner {args.owner}'s, the reader zenctl ({ZENCTL})", flush=True)
    report = hs.Report()
    for name in args.only or sorted(SECTIONS):
        try:
            SECTIONS[name](report)
        except Exception as e:  # noqa: BLE001 - a section that raises is a failed check, the rest go on
            import traceback

            report.check(f"§{name}", "the section ran", False, f"{type(e).__name__}: {e}\n{traceback.format_exc()}")
    bad = [r for r in report.rows if not r[2]]
    by: dict[str, list[bool]] = {}
    for sec, _, ok, _ in report.rows:
        by.setdefault(sec, []).append(ok)
    for sec, oks in by.items():
        print(f"{sec}: {sum(oks)} of {len(oks)}")
    print(f"{len(report.rows) - len(bad)} of {len(report.rows)} checks passed")
    return 1 if bad else 0



# -- the two owners' bytes, side by side ------------------------------------------------------

def _mask_record(name: str, rec: bytes) -> tuple[Any, bool]:
    """A record decoded with its time fields left out, and whether zk2py's
    writer gives its bytes back from what it read (so that two records
    equal once read differ in their time bits alone)."""
    from . import health_v2 as h2

    r = h2.read(name, h2.TAGS[name], rec, 0)
    v = {k: x for k, x in r.expect().items() if k not in ("since", "at")}
    try:
        again = h2.write(name, r.value, 0) == rec
    except Exception:  # noqa: BLE001 - a value zk2py's writer refuses: not given back
        again = False
    return v, again


def _mask_snapshot(page: dict[str, Any]) -> dict[str, Any]:
    """A snapshot page with its run, asof, time, and each entry's run, seq
    and conf left out, each entry's record read with its time left out."""
    from . import health_v2 as h2

    name = h2.STATUS if page.get("coll") == 0 and not any(e.get("id") for e in page.get("live", [])) else None

    def entry(e: dict[str, Any]) -> dict[str, Any]:
        out = {k: x for k, x in e.items() if k not in ("run", "run_width", "seq", "conf", "record")}
        if "record" in e:
            kind = h2.CHECK if e.get("id") else h2.STATUS
            out["record"] = _mask_record(kind, bytes.fromhex(e["record"]))[0]
        return out

    del name
    return {"tag": page.get("tag"), "run_width": page.get("run_width"), "coll": page.get("coll"),
            "live": [entry(e) for e in page.get("live", [])], "tombs": [entry(e) for e in page.get("tombs", [])],
            "time_known": page.get("time") is not None}


def side_by_side(report: Any) -> None:
    """One state on both owners, each with a 16-bit run, read by one raw
    subscriber and one GET: each put's bytes, the heartbeat's and the
    snapshots', zk2py's beside the reference's, and whether they differ in
    anything but time, run and seq."""
    import zenoh

    from . import health_v2 as h2
    from . import heartbeat, live, zk2c

    hs = _hs()
    sec = "both owners' bytes, side by side"
    bus = hs.Bus()
    heard: list[tuple[str, str, bytes, bytes | None]] = []
    lock = threading.Lock()

    def on(sample: Any) -> None:
        with lock:
            heard.append((str(sample.key_expr), "put" if sample.kind == zenoh.SampleKind.PUT else "delete",
                          sample.payload.to_bytes(),
                          None if sample.attachment is None else sample.attachment.to_bytes()))

    subs = [bus.s.declare_subscriber(k, zenoh.handlers.Callback(on))
            for k in ("zk2/lab/*/health.v2/**", "zk2/lab/*/freshness.v2/stream/heartbeat/health.v2")]
    try:
        time.sleep(0.3)
        tc = str(hs.INTEROP / "tc.v1.toml")
        owners = {"zk2py": bus.owner("lab/py", "--level", "ok", "--reason", "serving", "--contract", tc,
                                     "--run-record", os.path.join(bus.tmp, "py.run"), impl="zk2py"),
                  "rust": bus.owner("lab/rs", "--level", "ok", "--reason", "serving", "--contract", tc,
                                    "--run-record", os.path.join(bus.tmp, "rs.run"), impl="rust")}
        time.sleep(1.0)
        # ``status degraded tc_netns_gone`` after disk's recovery: the owner
        # example puts its declared status back itself (``raised_by=-``),
        # zk2py's waits for its application (§2.2: "then the status, if it
        # improves"), so both are told, and the example puts nothing again
        steps = ["status degraded tc_netns_gone", "check disk failed timeout", "fault tc_netns_restarted degraded 3",
                 "fault tcd_gone failed", "check net unknown cannot_run", "retire net", "check disk ok ok",
                 "status degraded tc_netns_gone", "status ok kernel too old"]
        answers: dict[str, list[str]] = {k: [] for k in owners}
        for line in steps:
            for k, o in owners.items():
                answers[k].append(o.cmd(line))
            time.sleep(0.3)
        time.sleep(0.5)
        gets = {k: live.get_state(bus.t, f"zk2/{a}/health.v2/state/**")
                for k, a in (("zk2py", "lab/py"), ("rust", "lab/rs"))}
        with lock:
            got = list(heard)
        by: dict[str, list[tuple[str, str, bytes, bytes | None]]] = {"zk2py": [], "rust": []}
        for x in got:
            by["zk2py" if "/lab/py/" in x[0] else "rust"].append(x)
        rows = []

        def rel(key: str) -> str:
            return key.split("/", 3)[3]

        # pair each put by its key, kind and place among that key's
        pairs: list[tuple[Any, Any]] = []
        count: dict[tuple[str, str], int] = {}
        rs_by: dict[tuple[str, str], list[Any]] = {}
        for x in by["rust"]:
            rs_by.setdefault((rel(x[0]), x[1]), []).append(x)
        for x in by["zk2py"]:
            k_ = (rel(x[0]), x[1])
            i_ = count.get(k_, 0)
            count[k_] = i_ + 1
            if i_ < len(rs_by.get(k_, [])):
                pairs.append((x, rs_by[k_][i_]))
        n = len(pairs)
        seq_py = [(rel(k), kind) for k, kind, _, _ in by["zk2py"]]
        seq_rs = [(rel(k), kind) for k, kind, _, _ in by["rust"]]
        report.check(sec, "the two owners put the same keys, of the same kinds, in the same order",
                     seq_py == seq_rs, f"zk2py {seq_py}\n       rust  {seq_rs}\n       answers {answers}")
        same_all = True
        lo = h2.standard_layout()
        for (kp, kind, pp, ap), (kr, _, pr, ar) in pairs:
            r = rel(kp)
            wp, wr = (pp, pr) if kind == "put" else (ap or b"", ar or b"")
            ep_, er_ = zk2c.parse(wp, kind), zk2c.parse(wr, kind)
            head = (ep_.form, ep_.width, ep_.tag) == (er_.form, er_.width, er_.tag) and wp[0] == wr[0]
            if "heartbeat" in r:
                bp, br = heartbeat.read(lo, ep_.record or b""), heartbeat.read(lo, er_.record or b"")

                def masked(b: dict[str, Any]) -> Any:
                    return {s: {k: v for k, v in (x or {}).items() if k != "digest"} if isinstance(x, dict) else x
                            for s, x in b["sections"].items()}

                def digest_ok(side: str, put: Any, body: dict[str, Any]) -> bool:
                    """The digest each owner wrote is core §7.5's over its own
                    key and its status's stamp then: the keys differ, so the
                    digests do too."""
                    seq_ = by[side]
                    before = [x for x in seq_[:seq_.index(put)] if x[0].endswith("/health.v2/state/status")
                              and x[1] == "put"]
                    if not before:
                        return False
                    st_ = zk2c.parse(before[-1][2], "put")
                    want = format(heartbeat.digest([(before[-1][0], st_.width, st_.run, st_.seq)]), "08x")
                    return body["sections"]["state/status"].get("digest") == want

                dg = digest_ok("zk2py", (kp, kind, pp, ap), bp) and digest_ok("rust", (kr, kind, pr, ar), br)
                same = head and masked(bp) == masked(br) and heartbeat.reading_of(bp["time"]) == \
                    heartbeat.reading_of(br["time"]) and dg
                what = (f"sections {masked(bp)}; each digest core §7.5's over its own key: {dg} "
                        f"({bp['sections']['state/status'].get('digest')}, "
                        f"{br['sections']['state/status'].get('digest')})")
            elif kind == "delete":
                same = head and ep_.record == er_.record
                what = "a delete's attachment"
            else:
                name = h2.STATUS if r.endswith("state/status") else h2.CHECK if "/checks/" in r else h2.FAULT
                vp, gp = _mask_record(name, ep_.record or b"")
                vr, gr = _mask_record(name, er_.record or b"")
                same = head and vp == vr and gp and gr
                what = f"{vp}" + ("" if vp == vr else f" | {vr}") + ("" if gp and gr else f" (given back {gp}, {gr})")
            same_all = same_all and same
            rows.append(f"{r} {kind}: zk2py {wp.hex()} | rust {wr.hex()} | {'same' if same else 'DIFFERS'} but "
                        f"time, run and seq: {what}")
        report.check(sec, "every put and delete: the same CH, form, width and tag, and the same record but for its "
                          "time, each given back by zk2py's writer from what it read; a heartbeat's digest each "
                          "owner's own key's",
                     same_all and n == len(by["zk2py"]) == len(by["rust"]) and n > 0, "\n       ".join(rows))
        snaps = {k: [p for s_ in st.snapshots for p in (s_.pages[i] for i in sorted(s_.pages))]
                 for k, st in gets.items()}
        raw = {k: [r_.raw.hex() for r_ in st.replies if getattr(r_, "raw", None)] for k, st in gets.items()}
        mp, mr = [_mask_snapshot(p) for p in snaps["zk2py"]], [_mask_snapshot(p) for p in snaps["rust"]]
        report.check(sec, "one GET each: the snapshots' pages are the same but for run, asof, time, seq and conf",
                     mp == mr and len(mp) == 2,
                     f"zk2py {mp}\n       rust  {mr}\n       pages zk2py {raw['zk2py']}\n"
                     f"       pages rust  {raw['rust']}")
    finally:
        for s_ in subs:
            s_.undeclare()
        bus.close()


SECTIONS["bytes"] = side_by_side



# -- words by table, beyond the scenarios (F-593, F-594, F-599) ---------------------------

WORDS = REPO / "impl" / "python" / "interop" / "health-v2" / "words"
#: (case, the owner's own contracts, the word put, what §2.10 makes of it)
WORD_CASES = [
    ("not_used (F-593)", ["hv1tab.v1.toml"], "tc_netns_gone", 63),
    ("differ (F-599)", ["empty.v1.toml", "full.v1.toml"], "tc_a", 63),
    ("value (F-594)", ["badname.v1.toml"], "tc_gone", 63),
    ("disagrees (F-594)", ["disagree.v1.toml"], "tc_a", 63),
    ("valid, beside", ["full.v1.toml"], "tc_a", 32),
]


def words(report: Any) -> None:
    """Each owner given a contract of its own whose table §2.10 makes
    ``not_used``, ``differ``, ``value`` or ``disagrees``: whether it starts,
    the word it writes, and what ``zenctl health`` and zk2py's reader show.
    Each case is run with zk2py's owner and the reference's."""
    from . import health_v2 as h2
    from . import health_v2_live as hl
    from .contract import load_contract

    hs = _hs()
    for impl in ("zk2py", "rust"):
        bus = hs.Bus()
        try:
            for n, (case, files, word, want) in enumerate(WORD_CASES):
                sec = f"words by table, {case} ({impl}'s owner)"
                addr = f"lab/w{n}"
                paths = [str(WORDS / f) for f in files]
                s = bus.listener(addr)
                flags = ["--level", "ok", "--reason", "serving"]
                for p_ in paths:
                    flags += ["--contract", p_]
                try:
                    o = bus.owner(addr, *flags, impl=impl)
                except Exception as e:  # noqa: BLE001 - an owner that refuses to start, named
                    report.check(sec, f"the owner starts with {files}", False,
                                 f"{type(e).__name__}: {str(e)[-400:]}")
                    continue
                said_ = o.cmd(f"status degraded {word}")
                time.sleep(0.4)
                puts = [x for x in s.of("state/status") if x["kind"] == "put"]
                got = s.record(puts[-1]).word if puts else None
                got = 63 if got == h2.OUT_OF_RANGE else got
                held = {c.fingerprint: c.canonical for c in [hl.standard(), *map(load_contract, paths)]}
                r, info = hl.read_near(bus.t, addr, held=held)
                zc, zd, ze = health(bus.endpoint, addr, "--clocks-synced", *[a for p_ in paths
                                                                               for a in ("--contracts", p_)])
                zr = row(zd, addr)
                report.check(sec, f"the owner writes {word!r} as {want}; zk2py's reader and zenctl show it "
                                  + ("as a word no table names" if want == 63 else "by its name"),
                             got == want and hl.shown(r.status, info["tables"]) ==
                             ("degraded, a word no table names" if want == 63 else f"degraded, {word}")
                             and said(zr, "status", "word", "number") == want
                             and (want == 63 or said(zr, "status", "word", "name") == word),
                             f"answer {said_!r}; wrote {got}; zk2py shows {hl.shown(r.status, info['tables'])!r}, "
                             f"tables {[(k, t.expect()) for k, t in info['tables'].items()]}; zenctl exit {zc}: "
                             f"{said(zr, 'status', 'word')}; log {o.state()['log']}; {ze[-200:] if not zr else ''}")
                s.close()
        finally:
            bus.close()


SECTIONS["words"] = words



# -- under a catalog, by owner heartbeat, read by zenctl ----------------------------------

def zepoch(report: Any) -> None:
    """An owner under a catalog's later epoch, confirming by owner
    heartbeat, read by ``zenctl health``: with ``--catalog``, each time
    dated from the catalog's epoch; without, what it shows; over a window,
    the status confirmed by the owner heartbeat, and the clock question
    answered from it, ahead too."""
    hs = _hs()
    who = "the reference" if hs.OWNER == "rust" else "zk2py"
    sec = f"under a catalog's epoch, by owner heartbeat ({who}'s owner, zenctl as T)"
    epoch = 1767225600
    bus = hs.Bus()
    stop = threading.Event()
    ref = "zk2py/health-v2/reference"
    try:
        path = os.path.join(bus.tmp, "catalog.json")
        catalog = hs.make_catalog(path, epoch, [REPO / "spec" / "profiles" / "health" / "health.v2.toml"])
        o = bus.owner("lab/cat", "--level", "degraded", "--reason", "serving", "--catalog", path,
                      "--check", "disk=degraded:timeout", "--owner-heartbeat", "--reference", ref,
                      "--run-record", os.path.join(bus.tmp, "cat.run"))
        time.sleep(1.0)
        now_s = time.time()
        c1, d1, _ = health(bus.endpoint, "lab/cat", "--clocks-synced", "--catalog", path)
        r1 = row(d1, "lab/cat")
        since, at = said(r1, "status", "since_s"), [c.get("at_s") for c in r1.get("checks") or []]
        near = lambda x: isinstance(x, (int, float)) and abs(x - now_s) <= 5  # noqa: E731
        report.check(sec, "with --catalog: the status's since and the check's at are this instant, Unix seconds, "
                          "each read from the catalog's epoch",
                     near(since) and at and all(near(x) for x in at),
                     f"catalog {catalog.id}; since_s {since}, at_s {at}; now {round(now_s)}; {brief(r1)}")
        c0, d0, e0 = health(bus.endpoint, "lab/cat", "--clocks-synced")
        r0 = row(d0, "lab/cat")
        report.check(sec, "without --catalog: what zenctl shows of an owner naming a catalog it does not hold "
                          "(its usage: \"leaves them undated, so its members read unconfirmed by time\")",
                     not near(said(r0, "status", "since_s")),
                     f"exit {c0}; since_s {said(r0, 'status', 'since_s')}; {brief(r0)}; {e0[-300:]}")
        c2, d2, _ = health(bus.endpoint, "lab/cat", "--catalog", path, "--for", "35", timeout=120)
        r2 = row(d2, "lab/cat")
        report.check(sec, "a 35 s window with --catalog: the owner heartbeat confirms the status, zenctl judges "
                          "unhealthy, degraded, never stale, and answers the clock question no, confirmed, from it",
                     verdict(r2) == ("unhealthy", "degraded", "degraded")
                     and (said(r2, "clock_ahead", "answer"), said(r2, "clock_ahead", "reason")) == ("no", "confirmed"),
                     brief(r2))
        g = Background("health", "lab/cat", "--catalog", path, "--for", "35", "-c", bus.endpoint)
        g0 = Background("health", "lab/cat", "--for", "35", "-c", bus.endpoint)
        g.listening.wait(20)
        g0.listening.wait(20)

        def reference() -> None:
            while not stop.wait(0.3):
                bus.g.put(ref, b"beat")

        threading.Thread(target=reference, daemon=True).start()
        time.sleep(0.5)
        o.cmd("clock 2000")
        c3, d3, _ = g.wait(120)
        r3 = row(d3, "lab/cat")
        report.check(sec, "its guard holding: zenctl hears the clock_ahead fault, and answers yes, ahead, from the "
                          "owner heartbeat's time",
                     (said(r3, "clock_ahead", "answer"), said(r3, "clock_ahead", "reason")) == ("yes", "ahead")
                     and said(r3, "last_fault", "code") == "clock_ahead", brief(r3))
        c4, d4, e4 = g0.wait(120)
        r4 = row(d4, "lab/cat")
        report.check(sec, "F-598's first case: without --catalog, over the same hold, what zenctl answers of the "
                          "clock from an owner heartbeat whose groups it cannot read (\"whatever became of its "
                          "groups\": yes, ahead)",
                     (said(r4, "clock_ahead", "answer"), said(r4, "clock_ahead", "reason")) == ("yes", "ahead"),
                     f"exit {c4}; {brief(r4)}; {e4[-200:] if not r4 else ''}")
    finally:
        stop.set()
        bus.close()


def v1ahead(report: Any) -> None:
    """F-600: a service whose descriptor lists ``health.v1`` alone, its
    guard holding, read by ``zenctl health``: over a window that hears no
    fault, ``health.v1``'s question (v2 §2.12: "Only this major answers yes
    from it"), never this major's yes."""
    hs = _hs()
    sec = f"F-600: health.v1 alone, its guard holding ({'the reference' if hs.OWNER == 'rust' else 'zk2py'}'s owner)"
    bus = hs.Bus()
    stop = threading.Event()
    ref = "zk2py/health-v2/reference-v1"
    try:
        if hs.OWNER != "rust":
            report.check(sec, "run", True, "zk2py's health.v1 owner is round 120's: this check is the reference's "
                                           "(--owner rust)")
            return
        o = bus.owner("lab/v1", "--v1", "--level", "ok", "--reason", "serving", "--reference", ref,
                      "--offset-ms", "2000")

        def reference() -> None:
            while not stop.wait(0.3):
                bus.g.put(ref, b"beat")

        threading.Thread(target=reference, daemon=True).start()
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline and not o.state()["ahead"]:
            time.sleep(0.05)
        time.sleep(2.0)
        c, d, e = health(bus.endpoint, "lab/v1", "--for", "35", timeout=120)
        r = row(d, "lab/v1")
        report.check(sec, "a window after the fault, its heartbeats ahead: zenctl reads lab/v1 under health.v1, and "
                          "does not answer yes, ahead",
                     r.get("profile") == "health.v1" and said(r, "clock_ahead", "reason") != "ahead",
                     f"exit {c}; profile {r.get('profile')}; clock {r.get('clock_ahead')}; {e[-200:] if not r else ''}")
    finally:
        stop.set()
        bus.close()


SECTIONS["epoch-oh"] = zepoch
SECTIONS["v1ahead"] = v1ahead


if __name__ == "__main__":
    sys.exit(main())

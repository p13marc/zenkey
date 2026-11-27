"""``scenarios-v2.md`` §9, §11 and §12 (``desired.v2`` text 0.1), live, in each
pairing of the two implementations (#609, round 121). Run through
:mod:`zk2py.desired_v2_scenarios` (``--only 9``, ``11``, ``12``), which holds
the bus, the parties and the pairing.

**§9, a tool's questions,** with the reference's ``zenctl`` as T, a black box
learnt from its ``--help`` (``zenctl desired``, ``check conform``'s
``desired-report``, ``desired-binding`` and ``desired-author``), and zk2py's
tool beside it (:mod:`zk2py.desired_v2_tool`). Every reading of both is
printed on a ``COMPARE`` line: the two tools' answers, the report's record
and fields as each read them, the author's version, and what each shows
archived. Two lines set side by side say where the implementations differ.

**"As v1's".** §9's Setup takes v1's, whose T "With its author absent, ...
takes D from the target's requirement and binding (§2.17)". ``v2.md``
§2.17 removed that: "The slice from the target alone goes". Step 4 is read
three ways: as §2.17 says, no revision of the author's interface in hand
(``no-slice``); with that revision given offline (``--contracts``,
``contracts``); and v1's way, D from the target alone (``v1-way``), which
zk2py's tool can be told and ``zenctl`` cannot.

**§11** runs each implementation's target, given the two copies.

**§12** runs zk2py's ``config.v1`` owner fed through ``desired.v2``
(:class:`zk2py.config.ConfigOwner`, ``fed_major=2``), in process. The
reference's owner example has no ``config`` owner fed through either major
(its ``config desired <json>`` takes a document by command), so §12 is the
runtime's alone here.
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
import time
from pathlib import Path
from typing import Any, Sequence

from . import desired_v2 as d2, desired_v2_scenarios as sc, desired_v2_tool as tool, zk2c
from .alarms_scenarios import Report, _wait
from .blackbox import BLACKBOX

ZENCTL = BLACKBOX / "zenctl"
TASKING = sc.REPO / "impl" / "python" / "interop" / "desired-v2" / "tasking.v2.toml"
TARGET, AUTHOR, D, RP = sc.TARGET, sc.AUTHOR, sc.D, sc.RP
v, converged = sc.v, sc.converged

#: §9's Expected, per step (converged, refused, last_known, in_force), each
#: (answer, reason). 1 to 5 are v1's, which v2's "1 to 5 as v1's" keeps.
#: 6's once and 7 state all four since text 0.2 (F-605): "Refused: no,
#: converged. Last-known: no, current." in each.
S9_EXPECT: dict[Any, dict[str, tuple[str, str]]] = {
    1: {"converged": ("yes", "converged"), "refused": ("no", "converged"), "last_known": ("no", "current"),
        "in_force": ("yes", "desired")},
    2: {"converged": ("no", "refused"), "refused": ("yes", "refused"), "last_known": ("no", "current"),
        "in_force": ("yes", "desired")},
    3: {"converged": ("no", "failed"), "refused": ("yes", "failed"), "last_known": ("no", "current"),
        "in_force": ("yes", "partial")},
    4: {"converged": ("not_asked", "author_absent"), "refused": ("no", "converged"),
        "last_known": ("yes", "last_known"), "in_force": ("yes", "desired")},
    5: {"converged": ("not_asked", "unbound"), "refused": ("not_asked", "unbound"),
        "last_known": ("not_asked", "unbound"), "in_force": ("no", "unbound")},
    6: {"converged": ("not_yet", "behind"), "refused": ("no", "converged"), "last_known": ("no", "current"),
        "in_force": ("yes", "desired")},
    7: {"converged": ("unobservable", "author_silent"), "refused": ("no", "converged"),
        "last_known": ("no", "current"), "in_force": ("yes", "desired")},
    # beyond §9 (F-585): after §3 step 4, a revert on a snapshot with no
    # entry, its report reverted with no observed version, beside the
    # author's no entry under the same run: §2.17 "under another run or with
    # no observed version: unobservable, author_silent"; conclusions.json's
    # case 20
    "F-585": {"converged": ("unobservable", "author_silent"), "refused": ("no", "reverted"),
              "last_known": ("no", "current"), "in_force": ("no", "baseline")},
}
#: the answers a step states where it states fewer than four (none, since
#: text 0.2: round 121 compared two at step 7)
STATED: dict[Any, tuple[str, ...]] = {}


def zenctl(args: Sequence[str], timeout: float = 90.0) -> tuple[int | None, Any, str]:
    """The reference's tool as a black box: (exit, its JSON, stderr). Its
    config directory is a fresh one under the scratch directory."""
    cfg = sc.SCRATCH / "zcfg-r121"
    cfg.mkdir(parents=True, exist_ok=True)
    env = dict(os.environ, ZENKEY_EXPLORER_CONFIG_DIR=str(cfg))
    try:
        p = subprocess.run([str(ZENCTL), *args, "--format", "json"], capture_output=True, text=True, env=env,
                           timeout=timeout)
    except subprocess.TimeoutExpired:
        return None, None, "timed out"
    try:
        doc = json.loads(p.stdout)
    except ValueError:
        doc = None
    return p.returncode, doc, p.stderr


def z_row(doc: Any, role: str = "task", target: str = TARGET) -> dict[str, Any] | None:
    for r in (doc or {}).get("rows") or []:
        if r.get("row") == "desired" and r.get("role") == role and r.get("target") == target:
            return r
    return None


def z_answers(row: dict[str, Any] | None) -> dict[str, tuple[str, str]] | None:
    if row is None:
        return None
    return {q: (a.get("answer"), a.get("reason")) for q, a in (row.get("answers") or {}).items()}


def z_case(doc: Any, case: str) -> list[tuple[Any, Any]]:
    return [((r.get("verdict") or {}).get("answer"), (r.get("verdict") or {}).get("reason") or r.get("detail"))
            for r in (doc or {}).get("rows") or [] if r.get("case") == case]


def _norm_z(document: Any) -> dict[str, Any] | None:
    """zenctl's reading of a report, ``report.document``, in one shape with
    zk2py's: its words, whether each version is stated and its revision,
    and the refusal's code and indices. Runs and seqs are left out: they
    differ by pairing."""
    if not isinstance(document, dict):
        return None
    ref = document.get("refusal") or {}
    obs, inf = document.get("observed"), document.get("in_force")
    return {"source": document.get("source"), "outcome": document.get("outcome"),
            "last_known": document.get("last_known"),
            "observed": isinstance(obs, dict) and obs.get("version") is not None,
            "revision": obs.get("revision") if isinstance(obs, dict) else None,
            # zenctl leaves in_force out where it is none
            "in_force": inf if isinstance(inf, str) else ("version" if isinstance(inf, dict) else "none"),
            "in_force_revision": inf.get("revision") if isinstance(inf, dict) else None,
            "code": ref.get("code") or document.get("code"),
            "pointer": ref.get("pointer", document.get("pointer")), "detail": ref.get("detail", document.get("detail"))}


def _norm_py(fields: Any) -> dict[str, Any] | None:
    if not isinstance(fields, dict):
        return None
    observed = any(f"observed_{x}" in fields for x in ("run16", "run32"))
    in_force = fields.get("in_force")
    return {"source": fields.get("source"), "outcome": fields.get("outcome"), "last_known": fields.get("last_known"),
            "observed": observed, "revision": fields.get("revision"),
            "in_force": "version" if in_force in ("observed", "other") else in_force,
            "in_force_revision": fields.get("in_force_revision", fields.get("revision") if in_force == "observed"
                                            else None),
            "code": fields.get("code"), "pointer": fields.get("pointer"), "detail": fields.get("detail")}


class S9:
    """One run of §9 in one pairing: each step asked of both tools."""

    def __init__(self, rp: Report, sec: str) -> None:
        self.rp, self.sec = rp, sec
        self.compared: list[dict[str, Any]] = []

    def ask(self, bus: Any, step: Any, label: str = "", *, z_extra: Sequence[str] = (), contracts: Sequence[Any] = (),
            from_requirement: bool = False, ask_z: bool = True) -> tuple[Any, Any, Any]:
        rp, sec = self.rp, self.sec
        want = S9_EXPECT[step]
        stated = STATED.get(step)
        tag = f"{step}{' (' + label + ')' if label else ''}"
        mine = tool.read(bus.g1, TARGET, "task", contracts=contracts, from_requirement=from_requirement)
        m = mine[0].pairs() if mine else None
        cut = (lambda x: x if x is None or stated is None else {q: x.get(q) for q in stated})
        rp.check(sec, f"{tag}. zk2py's tool answers as Expected {step}"
                      + (f" ({', '.join(stated)} stated)" if stated else ""),
                 cut(m) == cut(want), f"{m}")
        rc, doc, err = (None, None, "")
        z = None
        if ask_z:
            rc, doc, err = zenctl(["desired", TARGET, "-c", bus.ep1, "--timeout", "1", *z_extra])
            z = z_answers(z_row(doc))
            rp.check(sec, f"{tag}. zenctl desired answers as Expected {step}"
                          + (f" ({', '.join(stated)} stated)" if stated else ""),
                     cut(z) == cut(want), f"{z}, exit {rc}{'' if z else ', ' + err[-300:]}")
        row = z_row(doc) or {}
        st = bus.get(RP, 2 if bus.r2 is not None else 1)
        raw = next((r.raw for r in st.replies if r.key == RP and r.raw), b"")
        record = zk2c.parse(raw, "put").record.hex() if raw else None
        mrow = mine[0] if mine else None
        line = {"pairing": sc.pairing(), "step": tag, "zenctl": {"exit": rc, "answers": z, "profile": row.get("profile"),
                                                                 "document": (row.get("report") or {}).get("document"),
                                                                 "author": row.get("author"),
                                                                 "place": (row.get("report") or {}).get("place"),
                                                                 "line": (row.get("report") or {}).get("line"),
                                                                 "slice_from": row.get("slice_from"),
                                                                 "archived": row.get("archived")},
                "zk2py": {"answers": m, "profile": mrow and mrow.profile,
                          "report": None if mrow is None or mrow.report is None else mrow.report.report,
                          "author_current": mrow and mrow.author_current, "author_version": mrow and mrow.author_version,
                          "slice_from": mrow and mrow.slice_from, "place": mrow and mrow.place,
                          "line": mrow and mrow.line,
                          "archived": [(a, ver, None if p is None else p.decode("utf-8", "replace"))
                                       for a, ver, p in (mrow.archived if mrow else [])]},
                "record": record}
        nz = _norm_z(line["zenctl"]["document"])
        npy = _norm_py(line["zk2py"]["report"])
        line["differ"] = sorted(k for k in (set(nz or {}) | set(npy or {}))
                                if (nz or {}).get(k) != (npy or {}).get(k)) if ask_z and nz is not None else None
        line["answers_differ"] = sorted(q for q in (set(z or {}) | set(m or {})) if (z or {}).get(q) != (m or {}).get(q)) \
            if ask_z else None
        print("COMPARE " + json.dumps(line, sort_keys=True, default=str), flush=True)
        self.compared.append(line)
        return mrow, doc, rc

    def conform(self, bus: Any, step: int) -> None:
        """``check conform``'s three ``desired-*`` cases, on the target and on
        the author."""
        rp, sec = self.rp, self.sec
        rc, tdoc, terr = zenctl(["check", "conform", TARGET, "worker.v2", "-c", bus.ep1, "--timeout", "1",
                                 "--for", "2"])
        rc2, adoc, aerr = zenctl(["check", "conform", AUTHOR, "tasking.v2", "-c", bus.ep1, "--timeout", "1",
                                  "--for", "2"])
        got = {"target": {c: z_case(tdoc, c) for c in ("desired-report", "desired-binding", "desired-author")},
               "author": {c: z_case(adoc, c) for c in ("desired-report", "desired-binding", "desired-author")},
               "exits": [rc, rc2]}
        print("CONFORM " + json.dumps({"pairing": sc.pairing(), "step": step, **got,
                                       "other": {"target": [(r.get("case"), (r.get("verdict") or {}).get("answer"))
                                                            for r in (tdoc or {}).get("rows") or []
                                                            if (r.get("verdict") or {}).get("answer")
                                                            not in (None, "not_established", "not_asked")],
                                                 "author": [(r.get("case"), (r.get("verdict") or {}).get("answer"),
                                                             r.get("subject"), (r.get("verdict") or {}).get("reason"))
                                                            for r in (adoc or {}).get("rows") or []
                                                            if (r.get("verdict") or {}).get("answer")
                                                            not in (None, "not_established", "not_asked")]}},
                                      sort_keys=True, default=str), flush=True)
        t, a = got["target"], got["author"]
        rp.check(sec, f"{step}. check conform on lab/worker worker.v2: desired-report and desired-binding pass, "
                      "reading its packed report (desired.v2 §2.10)",
                 [x[0] for x in t["desired-report"]] == ["not_established"]
                 and [x[0] for x in t["desired-binding"]] == ["not_established"], f"{t}")
        rp.check(sec, f"{step}. check conform on lab/author tasking.v2: desired-author passes",
                 [x[0] for x in a["desired-author"]] == ["not_established"], f"{a['desired-author']}")


def section9(rp: Report) -> None:
    sec = "§9 a tool's questions" + f" [{sc.pairing()}, zenctl as T]"
    s9 = S9(rp, sec)
    # 1 to 3 and 6, on §1's bus and §4's steps
    bus = sc.Bus(cut=True)
    try:
        author = bus.author(record=False, tokenless=("tasking.v2",))
        target = bus.target(on=2)
        sc._converge(author, target, 1)
        bus.cut()
        s2 = author.put(TARGET, v(2))
        time.sleep(1.0)
        bus.heal()
        ok = _wait(lambda: target.report() == converged(s2, 2), 10.0, 0.05)
        rp.check(sec, "setup 1: after §1's step 4 the target converged on v2", ok, str(target.report()))
        row, doc, rc = s9.ask(bus, 1)
        zr = z_row(doc) or {}
        rp.check(sec, "1. zenctl desired exits 0; its row is profile desired.v2, the slice from the author's "
                      "contract, the author's current an entry at v2's version, written <run>/<seq>",
                 rc == 0 and zr.get("profile") == "desired.v2" and zr.get("slice_from") == "author_contract"
                 and (zr.get("author") or {}).get("current") == "entry" and (zr.get("author") or {}).get("stamp") == s2,
                 f"exit {rc}, {zr.get('profile')}, {zr.get('slice_from')}, {zr.get('author')}, s2 {s2}")
        rp.check(sec, "1. zk2py's tool: the slice from the author's contract, the author's entry at v2's version",
                 row is not None and row.slice == D and row.slice_from == "author_contract"
                 and row.author_current == "entry" and row.author_version == s2,
                 f"{row and (row.slice_from, row.author_current, row.author_version)}")
        s9.conform(bus, 1)
        # §4 step 1: refused, reach at /endpoint
        s3 = author.put(TARGET, v(3, endpoint="tcp/10.0.0.9:7447"))
        _wait(lambda: (target.report() or {}).get("outcome") == "refused", 10.0, 0.05)
        row, doc, rc = s9.ask(bus, 2)
        zrep = (z_row(doc) or {}).get("report") or {}
        ref = (zrep.get("document") or {}).get("refusal") or {}
        words = ("/endpoint", "names how the worker reaches the bus")
        rp.check(sec, "2. zenctl exits 1 (a refused act); its reading of the refusal is pointer 3 and detail 1, "
                      "shown as the target's contract's words, /endpoint and \"names how the worker reaches the "
                      "bus\" (§5: \"read with the target's contract\")",
                 rc == 1 and ref.get("pointer") == 3 and ref.get("detail") == 1 and ref.get("code") == "reach"
                 and (zrep.get("place"), zrep.get("line")) == words,
                 f"exit {rc}, {ref}, place {zrep.get('place')!r}, line {zrep.get('line')!r}")
        rp.check(sec, "2. zk2py's tool reads the same place and line from the target's contract's tables",
                 row is not None and (row.place, row.line) == words, f"{row and (row.place, row.line)}")
        s9.conform(bus, 2)
        # §4 step 5: the drive fails after the rate
        target.cmd("desired fail-next")
        author.put(TARGET, v(4, rate_hz=20, mode="halt"))
        _wait(lambda: (target.report() or {}).get("outcome") == "failed", 10.0, 0.05)
        row, doc, rc = s9.ask(bus, 3)
        rp.check(sec, "3. zenctl exits 1 (a failed act)", rc == 1, f"exit {rc}")
        # 6: v5 converged, the next act held, v6 put
        s5 = author.put(TARGET, v(5))
        _wait(lambda: target.report() == converged(s5, 5), 10.0, 0.05)
        target.cmd("desired hold")
        s6 = author.put(TARGET, v(6))
        time.sleep(0.5)
        row, doc, rc = s9.ask(bus, 6, "once")
        zr = z_row(doc) or {}
        zd = (zr.get("report") or {}).get("document") or {}
        mine_rev = (row.report.report or {}).get("revision") if row is not None and row.report is not None else None
        rp.check(sec, "6. once: the report's observed version is v5's, the author's v6's, in both tools; zenctl "
                      "exits 2, no verdict",
                 rc == 2 and (zd.get("observed") or {}).get("version") == s5
                 and (zr.get("author") or {}).get("stamp") == s6 and row is not None and row.author_version == s6
                 and mine_rev == 5,
                 f"exit {rc}, zenctl observed {zd.get('observed')}, author {zr.get('author')}, zk2py "
                 f"{row and row.author_version}, s5 {s5}, s6 {s6}")
        rc, doc, _ = zenctl(["desired", TARGET, "-c", bus.ep1, "--timeout", "1", "--clocks-synced"])
        zr = z_row(doc) or {}
        rp.check(sec, "6. once, with --clocks-synced: still no verdict, exit 2, and no behind: \"A version has no "
                      "time, so T finds nothing from one reading, whatever clock it trusts\"",
                 rc == 2 and not zr.get("behind") and z_answers(zr) == S9_EXPECT[6], f"exit {rc}, {zr.get('behind')}")
        rc, doc, _ = zenctl(["desired", TARGET, "-c", bus.ep1, "--timeout", "1", "--for", "2"])
        zr = z_row(doc) or {}
        zb = zr.get("behind")
        rp.check(sec, "6. over a 2 s window, zenctl desired --for 2 finds the target behind v6 for at least 2 s, by "
                      "the window, exit 1", rc == 1 and isinstance(zb, dict) and zb.get("by") == "window"
                 and zb.get("version") == s6, f"exit {rc}, behind {zb}, readings {(doc or {}).get('readings')}")
        rc, doc, _ = zenctl(["desired", TARGET, "-c", bus.ep1, "--timeout", "1", "--for", "1", "--clocks-synced"])
        zb = (z_row(doc) or {}).get("behind")
        rp.check(sec, "6. --for 1 --clocks-synced: behind by the window alone, never by the clock",
                 rc == 1 and isinstance(zb, dict) and zb.get("by") == "window", f"exit {rc}, behind {zb}")
        w = tool.watch(bus.g1, TARGET, 2.0, "task")
        rp.check(sec, "6. over a 2 s window zk2py's tool finds the target behind v6, every answer still not yet",
                 w.behind.get("task") == s6 and all(r.answers["converged"].answer == "not_yet" for r in w.rows),
                 f"{w.readings} readings, behind {w.behind}")
        target.cmd("desired release")
    finally:
        bus.close()
    # 4, on §7's bus
    bus = sc.Bus()
    try:
        from .contract import load_contract

        archive = bus.archive()
        author, target = bus.author(), bus.target(archive=sc.ArchiveBox.ADDRESS)
        s1 = sc._converge(author, target, 1)
        _wait(lambda: sc._archived(archive, s1), 5.0, 0.3)
        author.close_cleanly()
        target.kill()
        _wait(lambda: not bus.author_seen(1), 15.0, 0.1)
        new = bus.target(archive=sc.ArchiveBox.ADDRESS)
        ok = _wait(lambda: new.report() == {**converged(s1, 1), "last_known": True}, 10.0, 0.05)
        rp.check(sec, "setup 4: after §7's step 1 the target acted on the archive's v1, last_known true", ok,
                 str(new.report()))
        tasking = load_contract(TASKING, spec_dir=sc.REPO / "spec")
        for label, z_extra, contracts, v1way in (("no-slice", (), (), False),
                                                  ("contracts", ("--contracts", str(TASKING)), (tasking,), False),
                                                  ("v1-way", (), (), True)):
            row, doc, rc = s9.ask(bus, 4, label, z_extra=z_extra, contracts=contracts, from_requirement=v1way,
                                  ask_z=not v1way)
            zr = z_row(doc) or {}
            z_arch = zr.get("archived")
            m_arch = row.archived if row else []
            rp.note(sec, f"4 ({label}): zenctl slice {zr.get('slice')!r} from {zr.get('slice_from')!r}, archived "
                         f"{json.dumps(z_arch)[:300]}; zk2py slice {row and row.slice!r} from "
                         f"{row and row.slice_from!r}, archived {[(a, ver) for a, ver, _ in m_arch]}; exit {rc}")
            shows = bool(m_arch) and all(ver == s1 for _, ver, _ in m_arch)
            if label == "no-slice":
                rp.check(sec, "4 (no-slice). 0.2's Expected 4 (F-604): \"a T with no revision of tasking.v2 in hand "
                              "has no slice. It gives the same four answers, and shows no archive\": neither tool "
                              "has a slice, and neither shows one",
                         row is not None and row.slice is None and zr.get("slice") is None and not m_arch,
                         f"zk2py {row and row.slice}, zenctl {zr.get('slice')}")
            elif label == "contracts":
                rp.check(sec, "4 (contracts). 0.2's Setup (F-604): \"T holds tasking.v2 offline, as §2.17 allows. So "
                              "it has the slice in step 4\": both tools take D from it, and show the archive's v1 as "
                              "last-known, never current (v1's Expected 4)",
                         row is not None and row.slice == D and zr.get("slice") == D and shows
                         and bool(z_arch), f"zk2py {m_arch and [(a, ver) for a, ver, _ in m_arch]}, zenctl {z_arch}")
            else:
                rp.check(sec, "4 (v1-way), beyond the text since 0.2 (\"v1's slice from the target's requirement "
                              "goes\"): D from the target's requirement and binding, as v1's §9 said: zk2py's tool "
                              "shows the archive's v1, and the same four answers",
                         row is not None and row.slice == D and row.slice_from == "requirement" and shows,
                         f"{row and (row.slice, row.slice_from)}")
    finally:
        bus.close()
    # 5, against §8's target
    bus = sc.Bus()
    try:
        author = bus.author()
        author.put(TARGET, v(1))
        t0 = time.monotonic()
        bus.target(unbound=True)
        _wait(lambda: bus.token_put(TARGET, t0) is not None, 10.0, 0.01)
        time.sleep(1.0)
        row, doc, rc = s9.ask(bus, 5)
        rp.check(sec, "5. zenctl desired exits 2: the role not asked", rc == 2, f"exit {rc}")
    finally:
        bus.close()
    # 7, after §15's step 1
    bus = sc.Bus(cut=True)
    try:
        author = bus.author(window=10)
        target = bus.target(on=2)
        s1 = sc._converge(author, target, 1)
        bus.cut()
        author.delete(TARGET)
        time.sleep(15.0)
        author.kill()
        author = bus.author(window=10)
        kind, head, _ = bus.entry(D)
        bus.heal()
        _wait(lambda: any(g["held"] == s1 for g in author.gets()), 10.0, 0.1)
        time.sleep(2.0)
        rp.check(sec, "setup 7: after §15's step 1, the target converged on v1 under R, the author's new instance "
                      "answering a snapshot with no entry under another run",
                 target.report() == converged(s1, 1) and kind == "no_entry"
                 and (d2.Version.parse(head) or d2.Version.parse("0/0")).run != (d2.Version.parse(s1) or
                                                                                 d2.Version.parse("0/0")).run,
                 f"{target.report()}, GET {kind} {head}, s1 {s1}")
        row, doc, rc = s9.ask(bus, 7)
        zr = z_row(doc) or {}
        rp.check(sec, "7. zenctl: the author's current is no_entry under its new run, and exit 2",
                 rc == 2 and (zr.get("author") or {}).get("current") == "no_entry", f"exit {rc}, {zr.get('author')}")
        rp.check(sec, "7. zk2py's tool: the author's no entry under its new run",
                 row is not None and row.author_current == "no_entry", f"{row and (row.author_current, row.author_version)}")
    finally:
        bus.close()
    # beyond §9 (F-585): §3 step 4's revert past the window, read by both tools
    bus = sc.Bus(cut=True)
    try:
        author = bus.author(window=10)
        target = bus.target(on=2)
        sc._converge(author, target, 3)
        bus.cut()
        author.delete(TARGET)
        time.sleep(15.0)
        bus.heal()
        want = {"source": "baseline", "outcome": "reverted", "last_known": False, "in_force": "none"}
        ok = _wait(lambda: target.report() == want, 10.0, 0.05)
        kind, head, _ = bus.entry(D)
        rp.check(sec, "setup F-585: after §3 step 4 the target reverted on a snapshot with no entry under its held "
                      "run; its report has no observed version", ok and kind == "no_entry",
                 f"{target.report()}, GET {kind} {head}")
        s9.ask(bus, "F-585")
    finally:
        bus.close()
    differ = [(c["step"], c["differ"], c["answers_differ"]) for c in s9.compared
              if c.get("differ") or c.get("answers_differ")]
    rp.note(sec, f"the two tools, side by side over {len(s9.compared)} readings: "
                 + ("they differ in " + json.dumps(differ) if differ else "the same answers and the same reading of "
                    "every report, but for run and seq"))


# -- §11: a role on two documents refuses the start --------------------------------------------------

TWO = sc.REPO / "impl" / "python" / "interop" / "desired-v2-two"


def section11(rp: Report) -> None:
    """§11, "As v1's, with tasking.v2 and worker.v2": one router and no
    author. 1. The target given ``desired-v2-two``'s worker.v2, both (its
    role naming tasks and plans), and tasking.v2, two documents: refuses to
    start, exit 1, ``two_documents``, no instance token, no report. 2. The
    control: the scenarios' worker.v2 against the two documents' tasking.v2
    starts, and its report on ``Rp`` is the baseline's."""
    sec = "§11 a role on two documents refuses the start" + f" [{sc.impl_name(sc.TARGET_IMPL)} target]"
    one = sc.REPO / "impl" / "python" / "interop" / "desired-v2" / "worker.v2.toml"
    bus = sc.Bus()
    try:
        for step, worker in (("1", TWO / "worker.v2.toml"), ("2", one)):
            t = time.monotonic()
            if sc.TARGET_IMPL == "zk2py":
                argv = [sys.executable, "-m", "zk2py.desired_v2_owner", "--connect", bus.ep1, "--desired-target",
                        AUTHOR, "--desired-presence-watch", "--desired-worker", str(worker), "--desired-contract",
                        str(TWO / "tasking.v2.toml"), TARGET]
            else:
                argv = [str(BLACKBOX / "owner"), "--connect", bus.ep1, "--desired-v2", "--desired-target", AUTHOR,
                        "--desired-watch", TARGET, str(worker), str(TWO / "tasking.v2.toml")]
            env = dict(os.environ, PYTHONPATH=str(sc.REPO / "impl" / "python"))
            p = subprocess.Popen(argv, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                 text=True, env=env)
            _wait(lambda: p.poll() is not None or bus.token_put(TARGET, t) is not None, 5.0, 0.05)
            if step == "2":
                _wait(lambda: bool(bus.since(t, RP)), 2.0, 0.05)
            code = p.poll()
            token = bus.token_put(TARGET, t)
            reports = bus.since(t, RP)
            if p.poll() is None:
                try:
                    p.stdin.close()
                except OSError:
                    pass
                try:
                    p.wait(10)
                except subprocess.TimeoutExpired:
                    p.kill()
                    p.wait(5)
            err = (p.stderr.read() if p.stderr else "").strip()
            if step == "1":
                rp.check(sec, "1. the role is two_documents, and the target refuses to start: no instance token "
                              "within 5 s, no report on Rp; it stops, exit 1, saying not started: … two_documents",
                         code == 1 and token is None and not reports and "not started" in err
                         and "two_documents" in err,
                         f"exit {code}; token {token}; reports {len(reports)}; {err[-300:]!r}")
            else:
                first = reports[0] if reports else None
                rec = zk2c.parse(first[3], "put").record if first else None
                rp.check(sec, "2. the control, its requirement naming one of the two: S sees its instance token, "
                              "and its report on Rp is the baseline's, 000000",
                         token is not None and rec == bytes(3), f"exit {code}; token {token}; record "
                                                                f"{rec.hex() if rec is not None else None}; "
                                                                f"{err[-200:]!r}")
            time.sleep(0.5)
    finally:
        bus.close()


# -- §12: a config.v1 owner's re-mint is a restart, fed through desired.v2 ------------------------------

def section12(rp: Report) -> None:
    """§12, "As v1's, the author's contract listing desired.v2. The owner's
    report is its own, in its view or its record": zk2py's
    :class:`zk2py.config.ConfigOwner` fed through ``desired.v2``, once
    refusing the re-mint and once re-minting as a restart, against zk2py's
    author of ``interop/desired-v2/configs.v1.toml`` (``configs.v1`` at minor
    2, its ``uses`` ``desired.v2``). v1's runner (``desired_scenarios``)
    holds the steps; this runs them on this major."""
    from . import desired_scenarios as v1sc

    sec = "§12 a config.v1 owner's re-mint is a restart, fed through desired.v2"
    if sc.TARGET_IMPL != "zk2py" or sc.AUTHOR_IMPL != "zk2py":
        rp.cannot(sec, "the runtime's, run with zk2py's config.v1 owner alone: the reference's owner example has no "
                       "config owner fed through desired.v2 (its `config desired <json>` takes the document by "
                       "command)")
        return
    v1sc.section12(v1sc.Ctx(rp, "zk2py", "zk2py", False), major=2)


# -- both majors side by side, read by the reference's tools ----------------------------------------

AUTHOR1, TARGET1 = "lab/author1", "lab/worker1"
D1 = "zk2/lab/author1/tasking.v1/state/tasks/lab/worker1"


class V1Party(sc.Party):
    """A ``desired.v1`` author or target of either implementation: zk2py's
    ``python -m zk2py.desired_owner``, or the reference's owner example with
    no ``--desired-v2``. Its words are the owner example's."""

    def __init__(self, impl: str, endpoint: str, flags: list[str], address: str) -> None:
        self.impl = impl
        exe = [sys.executable, "-m", "zk2py.desired_owner"] if impl == "zk2py" else [str(BLACKBOX / "owner")]
        super().__init__([*exe, "--connect", endpoint, *flags, address], address)

    def report(self) -> Any:
        return self._json("report")


def section_majors(rp: Report) -> None:
    """``desired.v2`` §2.18, "A tool reads both, per role: the report by its
    type identity, and the author's version by its revision's major": one
    deployment, a ``desired.v1`` role (``lab/worker1`` on ``lab/author1``'s
    ``tasking.v1``) beside a ``desired.v2`` one (``lab/worker`` on
    ``lab/author``'s ``tasking.v2``), each pair of one implementation, the
    run's ``--author`` for the v2 pair and the other for the v1 pair. T is
    ``zenctl desired`` with no target, and ``check conform`` on each target
    and author; zk2py's tool reads each target beside it."""
    v1_impl = "rust" if sc.AUTHOR_IMPL == "zk2py" else "zk2py"
    sec = (f"both majors side by side [desired.v1 pair {sc.impl_name(v1_impl)}, desired.v2 pair "
           f"{sc.impl_name(sc.AUTHOR_IMPL)}, zenctl as T]")
    bus = sc.Bus()
    try:
        a1 = V1Party(v1_impl, bus.ep1, ["--desired-author"], AUTHOR1)
        bus.parties.append(a1)
        t1 = V1Party(v1_impl, bus.ep1, ["--desired-target", AUTHOR1, "--desired-watch"], TARGET1)
        bus.parties.append(t1)
        a2 = bus.author(sc.AUTHOR_IMPL)
        t2 = bus.target(sc.AUTHOR_IMPL)
        st1 = a1.put(TARGET1, v(1))
        ok1 = _wait(lambda: (t1.report() or {}).get("outcome") == "converged", 10.0, 0.1)
        s2 = sc._converge(a2, t2, 1)
        rp.check(sec, "setup: each target converged on v1, the v1 one under a Zenoh timestamp, the v2 one under "
                      "<run>/<seq>", ok1 and t2.report() == converged(s2, 1), f"v1 {st1} {t1.report()}; v2 {s2}")
        rc, doc, err = zenctl(["desired", "-c", bus.ep1, "--timeout", "1"])
        r1, r2 = z_row(doc, target=TARGET1) or {}, z_row(doc, target=TARGET) or {}
        print("MAJORS " + json.dumps({"v1_impl": v1_impl, "v2_impl": sc.AUTHOR_IMPL, "exit": rc, "v1": r1, "v2": r2},
                                     sort_keys=True, default=str), flush=True)
        d1 = (r1.get("report") or {}).get("document") or {}
        d2doc = (r2.get("report") or {}).get("document") or {}
        rp.check(sec, "zenctl desired, no target: one row per role, exit 0 (both converged)",
                 rc == 0 and bool(r1) and bool(r2), f"exit {rc}, rows {[(r.get('target'), r.get('profile')) for r in (doc or {}).get('rows') or []]}; {err[-300:]}")
        rp.check(sec, "the v1 role's row: profile desired.v1, its desired.v1.Report as the target put it (its key "
                      "the slice, its stamp <time>/<id>), the author's current a put with a time stamp",
                 r1.get("profile") == "desired.v1" and d1.get("key") == D1
                 and "/" in str((d1.get("observed") or {}).get("stamp", "")) and z_answers(r1) == S9_EXPECT[1],
                 f"{r1.get('profile')}, {json.dumps(d1)[:300]}, {(r1.get('author') or {})}")
        rp.check(sec, "the v2 role's row: profile desired.v2, its packed Report read (observed <run>/<seq>), the "
                      "author's current an entry at that version",
                 r2.get("profile") == "desired.v2" and (d2doc.get("observed") or {}).get("version") == s2
                 and (r2.get("author") or {}).get("current") == "entry" and z_answers(r2) == S9_EXPECT[1],
                 f"{r2.get('profile')}, {d2doc}, {r2.get('author')}")
        for who, iface in ((TARGET1, "worker.v1"), (TARGET, "worker.v2")):
            rc, cdoc, _ = zenctl(["check", "conform", who, iface, "-c", bus.ep1, "--timeout", "1", "--for", "2"])
            rp.check(sec, f"check conform {who} {iface}: desired-report and desired-binding pass, each its major's",
                     [x[0] for x in z_case(cdoc, "desired-report")] == ["not_established"]
                     and [x[0] for x in z_case(cdoc, "desired-binding")] == ["not_established"],
                     f"{z_case(cdoc, 'desired-report')}, {z_case(cdoc, 'desired-binding')}")
        mine1 = tool.read(bus.g1, TARGET1, "task")
        mine2 = tool.read(bus.g1, TARGET, "task")
        rp.check(sec, "zk2py's tool reads each role by its author's major: the v1 role by desired.v1's tool, the v2 "
                      "role by this one, both converged",
                 bool(mine1) and mine1[0].v1 is not None and mine1[0].answers.get("converged") is not None
                 and mine1[0].answers["converged"].answer == "yes" and bool(mine2) and mine2[0].profile == "desired.v2"
                 and mine2[0].pairs() == S9_EXPECT[1],
                 f"v1 {mine1 and {q: (a.answer, a.reason) for q, a in mine1[0].answers.items()}}; v2 "
                 f"{mine2 and mine2[0].pairs()}")
    finally:
        bus.close()

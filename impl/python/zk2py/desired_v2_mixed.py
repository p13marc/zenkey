"""desired.v2 §2.18, "both majors side by side", on real sessions, with each
implementation's author and target (#609, round 119)::

    python -m zk2py.desired_v2_mixed [--only v1-target|v2-target]...

- ``v1-target``: **a target built for desired.v1, under an author of
  desired.v2.** The author's interface is ``tasking.v1`` in a revision whose
  ``uses`` went "from ``desired.v1`` to ``desired.v2``"
  (``interop/desired-mixed/tasking.v1.toml``, minor 1). The target is
  ``worker.v1``'s, built on minor 0. §2.18: "Targets first. A target built
  before this major, bound to an author of it, reads every version as
  unstamped (v1 §2.6). It acts on none. That is safe, and inert." Run
  through a router whose timestamping is off, and through one whose
  timestamping is on, zenoh's default for a router: core §4.2, "Observing
  S1", says such puts then "arrive with the router's zid", and "Its deletes
  and replies arrive with none either way".
- ``v2-target``: **a target built for desired.v2, under an author of
  desired.v1.** The author's interface is ``tasking.v2`` in a revision that
  uses ``desired.v1`` (``interop/desired-mixed/tasking.v2.toml``). The
  target is ``worker.v2``'s: it declares the packed ``Report`` alone. It is
  run **told** (given that revision as its author's) and **not told** (it
  holds the scenarios' ``tasking.v2``, which uses ``desired.v2``, while its
  author's descriptor names the other).

Each party is zk2py's process or the reference's owner example, a black box.
The reference's author is its owner example given the revision's file: with
``--desired-author`` alone for ``tasking.v1``, and with ``--desired-v2`` for
``tasking.v2`` ("a tasking.v2 ... file given is used instead"). What it
writes is read on the wire, not assumed: S records each delivery's Zenoh
timestamp, and G what a GET draws.

A check here states what was seen. Where that is not what §2.18 says, the
check's name says so, and the finding is in ``SPEC-FINDINGS.md``.

``--only s16`` (round 125) runs ``scenarios-v2.md`` §16 as text 0.2 words
it, in each pairing: runs A and B are ``v1-target`` with the reference's
target told ``--desired-word-alone`` ("In §16's runs A and B, the
reference's target reads its author's revision by the deployment's word
alone, as one built before this major does"); run C is ``v2-target`` told,
and its refusal names the role; run D is :func:`run_d`, the same target
told the scenarios' own ``tasking.v2`` while its author runs run C's
revision, and the author's descriptor reaches it.

``--only run-e`` (round 127, text 0.3) runs §16's run E alone, and
``s16`` runs it beside A to D: :func:`run_e`, run D's target under an
author that starts on the scenarios' own revision, then restarts on run
C's from its record. ``--repeat <n>`` runs it n times in each pairing.
``--only run-e-after`` runs it with zk2py's author putting its documents
again after its descriptor (``--desired-after-descriptor``), beside each
target.

``--only run-e-late`` (round 127's mend) runs it with zk2py's author
waiting 1,500 ms between its puts again and its descriptor
(``--desired-descriptor-late-ms``), beside each target: a first read that
cannot find the run stated.

``--cross`` (round 127's mend) runs run E in the two cross pairings alone:
zk2py's author with the reference's target, and the reference's author
with zk2py's target, each author putting before its descriptor, as both
do by default. zk2py's target then says what each hold did (``desired
holds``): desired.v2 0.3's mend, "None states the run yet".
"""

from __future__ import annotations

import argparse
import json
import sys
import threading
import time
from pathlib import Path
from typing import Any, Sequence

from . import desired_v2 as d2, zk2c
from .alarms_scenarios import Report
from .blackbox import BLACKBOX
from .desired_v2_scenarios import AUTHOR, REPO, TARGET, Party, impl_name

MIXED = REPO / "impl" / "python" / "interop" / "desired-mixed"
DOC1 = {"revision": 1, "mode": "run", "rate_hz": 10}
DOC2 = {"revision": 2, "mode": "run", "rate_hz": 20}
BASELINE = {"revision": 0, "mode": "idle", "rate_hz": 1}


class Proc(Party):
    """Any owner process of either implementation, by its argv."""

    def __init__(self, impl: str, argv: list[str], address: str) -> None:
        self.impl = impl
        super().__init__(argv, address)

    def say(self, word: str) -> Any:
        got = self.cmd(f"desired {word}")
        prefix = f"desired {word} "
        if got is None or not got.startswith(prefix):
            return got
        try:
            return json.loads(got[len(prefix):])
        except ValueError:
            return got[len(prefix):]

    def n_acts(self) -> int | None:
        got = self.cmd("desired acts") or ""
        words = got.split(" ")
        return int(words[2]) if len(words) > 2 and words[:2] == ["desired", "acts"] else None


def _py(module: str, endpoint: str, flags: list[str], address: str) -> Proc:
    return Proc("zk2py", [sys.executable, "-m", module, "--connect", endpoint, *flags, address], address)


def _rust(endpoint: str, flags: list[str], address: str, files: Sequence[str] = ()) -> Proc:
    return Proc("rust", [str(BLACKBOX / "owner"), "--connect", endpoint, *flags, address, *files], address)


class Wire:
    """R1, S on the author's state keys, and G."""

    def __init__(self, timestamping: bool) -> None:
        import zenoh

        from . import live
        from .live_interop import _r1

        self.zenoh, self.live = zenoh, live
        self.r1, self.ep, self.zid = _r1(timestamping=timestamping)
        self.lock = threading.Lock()
        #: (kind, key, the Zenoh timestamp's id or None, the in-band version's text or None)
        self.got: list[tuple[str, str, str | None, str | None]] = []
        self.s = live.open_client(self.ep)
        self.sub = self.s.declare_subscriber(f"zk2/{AUTHOR}/*/state/**", zenoh.handlers.Callback(self._s))
        self.g = live.open_client(self.ep)
        self.parties: list[Proc] = []

    def _s(self, smp: Any) -> None:
        rd = zk2c.read_sample(smp)
        ts = smp.timestamp
        with self.lock:
            self.got.append(("delete" if smp.kind == self.zenoh.SampleKind.DELETE else "put", str(smp.key_expr),
                             None if ts is None else str(ts).split("/", 1)[1],
                             None if rd.stamp is None else d2.Version.of(rd.stamp.width, rd.stamp.run,
                                                                         rd.stamp.seq).text))

    def samples(self, key: str) -> list[tuple[str, str | None, str | None]]:
        with self.lock:
            return [(k, ts, ver) for k, kk, ts, ver in self.got if kk == key]

    def get(self, key: str) -> str:
        """What a GET of one key draws: ``snapshot`` (an entry in one),
        ``per_key`` (a reply of its own), ``no_entry`` or ``silent``."""
        st = self.live.get_state(self.g, key, 1.0)
        for r in st.replies:
            if r.key == key and r.refusal is None:
                return "per_key" if r.snapshot_key is None else "snapshot"
        return "no_entry" if any(h.key == key and h.done for h in st.snapshots) else "silent"

    def add(self, p: Proc) -> Proc:
        self.parties.append(p)
        return p

    def close(self) -> None:
        for p in self.parties:
            try:
                p.kill()
            except Exception:  # noqa: BLE001 - closing anyway
                pass
        self.sub.undeclare()
        for x in (self.s, self.g):
            self.live.close(x)
        self.r1.close()


def _drive(w: Wire, author: Proc, target: Proc, key: str) -> dict[str, Any]:
    """Two puts and a delete by the author; what the target says after
    each, and what the wire carried."""
    out: dict[str, Any] = {}
    time.sleep(1.0)
    for name, line in (("put1", f"desired put {TARGET} {json.dumps(DOC1)}"),
                       ("put2", f"desired put {TARGET} {json.dumps(DOC2)}"), ("delete", f"desired delete {TARGET}")):
        out[f"{name}_said"] = author.cmd(line)
        time.sleep(2.0)
        if name == "put1":
            out["get"] = w.get(key)
        out[f"{name}_acts"] = target.n_acts()
        out[f"{name}_report"] = target.say("report")
        out[f"{name}_running"] = target.say("running")
    out["wire"] = w.samples(key)
    return out


def v1_target(rp: Report, a_impl: str, t_impl: str, timestamping: bool, s16: bool = False,
              hlc: bool = False) -> dict[str, Any]:
    sec = (f"{'§16 run ' + ('B' if timestamping else 'A') + ': ' if s16 else ''}a desired.v1 target under a "
           f"desired.v2 author [{impl_name(a_impl)} author, {impl_name(t_impl)} target, the router's timestamping "
           f"{'on' if timestamping else 'off'}{', the target session HLC on' if hlc else ''}]")
    w = Wire(timestamping)
    key = f"zk2/{AUTHOR}/tasking.v1/state/tasks/{TARGET}"
    contract = str(MIXED / "tasking.v1.toml")
    try:
        if a_impl == "zk2py":
            author = w.add(_py("zk2py.desired_v2_owner", w.ep, ["--desired-author", "--desired-contract", contract],
                               AUTHOR))
        else:
            author = w.add(_rust(w.ep, ["--desired-author"], AUTHOR, [contract]))
        if t_impl == "zk2py":
            target = w.add(_py("zk2py.desired_owner", w.ep, ["--desired-target", AUTHOR, "--desired-watch",
                                                             *(["--hlc"] if hlc else [])], TARGET))
        else:
            target = w.add(_rust(w.ep, ["--desired-target", AUTHOR, "--desired-watch",
                                        *(["--desired-word-alone"] if s16 else []), *(["--hlc"] if hlc else [])],
                                 TARGET))
        if s16:
            time.sleep(1.0)
            seed = target.say("report")
            rp.check(sec, "setup: the target starts after the author, and its seed acts on nothing: its report "
                          "{key: D, source: baseline}", target.n_acts() == 0 and isinstance(seed, dict)
                     and seed.get("source") == "baseline" and "observed" not in seed, f"{seed}")
        got = _drive(w, author, target, key)
        puts = [x for x in got["wire"] if x[0] == "put"]
        dels = [x for x in got["wire"] if x[0] == "delete"]
        want_ts = w.zid if timestamping else None
        rp.check(sec, "the author's revision uses desired.v2: its two puts carry an in-band version and no Zenoh "
                      "timestamp of its own (none, or the router's id); its delete carries none; a GET draws a "
                      "snapshot (§2.4, core §4.2 \"Observing S1\")",
                 len(puts) == 2 and all(ts == want_ts and ver for _, ts, ver in puts)
                 and len(dels) == 1 and dels[0][1] is None and got["get"] == "snapshot",
                 f"S {got['wire']}, R1 {w.zid}, GET {got['get']}")
        acts = [got["put1_acts"], got["put2_acts"], got["delete_acts"]]
        rep = got["delete_report"] if isinstance(got["delete_report"], dict) else {}
        obs = (rep.get("observed") or {})
        if hlc:
            # measured, never judged here: §2.18 names the routers alone (F-617)
            rp.note(sec, f"the target's own session stamping, no router stamping: acts {acts}, report {rep}, "
                         f"runs {got['delete_running']}")
            rp.check(sec, "the run ran: two puts and a delete, each read by the target", None not in acts,
                     f"acts {acts}")
        elif not timestamping:
            rp.check(sec, "§2.18 \"reads every version as unstamped (v1 §2.6). It acts on none\": no act after two "
                          "puts and a delete, its report the baseline's, and what it runs the baseline",
                     acts == [0, 0, 0] and rep.get("source") == "baseline" and "observed" not in rep
                     and got["delete_running"] == BASELINE, f"acts {acts}, {rep}")
        else:
            rp.check(sec, "§2.18 (0.2, F-588) \"Where one timestamps\": through a router that timestamps, the "
                          "target acts on both puts, under the router's id; the delete, which carries no timestamp, "
                          "is ignored: it stays converged on the deleted v2, and runs it (§16 Expected 1 to 3)",
                     acts == [1, 2, 2] and rep.get("outcome") == "converged" and obs.get("revision") == 2
                     and str(obs.get("stamp", "")).endswith("/" + w.zid) and got["delete_running"] == DOC2,
                     f"acts {acts}, {rep}, runs {got['delete_running']}")
        return {"acts": acts, "report": rep, "running": got["delete_running"]}
    finally:
        w.close()


def v2_target(rp: Report, a_impl: str, t_impl: str, told: bool, s16: bool = False) -> dict[str, Any]:
    sec = (f"{'§16 run C: ' if s16 and told else ''}a desired.v2 target under a desired.v1 author "
           f"[{impl_name(a_impl)} author, {impl_name(t_impl)} target, {'told' if told else 'not told'} its author's "
           "revision]")
    w = Wire(True)
    key = f"zk2/{AUTHOR}/tasking.v2/state/tasks/{TARGET}"
    contract = str(MIXED / "tasking.v2.toml")
    try:
        if a_impl == "zk2py":
            author = w.add(_py("zk2py.desired_v2_owner", w.ep, ["--desired-author", "--desired-contract", contract],
                               AUTHOR))
        else:
            author = w.add(_rust(w.ep, ["--desired-author", "--desired-v2"], AUTHOR, [contract]))
        if t_impl == "zk2py":
            target = w.add(_py("zk2py.desired_v2_owner", w.ep,
                               ["--desired-target", AUTHOR, "--desired-presence-watch",
                                *(["--desired-contract", contract] if told else [])], TARGET))
        else:
            target = w.add(_rust(w.ep, ["--desired-target", AUTHOR, "--desired-v2", "--desired-watch"], TARGET,
                                 [contract] if told else []))
        if told:
            time.sleep(1.0)
            err = "".join(target.stderr).strip().splitlines()
            rp.check(sec, "told its author's revision uses desired.v1, the target, which declares the packed Report "
                          "alone, does not start: no ready line, exit 1, and no instance token; its refusal names "
                          "its role `task` (§2.18, 0.2: \"It names the role\"; §16 Expected 4)",
                     target.instance is None and target.proc.poll() == 1
                     and not w.live.list_presence(w.g, f"zk2/{TARGET}/@zk/instance/*").instances
                     and bool(err) and "task" in err[-1], f"stderr: {err[-1] if err else ''}")
            return {"started": target.instance is not None, "stderr": err[-1] if err else ""}
        got = _drive(w, author, target, key)
        puts = [x for x in got["wire"] if x[0] == "put"]
        dels = [x for x in got["wire"] if x[0] == "delete"]
        ids = {ts for _, ts, _ in got["wire"]}
        rp.check(sec, "the author's revision uses desired.v1: each put and its delete carry a Zenoh timestamp of the "
                      "author's own, never the router's, and a GET draws a reply per key (v1 §2.4, core §4.3)",
                 len(puts) == 2 and len(dels) == 1 and len(ids) == 1 and None not in ids and w.zid not in ids
                 and got["get"] == "per_key", f"S {got['wire']}, R1 {w.zid}, GET {got['get']}")
        acts = [got["put1_acts"], got["put2_acts"], got["delete_acts"]]
        st = w.live.get_state(w.g, f"zk2/{TARGET}/worker.v2/state/desired/task", 1.0)
        raw = next((r.raw for r in st.replies if r.raw), b"")
        rd = d2.read_put(raw) if raw else None
        fields = rd.report if rd is not None else None
        ver = puts and dels and dels[0][2]
        rp.check(sec, "NOT as §2.18 says (\"It orders a desired.v1 author's versions by v1 §2.6\", \"reported in a "
                      "desired.v1.Report\"): holding the scenarios' tasking.v2, the target orders the author's "
                      "versions by their in-band run and seq, acts on each, and reports them in a packed Report",
                 acts == [1, 2, 3] and fields is not None and fields.get("outcome") == "reverted"
                 and d2.stated(fields) is not None and d2.stated(fields).text == ver and got["delete_running"] == BASELINE,
                 f"acts {acts}, Rp {None if not raw else zk2c.parse(raw, 'put').record.hex()} {fields}")
        return {"acts": acts, "report": fields, "running": got["delete_running"]}
    finally:
        w.close()


def run_d(rp: Report, a_impl: str, t_impl: str) -> dict[str, Any]:
    """§16 run D: worker.v2's target, told the scenarios' own tasking.v2,
    under an author running run C's revision (tasking.v2 using desired.v1),
    whose descriptor reaches the target. Steps 5 and 6; Expected 5 and 6."""
    sec = f"§16 run D: a target that learns its author's major after its start [{impl_name(a_impl)} author, " \
          f"{impl_name(t_impl)} target]"
    w = Wire(True)
    key = f"zk2/{AUTHOR}/tasking.v2/state/tasks/{TARGET}"
    contract = str(MIXED / "tasking.v2.toml")
    rp_key = f"zk2/{TARGET}/worker.v2/state/desired/task"
    run_key = f"zk2/{TARGET}/worker.v2/state/running"
    try:
        if a_impl == "zk2py":
            author = w.add(_py("zk2py.desired_v2_owner", w.ep, ["--desired-author", "--desired-contract", contract],
                               AUTHOR))
        else:
            author = w.add(_rust(w.ep, ["--desired-author", "--desired-v2"], AUTHOR, [contract]))
        time.sleep(0.5)
        heard: list[tuple[str, bytes]] = []
        sub = w.s.declare_subscriber(f"zk2/{TARGET}/worker.v2/state/**", w.zenoh.handlers.Callback(
            lambda smp: heard.append((str(smp.key_expr), smp.payload.to_bytes()))))
        if t_impl == "zk2py":
            target = w.add(_py("zk2py.desired_v2_owner", w.ep, ["--desired-target", AUTHOR, "--desired-presence-watch"],
                               TARGET))
        else:
            target = w.add(_rust(w.ep, ["--desired-target", AUTHOR, "--desired-v2", "--desired-watch"], TARGET))
        time.sleep(2.0)
        seeds = target.cmd("desired seeds") or ""
        words = dict(x.split("=", 1) for x in seeds.split()[3:] if "=" in x)
        st = w.live.get_state(w.g, rp_key, 1.0)
        raw = next((r.raw for r in st.replies if r.key == rp_key and r.raw), b"")
        base = d2.read_put(raw).report if raw else None
        rp.check(sec, "5. the target starts (its word names a revision it can report), reads its author's "
                      "descriptor, and says where it says what its seed found that it names one that uses "
                      "desired.v1; Rp holds the baseline's report",
                 target.instance is not None and str(words.get("revision", "")).startswith("stopped:desired.v1:")
                 and base is not None and base.get("source") == "baseline" and "observed_seq" not in base,
                 f"{seeds!r}, Rp {base}")
        n0 = len(heard)
        acts0 = target.n_acts()
        said = author.cmd(f"desired put {TARGET} {json.dumps(DOC1)}")
        time.sleep(2.5)
        wire = w.samples(key)
        later = heard[n0:]
        rp.check(sec, "6. the author puts v1: the target acts on nothing; Rp and Run receive nothing (§2.18, 0.2: "
                      "\"While the descriptor names it, a target MUST NOT act on a version of that author's\")",
                 bool(said) and any(k == "put" for k, _, _ in wire) and target.n_acts() == acts0 == 0
                 and not [x for x in later if x[0] in (rp_key, run_key)],
                 f"author {said!r}, acts {acts0} -> {target.n_acts()}, after the put {[(k.rsplit('/', 1)[1], v.hex()) for k, v in later]}, "
                 f"seeds {target.cmd('desired seeds')!r}")
        sub.undeclare()
        return {"seeds": seeds, "report": base, "after": later}
    finally:
        w.close()


def _descriptor_at_first_put(w: Wire, key: str, after: set[str]) -> dict[str, Any]:
    """What a read of the author's descriptors draws at the first put on
    ``key`` under a run not in ``after``, asked from G the moment S hears
    it, as a target that holds the put would ask (§2.18, 0.3): the runs the
    descriptors state, and how long the read took. Set going before the
    author restarts; read with :func:`_wait_first`."""
    out: dict[str, Any] = {}
    done = threading.Event()

    def ask(run: str, t0: float) -> None:
        from . import freshness_v2 as f2

        docs = f2.descriptors_of(w.g, AUTHOR, 1.0)
        out.update({"run": run, "stated": [d.get("run") for d in docs], "ms": round((time.monotonic() - t0) * 1000)})
        done.set()

    def heard(smp: Any) -> None:
        if done.is_set() or "run" in out:
            return
        rd = zk2c.read_sample(smp)
        if rd.stamp is None:
            return
        run = d2.Version.of(rd.stamp.width, rd.stamp.run, rd.stamp.seq).run.text
        if run in after:
            return
        out["run"] = run
        threading.Thread(target=ask, args=(run, time.monotonic()), daemon=True).start()

    sub = w.g.declare_subscriber(key, w.zenoh.handlers.Callback(heard))
    out["_sub"], out["_done"] = sub, done
    return out


def run_e(rp: Report, a_impl: str, t_impl: str, tmp: str, after: bool = False, late_ms: int = 0) -> dict[str, Any]:
    """§16 run E (desired.v2 0.3), each run in a directory of its own: run D's target, told the scenarios' own
    tasking.v2; its author starts on that revision, then ends without
    closing and starts again on run C's revision (tasking.v2 using
    desired.v1), under a new run, from its record, and puts v1 again and
    then v2. Steps 7 and 8; Expected 7 and 8: "the target judges nothing of
    the new run before it reads the descriptor stating that run (§2.18). It
    names run C's revision, so the role stops. The target says so, and acts
    on neither put. `Rp` and `Run` receive nothing."

    Beside it, G reads the author's descriptors the moment S hears the new
    run's first put: whether a target that holds that put then finds the
    new run stated (core §8.2 puts a state's value before step 4, and
    names no place for it beside step 3)."""
    import tempfile

    sec = (f"§16 run E: a new run read first [{impl_name(a_impl)} author"
           f"{', its puts again after its descriptor' if after else ''}"
           f"{f', its descriptor {late_ms} ms after its puts again' if late_ms else ''}, {impl_name(t_impl)} target]")
    w = Wire(True)
    key = f"zk2/{AUTHOR}/tasking.v2/state/tasks/{TARGET}"
    run_c = str(MIXED / "tasking.v2.toml")
    rp_key = f"zk2/{TARGET}/worker.v2/state/desired/task"
    run_key = f"zk2/{TARGET}/worker.v2/state/running"
    d = tempfile.mkdtemp(prefix=f"run-e-{a_impl}-{t_impl}-", dir=tmp)
    rec, runs = str(Path(d) / "author.record.json"), str(Path(d) / "author.run.json")

    def author(files: Sequence[str]) -> Proc:
        if a_impl == "zk2py":
            return w.add(_py("zk2py.desired_v2_owner", w.ep,
                             ["--desired-author", "--desired-record", rec, "--desired-run-record", runs,
                              *(["--desired-after-descriptor"] if after else []),
                              *(["--desired-descriptor-late-ms", str(late_ms)] if late_ms else []),
                              *(["--desired-contract", files[0]] if files else [])], AUTHOR))
        return w.add(_rust(w.ep, ["--desired-author", "--desired-v2", "--desired-record", rec, "--run-file", runs],
                           AUTHOR, files))

    try:
        a1 = author(())
        time.sleep(0.5)
        heard: list[tuple[float, str, bytes]] = []
        sub = w.s.declare_subscriber(f"zk2/{TARGET}/worker.v2/state/**", w.zenoh.handlers.Callback(
            lambda smp: heard.append((time.monotonic(), str(smp.key_expr), smp.payload.to_bytes()))))
        d_at: list[tuple[float, str | None]] = []

        def on_d(smp: Any) -> None:
            rd = zk2c.read_sample(smp)
            d_at.append((time.monotonic(), None if rd.stamp is None else
                         d2.Version.of(rd.stamp.width, rd.stamp.run, rd.stamp.seq).text))

        sub_d = w.s.declare_subscriber(key, w.zenoh.handlers.Callback(on_d))
        tokens: list[tuple[float, str, str]] = []
        sub_t = w.s.liveliness().declare_subscriber(f"zk2/{AUTHOR}/@zk/instance/*", w.zenoh.handlers.Callback(
            lambda smp: tokens.append((time.monotonic(), "put" if smp.kind == w.zenoh.SampleKind.PUT else "delete",
                                       str(smp.key_expr).rsplit("/", 1)[1][:8]))))
        if t_impl == "zk2py":
            target = w.add(_py("zk2py.desired_v2_owner", w.ep, ["--desired-target", AUTHOR, "--desired-presence-watch"],
                               TARGET))
        else:
            target = w.add(_rust(w.ep, ["--desired-target", AUTHOR, "--desired-v2", "--desired-watch"], TARGET))
        time.sleep(1.5)
        said1 = a1.cmd(f"desired put {TARGET} {json.dumps(DOC1)}")
        time.sleep(2.0)
        acts7 = target.n_acts()
        st = w.live.get_state(w.g, rp_key, 1.0)
        raw = next((r.raw for r in st.replies if r.key == rp_key and r.raw), b"")
        rep7 = d2.read_put(raw).report if raw else None
        runs7 = {v.split("/")[0] for k, _, v in w.samples(key) if k == "put" and v}
        rp.check(sec, "7. the target starts, the author puts v1: the target acts on v1, and the report is converged "
                      "on it",
                 bool(said1) and acts7 == 1 and rep7 is not None and rep7.get("outcome") == "converged"
                 and target.say("running") == DOC1, f"author {said1!r}, acts {acts7}, Rp {rep7}")
        seeds7 = target.cmd("desired seeds") or ""
        probe = _descriptor_at_first_put(w, key, runs7)
        a1.kill()
        t8 = time.monotonic()
        a2 = author((run_c,))
        time.sleep(1.5)
        said2 = a2.cmd(f"desired put {TARGET} {json.dumps(DOC2)}")
        time.sleep(3.0)
        probe["_done"].wait(3.0)
        probe.pop("_sub").undeclare()
        probe.pop("_done")
        wire = [(k, ts, v) for k, ts, v in w.samples(key)]
        new_puts = [v for k, _, v in wire if k == "put" and v and v.split("/")[0] not in runs7]
        later = [(k.rsplit("/", 1)[1], b.hex()) for t, k, b in heard if t >= t8 and k in (rp_key, run_key)]
        seeds8 = target.cmd("desired seeds") or ""
        holds = target.cmd("desired holds") if t_impl == "zk2py" else None
        words = dict(x.split("=", 1) for x in seeds8.split()[3:] if "=" in x)
        acts8 = target.n_acts()
        rp.check(sec, "8. the author restarts on run C's revision under a new run, from its record, and puts v1 "
                      "again and then v2: the target reads the descriptor stating that run before it judges either, "
                      "names run C's revision and stops the role; it says so where it says what its seed found, "
                      "acts on neither put, and Rp and Run receive nothing (§2.18, 0.3)",
                 a2.instance is not None and len(new_puts) >= 2 and bool(said2) and acts8 == acts7 and not later
                 and str(words.get("revision", "")).startswith("stopped:desired.v1:")
                 and target.say("running") == DOC1,
                 f"author {said2!r}, the new run's puts {new_puts}, acts {acts7} -> {acts8}, after the restart "
                 f"{later}, seeds {seeds7!r} -> {seeds8!r}")
        rp.note(sec, f"G's descriptor read at the new run's first put: {probe}")
        if holds is not None:
            rp.note(sec, f"zk2py's target, what each hold did: {holds}")
        ms = lambda t: round((t - t8) * 1000)  # noqa: E731
        rp.note(sec, "after the kill, ms: the new run's puts on D "
                     f"{[(ms(t), v) for t, v in d_at if t >= t8]}, the author's instance tokens "
                     f"{[(ms(t), k, i) for t, k, i in tokens if t >= t8]}, Rp and Run "
                     f"{[(ms(t), k.rsplit('/', 1)[1]) for t, k, _ in heard if t >= t8]}")
        sub_d.undeclare()
        sub_t.undeclare()
        sub.undeclare()
        return {"seeds": seeds8, "acts": [acts7, acts8], "after": later, "probe": probe, "puts": new_puts,
                "holds": holds}
    finally:
        w.close()


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="python -m zk2py.desired_v2_mixed", description=__doc__.split("\n")[0])
    ap.add_argument("--only", action="append", choices=["v1-target", "v2-target", "s16", "run-e", "run-e-after", "run-e-late",
                                                       "hlc"])
    ap.add_argument("--repeat", type=int, default=1, help="run E this many times in each pairing")
    ap.add_argument("--cross", action="store_true", help="run E in the two cross pairings alone")
    args = ap.parse_args(argv)
    rp = Report()
    only = args.only or ["v1-target", "v2-target"]
    impls = ("zk2py", "rust")
    runs: list[tuple[Any, tuple[Any, ...]]] = []
    if "v1-target" in only:
        runs += [(v1_target, (a, t, ts)) for ts in (False, True) for a in impls for t in impls]
    if "v2-target" in only:
        runs += [(v2_target, (a, t, told)) for told in (False, True) for a in impls for t in impls]
    if "s16" in only:
        runs += [(v1_target, (a, t, ts, True)) for ts in (False, True) for a in impls for t in impls]
        runs += [(v2_target, (a, t, True, True)) for a in impls for t in impls]
        runs += [(run_d, (a, t)) for a in impls for t in impls]
    if "s16" in only or "run-e" in only:
        import os
        import tempfile

        tmp = tempfile.mkdtemp(prefix="zk2py-run-e-", dir=os.environ.get("ZK2PY_SCRATCH"))
        pairs = [("zk2py", "rust"), ("rust", "zk2py")] if args.cross else [(a, t) for a in impls for t in impls]
        runs += [(run_e, (a, t, tmp)) for _ in range(args.repeat) for a, t in pairs]
    if "run-e-after" in only:
        import os
        import tempfile

        tmp = tempfile.mkdtemp(prefix="zk2py-run-e-", dir=os.environ.get("ZK2PY_SCRATCH"))
        runs += [(run_e, ("zk2py", t, tmp, True)) for _ in range(args.repeat) for t in impls]
    if "run-e-late" in only:
        import os
        import tempfile

        tmp = tempfile.mkdtemp(prefix="zk2py-run-e-", dir=os.environ.get("ZK2PY_SCRATCH"))
        runs += [(run_e, ("zk2py", t, tmp, False, 1500)) for _ in range(args.repeat) for t in impls]
    if "hlc" in only:
        # round 125 (F-617): run A's router, the target's own session's HLC on
        runs += [(v1_target, (a, t, False, True, True)) for a in impls for t in impls]
    for fn, a in runs:
        try:
            fn(rp, *a)
        except Exception as e:  # noqa: BLE001 - a case that raises fails, and the rest run
            import traceback

            traceback.print_exc()
            rp.check(f"{fn.__name__} {a}", "the case ran to its end", False, f"{type(e).__name__}: {e}")
    passed = sum(1 for r in rp.rows if r[2])
    print(f"{passed} of {len(rp.rows)} checks passed; not run: {len(rp.not_run)}", flush=True)
    return 0 if passed == len(rp.rows) else 1


if __name__ == "__main__":
    sys.exit(main())

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
"""

from __future__ import annotations

import argparse
import json
import sys
import threading
import time
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


def v1_target(rp: Report, a_impl: str, t_impl: str, timestamping: bool) -> dict[str, Any]:
    sec = (f"a desired.v1 target under a desired.v2 author [{impl_name(a_impl)} author, {impl_name(t_impl)} target, "
           f"the router's timestamping {'on' if timestamping else 'off'}]")
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
            target = w.add(_py("zk2py.desired_owner", w.ep, ["--desired-target", AUTHOR, "--desired-watch"], TARGET))
        else:
            target = w.add(_rust(w.ep, ["--desired-target", AUTHOR, "--desired-watch"], TARGET))
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
        if not timestamping:
            rp.check(sec, "§2.18 \"reads every version as unstamped (v1 §2.6). It acts on none\": no act after two "
                          "puts and a delete, its report the baseline's, and what it runs the baseline",
                     acts == [0, 0, 0] and rep.get("source") == "baseline" and "observed" not in rep
                     and got["delete_running"] == BASELINE, f"acts {acts}, {rep}")
        else:
            rp.check(sec, "NOT as §2.18 says: through a router that timestamps, the target acts on both puts, under "
                          "the router's id; the delete, which carries no timestamp, is ignored: it stays converged "
                          "on the deleted v2, and runs it",
                     acts == [1, 2, 2] and rep.get("outcome") == "converged" and obs.get("revision") == 2
                     and str(obs.get("stamp", "")).endswith("/" + w.zid) and got["delete_running"] == DOC2,
                     f"acts {acts}, {rep}, runs {got['delete_running']}")
        return {"acts": acts, "report": rep, "running": got["delete_running"]}
    finally:
        w.close()


def v2_target(rp: Report, a_impl: str, t_impl: str, told: bool) -> dict[str, Any]:
    sec = (f"a desired.v2 target under a desired.v1 author [{impl_name(a_impl)} author, {impl_name(t_impl)} target, "
           f"{'told' if told else 'not told'} its author's revision]")
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
                          "alone, does not start: no ready line, exit 1, and no instance token",
                     target.instance is None and target.proc.poll() == 1
                     and not w.live.list_presence(w.g, f"zk2/{TARGET}/@zk/instance/*").instances,
                     f"stderr: {err[-1] if err else ''}")
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


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="python -m zk2py.desired_v2_mixed", description=__doc__.split("\n")[0])
    ap.add_argument("--only", action="append", choices=["v1-target", "v2-target"])
    args = ap.parse_args(argv)
    rp = Report()
    only = args.only or ["v1-target", "v2-target"]
    impls = ("zk2py", "rust")
    runs: list[tuple[Any, tuple[Any, ...]]] = []
    if "v1-target" in only:
        runs += [(v1_target, (a, t, ts)) for ts in (False, True) for a in impls for t in impls]
    if "v2-target" in only:
        runs += [(v2_target, (a, t, told)) for told in (False, True) for a in impls for t in impls]
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

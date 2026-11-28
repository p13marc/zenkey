"""``alarms.v2``'s scenarios (``spec/profiles/alarms/scenarios-v2.md``, text
0.1) live, in both cross pairings (#609, round 124)::

    python -m zk2py.alarms_v2_scenarios --pairing ref-owner|zk2py-owner [--only 1|…|12]... [--out <dir>]

- **``ref-owner``:** the reference's owner example (``owner --alarms-v2``, a
  black box) is every owner; zk2py's S, G and T read it
  (:mod:`zk2py.alarms_v2_live`).
- **``zk2py-owner``:** zk2py's owner process (:mod:`zk2py.alarms_v2_owner`)
  is every ``alarms.v2`` owner, driven by the same words; the reference's
  ``zenctl alarms`` and ``check conform`` read it. zk2py's S still hears
  every put, so each section's bytes are kept for the comparison.

``lab/old`` (an ``alarms.v1`` owner) is the reference's owner example in
both, and ``lab/liar`` (§9 step 5) zk2py's owner in both: the example's
usage has no word for a record put on purpose. Every router is
zenoh-python's. Each run writes ``<out>/<pairing>.json``: every check, and
every put S heard, by section and step.

Exit 0 when every check passes, 1 when any fails.
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
from pathlib import Path
from typing import Any

from . import alarms as v1
from . import alarms_v2 as a2
from . import alarms_v2_live as al
from .blackbox import BLACKBOX

REPO = Path(__file__).resolve().parents[3]
INTEROP = REPO / "impl" / "python" / "interop" / "alarms-v2"
TC, TC_LATER, TC_EARLIER = INTEROP / "tc.v1.toml", INTEROP / "tc.v1-later.toml", INTEROP / "tc.v1-earlier.toml"
ZENCTL, OWNER = BLACKBOX / "zenctl", BLACKBOX / "owner"
NS = 1_000_000_000
PAIRING = "ref-owner"
A = {"rule": "link_down", "severity": "warning", "summary": "Link to r2 on eth0 is down",
     "labels": {"peer": "r2", "port": "eth0"}}
B = {"rule": "door_open", "severity": "warning", "summary": "Enclosure door open"}
C = {"rule": "link_down", "severity": "warning", "summary": "Link to r3 on eth1 is down",
     "labels": {"peer": "r3", "port": "eth1"}}
KA, KB, KC, KH = "a659f813308ad1da", "abb089906038056a", "f3ba0819e7b57762", "335f24192ff5d0d4"


class Report:
    def __init__(self) -> None:
        self.rows: list[dict[str, Any]] = []
        self.puts: dict[str, list[dict[str, Any]]] = {}

    def check(self, section: str, name: str, ok: bool, detail: Any = "") -> bool:
        self.rows.append({"section": section, "check": name, "ok": bool(ok), "detail": detail})
        print(f"{'ok  ' if ok else 'FAIL'} {section}: {name}" + (f"\n       {detail}" if detail not in ("", None) else ""),
              flush=True)
        return bool(ok)

    def note(self, section: str, text: str) -> None:
        """A measurement kept beside the checks, and not counted."""
        self.rows.append({"section": section, "note": text, "ok": True, "counted": False})
        print(f"NOTE {section}: {text}", flush=True)

    def keep(self, section: str, step: str, listener: al.Listener, since: int) -> list[dict[str, Any]]:
        """Every put S heard since ``since``: key, kind, payload in hex, and
        the record read (or the words)."""
        out = []
        for x in listener.all(since):
            if "/alarms.v2/state/" not in x["key"]:
                continue
            item = {"step": step, "key": x["key"].split("/alarms.v2/state/", 1)[1], "kind": x["kind"],
                    "hex": x["wire"].hex(), "bytes": len(x["wire"])}
            if "/state/alarms/" in x["key"] and x["kind"] == "put":
                r = a2.read_put(x["wire"])
                item["record"] = r.value if r.reads else {"undecodable": r.undecodable}
            elif "/state/words/" in x["key"] and x["kind"] == "put":
                item["words"] = a2.read_words_put(x["wire"])
            out.append(item)
        self.puts.setdefault(section, []).extend(out)
        return out


# -- the owners -----------------------------------------------------------------------------

class OwnerProc:
    """One owner process: the reference's owner example (``rust``) or zk2py's
    (``zk2py``), driven by the example's words. ``v1`` starts the example
    with ``--alarms`` in place of ``--alarms-v2``."""

    def __init__(self, impl: str, endpoint: str, address: str, *, contracts: list[Path] = (),
                 record: str | None = None, rules: str | None = None, bound: int | None = None,
                 tokenless: list[str] = (), run_file: str | None = None, v1_owner: bool = False,
                 first_hop: bool = False):
        self.impl, self.address = impl, address
        flags: list[str] = ["--first-hop"] if first_hop else []
        if record:
            flags += ["--alarms-record", record]
        if rules:
            flags += ["--alarms-rules", rules]
        if bound is not None:
            flags += ["--cardinality", f"{'alarms.v1' if v1_owner else 'alarms.v2'}:state/alarms/"
                                       f"{{{'alert_key' if v1_owner else 'slot'}}}={bound}"]
        for t in tokenless:
            flags += ["--tokenless", t]
        if run_file:
            flags += ["--run-file", run_file]
        if impl == "rust":
            cmd = [str(OWNER), "--connect", endpoint, "--alarms" if v1_owner else "--alarms-v2", *flags, address,
                   *map(str, contracts)]
            env = None
        else:
            cmd = [sys.executable, "-m", "zk2py.alarms_v2_owner", "--connect", endpoint, *flags, address,
                   *map(str, contracts)]
            env = dict(os.environ, PYTHONPATH=str(REPO / "impl" / "python"))
        self.p = subprocess.Popen(cmd, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                  text=True, env=env)
        self.lines: queue.Queue[str] = queue.Queue()
        self.err: list[str] = []
        threading.Thread(target=self._pump, args=(self.p.stdout, self.lines.put), daemon=True).start()
        threading.Thread(target=self._pump, args=(self.p.stderr, self.err.append), daemon=True).start()
        self.ready = ""
        deadline = time.monotonic() + 30
        while time.monotonic() < deadline:
            try:
                line = self.lines.get(timeout=0.5)
            except queue.Empty:
                if self.p.poll() is not None:
                    break
                continue
            if line.startswith("ready"):
                self.ready = line
                break
        if not self.ready:
            raise RuntimeError(f"{impl} {address}: no ready line; stderr {self.err[-5:]}")
        self.instance = self.ready.split()[1].rsplit("/", 1)[1]

    @staticmethod
    def _pump(stream: Any, sink: Any) -> None:
        for line in stream:
            sink(line.rstrip("\n"))

    def cmd(self, line: str, timeout: float = 10.0) -> str:
        assert self.p.stdin is not None
        self.p.stdin.write(line + "\n")
        self.p.stdin.flush()
        try:
            return self.lines.get(timeout=timeout)
        except queue.Empty:
            return ""

    def raise_(self, a: dict[str, Any], **over: Any) -> dict[str, Any]:
        got = self.cmd("alarm raise " + json.dumps({**a, **over}))
        return parse_answer(got)

    def resolve(self, key: str) -> dict[str, Any]:
        return parse_answer(self.cmd(f"alarm resolve {key}"))

    def kill(self) -> None:
        self.p.kill()
        self.p.wait(10)

    def close(self) -> None:
        if self.p.poll() is not None:
            return
        try:
            if self.impl == "zk2py":
                self.p.stdin.write("close\n")
                self.p.stdin.flush()
            self.p.stdin.close()
            self.p.wait(15)
        except Exception:  # noqa: BLE001 - a process already gone
            self.p.kill()


def parse_answer(line: str) -> dict[str, Any]:
    """``alarm <act> <alert_key> [<slot>] <json>``, ``alarm none <key>`` or
    ``refused …``."""
    w = line.split(" ", 4)
    if not line.startswith("alarm "):
        return {"act": "refused" if line.startswith("refused") else "?", "line": line}
    if w[1] == "none":
        return {"act": "none", "key": w[2]}
    out: dict[str, Any] = {"act": w[1], "key": w[2]}
    if len(w) > 3 and w[3].isdigit():
        out["slot"] = int(w[3])
        out.update(json.loads(w[4]) if len(w) > 4 else {})
    elif len(w) > 3:
        out["document"] = json.loads(" ".join(w[3:]))
    return out


class Bus:
    def __init__(self, report: Report, section: str) -> None:
        from . import live
        from .live_interop import _r1

        self.report, self.section = report, section
        self.router, self.endpoint, _ = _r1()
        self.s = live.open_client(self.endpoint)
        self.g = live.open_client(self.endpoint)
        self.procs: list[OwnerProc] = []
        self.listeners: list[al.Listener] = []
        self.tmp = tempfile.mkdtemp(prefix="zk2py-alarms-v2-", dir=os.environ.get("ZK2PY_SCRATCH"))

    def owner(self, address: str, impl: str | None = None, **kw: Any) -> OwnerProc:
        impl = impl or ("rust" if PAIRING == "ref-owner" else "zk2py")
        kw.setdefault("run_file", os.path.join(self.tmp, address.replace("/", "_") + ".run"))
        p = OwnerProc(impl, self.endpoint, address, **kw)
        self.procs.append(p)
        return p

    def listener(self, address: str, contract: Any = None) -> al.Listener:
        x = al.Listener(self.s, address, contract)
        self.listeners.append(x)
        return x

    def zenctl(self, *args: str, timeout: float = 60) -> tuple[int, Any, str]:
        """``zenctl alarms --format json`` on this bus: (exit, document, stderr)."""
        z = subprocess.run([str(ZENCTL), "alarms", "-c", self.endpoint, "--format", "json", "--clocks-synced",
                            "--timeout", "2", *args], capture_output=True, text=True, timeout=timeout)
        try:
            doc = json.loads(z.stdout) if z.stdout.strip() else None
        except ValueError:
            doc = {"unparsed": z.stdout[-2000:]}
        return z.returncode, doc, z.stderr

    def close(self) -> None:
        for p in self.procs:
            p.close()
        for x in self.listeners:
            try:
                x.close()
            except Exception:  # noqa: BLE001
                pass
        for s in (self.s, self.g, self.router):
            s.close()


def held_revisions(*files: Path) -> dict[str, Any]:
    from .contract import load_contract

    out = {}
    for f in files:
        c = load_contract(f)
        out[c.fingerprint] = c.canonical
    return out


def zen_alarms(doc: Any) -> list[dict[str, Any]]:
    """zenctl alarms' ``rows`` of ``row`` ``alarm``."""
    return [] if not isinstance(doc, dict) else [r for r in doc.get("rows") or [] if r.get("row") == "alarm"]


def zen_owner(doc: Any, address: str) -> dict[str, Any] | None:
    for r in (doc or {}).get("rows") or [] if isinstance(doc, dict) else []:
        if r.get("row") == "owner" and r.get("address") == address:
            return r
    return None


def zen_of(doc: Any, slot: int, service: str = "svc") -> dict[str, Any] | None:
    for x in zen_alarms(doc):
        if x.get("slot") == slot and (x.get("alarm") or {}).get("service", service) == service:
            return x
    return None


def pair(x: Any) -> Any:
    return None if x is None else {"answer": x.get("answer"), "reason": x.get("reason")}


def rec_core(r: dict[str, Any] | None) -> Any:
    """A record but its time: what two owners' records of one state share."""
    return None if r is None else {k: v for k, v in r.items() if k != "at"}


# -- the sections ---------------------------------------------------------------------------

def section1(rep: Report) -> None:
    sec = "§1"
    bus = Bus(rep, sec)
    try:
        S = bus.listener("lab/svc")
        o = bus.owner("lab/svc", contracts=[TC])
        time.sleep(1.0)
        held = held_revisions(TC)
        col = al.collection_read(bus.g, "lab/svc")
        desc_run = al.read_owner(bus.g, "lab/svc", held).run
        snap = col.snapshot()
        rep.check(sec, "Expected 1: G's GET is one page of 16 bytes, no entry, collection 0, under the descriptor's run",
                  [len(col.page_bytes), [len(b) for b in col.page_bytes], snap and snap["live"], col.pages[0]["coll"]
                   if col.pages else None, col.run_hex() == desc_run] == [1, [16], [], 0, True],
                  {"pages": [b.hex() for b in col.page_bytes], "run": col.run_hex(), "descriptor_run": desc_run})
        t = time.monotonic_ns()
        ans = o.raise_(A)
        time.sleep(0.8)
        puts = rep.keep(sec, "2 raise A", S, t)
        rec = puts[0].get("record") if puts else None
        rep.check(sec, "Expected 2: one put on alarms/0, 18 bytes, CH 0x0b, a 12-byte record: firing, warning, rule 1, "
                  "labels 2, argument not_available, no key; nothing on words/0",
                  [[p["key"] for p in puts], [p["bytes"] for p in puts], puts[0]["hex"][:2] if puts else None,
                   rec_core(rec)] == [["alarms/0"], [18], "0b",
                                      {"state": "firing", "severity": "warning", "rule": 1, "labels": 2,
                                       "argument": "not_available"}], {"puts": puts, "answer": ans})
        at_ok = rec is not None and "at" in rec and abs(rec["at"] / 1000 - time.time()) < 3
        rep.check(sec, "Expected 2: at within 1 s of the step", at_ok, rec)
        steps = al.read_slots(S, a2.Table(tuple(held[next(iter(held))]["resources"][0]["annotations"]["alarms.rules"])))
        rep.check(sec, "Expected 2: S reads raised", [st.expect() for _, st in steps.get(0, [])] == [["raised"]])
        r = al.read_owner(bus.g, "lab/svc", held)
        j = r.slots.get(0)
        g_ent = (r.collection.snapshot() or {}).get("live", []) if r.collection else []
        if PAIRING == "ref-owner":
            rep.check(sec, "Expected 2: G's snapshot lists one entry, id 0, the same record, vouched for at the "
                      "snapshot's time; G judges yes, firing",
                      [[e["id"] for e in g_ent], [e["record"] for e in g_ent] == [rec],
                       [e["confirmation"] for e in g_ent], j and j.firing.pair()]
                      == [["0"], True, ["time"], {"answer": "yes", "reason": "firing"}],
                      {"entries": g_ent, "verdict": str(r.verdict)})
        else:
            code, doc, err = bus.zenctl("--contracts", str(TC), "zk2/lab/svc/**")
            z = zen_of(doc, 0)
            rep.check(sec, "Expected 2 (zenctl alarms as G): slot 0 judged yes, firing, named link_down",
                      [pair((z or {}).get("firing")), ((z or {}).get("words") or {}).get("rule")]
                      == [{"answer": "yes", "reason": "firing"}, "link_down"], {"exit": code, "alarm": z})
        t = time.monotonic_ns()
        ans3 = o.raise_(A)
        time.sleep(0.8)
        rep.check(sec, "Expected 3: a duplicate observation puts nothing", rep.keep(sec, "3 raise A again", S, t) == [],
                  ans3)
        t = time.monotonic_ns()
        o.raise_(B)
        time.sleep(0.8)
        puts = rep.keep(sec, "4 raise B", S, t)
        rep.check(sec, "Expected 4: one put on alarms/1, 15 bytes: a 9-byte record, rule 3, no labels",
                  [[p["key"] for p in puts], [p["bytes"] for p in puts], [rec_core(p.get("record")) for p in puts]]
                  == [["alarms/1"], [15], [{"state": "firing", "severity": "warning", "rule": 3,
                                            "argument": "not_available"}]], puts)
        old = bus.owner("lab/old", impl="rust", v1_owner=True)
        old.raise_(A)
        time.sleep(1.0)
        if PAIRING == "ref-owner":
            r_svc = al.read_owner(bus.g, "lab/svc", held)
            r_old = v1.read_owner(bus.g, "lab/old", trust=v1.fr.ClockTrust(word=True))
            ans = al.any_firing("warning", [r_svc, r_old.owner_read()])
            rep.check(sec, "Expected 5: lab/svc read by alarms.v2, lab/old by alarms.v1; yes, any_firing, 3",
                      [r_svc.major, ans["answer"], ans["reason"], ans["firing"]] == ["alarms.v2", "yes", "any_firing", 3],
                      ans)
        else:
            code, doc, err = bus.zenctl("--contracts", str(TC), "zk2/lab/**")
            af = (doc or {}).get("any_firing") or {}
            owners = sorted((str(x.get("address")), str(x.get("profile")), str(x.get("verdict")))
                            for x in (doc or {}).get("rows") or [] if x.get("row") == "owner")
            rep.check(sec, "Expected 5 (zenctl alarms over zk2/lab/**): yes, any_firing, 3, both owners established",
                      [code, af.get("answer"), af.get("reason"), af.get("firing"), af.get("established")]
                      == [1, "yes", "any_firing", 3, 2], {"any_firing": af, "owners": owners})
            conform(bus, rep, sec, "lab/svc", lambda: (o.raise_(C), o.resolve(KC)))
    finally:
        bus.close()


def judged(bus: Bus, address: str, held_files: list[Path], **kw: Any) -> dict[str, Any]:
    """The pairing's reader: zk2py's (``ref-owner``) or ``zenctl alarms``
    (``zk2py-owner``) over one owner -> its verdict, and each slot's first
    and last answers, its record and how it is shown."""
    if PAIRING == "ref-owner":
        r = al.read_owner(bus.g, address, held_revisions(*held_files), **kw)
        return {"verdict": None if r.verdict is None else r.verdict.expect(),
                "slots": {s: {"firing": j.firing.pair(), "stale": j.stale.pair(), "record": j.record,
                              "shown": j.shown(), "alert_key": j.named.alert_key if j.named else None}
                          for s, j in sorted(r.slots.items())},
                "page_bytes": [] if r.collection is None else [len(b) for b in r.collection.page_bytes],
                "run": None if r.collection is None else r.collection.run_hex(), "descriptor_run": r.run}
    args = [x for f in held_files for x in ("--contracts", str(f))]
    code, doc, err = bus.zenctl(*args, f"zk2/{address}/**")
    own = zen_owner(doc, address) or {}
    service = address.split("/", 1)[1]
    slots = {}
    for x in zen_alarms(doc):
        if (x.get("alarm") or {}).get("service") != service or x.get("slot") is None:
            continue
        w = x.get("words") or {}
        slots[x["slot"]] = {"firing": pair(x.get("firing")), "stale": pair(x.get("stale")),
                            "record": (x.get("latest") or {}).get("record"),
                            "shown": w.get("rule") or w.get("unnamed") or json.dumps(w),
                            "alert_key": (x.get("alarm") or {}).get("alert_key"), "words": w}
    return {"verdict": {"verdict": own.get("verdict"), "reason": own.get("reason")}, "slots": slots, "exit": code,
            "any_firing": (doc or {}).get("any_firing") if isinstance(doc, dict) else None}


YES_FIRING = {"answer": "yes", "reason": "firing"}
NO_RESOLVED = {"answer": "no", "reason": "resolved"}


def section3(rep: Report) -> None:
    sec = "§3"
    bus = Bus(rep, sec)
    try:
        S = bus.listener("lab/svc")
        o = bus.owner("lab/svc", contracts=[TC])
        time.sleep(1.0)
        o.raise_(A)
        time.sleep(0.5)
        t = time.monotonic_ns()
        o.resolve(KA)
        time.sleep(0.8)
        puts = rep.keep(sec, "1 resolve A", S, t)
        rep.check(sec, "Expected 1: one put on alarms/0, resolved, warning, a new at, rule 1, labels 2; S reads resolved",
                  [[p["key"] for p in puts], [rec_core(p.get("record")) for p in puts]]
                  == [["alarms/0"], [{"state": "resolved", "severity": "warning", "rule": 1, "labels": 2,
                                      "argument": "not_available"}]], puts)
        rep.check(sec, "Expected 1: S reads raised, then resolved",
                  [st.expect() for _, st in al.read_slots(S).get(0, [])] == [["raised"], ["resolved"]])
        g1 = judged(bus, "lab/svc", [TC])
        rep.check(sec, "Expected 2: G within 1 s: slot 0 live, the resolved record; no, resolved",
                  [g1["verdict"], (g1["slots"].get(0) or {}).get("firing"),
                   ((g1["slots"].get(0) or {}).get("record") or {}).get("state")]
                  == [{"verdict": "established", "reason": "snapshot"}, NO_RESOLVED, "resolved"], g1)
        time.sleep(65)
        g2 = judged(bus, "lab/svc", [TC])
        rep.check(sec, "Expected 1 and 3: no delete within 65 s; G 65 s later reads the same",
                  [[x["kind"] for x in S.all(t) if "/alarms/0" in x["key"]], (g2["slots"].get(0) or {}).get("firing")]
                  == [["put"], NO_RESOLVED], g2)
        deadline = time.monotonic() + 460
        while not S.heartbeat_wires(t) and time.monotonic() < deadline:
            time.sleep(1.0)
        time.sleep(0.5)
        beats = S.beats(t)
        last = beats[-1][1] if beats else {}
        sec_ = (last.get("read") or {}).get("sections", {}).get(a2.ALARMS_RESOURCE)
        from . import heartbeat
        wires = S.heartbeat_wires(t)
        body = None
        live_n = None
        if wires:
            try:
                env = __import__("zk2py.zk2c", fromlist=["parse"]).parse(wires[-1], "put")
                body = heartbeat.read(a2.heartbeat_layout(), env.record or b"")
                live_n = body["sections"].get(a2.ALARMS_RESOURCE, {}).get("live")
            except Exception as e:  # noqa: BLE001
                body = repr(e)
        rep.check(sec, "Expected 4: the heartbeat's section has a live count of 1, slot 0's bit clear, in step",
                  [live_n, isinstance(sec_, dict) and sec_.get("confirmed") == []] == [1, True],
                  {"section": sec_, "body": body, "heartbeat": wires[-1].hex() if wires else None})
    finally:
        bus.close()


def section4(rep: Report) -> None:
    sec = "§4"
    bus = Bus(rep, sec)
    try:
        S = bus.listener("lab/svc")
        o = bus.owner("lab/svc", contracts=[TC])
        time.sleep(1.0)
        o.raise_(A)
        time.sleep(0.5)
        held = held_revisions(TC)
        L0 = al.read_owner(bus.g, "lab/svc", held)
        l_rec = L0.slots[0].record
        l_run = L0.collection.run_hex()
        o.resolve(KA)
        time.sleep(0.5)
        t = time.monotonic_ns()
        a1 = o.raise_(B)
        time.sleep(0.8)
        rep.check(sec, "Expected 1: B firing on slot 0; S reads A replaced, then B raised",
                  [a1.get("slot"), [st.expect() for _, st in al.read_slots(S).get(0, [])][-1]]
                  == [0, ["replaced", "raised"]], a1)
        L1 = al.read_owner(bus.g, "lab/svc", held)
        rd = a2.SlotReader(a2.Table(tuple(held[next(iter(held))]["resources"][0]["annotations"]["alarms.rules"])))
        rd.step(a2.Read(l_rec), l_run)
        st = rd.step(a2.Read(L1.slots[0].record), L1.collection.run_hex())
        ans = a2.judge_reading(a2.Reading("present", {"lists": True}, None, ("fresh", "within_horizon"), "rebound",
                                          held_whole=True), v1.Q_FIRING).pair()
        rep.check(sec, "Expected 2 (zk2py's L): slot 0 holds B under L's run; L reads A rebound, then B raised; "
                  "A no, rebound", [L1.collection.run_hex() == l_run, st.expect(), ans]
                  == [True, ["rebound", "raised"], {"answer": "no", "reason": "rebound"}], st.expect())
        if PAIRING == "zk2py-owner":
            g = judged(bus, "lab/svc", [TC])
            rep.check(sec, "Expected 2 (zenctl): slot 0 shows B, firing; A is held by no slot",
                      [(g["slots"].get(0) or {}).get("alert_key"), (g["slots"].get(0) or {}).get("firing")]
                      == [KB, YES_FIRING], g)
        t = time.monotonic_ns()
        a3 = o.raise_(A)
        time.sleep(0.5)
        rep.check(sec, "Expected 3: A firing on slot 1, its at later than its resolve's",
                  [a3.get("slot"), (a3.get("record") or {}).get("state")] == [1, "firing"], a3)
        b_res = o.resolve(KB)
        b_again = o.raise_(B)
        time.sleep(0.5)
        rep.keep(sec, "1-4", S, 0)
        rep.check(sec, "Expected 4: B resolved on slot 0, then B firing on slot 0",
                  [b_res.get("slot"), b_again.get("slot"), (b_again.get("record") or {}).get("state")] == [0, 0, "firing"],
                  [b_res, b_again])
    finally:
        bus.close()


def section7(rep: Report) -> None:
    sec = "§7"
    bus = Bus(rep, sec)
    try:
        S = bus.listener("lab/svc")
        o = bus.owner("lab/svc", contracts=[TC], bound=2)
        time.sleep(1.0)
        ra, rb = o.raise_(A), o.raise_(B)
        time.sleep(0.8)
        t = time.monotonic_ns()
        rc = o.raise_(C)
        time.sleep(0.8)
        rep.check(sec, "Expected 1 and 2: A on 0, B on 1; C refused, nothing put",
                  [ra.get("slot"), rb.get("slot"), rc.get("act"), rep.keep(sec, "2 raise C", S, t)]
                  == [0, 1, "refused", []], rc)
        t = time.monotonic_ns()
        o.resolve(KB)
        rc2 = o.raise_(C)
        time.sleep(0.8)
        puts = rep.keep(sec, "3 resolve B, raise C", S, t)
        rep.check(sec, "Expected 3: B resolved on 1, then C firing on 1; S reads B replaced",
                  [[(p["key"], (p.get("record") or {}).get("state")) for p in puts], rc2.get("slot"),
                   [st.expect() for _, st in al.read_slots(S).get(1, [])][-1]]
                  == [[("alarms/1", "resolved"), ("alarms/1", "firing")], 1, ["replaced", "raised"]], puts)
        rb2 = o.raise_(B)
        rep.check(sec, "Expected 4: B's raise refused", rb2.get("act") == "refused", rb2)
    finally:
        bus.close()


def _record_file(path: str) -> dict[str, Any]:
    return json.loads(Path(path).read_text(encoding="utf-8"))


def section5(rep: Report) -> None:
    sec = "§5"
    bus = Bus(rep, sec)
    try:
        record = os.path.join(bus.tmp, "svc.record.json")
        S = bus.listener("lab/svc")
        o = bus.owner("lab/svc", contracts=[TC], record=record)
        time.sleep(1.0)
        ra = o.raise_(A, severity="critical")
        o.raise_(B)
        o.raise_({"rule": "disk_full", "severity": "warning", "summary": "/var is full", "labels": {"mount": "/var"}})
        time.sleep(0.8)
        t0_at = (ra.get("record") or {}).get("at")
        o.kill()
        t = time.monotonic_ns()
        time.sleep(3.0)
        rep.check(sec, "Expected 1: the process ends with no close; S hears no put and no delete in 3 s",
                  [x for x in S.all(t) if "/alarms.v2/state/" in x["key"]] == [])
        doc = _record_file(record)
        slots = doc["slots"]
        a_entry = slots["0"]
        slots["3"] = {"record": {"state": "resolved", "severity": "warning", "rule": 2, "labels": 7,
                                 "argument": "not_available", "at": t0_at},
                      "words": {"rule": "sensor_lost", "labels": {"sensor": "7"}, "summary": "Sensor 7 lost"},
                      "alert_key": v1.derive("sensor_lost", {"sensor": "7"}).key}
        slots["4"] = dict(a_entry, alert_key="0000000000000001")
        Path(record).write_text(json.dumps(doc), encoding="utf-8")
        t = time.monotonic_ns()
        o2 = bus.owner("lab/svc", contracts=[TC_LATER], record=record)
        held = [TC, TC_LATER]
        if PAIRING == "ref-owner":
            r = al.read_owner(bus.g, "lab/svc", held_revisions(*held))
            snap = r.collection.snapshot() if r.collection else None
            entries = {e["slot"]: e for e in (snap or {}).get("live", [])}
            rep.check(sec, "Expected 3: G's GET at the token: a snapshot under the new instance's run, the one its "
                      "descriptor states",
                      [r.collection.run_hex() == r.run, r.verdict.expect()] == [True, {"verdict": "established",
                                                                                        "reason": "snapshot"}],
                      {"run": r.collection.run_hex(), "descriptor": r.run})
            rep.check(sec, "Expected 3: slots 0 A firing, at t0 kept; 1 B firing; 2 D firing, rule 6, labels 0, no "
                      "key; neither 3 nor 4",
                      [sorted(entries), (entries.get(0) or {}).get("record", {}).get("at") == t0_at,
                       rec_core((entries.get(2) or {}).get("record"))]
                      == [[0, 1, 2], True, {"state": "firing", "severity": "warning", "rule": 6, "labels": 0,
                                            "argument": "not_available"}], {e: v.get("record") for e, v in entries.items()})
            rep.check(sec, "Expected 3: D derives the same alert key by the new table",
                      (r.slots.get(2) and r.slots[2].named and r.slots[2].named.alert_key) == "719d159937e54eb8",
                      r.slots.get(2) and r.slots[2].shown())
        else:
            g = judged(bus, "lab/svc", held)
            rep.check(sec, "Expected 3 (zenctl): slots 0, 1, 2 firing, D named disk_full by the new table; no 3 or 4",
                      [sorted(g["slots"]), [g["slots"][k]["firing"] for k in sorted(g["slots"])],
                       (g["slots"].get(2) or {}).get("shown"), (g["slots"].get(2) or {}).get("alert_key")]
                      == [[0, 1, 2], [YES_FIRING] * 3, "disk_full", "719d159937e54eb8"], g)
        time.sleep(0.5)
        steps = al.read_slots(S, a2.Table(tuple(held_revisions(TC_LATER)[next(iter(held_revisions(TC_LATER)))]
                                                ["resources"][0]["annotations"]["alarms.rules"])))
        rep.keep(sec, "3 restart", S, t)
        rep.check(sec, "Expected 3: S reads A confirmed on 0, B confirmed on 1, D refreshed on 2",
                  [steps.get(k, [(None, None)])[-1][1].expect() if steps.get(k) else None for k in (0, 1, 2)]
                  == [["confirmed"], ["confirmed"], ["refreshed"]],
                  {k: [st.expect() for _, st in v] for k, v in steps.items()})
        ad = o2.cmd("alarm adoption")
        t = time.monotonic_ns()
        rb = o2.resolve(KB)
        time.sleep(0.8)
        puts = rep.keep(sec, "3 first evaluation", S, t)
        rep.check(sec, "Expected 3: at the first evaluation S receives B resolved on slot 1",
                  [(p["key"], (p.get("record") or {}).get("state")) for p in puts] == [("alarms/1", "resolved")],
                  {"puts": puts, "adoption": ad, "resolve": rb})
        g = judged(bus, "lab/svc", held)
        rep.check(sec, "Expected 4: G judges A yes, firing, never previous_instance",
                  (g["slots"].get(0) or {}).get("firing") == YES_FIRING, g["slots"].get(0))
    finally:
        bus.close()


def section9(rep: Report) -> None:
    sec = "§9"
    bus = Bus(rep, sec)
    try:
        SP = bus.listener("lab/plain")
        plain = bus.owner("lab/plain")
        svc = bus.owner("lab/svc", contracts=[TC])
        time.sleep(1.0)
        t = time.monotonic_ns()
        plain.raise_(A, summary="Link to r2 is down")
        time.sleep(0.8)
        puts = rep.keep(sec, "1 lab/plain raises A", SP, t)
        from . import zk2c
        seqs = {p["key"]: zk2c.parse(bytes.fromhex(p["hex"]), "put").seq for p in puts}
        rec = next((p.get("record") for p in puts if p["key"] == "alarms/0"), None)
        rep.check(sec, "Expected 1: words/0 (CH 0x00, strict JSON) and alarms/0 (23 bytes: not_available, no labels, "
                  "key 0xa659f813308ad1da), the words' seq below the record's",
                  [sorted(p["key"] for p in puts), [p["hex"][:2] for p in puts if p["key"] == "words/0"],
                   [p["bytes"] for p in puts if p["key"] == "alarms/0"], rec_core(rec),
                   seqs.get("words/0", 0) < seqs.get("alarms/0", -1)]
                  == [["alarms/0", "words/0"], ["00"], [23], {"state": "firing", "severity": "warning",
                                                              "rule": "not_available", "argument": "not_available",
                                                              "key": int(KA, 16)}, True], puts)
        t = time.monotonic_ns()
        r2 = plain.raise_(A, summary="Link to r2 flaps")
        time.sleep(0.8)
        puts = rep.keep(sec, "2 lab/plain refreshes A", SP, t)
        rep.check(sec, "Expected 2: one put on words/0, none on alarms/0",
                  [p["key"] for p in puts] == ["words/0"], {"puts": puts, "answer": r2})
        SS = bus.listener("lab/svc")
        t = time.monotonic_ns()
        svc.raise_(A)
        svc.raise_({"rule": "disk_full", "severity": "warning", "summary": "/var is full", "labels": {"mount": "/var"}})
        time.sleep(0.8)
        puts = rep.keep(sec, "3 lab/svc raises A and W", SS, t)
        by_seq = sorted((zk2c.parse(bytes.fromhex(p["hex"]), "put").seq, p["key"]) for p in puts if p["key"] != "alarms/0")
        rep.check(sec, "Expected 3: lab/svc's A one put of 18 bytes on alarms/0; W words on words/1, then a record by "
                  "words on alarms/1 (by seq: the two ride two priorities)",
                  [[(p["key"], p["bytes"]) for p in puts if p["key"] == "alarms/0"], [k for _, k in by_seq]]
                  == [[("alarms/0", 18)], ["words/1", "alarms/1"]],
                  {"arrival": [p["key"] for p in puts], "by seq": by_seq, "puts": puts})
        for who, files in (("T", [TC]), ("T2", [TC_EARLIER])):
            gs, gp = judged(bus, "lab/svc", files), judged(bus, "lab/plain", files)
            a_svc, w_svc, a_plain = gs["slots"].get(0) or {}, gs["slots"].get(1) or {}, gp["slots"].get(0) or {}
            firing = [a_svc.get("firing"), w_svc.get("firing"), a_plain.get("firing")]
            if who == "T":
                ok = ["link_down" in str(a_svc.get("shown")), "Link to r2 flaps" in json.dumps(a_plain),
                      a_svc.get("alert_key") == a_plain.get("alert_key") == KA] == [True, True, True]
                rep.check(sec, "Expected 4 (T, tc.v1): lab/svc's A as link_down \"Link to r2 on eth0 is down\"; "
                          "lab/plain's A by its words \"Link to r2 flaps\"; both a659f813308ad1da",
                          ok and "Link to r2 on eth0 is down" in json.dumps(a_svc), {"svc": a_svc, "plain": a_plain})
            elif PAIRING == "zk2py-owner":
                # zenctl retrieves the revision a descriptor names that it does not
                # hold (its --help: "one not held retrieved by fingerprint"), so it
                # is no T2: it names A by the named revision, never door_open.
                ok = ["door_open" not in json.dumps(a_svc), (a_svc.get("words") or {}).get("rule") == "link_down",
                      "/var is full" in json.dumps(w_svc), "Link to r2 flaps" in json.dumps(a_plain)]
                rep.check(sec, "Expected 4 (zenctl given the earlier revision alone): never door_open; it retrieves "
                          "the named revision, so A is link_down; lab/plain's A and lab/svc's W by their words",
                          all(ok), {"svc A": a_svc, "svc W": w_svc, "plain": a_plain, "ok": ok})
            else:
                ok = ["door_open" not in json.dumps(a_svc), "unnamed" in json.dumps(a_svc) or "rule 1" in json.dumps(a_svc),
                      "/var is full" in json.dumps(w_svc), "Link to r2 flaps" in json.dumps(a_plain)]
                rep.check(sec, "Expected 4 (T2, the earlier revision alone): lab/svc's A unnamed, rule 1, labels 2, "
                          "never door_open; lab/plain's A and lab/svc's W by their words", all(ok),
                          {"svc A": a_svc, "svc W": w_svc, "plain": a_plain, "ok": ok})
            rep.check(sec, f"Expected 4 ({who}): every alarm yes, firing", firing == [YES_FIRING] * 3, firing)
        liar = bus.owner("lab/liar", impl="zk2py")
        time.sleep(0.5)
        rec = a2.write({"state": "firing", "severity": "warning", "rule": "not_available",
                        "argument": "not_available", "key": int(KA, 16)})
        words = json.dumps({"rule": "disk_full", "labels": {"mount": "/var"}, "summary": "/var is full"})
        liar.cmd(f"raw 0 {rec.hex()} {words}")
        time.sleep(0.8)
        if PAIRING == "ref-owner":
            r = al.read_owner(bus.g, "lab/liar", {})
            t_ = (r.words.get(0) or (None,))[0]
            rep.check(sec, "Expected 5: no, mismatch; the alarm shown by its alert key, with no words",
                      [None if t_ is None else t_.answer(), r.slots[0].shown() if 0 in r.slots else None]
                      == [{"answer": "no", "reason": "mismatch"}, f"by words, {KA}"], r.slots.get(0) and r.slots[0].shown())
        else:
            code, doc, err = bus.zenctl("zk2/lab/liar/**")
            z = zen_of(doc, 0, "liar")
            rep.check(sec, "Expected 5 (zenctl): no, mismatch; shown by its alert key, with no words",
                      [pair((z or {}).get("rederives")), (z or {}).get("words")] ==
                      [{"answer": "no", "reason": "mismatch"}, {"by": "key", "reason": "mismatch"}], z)
    finally:
        bus.close()


def section10(rep: Report) -> None:
    sec = "§10"
    bus = Bus(rep, sec)
    try:
        oa = bus.owner("lab/a", contracts=[TC])
        ob = bus.owner("lab/b", contracts=[TC], tokenless=["alarms.v2"])
        time.sleep(1.0)
        for x in (A, B, C):
            oa.raise_(x)
        time.sleep(0.8)
        if PAIRING == "ref-owner":
            ra, rb = al.read_owner(bus.g, "lab/a", held_revisions(TC)), al.read_owner(bus.g, "lab/b", held_revisions(TC))
            w, c = al.any_firing("warning", [ra, rb]), al.any_firing("critical", [ra, rb])
            rep.check(sec, "Expected 1: at warning yes, any_firing, 3; at critical no, none_firing; both established "
                      "by snapshot", [[w["answer"], w["reason"], w["firing"]], [c["answer"], c["reason"]],
                                      ra.verdict.expect(), rb.verdict.expect()]
                      == [["yes", "any_firing", 3], ["no", "none_firing"], {"verdict": "established", "reason": "snapshot"},
                          {"verdict": "established", "reason": "snapshot"}], [w, c])
            rep.check(sec, "Expected 1: lab/a's answer one page of 70 bytes; lab/b's one page of 16 bytes",
                      [[len(b) for b in ra.collection.page_bytes], [len(b) for b in rb.collection.page_bytes]]
                      == [[70], [16]], {"a": [b.hex() for b in ra.collection.page_bytes],
                                        "b": [b.hex() for b in rb.collection.page_bytes]})
        else:
            for floor, want in (("warning", ["yes", "any_firing", 3, 1]), ("critical", ["no", "none_firing", 0, 0])):
                code, doc, err = bus.zenctl("--at-least", floor, "--contracts", str(TC), "zk2/lab/*/alarms.v2/state/alarms/*")
                af = (doc or {}).get("any_firing") or {}
                owners = [(o.get("address"), o.get("verdict"), o.get("reason")) for o in (doc or {}).get("rows", [])
                          if o.get("row") == "owner"]
                rep.check(sec, f"Expected 1 (zenctl at {floor}): {want[0]}, {want[1]}, {want[2]}; both owners "
                          "established by snapshot", [af.get("answer"), af.get("reason"), af.get("firing"), code] == want
                          and sorted(owners) == [("lab/a", "established", "snapshot"), ("lab/b", "established", "snapshot")],
                          {"any_firing": af, "owners": owners})
        ob.raise_(B, severity="critical")
        time.sleep(0.8)
        if PAIRING == "ref-owner":
            rb = al.read_owner(bus.g, "lab/b", held_revisions(TC))
            c = al.any_firing("critical", [ra, rb])
            rep.check(sec, "Expected 2: at critical yes, any_firing, 1", [c["answer"], c["firing"]] == ["yes", 1], c)
            run_before = rb.run
        else:
            code, doc, err = bus.zenctl("--at-least", "critical", "--contracts", str(TC), "zk2/lab/*/alarms.v2/state/alarms/*")
            af = (doc or {}).get("any_firing") or {}
            rep.check(sec, "Expected 2 (zenctl): at critical yes, any_firing, 1",
                      [af.get("answer"), af.get("firing"), code] == ["yes", 1, 1], af)
        ob.kill()
        ob2 = bus.owner("lab/b", contracts=[TC], tokenless=["alarms.v2"])
        time.sleep(0.8)
        if PAIRING == "ref-owner":
            gap = al.read_owner(bus.g, "lab/b", held_revisions(TC))
            # before T reads again: its earlier read stands, and the new instance's appearing is a gap
            before = a2.established("present", {"lists": True}, None, a2.Read2(True, True, True, rb.collection.run_hex()),
                                    gap=gap.run != run_before, runs=[gap.run])
            # §2.14 step 5: the B T holds is under the earlier instance's run, which
            # no present descriptor states: previous_instance, never yes.
            held_b = a2.judge_reading(a2.Reading("present", {"lists": True}, None, ("fresh", "within_horizon"), "firing",
                                                 previous_instance=rb.collection.run_hex() != gap.run))
            c0 = al.any_firing("critical", [ra, v1.OwnerRead(before.verdict, (v1.AlarmRead(held_b.answer, "critical"),))])
            c1 = al.any_firing("critical", [ra, gap])
            rep.check(sec, "Expected 3: before the read unobservable, owner_unobservable (owner_gap); after it no, "
                      "none_firing; the new snapshot has no entry, under the run the descriptor states",
                      [[c0["answer"], c0["reason"], before.reason], [c1["answer"], c1["reason"]], gap.slots,
                       gap.collection.run_hex() == gap.run]
                      == [["unobservable", "owner_unobservable", "owner_gap"], ["no", "none_firing"], {}, True], [c0, c1])
        else:
            code, doc, err = bus.zenctl("--at-least", "critical", "--contracts", str(TC), "zk2/lab/*/alarms.v2/state/alarms/*")
            af = (doc or {}).get("any_firing") or {}
            rep.check(sec, "Expected 3 (zenctl, after the read): no, none_firing", [af.get("answer"), af.get("reason"), code]
                      == ["no", "none_firing", 0], af)
    finally:
        bus.close()


def section11(rep: Report) -> None:
    sec = "§11"
    bus = Bus(rep, sec)
    try:
        svc = bus.owner("lab/svc", contracts=[TC])
        old = bus.owner("lab/old", impl="rust", v1_owner=True)
        time.sleep(1.0)
        svc.raise_(A, severity="critical")
        old.raise_({"rule": "fan_slow", "severity": "info", "summary": "Fan slow"})
        time.sleep(0.8)
        if PAIRING == "ref-owner":
            trust = v1.fr.ClockTrust(word=True)
            t1 = [v1.read_owner(bus.g, a, trust=trust) for a in ("lab/svc", "lab/old")]
            ans = v1.any_firing("warning", [r.owner_read() for r in t1])
            rep.check(sec, "Expected 1 (T1, alarms.v1's text alone): lab/svc not asked, not_listed; lab/old "
                      "established; no, none_firing, one established and one not asked",
                      [[r.verdict.verdict, r.verdict.reason] for r in t1] + [[ans["answer"], ans["reason"], ans["owners"]]]
                      == [["not_asked", "not_listed"], ["established", "answered"],
                          ["no", "none_firing", {"established": 1, "not_asked": 1, "unobservable": 0}]], ans)
            ans2 = v1.any_firing("warning", [t1[0].owner_read()])
            rep.check(sec, "Expected 2 (T1 over lab/svc's alarms.v1 keys): not asked, no_owner",
                      [ans2["answer"], ans2["reason"]] == ["not_asked", "no_owner"], ans2)
            t2 = [al.read_owner(bus.g, "lab/svc", held_revisions(TC)), v1.read_owner(bus.g, "lab/old", trust=trust)]
            ans3 = al.any_firing("warning", [t2[0], t2[1].owner_read()])
            ref = t2[0].slots[0].named.alert_key if 0 in t2[0].slots and t2[0].slots[0].named else None
            rep.check(sec, "Expected 3 (T2): yes, any_firing, 1; A by its reference lab, svc, a659f813308ad1da",
                      [ans3["answer"], ans3["firing"], ref] == ["yes", 1, KA], ans3)
        else:
            code, doc, err = bus.zenctl("--contracts", str(TC), "zk2/lab/*/alarms.v1/state/alarms/*")
            af = (doc or {}).get("any_firing") or {}
            z = zen_of(doc, 0)
            rep.check(sec, "Expected 3 (zenctl as T2, over the alarms.v1 selector): yes, any_firing, 1; A by its "
                      "reference", [af.get("answer"), af.get("firing"), (z or {}).get("alarm")]
                      == ["yes", 1, {"alert_key": KA, "service": "svc", "system": "lab"}], af)
            rep.check(sec, "Expected 1 and 2: T1, a reader of alarms.v1 alone, is no reference tool", True,
                      "not run: the reference's readers read both majors")
    finally:
        bus.close()


def section12(rep: Report) -> None:
    sec = "§12"
    bus = Bus(rep, sec)
    try:
        record = os.path.join(bus.tmp, "svc.record.json")
        S = bus.listener("lab/svc")
        o1 = bus.owner("lab/svc", impl="rust", v1_owner=True, record=record)
        time.sleep(1.0)
        a1 = o1.raise_(A, severity="critical")
        time.sleep(0.5)
        t0 = (a1.get("document") or {}).get("timestamp")
        nt = a2.Notifier("lab", "svc", a2.Table(tuple(held_revisions(TC)[next(iter(held_revisions(TC)))]["resources"][0]
                                                     ["annotations"]["alarms.rules"])))
        told = {("lab", "svc", KA): ("yes", "critical")}
        nt.told.update(told)
        o1.kill()
        t = time.monotonic_ns()
        o2 = bus.owner("lab/svc", contracts=[TC], record=record)
        time.sleep(1.0)
        g = al.read_owner(bus.g, "lab/svc", held_revisions(TC))
        puts = rep.keep(sec, "1 restart as alarms.v2", S, t)
        v2puts = [p for p in puts if p["key"].startswith("alarms/")]
        rec = v2puts[0].get("record") if v2puts else None
        v1keys = [x for x in S.all(t) if "/alarms.v1/" in x["key"]]
        rep.check(sec, "Expected 1: the descriptor lists alarms.v2 and not alarms.v1; A firing on alarms.v2 slot 0: "
                  "critical, rule 1, labels 2, at t0's second; nothing on an alarms.v1 key",
                  [g.major, [p["key"] for p in v2puts], rec_core(rec), rec and t0 is not None and rec.get("at") == t0 - t0 % 1000,
                   v1keys] == ["alarms.v2", ["alarms/0"], {"state": "firing", "severity": "critical", "rule": 1,
                                                           "labels": 2, "argument": "not_available"}, True, []],
                  {"puts": puts, "t0": t0, "adoption": o2.cmd("alarm adoption")})
        j = judged(bus, "lab/svc", [TC])
        notes = nt.put(0, a2.Read(rec), g.collection.run_hex() if g.collection else None) if rec else ["no record"]
        rep.check(sec, "Expected 2: A by its reference yes, firing; the acknowledgement against t0 applies; N reports "
                  "nothing", [(j["slots"].get(0) or {}).get("firing"), a2.ack_applies(t0, rec.get("at") if rec else None),
                              notes] == [YES_FIRING, True, []], {"judged": j["slots"].get(0), "notes": notes})
    finally:
        bus.close()


def section8(rep: Report) -> None:
    """§8 with the reference's ``archive`` (its usage: ``--record <owner>
    <selector> <contract.toml>``, up once it says ``aligned``)."""
    sec = "§8"
    bus = Bus(rep, sec)
    arch = None
    try:
        from . import live
        S = bus.listener("lab/svc")
        o = bus.owner("lab/svc", contracts=[TC])
        time.sleep(1.0)
        o.raise_(A)
        time.sleep(0.5)

        arch = subprocess.Popen([str(BLACKBOX / "archive"), "--connect", bus.endpoint, "--system", "lab", "--service",
                                 "archive", "--record", "lab/svc", "zk2/lab/svc/alarms.v2/state/**", str(a2.CONTRACT)],
                                stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        lines: list[str] = []
        threading.Thread(target=lambda: [lines.append(x.rstrip("\n")) for x in arch.stdout], daemon=True).start()
        deadline = time.monotonic() + 30
        while time.monotonic() < deadline and not any(x.startswith("aligned") for x in lines):
            time.sleep(0.2)
        time.sleep(1.0)
        g1 = judged(bus, "lab/svc", [TC])
        rep.check(sec, "Expected 1: T judges A yes, firing", (g1["slots"].get(0) or {}).get("firing") == YES_FIRING,
                  {"archive": lines[-3:], "slot 0": g1["slots"].get(0)})
        t = time.monotonic_ns()
        o.close()
        time.sleep(2.0)
        rep.check(sec, "Expected 2: a clean close: no resolved put and no delete",
                  [x for x in S.all(t) if "/state/alarms/" in x["key"]] == [])
        pres = live.list_presence(bus.g, "zk2/lab/svc/@zk/**")
        form = v1.archive_form("lab/archive", "zk2/lab/svc/alarms.v2/state/alarms/**")
        got = live.get_state(bus.g, form, timeout=1.0)
        arch_recs = {}
        for r in got.replies:
            if r.deleted or r.refusal:
                continue
            rd = a2.read(a2.record().tag, r.payload)
            arch_recs[r.key] = rd.value if rd.reads else {"undecodable": rd.undecodable}
        held_tab = a2.Table(tuple(held_revisions(TC)[next(iter(held_revisions(TC)))]["resources"][0]["annotations"]
                                  ["alarms.rules"]))
        named = {k: a2.name_record(held_tab, v).shown() if "undecodable" not in v else None for k, v in arch_recs.items()}
        unnamed = {k: a2.numbers_of(v) for k, v in arch_recs.items() if "undecodable" not in v}
        if PAIRING == "ref-owner":
            r = al.read_owner(bus.g, "lab/svc", held_revisions(TC))
            ans = al.any_firing("warning", [r])
            rep.check(sec, "Expected 3: tokens gone; A not asked, absent; the archive's slot 0 firing, last-known, named "
                      "by the revision T holds from step 1, \"rule 1, labels 2\" without it; anything firing not yes",
                      [pres.complete and not pres.instances, r.verdict.expect(), len(arch_recs),
                       [v.get("state") for v in arch_recs.values()], list(named.values()), list(unnamed.values()),
                       ans["answer"] != "yes"]
                      == [True, {"verdict": "not_asked", "reason": "absent"}, 1, ["firing"],
                          ["Link to r2 on eth0 is down"], ["rule 1, labels 2"], True],
                      {"archive": arch_recs, "any_firing": ans, "form": form})
        else:
            code, doc, err = bus.zenctl("--contracts", str(TC), "zk2/lab/**")
            af = (doc or {}).get("any_firing") or {}
            rows = [x for x in (doc or {}).get("rows") or []]
            rep.check(sec, "Expected 3 (zenctl): the owner not asked, absent; its archived record shown last-known, "
                      "never firing now; anything firing not yes",
                      [af.get("answer") != "yes", [(x.get("row"), x.get("verdict"), x.get("reason")) for x in rows
                                                   if x.get("row") == "owner"]],
                      {"any_firing": af, "rows": rows, "stderr": err[-800:]})
    finally:
        if arch is not None:
            try:
                arch.stdin.close()
                arch.wait(10)
            except Exception:  # noqa: BLE001
                arch.kill()
        bus.close()


def section13(rep: Report) -> None:
    """§2 step 4 and §6 at the standard 900 s horizon, the slow tier: lab/svc
    holds H, escalated and refreshed; lab/hung holds A unconfirmed and B
    resolved. 905 s on, S, G and zenctl judge each."""
    sec = "§2+§6 at 900 s"
    bus = Bus(rep, sec)
    try:
        S = bus.listener("lab/svc")
        SH = bus.listener("lab/hung")
        o = bus.owner("lab/svc", contracts=[TC])
        hung = bus.owner("lab/hung", contracts=[TC])
        time.sleep(1.0)
        h = {"rule": "hot", "severity": "warning", "summary": "Temperature 85.5 C", "reading": "85.5"}
        o.raise_(h)
        o.raise_(h, severity="critical")
        o.raise_(h, severity="critical", reading="86.0")
        hung.raise_(A)
        hung.raise_(B)
        hung.resolve(KB)
        time.sleep(0.5)
        hung.cmd(f"alarm confirm {KA} off")
        t = time.monotonic_ns()
        time.sleep(905)
        from . import heartbeat, zk2c
        out = {}
        for name, L, slots in (("lab/svc", S, (0,)), ("lab/hung", SH, (0, 1))):
            wires = L.heartbeat_wires(t)
            env = zk2c.parse(wires[-1], "put") if wires else None
            body = heartbeat.read(a2.heartbeat_layout(), env.record or b"") if env else None
            out[name] = {"heartbeats": len(wires), "tag": env and env.tag, "bytes": len(wires[-1]) if wires else None,
                         "section": (body or {}).get("sections", {}).get(a2.ALARMS_RESOURCE),
                         "puts": [x["key"] for x in L.all(t) if "/state/alarms/" in x["key"]],
                         "S": {k: v1.fr.judge("state", v1.HORIZON, [L.observation(k)]).verdict for k in slots},
                         "last": wires[-1].hex() if wires else None}
        rep.check(sec, "§2 Expected 4: no put of H for 905 s; heartbeats at most 450 s apart, tag 7, 17 bytes, live "
                  "count 1, slot 0's bit set; S judges H yes, firing",
                  [out["lab/svc"]["puts"], out["lab/svc"]["heartbeats"] >= 2, out["lab/svc"]["tag"],
                   out["lab/svc"]["bytes"], (out["lab/svc"]["section"] or {}).get("live"),
                   (out["lab/svc"]["section"] or {}).get("vouched"), out["lab/svc"]["S"][0]]
                  == [[], True, 7, 17, 1, [0], "fresh"], out["lab/svc"])
        rep.check(sec, "§6 Expected 1 and 2: no put of A; live count 2, both bits clear; S: A stale, B's record resolved",
                  [out["lab/hung"]["puts"], (out["lab/hung"]["section"] or {}).get("live"),
                   (out["lab/hung"]["section"] or {}).get("vouched"), out["lab/hung"]["S"][0]]
                  == [[], 2, [], "stale"], out["lab/hung"])
        g = judged(bus, "lab/hung", [TC])
        gs = judged(bus, "lab/svc", [TC])
        rep.check(sec, "§6 Expected 2 (G): A unobservable, stale; B no, resolved; §2's H yes, firing",
                  [(g["slots"].get(0) or {}).get("firing"), (g["slots"].get(1) or {}).get("firing"),
                   (gs["slots"].get(0) or {}).get("firing")]
                  == [{"answer": "unobservable", "reason": "stale"}, NO_RESOLVED, YES_FIRING], [g["slots"], gs["slots"]])
        code, doc, err = bus.zenctl("--contracts", str(TC), "zk2/lab/**")
        zs = {f"{(x.get('alarm') or {}).get('service')}/{x.get('slot')}": pair(x.get("firing")) for x in zen_alarms(doc)}
        rep.check(sec, "§6 Expected 2 and §2 Expected 4 (zenctl, the reference's reader of either owner): A "
                  "unobservable, stale; B no, resolved; H yes, firing",
                  [zs.get("hung/0"), zs.get("hung/1"), zs.get("svc/0")]
                  == [{"answer": "unobservable", "reason": "stale"}, NO_RESOLVED, YES_FIRING], zs)
    finally:
        bus.close()


def _strip(wire: bytes) -> dict[str, Any]:
    """A put's payload, its envelope's stamp apart: CH, run, seq, and the
    rest (a record, a page's body, a heartbeat's body)."""
    from . import zk2c

    env = zk2c.parse(wire, "put")
    return {"ch": f"{wire[0]:02x}", "run": env.run, "seq": env.seq, "rest": (env.record or b"").hex(),
            "bytes": len(wire)}


def section14(rep: Report) -> None:
    """The bytes side by side: the reference's owner (``lab/ref``) and zk2py's
    (``lab/zk``) on one bus, the same contract (the 4 s copy, so a heartbeat
    comes within 2 s), the same acts, every put, page and heartbeat captured
    by zk2py's S and compared but for time, run and seq."""
    sec = "bytes"
    bus = Bus(rep, sec)
    try:
        from . import heartbeat, zk2c

        short = _short_contract(bus)
        sc = al.standard(4)
        owners = {"ref": "rust", "zk": "zk2py"}
        L = {n: bus.listener(f"lab/{n}", sc) for n in owners}
        O = {n: bus.owner(f"lab/{n}", impl=i, contracts=[short, TC]) for n, i in owners.items()}
        time.sleep(1.0)
        acts = [("raise A", lambda o: o.raise_(A)), ("raise B", lambda o: o.raise_(B)),
                ("raise W by words", lambda o: o.raise_({"rule": "disk_full", "severity": "warning",
                                                         "summary": "/var is full", "labels": {"mount": "/var"}})),
                ("raise H 85.5", lambda o: o.raise_({"rule": "hot", "severity": "info", "summary": "Temperature 85.5 C",
                                                     "reading": "85.5"})),
                ("resolve B", lambda o: o.resolve(KB)),
                ("raise C on B's slot", lambda o: o.raise_(C))]
        rows = []
        for name, act in acts:
            t = time.monotonic_ns()
            for n in owners:
                act(O[n])
            time.sleep(0.8)
            got = {}
            for n in owners:
                got[n] = [(x["key"].split("/alarms.v2/state/", 1)[1], _strip(x["wire"])) for x in L[n].all(t)
                          if "/alarms.v2/state/" in x["key"]]
            same = [[(k, {kk: vv for kk, vv in v.items() if kk not in ("run", "seq")}) for k, v in got[n]] for n in owners]
            # a record's at differs by the second each owner's act fell in: compare its reading, at apart
            def norm(items: list[Any]) -> list[Any]:
                out = []
                for k, v in items:
                    if k.startswith("alarms/"):
                        r = a2.read(a2.record().tag, bytes.fromhex(v["rest"]))
                        out.append((k, v["ch"], v["bytes"], rec_core(r.value)))
                    else:
                        out.append((k, v["ch"], v["bytes"], v["rest"]))
                return sorted(out, key=str)
            rows.append({"act": name, "ref": got["ref"], "zk": got["zk"]})
            rep.check(sec, f"{name}: the two owners put the same keys, CH, sizes and records (at apart)",
                      norm(got["ref"]) == norm(got["zk"]), {"ref": got["ref"], "zk": got["zk"]})
        pages = {n: al.collection_read(bus.g, f"lab/{n}") for n in owners}
        pr = {n: [_strip(b) for b in pages[n].page_bytes] for n in owners}
        snaps = {n: pages[n].snapshot() for n in owners}
        def entries(n: str) -> Any:
            return [(e["slot"], e["confirmation"], rec_core(e["record"])) for e in (snaps[n] or {}).get("live", [])]
        rep.check(sec, "the alarms' snapshot: one page each, the same size and entries (time, at, run and seq apart)",
                  [[len(p) for p in pages["ref"].page_bytes] == [len(p) for p in pages["zk"].page_bytes],
                   entries("ref") == entries("zk")] == [True, True],
                  {"ref": pr["ref"], "zk": pr["zk"], "entries": {n: entries(n) for n in owners}})
        wpages = {n: al.collection_read(bus.g, f"lab/{n}", prefix=a2.WORDS_PREFIX) for n in owners}
        rep.check(sec, "the words' snapshot: the same size and words",
                  [len(b) for b in wpages["ref"].page_bytes] == [len(b) for b in wpages["zk"].page_bytes]
                  and [(e["slot"], e["words"]) for e in (wpages["ref"].snapshot() or {}).get("live", [])]
                  == [(e["slot"], e["words"]) for e in (wpages["zk"].snapshot() or {}).get("live", [])],
                  {n: [b.hex() for b in wpages[n].page_bytes] for n in owners})
        t = time.monotonic_ns()
        time.sleep(4.5)
        hb = {}
        for n in owners:
            w = L[n].heartbeat_wires(t)
            env = zk2c.parse(w[-1], "put") if w else None
            body = heartbeat.read(a2.heartbeat_layout(sc.canonical, sc.fingerprint), env.record or b"") if env else None
            hb[n] = {"wire": w[-1].hex() if w else None, "bytes": len(w[-1]) if w else None,
                     "section": (body or {}).get("sections", {}).get(a2.ALARMS_RESOURCE), "count": len(w)}
        same_shape = {k: v for k, v in (hb["ref"]["section"] or {}).items() if k != "digest"} == \
            {k: v for k, v in (hb["zk"]["section"] or {}).items() if k != "digest"}
        rep.check(sec, "the heartbeat: the same size, live count and bits (each digest its own runs')",
                  hb["ref"]["bytes"] == hb["zk"]["bytes"] and same_shape, hb)
        rep.puts["bytes-side-by-side"] = rows + [{"pages": pr, "words": {n: [b.hex() for b in wpages[n].page_bytes]
                                                                          for n in owners}, "heartbeats": hb}]
    finally:
        bus.close()


def section15(rep: Report) -> None:
    """F-614's evidence: one owner read as if across a face (granted), where
    "the deployment's word stands for the descriptor, and no run is
    compared" (§2.15): zk2py's reader and ``zenctl alarms --across-face
    granted``, each over A and B firing on two slots."""
    sec = "across a face"
    bus = Bus(rep, sec)
    try:
        from .contract import load_contract

        o = bus.owner("lab/svc", contracts=[TC])
        time.sleep(1.0)
        o.raise_(A)
        o.raise_(B)
        time.sleep(0.8)
        # the deployment's word names the owner's revisions: tc.v1's, and
        # alarms.v2's standard one, whose horizon a far reader ages by
        # (§2.7, 0.2: "Across a face, the deployment's word names the
        # revision (§2.11)")
        r = al.read_owner(bus.g, "lab/svc", held_revisions(TC), across="granted",
                          named=[load_contract(TC).fingerprint, a2.standard().fingerprint])
        mine = {s_: j.firing.pair() for s_, j in r.slots.items()}
        takes = {k: a2.reader_slot([a2.SlotSeen(s_, r.collection.run_hex(), 0)], None)
                 for s_, k in ((0, KA), (1, KB))}
        code, doc, err = bus.zenctl("--across-face", "granted", "--contracts", str(TC),
                                    "zk2/lab/svc/alarms.v2/state/alarms/**")
        zs = {x.get("slot"): pair(x.get("firing")) for x in zen_alarms(doc)}
        own = zen_owner(doc, "lab/svc")
        rep.check(sec, "§2.15's far reader: established by the snapshot, each firing entry yes, firing (zk2py's "
                  "reader, which judges each entry by its slot); §2.6's reader with no run takes no slot (F-614)",
                  [r.verdict.expect(), mine, takes] == [{"verdict": "established", "reason": "snapshot"},
                                                        {0: YES_FIRING, 1: YES_FIRING}, {KA: None, KB: None}],
                  {"zk2py": mine, "takes": takes})
        rep.check(sec, "zenctl alarms --across-face granted: what the reference's reader answers of the same "
                  "two slots", True, {"exit": code, "owner": own, "slots": zs,
                                      "any_firing": (doc or {}).get("any_firing") if isinstance(doc, dict) else None,
                                      "stderr": err[-600:]})
    finally:
        bus.close()


def section17(rep: Report) -> None:
    """§6's state on the copy, read as across a face (granted), where "the
    deployment's word names the revision (§2.11): the alarms.v2 revision a
    reader is given, else the standard one, which every reader holds"
    (§2.7, 0.2's mend, F-622): zk2py's reader given a word that names the
    copy, one that names no alarms.v2 revision with the copy given and not
    given, and one that names the copy it does not hold; and ``zenctl
    alarms --across-face granted``, given the copy by ``--contracts`` and
    not given it. Not given, each reads the standard revision's 900 s, and
    calls the copy's stale A firing: "A deployment that runs another
    revision gives it"."""
    sec = "§6 across a face"
    bus = Bus(rep, sec)
    try:
        from .contract import load_contract

        short = _short_contract(bus)
        sc = al.standard(4)
        o = bus.owner("lab/svc", contracts=[short, TC])
        time.sleep(1.0)
        o.raise_(A)
        o.raise_(B)
        o.resolve(KB)
        time.sleep(0.5)
        o.cmd(f"alarm confirm {KA} off")
        time.sleep(7.0)
        tc = load_contract(TC).fingerprint
        out: dict[str, Any] = {}
        for name, named, files in (("the word names the copy, held", [tc, sc.fingerprint], [TC, short]),
                                   ("the word names no alarms.v2 revision, the copy given", [tc], [TC, short]),
                                   ("the word names no alarms.v2 revision, none given", [tc], [TC]),
                                   ("the word names the copy, not held", [tc, sc.fingerprint], [TC])):
            r = al.read_owner(bus.g, "lab/svc", held_revisions(*files), across="granted", named=named)
            out[f"zk2py: {name}"] = {"revision": r.revision, "horizon": r.horizon_s, "verdict": r.verdict.expect(),
                                     "slots": {s_: j.firing.pair() for s_, j in sorted(r.slots.items())}}
        for name, files in (("given the copy", [TC, short]), ("not given it", [TC])):
            code, doc, err = bus.zenctl("--across-face", "granted",
                                        *[x for f in files for x in ("--contracts", str(f))],
                                        "zk2/lab/svc/alarms.v2/state/alarms/**")
            out[f"zenctl: {name}"] = {"exit": code, "slots": {x["slot"]: pair(x.get("firing")) for x in zen_alarms(doc)},
                                      "any_firing": (doc or {}).get("any_firing", {}).get("answer")
                                      if isinstance(doc, dict) else None}
        nh = {"answer": "unobservable", "reason": "not_a_horizon"}
        rep.check(sec, "zk2py's far reader (0.2's mend, F-622): a word that names the copy it holds, or names no "
                  "alarms.v2 revision with the copy given, ages A at 4 s, unobservable, stale; one that names none "
                  "with none given reads the standard revision, 900 s, and A yes, firing; the copy named and "
                  "unheld gives no horizon, A unobservable, not_a_horizon; B no, resolved in each",
                  [(x["revision"], x["horizon"], x["slots"]) for k, x in out.items() if k.startswith("zk2py")]
                  == [("held", 4, {0: UNOBS_STALE, 1: NO_RESOLVED}), ("given", 4, {0: UNOBS_STALE, 1: NO_RESOLVED}),
                      ("standard", 900, {0: YES_FIRING, 1: NO_RESOLVED}),
                      ("not_in_hand", None, {0: nh, 1: NO_RESOLVED})], out)
        rep.check(sec, "zenctl alarms --across-face granted, given the copy: A unobservable, stale, at its 4 s; "
                  "B no, resolved", out["zenctl: given the copy"]["slots"] == {0: UNOBS_STALE, 1: NO_RESOLVED}, out)
        rep.check(sec, "zenctl alarms --across-face granted, not given the copy: the standard revision, A yes, "
                  "firing, B no, resolved, anything firing yes; zk2py's reader with none given agrees",
                  out["zenctl: not given it"]["slots"] == {0: YES_FIRING, 1: NO_RESOLVED}
                  and out["zenctl: not given it"]["any_firing"] == "yes"
                  and out["zenctl: not given it"]["slots"]
                  == out["zk2py: the word names no alarms.v2 revision, none given"]["slots"], out)
    finally:
        bus.close()


def _short_contract(bus: Bus) -> Path:
    """scenarios-v2, Conventions: "a copy of the standard contract with
    freshness.ttl_s = 4, made in memory and never published"; written to the
    run's working directory for the owner processes."""
    d = Path(bus.tmp) / "short"
    d.mkdir(exist_ok=True)
    (d / "schemas").mkdir(exist_ok=True)
    (d / "alarms.v2.json").write_bytes(a2.SCHEMA_PATH.read_bytes())
    (d / "schemas" / "alarms.v2.json").write_bytes(a2.WORDS_SCHEMA_PATH.read_bytes())
    f = d / "alarms.v2.toml"
    f.write_text(a2.CONTRACT.read_text(encoding="utf-8").replace('"freshness.ttl_s" = 900', '"freshness.ttl_s" = 4'),
                 encoding="utf-8")
    return f


def section2(rep: Report) -> None:
    sec = "§2"
    bus = Bus(rep, sec)
    try:
        short = _short_contract(bus)
        sc = al.standard(4)
        S = bus.listener("lab/svc", sc)
        o = bus.owner("lab/svc", contracts=[short, TC])
        time.sleep(1.0)
        h = {"rule": "hot", "severity": "warning", "summary": "Temperature 85.5 C", "reading": "85.5"}
        r0 = o.raise_(h)
        time.sleep(0.5)
        t = time.monotonic_ns()
        r1 = o.raise_(h, severity="critical")
        time.sleep(0.5)
        r2 = o.raise_(h, severity="critical", reading="86.0")
        time.sleep(0.3)
        r3 = o.raise_(h, severity="critical", reading="86.5")
        time.sleep(0.6)
        puts = rep.keep(sec, "1-3", S, t)
        recs = [p.get("record") for p in puts if p["key"] == "alarms/0"]
        rep.check(sec, "Expected 1: a put on slot 0, critical, its at later than t0; S reads escalated",
                  [len(recs) >= 1 and recs[0]["severity"] == "critical" and recs[0]["at"] > r0["record"]["at"],
                   [st.expect() for _, st in al.read_slots(S).get(0, [])][1:2]] == [True, [["escalated"]]],
                  {"raise": r0, "escalate": r1})
        rep.check(sec, "Expected 2: a put on slot 0, the escalation's at kept, argument 1260; S reads refreshed",
                  [len(recs) >= 2 and recs[1]["at"] == recs[0]["at"] and recs[1]["argument"] == 1260,
                   [st.expect() for _, st in al.read_slots(S).get(0, [])][2:3]] == [True, [["refreshed"]]], r2)
        rep.check(sec, "Expected 3: no put within the refresh window of step 2 (half the 4 s horizon)",
                  len(recs) == 2, {"step 3": r3, "records": recs})
        t = time.monotonic_ns()
        time.sleep(6.0)
        later = [p for p in rep.keep(sec, "4 wait past the horizon", S, t) if p["key"] == "alarms/0"]
        wires = S.heartbeat_wires(t)
        from . import heartbeat, zk2c
        env = zk2c.parse(wires[-1], "put") if wires else None
        body = heartbeat.read(a2.heartbeat_layout(sc.canonical, sc.fingerprint), env.record or b"") if env else None
        obs = S.observation(0)
        v = v1.fr.judge("state", v1.fr.horizon("state", {v1.fr.KEY: 4}), [obs])
        rep.check(sec, "Expected 4: heartbeats at least every half horizon, form 6, a 17-byte payload, live count 1, "
                  "slot 0's bit set; H yes, firing past the horizon",
                  [bool(wires), env and env.form, len(wires[-1]) if wires else None,
                   body and body["sections"].get(a2.ALARMS_RESOURCE, {}).get("live"),
                   body and body["sections"].get(a2.ALARMS_RESOURCE, {}).get("vouched"), v.verdict]
                  == [True, "heartbeat", 17, 1, [0], "fresh"],
                  {"heartbeats": len(wires), "tag": env and env.tag, "copy's tag": heartbeat.tag_of(sc.fingerprint),
                   "last": wires[-1].hex() if wires else None, "after the window": later})
        got = readers(bus, "lab/svc", short)
        rep.check(sec, "Expected 4, every reader on the copy (0.2's Conventions: \"S, G and the tools hold the copy. "
                  "Each is given it, or retrieves it\"): zk2py's G and zenctl alarms, each given the copy and not "
                  "given it, age H at the copy's 4 s, and answer it yes, firing, past that horizon, by its "
                  "confirmations",
                  all(x["slots"].get(0) == YES_FIRING for x in got.values())
                  and [x.get("horizon") for k, x in got.items() if k.startswith("zk2py")] == [4, 4],
                  got)
    finally:
        bus.close()


def readers(bus: Bus, address: str, short: Path, *, given: bool = True) -> dict[str, Any]:
    """§2.7 (0.2), "The horizon is the revision's": zk2py's G
    (:func:`zk2py.alarms_v2_live.read_owner`) and ``zenctl alarms``, each
    given the copy beside ``tc.v1`` (``--contracts``), and given ``tc.v1``
    alone, so that each retrieves the revision the descriptor names (core
    §8.4) -> each reader's answer to "is this alarm firing?" by slot, and
    what it had of the revision."""
    service = address.split("/", 1)[1]
    out: dict[str, Any] = {}
    for name, files in ((("given", [TC, short]),) if given else ()) + (("not given", [TC]),):
        r = al.read_owner(bus.g, address, held_revisions(*files))
        out[f"zk2py, {name}"] = {"revision": r.revision, "horizon": r.horizon_s,
                                 "slots": {s_: j.firing.pair() for s_, j in sorted(r.slots.items())}}
        code, doc, err = bus.zenctl(*[x for f in files for x in ("--contracts", str(f))], f"zk2/{address}/**")
        out[f"zenctl, {name}"] = {
            "exit": code, "slots": {x["slot"]: pair(x.get("firing")) for x in zen_alarms(doc)
                                    if (x.get("alarm") or {}).get("service", service) == service
                                    and x.get("slot") is not None},
            "owner": {k: v for k, v in (zen_owner(doc, address) or {}).items() if k in ("verdict", "reason")},
            "any_firing": (doc or {}).get("any_firing") if isinstance(doc, dict) else None,
            **({"stderr": err.strip()[-300:]} if err.strip() else {})}
    return out


def section6(rep: Report) -> None:
    _section6(rep, first_hop=False)


def section16(rep: Report) -> None:
    """§6 on the copy, its owner on a first hop (core §8.1, §8.4: "no
    bundle held"): a reader not given the copy can neither hold nor retrieve
    the revision the descriptor names (§2.7, 0.2: "A revision not in hand
    gives no horizon ... reads that owner's alarms' freshness as
    unobservable, never at 900 s"; round 127)."""
    _section6(rep, first_hop=True)


UNOBS_STALE = {"answer": "unobservable", "reason": "stale"}


def _section6(rep: Report, first_hop: bool) -> None:
    sec = "§6" + (", the owner on a first hop" if first_hop else "")
    bus = Bus(rep, sec)
    try:
        short = _short_contract(bus)
        sc = al.standard(4)
        S = bus.listener("lab/svc", sc)
        o = bus.owner("lab/svc", contracts=[short, TC], first_hop=first_hop)
        time.sleep(1.0)
        o.raise_(A)
        o.raise_(B)
        o.resolve(KB)
        time.sleep(0.5)
        o.cmd(f"alarm confirm {KA} off")
        t = time.monotonic_ns()
        time.sleep(7.0)
        wires = S.heartbeat_wires(t)
        from . import heartbeat, zk2c
        env = zk2c.parse(wires[-1], "put") if wires else None
        body = heartbeat.read(a2.heartbeat_layout(sc.canonical, sc.fingerprint), env.record or b"") if env else None
        sec_ = (body or {}).get("sections", {}).get(a2.ALARMS_RESOURCE) or {}
        hz = v1.fr.horizon("state", {v1.fr.KEY: 4})
        va = v1.fr.judge("state", hz, [S.observation(0)])
        ra = a2.judge_reading(a2.Reading("present", {"lists": True}, None, (va.verdict, va.reason), "firing"))
        rs = a2.judge_reading(a2.Reading("present", {"lists": True}, None, (va.verdict, va.reason), "firing"), v1.Q_STALE)
        rep.check(sec, "Expected 1 and 2: no put of A; each heartbeat live count 2, both bits clear; S judges A "
                  "unobservable, stale, and the last question yes, stale; B no, resolved",
                  [[x["key"] for x in S.all(t) if "/state/alarms/" in x["key"]], sec_.get("live"), sec_.get("vouched"),
                   ra.pair(), rs.pair()]
                  == [[], 2, [], UNOBS_STALE, {"answer": "yes", "reason": "stale"}],
                  {"heartbeat": wires[-1].hex() if wires else None, "section": sec_})
        r = al.read_owner(bus.g, "lab/svc", held_revisions(TC, short))
        snap = r.collection.snapshot() if r.collection else None
        rep.check(sec, "Expected 2: G's snapshot lists A with its last confirmation, past the horizon, and B with its own",
                  [[(e["slot"], e["confirmation"]) for e in (snap or {}).get("live", [])]] == [[(0, "conf"), (1, "conf")]],
                  (snap or {}).get("live"))
        got = readers(bus, "lab/svc", short)
        given = {k: v for k, v in got.items() if k.endswith(", given")}
        rep.check(sec, "Expected 2, every reader given the copy (0.2's Conventions, F-613's answer): zk2py's G and "
                  "zenctl alarms age A at the copy's 4 s: unobservable, stale at 7 s; B no, resolved",
                  all(x["slots"].get(0) == UNOBS_STALE and x["slots"].get(1) == NO_RESOLVED for x in given.values()),
                  given)
        nog = {k: v for k, v in got.items() if k.endswith("not given")}
        if not first_hop:
            rep.check(sec, "Expected 2, every reader not given the copy: each retrieves the revision the descriptor "
                      "names (core §8.4), and ages A at its 4 s: unobservable, stale; B no, resolved",
                      all(x["slots"].get(0) == UNOBS_STALE and x["slots"].get(1) == NO_RESOLVED for x in nog.values())
                      and nog["zk2py, not given"]["revision"] == "retrieved", nog)
        else:
            zs = nog["zenctl, not given"]["slots"].get(0) or {}
            rep.check(sec, "§2.7 (0.2), a revision not in hand: the owner on a first hop holds no bundle, so a reader "
                      "not given the copy neither holds nor can retrieve it; each reads A's freshness unobservable, "
                      "never at 900 s (zk2py: not_a_horizon), and B no, resolved, whatever its age",
                      nog["zk2py, not given"]["slots"].get(0) == {"answer": "unobservable", "reason": "not_a_horizon"}
                      and nog["zk2py, not given"]["revision"] == "not_in_hand" and zs.get("answer") == "unobservable"
                      and all(x["slots"].get(1) == NO_RESOLVED for x in nog.values()), nog)
            rep.note(sec, f"zenctl's reason for a revision not in hand: {zs.get('reason')!r}; its owner row "
                          f"{nog['zenctl, not given'].get('owner')}, any_firing {nog['zenctl, not given'].get('any_firing')}")
    finally:
        bus.close()


def conform(bus: Bus, rep: Report, sec: str, address: str, during: Any = None) -> None:
    """``check conform``'s ``alarm-*`` cases, and its others, of an
    ``alarms.v2`` owner, over a 4 s window during which ``during`` acts."""
    res: dict[str, Any] = {}

    def run() -> None:
        res["z"] = subprocess.run([str(ZENCTL), "check", "conform", "-c", bus.endpoint, "--format", "json",
                                   "--clocks-synced", address, "alarms.v2", "--for", "4"],
                                  capture_output=True, text=True, timeout=120)

    t = threading.Thread(target=run)
    t.start()
    time.sleep(1.5)
    if during is not None:
        during()
    t.join()
    z = res["z"]
    try:
        doc = json.loads(z.stdout)
    except ValueError:
        doc = {"unparsed": z.stdout[-3000:]}
    rows = [r for r in doc.get("rows") or [] if r.get("row") == "case"]
    verdicts = {f"{r['case']} {r.get('subject')}": (r.get("verdict") or {}).get("answer") for r in rows}
    alarm = {k: v for k, v in verdicts.items() if k.startswith("alarm")}
    rep.check(sec, f"check conform {address} alarms.v2: no violation, its alarm-key and alarm-document cases "
              "established no violation", z.returncode in (0, 2) and len(alarm) == 2
              and all(v == "not_established" for v in alarm.values())
              and not any(v == "violation" for v in verdicts.values()),
              {"exit": z.returncode, "alarm": alarm, "others": {k: v for k, v in verdicts.items() if v != "not_asked"},
               "reasons": {f"{r['case']} {r.get('subject')}": (r.get("verdict") or {}).get("reason")
                           for r in rows if r["case"].startswith("alarm")}, "stderr": z.stderr[-400:]})


def main(argv: list[str] | None = None) -> int:
    global PAIRING
    ap = argparse.ArgumentParser(prog="python -m zk2py.alarms_v2_scenarios")
    ap.add_argument("--pairing", choices=("ref-owner", "zk2py-owner"), default="ref-owner")
    ap.add_argument("--only", action="append")
    ap.add_argument("--out")
    a = ap.parse_args(argv)
    PAIRING = a.pairing
    rep = Report()
    sections = {k[len("section"):]: v for k, v in globals().items() if k.startswith("section") and callable(v)}
    for name in sorted(sections, key=lambda x: int(x)):
        if a.only and name not in a.only:
            continue
        try:
            sections[name](rep)
        except Exception as e:  # noqa: BLE001 - one section's crash is its failure
            import traceback

            traceback.print_exc()
            rep.check(f"§{name}", "the section ran", False, repr(e))
    if a.out:
        Path(a.out).mkdir(parents=True, exist_ok=True)
        def plain(x: Any) -> Any:
            if isinstance(x, dict):
                return {str(k): plain(v) for k, v in x.items()}
            if isinstance(x, (list, tuple)):
                return [plain(v) for v in x]
            return x if isinstance(x, (str, int, float, bool)) or x is None else str(x)

        name = f"{PAIRING}{'-' + '-'.join(a.only) if a.only else ''}.json"
        (Path(a.out) / name).write_text(json.dumps(plain({"rows": rep.rows, "puts": rep.puts}), indent=1))
    counted = [r for r in rep.rows if r.get("counted", True)]
    bad = [r for r in counted if not r["ok"]]
    print(f"{len(counted) - len(bad)} of {len(counted)} checks passed", flush=True)
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())

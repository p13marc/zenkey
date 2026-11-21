"""``freshness.v2`` 0.9's owner heartbeat on a bus, between two zk2py
processes (#609, round 113): ``python -m zk2py.owner_heartbeat_live``.

No section of ``scenarios-v2.md`` runs one on this main (the text cites a
§8 the file does not hold, F-547), and the reference's owner sends none, so
the steps are zk2py's own, each a sentence of ``v2.md`` §2.4, §2.5 or §3.

**The parties.** The parent holds R1, a zenoh-python router on a free
loopback port, and the readers, each a client of it. The owner is a child
process, ``--owner``: zk2py's :class:`zk2py.owner.Owner` at ``lab/node``,
given a catalog and told to cover every interface it may
(:attr:`Owner.owner_heartbeats`), driven by lines on its standard input.

**The contracts** are ``interop/freshness/zk2py_ohb_{a,b,c}.v1.toml``:
three interfaces, ``a`` with a ``link`` collection of one member
(``status``, 2 s) and a ``host`` templated one (``peer/{peer}``, 4 s), ``b``
a ``link`` one (``config``, 2 s), ``c`` a templated ``link`` one
(``alarms/{alarm}``, 4 s). The catalog lists the three, and the standard
contract's revision 1.1.

**The steps.**
1. The owner starts, puts ``a``'s status and two peers, ``b``'s config and
   two of ``c``'s alarms. Its descriptor names the catalog and lists
   ``freshness.v2`` with nothing unavailable.
2. S, a :class:`zk2py.freshness_v2.Provider` that holds the catalog,
   subscribes: seeded for ``a`` and ``c``, and not for ``b`` (a late joiner
   with no seed, §2.5). It listens 5 s.
3. The owner loses one ``link`` owner heartbeat (its seq taken, no put). S
   listens until two more arrive.
4. N, a Provider that holds no catalog, listened beside S from step 2.
5. A second owner, ``lab/alone``, given no catalog, serves ``b`` alone. S2
   subscribes to both forms.

Exit 0 when every check passes, 1 when any fails.
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

from . import catalog as cat
from . import freshness_v2 as fv2
from . import heartbeat, owner_heartbeat, zk2c

REPO = Path(__file__).resolve().parents[3]
INTEROP = REPO / "impl" / "python" / "interop" / "freshness"
FILES = [INTEROP / f"zk2py_ohb_{n}.v1.toml" for n in "abc"]
ADDRESS = "lab/node"
A, B, C = "zk2py_ohb_a.v1", "zk2py_ohb_b.v1", "zk2py_ohb_c.v1"
BASE = f"zk2/{ADDRESS}"
STATUS, CONFIG = f"{BASE}/{A}/state/status", f"{BASE}/{B}/state/config"
PEERS = [f"{BASE}/{A}/state/peer/p1", f"{BASE}/{A}/state/peer/p2"]
ALARMS = [f"{BASE}/{C}/state/alarms/x", f"{BASE}/{C}/state/alarms/y"]
JITTER_NS = 300_000_000


def _contracts() -> list[Any]:
    from .contract import load_contract

    out = [load_contract(f) for f in FILES]
    for c in out:
        if not c.valid:
            raise SystemExit(f"{c.path}: {c.codes}")
    return out


def catalog_doc() -> dict[str, Any]:
    """The run's catalog: the three interfaces, then the standard contract."""
    entries = [(c.interface, c.fingerprint) for c in _contracts()]
    std = fv2.standard_contract()
    entries.append((std.interface, std.fingerprint))
    return {"format": cat.FORMAT, "epoch": 1_700_000_000,
            "entries": [{"index": i, "iface": n, "contract": fp} for i, (n, fp) in enumerate(entries)]}


# -- the owner, a process of its own ----------------------------------------------------

def run_owner(endpoint: str, catalog_path: str) -> int:
    from .owner import Owner

    o = Owner("lab", "node", _contracts(), connect=endpoint)
    o.catalog = cat.read(Path(catalog_path).read_bytes())
    o.owner_heartbeats = True
    o.start()
    print(f"ready {o.instance_key}", flush=True)
    for line in sys.stdin:
        words = line.split()
        if not words:
            continue
        try:
            if words[0] == "put":
                o.set_state(words[1], words[2].encode())
            elif words[0] == "delete":
                o.delete_state(words[1])
            elif words[0] == "lose":
                o.ohb_lose = frozenset(o.ohb_lose) | {words[1]}
            elif words[0] == "report":
                print("report " + json.dumps({
                    "owner_heartbeats": [[cls, ib.seq, sorted(v)] for cls, ib, v in o.owner_heartbeats_sent],
                    "lost": [[cls, ib.seq] for cls, ib in o.ohb_lost],
                    "heartbeats": len(o.heartbeats_sent), "covered": o.ohb_index}), flush=True)
                continue
            elif words[0] == "quit":
                break
            else:
                print(f"refused {line.strip()}: unknown command", flush=True)
                continue
            print(f"ok {line.strip()}", flush=True)
        except Exception as e:  # noqa: BLE001 - said, and the run goes on
            print(f"refused {line.strip()}: {type(e).__name__}: {e}", flush=True)
    o.close()
    return 0


class Child:
    """The owner process, read line by line."""

    def __init__(self, endpoint: str, catalog_path: str):
        self.p = subprocess.Popen([sys.executable, "-m", "zk2py.owner_heartbeat_live", "--owner", endpoint,
                                   catalog_path], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                  stderr=subprocess.PIPE, text=True)
        self.lines: list[str] = []
        self.err: list[str] = []
        self.cv = threading.Condition()
        threading.Thread(target=self._pump, args=(self.p.stdout, self.lines), daemon=True).start()
        threading.Thread(target=self._pump, args=(self.p.stderr, self.err), daemon=True).start()
        self.at = 0

    def _pump(self, stream: Any, into: list[str]) -> None:
        for line in stream:
            with self.cv:
                into.append(line.rstrip("\n"))
                self.cv.notify_all()

    def wait_for(self, prefix: str, timeout: float = 30.0) -> str | None:
        end = time.monotonic() + timeout
        with self.cv:
            while True:
                for i in range(self.at, len(self.lines)):
                    if self.lines[i].startswith(prefix):
                        self.at = i + 1
                        return self.lines[i]
                left = end - time.monotonic()
                if left <= 0 or self.p.poll() is not None and self.at >= len(self.lines):
                    return None
                self.cv.wait(min(left, 0.2))

    def command(self, line: str, answer: str = "ok ", timeout: float = 10.0) -> str | None:
        assert self.p.stdin is not None
        self.p.stdin.write(line + "\n")
        self.p.stdin.flush()
        return self.wait_for(answer, timeout)

    def close(self) -> None:
        try:
            if self.p.poll() is None:
                assert self.p.stdin is not None
                self.p.stdin.write("quit\n")
                self.p.stdin.flush()
                self.p.wait(timeout=10)
        except Exception:  # noqa: BLE001 - it is ended below either way
            pass
        if self.p.poll() is None:
            self.p.kill()  # the process this runner started, by its own handle


# -- the run ----------------------------------------------------------------------------

def _sections(payload: bytes, held: cat.Catalog, layouts: dict[tuple[str, str], heartbeat.Layout]) -> dict[str, Any]:
    """An owner heartbeat's groups, each read whole with its revision: the
    sections its bytes hold, by interface."""
    put = owner_heartbeat.read(payload)
    out = {}
    for g in owner_heartbeat.read_groups(put.body, held.id, [held], layouts):
        assert g.entry is not None
        out[g.entry[0]] = g.sections
    return out


def _entry(session: Any, address: str) -> dict[str, Any] | None:
    """``freshness.v2``'s entry in the descriptor of ``address``'s present
    instance, read on the bus."""
    from . import live

    pres = live.list_presence(session, f"zk2/{address}/@zk/instance/*")
    if not pres.instances:
        return None
    d = live.get_descriptor(session, f"zk2/{address}/@zk/instance/{pres.instances[0]['instance']}", timeout=1.0)
    if len(d) != 1 or not d[0].ok:
        return None
    return next((e for e in json.loads(d[0].payload)["interfaces"] if e["iface"] == "freshness.v2"), None)


def alone(session: Any, endpoint: str, held: cat.Catalog, check: Any) -> None:
    """Step 5: an owner given no catalog, in this process: "An owner given
    no catalog writes no index. It sends heartbeats alone." S2 subscribes to
    both forms and hears the one."""
    import zenoh

    from .owner import Owner

    c = _contracts()[1]
    o = Owner("lab", "alone", [c], connect=endpoint)
    o.owner_heartbeats = True  # asked, and no catalog given: nothing is covered
    o.start()
    heard: list[str] = []
    sub = session.declare_subscriber("zk2/lab/alone/freshness.v2/stream/heartbeats/*", zenoh.handlers.Callback(
        lambda sample: heard.append(str(sample.key_expr))))
    try:
        key = f"zk2/lab/alone/{B}/state/config"
        o.set_state(key, b"v1")
        s2 = fv2.Provider(session, "lab/alone", [(c.canonical, c.fingerprint, None)], held=[held], seed=[B],
                          wait_s=0.0)
        try:
            time.sleep(2.5)
            entry = _entry(session, "lab/alone")
            beats = s2.subs[B].answers()
            check("step 5: an owner given no catalog sends heartbeats alone: S2, subscribed to both forms, hears "
                  "b's heartbeat and no owner heartbeat, and config is fresh (§2.4 \"Which interfaces\", §2.5 "
                  "\"Both forms\")",
                  len(beats) >= 2 and not heard and not s2.answers() and o.ohb_index == {}
                  and s2.judge(key).verdict == "fresh" and s2.reader.named is None,
                  f"{len(beats)} heartbeats, {len(heard)} puts on a class key")
            check("step 5: it does not expose heartbeats/{class}: its freshness.v2 entry, revision 1.1, lists the "
                  "resource unavailable, the cause config zk2py's choice (§3, core §3.3; F-549)",
                  entry is not None and entry.get("unavailable") == [{"resource": "stream/heartbeats/{class}",
                                                                     "cause": "config"}], json.dumps(entry))
        finally:
            s2.close()
    finally:
        sub.undeclare()
        o.close()


def run() -> int:
    from . import live
    from .blackbox import workdir
    from .live_interop import _r1

    rows: list[tuple[str, bool, str]] = []

    def check(name: str, ok: bool, detail: str = "") -> bool:
        rows.append((name, bool(ok), detail))
        print(f"{'PASS' if ok else 'FAIL'} {name}" + (f": {detail}" if detail else ""), flush=True)
        return bool(ok)

    contracts = _contracts()
    held = cat.read(json.dumps(catalog_doc()))
    layouts = {(c.interface, c.fingerprint): heartbeat.layout_of(c.canonical, c.fingerprint) for c in contracts}
    router, endpoint, _ = _r1()
    sessions = [live.open_client(endpoint) for _ in range(2)]
    child = None
    providers: list[fv2.Provider] = []
    try:
        with workdir("zk2py-ohb-") as tmp:
            path = Path(tmp) / "catalog.json"
            path.write_text(json.dumps(catalog_doc()))
            child = Child(endpoint, str(path))
            ready = child.wait_for("ready ", 60)
            if not check("step 1: the owner process starts", ready is not None, "\n".join(child.err[-5:])):
                return 1
            for k in [STATUS, CONFIG, *PEERS, *ALARMS]:
                child.command(f"put {k} v1")
            named = fv2.descriptor_catalog(sessions[0], ADDRESS)
            check("step 1: its descriptor names the catalog it was given (core §3.3)", named == held.id,
                  f"{named} and {held.id}")
            std = fv2.standard_contract()
            entry = _entry(sessions[0], ADDRESS)
            check("step 1: it implements the standard contract's revision 1.1 beside the three, and exposes "
                  "heartbeats/{class}: its entry lists nothing unavailable (§3)",
                  entry is not None and entry.get("contract") == std.fingerprint and std.minor == 1
                  and "unavailable" not in entry, json.dumps(entry))
            t0 = time.monotonic_ns()
            revs = [(c.canonical, c.fingerprint, None) for c in contracts]
            # S holds the catalog; N holds none. S seeds a and c, never b.
            s = fv2.Provider(sessions[0], ADDRESS, revs, held=[held], seed=[A, C], wait_s=0.0)
            n = fv2.Provider(sessions[1], ADDRESS, revs, held=[], reread=False, wait_s=0.0)
            providers += [s, n]
            check("step 2: S subscribes to one owner heartbeat key per class its collections have (§2.5 \"Both "
                  "forms\")", sorted(s.keys) == ["host", "link"],
                  json.dumps(s.keys))
            time.sleep(5.0)
            link, host = s.answers("link"), s.answers("host")
            read_link = [h for h in link if "read" in h.answer]
            check("step 2: three interfaces in one owner heartbeat: every link put read, a group for a, b and c in "
                  "ascending index (§2.4 \"The groups\")",
                  len(read_link) >= 4 and all([g.get("iface") for g in h.answer["read"]["groups"]] == [A, B, C]
                                              for h in read_link),
                  f"{len(link)} heard, {len(read_link)} read")
            check("step 2: one host owner heartbeat holds a's group alone, b and c having no host collection",
                  len(host) >= 2 and all([g.get("iface") for g in h.answer.get("read", {}).get("groups", [])] == [A]
                                         for h in host), f"{len(host)} heard")
            # "In a group, an owner MUST set the map bit of no collection of another class."
            on_link = [_sections(h.payload, held, layouts) for h in link]
            on_host = [_sections(h.payload, held, layouts) for h in host]
            check("step 2: a host collection never on the link key: no link put's bytes hold a section of "
                  "a's peer/{peer}, and S reads it other_class there (§2.4 \"Only its class\", §2.5)",
                  bool(on_link) and all("state/peer/{peer}" not in x[A] and "state/status" in x[A] for x in on_link)
                  and all(h.answer["read"]["groups"][0]["sections"]["state/peer/{peer}"] == "other_class"
                          for h in read_link),
                  json.dumps(sorted({r for x in on_link for r in x[A]})))
            check("step 2: no link collection on the host key: its bytes hold peer/{peer}'s section alone, and S "
                  "reads status other_class there",
                  bool(on_host) and all(sorted(x[A]) == ["state/peer/{peer}"] for x in on_host)
                  and all(h.answer["read"]["groups"][0]["sections"]["state/status"] == "other_class" for h in host),
                  json.dumps(sorted({r for x in on_host for r in x[A]})))
            # "One form per interface": no heartbeat on any covered interface's own key.
            beats = {i: len(sub.answers()) for i, sub in s.subs.items()}
            check("step 2: no heartbeat covers an interface its owner heartbeats cover (§2.4 \"One form per "
                  "interface\"): S hears none on the three heartbeat keys it subscribes to", not any(beats.values()),
                  json.dumps(beats))

            def apart(hs: list[Any]) -> int:
                at = [h.arrival_ns for h in hs]
                return max((b - a for a, b in zip(at, at[1:])), default=0)

            check("step 2: the link key's puts are at most half its shortest horizon apart, 1 s of status's and "
                  "config's 2 s, and the host key's 2 s of peer's 4 s (§2.4 \"The period\"), plus jitter",
                  0 < apart(link) <= 1_000_000_000 + JITTER_NS and 0 < apart(host) <= 2_000_000_000 + JITTER_NS,
                  f"{apart(link) / 1e9:.3f} s and {apart(host) / 1e9:.3f} s")
            # A divergence in one group: b was never seeded.
            b_answers = [h.answer["read"]["groups"][1]["sections"]["state/config"] for h in read_link]
            others = [(h.answer["read"]["groups"][0]["sections"]["state/status"],
                       h.answer["read"]["groups"][2]["sections"]["state/alarms/{alarm}"]) for h in read_link]
            first_ok = next((i for i, x in enumerate(b_answers) if isinstance(x, dict)), len(b_answers))
            check("step 2: a divergence in one group: b's group diverges twice, S holding no member of it, while "
                  "a's and c's groups of the same puts confirm their members; a group that confirms nothing takes "
                  "nothing from the groups beside it (§2.5)",
                  b_answers[:2] == ["diverged", "diverged"]
                  and all(isinstance(x, dict) and isinstance(y, dict) and x["confirmed"] == [STATUS]
                          and y["confirmed"] == ALARMS for x, y in others),
                  json.dumps(b_answers[:4]))
            check("step 2: the re-read follows two in a row, and b's group confirms config from then on (§2.5 "
                  "\"The re-read\")",
                  len(s.subs[B].reads) == 1 and 2 <= first_ok < len(b_answers)
                  and all(isinstance(x, dict) and x["confirmed"] == [CONFIG] for x in b_answers[first_ok:]),
                  f"{len(s.subs[B].reads)} read, confirmed from the link put {first_ok}")
            verdicts = {k: s.judge(k).pair() for k in [STATUS, CONFIG, *PEERS, *ALARMS]}
            check("step 2: every member is fresh to S, each confirmed on its class's key",
                  all(v.get("verdict") == "fresh" for v in verdicts.values()), json.dumps(verdicts))
            seqs = [h.stamp.seq for h in link if h.stamp is not None]
            check("step 2: the link key's seqs are its own, in sequence, and no put read after a gap",
                  seqs == list(range(seqs[0], seqs[0] + len(seqs))) and not any(h.answer["read"]["gap"]
                                                                                  for h in read_link),
                  f"{seqs[0]} to {seqs[-1]}")
            # N holds no catalog.
            n_link = n.answers("link")
            check("step 4: N holds no catalog, and reads the id its owner names in its descriptor: every group's "
                  "index is refused unknown_catalog, the owner heartbeat still a read one, and nothing is "
                  "confirmed (§2.5 \"No index\")",
                  bool(n_link) and all("read" in h.answer and all(g.get("refused") == "unknown_catalog"
                                                                    for g in h.answer["read"]["groups"])
                                       for h in n_link) and not any(sub.last_conf for sub in n.subs.values()),
                  f"{len(n_link)} heard, named {n.reader.named}")
            # Step 3: one lost.
            before = len(s.answers("link"))
            host_before = len(s.answers("host"))
            child.command("lose link")
            end = time.monotonic() + 6.0
            while time.monotonic() < end and len(s.answers("link")) < before + 1:
                time.sleep(0.02)
            after_gap = s.answers("link")[before:before + 1]
            lost_now = sorted(s.reader.lost("link"))
            obs = {k: s.subs[k.split("/")[3]].observation(k).complete for k in [STATUS, CONFIG, *PEERS, *ALARMS]}
            check("step 3: one owner heartbeat lost: the next link put read follows a gap in the key's seqs",
                  len(after_gap) == 1 and after_gap[0].answer.get("read", {}).get("gap") is True,
                  json.dumps(after_gap[0].answer)[:160] if after_gap else "none heard")
            check("step 3: it is lost for every interface the key covers at once: the link collections of a, b and "
                  "c read as lost deliveries, and a's host collection does not (§2.5 \"A lost owner heartbeat\", "
                  "\"Two keys are two sequences\")",
                  lost_now == [(A, "state/status"), (B, "state/config"), (C, "state/alarms/{alarm}")]
                  and obs == {STATUS: False, CONFIG: False, PEERS[0]: True, PEERS[1]: True, ALARMS[0]: False,
                              ALARMS[1]: False}, json.dumps(obs))
            check("step 3: the owner heartbeat that follows the gap still confirms its groups' members",
                  len(after_gap) == 1 and [sorted(g["sections"]) for g in after_gap[0].answer["read"]["groups"]]
                  and all(any(isinstance(x, dict) and x["confirmed"] for x in g["sections"].values())
                          for g in after_gap[0].answer["read"]["groups"]))
            while time.monotonic() < end and len(s.answers("link")) < before + 2:
                time.sleep(0.02)
            nxt = s.answers("link")[before + 1:before + 2]
            obs2 = {k: s.subs[k.split("/")[3]].observation(k).complete for k in [STATUS, CONFIG, *ALARMS]}
            check("step 3: until an owner heartbeat arrives in sequence on that key: the next one does, and "
                  "nothing reads lost", len(nxt) == 1 and nxt[0].answer["read"]["gap"] is False
                  and s.reader.lost("link") == [] and all(obs2.values()), json.dumps(obs2))
            check("step 3: the host key heard no gap", not any(h.answer["read"]["gap"] for h in s.answers("host"))
                  and len(s.answers("host")) >= host_before)
            got = child.command("report", "report ")
            rep = json.loads(got.split(" ", 1)[1]) if got else {}
            check("the owner's own count: it covers the three by their catalog indices, sent no heartbeat and "
                  "lost one link owner heartbeat",
                  rep.get("covered") == {A: 0, B: 1, C: 2} and rep.get("heartbeats") == 0
                  and [x[0] for x in rep.get("lost", [])] == ["link"],
                  json.dumps({k: rep.get(k) for k in ("covered", "heartbeats", "lost")}))
            sent = [x for x in rep.get("owner_heartbeats", []) if x[0] == "link"]
            check("every link owner heartbeat the owner sent from S's first one on reached S",
                  bool(seqs) and [x[1] for x in sent if x[1] >= seqs[0]][:len(s.answers("link"))]
                  == [h.stamp.seq for h in s.answers("link") if h.stamp is not None],
                  f"{len(sent)} sent, {len(s.answers('link'))} heard")
            alone(sessions[1], endpoint, held, check)
            sizes = sorted({len(h.payload) for h in s.answers("link")})
            check("the link put's size is core §7.5's: 6 bytes over its body under a 16-bit run, or 8",
                  all(len(h.payload) == owner_heartbeat.size_of(
                      h.stamp.width, [len(g.bits) for g in owner_heartbeat.read(h.payload).body.groups])
                      for h in s.answers("link") if h.stamp is not None), f"{sizes} bytes")
    finally:
        for p in providers:
            p.close()
        if child is not None:
            child.close()
        for x in sessions:
            x.close()
        router.close()
    failed = [r for r in rows if not r[1]]
    print(f"owner_heartbeat_live: {len(rows) - len(failed)}/{len(rows)}", flush=True)
    return 1 if failed else 0


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="python -m zk2py.owner_heartbeat_live", description=__doc__.split("\n")[0])
    ap.add_argument("--owner", nargs=2, metavar=("ENDPOINT", "CATALOG"), help="run the owner process")
    args = ap.parse_args(argv)
    if args.owner:
        return run_owner(*args.owner)
    return run()


if __name__ == "__main__":
    sys.exit(main())

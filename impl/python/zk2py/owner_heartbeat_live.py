"""``freshness.v2`` 0.9's owner heartbeat on a bus, between two zk2py
processes (#609, round 113): ``python -m zk2py.owner_heartbeat_live``.

The steps are zk2py's own, each a sentence of ``v2.md`` §2.4, §2.5 or §3:
round 113 wrote them when ``scenarios-v2.md`` held no §8 (F-547) and the
reference's owner sent none. Since text 0.10 the file holds §8, which
:mod:`zk2py.freshness_v2_s8_live` runs as worded, with the reference's
owner too; these steps stay, for what §8 does not hold: a link key of three
groups, a reader with no catalog, an owner given none. ``--forms`` (round
115) puts another form on a heartbeat's own key beside each owner's.

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
import os
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
            elif words[0] == "miss":
                # a put the owner makes and no reader hears: its stamp moves,
                # and the put is dropped at the publisher (a test rig's)
                with o._writes:
                    real = o._publishers[words[1]]
                    o._publishers[words[1]] = _Dropped()
                    try:
                        o.set_state(words[1], words[2].encode())
                    finally:
                        o._publishers[words[1]] = real
            elif words[0] == "mute":
                # a test rig's: the instance's descriptor queryable undeclared, so
                # that no descriptor answers a GET while the owner keeps putting
                o._instance_entities[0].undeclare()
            elif words[0] == "slow":
                # a test rig's (round 125, deliverable 3): the descriptor
                # queryable answers each GET, but only <ms> after it arrives,
                # as a slow owner's would: a reader whose timeout is shorter
                # draws what zenoh gives a GET left unanswered at its end
                import zenoh

                delay = int(words[1]) / 1000
                o._instance_entities[0].undeclare()
                key = o.instance_key
                doc = o._docs.get(o.instance) if hasattr(o, "_docs") else None
                if doc is None:
                    # a rig that would answer nothing reads as silence, not as
                    # a late answer: refuse it at once (review-bot on #1171)
                    print("error: slow: the owner holds no descriptor to answer with", flush=True)
                    continue

                def late(q: Any, key: str = key) -> None:
                    def answer() -> None:
                        time.sleep(delay)
                        try:
                            q.reply(key, o._docs.get(o.instance, doc) if hasattr(o, "_docs") else doc,
                                    encoding="application/json")
                        except Exception:  # noqa: BLE001 - its GET has ended: nothing to answer
                            pass
                        finally:
                            q.drop()
                    threading.Thread(target=answer, daemon=True).start()

                assert o.session is not None
                o._instance_entities[0] = o.session.declare_queryable(key, zenoh.handlers.Callback(late),
                                                                      complete=True)
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


class _Dropped:
    """A publisher whose puts go nowhere."""

    def put(self, *a: Any, **k: Any) -> None:
        return None


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
    try:
        put = owner_heartbeat.read(payload)
    except owner_heartbeat.Refused:
        return {}  # a put §7.5 refuses holds no section: the checks then fail, and the run goes on
    out = {}
    for g in owner_heartbeat.read_groups(put.body, held.id, [held], layouts):
        if g.entry is not None:
            out[g.entry[0]] = g.sections or {}
    return out


def _groups(h: Any) -> list[dict[str, Any]]:
    """A heard owner heartbeat's groups as the reader answered them; none
    where it was refused or older."""
    return h.answer.get("read", {}).get("groups", [])


def _gap(h: Any) -> Any:
    """Whether a heard owner heartbeat was read after a gap; None where it
    was not read."""
    return h.answer.get("read", {}).get("gap")


def _sec(h: Any, n: int, resource: str) -> Any:
    """The reader's answer for one collection of group ``n``; None where the
    put was not read, holds no such group, or the group's sections were not
    read. A broken bus then gives FAIL rows, never a traceback (review-bot
    on #1141)."""
    groups = _groups(h)
    return (groups[n].get("sections") or {}).get(resource) if n < len(groups) else None


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
                  "resource unavailable, the cause config, \"the owner was given no catalog\" (§3, text 0.10; "
                  "core §3.3)",
                  entry is not None and entry.get("unavailable") == [{"resource": "stream/heartbeats/{class}",
                                                                     "cause": "config"}], json.dumps(entry))
        finally:
            s2.close()
    finally:
        sub.undeclare()
        o.close()


def reference(session: Any, endpoint: str) -> None:
    """The reference's owner example, a black box, on the same bus: which
    revision of the standard contract it implements, and the cause it lists.
    A note here; :mod:`zk2py.freshness_v2_s8_live` ``--only cause`` checks
    it against §3 (text 0.10, F-549's answer)."""
    from .blackbox import BLACKBOX
    from .live_interop import Owner as Exe

    try:
        p = Exe(BLACKBOX / "owner", "lab/ref", [FILES[1]], connect=endpoint)
    except Exception as e:  # noqa: BLE001 - no black box, no note
        print(f"NOTE the reference's owner was not run: {type(e).__name__}: {e}", flush=True)
        return
    try:
        if p.wait_for("ready ", 120) is None:
            print("NOTE the reference's owner did not start", flush=True)
            return
        entry = None
        for _ in range(10):  # its instance token may trail its ready line
            entry = _entry(session, "lab/ref")
            if entry is not None:
                break
            time.sleep(0.3)
        std = fv2.standard_contract()
        which = "no entry read" if entry is None else "revision 1.1" if entry.get("contract") == std.fingerprint \
            else "another revision than 1.1"
        print(f"NOTE the reference's owner, sending heartbeats alone, implements freshness.v2 at {which}: "
              f"{json.dumps(entry)}", flush=True)
    finally:
        p.close()


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
            # E knows no catalog at its first owner heartbeat (text 0.11 §2.5, "Before it knows"): it reads no
            # descriptor until one arrives.
            e = fv2.Provider(sessions[1], ADDRESS, revs, held=[held], reread=False, wait_s=0.0, read_at_start=False)
            providers += [s, n, e]
            check("step 2: S subscribes to one owner heartbeat key per class its collections have (§2.5 \"Both "
                  "forms\")", sorted(s.keys) == ["host", "link"],
                  json.dumps(s.keys))
            time.sleep(5.0)
            link, host = s.answers("link"), s.answers("host")
            read_link = [h for h in link if "read" in h.answer]
            check("step 2: three interfaces in one owner heartbeat: every link put read, a group for a, b and c in "
                  "ascending index (§2.4 \"The groups\")",
                  len(read_link) >= 4 and all([g.get("iface") for g in _groups(h)] == [A, B, C]
                                              for h in read_link),
                  f"{len(link)} heard, {len(read_link)} read")
            check("step 2: one host owner heartbeat holds a's group alone, b and c having no host collection",
                  len(host) >= 2 and all([g.get("iface") for g in h.answer.get("read", {}).get("groups", [])] == [A]
                                         for h in host), f"{len(host)} heard")
            # freshness.v2 0.11 §2.5, "Before it knows" and "Where the id is" (F-557's answer, round 117)
            e_link = [h for h in e.answers("link") if "read" in h.answer]
            first_known = next((i for i, h in enumerate(e_link) if any("refused" not in g for g in _groups(h))), None)
            runs = sorted({h.stamp.run_hex() for h in e_link if h.stamp is not None})
            check("before it knows (text 0.11 §2.5): E, which read no descriptor before its first owner heartbeat, "
                  "refuses: each group of the puts heard before the id is unknown_catalog, each put still a read "
                  "one in sequence; it reads its owner's descriptors once for the run, the one that states the "
                  "put's run, learns the catalog's id there, and from then on reads every group, with no gap "
                  "between the last put refused and the first read",
                  first_known is not None and 1 <= first_known <= 3
                  and all(g.get("refused") == "unknown_catalog" for h in e_link[:first_known] for g in _groups(h))
                  and all("refused" not in g for h in e_link[first_known:] for g in _groups(h))
                  and not any(_gap(h) for h in e_link) and e.naming.reads == 1
                  and len(runs) == 1 and e.naming.known.get(runs[0]) == held.id,
                  f"{len(e_link)} read, the first {first_known} unknown_catalog; {e.naming.reads} descriptor read; "
                  f"run {runs} names {e.naming.known}")
            check("N, which holds no catalog, read its owner's descriptors at its start, knows the id its run "
                  "names, and asks at no put (\"asks again later, and not at every put\"), where through round "
                  "116 zk2py's reader that knew no id read again after each owner heartbeat",
                  n.naming.reads == 0 and n.naming.known.get(runs[0] if runs else "") == held.id,
                  f"{n.naming.reads} read at a put, over {len(n.answers('link'))} puts")
            # "In a group, an owner MUST set the map bit of no collection of another class."
            on_link = [_sections(h.payload, held, layouts) for h in link]
            on_host = [_sections(h.payload, held, layouts) for h in host]
            check("step 2: a host collection never on the link key: no link put's bytes hold a section of "
                  "a's peer/{peer}, and S reads it other_class there (§2.4 \"Only its class\", §2.5)",
                  bool(on_link) and all("state/peer/{peer}" not in x.get(A, {}) and "state/status" in x.get(A, {})
                                        for x in on_link)
                  and all(_sec(h, 0, "state/peer/{peer}") == "other_class" for h in read_link),
                  json.dumps(sorted({r for x in on_link for r in x.get(A, {})})))
            check("step 2: no link collection on the host key: its bytes hold peer/{peer}'s section alone, and S "
                  "reads status other_class there",
                  bool(on_host) and all(sorted(x.get(A, {})) == ["state/peer/{peer}"] for x in on_host)
                  and all(_sec(h, 0, "state/status") == "other_class" for h in host),
                  json.dumps(sorted({r for x in on_host for r in x.get(A, {})})))
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
            b_answers = [_sec(h, 1, "state/config") for h in read_link]
            others = [(_sec(h, 0, "state/status"), _sec(h, 2, "state/alarms/{alarm}")) for h in read_link]
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
                  bool(seqs) and seqs == list(range(seqs[0], seqs[0] + len(seqs))) and not any(_gap(h)
                                                                                  for h in read_link),
                  f"{seqs[0]} to {seqs[-1]}" if seqs else "no link put heard")
            # N holds no catalog.
            n_link = n.answers("link")
            check("step 4: N holds no catalog, and reads the id its owner names in its descriptor: every group's "
                  "index is refused unknown_catalog, the owner heartbeat still a read one, and nothing is "
                  "confirmed (§2.5 \"No index\")",
                  bool(n_link) and all("read" in h.answer and all(g.get("refused") == "unknown_catalog"
                                                                    for g in _groups(h))
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
                  len(after_gap) == 1 and bool(_groups(after_gap[0]))
                  and all(any(isinstance(x, dict) and x["confirmed"] for x in (g.get("sections") or {}).values())
                          for g in _groups(after_gap[0])))
            while time.monotonic() < end and len(s.answers("link")) < before + 2:
                time.sleep(0.02)
            nxt = s.answers("link")[before + 1:before + 2]
            obs2 = {k: s.subs[k.split("/")[3]].observation(k).complete for k in [STATUS, CONFIG, *ALARMS]}
            check("step 3: until an owner heartbeat arrives in sequence on that key: the next one does, and "
                  "nothing reads lost", len(nxt) == 1 and _gap(nxt[0]) is False
                  and s.reader.lost("link") == [] and all(obs2.values()), json.dumps(obs2))
            check("step 3: the host key heard no gap", not any(_gap(h) for h in s.answers("host"))
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
            reference(sessions[1], endpoint)
            sizes = sorted({len(h.payload) for h in s.answers("link")})

            def sized(h: Any) -> bool:
                try:
                    groups = owner_heartbeat.read(h.payload).body.groups
                except owner_heartbeat.Refused:
                    return False
                return h.stamp is not None and len(h.payload) == owner_heartbeat.size_of(
                    h.stamp.width, [len(g.bits) for g in groups])

            check("the link put's size is core §7.5's: 6 bytes over its body under a 16-bit run, or 8",
                  bool(sizes) and all(sized(h) for h in s.answers("link")), f"{sizes} bytes")
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


# -- the reference's readers beside zk2py's owner (round 113, #1110's H2c) ----------------

class Watch:
    """``zenctl watch``, a black box: the reference's consumer on one
    resource, its ndjson rows and its stderr."""

    def __init__(self, ep: str, iface: str, resource: str, catalog: str | None, for_s: float,
                 address: str = ADDRESS, extra: tuple[str, ...] = ()):
        from .blackbox import BLACKBOX

        argv = [str(BLACKBOX / "zenctl"), "watch", address, iface, resource, "--for", f"{for_s:g}", "--format",
                "ndjson", "-c", ep, *[a for f in FILES for a in ("--contracts", str(f))],
                *(["--catalog", catalog] if catalog else []), *extra]
        self.p = subprocess.Popen(argv, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        self.rows: list[dict[str, Any]] = []
        self.err: list[str] = []
        self._t = threading.Thread(target=self._pump, daemon=True)
        self._t.start()
        threading.Thread(target=lambda: self.err.extend(self.p.stderr), daemon=True).start()

    def _pump(self) -> None:
        for line in self.p.stdout:
            try:
                self.rows.append(json.loads(line))
            except ValueError:
                pass

    def wait(self, timeout: float = 40.0) -> None:
        try:
            self.p.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            self.p.kill()  # the process this runner started, by its own handle
            self.p.wait()
        self._t.join(timeout=2.0)

    def confirmations(self, key_class: str = "link") -> dict[int, dict[str, Any]]:
        """Its confirmation rows on one class key, by the heartbeat's seq."""
        return {r["heartbeat"]["seq"]: r for r in list(self.rows)
                if r.get("row") == "confirmation" and r.get("key", "").endswith(f"/heartbeats/{key_class}")}

    def summary(self) -> dict[str, Any]:
        return next((r for r in self.rows if r.get("row") == "summary"), {})


def _zecho(ep: str, extra: list[str], count: int, timeout: float = 20.0) -> tuple[list[dict[str, Any]], str]:
    """``zenctl echo``, a black box: its ndjson rows and its stderr."""
    from .blackbox import BLACKBOX

    argv = [str(BLACKBOX / "zenctl"), "echo", *extra, "--count", str(count), "--format", "ndjson", "-c", ep,
            *[a for f in FILES for a in ("--contracts", str(f))]]
    p = subprocess.Popen(argv, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    try:
        out, err = p.communicate(timeout=timeout)
    except subprocess.TimeoutExpired:
        p.kill()  # the process this runner started, by its own handle
        out, err = p.communicate()
    rows = []
    for line in out.splitlines():
        try:
            rows.append(json.loads(line))
        except ValueError:
            pass
    return rows, err


def _pyecho(ep: str, extra: list[str], window: float) -> list[dict[str, Any]]:
    """zk2py's echo tool, a process of its own: its rows."""
    import os

    env = dict(os.environ, PYTHONPATH=str(REPO / "impl" / "python"))
    p = subprocess.run([sys.executable, "-m", "zk2py.echo_tool", "-c", ep, "--for", f"{window:g}",
                        *[a for f in FILES for a in ("--contracts", str(f))], *extra],
                       capture_output=True, text=True, timeout=60, env=env)
    try:
        return json.loads(p.stdout)["samples"]
    except (ValueError, KeyError):
        return []


def run_reference() -> int:
    """zk2py's owner under a catalog, and the reference's readers: ``zenctl
    watch`` (the runtime's consumer) and ``zenctl echo``, each with the
    catalog and without, zk2py's Provider and echo tool beside them on the
    same puts.

    1. The owner puts its six members. Every reader starts after, a late
       joiner. Five seconds. Then, from the runner's own session, two puts
       on the ``link`` class key that are no owner heartbeat (core 0.109
       §7.5, "Another form on a heartbeat's key"; round 114): a mutation
       whose record is an owner heartbeat's body, and an owner heartbeat's
       own bytes under an Encoding that carries no envelope, each stamped
       as the owner's next.
    2. The owner makes a put of b's config that no reader hears. 3.5 s.
    3. The owner loses one ``link`` owner heartbeat. Three seconds."""
    from concurrent.futures import ThreadPoolExecutor

    from . import across, live
    from .blackbox import workdir
    from .live_interop import _r1

    rows: list[tuple[str, bool, str]] = []

    def check(name: str, ok: bool, detail: str = "") -> bool:
        rows.append((name, bool(ok), detail))
        print(f"{'PASS' if ok else 'FAIL'} {name}" + (f": {detail}" if detail else ""), flush=True)
        return bool(ok)

    def note(text: str) -> None:
        print(f"NOTE {text}", flush=True)

    contracts = _contracts()
    held = cat.read(json.dumps(catalog_doc()))
    router, endpoint, _ = _r1()
    session = live.open_client(endpoint)
    child = None
    s = None
    watches: dict[str, Watch] = {}
    try:
        with workdir("zk2py-ohb-ref-") as tmp:
            path = str(Path(tmp) / "catalog.json")
            Path(path).write_text(json.dumps(catalog_doc()))
            child = Child(endpoint, path)
            if not check("the owner process starts", child.wait_for("ready ", 60) is not None,
                         "\n".join(child.err[-5:])):
                return 1
            for k in [STATUS, CONFIG, *PEERS, *ALARMS]:
                child.command(f"put {k} v1")
            # Step 1: every reader a late joiner.
            for_s = 17.0
            on_key = Watch(endpoint, "freshness.v2", "heartbeats/{class}", path, 4.0)
            watches = {"a": Watch(endpoint, A, "status", path, for_s), "b": Watch(endpoint, B, "config", path, for_s),
                       "c": Watch(endpoint, C, "alarms/{alarm}", path, for_s),
                       "peer": Watch(endpoint, A, "peer/{peer}", path, for_s),
                       "none": Watch(endpoint, A, "status", None, for_s),
                       "none_host": Watch(endpoint, A, "peer/{peer}", None, for_s)}
            s = fv2.Provider(session, ADDRESS, [(c.canonical, c.fingerprint, None) for c in contracts], held=[held],
                             wait_s=0.0)
            sel = f"zk2/{ADDRESS}/freshness.v2/stream/heartbeats/*"
            pool = ThreadPoolExecutor(max_workers=6)
            echo_cat = pool.submit(_zecho, endpoint, [sel, "--catalog", path], 3)
            echo_none = pool.submit(_zecho, endpoint, [sel], 3)
            echo_face_cat = pool.submit(_zecho, endpoint, ["--across-face", ADDRESS, "--catalog", path], 2, 6.0)
            echo_face_none = pool.submit(_zecho, endpoint, ["--across-face", ADDRESS], 2, 6.0)
            py_cat = pool.submit(_pyecho, endpoint, ["--catalog", path, sel], 4.0)
            py_none = pool.submit(_pyecho, endpoint, [sel], 4.0)
            time.sleep(5.0)
            settled = max((h.stamp.seq for h in s.answers("link") if h.stamp is not None), default=-1)
            # Step 1b (core 0.109 §7.5, "Another form on a heartbeat's key";
            # round 114): two puts on the link class key that are no owner
            # heartbeat, from this runner's own session, each stamped as the
            # owner's next would be. Its body is a valid one of one group,
            # a's, that marks no collection. A reader that took either would
            # read it at that seq, and then find the owner's own put "older".
            a_layout = heartbeat.layout_of(contracts[0].canonical, contracts[0].fingerprint)
            quiet = owner_heartbeat.write_body(None, [owner_heartbeat.group_of(0, a_layout, {})])
            link_key = f"zk2/{ADDRESS}/freshness.v2/stream/heartbeats/link"

            def after_next() -> Any:
                """The next link owner heartbeat this reader reads, just heard."""
                n = len([h for h in s.answers("link") if "read" in h.answer])
                end = time.monotonic() + 5.0
                while time.monotonic() < end:
                    hs = [h for h in s.answers("link") if "read" in h.answer]
                    if len(hs) > n:
                        return hs[-1]
                    time.sleep(0.01)
                return None

            injected: dict[str, tuple[int, bytes] | None] = {"mutation": None, "bare": None}
            h1 = after_next()
            if h1 is not None and h1.stamp is not None:
                st1 = zk2c.InBand(h1.stamp.width, h1.stamp.run, h1.stamp.seq + 1)
                injected["mutation"] = (st1.seq, zk2c.mutation(st1, quiet))
                session.put(link_key, injected["mutation"][1])
            h2 = after_next()
            if h2 is not None and h2.stamp is not None:
                st2 = zk2c.InBand(h2.stamp.width, h2.stamp.run, h2.stamp.seq + 1)
                injected["bare"] = (st2.seq, zk2c.write("heartbeat", st2.width, st2.run, st2.seq, 0, quiet))
                # form 6's own bytes, under an Encoding that carries no envelope
                session.put(link_key, injected["bare"][1], encoding="application/octet-stream")
            after_next()
            # Step 2: a put no reader hears.
            child.command(f"miss {CONFIG} v3")
            time.sleep(3.5)
            # Step 3: one lost.
            child.command("lose link")
            time.sleep(3.0)
            for w in watches.values():
                w.wait()
            got = child.command("report", "report ")
            rep = json.loads(got.split(" ", 1)[1]) if got else {}
            lost_seq = next((x[1] for x in rep.get("lost", []) if x[0] == "link"), None)

            # -- zenctl watch, given the catalog ---------------------------------------------
            za, zb, zc = (watches[n].confirmations() for n in "abc")
            mine = {h.stamp.seq: h.answer for h in s.answers("link") if h.stamp is not None and "read" in h.answer}

            def py(seq: int, n: int, resource: str) -> Any:
                groups = mine.get(seq, {}).get("read", {}).get("groups", [])
                if n >= len(groups):
                    return None  # no such group read: the check fails, and the run goes on
                g = groups[n]
                return (g.get("sections") or {}).get(resource, g.get("answer") or g.get("refused"))

            res = {"a": (0, "state/status", [STATUS]), "b": (1, "state/config", [CONFIG]),
                   "c": (2, "state/alarms/{alarm}", ALARMS)}
            one = [q for q in sorted(za) if q in zb and q in zc and q <= settled
                   and all(z[q]["confirmed"] == res[n][2] for n, z in (("a", za), ("b", zb), ("c", zc)))]
            check("the reference's consumer reads zk2py's owner heartbeat: three interfaces confirmed by one put, "
                  "zenctl watch on a's status, b's config and c's alarms each printing a confirmation row for the "
                  "same seq on …/heartbeats/link, by its group 0, 1 and 2, the members zk2py's reader confirms by it",
                  bool(one) and [za[one[0]]["group"], zb[one[0]]["group"], zc[one[0]]["group"]] == [0, 1, 2]
                  and one[0] in mine and all(py(one[0], *res[n][:2]) == {"confirmed": res[n][2]} for n in "abc"),
                  f"seqs {one[:6]} of the run")
            zp = watches["peer"].confirmations("host")
            check("a's host collection is confirmed to the reference's consumer on …/heartbeats/host alone, and no "
                  "row of the three link watchers names the host key",
                  bool(zp) and any(r["confirmed"] == PEERS for r in zp.values())
                  and not any(watches[n].confirmations("host") for n in "abc")
                  and not watches["peer"].confirmations("link"), f"{len(zp)} rows")
            # every heartbeat both sides read, from the settled one on: the same word
            differ = []
            for n, z in (("a", za), ("b", zb), ("c", zc)):
                for q in sorted(z):
                    if q < settled or q not in mine:
                        continue
                    theirs = ("diverged" if z[q]["diverged"] else {"confirmed": z[q]["confirmed"]}, z[q]["lost"])
                    ours = (py(q, *res[n][:2]), mine[q].get("read", {}).get("gap"))
                    if theirs != ours:
                        differ.append(f"seq {q}, {n}: zenctl {json.dumps(theirs)}, zk2py {json.dumps(ours)}")
            compared = sum(1 for z in (za, zb, zc) for q in z if q >= settled and q in mine)
            check("from the settled heartbeat on, each group both sides read draws the same word from both: "
                  "confirmed with the same members or diverged, and lost or not", compared >= 15 and not differ,
                  f"{compared} group readings compared; " + ("; ".join(differ[:4]) or "none differs"))
            # a divergence in one group
            div = [q for q in sorted(zb) if q > settled and zb[q]["diverged"]]
            check("a divergence in one group: after a put of b's config no reader heard, zenctl watch reads b's "
                  "group diverged twice while a's and c's groups of the same two puts confirm; then it confirms "
                  "config again, having read it again as §2.5's SHOULD says; zk2py's reader the same, seq for seq",
                  len(div) == 2 and all(za.get(q, {}).get("confirmed") == [STATUS]
                                        and zc.get(q, {}).get("confirmed") == ALARMS for q in div)
                  and any(q > div[-1] and zb[q]["confirmed"] == [CONFIG] for q in zb)
                  and [q for q in sorted(mine) if q > settled and py(q, 1, "state/config") == "diverged"] == div,
                  f"diverged at {div}")
            # one lost
            lost_rows = {n: [q for q, r in z.items() if r["lost"]] for n, z in (("a", za), ("b", zb), ("c", zc))}
            my_gaps = [q for q, a in mine.items() if a["read"]["gap"]]
            check("one lost: the owner took a seq and put nothing; each of the three watchers marks the next row "
                  "lost, that seq plus 1, still confirming its members, and the one after not lost; zk2py's reader "
                  "reads the same put after a gap; the host watcher marks none; each summary counts one lost",
                  lost_seq is not None and all(v == [lost_seq + 1] for v in lost_rows.values())
                  and my_gaps == [lost_seq + 1] and all(lost_seq + 2 in z for z in (za, zb, zc))
                  and not any(r["lost"] for r in zp.values())
                  and [watches[n].summary().get("heartbeats_lost") for n in ("a", "b", "c", "peer")] == [1, 1, 1, 0]
                  and za.get(lost_seq + 1, {}).get("confirmed") == [STATUS],
                  f"the owner lost seq {lost_seq}; zenctl lost at {lost_rows}; zk2py's gap at {my_gaps}")
            # -- another form on the owner heartbeat's key (core 0.109) ----------------------
            heard = {h.payload: h.answer for h in s.answers("link")}
            for what, words in (("mutation", "a mutation whose record is an owner heartbeat's body, stamped as the "
                                             "owner's next"),
                                ("bare", "an owner heartbeat's own bytes under an Encoding that carries no envelope "
                                         "(\"A put with no envelope on either key is refused the same way\")")):
                inj = injected[what]
                if inj is None:
                    check(f"another form on the owner heartbeat's key, {what}: the put was made", False, "no owner "
                          "heartbeat was heard to stamp it after")
                    continue
                q, payload = inj
                rows_at = {n: [r for r in watches[n].rows if r.get("row") == "confirmation"
                               and r.get("key", "").endswith("/heartbeats/link") and r["heartbeat"]["seq"] == q]
                           for n in "abc"}
                mine_q = mine.get(q, {}).get("read", {})
                check(f"another form on the owner heartbeat's key: {words}. zk2py's reader refuses it `form`, and "
                      "reads the owner's own put of that seq in sequence, no gap and not older; each of the three "
                      "zenctl watchers prints one confirmation row at that seq, confirming its members by the "
                      "owner's put, not lost: neither side took the put as an owner heartbeat (core §7.5)",
                      heard.get(payload) == {"refused": "form"} and mine_q.get("gap") is False
                      and all(len(rows_at[n]) == 1 and rows_at[n][0]["confirmed"] == res[n][2]
                              and not rows_at[n][0]["lost"] for n in "abc"),
                      f"seq {q}: zk2py {heard.get(payload)}, then gap {mine_q.get('gap')}; zenctl rows "
                      + json.dumps({n: [(r['confirmed'], r['lost'], r.get('diverged')) for r in rows_at[n]]
                                    for n in 'abc'}))
            # -- a reader given no catalog (freshness.v2 0.11 §2.5 and §5, observer 0.35; round 117) --
            wn, wh = watches["none"], watches["none_host"]
            kinds = sorted({r.get("row", "?") for r in wn.rows})
            line = wn.err[0].strip() if wn.err else ""
            check("a reader given no catalog (freshness.v2 0.11 §2.5 \"A subscriber that holds no catalog … need "
                  "not subscribe to one\"): zenctl watch on a's status prints no confirmation row and counts no "
                  "heartbeat in its summary; zk2py's reader given none reads each group unknown_catalog "
                  "(owner_heartbeat_live's N). Neither confirms a member",
                  "confirmation" not in kinds and wn.summary().get("heartbeats") == 0,
                  f"rows {kinds}; summary heartbeats {wn.summary().get('heartbeats')}, lost "
                  f"{wn.summary().get('heartbeats_lost')}; exit {wn.p.returncode}")
            check("and it says so, where through round 116 it said nothing (F-556): one line on stderr, that no "
                  "heartbeat was read and no catalog given, that the `link` class's owner heartbeat key was not "
                  "subscribed, that such an owner is heard as one that confirms nothing, and that it is no finding "
                  "about the owner: §5's \"A tool that subscribed to no owner heartbeat key hears none. It cannot "
                  "tell an owner that sends them from one that confirms nothing\", and \"no finding about the "
                  "owner\". It names no catalog's id, which §5 lets a tool name and does not ask",
                  len(wn.err) == 1 and "no catalog given" in line and "`link` class" in line
                  and "not subscribed" in line and "confirms nothing" in line and "no finding about the owner" in line
                  and held.id not in line, f"stderr {wn.err[:3]}")
            line_h = wh.err[0].strip() if wh.err else ""
            check("the same reader on a's peer/{peer}, a host collection: the line names the `host` class's key, "
                  "the one that collection's class has (§2.5 \"one for each class that the collections it follows "
                  "have\")",
                  len(wh.err) == 1 and "`host` class" in line_h and "`link` class" not in line_h
                  and not wh.confirmations("host") and not wh.confirmations("link"), f"stderr {wh.err[:3]}")
            # the same verb, no catalog, beside an owner that sends heartbeats alone: a heartbeat is read, no line
            from .owner import Owner

            lone = Owner("lab", "alone", [contracts[1]], connect=endpoint)
            lone.start()
            try:
                lone.set_state(f"zk2/lab/alone/{B}/state/config", b"v1")
                wl = Watch(endpoint, B, "config", None, 4.0, address="lab/alone")
                wl.wait()
            finally:
                lone.close()
            beats = [r for r in wl.rows if r.get("row") == "confirmation"]
            check("beside an owner that sends heartbeats alone, the same verb with no catalog reads the interface's "
                  "heartbeat, prints its confirmation rows and no such line: the line is drawn only where no "
                  "heartbeat of the collection was read",
                  len(beats) >= 2 and not wl.err and wl.summary().get("heartbeats", 0) >= 2,
                  f"{len(beats)} confirmation rows, summary heartbeats {wl.summary().get('heartbeats')}, stderr "
                  f"{wl.err[:2]}")
            note("zenctl watch with no catalog now tells its operator that an owner's confirmations may be out of "
                 "its reach (F-556, resolved); it reads no descriptor's `catalog`, and by hand it says the same on an "
                 "address no owner serves: silence, and no verdict (§5)")
            on_key.wait()
            shown = sorted({(r.get("payload", {}).get("as"), r.get("payload", {}).get("declared"),
                            "groups" in json.dumps(r)) for r in on_key.rows if r.get("row") == "sample"})
            note("zenctl watch on the owner heartbeat's own resource, freshness.v2 heartbeats/{class}, given the "
                 f"catalog, shows each put as (as, declared, any group shown): {shown}; observer §7.3 has a tool "
                 "show its groups, as zenctl echo does (the reference, observed)")
            # -- zenctl echo -----------------------------------------------------------------
            zrows, _ = echo_cat.result()
            prow = {(r["key"], (r.get("envelope") or {}).get("seq")): r for r in py_cat.result()
                    if r.get("heartbeat")}
            both = [(z, prow[(z["key"], z["value"]["seq"])]) for z in zrows
                    if (z["key"], z.get("value", {}).get("seq")) in prow]
            same = [all(pg["index"] == zg["index"] and pg["tag"] == zg["tag"] and pg["bits"] == zg["size"]
                        and pg.get("iface") == zg.get("covers") and pg["vouches"] == zg.get("sections")
                        for pg, zg in zip(p_["heartbeat"]["groups"], z["value"]["groups"]))
                    and len(p_["heartbeat"]["groups"]) == len(z["value"]["groups"]) for z, p_ in both]
            check("zenctl echo given the catalog shows each owner heartbeat's groups, each with the interface its "
                  "index names and its sections; zk2py's echo tool shows the same of the same puts: index, tag, "
                  "size, interface and what each group vouches for (observer §7.3, §7.5)",
                  len(zrows) == 3 and bool(both) and all(same),
                  f"{len(zrows)} rows of zenctl's, {len(both)} puts both showed")
            zrows, _ = echo_none.result()
            prow = {(r["key"], (r.get("envelope") or {}).get("seq")): r for r in py_none.result() if r.get("heartbeat")}
            both = [(z, prow[(z["key"], z["value"]["seq"])]) for z in zrows
                    if (z["key"], z.get("value", {}).get("seq")) in prow]
            check("given no catalog, both show every group by its index, its tag and its size, and read none: "
                  "zenctl echo names each unread, unknown_catalog",
                  len(zrows) == 3 and bool(both)
                  and all([(g["index"], g["tag"], g["size"]) for g in z["value"]["groups"]]
                          == [(g["index"], g["tag"], g["bits"]) for g in p_["heartbeat"]["groups"]]
                          and all(g.get("unread") == "unknown_catalog" and "sections" not in g
                                  for g in z["value"]["groups"])
                          and all(g["vouches"] is None for g in p_["heartbeat"]["groups"]) for z, p_ in both),
                  json.dumps(zrows[0]["value"]["groups"])[:200] if zrows else "no row")
            # -- the scope across a face, no policy ------------------------------------------
            for fut, given, want_n in ((echo_face_cat, True, 9), (echo_face_none, False, 7)):
                _, err = fut.result()
                line = next((ln for ln in err.splitlines() if ln.startswith("echoing ")), "")
                theirs = sorted(x.strip() for x in line.removeprefix("echoing ").split(" — ")[0].split(" + "))
                ours = sorted(across.scopes(contracts, [ADDRESS], catalog_given=given).scopes)
                check(f"across a face with no policy, {'given the catalog' if given else 'given no catalog'}: "
                      f"zenctl echo declares {want_n} selectors, zk2py's scopes key for key, "
                      + ("the two owner heartbeat keys among them" if given else "no owner heartbeat key among "
                         "them") + " (observer 0.34 §7.5)",
                      theirs == ours and len(ours) == want_n
                      and (sum("/heartbeats/" in x for x in ours) == (2 if given else 0)), f"{len(theirs)} and "
                      f"{len(ours)}; only zenctl's {sorted(set(theirs) - set(ours))[:3]}, only zk2py's "
                      f"{sorted(set(ours) - set(theirs))[:3]}")
            pool.shutdown(wait=False)
    finally:
        if s is not None:
            s.close()
        for w in watches.values():
            if w.p.poll() is None:
                w.p.kill()  # the process this runner started, by its own handle
        if child is not None:
            child.close()
        session.close()
        router.close()
    failed = [r for r in rows if not r[1]]
    print(f"owner_heartbeat_live --reference: {len(rows) - len(failed)}/{len(rows)}", flush=True)
    return 1 if failed else 0


def run_silent(slow_ms: int | None = None, window: float = 9.0) -> int:
    """freshness.v2 §2.5, "The catalog its owner names", beside an owner
    whose descriptor does not answer (round 117), or answers ``slow_ms``
    after each GET (round 125): zk2py's owner under a catalog, sending
    owner heartbeats, its descriptor queryable undeclared or slowed (a test
    rig's). On the near side, with no face:

    - zk2py's subscriber that holds the catalog and was given no word as
      such: 0.13 "A read that draws no reply is no failure. The
      deployment's word stands for it", "a tokenless owner's, or a slow
      one's. So it is read as one." Round 117's subscriber read each group
      ``unknown_catalog``; round 125's reads them through the one catalog
      it holds, the one it was provisioned;
    - zk2py's subscriber given the deployment's word reads every group;
    - the reference's consumer, ``zenctl watch --catalog --timeout 1`` (a
      black box): what it prints.

    With ``slow_ms``, a GET with a 1 s timeout draws what zenoh gives a GET
    left unanswered at its end (deliverable 3): the rig's own GET says it."""
    from . import live
    from .blackbox import BLACKBOX, workdir
    from .live_interop import _r1

    name = "--silent" if slow_ms is None else f"--slow {slow_ms}"
    rows: list[tuple[str, bool, str]] = []

    def check(name: str, ok: bool, detail: str = "") -> bool:
        rows.append((name, bool(ok), detail))
        print(f"{'PASS' if ok else 'FAIL'} {name}" + (f": {detail}" if detail else ""), flush=True)
        return bool(ok)

    contracts = _contracts()
    held = cat.read(json.dumps(catalog_doc()))
    router, endpoint, _ = _r1()
    sessions = [live.open_client(endpoint) for _ in range(2)]
    child = None
    providers: list[Any] = []
    try:
        with workdir("zk2py-ohb-silent-") as tmp:
            path = str(Path(tmp) / "catalog.json")
            Path(path).write_text(json.dumps(catalog_doc()))
            child = Child(endpoint, path)
            if not check("the owner process starts", child.wait_for("ready ", 60) is not None,
                         "\n".join(child.err[-5:])):
                return 1
            for k in [STATUS, CONFIG, *PEERS, *ALARMS]:
                child.command(f"put {k} v1")
            child.command("mute" if slow_ms is None else f"slow {slow_ms}")
            time.sleep(0.3)
            t0 = time.monotonic()
            answered, errors = fv2.descriptor_read(sessions[0], ADDRESS, 1.0)
            took = time.monotonic() - t0
            if slow_ms is None:
                check("the rig: the owner's descriptor answers no GET, and its owner heartbeats go on: a GET draws "
                      "no reply at all", answered == [] and errors == [], f"{len(answered)} descriptors, errors "
                      f"{errors}, {took:.2f} s")
            else:
                check(f"the rig: the owner's descriptor answers {slow_ms} ms after each GET; a GET with a 1 s "
                      "timeout draws no descriptor, and zenoh's error reply `Timeout` at its end (deliverable 3)",
                      answered == [] and errors and all(e == fv2.TIMEOUT_PAYLOAD for e in errors),
                      f"{len(answered)} descriptors, errors {errors}, {took:.2f} s")
            revs = [(c.canonical, c.fingerprint, None) for c in contracts]
            again = 2.0
            p0 = fv2.Provider(sessions[0], ADDRESS, revs, held=[held], seed=[A, B, C], wait_s=0.0, reread=False,
                              get_timeout=0.5 if slow_ms is None else 1.0, ask_again_s=again)
            pw = fv2.Provider(sessions[1], ADDRESS, revs, held=[held], named=held.id, seed=[A, B, C], wait_s=0.0,
                              reread=False)
            ready_ns = time.monotonic_ns()  # p0's start read has ended: puts heard during it were refused
            providers += [p0, pw]
            w = Watch(endpoint, A, "status", path, window, extra=("--timeout", "1")) \
                if (BLACKBOX / "zenctl").is_file() else None
            time.sleep(window)
            early = [h for h in p0.answers("link") if "read" in h.answer and h.arrival_ns < ready_ns]
            h0 = [h for h in p0.answers("link") if "read" in h.answer and h.arrival_ns >= ready_ns]
            if early:
                print(f"NOTE {len(early)} owner heartbeats heard while zk2py's start read was out, each group "
                      f"{sorted({g.get('refused', 'read') for h in early for g in _groups(h)})}: \"Before it "
                      f"knows\", it refuses", flush=True)
            if slow_ms is None:
                check("zk2py's subscriber, holding the one catalog and given no word as such (0.13 §2.5: \"a read "
                      "that draws no reply cannot be told from a face: a tokenless owner's. So it is read as one\"): "
                      "every owner heartbeat after its start read a read one, every group read through the catalog "
                      "it holds, the members confirmed",
                      len(h0) >= 6 and all("refused" not in g for h in h0 for g in _groups(h))
                      and any(sub.last_conf for sub in p0.subs.values()),
                      f"{len(h0)} read, {p0.naming.silences} silent reads, {p0.naming.errors} failed")
                check("and the word stands for each run it read silent: one read at the start, one at the first put "
                      "of the run, and none after", p0.naming.reads <= 1 and p0.naming.started == 1,
                      f"{p0.naming.reads} reads for a run, {p0.naming.started} at the start, over {len(h0)} puts")
            else:
                # the mend of round 125 (F-616, answered): "A read that ends in
                # zenoh's `Timeout` error reply, its owner answering too late,
                # is an error reply, and fails. It waits for the next read,
                # never for a guess."
                check("zk2py's subscriber beside an owner whose descriptor answers too late (0.13 §2.5, as the mend "
                      "words it: a `Timeout` error reply fails): every owner heartbeat a read one, every group "
                      "unknown_catalog, nothing confirmed by one",
                      len(h0) >= 6 and all(g.get("refused") == "unknown_catalog" for h in h0 for g in _groups(h))
                      and not any(sub.last_conf for sub in p0.subs.values()) and p0.naming.errors >= 1,
                      f"{len(h0)} read, {p0.naming.silences} silent reads, {p0.naming.errors} failed")
                check(f"and it asks again later, not at every put: one read each {again:g} s after a failure at most",
                      1 <= p0.naming.reads <= int(window / again) + 2 and p0.naming.reads < len(h0),
                      f"{p0.naming.reads} reads for a run over {len(h0)} puts in {window:g} s")

            hw = [h for h in pw.answers("link") if "read" in h.answer]
            check("zk2py's subscriber given the deployment's word as across a face reads every group through it, "
                  "and reads no descriptor",
                  len(hw) >= 6 and all("refused" not in g for h in hw for g in _groups(h))
                  and any(sub.last_conf for sub in pw.subs.values()) and pw.naming.reads == 0, f"{len(hw)} read")
            if w is not None:
                w.wait(window + 40)
                dump = Path(os.environ.get("ZK2PY_DUMP", "")) if os.environ.get("ZK2PY_DUMP") else None
                if dump is not None:
                    dump.write_text("\n".join(json.dumps(r) for r in w.rows) + "\n" + "".join(w.err))
                conf = w.confirmations("link")
                confirmed = [q for q, r in sorted(conf.items()) if r.get("confirmed") == [STATUS]]
                print(f"NOTE the reference's consumer (zenctl watch --catalog --timeout 1, on the near side, its "
                      f"descriptor read {'unanswered' if slow_ms is None else f'answered {slow_ms} ms late'}): "
                      f"{len(conf)} confirmation rows on the link key, {len(confirmed)} confirming status; summary "
                      f"heartbeats {w.summary().get('heartbeats')}; stderr {w.err[:2]}", flush=True)
                if slow_ms is None:
                    check("the reference's consumer beside the same owner reads its groups through the catalog it "
                          "was given: confirmation rows, as zk2py's subscriber now does", len(confirmed) >= 3,
                          f"{len(confirmed)} rows confirm status")
                elif slow_ms > 5000:
                    # past the reference's own wait of about 5 s (round 125): both reads fail
                    check(f"the reference's consumer beside an owner {slow_ms} ms late reads no group either: no "
                          "confirmation row, no heartbeat counted, as zk2py's subscriber reads each unknown_catalog "
                          "(F-616 answered: the two agree)",
                          not confirmed and not w.summary().get("heartbeats"),
                          f"{len(confirmed)} rows, {w.summary().get('heartbeats')} heartbeats")
                else:
                    # within the reference's wait: its read draws the descriptor, zk2py's at 1 s does not
                    print(f"NOTE beside an owner {slow_ms} ms late, within the reference's own wait of about 5 s, "
                          f"its consumer confirms status in {len(confirmed)} rows; zk2py's read, of 1 s, failed: "
                          "the two reads wait differently, and each follows §2.5", flush=True)
    finally:
        for p in providers:
            p.close()
        if child is not None:
            child.close()
        for x in sessions:
            x.close()
        router.close()
    failed = [r for r in rows if not r[1]]
    print(f"owner_heartbeat_live {name}: {len(rows) - len(failed)}/{len(rows)}", flush=True)
    return 1 if failed else 0


def run_forms() -> int:
    """Core 0.109 §7.5, "Another form on a heartbeat's key", on a heartbeat's
    own key, on a bus (round 115, the carry of round 114, whose step put on
    the ``link`` class key alone). For each owner, zk2py's and the
    reference's example (a black box), sending heartbeats alone for
    ``beacon.v2`` at ``lab/beacon``: beside its puts on
    ``…/freshness.v2/stream/heartbeat/beacon.v2``, from the runner's own
    session, a mutation whose record is a heartbeat's body, then a
    heartbeat's own bytes under an Encoding that carries no envelope, each
    stamped as the owner's next. The readers are zk2py's
    :class:`zk2py.freshness_v2.Subscriber` and ``zenctl watch`` on
    ``status``."""
    from . import freshness_v2_scenarios as sc, live
    from .blackbox import BLACKBOX
    from .live_interop import _r1

    rows: list[tuple[str, bool, str]] = []

    def check(name: str, ok: bool, detail: str = "") -> bool:
        rows.append((name, bool(ok), detail))
        print(f"{'PASS' if ok else 'FAIL'} {name}" + (f": {detail}" if detail else ""), flush=True)
        return bool(ok)

    c = sc._contract()
    layout = heartbeat.layout_of(c.canonical, c.fingerprint)
    key = heartbeat.heartbeat_key("lab", "beacon", sc.IFACE)
    impls = ["py"] + (["rust"] if (BLACKBOX / "owner").is_file() else [])
    for impl in impls:
        who = "the reference's owner" if impl == "rust" else "zk2py's owner"
        router, endpoint, _ = _r1()
        session = live.open_client(endpoint)
        o = s = w = None
        try:
            o = sc.make_owner(impl, endpoint)
            o.put("status", b"up")
            time.sleep(0.3)
            s = sc.subscriber(session, ["state/status"], seed=True)
            w = sc.ZWatch(endpoint, "lab/beacon", sc.IFACE, "state/status", 9.0) \
                if (BLACKBOX / "zenctl").is_file() else None
            time.sleep(2.5)
            quiet = heartbeat.write(layout, {"time": None, "sections": {}})

            def after_next() -> Any:
                n = len([h for h in s.answers() if "read" in h.answer])
                end = time.monotonic() + 5.0
                while time.monotonic() < end:
                    hs = [h for h in s.answers() if "read" in h.answer]
                    if len(hs) > n:
                        return hs[-1]
                    time.sleep(0.01)
                return None

            injected: dict[str, tuple[int, bytes] | None] = {"mutation": None, "bare": None}
            h1 = after_next()
            if h1 is not None and h1.stamp is not None:
                st = zk2c.InBand(h1.stamp.width, h1.stamp.run, h1.stamp.seq + 1)
                injected["mutation"] = (st.seq, zk2c.mutation(st, quiet))
                session.put(key, injected["mutation"][1])
            h2 = after_next()
            if h2 is not None and h2.stamp is not None:
                st = zk2c.InBand(h2.stamp.width, h2.stamp.run, h2.stamp.seq + 1)
                injected["bare"] = (st.seq, zk2c.heartbeat(st, quiet, layout.tag))
                session.put(key, injected["bare"][1], encoding="application/octet-stream")
            after_next()
            time.sleep(1.5)
            if w is not None:
                w.wait()
            answers = s.answers()
            by_seq = {h.stamp.seq: h.answer for h in answers if h.stamp is not None and "read" in h.answer}
            refused = [h.refusal for h in answers if h.refusal is not None]
            for what, words in (("mutation", "a mutation whose record is a heartbeat's body"),
                                ("bare", "a heartbeat's own bytes under application/octet-stream")):
                inj = injected[what]
                if inj is None:
                    check(f"{who}, another form on the heartbeat's own key, {what}: the put was made", False,
                          "no heartbeat was heard to stamp it after")
                    continue
                q = inj[0]
                mine = by_seq.get(q, {}).get("read", {})
                zrows = [] if w is None else [r for _, r in w.rows if r.get("row") == "confirmation"
                                              and (r.get("heartbeat") or {}).get("seq") == q]
                check(f"{who}, another form on the heartbeat's own key: {words}, stamped as the owner's next. "
                      "zk2py's reader refuses it `form` and reads the owner's own heartbeat of that seq in "
                      "sequence, confirming status" + ("" if w is None else "; zenctl watch prints one "
                                                       "confirmation row at that seq, by the owner's put, not lost")
                      + " (core §7.5)",
                      refused.count("form") == 2 and mine.get("gap") is False
                      and isinstance(mine.get("sections", {}).get("state/status"), dict)
                      and (w is None or (len(zrows) == 1 and zrows[0].get("confirmed") == [sc.STATUS]
                                         and not zrows[0].get("lost"))),
                      f"seq {q}: zk2py's refusals {refused}, then gap {mine.get('gap')}; zenctl rows at that seq "
                      + json.dumps([(r.get("confirmed"), r.get("lost")) for r in zrows]))
        finally:
            if s is not None:
                s.close()
            if w is not None and w.proc.poll() is None:
                w.proc.kill()  # the process this runner started, by its own handle
            if o is not None:
                o.close()
            session.close()
            router.close()
    failed = [r for r in rows if not r[1]]
    print(f"owner_heartbeat_live --forms: {len(rows) - len(failed)}/{len(rows)}", flush=True)
    return 1 if failed else 0


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="python -m zk2py.owner_heartbeat_live", description=__doc__.split("\n")[0])
    ap.add_argument("--forms", action="store_true",
                    help="another form on a heartbeat's own key, beside each owner's heartbeats (core §7.5)")
    ap.add_argument("--silent", action="store_true",
                    help="an owner whose descriptor does not answer: zk2py's subscribers and zenctl watch (a black box)")
    ap.add_argument("--slow", type=int, metavar="MS",
                    help="an owner whose descriptor answers MS after each GET: zk2py's subscribers and zenctl "
                         "watch beside it (round 125, deliverable 3)")
    ap.add_argument("--window", type=float, default=9.0, help="--silent and --slow: the readers' window, in s")
    ap.add_argument("--owner", nargs=2, metavar=("ENDPOINT", "CATALOG"), help="run the owner process")
    ap.add_argument("--reference", action="store_true",
                    help="zk2py's owner with the reference's readers, zenctl watch and zenctl echo (black boxes)")
    args = ap.parse_args(argv)
    if args.owner:
        return run_owner(*args.owner)
    if args.reference:
        return run_reference()
    if args.forms:
        return run_forms()
    if args.silent:
        return run_silent(None, args.window)
    if args.slow is not None:
        return run_silent(args.slow, args.window)
    return run()


if __name__ == "__main__":
    sys.exit(main())

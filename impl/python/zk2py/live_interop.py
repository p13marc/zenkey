"""The live interop runner: ``python -m zk2py.live_interop``.

Spawns the Rust owner example as a black box (its documented contract: a
zenoh router on an ephemeral loopback port; ``listening tcp/…`` then
``ready zk2/<system>/<service>/@zk/instance/<id>`` on stdout; it implements
every contract given, exposes every resource, holds every gated capability,
publishes no data, and runs until its stdin closes). zk2py connects as a
client and checks, from the spec alone:

- **presence** (§8.1, §8.2): acted on as seen on the bus, not on the
  ``ready`` line. One instance token, and one interface token per
  implemented interface whose ``<fp16>`` is the start of the fingerprint
  zk2py computes from the TOML;
- **the descriptor** (§3.3): GET at the instance key, valid for zk2py's
  checker against the contracts (no D code at all), naming exactly the
  expected interfaces with zk2py's fingerprints;
- **retrieval** (§8.4): every bundle retrieved by the procedure, verified
  against the fingerprint the descriptor names, and byte-identical to the
  bundle zk2py builds. Also: an unknown revision is reported unavailable
  after the retry, and a corrupt nearest holder is refused (scenarios
  retrieval.md §2, §3);
- **presence at scale** (presence.md §4): with a liveliness subscriber held,
  a callback GET lists every one of N extra tokens;
- **shutdown:** closing the owner's stdin makes it exit, with status 0.

Exit 0 when every check passes, 1 when any fails, 2 when it could not run.
"""

from __future__ import annotations

import argparse
import base64
import json
import os
import queue
import subprocess
import sys
import threading
import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

REPO = Path(__file__).resolve().parents[3]
DEFAULT_OWNER = REPO / "target" / "debug" / "examples" / "owner"
#: (service, contracts): one owner process each.
DEFAULT_RUNS = [
    ("vehicle-01/navigation", ["examples/zk2/walkthrough/nav.v2.toml"]),
    # Not thruster.v1 here: its required role has no binding, so the owner
    # must not start (§3.2); REFUSAL_RUNS checks exactly that.
    ("site-1/interop", ["examples/zk2/walkthrough/camera.v1.toml",
                        "examples/zk2/zensight/zs.snmp.v1.toml",
                        "impl/python/interop/zk2py_probe.v1.toml"]),
]
#: §8.1 (0.5): "how long a tool waits for presence after an owner starts"
#: is the caller's choice, and "the scenarios, and so a conformance run, use
#: 1 s". zk2py counts it from its client session's connection to the
#: owner's router: §8.1 (0.6) starts the wait at "the later of the tool's
#: session connecting and the owner's launch", the connection here.
PRESENCE_WAIT_S = 1.0
#: Owners that MUST NOT start: a required role their configuration binds to
#: nothing (§3.2, 0.5; presence.md §2 step 4). The owner example takes no
#: binding configuration, so every required role is unbound.
REFUSAL_RUNS = [
    ("vehicle-01/thrusters", ["examples/zk2/walkthrough/thruster.v1.toml"]),
]
#: The harness's own 2,000 tokens (presence at scale) are not an owner's;
#: their propagation gets a harness bound, not the conformance one.
SCALE_PROPAGATION_S = 10.0


class CannotRun(Exception):
    """The runner itself cannot proceed (exit 2)."""


@dataclass
class Report:
    rows: list[tuple[str, str, bool, str]] = field(default_factory=list)

    def check(self, run: str, name: str, ok: bool, detail: str = "") -> bool:
        self.rows.append((run, name, ok, detail))
        print(f"{'PASS' if ok else 'FAIL'} [{run}] {name}" + (f": {detail}" if detail else ""), flush=True)
        return ok

    def info(self, run: str, text: str) -> None:
        print(f"info [{run}] {text}", flush=True)


class Owner:
    """The owner example as a child process, read line by line."""

    def __init__(self, exe: Path, service: str, contracts: list[Path]):
        if not exe.is_file():
            raise CannotRun(f"{exe} not found: build it with "
                            "`cargo build -q -p zenkey --example owner`")
        self.proc = subprocess.Popen([str(exe), service, *map(str, contracts)],
                                     stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                     stderr=subprocess.PIPE, text=True)
        self.lines: queue.Queue[str] = queue.Queue()
        self.stderr: list[str] = []
        threading.Thread(target=self._pump, args=(self.proc.stdout, self.lines.put), daemon=True).start()
        threading.Thread(target=self._pump, args=(self.proc.stderr, self.stderr.append),
                         daemon=True).start()
        self.seen: list[str] = []

    @staticmethod
    def _pump(stream, sink) -> None:
        for line in stream:
            sink(line.rstrip("\n"))

    def wait_for(self, prefix: str, timeout: float) -> str | None:
        """The rest of the first stdout line starting with ``prefix``."""
        for line in self.seen:
            if line.startswith(prefix):
                return line[len(prefix):]
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            try:
                line = self.lines.get(timeout=0.2)
            except queue.Empty:
                if self.proc.poll() is not None:
                    return None
                continue
            self.seen.append(line)
            if line.startswith(prefix):
                return line[len(prefix):]
        return None

    def close(self, timeout: float = 15.0) -> int | None:
        """Close stdin and wait. None when the owner had to be killed."""
        try:
            self.proc.stdin.close()
        except OSError:
            pass
        try:
            return self.proc.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            self.proc.kill()
            self.proc.wait()
            return None


def _corrupt(bundle_bytes: bytes) -> bytes:
    """A bundle that fails verification at step 9 (``schema_hash``), or at
    step 12 for a contract with no artifact: what a corrupt holder sends."""
    from . import jcs

    doc = json.loads(bundle_bytes)
    if doc["schemas"]:
        sid = sorted(doc["schemas"])[0]
        entry = doc["schemas"][sid]
        if entry["kind"] == "protobuf":
            entry["data"] = base64.b64encode(b"corrupt").decode()
        else:
            entry["data"] = {**entry["data"], "$comment": "tampered"}
    else:
        doc["contract"]["interface"] = doc["contract"]["interface"] + "x"
    return jcs.dumps(doc)


def _describe(r) -> str:
    return "; ".join(f"{a.target}: " + (", ".join(f"{tag} ({enc})" for _, tag, enc in a.replies) or "no reply")
                     for a in r.attempts)


def run_one(report: Report, exe: Path, service: str, paths: list[Path], scale: int) -> None:
    import zenoh

    from . import bundle, live
    from .contract import load_contract
    from .descriptor import check_descriptor

    run = f"{service} ← {', '.join(p.name for p in paths)}"
    contracts = [load_contract(p) for p in paths]
    for p, c in zip(paths, contracts):
        if not c.valid:
            raise CannotRun(f"{p} does not load: {c.codes}")
    by_iface = {c.interface: c for c in contracts}
    built = {c.interface: bundle.build(c) for c in contracts}
    system, svc = service.split("/")

    owner = Owner(exe, service, paths)
    try:
        endpoint = owner.wait_for("listening ", 120)
        if endpoint is None:
            raise CannotRun(f"the owner printed no `listening` line (stderr: {owner.stderr[-3:]})")
        try:
            session = live.open_client(endpoint)
        except zenoh.ZError as e:
            raise CannotRun(f"cannot open a client session to {endpoint}: {e}") from e
        try:
            _checks(report, run, session, endpoint, owner, system, svc, by_iface, built, scale)
        finally:
            session.close()
    finally:
        code = owner.close()
        report.check(run, "the owner exits when its stdin closes", code == 0,
                     "killed after 15 s" if code is None else f"exit status {code}")


def _checks(report: Report, run: str, session, endpoint: str, owner: Owner, system: str, svc: str,
            by_iface: dict[str, Any], built: dict[str, bytes], scale: int) -> None:
    import zenoh

    from . import live
    from .descriptor import check_descriptor

    # -- presence, acted on as seen (§8.1, §8.2) --------------------------
    expected_alive = {(i, c.fingerprint.removeprefix("sha256:")[:16]) for i, c in by_iface.items()}
    selector = f"zk2/{system}/{svc}/@zk/**"
    deadline = time.monotonic() + PRESENCE_WAIT_S
    pres = live.list_presence(session, selector)
    while time.monotonic() < deadline:
        have = {(a["iface"], a["fp16"]) for a in pres.alive}
        if pres.instances and expected_alive <= have:
            break
        if owner.proc.poll() is not None:
            report.check(run, "the owner is still running", False,
                         f"exit status {owner.proc.returncode}: {' | '.join(owner.stderr[-3:])}")
            return
        time.sleep(0.2)
        pres = live.list_presence(session, selector)
    report.check(run, "presence GET completed before its timeout (callback handler, §8.1)",
                 pres.complete, f"{pres.elapsed_s:.3f} s, {pres.count} tokens")
    if not report.check(run, "exactly one instance token", len(pres.instances) == 1,
                        str([i["instance"] for i in pres.instances])):
        return
    instance = pres.instances[0]["instance"]
    instance_key = f"zk2/{system}/{svc}/@zk/instance/{instance}"
    ready = owner.wait_for("ready ", 5)
    report.check(run, "the `ready` line names the instance token seen on the bus",
                 ready == instance_key, f"ready={ready!r}")
    got_alive = {(a["iface"], a["fp16"]) for a in pres.alive}
    report.check(run, "one interface token per interface, <fp16> from zk2py's fingerprint",
                 got_alive == expected_alive and len(pres.alive) == len(by_iface)
                 and all(a["instance"] == instance for a in pres.alive),
                 f"seen {sorted(got_alive)}, expected {sorted(expected_alive)}")
    report.check(run, "every token under the service is a zk2 control key", not pres.other,
                 str(pres.other))
    # §8.1 (0.5): "An owner with no member yet holds no member token: one
    # that publishes nothing under the template holds none."
    report.check(run, "no member token: the owner publishes nothing (§8.1)", not pres.members,
                 f"{len(pres.members)} member tokens")

    # -- the descriptor (§3.3) --------------------------------------------
    answers = live.get_descriptor(session, instance_key)
    oks = [a for a in answers if a.ok]
    if not report.check(run, "the descriptor GET has exactly one value reply, on the instance key",
                        len(oks) == 1 and len(answers) == 1 and oks[0].key == instance_key,
                        f"{len(answers)} replies"):
        return
    d = oks[0]
    report.info(run, f"descriptor encoding {d.encoding!r}, {len(d.payload)} bytes")
    # §3.3 "The GET" (0.5): "one reply … Encoding application/json, no
    # attachment and no timestamp".
    report.check(run, "the descriptor reply: application/json, no timestamp, no attachment (§3.3)",
                 d.encoding == "application/json" and not d.has_timestamp and not d.has_attachment,
                 f"{d.encoding}, timestamp {d.has_timestamp}, attachment {d.has_attachment}")
    codes = check_descriptor(d.payload, list(by_iface.values()))
    report.check(run, "the descriptor has no D code against the contracts", codes == [], str(codes))
    doc = json.loads(d.payload)
    report.check(run, "descriptor service and instance", doc.get("service") == f"{system}/{svc}"
                 and doc.get("instance") == instance, f"{doc.get('service')}, {doc.get('instance')}")
    listed = {e["iface"]: e for e in doc.get("interfaces", [])}
    report.check(run, "the descriptor lists exactly the implemented interfaces",
                 set(listed) == set(by_iface), f"{sorted(listed)}")
    for iface, c in sorted(by_iface.items()):
        e = listed.get(iface, {})
        report.check(run, f"{iface}: the descriptor's fingerprint is zk2py's",
                     e.get("contract") == c.fingerprint, f"{e.get('contract')} vs {c.fingerprint}")
        report.check(run, f"{iface}: minor, token, nothing unavailable",
                     e.get("minor") == c.minor and e.get("token", True) is True and not e.get("unavailable"),
                     f"minor {e.get('minor')} (contract {c.minor}), token {e.get('token', True)}, "
                     f"unavailable {e.get('unavailable')}")
    gated = sorted({g.removeprefix("capability:") for c in by_iface.values()
                    for r in c.canonical["resources"] for g in r["gate"] if g.startswith("capability:")})
    report.check(run, "capabilities: every capability a gate names", sorted(doc.get("capabilities", [])) == gated,
                 f"{doc.get('capabilities')} vs {gated}")
    declared = {(iface, role, req["interface"]) for iface, c in by_iface.items()
                for role, req in c.canonical["requires"].items()}
    listed_roles = {(r.get("declared_by"), r["role"], r["interface"]) for r in doc.get("requires", [])}
    report.check(run, "R3: every contract-declared role is listed in requires",
                 declared <= listed_roles, f"declared {sorted(declared)}, listed {sorted(listed_roles)}")
    # §3.2 (0.5): "An unbound optional role is listed in the descriptor all
    # the same, with "bindings": [] and "params": {}". The owner example has
    # no binding configuration, so every role here is unbound.
    unbound_ok = all(r.get("bindings") == [] and r.get("params", {}) == {}
                     for r in doc.get("requires", []))
    report.check(run, "unbound roles are listed with bindings [] and params {} (§3.2)", unbound_ok,
                 str([(r["role"], r.get("bindings"), r.get("params")) for r in doc.get("requires", [])]))
    # §3.3 (0.5): "profiles is the union of the uses of the contracts the
    # instance implements, sorted and deduplicated".
    uses = sorted({u for c in by_iface.values() for u in c.canonical["uses"]})
    report.check(run, "profiles is the union of the contracts' uses (§3.3)", doc.get("profiles") == uses,
                 f"{doc.get('profiles')} vs {uses}")

    # -- retrieval (§8.4) -------------------------------------------------
    for iface in sorted(by_iface):
        fp = listed.get(iface, {}).get("contract") or by_iface[iface].fingerprint
        r = live.retrieve_bundle(session, iface, fp)
        if report.check(run, f"{iface}: bundle retrieved and verified (§8.4, §9.6)", r.available, _describe(r)):
            report.check(run, f"{iface}: the retrieved bundle is byte-identical to zk2py's build",
                         r.data == built[iface], f"{len(r.data)} vs {len(built[iface])} bytes")
            report.info(run, f"{iface}: accepted on {r.attempts[-1].target}, "
                             f"encoding {r.attempts[-1].replies[-1][2]!r}")
        # §8.4 (0.5): "A holder answers with one reply, the bundle's bytes,
        # with Encoding application/json" (a caller does not depend on it;
        # this checks the holder).
        all_replies = list(live._answers(session, live.contract_key(iface, fp),
                                         zenoh.QueryTarget.ALL, live.GET_TIMEOUT_S))
        report.check(run, f"{iface}: the holder answers one reply, application/json (§8.4)",
                     len(all_replies) == 1 and all_replies[0].ok
                     and all_replies[0].encoding == "application/json",
                     f"{[(a.ok, a.encoding) for a in all_replies]}")

    first = sorted(by_iface)[0]
    unknown = "sha256:" + "0" * 64
    r = live.retrieve_bundle(session, first, unknown)
    report.check(run, "an unheld revision is reported unavailable after BestMatching then All",
                 not r.available and [a.target for a in r.attempts] == ["BestMatching", "All"], _describe(r))

    # A corrupt nearest holder: this session holds a complete queryable on
    # the real contract key, answering with a tampered bundle
    # (retrieval.md §2). A valid bundle must still be accepted, and the
    # corrupt one never.
    key = live.contract_key(first, by_iface[first].fingerprint)
    bad = _corrupt(built[first])

    def answer(query) -> None:
        query.reply(str(query.key_expr), bad, encoding="application/json")

    import zenoh
    q = session.declare_queryable(key, zenoh.handlers.Callback(answer), complete=True)
    try:
        r = live.retrieve_bundle(session, first, by_iface[first].fingerprint)
        refused = [tag for a in r.attempts for ok, tag, _ in a.replies if not ok]
        report.check(run, "a corrupt nearest holder: its reply is refused, a valid bundle accepted",
                     r.available and r.data == built[first], _describe(r))
        report.info(run, f"corrupt holder: refusals {refused}, accepted on {r.attempts[-1].target}")
        qbad = session.declare_queryable(live.contract_key(first, unknown),
                                         zenoh.handlers.Callback(answer), complete=True)
        try:
            r = live.retrieve_bundle(session, first, unknown)
            report.check(run, "only corrupt holders: unavailable, nothing accepted (retrieval.md §3)",
                         not r.available and any(not ok for a in r.attempts for ok, _, _ in a.replies),
                         _describe(r))
        finally:
            qbad.undeclare()
    finally:
        q.undeclare()

    # -- presence at scale (presence.md §4) -------------------------------
    if scale > 0:
        _scale_check(report, run, session, endpoint, system, scale)


def _scale_check(report: Report, run: str, session, endpoint: str, system: str, n: int) -> None:
    """A second client declares ``n`` instance tokens; this session holds a
    liveliness subscriber (a callback, F-47) and lists everything with a
    callback GET."""
    import zenoh

    from . import live

    holder = live.open_client(endpoint)
    try:
        tokens = [holder.liveliness().declare_token(f"zk2/{system}/load{i}/@zk/instance/{i:016x}")
                  for i in range(n)]
        sub = session.liveliness().declare_subscriber("zk2/*/*/@zk/**", zenoh.handlers.Callback(lambda s: None))
        try:
            pres = live.list_presence(session, f"zk2/{system}/*/@zk/**")
            deadline = time.monotonic() + SCALE_PROPAGATION_S
            while time.monotonic() < deadline and sum(1 for i in pres.instances
                                                      if i["service"].startswith("load")) < n:
                time.sleep(0.2)
                pres = live.list_presence(session, f"zk2/{system}/*/@zk/**")
            loaded = sum(1 for i in pres.instances if i["service"].startswith("load"))
            report.check(run, f"presence at scale: a callback GET lists all {n} tokens while a "
                              "liveliness subscriber is held (presence.md §4)",
                         loaded == n and pres.complete, f"{loaded}/{n} in {pres.elapsed_s:.3f} s, "
                                                        f"complete={pres.complete}")
        finally:
            sub.undeclare()
        for t in tokens:
            t.undeclare()
    finally:
        holder.close()


def run_refusal(report: Report, exe: Path, service: str, paths: list[Path]) -> None:
    """§3.2 (0.5): "An owner whose configuration binds a required role to
    nothing MUST NOT start …: no instance token appears." Watched from a
    client for the conformance second, or until the owner exits."""
    from . import live

    run = f"{service} ← {', '.join(p.name for p in paths)}"
    system, svc = service.split("/")
    owner = Owner(exe, service, paths)
    seen: set[str] = set()
    watched = 0  # presence GETs that completed while the owner ran
    try:
        endpoint = owner.wait_for("listening ", 120)
        if endpoint is not None and owner.proc.poll() is None:
            try:
                session = live.open_client(endpoint)
            except Exception:  # noqa: BLE001 - the router may already be gone
                session = None
            if session is not None:
                try:
                    deadline = time.monotonic() + PRESENCE_WAIT_S
                    while time.monotonic() < deadline and owner.proc.poll() is None:
                        pres = live.list_presence(session, f"zk2/{system}/{svc}/@zk/**")
                        seen |= {i["instance"] for i in pres.instances}
                        watched += pres.complete
                        time.sleep(0.1)
                finally:
                    session.close()
        ready = owner.wait_for("ready ", 0.5)
    finally:
        code = owner.close()
    # The owner is its own router: when it refuses, it exits, and a client
    # may never connect. So the check rests on what can be observed: no
    # instance token while it ran, no `ready` line (SPEC-FINDINGS F-61).
    report.check(run, "an unbound required role: the owner does not start (no instance token "
                      "while it ran, no `ready` line) (§3.2)",
                 not seen and ready is None,
                 f"instance tokens {sorted(seen)} over {watched} presence GETs before the owner "
                 f"exited, ready {ready!r}; stderr: {' | '.join(owner.stderr[-1:])}")
    report.info(run, f"owner exit status {code}")


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="python -m zk2py.live_interop", description=__doc__.split("\n")[0])
    ap.add_argument("--owner", type=Path, default=Path(os.environ.get("ZK2PY_OWNER", DEFAULT_OWNER)),
                    help="the owner example binary")
    ap.add_argument("--run", action="append", metavar="SYSTEM/SERVICE=CONTRACT[,CONTRACT…]",
                    help="one owner run; repeatable (default: two runs over the examples and "
                         "impl/python/interop)")
    ap.add_argument("--scale", type=int, default=2000,
                    help="extra tokens for the presence-at-scale check, in the first run (0: skip)")
    args = ap.parse_args(argv)
    runs = DEFAULT_RUNS
    if args.run:
        runs = []
        for spec in args.run:
            service, _, files = spec.partition("=")
            runs.append((service, files.split(",")))
    report = Report()
    try:
        import zenoh  # noqa: F401
    except ImportError as e:
        print(f"error: zenoh-python is not installed: {e}", file=sys.stderr)
        return 2
    try:
        for i, (service, files) in enumerate(runs):
            run_one(report, args.owner, service, [REPO / f if not Path(f).is_absolute() else Path(f)
                                                  for f in files], args.scale if i == 0 else 0)
        if not args.run:
            for service, files in REFUSAL_RUNS:
                run_refusal(report, args.owner, service, [REPO / f for f in files])
    except CannotRun as e:
        print(f"error: could not run: {e}", file=sys.stderr)
        return 2
    passed = sum(1 for r in report.rows if r[2])
    failed = len(report.rows) - passed
    print(f"live interop: {passed} passed, {failed} failed")
    for run, name, ok, detail in report.rows:
        if not ok:
            print(f"FAIL [{run}] {name}: {detail}")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())

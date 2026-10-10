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
- **retrieval** (§8.4): every bundle retrieved by the procedure, by the
  full fingerprint the descriptor names for the token's ``fp16`` (0.8),
  verified, and byte-identical to the bundle zk2py builds. Also: an unknown
  revision is reported unavailable after the retry, and a corrupt nearest
  holder is refused (scenarios retrieval.md §2, §3);
- **presence at scale** (presence.md §4): with a liveliness subscriber held,
  a callback GET lists every one of N extra tokens;
- **shutdown:** closing the owner's stdin makes it exit, with status 0.

Then the runs behind a router R1 of the runner's (a zenoh-python router):
the owner example as R1's client (state.md §1's stamp, and its templated
operations), zk2py's own owner read by the Rust ``consume`` example, the
refusals of presence.md §2, state.md §1, presence.md §1, presence.md §6 (a
read refused by access control, and one stalled past its timeout),
operations.md §1 (target and consolidation shown by behaviour),
operations.md §2 (fan-out over templates), the tool rules of 0.10 to 0.13,
access control from §11 (``zk2py.acl_interop``: security.md §1–§3 on
generated grants), ``hostid.v1`` with core R1's ``self.system``
providers (0.20), and ``freshness.v1``'s re-puts and judgements (0.21),
across both implementations. ``--only`` picks some of them.

Exit 0 when every check passes, 1 when any fails, 2 when it could not run.
A rule the owner example is known not to meet is reported XFAIL (or XPASS),
never as a failure.
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
    ("vehicle-01/echo", ["impl/python/interop/zk2py_echo.v1.toml"]),
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
SCALE_PROPAGATION_S = 30.0


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

    known: list[tuple[str, str, bool, str]] = field(default_factory=list)

    def known_deviation(self, run: str, name: str, ok: bool, detail: str, why: str) -> None:
        """A rule the Rust owner example is known not to meet yet, as the
        spec records it or SPEC-FINDINGS observes it: printed as XFAIL (or
        XPASS once it meets it), never counted as a failure of this
        runner."""
        self.known.append((run, name, ok, f"{detail} [{why}]"))
        print(f"{'XPASS' if ok else 'XFAIL'} [{run}] {name}: {detail} [{why}]", flush=True)


class Owner:
    """The owner example as a child process, read line by line."""

    def __init__(self, exe: Path, service: str, contracts: list[Path], connect: str | None = None,
                 hostid_root: str | None = None, hostid_ephemeral: bool = False):
        """``connect``: run it as a client of that router (its documented
        ``--connect <endpoint>``; it then prints ``connected …`` rather than
        ``listening …``). ``service`` may be ``@hostid.v1/<service>``, a
        minted system (hostid.v1 §2.3), with ``hostid_root`` its
        ``--hostid-root <dir>`` and ``hostid_ephemeral`` its
        ``--hostid-ephemeral`` (its usage, read by running it)."""
        if not exe.is_file():
            raise CannotRun(f"{exe} not found: build it with "
                            "`cargo build -q -p zenkey --example owner`")
        self.proc = subprocess.Popen([str(exe), *(["--connect", connect] if connect else []),
                                      *(["--hostid-root", hostid_root] if hostid_root else []),
                                      *(["--hostid-ephemeral"] if hostid_ephemeral else []),
                                      service, *map(str, contracts)],
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


def _roles_optional(report: Report, run: str, doc: dict[str, Any], by_iface: dict[str, Any]) -> None:
    """§3.3 (0.10): ``optional`` "is true for a role the instance works
    without, and absent otherwise … For a role a contract declares, it
    repeats that contract's [requires]"."""
    got, want = {}, {}
    for r in doc.get("requires", []):
        c = by_iface.get(r.get("declared_by"))
        if c is None or r.get("role") not in c.canonical["requires"]:
            continue
        got[r["role"]] = r.get("optional", "absent")
        want[r["role"]] = True if c.canonical["requires"][r["role"]]["optional"] else "absent"
    report.check(run, "each contract role's `optional`: true when the contract's role is optional, absent "
                      "otherwise (§3.3, 0.10)", got == want, f"descriptor {got}, contracts {want}")


def _meta_zid(report: Report, run: str, doc: dict[str, Any]) -> None:
    """§3.3 (0.10): "an owner SHOULD state its session's zid as meta.zid";
    (0.11) "An owner SHOULD write meta.zid as zenoh writes it": lowercase
    hexadecimal without leading zeros, at most 32 digits (Appendix B)."""
    import re

    zid = doc.get("meta", {}).get("zid") if isinstance(doc.get("meta"), dict) else None
    report.check(run, "the descriptor states meta.zid as zenoh writes it: lowercase hex, no leading zero, at "
                      "most 32 digits (§3.3, 0.10, 0.11)",
                 isinstance(zid, str) and re.fullmatch(r"[1-9a-f][0-9a-f]{0,31}", zid) is not None,
                 f"meta {doc.get('meta')}")


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
    if "zk2py_echo.v1" in by_iface and pres.alive:
        # §8.2 (0.7) "State values": put before the tokens, so a GET made
        # when the interface token appears finds it.
        first = live.get_state(session, f"zk2/{system}/{svc}/zk2py_echo.v1/state/health")
        report.check(run, "a state GET on first sight of the interface token finds the value (§8.2, F-68)",
                     [r.payload for r in first.replies] == [b"ok"], str([r.payload for r in first.replies]))
    report.check(run, "presence GET complete: the routers' final reply, no error reply (§8.1, 0.8)",
                 pres.complete, f"{pres.elapsed_s:.3f} s, {pres.reading}")
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
    _roles_optional(report, run, doc, by_iface)
    _meta_zid(report, run, doc)
    # §3.3 (0.19): "the union of two sets: the uses of the contracts the
    # instance implements; the derivation-only profiles the instance
    # follows", which today is hostid.v1 "listed by an instance whose system
    # is minted". The runner configured this system, so it knows: a literal
    # one adds nothing, and @hostid.v1 adds hostid.v1 (hostid.v1 §2.3, §2.8).
    minted = system == "@hostid.v1"
    profiles = sorted({u for c in by_iface.values() for u in c.canonical["uses"]}
                      | ({"hostid.v1"} if minted else set()))
    report.check(run, "profiles is the union of the contracts' uses and, for a minted system, hostid.v1 "
                      f"(§3.3, 0.19; this system is {'minted' if minted else 'literal'})",
                 doc.get("profiles") == profiles, f"{doc.get('profiles')} vs {profiles}")

    # -- retrieval (§8.4) -------------------------------------------------
    # 0.8 "From a token to a fingerprint": the token's fp16 is not enough to
    # retrieve by; the full fingerprint comes from the descriptor.
    token_fp16 = {a["iface"]: a["fp16"] for a in pres.alive}
    for iface in sorted(by_iface):
        fp = live.fingerprint_of(doc, iface, token_fp16.get(iface))
        if not report.check(run, f"{iface}: the token's fp16 leads to the descriptor's full fingerprint (§8.4)",
                            fp is not None, f"fp16 {token_fp16.get(iface)}, descriptor "
                                            f"{listed.get(iface, {}).get('contract')}"):
            continue
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

    # -- state and operations (§4, §5) ------------------------------------
    if "zk2py_echo.v1" in by_iface:
        _state_and_calls(report, run, session, system, svc, doc, None)

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
            def count(p) -> int:
                return sum(1 for i in p.instances if i["service"].startswith("load"))

            # The 2,000 tokens take about 3 s to reach the router from the
            # holder's client (16–17 polls on this host), longer under load:
            # each early GET is complete and sees what the router holds so
            # far. Every attempt's (count, complete) is kept for the report.
            pres = live.list_presence(session, f"zk2/{system}/*/@zk/**")
            attempts = [(count(pres), pres.complete)]
            deadline = time.monotonic() + SCALE_PROPAGATION_S
            while time.monotonic() < deadline and count(pres) < n:
                time.sleep(0.2)
                pres = live.list_presence(session, f"zk2/{system}/*/@zk/**")
                attempts.append((count(pres), pres.complete))
            loaded = count(pres)
            report.check(run, f"presence at scale: a callback GET lists all {n} tokens while a "
                              "liveliness subscriber is held (presence.md §4)",
                         loaded == n and pres.complete,
                         f"{loaded}/{n} in {pres.elapsed_s:.3f} s, complete={pres.complete}, "
                         f"{len(attempts)} attempts" + ("" if loaded == n else f": {attempts[:5]}…{attempts[-3:]}"))
        finally:
            sub.undeclare()
        for t in tokens:
            t.undeclare()
    finally:
        holder.close()


def run_rust_behind_r1(report: Report, exe: Path) -> None:
    """The Rust owner example as a client of a router R1 of the runner's
    (``--connect``), the setup state.md §1 asks for (F-69): the value it
    holds from the start is stamped with its own session's zid, not R1's,
    and that zid is its descriptor's ``meta.zid`` (§3.3, 0.10). It also
    serves ``zk2py_tc.v1``, whose operations are templated (§5.1), and
    ``zs.thresholds.v1``, whose role ``desired`` is optional: its
    descriptor says so (``optional: true``, 0.10)."""
    from . import live
    from .contract import load_contract

    service = "vehicle-02/tc"
    system, svc = service.split("/")
    paths = [REPO / ECHO, REPO / TC, REPO / THRESHOLDS]
    by_iface = {c.interface: c for c in (load_contract(p) for p in paths)}
    run = f"{service} ← {', '.join(p.name for p in paths)} (the owner example behind R1)"
    r1, r1_endpoint, r1_zid = _r1()
    try:
        tool = live.open_client(r1_endpoint)
        owner = Owner(exe, service, paths, connect=r1_endpoint)
        try:
            connected = owner.wait_for("connected ", 120)
            if not report.check(run, "the owner example connects to R1", connected == r1_endpoint,
                                f"connected {connected!r}; stderr {owner.stderr[-2:]}"):
                return
            selector = f"zk2/{system}/{svc}/@zk/**"
            deadline = time.monotonic() + PRESENCE_WAIT_S
            pres = live.list_presence(tool, selector)
            n = len(paths)
            while time.monotonic() < deadline and not (pres.instances and len(pres.alive) == n):
                time.sleep(0.05)
                pres = live.list_presence(tool, selector)
            if not report.check(run, f"presence through R1: one instance token, {n} interface tokens",
                                len(pres.instances) == 1 and len(pres.alive) == n and pres.complete,
                                pres.reading):
                return
            instance_key = f"zk2/{system}/{svc}/@zk/instance/{pres.instances[0]['instance']}"
            d = live.get_descriptor(tool, instance_key)
            doc = json.loads(d[0].payload) if len(d) == 1 and d[0].ok else {}
            _meta_zid(report, run, doc)
            _roles_optional(report, run, doc, by_iface)
            zid = doc.get("meta", {}).get("zid")
            st = live.get_state(tool, f"zk2/{system}/{svc}/zk2py_echo.v1/state/health")
            who = [live.attribute_stamp(r.stamp_id, doc) for r in st.replies]
            report.check(run, "state.md §1 through R1: the value held from the start is attributed to the owner "
                              "by meta.zid (§3.3, 0.10), and that zid is not R1's",
                         [r.payload for r in st.replies] == [b"ok"] and who == ["owner"] and zid != r1_zid,
                         f"{[(r.payload, r.stamp_id) for r in st.replies]} {who}, meta.zid {zid}, R1 {r1_zid}")
            # §3.3 (0.11): "A tool MUST compare two zids by value". The same
            # zid in capitals, padded to 32 digits, is another text.
            respelled = str(zid).upper().rjust(32, "0")
            again = [live.attribute_stamp(r.stamp_id, {"meta": {"zid": respelled}}) for r in st.replies]
            report.check(run, "by value, not text: meta.zid respelled in capitals and padded to 32 digits still "
                              "attributes the stamp to the owner (§3.3, 0.11)",
                         again == ["owner"] and respelled != zid, f"{respelled} → {again}")
            base = f"zk2/{system}/{svc}/zk2py_tc.v1/@op"
            diag = live.call(tool, f"{base}/diagnostics", b"x")
            report.check(run, "a parameterless operation answers through R1 (O3)",
                         [(r.kind, r.key) for r in diag.replies] == [("value", f"{base}/diagnostics")],
                         str([(r.kind, r.key) for r in diag.replies]))
            claimed = [e.get("unavailable") for e in doc.get("interfaces", []) if e.get("iface") == "zk2py_tc.v1"]

            def shown(res) -> str:
                return str([(r.kind, r.key, (r.envelope or {}).get("code")) for r in res.replies])

            # Its templated operations, which it serves since 0.9's fix
            # (the 0.8 round's two XFAILs).
            member = live.call(tool, f"{base}/interfaces/eth0/set", b"x")
            report.check(run, "a templated operation it claims exposed answers a concrete call, on the member's "
                              "key (O1, O3, §8.2 \"Exposed\")",
                         [(r.kind, r.key) for r in member.replies] == [("value", f"{base}/interfaces/eth0/set")],
                         f"{shown(member)}; descriptor lists unavailable {claimed}")
            malformed = live.call(tool, f"{base}/interfaces/ETH0/set", b"x")
            report.check(run, "interfaces/ETH0/set is invalid_request (operations.md §3 step 3, §5.1)",
                         [(r.envelope or {}).get("code") for r in malformed.replies] == ["invalid_request"],
                         shown(malformed))
            # operations.md §2 against it, where it serves the operation.
            fan = f"zk2/*/{svc}/zk2py_tc.v1/@op"
            o2 = live.call(tool, f"{fan}/interfaces/ETH0/set", b"x", fanout=True)
            report.check(run, "operations.md §2 step 1 (0.9): a wildcard call to set with ETH0 is "
                              "fanout_forbidden, O2 first (§5.1 \"The order of refusals\")",
                         [(r.envelope or {}).get("code") for r in o2.replies] == ["fanout_forbidden"], shown(o2))
            bad = live.call(tool, f"{fan}/interfaces/ETH0/reset", b"x", fanout=True)
            report.check(run, "operations.md §2 step 4: a fan-out to reset with ETH0 is invalid_request",
                         [(r.envelope or {}).get("code") for r in bad.replies] == ["invalid_request"], shown(bad))
            unbound = live.call(tool, f"{fan}/interfaces/*/reset", b"x", fanout=True)
            report.check(run, "a fan-out that leaves the parameter unbound: its echo names no member, so it is "
                              "refused internal (§5.1 \"Over a template\")",
                         [(r.envelope or {}).get("code") for r in unbound.replies] == ["internal"], shown(unbound))
        finally:
            code = owner.close()
            tool.close()
        report.check(run, "the owner exits when its stdin closes", code == 0,
                     "killed after 15 s" if code is None else f"exit status {code}")
    finally:
        r1.close()


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


# -- state and operations (§4, §5): either owner ------------------------------------

ECHO = "impl/python/interop/zk2py_echo.v1.toml"
NEEDS = "impl/python/interop/zk2py_needs.v1.toml"
BRINGUP = "impl/python/interop/zk2py_bringup.v1.toml"
TC = "impl/python/interop/zk2py_tc.v1.toml"
SCAN = "impl/python/interop/zk2py_scan.v1.toml"
BRINGUP_V1_1 = "impl/python/interop/rev/zk2py_bringup.v1.toml"
ARCHIVE_STANDIN = "impl/python/interop/stand-in/archive.v1.toml"
THRESHOLDS = "examples/zk2/zensight/zs.thresholds.v1.toml"
DEFAULT_CONSUME = REPO / "target" / "debug" / "examples" / "consume"


def _state_and_calls(report: Report, run: str, session, system: str, svc: str, owner_doc: dict[str, Any],
                     expect_typed: str | None) -> None:
    """The consumer's and the caller's side, against an owner serving
    ``zk2py_echo.v1``: a state GET per S4, calls per O1–O5, envelopes per
    §5.2. ``owner_doc`` is the owner's descriptor, by whose ``meta.zid`` a
    stamp is attributed (0.10). ``expect_typed`` is the code the owner
    gives ``@op/typed``'s malformed request, or None to accept any valid
    envelope."""
    from . import live

    base = f"zk2/{system}/{svc}/zk2py_echo.v1"
    key = f"{base}/state/health"
    st = live.get_state(session, key)
    ok = len(st.replies) == 1 and st.replies[0].key == key and not st.replies[0].deleted \
        and st.replies[0].payload == b"ok"
    report.check(run, "state GET (S4: All + Latest): the owner's one current value, 'ok'", ok,
                 f"{[(r.key, r.payload, r.deleted) for r in st.replies]} {st.errors}")
    if ok:
        r = st.replies[0]
        who = live.attribute_stamp(r.stamp_id, owner_doc)
        report.check(run, "the value is stamped by the owner (S1, S2): attributed by the descriptor's meta.zid "
                          "(§3.3, 0.10)", who == "owner",
                     f"{who}: stamp {r.stamp}, meta {owner_doc.get('meta')}")
        report.check(run, "the value carries its media type as its Encoding (§7.2)", r.encoding == "text/plain",
                     r.encoding)
    sel = live.get_state(session, f"{base}/state/*")
    report.check(run, "a state selector answers every matching live key (S2)",
                 [r.key for r in sel.replies] == [key], str([r.key for r in sel.replies]))
    none = live.get_state(session, f"{base}/state/nothing_here")
    report.check(run, "a key nobody holds: silence, reported as such, not a verdict (S6, O5)", none.silent,
                 f"{len(none.replies)} replies")

    echo = live.call(session, f"{base}/@op/echo", b"ping")
    report.check(run, "@op/echo: one value reply, the request's bytes, on the operation's key (O1, O3)",
                 [(r.kind, r.key, r.payload) for r in echo.replies] == [("value", f"{base}/@op/echo", b"ping")],
                 str([(r.kind, r.key, r.payload) for r in echo.replies]))
    refuse = live.call(session, f"{base}/@op/refuse", b"")
    report.check(run, "@op/refuse: one reply_err, an `app` envelope in zk2.core.v1.Error (O3, §5.2)",
                 len(refuse.replies) == 1 and refuse.replies[0].kind == "envelope"
                 and refuse.replies[0].envelope["code"] == "app"
                 and refuse.replies[0].encoding == "application/protobuf;zk2.core.v1.Error",
                 str([(r.kind, r.encoding, r.envelope or r.refusal) for r in refuse.replies]))
    # §5.2 (0.7): "With no error type there is no detail." @op/refuse
    # declares none.
    no_detail = len(refuse.replies) == 1 and refuse.replies[0].envelope is not None \
        and refuse.replies[0].envelope["detail"] is None
    report.check(run, "@op/refuse's app envelope carries no detail (no error type, §5.2, F-65)", no_detail,
                 str([(r.envelope or {}).get("detail") for r in refuse.replies]))
    typed = live.call(session, f"{base}/@op/typed", b"not json")
    good = len(typed.replies) == 1 and typed.replies[0].kind == "envelope" \
        and typed.replies[0].encoding == "application/json" \
        and (expect_typed is None or typed.replies[0].envelope["code"] == expect_typed)
    report.check(run, "@op/typed, a malformed request: one reply_err whose JSON envelope decodes (§5.2)", good,
                 str([(r.kind, r.encoding, r.envelope or r.refusal) for r in typed.replies]))
    fan = live.call(session, f"{base}/@op/*", b"ping", fanout=True)
    report.check(run, "a call on a wildcard key: fanout_forbidden from each operation, no value (O2)",
                 len(fan.replies) == 3 and all(r.kind == "envelope" and r.envelope["code"] == "fanout_forbidden"
                                               for r in fan.replies),
                 str([(r.kind, (r.envelope or {}).get("code")) for r in fan.replies]))
    silent = live.call(session, f"{base}/@op/nothing_here", b"ping")
    report.check(run, "a call nobody serves: silence, never \"no such operation\" (O5)", silent.silent,
                 f"{len(silent.replies)} replies")


def run_python_owner(report: Report, consume: Path) -> None:
    """zk2py as the owner (§8.2's order, §3.3, §4, §5), read by zk2py's own
    client and by the Rust ``consume`` example."""
    from . import bundle, live
    from .contract import load_contract
    from .descriptor import check_descriptor
    from .owner import Owner as PyOwner

    if not consume.is_file():
        raise CannotRun(f"{consume} not found: build it with `cargo build -q -p zenkey --example consume`")
    system, svc = "py-site", "echo"
    run = f"zk2py owner {system}/{svc} ← zk2py_echo.v1.toml"
    path = REPO / ECHO
    contract = load_contract(path)
    owner = PyOwner(system, svc, [contract])
    owner.start()
    try:
        session = live.open_client(owner.endpoint)
        try:
            deadline = time.monotonic() + PRESENCE_WAIT_S
            pres = live.list_presence(session, f"zk2/{system}/{svc}/@zk/**")
            while time.monotonic() < deadline and not (pres.instances and pres.alive):
                time.sleep(0.1)
                pres = live.list_presence(session, f"zk2/{system}/{svc}/@zk/**")
            fp16 = contract.fingerprint.removeprefix("sha256:")[:16]
            report.check(run, "presence: its instance token, then one interface token (§8.1, §8.2)",
                         [i["instance"] for i in pres.instances] == [owner.instance]
                         and [(a["iface"], a["fp16"]) for a in pres.alive] == [("zk2py_echo.v1", fp16)],
                         f"{[i['instance'] for i in pres.instances]} {[(a['iface'], a['fp16']) for a in pres.alive]}")
            answers = live.get_descriptor(session, owner.instance_key)
            ok = len(answers) == 1 and answers[0].ok and answers[0].encoding == "application/json" \
                and not answers[0].has_timestamp and not answers[0].has_attachment
            report.check(run, "its descriptor: one reply, application/json, no timestamp or attachment (§3.3)",
                         ok, str([(a.ok, a.encoding, a.has_timestamp) for a in answers]))
            fp = None
            doc: dict[str, Any] = {}
            if ok:
                codes = check_descriptor(answers[0].payload, [contract])
                report.check(run, "its descriptor has no D code", codes == [], str(codes))
                doc = json.loads(answers[0].payload)
                _meta_zid(report, run, doc)
                fp = live.fingerprint_of(doc, "zk2py_echo.v1", pres.alive[0]["fp16"] if pres.alive else None)
            r = live.retrieve_bundle(session, "zk2py_echo.v1", fp) if fp else None
            report.check(run, "its bundle, retrieved per §8.4 by the descriptor's fingerprint, verifies and is "
                              "the build's bytes",
                         r is not None and r.available and r.data == bundle.build(contract),
                         _describe(r) if r else f"no fingerprint from the token's fp16 and the descriptor")
            _state_and_calls(report, run, session, system, svc, doc, "invalid_request")
        finally:
            session.close()

        # The Rust consumer, as a client of zk2py's router.
        stamp = str(owner._held[f"zk2/{system}/{svc}/zk2py_echo.v1/state/health"].stamp)
        key = f"zk2/{system}/{svc}/zk2py_echo.v1/state/health"
        for op, want in (("@op/echo", "call value ping"),
                         ("@op/refuse", "call refused app "),
                         ("@op/typed", "call refused invalid_request ")):
            p = subprocess.run([str(consume), owner.endpoint, f"{system}/{svc}", str(path), "state/health", op],
                               capture_output=True, text=True, timeout=60)
            lines = p.stdout.splitlines()
            ok = (p.returncode == 0 and len(lines) == 3 and lines[0] == f"present {system}/{svc}"
                  and lines[1] == f"state {key} {stamp} ok" and lines[2].startswith(want))
            report.check(run, f"the Rust consumer reads zk2py's owner: present, the stamped state, {op}",
                         ok, f"exit {p.returncode}, {lines}, stderr {p.stderr.strip()[-200:]!r}")
    finally:
        owner.close()


def run_python_refusal(report: Report) -> None:
    """presence.md §2 steps 4 to 6 (0.6, 0.7, 0.17): zk2py's owner, a client
    of a router R1 that outlives it, watched by a liveliness subscriber
    through R1 declared before launch. Each refusal has its control,
    launched the same way, which shows its instance token within the wait;
    the refusal shows none, and a liveliness GET through R1 afterwards
    returns none.
    - step 4: a required role bound to nothing (§3.2);
    - step 5: an optional resource, its gate's capability held, neither
      exposed nor listed unavailable (§8.2 step 2); its control lists it;
    - step 6: archive.v1 in the tokenless set of an owner that does not
      implement it (§4.4, §8.2 step 2); its control's set is empty."""
    import zenoh

    from . import live
    from .contract import load_contract
    from .descriptor import check_descriptor
    from .owner import Owner as PyOwner, OwnerRefused

    system, svc = "py-site", "needs"
    contract = load_contract(REPO / NEEDS)
    r1, r1_endpoint, _ = _r1()
    try:
        watcher = live.open_client(r1_endpoint)
        seen: list[tuple[str, str]] = []
        sub = watcher.liveliness().declare_subscriber(
            f"zk2/{system}/{svc}/@zk/**",
            zenoh.handlers.Callback(lambda smp: seen.append((str(smp.kind), str(smp.key_expr)))),
            history=True)
        bound = {"upstream": ["py-site/echo"]}
        cases = [
            ("step 4: an unbound required role (§3.2)",
             dict(bindings=bound, capabilities={"cal"}),
             dict(capabilities={"cal"})),
            ("step 5: an optional resource neither exposed nor listed (§8.2 step 2)",
             dict(bindings=bound, capabilities={"cal"}, unavailable={"state/calibration": "config"}),
             dict(bindings=bound, capabilities={"cal"}, withhold={"state/calibration"})),
            # 0.17: zk2py_needs.v1 does not implement archive.v1; the
            # control's tokenless set is empty.
            ("step 6: archive.v1 in the tokenless set of an owner that does not implement it (§4.4, §8.2 "
             "step 2, 0.17)",
             dict(bindings=bound, capabilities={"cal"}, tokenless=set()),
             dict(bindings=bound, capabilities={"cal"}, tokenless={"archive.v1"})),
        ]
        try:
            for label, control_kw, refusal_kw in cases:
                run = f"zk2py owner {system}/{svc} ← zk2py_needs.v1.toml, {label}"
                seen.clear()
                control = PyOwner(system, svc, [contract], connect=r1_endpoint, **control_kw)
                control.start()
                deadline = time.monotonic() + PRESENCE_WAIT_S
                while time.monotonic() < deadline and not any(k == control.instance_key for _, k in seen):
                    time.sleep(0.05)
                report.check(run, "the control shows its instance token to the watcher within 1 s",
                             any(k == control.instance_key for _, k in seen), str(seen))
                if label.startswith("step 4"):
                    # §3.3 (0.10): a required role bound, an optional one
                    # unbound: `optional` on the second only.
                    d = live.get_descriptor(watcher, control.instance_key)
                    cdoc = json.loads(d[0].payload) if len(d) == 1 and d[0].ok else {}
                    report.check(run, "the control's descriptor has no D code against zk2py_needs.v1",
                                 bool(cdoc) and check_descriptor(d[0].payload, [contract]) == [],
                                 f"{len(d)} replies")
                    _roles_optional(report, run, cdoc, {contract.interface: contract})
                control.close()
                deadline = time.monotonic() + 5.0
                while time.monotonic() < deadline and \
                        live.list_presence(watcher, f"zk2/{system}/{svc}/@zk/**").count:
                    time.sleep(0.1)
                seen.clear()
                refused = PyOwner(system, svc, [contract], connect=r1_endpoint, **refusal_kw)
                try:
                    refused.start()
                    outcome = "started"
                except OwnerRefused as e:
                    outcome = f"refused: {e}"
                time.sleep(PRESENCE_WAIT_S)
                after = live.list_presence(watcher, f"zk2/{system}/{svc}/@zk/**")
                report.check(run, "the owner refuses, the watcher sees no token, and a GET through R1 "
                                  "returns none (presence.md §2)",
                             outcome.startswith("refused") and not [k for kind, k in seen if "PUT" in kind]
                             and after.count == 0 and after.complete,
                             f"{outcome}; watcher saw {seen}; GET after: {after.count} tokens")
                if outcome == "started":
                    refused.close()
        finally:
            sub.undeclare()
            watcher.close()
    finally:
        r1.close()


def _r1(timestamping: bool = True, adminspace: bool = False):
    """A router of the runner's own, which outlives every owner it serves
    (presence.md §2, state.md §1): (session, endpoint, zid). ``adminspace``
    enables its admin space, read-only, as §4.2 (0.10) asks of a deployment
    that wants S4 checked."""
    import zenoh

    from .owner import free_loopback_port

    port = free_loopback_port()
    conf = zenoh.Config()
    conf.insert_json5("mode", json.dumps("router"))
    conf.insert_json5("listen/endpoints", json.dumps([f"tcp/127.0.0.1:{port}"]))
    conf.insert_json5("scouting/multicast/enabled", "false")
    conf.insert_json5("timestamping/enabled", "true" if timestamping else "false")
    if adminspace:
        conf.insert_json5("adminspace", json.dumps({"enabled": True,
                                                    "permissions": {"read": True, "write": False}}))
    r1 = zenoh.open(conf)
    return r1, f"tcp/127.0.0.1:{port}", str(r1.zid())


def run_python_s1(report: Report) -> None:
    """state.md §1 (0.7): the owner and the consumer are clients of R1,
    whose timestamping is on; the control is an unstamped put through R1."""
    import zenoh

    from . import live
    from .contract import load_contract
    from .owner import Owner as PyOwner

    system, svc = "py-site", "stamps"
    run = f"zk2py owner {system}/{svc} ← zk2py_echo.v1.toml (state.md §1, through R1)"
    r1, r1_endpoint, r1_zid = _r1()
    try:
        consumer = live.open_client(r1_endpoint)
        third = live.open_client(r1_endpoint)
        owner = PyOwner(system, svc, [load_contract(REPO / ECHO)], connect=r1_endpoint)
        key = f"zk2/{system}/{svc}/zk2py_echo.v1/state/health"
        samples: list[Any] = []
        control: list[Any] = []
        sub = consumer.declare_subscriber(key, zenoh.handlers.Callback(samples.append))
        csub = consumer.declare_subscriber(f"zk2/{system}/control/x", zenoh.handlers.Callback(control.append))
        try:
            time.sleep(0.3)
            owner.start()
            owner_zid = str(owner.session.zid())
            time.sleep(0.3)
            samples.clear()  # the value held at start; the steps begin here
            owner.set_state(key, b"v1")
            stamp_v2 = owner.set_state(key, b"v2")
            time.sleep(0.3)
            st = live.get_state(consumer, key)
            owner.delete_state(key)
            third.put(f"zk2/{system}/control/x", b"unstamped")
            time.sleep(0.5)
            stamps = [smp.timestamp for smp in samples]
            ids = [None if t is None else str(t).split("/", 1)[1] for t in stamps]
            report.check(run, "every sample, the delete included, carries the owner session's zid, not R1's",
                         len(samples) == 3 and all(i == owner_zid for i in ids) and owner_zid != r1_zid,
                         f"{[(str(x.kind), x.payload.to_bytes()) for x in samples]}, ids {ids}, "
                         f"owner {owner_zid}, R1 {r1_zid}")
            ntp = [t.get_time_as_ntp64().as_nanos() for t in stamps if t is not None]
            report.check(run, "each timestamp is greater than the previous one", ntp == sorted(set(ntp))
                         and len(ntp) == 3, str(ntp))
            report.check(run, "the GET after v2 returns v2 with v2's timestamp (S2)",
                         len(st.replies) == 1 and st.replies[0].payload == b"v2"
                         and st.replies[0].stamp == str(stamp_v2),
                         f"{[(r.payload, r.stamp) for r in st.replies]} vs {stamp_v2}")
            cid = [None if x.timestamp is None else str(x.timestamp).split("/", 1)[1] for x in control]
            report.check(run, "the control: an unstamped put through R1 arrives with R1's zid",
                         cid == [r1_zid], f"{cid} vs {r1_zid}")
            # §3.3 (0.10): a tool attributes a stamp by the descriptor's
            # meta.zid, read from the bus.
            d = live.get_descriptor(consumer, owner.instance_key)
            doc = json.loads(d[0].payload) if len(d) == 1 and d[0].ok else {}
            mine = [live.attribute_stamp(i, doc) for i in ids]
            theirs = [live.attribute_stamp(i, doc) for i in cid]
            report.check(run, "by the descriptor's meta.zid, a tool attributes the three samples to the owner, "
                              "and the control's stamp as foreign (§3.3, 0.10)",
                         mine == ["owner"] * 3 and theirs == ["foreign"], f"{mine} {theirs}")
            # Step 3 (0.8, F-73): only the order is observable from
            # outside; the tick is not.
            v3 = owner.set_state(key, b"v3")
            v4 = owner.set_state(key, b"v4")
            d = v4.get_time_as_ntp64().as_nanos() - v3.get_time_as_ntp64().as_nanos()
            report.check(run, "v3, v4 back to back: v4's timestamp is greater than v3's (§4.3)",
                         d > 0, f"{d} ns")
            # The tick, "with a clock the implementation controls": a clock
            # reading behind the last stamp, so the next is that plus 1 ns,
            # zk2py's tick (§4.3).
            behind = v4.get_time_as_ntp64().as_nanos() - 5_000_000_000
            owner.clock = lambda: zenoh.Timestamp(
                zenoh.NTP64(behind // 1_000_000_000, behind % 1_000_000_000), v4.get_id())
            v5 = owner.set_state(key, b"v5")
            owner.clock = None
            tick = v5.get_time_as_ntp64().as_nanos() - v4.get_time_as_ntp64().as_nanos()
            report.check(run, "a clock reading 5 s behind the record: the next stamp is the record plus one "
                              "tick (§4.3, a clock zk2py controls)",
                         tick == 1 and v5.get_id() == v4.get_id(), f"{tick} ns")
        finally:
            sub.undeclare()
            csub.undeclare()
            owner.close()
            third.close()
            consumer.close()
    finally:
        r1.close()


def run_python_bringup(report: Report) -> None:
    """presence.md §1 (0.7), zk2py's owner a client of R1: a tool, up before
    the owner, acts the moment the tokens appear."""
    import zenoh

    from . import bundle, live
    from .contract import load_contract
    from .owner import Owner as PyOwner

    system, svc = "py-site", "bringup"
    run = f"zk2py owner {system}/{svc} ← zk2py_bringup.v1.toml (presence.md §1, through R1)"
    contract = load_contract(REPO / BRINGUP)
    base = f"zk2/{system}/{svc}/zk2py_bringup.v1"
    r1, r1_endpoint, _ = _r1()
    try:
        tool = live.open_client(r1_endpoint)
        got_instance, got_alive = threading.Event(), threading.Event()
        tokens: list[str] = []

        def on_token(smp) -> None:
            k = str(smp.key_expr)
            tokens.append(k)
            if "/@zk/instance/" in k:
                got_instance.set()
            if "/@zk/alive/" in k:
                got_alive.set()

        descriptor_puts: list[bytes] = []
        lsub = tool.liveliness().declare_subscriber("zk2/*/*/@zk/**", zenoh.handlers.Callback(on_token),
                                                     history=True)
        dsub = tool.declare_subscriber("zk2/*/*/@zk/instance/*", zenoh.handlers.Callback(
            lambda smp: descriptor_puts.append(smp.payload.to_bytes())))
        owner = PyOwner(system, svc, [contract], connect=r1_endpoint)
        try:
            time.sleep(0.3)
            owner.start()
            report.check(run, "the owner starts although its templated state has no member (§8.2)", True,
                         "started")
            seen = got_instance.wait(PRESENCE_WAIT_S)
            d = live.get_descriptor(tool, owner.instance_key) if seen else []
            # §8.4 (0.8): retrieve by the descriptor's full fingerprint.
            fp = live.fingerprint_of(json.loads(d[0].payload), "zk2py_bringup.v1") \
                if len(d) == 1 and d[0].ok else None
            b = live.retrieve_bundle(tool, "zk2py_bringup.v1", fp) if fp else None
            report.check(run, "the moment the instance token appears: the descriptor and the bundle answer",
                         seen and len(d) == 1 and d[0].ok and b is not None and b.available,
                         f"token {seen}, descriptor {len(d)} replies, bundle {b and b.available}")
            alive = got_alive.wait(PRESENCE_WAIT_S)
            c = live.call(tool, f"{base}/@op/echo", b"ping") if alive else None
            st = live.get_state(tool, f"{base}/state/health") if alive else None
            report.check(run, "the moment the interface token appears: the call succeeds (alive ⇒ callable)",
                         c is not None and [(r.kind, r.payload) for r in c.replies] == [("value", b"ping")],
                         str(c and [(r.kind, r.payload) for r in c.replies]))
            report.check(run, "and the state GET finds the value it started with (§8.2 \"State values\")",
                         st is not None and [r.payload for r in st.replies] == [b"ok"],
                         str(st and [r.payload for r in st.replies]))
            member = live.call(tool, f"{base}/@op/reset/a1", b"")
            malformed = live.call(tool, f"{base}/@op/reset/A1", b"")
            codes = [[(r.envelope or {}).get("code") for r in x.replies] for x in (member, malformed)]
            report.check(run, "a templated operation over its template: a well-formed member is not_found, "
                              "a key naming no member invalid_request (O1, O-4)",
                         codes == [["not_found"], ["invalid_request"]], str(codes))
            time.sleep(0.2)
            report.check(run, "a data subscriber up before the start received the first descriptor, "
                              "without a GET (§3.3, §8.2 step 3)",
                         owner.descriptor in descriptor_puts, f"{len(descriptor_puts)} puts")
            doc = json.loads(owner.descriptor)
            mine = [k for k in tokens if f"zk2/{system}/{svc}/" in k]
            report.check(run, "the templated state is claimed, not listed, and no member token appears",
                         doc["interfaces"][0]["unavailable"] == [] and not any("/@zk/member/" in k for k in mine),
                         f"unavailable {doc['interfaces'][0]['unavailable']}, tokens {mine}")
            report.check(run, "the owner holds an instance token and one interface token",
                         sorted(k.split("/@zk/")[1].split("/")[0] for k in mine) == ["alive", "instance"],
                         str(mine))
        finally:
            owner.close()
            lsub.undeclare()
            dsub.undeclare()
            tool.close()
    finally:
        r1.close()


class _StallProxy:
    """A TCP relay to a router whose router-to-client direction the runner
    can hold back (presence.md §6 step 3): held bytes wait in the relay,
    and flow again when released."""

    def __init__(self, upstream: str):
        import socket

        host, port = upstream.removeprefix("tcp/").rsplit(":", 1)
        self.upstream = (host, int(port))
        self.flowing = threading.Event()
        self.flowing.set()
        self.sock = socket.socket()
        self.sock.bind(("127.0.0.1", 0))
        self.sock.listen(8)
        self.endpoint = f"tcp/127.0.0.1:{self.sock.getsockname()[1]}"
        self._conns: list[Any] = []
        threading.Thread(target=self._accept, daemon=True).start()

    def _accept(self) -> None:
        import socket

        while True:
            try:
                c, _ = self.sock.accept()
            except OSError:
                return
            u = socket.create_connection(self.upstream)
            self._conns += [c, u]
            threading.Thread(target=self._pump, args=(c, u, None), daemon=True).start()
            threading.Thread(target=self._pump, args=(u, c, self.flowing), daemon=True).start()

    @staticmethod
    def _pump(src, dst, gate) -> None:
        try:
            while True:
                data = src.recv(65536)
                if not data:
                    break
                if gate is not None:
                    gate.wait()
                dst.sendall(data)
        except OSError:
            pass

    def close(self) -> None:
        self.flowing.set()
        for s in [self.sock, *self._conns]:
            try:
                s.close()
            except OSError:
                pass


def _acl_router(flow: str):
    """presence.md §6's R1: access control under ``allow``, denying
    ``liveliness_query`` on ``flow`` for ``zk2/*/*/@zk/alive/**``, for every
    subject. Returns (session, endpoint)."""
    import zenoh

    from .owner import free_loopback_port

    port = free_loopback_port()
    conf = zenoh.Config()
    conf.insert_json5("mode", json.dumps("router"))
    conf.insert_json5("listen/endpoints", json.dumps([f"tcp/127.0.0.1:{port}"]))
    conf.insert_json5("scouting/multicast/enabled", "false")
    conf.insert_json5("timestamping/enabled", "true")
    conf.insert_json5("access_control", json.dumps({
        "enabled": True,
        "default_permission": "allow",
        "rules": [{"id": "no-alive-reads", "messages": ["liveliness_query"], "flows": [flow],
                   "permission": "deny", "key_exprs": ["zk2/*/*/@zk/alive/**"]}],
        "subjects": [{"id": "everyone"}],
        "policies": [{"id": "deny-alive", "rules": ["no-alive-reads"], "subjects": ["everyone"]}],
    }))
    return zenoh.open(conf), f"tcp/127.0.0.1:{port}"


def run_python_presence_refused(report: Report) -> None:
    """presence.md §6 (0.8): a read refused by access control is complete
    and empty; a read that reached its timeout shows it. R1 is a zenoh-python
    router with the deny; the owner is zk2py's, a client of R1; the second
    tool reaches R1 through :class:`_StallProxy`. Then the control the
    scenario records as measured: the same deny on ``egress`` alone refuses
    nothing."""
    from . import live
    from .contract import load_contract
    from .owner import Owner as PyOwner

    system, svc = "py-site", "guarded"
    contract = load_contract(REPO / ECHO)
    for flow in ("ingress", "egress"):
        run = f"zk2py owner {system}/{svc} ← zk2py_echo.v1.toml (presence.md §6, deny on {flow})"
        r1, r1_endpoint = _acl_router(flow)
        try:
            owner = PyOwner(system, svc, [contract], connect=r1_endpoint)
            owner.start()
            try:
                # Step 1: R1's own session reads the interface token; "R1 has
                # no face of its own, so no rule applies to it".
                deadline = time.monotonic() + PRESENCE_WAIT_S
                own = live.list_presence(r1, f"zk2/{system}/{svc}/@zk/alive/**")
                while time.monotonic() < deadline and not own.alive:
                    time.sleep(0.05)
                    own = live.list_presence(r1, f"zk2/{system}/{svc}/@zk/alive/**")
                report.check(run, "step 1: R1's own session reads the interface token", len(own.alive) == 1,
                             own.reading)
                tool = live.open_client(r1_endpoint)
                try:
                    inst = live.list_presence(tool, "zk2/*/*/@zk/instance/*")
                    alive = live.list_presence(tool, "zk2/*/*/@zk/alive/**")
                finally:
                    tool.close()
                report.check(run, "step 2: the instance read holds the owner's instance token, no error reply",
                             [i["instance"] for i in inst.instances] == [owner.instance] and inst.complete
                             and not inst.errors, inst.reading)
                if flow == "ingress":
                    report.check(run, "step 2: the alive read is refused as absence: a final reply, no token, "
                                      "no error reply (§8.1, \"A refused read\")",
                                 alive.complete and alive.count == 0 and not alive.errors, alive.reading)
                else:
                    report.check(run, "control: the same deny on egress alone refuses nothing; the alive read "
                                      "holds the token (presence.md §6, measured)",
                                 alive.complete and len(alive.alive) == 1, alive.reading)
                    continue
                # Step 3 (0.9): the second tool reads zk2/*/*/@zk/instance/*
                # through a link the runner stalls, with the owner present.
                selector = "zk2/*/*/@zk/instance/*"
                proxy = _StallProxy(r1_endpoint)
                try:
                    tool2 = live.open_client(proxy.endpoint)
                    try:
                        flowing = live.list_presence(tool2, selector)
                        proxy.flowing.clear()
                        try:
                            stalled = live.list_presence(tool2, selector)
                        finally:
                            proxy.flowing.set()
                        time.sleep(0.3)
                        after = live.list_presence(tool2, selector)
                    finally:
                        tool2.close()
                finally:
                    proxy.close()
                report.check(run, "step 3: the open read holds the owner's instance token, no error reply",
                             [i["instance"] for i in flowing.instances] == [owner.instance] and flowing.complete
                             and not flowing.errors, flowing.reading)
                report.check(run, "step 3: the read held past its timeout ends with the error reply Timeout and "
                                  "no token; reported possibly incomplete, never absence",
                             not stalled.complete and stalled.errors == ["zenoh/string: Timeout"]
                             and stalled.count == 0 and stalled.reading.startswith("possibly incomplete"),
                             stalled.reading)
                report.check(run, "and once the link is released, the same read holds the token again",
                             [i["instance"] for i in after.instances] == [owner.instance] and after.complete,
                             after.reading)
            finally:
                owner.close()
        finally:
            r1.close()


def _tc_owner(system: str, endpoint: str, *, hold_s: float = 0.0, members: bool = False,
              busy: bool = False):
    """One ``tc`` host of operations.md (zk2py_tc.v1), a client of the
    router at ``endpoint``. Its ``set`` names the member its call binds and
    echoes; ``reset`` is one queryable per member (``members``), or one over
    the template whose handler names ``eth0`` unless the call binds another,
    or refuses every call ``busy``."""
    from .contract import load_contract
    from .owner import Owner as PyOwner

    def named(call) -> None:
        bound = call.bound.get("if")
        call.name(**{"if": bound[0] if bound else "eth0"})
        call.reply(call.payload)

    def refuse_busy(call) -> None:
        call.refuse("busy", "this host refuses every reset")

    handlers: dict[str, Any] = {"@op/interfaces/{if}/set": named}
    if busy:
        handlers["@op/interfaces/{if}/reset"] = refuse_busy
    elif not members:
        handlers["@op/interfaces/{if}/reset"] = named
    owner = PyOwner(system, "tc", [load_contract(REPO / TC)], connect=endpoint, handlers=handlers,
                    members={"@op/interfaces/{if}/reset": [{"if": "eth0"}, {"if": "eth1"}]} if members else None,
                    hold_s=hold_s)
    owner.start()
    return owner


def _wait_alive(session, selector: str, n: int) -> None:
    from . import live

    deadline = time.monotonic() + PRESENCE_WAIT_S * 3
    while time.monotonic() < deadline and len(live.list_presence(session, selector).alive) < n:
        time.sleep(0.05)


def run_python_o1(report: Report) -> None:
    """operations.md §1 (0.8): target and consolidation shown by behaviour.
    zk2py's owners and caller are clients of a router R1 of the runner's;
    the split-brain's second instance is a client of a second router R2,
    linked to R1."""
    import zenoh

    from . import live
    from .owner import free_loopback_port

    n, hold = 200, 0.5
    run = "zk2py owners h1/tc ← zk2py_tc.v1.toml (operations.md §1)"
    key = "zk2/h1/tc/zk2py_tc.v1/@op/interfaces/eth0/set"
    r1, r1_endpoint, _ = _r1()
    owners: list[Any] = []
    r2 = None
    try:
        caller = live.open_client(r1_endpoint)
        try:
            a = _tc_owner("h1", r1_endpoint, hold_s=hold)
            owners.append(a)
            _wait_alive(caller, "zk2/h1/tc/@zk/alive/**", 1)
            results = [live.call(caller, key, b"x", first=True) for _ in range(n)]
            firsts = [r.first_s for r in results if r.first_s is not None]
            values = sum(1 for r in results if [x.kind for x in r.replies] == ["value"])
            report.check(run, f"step 1: {n} calls, {n} executions, each returning on its first reply well "
                              f"before the server lets the query go ({hold} s)",
                         len(a.handled) == n and values == n and len(firsts) == n and max(firsts) < hold / 2
                         and all(r.done_s is None for r in results),
                         f"{len(a.handled)} executions, {values} values, first reply max "
                         f"{max(firsts or [0]):.3f} s")
            # The control: under Latest, the reply waits for completion.
            q: list[Any] = []
            done = threading.Event()
            t0 = time.monotonic()
            got: list[float] = []
            caller.get(key, zenoh.handlers.Callback(lambda rep: (q.append(rep), got.append(time.monotonic() - t0)),
                                                    done.set),
                       target=zenoh.QueryTarget.BEST_MATCHING, consolidation=zenoh.ConsolidationMode.LATEST,
                       payload=b"x", timeout=5.0)
            done.wait(10)
            report.check(run, "control: under Latest the same call's reply waits for the query to complete",
                         len(got) == 1 and got[0] >= hold * 0.9, f"reply after {got[0] if got else None} s")
            time.sleep(hold + 0.2)

            a.handled.clear()
            b = _tc_owner("h1", r1_endpoint)
            owners.append(b)
            _wait_alive(caller, "zk2/h1/tc/@zk/alive/**", 2)
            for _ in range(n):
                live.call(caller, key, b"x", first=True)
            time.sleep(hold + 0.2)
            report.check(run, f"step 2: a second instance on the same router: {n} executions between the two, "
                              f"never {2 * n} (BestMatching)",
                         len(a.handled) + len(b.handled) == n,
                         f"first {len(a.handled)}, second {len(b.handled)}")
            a.handled.clear()
            b.handled.clear()
            every = live._answers(caller, key, zenoh.QueryTarget.ALL, 5.0)
            got_all = list(every)
            time.sleep(hold + 0.2)
            report.check(run, "control: under target All, one call runs on both instances",
                         len(a.handled) + len(b.handled) == 2 and len(got_all) == 2,
                         f"first {len(a.handled)}, second {len(b.handled)}, {len(got_all)} replies")

            for o in owners:
                o.close()
            owners.clear()
            port = free_loopback_port()
            conf = zenoh.Config()
            conf.insert_json5("mode", json.dumps("router"))
            conf.insert_json5("listen/endpoints", json.dumps([f"tcp/127.0.0.1:{port}"]))
            conf.insert_json5("connect/endpoints", json.dumps([r1_endpoint]))
            conf.insert_json5("scouting/multicast/enabled", "false")
            conf.insert_json5("timestamping/enabled", "true")
            r2 = zenoh.open(conf)
            a = _tc_owner("h1", r1_endpoint)
            b = _tc_owner("h1", f"tcp/127.0.0.1:{port}")
            owners += [a, b]
            _wait_alive(caller, "zk2/h1/tc/@zk/alive/**", 2)
            both = [live.call(caller, key, b"x") for _ in range(n)]
            two = sum(1 for r in both if [x.kind for x in r.replies] == ["value", "value"])
            report.check(run, f"step 3: the second instance on another router: {2 * n} executions, one per side, "
                              "and the caller sees both replies of each call (None)",
                         len(a.handled) == n and len(b.handled) == n and two == n,
                         f"R1 side {len(a.handled)}, R2 side {len(b.handled)}, {two}/{n} calls with two values")
        finally:
            caller.close()
    finally:
        for o in owners:
            o.close()
        if r2 is not None:
            r2.close()
        r1.close()


def run_python_fanout(report: Report) -> None:
    """operations.md §2 (0.8), with zk2py's owners as h1, h2 and h3, and
    step 5's h1/scan, all clients of a router R1 of the runner's."""
    from . import live
    from .contract import load_contract
    from .owner import MemberRefused, Owner as PyOwner

    run = "zk2py owners h1..h3/tc, h1/scan ← zk2py_tc.v1, zk2py_scan.v1 (operations.md §2)"
    r1, r1_endpoint, _ = _r1()
    owners: list[Any] = []
    try:
        caller = live.open_client(r1_endpoint)
        try:
            h1 = _tc_owner("h1", r1_endpoint, members=True)
            h2 = _tc_owner("h2", r1_endpoint)
            h3 = _tc_owner("h3", r1_endpoint, busy=True)
            owners += [h1, h2, h3]
            refused_to_handler: list[str] = []

            def scan(call) -> None:
                call.name(port="p1")
                call.reply(b"open 22")
                call.reply(b"open 80")
                try:
                    call.name(port="p2")
                except MemberRefused as e:
                    refused_to_handler.append(str(e))
                    return
                call.reply(b"never")

            sc = PyOwner("h1", "scan", [load_contract(REPO / SCAN)], connect=r1_endpoint,
                         handlers={"@op/ports/{port}/scan": scan})
            sc.start()
            owners.append(sc)
            _wait_alive(caller, "zk2/*/tc/@zk/alive/**", 3)
            _wait_alive(caller, "zk2/h1/scan/@zk/alive/**", 1)

            def codes(res) -> list[str]:
                return sorted((r.envelope or {}).get("code", r.kind) for r in res.replies)

            # Step 1.
            one = live.call(caller, "zk2/h1/tc/zk2py_tc.v1/@op/interfaces/*/set", b"x", fanout=True)
            three = live.call(caller, "zk2/*/tc/zk2py_tc.v1/@op/interfaces/*/set", b"x", fanout=True)
            # 0.9: then ETH0, not canonical either; O2 comes first.
            eth0 = live.call(caller, "zk2/*/tc/zk2py_tc.v1/@op/interfaces/ETH0/set", b"x", fanout=True)
            report.check(run, "step 1: one, then three, then three fanout_forbidden refusals (ETH0 too: O2 "
                              "comes first, §5.1 \"The order of refusals\"), and 0 executions",
                         codes(one) == ["fanout_forbidden"] and codes(three) == ["fanout_forbidden"] * 3
                         and codes(eth0) == ["fanout_forbidden"] * 3 and not any(o.handled for o in owners),
                         f"{codes(one)} {codes(three)} {codes(eth0)}")
            # Step 2.
            diag = live.call(caller, "zk2/*/tc/zk2py_tc.v1/@op/diagnostics", b"x", fanout=True)
            report.check(run, "step 2: diagnostics with All + None: one reply per host, each on its own key",
                         sorted(r.key or "" for r in diag.replies)
                         == [f"zk2/h{i}/tc/zk2py_tc.v1/@op/diagnostics" for i in (1, 2, 3)],
                         str([(r.kind, r.key) for r in diag.replies]))
            # Step 3.
            reset = live.call(caller, "zk2/*/tc/zk2py_tc.v1/@op/interfaces/*/reset", b"x", fanout=True)
            vals = sorted(r.key for r in reset.replies if r.kind == "value")
            envs = [r.envelope["code"] for r in reset.replies if r.kind == "envelope"]
            report.check(run, "step 3: h1 two values (eth0, eth1), h2 one on the member it named, one busy "
                              "envelope, unattributed",
                         vals == ["zk2/h1/tc/zk2py_tc.v1/@op/interfaces/eth0/reset",
                                  "zk2/h1/tc/zk2py_tc.v1/@op/interfaces/eth1/reset",
                                  "zk2/h2/tc/zk2py_tc.v1/@op/interfaces/eth0/reset"]
                         and envs == ["busy"] and all(r.key is None for r in reset.replies if r.kind != "value"),
                         f"values {vals}, envelopes {envs}")
            pres = live.list_presence(caller, "zk2/*/tc/@zk/alive/**")
            silent = sorted({f"{a['system']}/{a['service']}" for a in pres.alive}
                            - {k.split("/")[1] + "/tc" for k in vals})
            report.check(run, "step 3: presence shows h3 holding the token and sending no value "
                              "(refused or silent, which the caller cannot tell)",
                         pres.complete and silent == ["h3/tc"], f"{pres.reading}; no value from {silent}")
            # Step 4.
            before = [(len(o.calls), len(o.handled)) for o in owners]
            bad = live.call(caller, "zk2/*/tc/zk2py_tc.v1/@op/interfaces/ETH0/reset", b"x", fanout=True)
            after = [(len(o.calls), len(o.handled)) for o in owners]
            report.check(run, "step 4: ETH0, not a canonical slug: two invalid_request envelopes (h2, h3), no "
                              "handler runs, h1's member queryables not selected (§5.1, 0.8)",
                         codes(bad) == ["invalid_request", "invalid_request"]
                         and [x[1] for x in after] == [x[1] for x in before]
                         and after[0][0] == before[0][0]
                         and [a[0] - b[0] for a, b in zip(after[1:3], before[1:3])] == [1, 1],
                         f"{codes(bad)}; queryable hits {[a[0] - b[0] for a, b in zip(after, before)]}, "
                         f"handler runs {[a[1] - b[1] for a, b in zip(after, before)]}")
            # Step 5.
            many = live.call(caller, "zk2/h1/scan/zk2py_scan.v1/@op/ports/*/scan", b"", fanout=True)
            report.check(run, "step 5: replies = many over a template: two values, both on ports/p1/scan, none "
                              "elsewhere; naming p2 is refused to the handler (§5.1, 0.8)",
                         [(r.kind, r.key, r.payload) for r in many.replies]
                         == [("value", "zk2/h1/scan/zk2py_scan.v1/@op/ports/p1/scan", b"open 22"),
                             ("value", "zk2/h1/scan/zk2py_scan.v1/@op/ports/p1/scan", b"open 80")]
                         and len(refused_to_handler) == 1,
                         f"{[(r.kind, r.key, r.payload) for r in many.replies]}; refused to the handler: "
                         f"{refused_to_handler}")
            # Step 6 (0.9): the same operation, served over the template by a
            # handler that names no member and sends nothing.
            quiet = PyOwner("h9", "scan", [load_contract(REPO / SCAN)], connect=r1_endpoint,
                            handlers={"@op/ports/{port}/scan": lambda call: None})
            quiet.start()
            owners.append(quiet)
            _wait_alive(caller, "zk2/h9/scan/@zk/alive/**", 1)
            nothing = live.call(caller, "zk2/h9/scan/zk2py_scan.v1/@op/ports/*/scan", b"", fanout=True)
            report.check(run, "step 6: a handler that names no member and sends nothing: no value and no "
                              "envelope, the handler having run (§5.1, \"Sending nothing needs no member\")",
                         nothing.silent and nothing.done_s is not None and len(quiet.handled) == 1,
                         f"{len(nothing.replies)} replies, completed {nothing.done_s is not None}, "
                         f"handler runs {len(quiet.handled)}")
            # §5.1 (0.9) "The order of refusals", on calibrate: optional,
            # gated on a capability no tc host holds, so implied absent.
            wild = live.call(caller, "zk2/*/tc/zk2py_tc.v1/@op/interfaces/ETH0/calibrate", b"x", fanout=True)
            gone = live.call(caller, "zk2/h2/tc/zk2py_tc.v1/@op/interfaces/ETH0/calibrate", b"x")
            causes = [(r.envelope or {}).get("cause") for r in gone.replies]
            report.check(run, "the order of refusals (§5.1, 0.9): a wildcard call to an operation not exposed is "
                              "fanout_forbidden first; a concrete one is unavailable (capability) before its "
                              "ETH0 names no member",
                         codes(wild) == ["fanout_forbidden"] * 3 and codes(gone) == ["unavailable"]
                         and causes == ["capability"], f"{codes(wild)} {codes(gone)} {causes}")
            # operations.md §3 step 3, beside it: the concrete call.
            concrete = live.call(caller, "zk2/h2/tc/zk2py_tc.v1/@op/interfaces/ETH0/set", b"x")
            report.check(run, "and §3 step 3: a concrete call to interfaces/ETH0/set is invalid_request, not "
                              "not_found", codes(concrete) == ["invalid_request"], str(codes(concrete)))
        finally:
            caller.close()
    finally:
        for o in owners:
            o.close()
        r1.close()


def run_python_tool_rules(report: Report) -> None:
    """The rules 0.10 to 0.13 state for a tool, where the bus shows them:
    - S4 needs the admin space (§4.2): off, the check is unobservable; on,
      read-only, it reads 0.11's two selectors, and a router with nothing
      under the storages selector runs no storage. An answer counts only
      when its replier id is the zid its key names, a verified router: one
      the session is connected to, or one a verified router's own document
      lists as a router (0.12, 0.13). security.md §3 steps 1-2: a client
      answering on R1's own key, with the admin space off and on, stays
      unverified. Step 4: a far router is verified through R1's document,
      and a client answering on its own key is not.
      A storage manager played from a client is unverified too; trusted by
      the operator, one on ``telemetry/**`` is clean and one on ``zk2/**``
      breaks S4;
    - a fault read from presence shapes holds in two reads a grace apart
      (§8.1): a shape gone by the second read passes, one still there is a
      fault;
    - revisions carry no order on the bus (§9.8): two owners serve two
      revisions side by side, and a tool orders them by the minor their
      descriptors state, or classifies both ways."""
    from . import bundle, live
    from .compat import classify_pair
    from .contract import load_contract
    from .owner import Owner as PyOwner

    run = "zk2py tool rules (0.10 to 0.13): S4's admin space, presence shapes, revisions on the bus"
    # S4 (§4.2, 0.10 to 0.12; Appendix B), and security.md §3 steps 1-2.
    import zenoh

    def spoof(session, key: str, doc: dict[str, Any]):
        """A client's queryable answering ``key`` with ``doc``: the spoof."""
        return session.declare_queryable(key, zenoh.handlers.Callback(
            lambda q: q.reply(key, json.dumps(doc), encoding="application/json")))

    for admin in (False, True):
        r, endpoint, zid = _r1(adminspace=admin)
        try:
            tool = live.open_client(endpoint)
            s = live.open_client(endpoint)
            try:
                plain = live.check_s4(tool)
                if admin:
                    report.check(run, "S4, R1's admin space on, read-only: R1 answers under its own replier id, "
                                      "verified; nothing under …/storage_manager/storages/**; plugins agree: clean",
                                 plain.verdict == "clean" and plain.routers == [zid] and not plain.unverified
                                 and not plain.storages and plain.plugins == {zid: None}, plain.detail)
                else:
                    report.check(run, "S4, R1's admin space off, zenoh 1.10.1's default: no answer, unobservable, "
                                      "never clean", plain.verdict == "unobservable" and not plain.unverified
                                 and not plain.routers, plain.detail)
                # security.md §3: S, a client and no router, answers on R1's
                # own key.
                own = f"@/{zid}/router"
                q = spoof(s, own, {"plugins": None})
                time.sleep(0.3)
                try:
                    answers = [(a.key, a.replier) for a in live._answers(tool, live.S4_ROUTERS,
                                                                           zenoh.QueryTarget.ALL, 1.0) if a.ok]
                    spoofed = live.check_s4(tool)
                finally:
                    q.undeclare()
                s_zid = str(s.zid())
                if admin:
                    report.check(run, "security.md §3 step 2: R1 answers too, under its own replier id, and is "
                                      "verified; S's answer on R1's key carries S's replier id, unverified, so the "
                                      "check is not clean either",
                                 sorted(answers) == sorted([(own, zid), (own, s_zid)])
                                 and spoofed.routers == [zid]
                                 and spoofed.unverified == [(own, s_zid, "a replier other than its key's router")]
                                 and spoofed.verdict == "unobservable",
                                 f"answers {answers}; {spoofed.verdict}: {spoofed.detail}")
                else:
                    report.check(run, "security.md §3 step 1: S's answer arrives on R1's own key, the only answer, "
                                      "its replier id S's zid, not R1's; held unverified: unobservable, never clean",
                                 answers == [(own, s_zid)] and s_zid != zid and not spoofed.routers
                                 and spoofed.unverified == [(own, s_zid, "a replier other than its key's router")]
                                 and spoofed.verdict == "unobservable",
                                 f"answers {answers}, R1 {zid}; {spoofed.verdict}: {spoofed.detail}")
                if admin:
                    # A storage manager played from a raw session, as 0.12
                    # says the reference's own live test does: a spoof, so
                    # unverified unless the operator trusts every answer.
                    fake = "1234567890abcdef"
                    base = f"@/{fake}/router/status/plugins/storage_manager/storages"
                    held = [spoof(s, f"@/{fake}/router", {"plugins": {"storage_manager": {}}}),
                            spoof(s, f"{base}/telemetry", {"key_expr": "telemetry/**", "volume": "memory"})]
                    try:
                        time.sleep(0.3)
                        quiet_trusted = live.check_s4(tool, trust=True)
                        held.append(spoof(s, f"{base}/all", {"key_expr": "zk2/**", "volume": "memory"}))
                        time.sleep(0.3)
                        loud = live.check_s4(tool)
                        loud_trusted = live.check_s4(tool, trust=True)
                    finally:
                        for h in held:
                            h.undeclare()
                    report.check(run, "a storage on zk2/** played from a client session: its answers are "
                                      "unverified, so they never break S4 and never let it be clean (§4.2, 0.12)",
                                 loud.verdict == "unobservable" and not loud.storages
                                 and len(loud.unverified) == 3, f"{loud.verdict}: {loud.detail}")
                    report.check(run, "trusted by the operator (0.12): the storage on telemetry/** leaves S4 "
                                      "clean, and the one on zk2/** breaks it",
                                 quiet_trusted.verdict == "clean"
                                 and [x[2] for x in quiet_trusted.storages] == ["telemetry/**"]
                                 and loud_trusted.verdict == "broken"
                                 and [x[2] for x in loud_trusted.storages if x[3]] == ["zk2/**"],
                                 f"{quiet_trusted.verdict} | {loud_trusted.verdict}: {loud_trusted.detail}")
            finally:
                s.close()
                tool.close()
        finally:
            r.close()

    # security.md §3 step 4 (0.13): a far router. R2 links to R1, both admin
    # spaces on, read-only; the tool is still a client of R1. Routers are
    # verified outward through R1's own document (§4.2), which lists R2 as
    # a `router` session. Then S answers @/<S's zid>/router under its own
    # replier id: R1 lists S as a `client`, so S stays unverified. A peer
    # tool connected to both routers verifies them directly, a cross-check.
    from .owner import free_loopback_port

    ra, ep_a, za = _r1(adminspace=True)
    port = free_loopback_port()
    conf = zenoh.Config()
    conf.insert_json5("mode", json.dumps("router"))
    conf.insert_json5("listen/endpoints", json.dumps([f"tcp/127.0.0.1:{port}"]))
    conf.insert_json5("connect/endpoints", json.dumps([ep_a]))
    conf.insert_json5("scouting/multicast/enabled", "false")
    conf.insert_json5("adminspace", json.dumps({"enabled": True, "permissions": {"read": True, "write": False}}))
    rb = zenoh.open(conf)
    zb = str(rb.zid())
    try:
        client = live.open_client(ep_a)
        s = live.open_client(ep_a)
        pconf = zenoh.Config()
        pconf.insert_json5("mode", json.dumps("peer"))
        pconf.insert_json5("connect/endpoints", json.dumps([ep_a, f"tcp/127.0.0.1:{port}"]))
        pconf.insert_json5("scouting/multicast/enabled", "false")
        peer = zenoh.open(pconf)
        try:
            deadline = time.monotonic() + 5.0
            far = live.check_s4(client)
            while time.monotonic() < deadline and len(far.routers) + len(far.unverified) < 2:
                time.sleep(0.2)
                far = live.check_s4(client)
            report.check(run, "security.md §3 step 4: a client tool on R1 verifies R2 through R1's own document, "
                              "which lists it as a router; R2's answer carries its own replier id: both "
                              "verified, clean (§4.2, 0.13)",
                         sorted(far.routers) == sorted([za, zb]) and not far.unverified
                         and far.verdict == "clean", f"{far.verdict}: {far.detail}")
            s_zid = str(s.zid())
            own = f"@/{s_zid}/router"
            q = spoof(s, own, {"plugins": None, "sessions": []})
            time.sleep(0.3)
            try:
                with_s = live.check_s4(client)
                r1_doc = next((json.loads(a.payload) for a in live._answers(
                    client, f"@/{za}/router", zenoh.QueryTarget.ALL, 1.0) if a.ok and a.replier == za), {})
            finally:
                q.undeclare()
            listed = {x.get("peer"): x.get("whatami") for x in r1_doc.get("sessions", [])}
            report.check(run, "step 4: S answers on its own key under its own replier id; R1 lists S as a client, "
                              "so S stays unverified (no verified router lists it), and the check is not clean",
                         listed.get(s_zid) == "client" and listed.get(zb) == "router"
                         and with_s.unverified == [(own, s_zid, "no verified router lists it")]
                         and sorted(with_s.routers) == sorted([za, zb]) and with_s.verdict == "unobservable",
                         f"R1 lists S as {listed.get(s_zid)!r}, R2 as {listed.get(zb)!r}; "
                         f"{with_s.verdict}: {with_s.detail}")
            both = live.check_s4(peer)
            report.check(run, "cross-check: a peer tool connected to both routers verifies them directly, and "
                              "reads the same verdict, clean",
                         sorted(both.routers) == sorted([za, zb]) and not both.unverified
                         and both.verdict == "clean", f"{both.verdict}: {both.detail}")
        finally:
            peer.close()
            s.close()
            client.close()
    finally:
        rb.close()
        ra.close()

    r1, r1_endpoint, _ = _r1()
    owners: list[Any] = []
    try:
        tool = live.open_client(r1_endpoint)
        helper = live.open_client(r1_endpoint)
        try:
            # Presence shapes (§8.1): an owner, then an interface token its
            # descriptor does not list, under its instance.
            system, svc = "py-site", "shapes"
            owner = PyOwner(system, svc, [load_contract(REPO / ECHO)], connect=r1_endpoint)
            owner.start()
            owners.append(owner)
            sel = f"zk2/{system}/{svc}/@zk/**"
            _wait_alive(tool, sel, 1)
            steady, passing = live.presence_faults(tool, sel)
            report.check(run, "presence shapes, steady: no fault and nothing passing", not steady and not passing,
                         f"faults {steady}, passing {passing}")
            stray = f"zk2/{system}/{svc}/@zk/alive/zk2py_probe.v1/{owner.instance}/{'0' * 16}"
            token = helper.liveliness().declare_token(stray)
            time.sleep(0.2)
            timer = threading.Timer(live.SHAPE_GRACE_S / 2, token.undeclare)
            timer.start()
            faults, passing = live.presence_faults(tool, sel)
            timer.join()
            report.check(run, "a token the descriptor does not list, gone within the grace: seen once, so it "
                              "passes, never a fault (§8.1, 0.10)",
                         not faults and passing == {(owner.instance_key, "unlisted", "zk2py_probe.v1")},
                         f"faults {faults}, passing {passing}")
            token = helper.liveliness().declare_token(stray)
            time.sleep(0.2)
            faults, passing = live.presence_faults(tool, sel)
            token.undeclare()
            report.check(run, "the same token still there a grace later: seen in both reads, a fault",
                         faults == {(owner.instance_key, "unlisted", "zk2py_probe.v1")} and not passing,
                         f"faults {faults}, passing {passing}")

            # Revisions on the bus (§9.8): zk2py_bringup.v1 at minor 0 and 1.
            a = PyOwner(system, "rev-a", [load_contract(REPO / BRINGUP)], connect=r1_endpoint)
            b = PyOwner(system, "rev-b", [load_contract(REPO / BRINGUP_V1_1)], connect=r1_endpoint)
            for o in (a, b):
                o.start()
                owners.append(o)
            seen = f"zk2/{system}/*/@zk/alive/zk2py_bringup.v1/**"
            _wait_alive(tool, seen, 2)
            pres = live.list_presence(tool, seen)
            revs = []
            for tok in sorted(pres.alive, key=lambda t: t["service"]):
                d = live.get_descriptor(tool, f"zk2/{tok['system']}/{tok['service']}/@zk/instance/{tok['instance']}")
                doc = json.loads(d[0].payload) if len(d) == 1 and d[0].ok else {}
                fp = live.fingerprint_of(doc, "zk2py_bringup.v1", tok["fp16"])
                got = live.retrieve_bundle(tool, "zk2py_bringup.v1", fp) if fp else None
                minor = next((e.get("minor") for e in doc.get("interfaces", [])
                              if e.get("iface") == "zk2py_bringup.v1"), None)
                if got is not None and got.verified is not None:
                    revs.append((tok["service"], minor, bundle.revision(got.verified)))
            if report.check(run, "two providers serve two revisions of zk2py_bringup.v1, each bundle retrieved "
                                 "by its descriptor's fingerprint", len(revs) == 2
                            and len({id(r[2]) for r in revs}) == 2,
                            str([(s, m) for s, m, _ in revs])):
                (_, ma, ra), (_, mb, rb) = revs
                ordered = classify_pair(ra, rb, ma, mb)
                both = classify_pair(ra, rb)
                report.check(run, "ordered by the minor the descriptors state (0 → 1): compatible (§9.8, 0.10)",
                             ordered == ("compatible", "ordered by minor 0 → 1"), str(ordered))
                report.check(run, "without that order, both ways: compatible one way, breaking the other, so "
                                  "undecided", both[0] == "undecided", str(both))
        finally:
            helper.close()
            tool.close()
    finally:
        for o in owners:
            o.close()
        r1.close()


def run_python_016(report: Report) -> None:
    """The rules 0.16 states, where zk2py's bus shows them (no access
    control; R1's admin space on, read-only):
    - §4.2 "A tool's S1 check": an owner in client mode under R1 is judged
      by its stamp against meta.zid (clean); an owner whose own session is
      a router, linked to R1, is unobservable, neither clean nor a finding;
    - Appendix B: without the replier id, every admin answer is unverified,
      and S4 and S1 are unobservable, never clean;
    - §4.4: an owner's tokenless set (U22) is honoured, and refused for
      archive.v1;
    - §2.6: the retention is the bound the consumer applies on replay, here
      against a stand-in storage that ignores `_time`, as the memory backend
      does (spike S5)."""
    import zenoh

    from . import live
    from .contract import load_contract
    from .descriptor import check_descriptor
    from .owner import Owner as PyOwner, OwnerRefused

    run = "zk2py tool rules (0.16, 0.17): S1 from a tool, Appendix B, §4.4's tokenless archive, §2.6's replay bound"
    r1, r1_endpoint, r1_zid = _r1(adminspace=True)
    owners: list[Any] = []
    try:
        tool = live.open_client(r1_endpoint)
        try:
            echo = load_contract(REPO / ECHO)
            client_owner = PyOwner("py-site", "s1-client", [echo], connect=r1_endpoint)
            router_owner = PyOwner("py-site", "s1-router", [echo], router_connect=r1_endpoint)
            for o in (client_owner, router_owner):
                o.start()
                owners.append(o)
            _wait_alive(tool, "zk2/py-site/s1-$*/@zk/alive/**", 2)
            readings = {}
            for name, o in (("client", client_owner), ("router", router_owner)):
                d = live.get_descriptor(tool, o.instance_key)
                doc = json.loads(d[0].payload) if len(d) == 1 and d[0].ok else {}
                st = live.get_state(tool, f"zk2/py-site/{o.service}/zk2py_echo.v1/state/health")
                stamp = st.replies[0].stamp_id if len(st.replies) == 1 else None
                readings[name] = (doc, stamp, live.s1_check(tool, doc, stamp))
            doc, stamp, (verdict, why) = readings["client"]
            report.check(run, "S1 from a tool (§4.2, 0.16): an owner in client mode under R1, which the tool "
                              "verified, is judged by its stamp against meta.zid: clean",
                         verdict == "clean", f"{verdict}: {why}; stamp {stamp}, meta.zid {doc.get('meta')}")
            doc, stamp, (verdict, why) = readings["router"]
            report.check(run, "S1 from a tool: an owner whose own session is a router, linked to R1 (R1's document "
                              "lists it as a router), is unobservable, neither clean nor a finding",
                         verdict == "unobservable" and "own router" in why
                         and live.attribute_stamp(stamp, doc) == "owner",
                         f"{verdict}: {why}; the stamp alone reads {live.attribute_stamp(stamp, doc)!r}")
            # §4.2 (0.17): a foreign stamp. The owner's clock is another
            # session's HLC, so its state carries that session's id.
            other = live.open_client(r1_endpoint)
            try:
                foreign = PyOwner("py-site", "s1-foreign", [echo], connect=r1_endpoint)
                foreign.start()
                owners.append(foreign)
                foreign.clock = other.new_timestamp
                foreign.set_state("zk2/py-site/s1-foreign/zk2py_echo.v1/state/health", b"stamped-elsewhere")
                foreign.clock = None
                time.sleep(0.3)
                d = live.get_descriptor(tool, foreign.instance_key)
                fdoc = json.loads(d[0].payload) if len(d) == 1 and d[0].ok else {}
                st = live.get_state(tool, "zk2/py-site/s1-foreign/zk2py_echo.v1/state/health")
                fstamp = st.replies[0].stamp_id if len(st.replies) == 1 else None
                f_admin = live.s1_check(tool, fdoc, fstamp)
                other_zid = str(other.zid())
            finally:
                other.close()
            # Appendix B (0.16): a binding without the replier id, so no
            # router is verified.
            live.READ_REPLIER = False
            try:
                s4 = live.check_s4(tool)
                cdoc, cstamp, _ = readings["client"]
                s1 = live.s1_check(tool, cdoc, cstamp)
                f_blind = live.s1_check(tool, fdoc, fstamp)
            finally:
                live.READ_REPLIER = True
            report.check(run, "Appendix B (0.16): without Reply.replier_id every admin answer is unverified (no "
                              "replier id): S4 is unobservable, and so is S1 for an owner's own stamp (§4.2, 0.17)",
                         s4.verdict == "unobservable" and s4.unverified
                         and all(u[2] == "no replier id" for u in s4.unverified) and s1[0] == "unobservable",
                         f"S4 {s4.verdict} ({len(s4.unverified)} unverified); S1 {s1[0]}: {s1[1]}")
            report.check(run, "§4.2 (0.17): a foreign stamp is a finding whatever else the tool read: with routers "
                              "verified, and with none verified",
                         fstamp is not None and live._zid_value(fstamp) == live._zid_value(other_zid)
                         and f_admin[0] == "finding" and f_blind[0] == "finding",
                         f"stamp {fstamp} (another session's {other_zid}); verified: {f_admin}; none: {f_blind}")
            # §4.4 and U22: the tokenless set.
            tokenless = PyOwner("py-site", "quiet", [echo], connect=r1_endpoint, tokenless={"zk2py_echo.v1"})
            tokenless.start()
            owners.append(tokenless)
            time.sleep(0.5)
            pres = live.list_presence(tool, "zk2/py-site/quiet/@zk/**")
            d = live.get_descriptor(tool, tokenless.instance_key)
            tdoc = json.loads(d[0].payload) if len(d) == 1 and d[0].ok else {}
            report.check(run, "U22: an interface in the owner's tokenless set has no interface token, and the "
                              "descriptor marks it \"token\": false, with no D code",
                         len(pres.instances) == 1 and not pres.alive
                         and [e.get("token") for e in tdoc.get("interfaces", [])] == [False]
                         and check_descriptor(d[0].payload, [echo]) == [],
                         f"{pres.reading}; token {[e.get('token') for e in tdoc.get('interfaces', [])]}")
            archive = load_contract(REPO / ARCHIVE_STANDIN)
            control = PyOwner("py-site", "archive", [archive], connect=r1_endpoint)
            control.start()
            owners.append(control)
            time.sleep(0.5)
            held = live.list_presence(tool, "zk2/py-site/archive/@zk/alive/archive.v1/**")
            control.close()
            owners.remove(control)
            time.sleep(0.5)
            refused = PyOwner("py-site", "archive", [archive], connect=r1_endpoint, tokenless={"archive.v1"})
            try:
                refused.start()
                outcome = "started"
                owners.append(refused)
            except OwnerRefused as e:
                outcome = f"refused: {e}"
            time.sleep(0.5)
            after = live.list_presence(tool, "zk2/py-site/archive/@zk/**")
            report.check(run, "§4.4 (0.16): an archive holds its archive.v1 token, and an owner configured with "
                              "archive.v1 in its tokenless set refuses to start, declaring nothing",
                         len(held.alive) == 1 and outcome.startswith("refused") and after.count == 0
                         and after.complete,
                         f"control: {len(held.alive)} archive.v1 token; {outcome}; after: {after.reading}")
        finally:
            tool.close()
        # §2.6 (0.16): the replay bound is the consumer's. A stand-in for a
        # union storage answers every occurrence, ignoring `_time`.
        helper = live.open_client(r1_endpoint)
        consumer = live.open_client(r1_endpoint)
        try:
            now = int(time.time() * 1000)
            base = "zk2/py-site/sensor/zk2py_ev.v1/events/alarm"
            keys = [f"{base}/{live.new_ulid(now - k * 5000)}" for k in range(1, 501)] + \
                   [f"{base}/{live.new_ulid(now - 3_660_000 - k * 5000)}" for k in range(1, 501)]

            def answer(q) -> None:
                for k in keys:
                    q.reply(k, b"occurrence")

            q = helper.declare_queryable("zk2/py-site/sensor/zk2py_ev.v1/events/**", zenoh.handlers.Callback(answer),
                                         complete=False)
            time.sleep(0.3)
            try:
                kept, dropped = live.replay_events(consumer, "zk2/*/*/*/events/**", 3600, now_ms=now)
            finally:
                q.undeclare()
            report.check(run, "§2.6 (0.16): retention is the consumer's bound on replay: against a storage that "
                              "prunes and filters nothing, the consumer keeps the 500 occurrences within 1 h by "
                              "their ULID's time (state.md §8)",
                         sorted(kept) == sorted(keys[:500]) and sorted(dropped) == sorted(keys[500:]),
                         f"kept {len(kept)}, dropped {len(dropped)} of {len(keys)}")
        finally:
            helper.close()
            consumer.close()
    finally:
        for o in owners:
            o.close()
        r1.close()


ORDER = "impl/python/interop/zk2py_order.v1.toml"
#: Core 0.22: one major of a profile per contract (E002), so the uses that
#: test 0.20's order are split between two contracts.
ORDER_B = "impl/python/interop/zk2py_order_b.v1.toml"
SYSINFO = "impl/python/interop/zk2py_sysinfo.v1.toml"
TRACKER = "impl/python/interop/zk2py_tracker.v1.toml"
#: Core §3.3 (0.20): zk2py_order.v1's and zk2py_order_b.v1's uses and hostid.v1, "sorted as §9.5
#: sorts a contract's uses: by name as a string, then by major as a number".
#: A bytewise sort would give a.b.v1, a.v1, hostid.v1, views.v10, views.v2.
ORDER_PROFILES = ["a.v1", "a.b.v1", "hostid.v1", "views.v2", "views.v10"]


def _presence(session, selector: str, instances: int, alive: int):
    """Presence within the conformance wait (§8.1), at least the counts
    asked for."""
    from . import live

    deadline = time.monotonic() + PRESENCE_WAIT_S
    pres = live.list_presence(session, selector)
    while time.monotonic() < deadline and not (len(pres.instances) >= instances and len(pres.alive) >= alive):
        time.sleep(0.05)
        pres = live.list_presence(session, selector)
    return pres


def _doc(session, instance_key: str) -> tuple[dict[str, Any], bytes]:
    from . import live

    d = live.get_descriptor(session, instance_key)
    if len(d) == 1 and d[0].ok:
        return json.loads(d[0].payload), d[0].payload
    return {}, b""


def run_python_hostid(report: Report, exe: Path) -> None:
    """hostid.v1 (profile text 0.2) and core R1 (0.20) across the two
    implementations, through a router R1 of the runner's:
    - the owner example minted (``@hostid.v1/echo`` over a temporary root
      holding M1), seen by zk2py: the system h-bbd1aa1db10b, the descriptor
      listing ``hostid.v1`` and ``meta.host``, its ``profiles`` in §3.3's
      order, no machine id on the bus (§2.11), and §2.12's question;
    - a zk2py consumer minted over the same root, so on the same system
      (bindings.md §5 the other way round): it binds ``self.system/echo``
      and ``self.system/*``, its descriptor lists them resolved (R3), and
      it reads the Rust owner's state and calls its operation through them;
    - over a root with no machine id, the shared file (§2.5) created by one
      implementation and read by the other, both ways: one system. The two
      at one minted address are §2.12's finding, and ``no`` once one
      leaves;
    - bindings.md §5 itself, with zk2py's detectors and trackers."""
    import re
    import shutil
    import tempfile

    from . import hostid, live
    from .contract import load_contract
    from .descriptor import check_descriptor
    from .hostid_scenarios import M1, SYSTEM, make_root, temps
    from .owner import Owner as PyOwner, resolve_providers

    base_dir = tempfile.mkdtemp(prefix="zk2py-hostid-")
    r1, ep, _ = _r1()
    rusts: list[Owner] = []
    owners: list[Any] = []
    tool = live.open_client(ep)
    want = SYSTEM[M1]
    SHARED_PATH = hostid.SHARED
    echo, needs, order, order_b = (load_contract(REPO / p) for p in (ECHO, NEEDS, ORDER, ORDER_B))
    try:
        # -- the owner example minted, seen by zk2py ---------------------
        run = "hostid.v1 0.2: the owner example minted over a root holding M1"
        root = make_root(base_dir, {"etc/machine-id": M1 + "\n"})
        rust = Owner(exe, "@hostid.v1/echo", [REPO / ECHO, REPO / ORDER, REPO / ORDER_B], connect=ep,
                     hostid_root=root)
        rusts.append(rust)
        ready = rust.wait_for("ready ", 120)
        if not report.check(run, f"it starts, and its instance key names the minted system {want} (§2.2, §2.7)",
                            ready is not None and ready.startswith(f"zk2/{want}/echo/@zk/instance/"),
                            f"ready {ready!r}; stderr {rust.stderr[-2:]}"):
            return
        pres = _presence(tool, f"zk2/{want}/echo/@zk/**", 1, 3)
        report.check(run, "presence through R1 at the minted address: one instance token, three interface "
                          "tokens",
                     len(pres.instances) == 1 and len(pres.alive) == 3 and pres.complete, pres.reading)
        rdoc, rraw = _doc(tool, ready)
        report.check(run, "its descriptor has no D code against its three contracts",
                     bool(rdoc) and check_descriptor(rraw, [echo, order, order_b]) == [], f"{len(rraw)} bytes")
        meta = rdoc.get("meta") or {}
        report.check(run, "its descriptor names the minted service and lists hostid.v1, with meta.host and "
                          "meta.zid (§2.8, §2.13)",
                     rdoc.get("service") == f"{want}/echo" and "hostid.v1" in (rdoc.get("profiles") or [])
                     and isinstance(meta.get("host"), str) and bool(meta.get("host")) and bool(meta.get("zid")),
                     f"service {rdoc.get('service')}, profiles {rdoc.get('profiles')}, meta {meta}")
        report.check(run, "its profiles: the union of zk2py_order.v1's and zk2py_order_b.v1's uses (one "
                          "major each, 0.22) and hostid.v1, in §3.3's order (0.20)",
                     rdoc.get("profiles") == ORDER_PROFILES, str(rdoc.get("profiles")))
        uuid = f"{M1[:8]}-{M1[8:12]}-{M1[12:16]}-{M1[16:20]}-{M1[20:]}"
        report.check(run, "no machine id on the bus (§2.11): M1 is in its descriptor in no spelling",
                     bool(rraw) and not any(s in rraw.lower() for s in (M1.encode(), uuid.encode())),
                     f"{len(rraw)} bytes read")
        answer, why, reads = live.hostid_collision(tool, f"{want}/echo")
        firsts = [first for _, rows in reads for _, _, first in rows]
        report.check(run, "§2.12 on its address: §5's first question is yes for it (its contracts' uses, "
                          "retrieved by fingerprint, do not list hostid.v1), and the answer is no",
                     answer == "no" and firsts == ["yes", "yes"], f"{answer}: {why}; first {firsts}")

        # -- a zk2py consumer on the same system --------------------------
        run = "core R1 0.20: a zk2py consumer minted over the same root, bound to self.system"
        consumer = PyOwner("@hostid.v1", "needs", [needs, order, order_b], connect=ep, hostid=hostid.Runtime(root),
                           bindings={"upstream": ["self.system/echo"], "peer": ["self.system/*"]},
                           capabilities={"cal"})
        consumer.start()
        owners.append(consumer)
        report.check(run, "a minted Rust owner and a minted zk2py owner over one root holding one machine id "
                          f"agree on the system: {want}",
                     consumer.system == want and ready.split("/")[1] == want,
                     f"zk2py {consumer.system}, Rust {ready.split('/')[1]}")
        _presence(tool, f"zk2/{want}/needs/@zk/**", 1, 3)
        cdoc, craw = _doc(tool, consumer.instance_key)
        report.check(run, "its descriptor has no D code against its three contracts",
                     bool(cdoc) and check_descriptor(craw, [needs, order, order_b]) == [], f"{len(craw)} bytes")
        bound = {r.get("role"): r.get("bindings") for r in cdoc.get("requires") or []}
        report.check(run, "its descriptor lists the self.system providers resolved (R3, 0.20)",
                     bound == {"upstream": [f"{want}/echo"], "peer": [f"{want}/*"]}, str(bound))
        report.check(run, "its profiles: the same five, in §3.3's order (0.20)",
                     cdoc.get("profiles") == ORDER_PROFILES, str(cdoc.get("profiles")))
        chost = (cdoc.get("meta") or {}).get("host")
        report.check(run, "both state the host they share as meta.host, for display (§2.13)",
                     chost is not None and chost == meta.get("host"),
                     f"zk2py {chost!r}, Rust {meta.get('host')!r}")
        peer = consumer.role_keys("peer", "zk2py_echo.v1", "state/health")
        st = live.get_state(consumer.session, peer[0])
        who = [live.attribute_stamp(r.stamp_id, rdoc) for r in st.replies]
        report.check(run, f"through peer's resolved binding {want}/*, a state GET (S4) answers the Rust owner's "
                          "value alone, attributed to it by meta.zid",
                     [(r.key, r.payload) for r in st.replies]
                     == [(f"zk2/{want}/echo/zk2py_echo.v1/state/health", b"ok")] and who == ["owner"],
                     f"{peer} → {[(r.key, r.payload) for r in st.replies]} {who}")
        up = consumer.role_keys("upstream", "zk2py_echo.v1", "@op/echo")
        res = live.call(consumer.session, up[0], b"ping")
        report.check(run, f"through upstream's resolved binding {want}/echo, the Rust owner's @op/echo answers "
                          "(O1, O3)",
                     [(r.kind, r.key, r.payload) for r in res.replies]
                     == [("value", f"zk2/{want}/echo/zk2py_echo.v1/@op/echo", b"ping")],
                     f"{up} → {[(r.kind, r.key, r.payload) for r in res.replies]}")
        consumer.close()
        owners.remove(consumer)
        rust.close()
        rusts.remove(rust)

        # -- the shared file, created by one and read by the other ---------
        def start(kind: str, root: str) -> tuple[Any, str | None]:
            if kind == "Rust":
                o = Owner(exe, "@hostid.v1/echo", [REPO / ECHO], connect=ep, hostid_root=root)
                rusts.append(o)
                line = o.wait_for("ready ", 120)
                return o, line.split("/")[1] if line else None
            o = PyOwner("@hostid.v1", "echo", [echo], connect=ep, hostid=hostid.Runtime(root))
            o.start()
            owners.append(o)
            return o, o.system

        def stop(o: Any) -> None:
            o.close()
            (rusts if isinstance(o, Owner) else owners).remove(o)

        for first, second in (("Rust", "zk2py"), ("zk2py", "Rust")):
            run = f"hostid.v1 0.2 §2.5: the shared file created by {first}, read by {second}"
            root = make_root(base_dir, {})
            a, sa = start(first, root)
            shared = os.path.join(root, "var", "lib", "zk2", "hostid")
            try:
                with open(shared, "rb") as f:
                    content = f.read()
                mode: int | None = os.stat(shared).st_mode & 0o7777
            except OSError as e:
                content, mode = repr(e).encode(), None
            report.check(run, f"{first} creates var/lib/zk2/hostid: 32 lowercase hex digits and a newline, mode "
                              "0644, no temporary file left (§2.5)",
                         re.fullmatch(rb"[0-9a-f]{32}\n", content) is not None and mode == 0o644
                         and not temps(root),
                         f"{content!r}, mode {oct(mode) if mode is not None else None}, temporaries {temps(root)}")
            b, sb = start(second, root)
            derived = hostid.derive(content)
            report.check(run, f"{second} reads it: one system, the file's derivation (§2.2)",
                         sa is not None and sa == sb == derived, f"{first} {sa}, {second} {sb}, derived {derived}")
            _presence(tool, f"zk2/{sa}/echo/@zk/**", 2, 2)
            answer, why, _ = live.hostid_collision(tool, f"{sa}/echo")
            report.check(run, "both at one minted address: §2.12's finding, its cause undecided",
                         answer == "finding", f"{answer}: {why}")
            stop(b)
            deadline = time.monotonic() + 5.0
            while time.monotonic() < deadline and \
                    len(live.list_presence(tool, f"zk2/{sa}/echo/@zk/instance/*").instances) > 1:
                time.sleep(0.1)
            answer, why, _ = live.hostid_collision(tool, f"{sa}/echo")
            report.check(run, f"{second} gone: the answer is no", answer == "no", f"{answer}: {why}")
            stop(a)

        # -- fail closed, and the ephemeral rung (§2.6, scenarios §3, §4) ---
        run = "hostid.v1 0.2 §2.6: the owner example over a root where no input gives an id and none can be created"
        root = make_root(base_dir, {}, dirs=("var/lib/zk2",))
        zk2dir = os.path.join(root, "var", "lib", "zk2")
        os.chmod(zk2dir, 0o555)
        try:
            deadline = time.monotonic() + 5.0
            while time.monotonic() < deadline and live.list_presence(tool, "zk2/*/echo/@zk/instance/*").instances:
                time.sleep(0.1)
            o = Owner(exe, "@hostid.v1/echo", [REPO / ECHO], connect=ep, hostid_root=root)
            rusts.append(o)
            line = o.wait_for("ready ", 30)
            try:
                code = o.proc.wait(timeout=30) if line is None else None
            except subprocess.TimeoutExpired:
                code = None
            stop(o)
            err = " ".join(o.stderr)
            after = live.list_presence(tool, "zk2/*/echo/@zk/instance/*")
            report.check(run, "it does not start: it exits non-zero, its error names the three paths, and R1 shows "
                              "no instance token (§2.6, §8.2)",
                         line is None and code not in (0, None) and not after.instances and after.complete
                         and all(p in err for p in ("/etc/machine-id", "/var/lib/dbus/machine-id", SHARED_PATH)),
                         f"ready {line!r}, exit {code}, tokens {len(after.instances)}; stderr {err[-300:]!r}")
            o = Owner(exe, "@hostid.v1/echo", [REPO / ECHO], connect=ep, hostid_root=root, hostid_ephemeral=True)
            rusts.append(o)
            line = o.wait_for("ready ", 120)
            system = line.split("/")[1] if line else None
            report.check(run, "with --hostid-ephemeral it starts, on a system of the minted shape, and writes no "
                              "file (§2.6, scenarios §4 step 1)",
                         system is not None and hostid.is_minted_shape(system) and not os.listdir(zk2dir),
                         f"system {system}, var/lib/zk2 {os.listdir(zk2dir)}; stderr {o.stderr[-2:]}")
            # scenarios §4 expected 1: "Each start logs that the system is
            # ephemeral, and names the three paths with their outcomes."
            # hostid.v1 0.3 (F-99): "wherever the process's operational logs
            # go, at its warning level or the equivalent", which a black-box
            # runner reads in "the process's own log output": the owner
            # example's stderr.
            deadline = time.monotonic() + 3.0
            said: list[str] = []
            while time.monotonic() < deadline and not said:
                # Its log lines carry terminal colours even on a pipe.
                said = [re.sub(r"\x1b\[[0-9;]*m", "", x) for x in o.stderr if "ephemeral" in x.lower()]
                time.sleep(0.05)
            report.check(run, "its start logs, at the warning level on its stderr, that the system is ephemeral, "
                              "naming the three paths with their outcomes (§2.6, scenarios §4 expected 1; "
                              "hostid.v1 0.3, F-99)",
                         bool(said) and "WARN" in said[0].upper()
                         and all(p in said[0] for p in ("/etc/machine-id", "/var/lib/dbus/machine-id", SHARED_PATH)),
                         f"stderr {o.stderr[-3:]}")
            stop(o)
        finally:
            os.chmod(zk2dir, 0o755)

        # -- bindings.md §5, zk2py's own ----------------------------------
        run = "bindings.md §5 (0.20): zk2py's detectors and trackers through R1"
        sysinfo, tracker = load_contract(REPO / SYSINFO), load_contract(REPO / TRACKER)
        dets = []
        for system, svc in (("vehicle-01", "det0"), ("vehicle-01", "det1"), ("vehicle-02", "det0")):
            d = PyOwner(system, svc, [sysinfo], connect=ep)
            d.start()
            owners.append(d)
            dets.append(d)
        got: dict[str, list[str]] = {"one": [], "all": []}
        wild: dict[str, list[str]] = {"one": [], "all": []}
        trackers = {}
        for name, provider in (("one", "self.system/det0"), ("all", "self.system/*")):
            tr = PyOwner("vehicle-01", name, [tracker], connect=ep, bindings={"sources": [provider]})
            tr.start()
            owners.append(tr)
            trackers[name] = tr
            tr.subscribe_role("sources", "zk2py_sysinfo.v1", "stream/cpu",
                              lambda key, payload, name=name: got[name].append(key),
                              discarded=wild[name].append)
        time.sleep(1.0)
        for d in dets:
            for i in range(3):
                d.publish(f"{d.prefix(sysinfo)}/stream/cpu", str(i).encode())
        # bindings.md §4 alongside: a put on a wildcard key, which reaches
        # both trackers with that key, and which each discards (R6).
        tool.put("zk2/vehicle-01/*/zk2py_sysinfo.v1/stream/cpu", b"wild")
        time.sleep(1.0)

        def froms(keys: list[str]) -> dict[str, int]:
            out: dict[str, int] = {}
            for k in keys:
                src = "/".join(k.split("/")[1:3])
                out[src] = out.get(src, 0) + 1
            return out

        report.check(run, "expected 1: vehicle-01/one receives from vehicle-01/det0 alone, and vehicle-01/all "
                          "from vehicle-01/det0 and vehicle-01/det1; neither from vehicle-02/det0, and the "
                          "wildcard put reaches each with its wildcard key and is discarded (R6)",
                     froms(got["one"]) == {"vehicle-01/det0": 3}
                     and froms(got["all"]) == {"vehicle-01/det0": 3, "vehicle-01/det1": 3}
                     and len(wild["one"]) == len(wild["all"]) == 1,
                     f"one {froms(got['one'])}, all {froms(got['all'])}; discarded {wild}")
        listed = {}
        for name, tr in trackers.items():
            tdoc, _ = _doc(tool, tr.instance_key)
            listed[name] = [r.get("bindings") for r in tdoc.get("requires") or []]
        report.check(run, "expected 2: a tool reads the trackers' descriptors listing the bindings resolved: "
                          "[\"vehicle-01/det0\"] and [\"vehicle-01/*\"]",
                     listed == {"one": [["vehicle-01/det0"]], "all": [["vehicle-01/*"]]}, str(listed))
        try:
            resolve_providers(["self.system/det0"], None)
            refused = "resolved"
        except ValueError as e:
            refused = f"refused: {e}"
        report.check(run, "expected 3: a tool binding a consumer of its own to self.system/det0 is refused: a "
                          "tool has no system of its own", refused.startswith("refused"), refused)
    finally:
        for o in rusts:
            o.close()
        for o in owners:
            o.close()
        tool.close()
        r1.close()
        shutil.rmtree(base_dir, ignore_errors=True)


BEACON = "impl/python/interop/freshness/beacon.v1.toml"
FRESH_CONTRACT = "impl/python/interop/freshness/zk2py_fresh.v1.toml"


def run_python_freshness(report: Report, exe: Path, consume: Path) -> None:
    """freshness.v1 (text 0.1) and core 0.21's re-put across the two
    implementations, through a router R1 of the runner's:
    - the owner example serving ``beacon.v1`` (``lab/beacon``) re-puts its
      state on its own (§2.4). zk2py's subscriber S, declared before it
      starts, and its GET reader G, which measures its clock from S's
      deliveries (§2.6, ground 2), judge its members, and zk2py's tool reads
      one verdict per resource (§5, scenarios §6);
    - a stopped re-put goes stale: the owner example stopped (SIGSTOP), its
      tokens still held, is stale to S and present to presence, both
      reported (§2.11). Continued (SIGCONT), it is fresh again. Closed, its
      tokens go and S still measures the age;
    - zk2py's owner of ``zk2py_fresh.v1`` re-puts, and the Rust ``consume``
      example sees each re-put as the stamp of the owner's GET answer (core
      S2, 0.21). Once zk2py closes the member's writer, it sees one stamp."""
    import signal

    from . import freshness as fr
    from . import live
    from .contract import load_contract
    from .owner import Owner as PyOwner

    if not consume.is_file():
        raise CannotRun(f"{consume} not found: build it with `cargo build -q -p zenkey --example consume`")
    beacon = load_contract(REPO / BEACON)
    h_status = fr.horizon("state", {"freshness.ttl_s": 2})
    base = "zk2/lab/beacon/beacon.v1"
    status, intent, note = f"{base}/state/status", f"{base}/state/intent", f"{base}/state/note"
    r1, ep, _ = _r1()
    s_session, g_session = live.open_client(ep), live.open_client(ep)
    trust = fr.ClockTrust()
    subs: list[fr.Subscriber] = []
    rust: Owner | None = None
    py: Any = None
    try:
        # -- the owner example re-puts; zk2py judges ----------------------
        run = "freshness.v1 0.1: the owner example serving beacon.v1 (lab/beacon), judged by zk2py"
        s = fr.Subscriber(s_session, [f"{base}/state/*", f"{base}/stream/*"], trust)
        subs.append(s)
        time.sleep(0.3)
        rust = Owner(exe, "lab/beacon", [REPO / BEACON], connect=ep)
        ready = rust.wait_for("ready ", 120)
        if not report.check(run, "it starts behind R1", ready is not None,
                            f"ready {ready!r}; stderr {rust.stderr[-2:]}"):
            return
        doc, _ = _doc(g_session, ready)
        zid = live._zid_value((doc.get("meta") or {}).get("zid"))
        report.check(run, "its descriptor lists freshness.v1, the profile its contract uses (§1, core §3.3)",
                     "freshness.v1" in (doc.get("profiles") or []), str(doc.get("profiles")))
        time.sleep(5.0)
        got = s.of(status)
        first = got[0].arrival_ns if got else 0
        window = [r for r in got if r.arrival_ns <= first + 5 * fr.NS]
        gaps = [(b.arrival_ns - a.arrival_ns) / fr.NS for a, b in zip(got, got[1:])]
        report.check(run, "status (ttl 2) is re-put on its own: at least 4 more puts in the 5 s after the first, no "
                          "two more than 1 s apart (with 200 ms of jitter), each the same payload and Encoding, each "
                          "stamped by the owner's zid, each stamp above the one before (§2.4, core S1)",
                     len(window) >= 5 and max(gaps, default=9) <= 1.2
                     and len({(r.payload, r.encoding) for r in got}) == 1
                     and all(live._zid_value(r.stamp_id) == zid and zid is not None for r in got)
                     and all(b.stamp_ns > a.stamp_ns for a, b in zip(got, got[1:])),
                     f"{len(got)} deliveries, {len(window)} in 5 s, largest gap {max(gaps, default=0):.3f} s, values "
                     f"{sorted({(r.payload, r.encoding) for r in got})}")
        report.check(run, "intent (ttl 0) and note (no horizon) are put once, never re-put (§2.3, §2.4)",
                     len(s.of(intent)) == 1 and len(s.of(note)) == 1,
                     f"intent {len(s.of(intent))}, note {len(s.of(note))} deliveries")
        before = s.of(status)[-1]
        reading = fr.get_reading(g_session, status)
        delivered = {r.stamp for r in s.of(status)}
        report.check(run, "G's reply carries the stamp of a re-put S received, the latest: a re-put is a mutation, "
                          "and the owner answers with its stamp (core S2, 0.21)",
                     reading.stamp in delivered and reading.stamp_ns is not None and before.stamp_ns is not None
                     and reading.stamp_ns >= before.stamp_ns,
                     f"reply {reading.stamp}, latest before {before.stamp}")
        v_s = fr.judge_observation("state", h_status, s.observation(status))
        v_g = fr.judge_observation("state", h_status, reading.observation(trust))
        measured = any(trust.measured(k) for k in trust.measurements if live._zid_value(k) == zid)
        report.check(run, "S and G both judge status fresh; G trusts its clock by measuring the owner's live puts "
                          "(§2.5, §2.6 ground 2)",
                     v_s.verdict == fr.FRESH and v_g.verdict == fr.FRESH and measured,
                     f"S {v_s.pair()}, G {v_g.pair()}, measured {measured}")
        fp = live.fingerprint_of(doc, "beacon.v1")
        r = live.retrieve_bundle(g_session, "beacon.v1", fp) if fp else None
        canon = r.verified.contract if r is not None and r.verified is not None else None
        report.check(run, "its bundle, retrieved by the descriptor's fingerprint (§8.4), is the contract zk2py "
                          "builds", canon is not None and canon == beacon.canonical, f"fingerprint {fp}")
        if canon is not None:
            got_r = fr.read_service(g_session, "lab/beacon", [canon], 3.0, fr.ClockTrust())
            verdicts = {k: v.verdict for k, (v, _) in got_r.items()}
            report.check(run, "zk2py's tool, over a 3 s window from the bundle (§5, scenarios §6): status fresh, "
                              "intent fresh, note not asked; level unobservable, since the example publishes no "
                              "stream sample and no member of it is known (§2.5)",
                         verdicts == {"state/status": fr.FRESH, "state/intent": fr.FRESH,
                                      "state/note": fr.NOT_ASKED, "stream/level": fr.UNOBSERVABLE},
                         str({k: (v.pair(), m) for k, (v, m) in got_r.items()}))

        # -- a stopped re-put goes stale ----------------------------------
        run = "freshness.v1 0.1: the owner example's re-puts stopped"
        rust.proc.send_signal(signal.SIGSTOP)
        try:
            t_stop = time.monotonic_ns()
            time.sleep(0.1)
            last = s.of(status)[-1].arrival_ns
            _sleep_until(last + fr.NS)
            v1 = fr.judge_observation("state", h_status, s.observation(status))
            _sleep_until(max(last + 3 * fr.NS, t_stop + 3 * fr.NS))
            v3 = fr.judge_observation("state", h_status, s.observation(status))
            pres = live.list_presence(g_session, "zk2/lab/beacon/@zk/**")
            silent = fr.get_reading(g_session, status)
            v_get = fr.judge_observation("state", h_status, silent.observation(trust))
            both = fr.judge("state", h_status, [s.observation(status), silent.observation(trust)])
            report.check(run, "stopped (SIGSTOP): S fresh 1 s after its last delivery, stale 3 s after it (§2.5)",
                         v1.verdict == fr.FRESH and v3.verdict == fr.STALE, f"{v1.pair()} then {v3.pair()}")
            report.check(run, "its instance and interface tokens are still present while status is stale: both "
                          "reported, neither folded into the other (§2.11)",
                         len(pres.instances) == 1 and len(pres.alive) == 1 and pres.complete
                         and v3.verdict == fr.STALE,
                         f"presence {len(pres.instances)}+{len(pres.alive)} tokens; status {v3.verdict}")
            report.check(run, "a GET of the stopped owner is silent, unobservable; with S's stale observation, the "
                              "member is stale (§2.6, §2.7)",
                         v_get.pair() == {"verdict": "unobservable", "reason": "silent"}
                         and both.verdict == fr.STALE, f"GET {v_get.pair()}, combined {both.pair()}")
        finally:
            rust.proc.send_signal(signal.SIGCONT)
        time.sleep(2.5)
        v_again = fr.judge_observation("state", h_status, s.observation(status))
        report.check(run, "continued (SIGCONT): the re-puts resume, and S judges status fresh again",
                     v_again.verdict == fr.FRESH, str(v_again.pair()))
        code = rust.close()
        rust = None
        t_close = time.monotonic_ns()
        last = s.of(status)[-1].arrival_ns
        _sleep_until(max(last + 3 * fr.NS, t_close + fr.NS))
        v_closed = fr.judge_observation("state", h_status, s.observation(status))
        after = [x for x in s.of(status) if x.arrival_ns > t_close]
        pres = live.list_presence(g_session, "zk2/lab/beacon/@zk/**")
        report.check(run, "closed: no re-put after it ends, its tokens are gone, and S still measures the age: "
                          "stale, not \"down\" (§2.4, §2.11)",
                     code == 0 and not after and pres.count == 0 and pres.complete and v_closed.verdict == fr.STALE,
                     f"exit {code}, {len(after)} deliveries after, {pres.count} tokens, status {v_closed.pair()}")

        # -- zk2py's re-puts, seen by the Rust consumer -------------------
        run = "freshness.v1 0.1: zk2py's owner of zk2py_fresh.v1 (lab/fresh), read by the Rust consume example"
        contract = load_contract(REPO / FRESH_CONTRACT)
        py = PyOwner("lab", "fresh", [contract], connect=ep)
        py.start()
        key = "zk2/lab/fresh/zk2py_fresh.v1/state/status"
        time.sleep(0.3)
        py.set_state(key, b"up")

        def consumed() -> tuple[int, list[str], str]:
            p = subprocess.run([str(consume), ep, "lab/fresh", str(REPO / FRESH_CONTRACT), "state/status",
                                "@op/echo"], capture_output=True, text=True, timeout=60)
            return p.returncode, p.stdout.splitlines(), p.stderr.strip()[-200:]

        def stamp_of(lines: list[str]) -> str | None:
            st = [x.split(" ") for x in lines if x.startswith("state ")]
            return st[0][2] if st and len(st[0]) == 4 and st[0][1] == key and st[0][3] == "up" else None

        time.sleep(1.5)
        c1 = consumed()
        time.sleep(1.5)
        c2 = consumed()
        s1, s2 = stamp_of(c1[1]), stamp_of(c2[1])
        reput_stamps = {str(st) for k, st in py.reputs if k == key}
        mine = live._zid_value(str(py.session.zid()))
        report.check(run, "two reads 1.5 s apart: the same value under two stamps, both re-puts zk2py made, both "
                          "with zk2py's zid, the second later (§2.4, core S2, 0.21)",
                     c1[0] == c2[0] == 0 and s1 is not None and s2 is not None and s1 != s2
                     and {s1, s2} <= reput_stamps
                     and live._zid_value(s1.split("/")[1]) == live._zid_value(s2.split("/")[1]) == mine
                     and int(s2.split("/")[0]) > int(s1.split("/")[0]),
                     f"exit {c1[0]}, {c2[0]}; stamps {s1}, {s2}; {len(reput_stamps)} re-puts; stderr {c1[2]!r}")
        py.close_writer(key)
        time.sleep(0.2)
        c3 = consumed()
        time.sleep(1.5)
        c4 = consumed()
        s3, s4 = stamp_of(c3[1]), stamp_of(c4[1])
        report.check(run, "the member's writer closed: two reads 1.5 s apart carry one stamp, the last re-put's: "
                          "the value is held, and nobody confirms it (§2.4)",
                     s3 is not None and s3 == s4 and s3 in reput_stamps | {str(py._held[key].stamp)},
                     f"stamps {s3}, {s4}")
    finally:
        if rust is not None:
            try:
                rust.proc.send_signal(signal.SIGCONT)
            except OSError:
                pass
            rust.close()
        if py is not None:
            py.close()
        for x in subs:
            x.close()
        s_session.close()
        g_session.close()
        r1.close()


def _sleep_until(mono_ns: int) -> None:
    d = (mono_ns - time.monotonic_ns()) / 1e9
    if d > 0:
        time.sleep(d)


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="python -m zk2py.live_interop", description=__doc__.split("\n")[0])
    ap.add_argument("--owner", type=Path, default=Path(os.environ.get("ZK2PY_OWNER", DEFAULT_OWNER)),
                    help="the owner example binary")
    ap.add_argument("--run", action="append", metavar="SYSTEM/SERVICE=CONTRACT[,CONTRACT…]",
                    help="one owner run; repeatable (default: two runs over the examples and "
                         "impl/python/interop)")
    ap.add_argument("--consume", type=Path, default=Path(os.environ.get("ZK2PY_CONSUME", DEFAULT_CONSUME)),
                    help="the consume example binary")
    ap.add_argument("--scale", type=int, default=2000,
                    help="extra tokens for the presence-at-scale check, in the first run (0: skip)")
    python_runs = {
        "rust-behind-r1": lambda r: run_rust_behind_r1(r, args.owner),
        "owner": lambda r: run_python_owner(r, args.consume), "refusal": run_python_refusal,
        "s1": run_python_s1, "bringup": run_python_bringup, "presence-refused": run_python_presence_refused,
        "o1": run_python_o1, "fanout": run_python_fanout, "tool-rules": run_python_tool_rules,
        "0.16": run_python_016, "hostid": lambda r: run_python_hostid(r, args.owner),
        "freshness": lambda r: run_python_freshness(r, args.owner, args.consume),
        "acl": lambda r: __import__("zk2py.acl_interop", fromlist=["run_python_acl"]).run_python_acl(r, REPO),
    }
    ap.add_argument("--only", action="append", choices=sorted(python_runs),
                    help="run only these runs, the ones behind a router of the runner's (repeatable), "
                         "and not the owner example's own-router runs")
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
        if args.only:
            for name in args.only:
                python_runs[name](report)
        else:
            for i, (service, files) in enumerate(runs):
                run_one(report, args.owner, service, [REPO / f if not Path(f).is_absolute() else Path(f)
                                                      for f in files], args.scale if i == 0 else 0)
        if not args.run and not args.only:
            for service, files in REFUSAL_RUNS:
                run_refusal(report, args.owner, service, [REPO / f for f in files])
            for run in python_runs.values():
                run(report)
    except CannotRun as e:
        print(f"error: could not run: {e}", file=sys.stderr)
        return 2
    passed = sum(1 for r in report.rows if r[2])
    failed = len(report.rows) - passed
    print(f"live interop: {passed} passed, {failed} failed, "
          f"{len(report.known)} known deviations of the Rust owner example")
    for run, name, ok, detail in report.known:
        print(f"{'XPASS' if ok else 'XFAIL'} [{run}] {name}: {detail}")
    for run, name, ok, detail in report.rows:
        if not ok:
            print(f"FAIL [{run}] {name}: {detail}")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())

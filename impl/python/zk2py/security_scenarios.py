"""``spec/scenarios/security.md`` §4 (core §11.1, §11.2, 0.50), the admin
read that admits no GET of a node's declaration records:
``python -m zk2py.security_scenarios [--zenctl PATH]``, and the live
runner's ``security4``.

**Setup**, as the section writes it. One zenoh-python router R1 in this
process, listening on port 0, its admin space on, read permitted, every
session authenticated by usrpwd, under ``default_permission: deny`` with a
generator's block for two principals:
- a tool that reads the admin space and inspects every service (§11.1
  Tool);
- a service principal that does neither. It holds a queryable ending in
  ``**`` on its own prefix, declared before step 1, so that R1 has a
  declaration record a GET of the records would answer.

R1's own session plays its storage manager's document for ``plans``
(``key_expr`` ``zk2/ground/fleet-mgr/**``): a zenoh-python router loads no
plugin, as in state.md §3 and §12. R1's own session also holds the
queryable on ``@/**`` that answers nothing and records every selector R1
routes to it (:class:`zk2py.adminspace.Recorder`): no principal may declare
one (§11.1). Nothing declares while a step runs: every session is open and
every queryable declared, then 0.5 s pass, before step 1. Every GET is
target ``All``, consolidation ``None``, with a 3 s timeout, and "ends
complete" means its final reply came before that timeout, with no error
reply.

**The blocks.**
- zk2py's own generator (:func:`zk2py.acl.generate`) over the two
  principals, ``tool`` and ``svc`` (``h1/tc``);
- with ``--zenctl``, the reference's ``zenctl acl gen``, a black box, over
  two enrollments, each block run as zenctl writes it:
  - ``impl/python/interop/security/section4.enrollment.toml``, the
    section's two principals as zk2py compiles them, whose grants are
    compared with zk2py's per principal, message and flow;
  - ``examples/zk2/acl/walkthrough.enrollment.toml``: the tool is
    ``ground``, which runs the tool ``ops`` (``admin = true``,
    ``inspects = ["*/*"]``) beside its services, and the service principal
    ``navigation``, among 13 principals.

  Step 4 replaces the key expressions of zenctl's admin-read rules by
  0.15's.

Beside the section, on the same blocks:
- zk2py's block check (:func:`zk2py.acl.block_hazards`) finds no hazard in
  the generated ``deny`` block, and finds step 4's;
- §11.2's two inclusion facts, each on a block that allows one key alone:
  under ``@/*/router``, ``@/*/*`` is refused; under the storage manager's
  status subtree, ``@/*/router/**`` is;
- 0.50's "This protects a router under deny alone": under the generator's
  ``allow`` block, the tool's GET of the queryable records is admitted, and
  answered with the service principal's record.

Every GET of a declaration record here is made beside no writer, so none
can freeze R1. The freeze itself is :mod:`zk2py.adminspace`'s measurement,
never a suite.

Exit 0 when every check passes, 1 when any fails, 2 when it could not run.
"""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path
from typing import Any

REPO = Path(__file__).resolve().parents[3]
STORAGES = "status/plugins/storage_manager/storages"
#: §4's storage, as R1's storage manager states it
PLANS = ("plans", "zk2/ground/fleet-mgr/**")
#: step 1: the tools' three reads (§11.1 Tool, 0.50)
READS = ["@/*/router", f"@/*/router/{STORAGES}/**", "@/*/*"]
#: step 2: the tool's GETs the admin read does not include
REFUSED = ["@/*/router/queryable/**", "@/*/router/**", "@/**"]
GET_TIMEOUT_S = 3.0
#: zenctl acl gen's inputs: (name, enrollment, contracts, the tool's user,
#: the service principal's, the key of the service principal's queryable)
ZENCTL_ENROLLMENTS = [
    ("section4", "impl/python/interop/security/section4.enrollment.toml",
     ["impl/python/interop/zk2py_echo.v1.toml", "impl/python/interop/zk2py_tc.v1.toml"], "tool", "svc",
     "zk2/h1/tc/zk2py_echo.v1/state/**"),
    ("walkthrough", "examples/zk2/acl/walkthrough.enrollment.toml",
     ["examples/zk2/walkthrough", "spec/profiles/health"], "ground", "navigation",
     "zk2/vehicle-01/navigation/nav.v2/state/**"),
]


def password(user: str) -> str:
    """A fixed, test-only password: R1 listens on loopback alone (port 0),
    and reads it from a usrpwd dictionary in a temporary directory, removed
    at the end of the run."""
    return f"pw-{user}"


# -- R1 -----------------------------------------------------------------------

class R1:
    """§4's R1 under ``block``, in this process: port 0, its endpoint from
    its own root document, the storage document and the recorder on its
    own session."""

    def __init__(self, block: dict[str, Any], users: list[str], workdir: str) -> None:
        import zenoh

        from .adminspace import Recorder, node_config, own_endpoint

        self.dictionary = os.path.join(workdir, "users.txt")
        Path(self.dictionary).write_text("".join(f"{u}:{password(u)}\n" for u in users))
        self.session = zenoh.open(node_config("router", block, self.dictionary))
        self.zid = str(self.session.zid())
        endpoint = own_endpoint(self.session)
        if endpoint is None:
            self.session.close()
            raise RuntimeError("R1 found no loopback endpoint of its own")
        self.endpoint: str = endpoint
        self.storage_key = f"@/{self.zid}/router/{STORAGES}/{PLANS[0]}"
        body = json.dumps({"key_expr": PLANS[1], "volume": "memory"})
        self.storage = self.session.declare_queryable(self.storage_key, zenoh.handlers.Callback(
            lambda q: q.reply(self.storage_key, body, encoding="application/json")))
        self.recorder = Recorder(self.session)
        self.clients: list[Any] = []
        self.entities: list[Any] = []

    def client(self, user: str) -> Any:
        from . import live

        s = live.open_client(self.endpoint, (user, password(user)))
        self.clients.append(s)
        return s

    def close(self) -> None:
        from .adminspace import close_bounded

        for e in self.entities:
            try:
                e.undeclare()
            except Exception:  # noqa: BLE001 - closing anyway
                pass
        close_bounded(*self.clients)
        self.recorder.close()
        self.storage.undeclare()
        close_bounded(self.session)


def ask(session: Any, selector: str, timeout: float = GET_TIMEOUT_S) -> dict[str, Any]:
    """One GET, target ``All``, consolidation ``None``: each value reply's
    key, replier id and payload, each error reply, whether it ended before
    ``timeout`` (complete), and how long it took. The selectors §4 asks are
    asked as its user asks them: zk2py's own readers never ask step 2's
    (:func:`zk2py.live.admin_guard`)."""
    import zenoh

    from . import live

    live.admin_guard(selector, user_asked=True)
    values: list[tuple[str, str | None, bytes]] = []
    errors: list[str] = []
    done = threading.Event()
    t0 = time.monotonic()

    def on_reply(r: Any) -> None:
        if r.ok is not None:
            values.append((str(r.ok.key_expr), live.replier_of(r), r.ok.payload.to_bytes()))
        else:
            errors.append(r.err.payload.to_string())

    session.get(selector, zenoh.handlers.Callback(on_reply, done.set), target=zenoh.QueryTarget.ALL,
                consolidation=zenoh.ConsolidationMode.NONE, timeout=timeout)
    ended = done.wait(timeout + 2.0)
    s = time.monotonic() - t0
    return {"selector": selector, "values": values, "errors": errors, "ended": ended,
            "complete": ended and not errors and s < timeout, "ms": round(s * 1000, 1)}


def _show(a: dict[str, Any]) -> str:
    keys = sorted({k for k, _, _ in a["values"]})
    return (f"{a['selector']}: {len(a['values'])} value(s) {keys[:4]}{' …' if len(keys) > 4 else ''}, "
            f"errors {a['errors']}, {'complete' if a['complete'] else 'not complete'} in {a['ms']} ms")


def _heard(r1: R1) -> list[str]:
    """What the recorder heard of one GET. Its callback runs before the GET's
    final reply can come, so by the time the GET has ended it has heard it."""
    time.sleep(0.05)
    return r1.recorder.take()


# -- the blocks ---------------------------------------------------------------

def zk2py_principals(repo: Path) -> list[Any]:
    from .acl import Principal
    from .contract import load_contract

    contracts = [load_contract(repo / f"impl/python/interop/{n}.v1.toml") for n in ("zk2py_echo", "zk2py_tc")]
    return [Principal("tool", "tool", inspects=["*/*"], admin_read=True),
            Principal("svc", "svc", service="h1/tc", implements=contracts)]


class Blocks:
    """One generator's blocks for §4: the ``deny`` block, step 4's (0.15's
    admin read), and the ``allow`` block; the tool's and the service
    principal's user names, and the key of the service principal's
    queryable ending in ``**``."""

    def __init__(self, name: str, deny: dict[str, Any], deny_015: dict[str, Any], allow: dict[str, Any],
                 tool: str, service: str, held: str) -> None:
        self.name, self.deny, self.deny_015, self.allow = name, deny, deny_015, allow
        self.tool, self.service, self.held = tool, service, held


def zk2py_blocks(repo: Path) -> Blocks:
    from . import acl

    ps = zk2py_principals(repo)
    return Blocks("zk2py's generator", acl.generate(ps, "deny").block,
                  acl.generate(ps, "deny", admin_read=acl.ADMIN_READ_015, hazard_ok=True).block,
                  acl.generate(ps, "allow").block, "tool", "svc", "zk2/h1/tc/zk2py_echo.v1/state/**")


def zenctl_block(zenctl: Path, repo: Path, posture: str, enrollment: str, contracts: list[str]) -> dict[str, Any]:
    """``zenctl acl gen`` over ``enrollment``, a black box: its JSON5, read
    by zenoh's own configuration loader."""
    from . import acl

    argv = [str(zenctl), "acl", "gen", "--enrollment", enrollment, "--json5"]
    for c in contracts:
        argv += ["--contracts", c]
    if posture == "allow":
        argv += ["--default-permission", "allow"]
    p = subprocess.run(argv, cwd=repo, capture_output=True, text=True, timeout=60)
    if p.returncode != 0:
        raise RuntimeError(f"zenctl acl gen exited {p.returncode}: {p.stderr.strip()[:300]}")
    return acl.read_block_json5(p.stdout)


def with_admin_read(block: dict[str, Any], keys: list[str]) -> dict[str, Any]:
    """``block`` with every rule whose key expressions are 0.50's admin read
    given ``keys`` instead: step 4's "Compile the block again with the
    tool's admin read replaced by 0.15's", on a block zk2py did not
    compile."""
    from . import acl

    out = json.loads(json.dumps(block))
    for r in out["rules"]:
        if sorted(r.get("key_exprs") or []) == sorted(acl.ADMIN_READ):
            r["key_exprs"] = list(keys)
    return out


def zenctl_blocks(zenctl: Path, repo: Path, spec: tuple) -> Blocks:
    from . import acl

    name, enrollment, contracts, tool, service, held = spec
    deny = zenctl_block(zenctl, repo, "deny", enrollment, contracts)
    return Blocks(f"zenctl acl gen ({name})", deny, with_admin_read(deny, acl.ADMIN_READ_015),
                  zenctl_block(zenctl, repo, "allow", enrollment, contracts), tool, service, held)


def allowed(block: dict[str, Any]) -> dict[tuple[str, str, str], list[str]]:
    """A ``deny`` block's allow rules as (user name, message, flow) -> the
    sorted keys its policies grant, a rule with no ``flows`` on both."""
    rules = {r["id"]: r for r in block["rules"]}
    users = {s["id"]: (s.get("usernames") or [s["id"]])[0] for s in block["subjects"]}
    out: dict[tuple[str, str, str], set[str]] = {}
    for p in block["policies"]:
        for sid in p["subjects"]:
            for rid in p["rules"]:
                r = rules[rid]
                if r.get("permission") != "allow":
                    continue
                for m in r["messages"]:
                    for f in r.get("flows") or ["ingress", "egress"]:
                        out.setdefault((users[sid], m, f), set()).update(r["key_exprs"])
    return {k: sorted(v) for k, v in out.items()}


def single_key_block(users_keys: dict[str, str]) -> dict[str, Any]:
    """§11.2's inclusion facts: each user allowed ``query`` on one key
    alone, and its ``reply``, under ``deny``."""
    rules, subjects, policies = [], [], []
    for user, key in users_keys.items():
        rules += [{"id": f"{user}-q", "messages": ["query"], "flows": ["ingress"], "permission": "allow",
                   "key_exprs": [key]},
                  {"id": f"{user}-r", "messages": ["reply"], "flows": ["egress"], "permission": "allow",
                   "key_exprs": [key]}]
        subjects.append({"id": user, "usernames": [user]})
        policies.append({"id": f"{user}-policy", "rules": [f"{user}-q", f"{user}-r"], "subjects": [user]})
    return {"enabled": True, "default_permission": "deny", "rules": rules, "subjects": subjects,
            "policies": policies}


# -- the section ------------------------------------------------------------------

def run_section4(report: Any, repo: Path = REPO, zenctl: Path | None = None) -> None:
    """security.md §4 with zk2py's block, and with zenctl's when ``zenctl``
    is a file; then §11.2's inclusion facts."""
    workdir = tempfile.mkdtemp(prefix="zk2py-sec4-")
    try:
        ours = zk2py_blocks(repo)
        sets = [ours]
        if zenctl is not None:
            if zenctl.is_file():
                for spec in ZENCTL_ENROLLMENTS:
                    try:
                        sets.append(zenctl_blocks(zenctl, repo, spec))
                    except (RuntimeError, subprocess.TimeoutExpired) as e:
                        report.check(f"security.md §4 (zenctl acl gen, {spec[0]})",
                                     f"zenctl acl gen compiled {spec[1]}", False, str(e))
                theirs = next((s for s in sets if s.name.endswith("(section4)")), None)
                if theirs is not None:
                    a, b = allowed(ours.deny), allowed(theirs.deny)
                    differ = {k: (a.get(k), b.get(k)) for k in sorted(set(a) | set(b)) if a.get(k) != b.get(k)}
                    report.check("security.md §4 (the two generators)", "for the section's two principals, zenctl "
                                 "acl gen's deny block grants what zk2py's does, per principal, message and flow",
                                 not differ, f"{len(set(a) | set(b))} (principal, message, flow) grants; "
                                 f"differing: {differ}")
            else:
                report.check("security.md §4 (zenctl acl gen)", "the section ran", False, f"{zenctl} not found")
        for blocks in sets:
            _section4(report, blocks, workdir)
        _inclusion(report, workdir)
    finally:
        shutil.rmtree(workdir, ignore_errors=True)


def _section4(report: Any, b: Blocks, workdir: str) -> None:
    from . import acl

    run = f"security.md §4 ({b.name}'s block, under deny)"
    hazards = acl.block_hazards(b.deny)
    old = [h for h in acl.block_hazards(b.deny_015) if h.kind == "allowed_query"]
    report.check(run, "zk2py's block check (§11.1, 0.50): the deny block allows no query that intersects a node's "
                      "declaration records; with 0.15's admin read it finds the tool's @/*/router/** rule",
                 not hazards and old and all(h.key == "@/*/router/**" and h.flow == "ingress" for h in old),
                 f"{[h.why() for h in hazards]}; with 0.15's: {[h.why() for h in old][:2]}")
    users = sorted({b.tool, b.service})
    r1 = R1(b.deny, users, workdir)
    try:
        import zenoh

        tool, svc = r1.client(b.tool), r1.client(b.service)
        tool_zid = str(tool.zid())
        r1.entities.append(svc.declare_queryable(b.held, zenoh.handlers.Callback(lambda q: None)))
        time.sleep(0.5)  # every session open and every queryable declared before step 1
        r1.recorder.take()
        own = f"@/{r1.zid}/router"
        # Step 1: the tool's three reads.
        got = {}
        for sel in READS:
            a = ask(tool, sel)
            got[sel] = (a, _heard(r1))
        routers, storages, roots = (got[s][0] for s in READS)
        plans = [json.loads(p).get("key_expr") for k, _, p in storages["values"] if k == r1.storage_key]
        report.check(run, "expected 1: R1 answers each of step 1's reads: its document, plans with its key_expr, "
                          "and its root document, each under R1's replier id, each GET complete",
                     [(k, r) for k, r, _ in routers["values"]] == [(own, r1.zid)]
                     and [(k, r) for k, r, _ in storages["values"]] == [(r1.storage_key, r1.zid)]
                     and plans == [PLANS[1]] and [(k, r) for k, r, _ in roots["values"]] == [(own, r1.zid)]
                     and all(got[s][0]["complete"] for s in READS),
                     " | ".join(_show(got[s][0]) for s in READS))
        report.check(run, "expected 1: the recorder on @/** hears each selector, once",
                     all(got[s][1] == [s] for s in READS), str({s: got[s][1] for s in READS}))
        # Step 2: the tool's GETs the admin read does not include.
        got2 = {}
        for sel in REFUSED:
            a = ask(tool, sel)
            got2[sel] = (a, _heard(r1))
        mine = f"@/{tool_zid}/session/"
        foreign = {s: [(k, r) for k, r, _ in got2[s][0]["values"] if not (k.startswith(mine) and r in (tool_zid, None))]
                   for s in REFUSED}
        report.check(run, "expected 2: each of step 2's GETs ends complete with nothing of R1's: "
                          + ", ".join(REFUSED),
                     all(got2[s][0]["complete"] and not foreign[s] for s in REFUSED),
                     " | ".join(_show(got2[s][0]) for s in REFUSED) + f"; not the tool's own: {foreign}")
        report.check(run, "expected 2: the recorder hears none of them: R1 refused each at the tool's face, before "
                          "routing it", all(not got2[s][1] for s in REFUSED), str({s: got2[s][1] for s in REFUSED}))
        report.check(run, "expected 2: @/** reaches the tool's own session, whose runtime answers "
                          "@/<its zid>/session/** itself",
                     bool(got2["@/**"][0]["values"]) and not foreign["@/**"],
                     f"{len(got2['@/**'][0]['values'])} answer(s), each under {mine}")
        # Step 3: the service principal reads step 1's three selectors.
        got3 = {}
        for sel in READS:
            a = ask(svc, sel)
            got3[sel] = (a, _heard(r1))
        report.check(run, "expected 3: nothing answers the service principal's three reads, each GET complete, and "
                          "the recorder hears none of them",
                     all(not got3[s][0]["values"] and got3[s][0]["complete"] and not got3[s][1] for s in READS),
                     " | ".join(_show(got3[s][0]) for s in READS) + f"; heard {[got3[s][1] for s in READS]}")
    finally:
        r1.close()
    # Step 4: 0.15's admin read.
    r1 = R1(b.deny_015, users, workdir)
    try:
        tool = r1.client(b.tool)
        time.sleep(0.5)
        r1.recorder.take()
        own = f"@/{r1.zid}/router"
        mesh = ask(tool, "@/*/*")
        mesh_heard = _heard(r1)
        routers = ask(tool, "@/*/router")
        routers_heard = _heard(r1)
        report.check(run, "expected 4: under 0.15's admin read (@/*/router, @/*/router/**), @/*/* is refused, "
                          "complete and unheard, though both keys intersect it; @/*/router is answered and heard",
                     not mesh["values"] and mesh["complete"] and not mesh_heard
                     and [(k, r) for k, r, _ in routers["values"]] == [(own, r1.zid)] and routers_heard == ["@/*/router"],
                     f"{_show(mesh)}, heard {mesh_heard} | {_show(routers)}, heard {routers_heard}")
    finally:
        r1.close()
    _under_allow(report, b, workdir)


def _under_allow(report: Any, b: Blocks, workdir: str) -> None:
    """0.50 §11.1: "Under allow, the generator compiles no admin read, and a
    GET wider than every deny is in none (§11.3). This protects a router
    under deny alone." Beside no writer."""
    import zenoh

    from . import acl

    run = f"core §11.1 (0.50), under allow ({b.name}'s block)"
    open_ = sorted({h.policy for h in acl.block_hazards(b.allow) if h.kind == "allow_posture"})
    r1 = R1(b.allow, sorted({b.tool, b.service}), workdir)
    try:
        tool, svc = r1.client(b.tool), r1.client(b.service)
        r1.entities.append(svc.declare_queryable(b.held, zenoh.handlers.Callback(lambda q: None)))
        time.sleep(0.5)
        r1.recorder.take()
        a = ask(tool, "@/*/router/queryable/**")
        heard = _heard(r1)
        record = f"@/{r1.zid}/router/queryable/{b.held}"
        report.check(run, "the tool's GET of R1's queryable records is admitted: the recorder hears it, and R1 "
                          "answers the service principal's record; zk2py's block check names every policy open",
                     heard == ["@/*/router/queryable/**"] and record in [k for k, _, _ in a["values"]]
                     and len(open_) == len(b.allow["policies"]),
                     f"{_show(a)}, heard {heard}; open policies {len(open_)} of {len(b.allow['policies'])}")
    finally:
        r1.close()


def _inclusion(report: Any, workdir: str) -> None:
    """§11.2 (0.50): "Under an allowed @/*/router, a query on @/*/* is
    refused; under an allowed
    @/*/router/status/plugins/storage_manager/storages/**, one on
    @/*/router/** is." Each pair intersects."""
    import zenoh

    run = "core §11.2 (0.50): a query is admitted by inclusion, never by intersection"
    status = f"@/*/router/{STORAGES}/**"
    pairs = {"router-only": ("@/*/router", "@/*/*"), "status-only": (status, "@/*/router/**")}
    r1 = R1(single_key_block({u: k for u, (k, _) in pairs.items()}), list(pairs), workdir)
    try:
        sessions = {u: r1.client(u) for u in pairs}
        time.sleep(0.5)
        r1.recorder.take()
        for user, (allowed, asked) in pairs.items():
            k_a, k_q = zenoh.KeyExpr(allowed), zenoh.KeyExpr(asked)
            inside = ask(sessions[user], allowed)
            inside_heard = _heard(r1)
            outside = ask(sessions[user], asked)
            outside_heard = _heard(r1)
            report.check(run, f"under an allowed {allowed} alone, {asked} (which it intersects and does not include) "
                              f"is refused, complete, empty and unheard; {allowed} itself is answered and heard",
                         k_a.intersects(k_q) and not k_a.includes(k_q)
                         and not outside["values"] and outside["complete"] and not outside_heard
                         and bool(inside["values"]) and inside_heard == [allowed],
                         f"{_show(outside)}, heard {outside_heard} | {_show(inside)}, heard {inside_heard}")
    finally:
        r1.close()


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="python -m zk2py.security_scenarios", description=__doc__.split("\n")[0])
    ap.add_argument("--zenctl", type=Path, default=None,
                    help="the reference's zenctl, whose acl gen block runs §4 too (default: zk2py's block alone)")
    args = ap.parse_args(argv)
    try:
        import zenoh  # noqa: F401
    except ImportError as e:
        print(f"error: zenoh-python is not installed: {e}", file=sys.stderr)
        return 2
    from .live_interop import Report

    report = Report()
    try:
        run_section4(report, REPO, args.zenctl)
    except RuntimeError as e:
        print(f"error: could not run: {e}", file=sys.stderr)
        return 2
    failed = [r for r in report.rows if not r[2]]
    print(f"security.md §4: {len(report.rows) - len(failed)} passed, {len(failed)} failed")
    for run, name, _, detail in failed:
        print(f"FAIL [{run}] {name}: {detail}")
    return 1 if failed else 0


if __name__ == "__main__":
    rc = main()
    sys.stdout.flush()
    os._exit(rc)

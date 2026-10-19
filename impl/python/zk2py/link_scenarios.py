"""The ``link.v1`` scenarios (``spec/profiles/link/scenarios.md``, text
0.7, core 0.32) on zenoh-python routers: ``just py-link`` (``python -m
zk2py.link_scenarios [--only 1 …]``), all thirteen sections.

- **The deployment** is the reference's example, ``examples/zk2/link/``,
  read as given: the ``radio.v1`` contract, the lab router's face policy
  (``radio.face.toml``, each section's changes applied to it) and the
  enrollment (``radio.enrollment.toml``: the two radios, and the console
  tool bound to every stream, state and event of ``radio.v1`` on ``lab/*``
  and calling ``reset`` there, run by ``local``, ``ground`` and
  ``ground-router``), read into ``zk2py.acl``'s principals.
- **The generator** is zk2py's: :func:`zk2py.link.plan`, compiled into N's
  ``access_control`` block under ``deny`` (``allow`` where §7 says so) by
  ``zk2py.acl``, the plan's denies in the far principal's policy, every
  other principal's subject narrowed off the face, and under ``allow`` the
  face's own subject in the far principal's policy (§2.4, 0.3,
  :func:`zk2py.link.compile_faces`; 0.2's :func:`zk2py.link.compile_face`
  for §7's control), the plan's ``downsampling`` block beside it, and N's
  ``gateway`` for a south region (:func:`zk2py.link.router_extras`).
- **A near principal on the face** (§7): X, a client of N over the face
  with ``local``'s credentials.
- **Hand edits** (§9 to §11): each step changes the generator's block
  (``World(block_edit=…)``); a tool's check is
  :func:`zk2py.link.check_router_block`, every subject that can match the
  far principal's session checked alone (0.4).
- **Two minors** (§8): ``radio.v1`` minor 1 is the example's contract with
  ``minor = 1`` and ``air/frames_total``'s annotations ``{
  "link.exposure" = "link" }``, written beside the run.
- **Measurements** the text states no expectation for are printed as
  ``NOTE`` lines, and are not counted.
- **The bus** is the text's: N, a zenoh-python router with timestamping and
  usrpwd, listening on ``tcp/127.0.0.1`` (the near side) and on a
  ``unixsock-stream`` path (the face). Every session authenticates as a
  user of its own. The owners are zk2py owners of ``radio.v1``.
- **G and L subscribe to the selectors their grants name** (0.2), one per
  stream, state and event resource, ``zk2/lab/*/radio.v1/<token>/<pattern>``,
  and hold a liveliness subscriber on ``zk2/lab/*/@zk/**``.
- **A south region** (§2): the far router F, user ``ground-router``, names
  its region ``ground`` and attaches to N over the face; G is a client of
  F. Its ``declare_queryable`` egress grant is §2.5's (0.2,
  :func:`zk2py.link.declaration_grant`), with the two controls.
- **The face going down** (§5) is G's session closed and opened again, as
  the reference does, G keeping the members it merged.

Exit 0 when every check passes, 1 when any fails.
"""

from __future__ import annotations

import argparse
import json
import os
import sys
import tempfile
import threading
import time
import tomllib
from collections.abc import Sequence
from pathlib import Path
from typing import Any

from . import freshness as fr
from . import link

REPO = Path(__file__).resolve().parents[3]
EXAMPLE = REPO / "examples" / "zk2" / "link"
RADIO = EXAMPLE / "radio.v1.toml"
FACE = EXAMPLE / "radio.face.toml"
ENROLLMENT = EXAMPLE / "radio.enrollment.toml"
PLAN_JSON = REPO / "spec" / "profiles" / "link" / "conformance" / "plan.json"
IFACE = "radio.v1"
#: The kind tokens a binding's default resources are of: "Every stream,
#: state and event (the default resources)".
BOUND_TOKENS = ("stream", "@stream", "state", "@state", "events")
#: §7, expected 1: "Every subject but ground's is selected by tcp, udp, tls,
#: quic, serial, unixpipe, vsock and ws", written as the scenario writes it.
NEAR = ("radio-a", "radio-b", "local", "ground-router")
NEAR_PROTOCOLS = ["tcp", "udp", "tls", "quic", "serial", "unixpipe", "vsock", "ws"]


#: ``policy()``'s "leave ``interval_s`` as the example writes it".
_KEEP = object()


class PolicyDrift(RuntimeError):
    """A section's change to ``radio.face.toml`` could not be applied: the
    example no longer has the line it rewrites."""


def policy(services: list[str] | None = None, *, far: str | None = None, attach: str | None = None,
           region: str | None = None, expose: str | None = None, interval_s: Any = _KEEP) -> str:
    """``radio.face.toml``, with a section's changes: each named key's line
    replaced, ``region`` added after ``attach``, ``interval_s`` removed with
    ``None``. Every requested change must find its line, and the result must
    read back with each requested value, else :class:`PolicyDrift`: a
    section never runs on the unmodified policy."""
    out = []
    changes: dict[str, Any] = {"far": far, "attach": attach, "expose": expose}
    requested = {k for k, v in changes.items() if v is not None}
    requested |= {"services"} if services is not None else set()
    requested |= {"region"} if region is not None else set()
    requested |= {"interval_s"} if interval_s is not _KEEP else set()
    applied: set[str] = set()
    for line in FACE.read_text().splitlines():
        key = line.split("=", 1)[0].strip() if "=" in line and not line.lstrip().startswith("#") else None
        if key == "services" and services is not None:
            line = f"services = {json.dumps(services)}"
            applied.add(key)
        elif key in changes and changes[key] is not None:
            line = f'{key:<12} = "{changes[key]}"'
            applied.add(key)
        elif key == "interval_s" and interval_s is not _KEEP:
            applied.add(key)
            if interval_s is None:
                continue
            line = f"interval_s   = {interval_s}"
        out.append(line)
        if key == "attach" and region is not None:
            out.append(f'region       = "{region}"')
            applied.add("region")
    missing = requested - applied
    if missing:
        raise PolicyDrift(f"{FACE}: no line for {sorted(missing)}, so the section's change cannot be applied")
    text = "\n".join(out) + "\n"
    doc = tomllib.loads(text)
    faces = doc.get("face") or {}
    if len(faces) != 1:
        raise PolicyDrift(f"{FACE}: the scenarios expect one face, found {sorted(faces)}")
    face = next(iter(faces.values()))
    want = {**{k: v for k, v in changes.items() if v is not None},
            **({"region": region} if region is not None else {})}
    wrong = {k: face.get(k) for k, v in want.items() if face.get(k) != v}
    if services is not None and doc.get("services") != services:
        wrong["services"] = doc.get("services")
    if interval_s is not _KEEP and face.get("interval_s") != interval_s:
        wrong["interval_s"] = face.get("interval_s")
    if wrong:
        raise PolicyDrift(f"{FACE}: the rewritten policy reads {wrong}, not the section's values")
    return text


class Report:
    def __init__(self) -> None:
        self.rows: list[tuple[str, str, bool, str]] = []
        self.lock = threading.Lock()

    def check(self, section: str, name: str, ok: bool, detail: str = "") -> bool:
        with self.lock:
            self.rows.append((section, name, ok, detail))
            print(f"{'PASS' if ok else 'FAIL'} [{section}] {name}" + (f": {detail}" if detail else ""), flush=True)
        return ok

    def note(self, section: str, text: str) -> None:
        """A measurement the text states no expectation for: printed, not
        counted."""
        with self.lock:
            print(f"NOTE [{section}] {text}", flush=True)


def _contract():
    from .contract import load_contract

    c = load_contract(RADIO)
    if not c.valid:
        raise RuntimeError(f"{RADIO}: {c.codes}")
    return c


def enrollment(contract: Any) -> tuple[list[Any], dict[str, Any]]:
    """``radio.enrollment.toml`` read into ``zk2py.acl``'s principals. The
    format is the reference generator's own (core §11.1, "The input"), read
    from the example as it reads: a principal runs ``services`` or
    ``tools``; a tool's ``bindings`` name an interface and its providers,
    and with no ``resources`` take every stream, state and event resource of
    it; its ``calls`` name operations. Returns the principals and, per tool,
    its (consume, call) uses. Anything it cannot resolve raises
    :class:`EnrollmentError`, naming it: an enrollment read wrongly would
    test a deployment that is not the example's."""
    from . import acl

    def need(table: dict[str, Any], key: str, where: str) -> Any:
        if key not in table:
            raise EnrollmentError(f"{ENROLLMENT}: {where} has no {key!r}")
        return table[key]

    def contract_of(iface: str, where: str) -> Any:
        if iface not in contracts:
            raise EnrollmentError(f"{ENROLLMENT}: {where} names interface {iface!r}, whose contract is not given "
                                  f"(given: {sorted(contracts)})")
        return contracts[iface]

    doc = tomllib.loads(ENROLLMENT.read_text())
    contracts = {contract.interface: contract}
    implements = {need(s, "address", "a [[service]]"): need(s, "implements", f"service {s.get('address')}")
                  for s in doc.get("service", [])}
    tools: dict[str, tuple[list[Any], list[Any]]] = {}
    for tool in doc.get("tool", []):
        tname = need(tool, "name", "a [[tool]]")
        consumes = []
        for role, b in (tool.get("bindings") or {}).items():
            where = f"tool {tname}'s binding {role}"
            iface = need(b, "interface", where)
            c = contract_of(iface, where)
            declared = {f"{r['token']}/{r['template']}" for r in c.canonical["resources"]
                        if r["token"] in BOUND_TOKENS}
            resources = b.get("resources") or sorted(declared)
            unknown = [r for r in resources if r not in declared]
            if unknown:
                raise EnrollmentError(f"{ENROLLMENT}: {where} binds {unknown}, which {iface} does not declare")
            consumes.append(acl.Use(iface, list(need(b, "providers", where)), resources))
        calls = []
        for cl in tool.get("calls", []):
            where = f"tool {tname}'s calls"
            iface = need(cl, "interface", where)
            c = contract_of(iface, where)
            names = {r["template"] for r in c.canonical["resources"] if r["token"] == "@op"}
            ops = list(need(cl, "operations", where))
            unknown = [op for op in ops if op not in names]
            if unknown:
                raise EnrollmentError(f"{ENROLLMENT}: {where} name operations {unknown}, which {iface} does not "
                                      f"declare (it declares {sorted(names)})")
            calls.append(acl.Use(iface, list(need(cl, "providers", where)), [f"@op/{op}" for op in ops]))
        tools[tname] = (consumes, calls)
    principals = []
    for pr in doc.get("principal", []):
        user = need(pr, "user", "a [[principal]]")
        services = pr.get("services") or []
        if len(services) > 1:
            raise EnrollmentError(f"{ENROLLMENT}: principal {user} runs {services}; zk2py.acl takes one service "
                                  "per principal")
        consumes, calls = [], []
        for name in pr.get("tools") or []:
            if name not in tools:
                raise EnrollmentError(f"{ENROLLMENT}: principal {user} runs tool {name!r}, which the enrollment "
                                      f"does not define (it defines {sorted(tools)})")
            consumes += tools[name][0]
            calls += tools[name][1]
        impl: list[Any] = []
        if services:
            if services[0] not in implements:
                raise EnrollmentError(f"{ENROLLMENT}: principal {user} runs service {services[0]!r}, which no "
                                      "[[service]] describes")
            impl = [contract_of(i, f"service {services[0]}") for i in implements[services[0]]]
        principals.append(acl.Principal(user, user, service=services[0] if services else None, implements=impl,
                                        consumes=consumes, calls=calls))
    return principals, tools


class EnrollmentError(RuntimeError):
    """``radio.enrollment.toml`` names something zk2py cannot resolve."""


def derive(policy_text: str, revisions: list[Any] | None = None) -> dict[str, Any]:
    """The plan, from the policy, the deployment and the contract (§2.3),
    or every revision of it given in ``revisions`` (0.3)."""
    if revisions is None:
        return link.plan(policy_text, {"radio-a": [IFACE], "radio-b": [IFACE]}, {IFACE: link.resources_of(_contract())})
    return link.plan(policy_text, {"radio-a": [IFACE], "radio-b": [IFACE]},
                     {IFACE: {"revisions": [link.resources_of(c) for c in revisions]}})


def minor1(workdir: str) -> Any:
    """§8's minor 1: "differs from it only in air/frames_total's
    annotations, { "link.exposure" = "link" }", written into ``workdir``."""
    from .contract import load_contract

    lines, out, seen = RADIO.read_text().splitlines(), [], set()
    in_frames = False
    for line in lines:
        if line.strip() == "minor = 0":
            line = "minor = 1"
            seen.add("minor")
        if line.startswith("["):
            in_frames = line.startswith('[resources."air/frames_total"]')
        out.append(line)
        if in_frames and line.startswith("type"):
            out.append('annotations = { "link.exposure" = "link" }')
            seen.add("annotations")
    if seen != {"minor", "annotations"}:
        raise PolicyDrift(f"{RADIO}: cannot derive minor 1, found {sorted(seen)}")
    path = Path(workdir) / "radio.v1.minor1.toml"
    path.write_text("\n".join(out) + "\n")
    c = load_contract(path)
    if not c.valid:
        raise RuntimeError(f"{path}: {c.codes}")
    return c


#: §7's blocks: "0.3" is the generator's; "0.2" is 0.2's (step 2); "no-face"
#: narrows and defines no subject of the face's own (step 4); "own" gives the
#: face's subject a policy of its own holding the plan's denies alone (step
#: 5), under deny too.
GENERATORS = {"0.3": {}, "no-face": {"face_subject": None}, "own": {"face_subject": "own", "any_posture": True}}


def check(block: dict[str, Any], policy_text: str,
          downsampling: Sequence[dict[str, Any]] | None = None) -> list[dict[str, Any]]:
    """A tool's §2.15 check of an ``access_control`` block for the radio
    face, with the downsampling items beside it."""
    pol, _ = link.parse_policy(policy_text)
    return link.check_router_block(block, pol, "radio", [x for ds in downsampling or [] for x in ds.get("rules", [])])


def generate_face(policy_text: str, face_plan: dict[str, Any], block: dict[str, Any], far: str,
                  generator: str) -> dict[str, Any]:
    if generator == "0.2":
        return link.compile_face(face_plan, block, far)
    pol, _ = link.parse_policy(policy_text)
    return link.compile_faces(pol, [face_plan], block, **GENERATORS[generator])


def mint_certificates(workdir: str, cns: list[str]) -> dict[str, str]:
    """§11's setup: "its certificate and a client certificate per CN … are
    signed by one CA minted for the run", by openssl, P-256 keys. Returns
    the paths: ``ca``, ``server``, ``server.key``, ``client-<cn>`` and
    ``client-<cn>.key``."""
    import subprocess

    def run(*args: str) -> None:
        done = subprocess.run(["openssl", *args], capture_output=True, text=True, cwd=workdir)
        if done.returncode != 0:
            raise RuntimeError(f"openssl {' '.join(args[:2])} failed ({done.returncode}): "
                               f"{(done.stderr or done.stdout).strip()}")

    run("req", "-x509", "-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:prime256v1", "-nodes", "-keyout", "ca.key",
        "-out", "ca.pem", "-days", "2", "-subj", "/CN=zk2py-link-ca")
    out = {"ca": os.path.join(workdir, "ca.pem")}

    def leaf(name: str, cn: str, ext: str) -> None:
        with open(os.path.join(workdir, f"{name}.ext"), "w") as f:
            f.write(ext)
        run("req", "-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:prime256v1", "-nodes", "-keyout", f"{name}.key",
            "-out", f"{name}.csr", "-subj", f"/CN={cn}")
        run("x509", "-req", "-in", f"{name}.csr", "-CA", "ca.pem", "-CAkey", "ca.key", "-CAcreateserial",
            "-out", f"{name}.pem", "-days", "2", "-extfile", f"{name}.ext")
        out[name], out[f"{name}.key"] = os.path.join(workdir, f"{name}.pem"), os.path.join(workdir, f"{name}.key")

    leaf("server", "localhost", "subjectAltName=IP:127.0.0.1,DNS:localhost\nextendedKeyUsage=serverAuth\n"
                                "basicConstraints=CA:FALSE\nkeyUsage=digitalSignature\n")
    for cn in cns:
        leaf(f"client-{cn}", cn, "extendedKeyUsage=clientAuth\nbasicConstraints=CA:FALSE\nkeyUsage=digitalSignature\n")
    return out


def open_tls(w: "World", user: str, cn: str) -> Any:
    """A client of N over TLS presenting ``cn``'s client certificate beside
    ``user``'s user name and password (§11's step 4)."""
    import zenoh

    conf = zenoh.Config()
    conf.insert_json5("mode", json.dumps("client"))
    conf.insert_json5("connect/endpoints", json.dumps([w.tls_endpoint]))
    conf.insert_json5("scouting/multicast/enabled", "false")
    conf.insert_json5("transport/auth/usrpwd", json.dumps({"user": user, "password": f"{user}-pw"}))
    conf.insert_json5("transport/link/tls", json.dumps({
        "root_ca_certificate": w.certs["ca"], "enable_mtls": True,
        "connect_private_key": w.certs[f"client-{cn}.key"], "connect_certificate": w.certs[f"client-{cn}"]}))
    return zenoh.open(conf)


def close_each(things: list[Any], what: str) -> None:
    """Close each of ``things``, closing the rest when one fails, and log
    the failures once, on stderr: a scenario's result does not turn on a
    close, but a failure to close is not hidden either."""
    failed = []
    for x in things:
        try:
            x.close()
        except Exception as e:  # noqa: BLE001 - closing the rest anyway
            failed.append(f"{type(e).__name__}: {e}")
    if failed:
        print(f"note: {len(failed)} of {len(things)} of {what} failed to close: {failed[0]}"
              + (f" (and {len(failed) - 1} more)" if len(failed) > 1 else ""), file=sys.stderr, flush=True)


def noted(report: "Report", sec: str, what: str, run: Any) -> Any:
    """Run a measurement whose result is only a NOTE: its failure is a NOTE
    too, never a failed section (review-bot on PR #794). Returns the
    result, or None."""
    try:
        return run()
    except Exception as e:  # noqa: BLE001 - a measurement that cannot run is a note
        report.note(sec, f"{what}: could not be measured ({type(e).__name__}: {e})")
        return None


class Tokens:
    def __init__(self, session: Any, selectors: list[str]):
        import zenoh

        self.rows: list[str] = []
        self.subs = [session.liveliness().declare_subscriber(s, zenoh.handlers.Callback(
            lambda smp: self.rows.append(str(smp.key_expr)) if smp.kind == zenoh.SampleKind.PUT else None),
            history=True) for s in selectors]


class World:
    """N with the generated block, G across the face (or F and G for a
    south region), L on the near side, and the owners on demand.
    ``queryable_grant`` (a south region): ``None`` for none, ``"selectors"``
    for G's own GET and call selectors, ``"owner"`` for §2.5's.
    ``face_local``: also X, a session with ``local``'s credentials over the
    face (§7). ``posture`` and ``generator`` (:data:`GENERATORS`, or
    ``"0.2"``) choose the block, and ``block_edit`` changes it by hand
    (§9 to §11); ``revisions`` are the contracts the plan is derived from
    (§8), the example's alone by default. ``tls``: N also listens on
    ``tls/127.0.0.1`` with mTLS, certificates minted for the run
    (:func:`mint_certificates`, §11's step 4)."""

    def __init__(self, policy_text: str, *, far: str = "ground", attach: str = "client",
                 queryable_grant: str | None = "owner", deny_override: tuple[str, list[str]] | None = None,
                 downsampling_override: list[dict[str, Any]] | None = None, tombstone_s: float = 60.0,
                 face_local: bool = False, posture: str = "deny", generator: str = "0.3",
                 revisions: list[Any] | None = None, block_edit: Any = None, tls: bool = False,
                 faces: str = "first"):
        import zenoh

        from . import acl, live
        from .owner import free_loopback_port

        self.contract = _contract()
        self.policy_text = policy_text
        p = derive(policy_text, revisions)
        if "refused" in p:
            raise ValueError(f"the generator refuses the policy: {p['refused']}")
        self.plan = p["faces"][0]
        self.tombstone_s = tombstone_s
        principals, tools = enrollment(self.contract)
        consumes, calls = tools["console"]
        self.selectors = [s for u in consumes for s in acl.use_selectors(u)]
        block = acl.generate(principals, posture).block
        if faces == "all":
            # §13: the generator compiles every face of the policy, each
            # face's denies into its far principal's policy (§2.4, 0.7).
            pol, _ = link.parse_policy(policy_text)
            block = link.compile_faces(pol, p["faces"], block, **GENERATORS[generator])
        else:
            block = generate_face(policy_text, self.plan, block, far, generator)
        if block_edit is not None:
            block = block_edit(block)
        if deny_override is not None:
            rid, keys = deny_override
            for r in block["rules"]:
                if r.get("id") == f"face-{self.plan['name']}-{rid}":
                    r["key_exprs"] = keys
        self.queryable_keys: list[str] = []
        if attach == "south-region" and queryable_grant:
            if queryable_grant == "owner":
                # §2.5 (0.2): "over every queryable its side may query, as
                # the owner declares it".
                keys = link.declaration_grant(
                    [(u.interface, u.providers) for u in consumes
                     if any(r.split("/", 1)[0] in ("state", "@state") for r in u.resources)],
                    [(u.interface, u.providers, r.split("/", 1)[1]) for u in calls for r in u.resources])
            else:
                # scenarios.md §2 step 3: "G's own GET and call selectors
                # only, …/state/peers/* and …/@op/reset".
                keys = [f"zk2/lab/*/{IFACE}/state/peers/*", f"zk2/lab/*/{IFACE}/@op/reset"]
            self.queryable_keys = keys
            block["rules"].append({"id": "far-queryables", "messages": ["declare_queryable"], "flows": ["egress"],
                                   "permission": "allow", "key_exprs": keys})
            for pol in block["policies"]:
                if far in pol.get("subjects", []):
                    pol["rules"].append("far-queryables")
        # A hand edit that leaves a block zenoh refuses at load says so here.
        link.check_ids(block["subjects"], block["policies"], block["rules"])
        self.block = block
        extras = link.router_extras(self.plan)
        if downsampling_override is not None:
            extras["downsampling"] = downsampling_override
        self.downsampling = extras.get("downsampling", [])
        self.workdir = tempfile.mkdtemp(prefix="zk2py-link-")
        self.sock = os.path.join(self.workdir, "face.sock")
        users = os.path.join(self.workdir, "users.txt")
        with open(users, "w") as f:
            for pr in principals:
                f.write(f"{pr.user}:{pr.user}-pw\n")
        port = free_loopback_port()
        self.near = f"tcp/127.0.0.1:{port}"
        conf = zenoh.Config()
        conf.insert_json5("mode", json.dumps("router"))
        listen = [self.near, f"unixsock-stream/{self.sock}"]
        self.tls_endpoint: str | None = None
        if tls:
            self.certs = mint_certificates(self.workdir, ["local", "other"])
            self.tls_endpoint = f"tls/127.0.0.1:{free_loopback_port()}"
            listen.append(self.tls_endpoint)
        conf.insert_json5("listen/endpoints", json.dumps(listen))
        conf.insert_json5("scouting/multicast/enabled", "false")
        conf.insert_json5("timestamping/enabled", "true")
        conf.insert_json5("transport/auth/usrpwd", json.dumps({"dictionary_file": users}))
        if tls:
            conf.insert_json5("transport/link/tls", json.dumps({
                "root_ca_certificate": self.certs["ca"], "listen_private_key": self.certs["server.key"],
                "listen_certificate": self.certs["server"], "enable_mtls": True}))
        conf.insert_json5("access_control", json.dumps(block))
        for k, v in extras.items():
            conf.insert_json5(k, json.dumps(v))
        self.closers: list[Any] = []
        self.far_router = None
        self.far, self.attach = far, attach
        self.g = self.l = self.gs = self.ls = self.fl = self.fls = self.flt = None
        self.owners: dict[str, Any] = {}
        self.n = zenoh.open(conf)
        try:
            self.g = self._open_g()
            self.l = live.open_client(self.near, ("local", "local-pw"))
            self.gs = fr.Subscriber(self.g, self.selectors)
            self.ls = fr.Subscriber(self.l, self.selectors)
            presence = ["zk2/lab/*/@zk/**"]
            self.gt, self.lt = Tokens(self.g, presence), Tokens(self.l, presence)
            if face_local:
                self.fl = live.open_client(f"unixsock-stream/{self.sock}", ("local", "local-pw"))
                self.fls = fr.Subscriber(self.fl, self.selectors)
                self.flt = Tokens(self.fl, presence)
        except BaseException:
            # A world that cannot start closes what it opened, N first among
            # them, before the error goes on.
            self.close()
            raise
        time.sleep(0.5)

    def _open_g(self) -> Any:
        import zenoh

        from . import live
        from .owner import free_loopback_port

        if self.attach == "client":
            return live.open_client(f"unixsock-stream/{self.sock}", (self.far, f"{self.far}-pw"))
        if self.far_router is None:
            fport = free_loopback_port()
            conf = zenoh.Config()
            conf.insert_json5("mode", json.dumps("router"))
            conf.insert_json5("listen/endpoints", json.dumps([f"tcp/127.0.0.1:{fport}"]))
            conf.insert_json5("connect/endpoints", json.dumps([f"unixsock-stream/{self.sock}"]))
            conf.insert_json5("scouting/multicast/enabled", "false")
            conf.insert_json5("timestamping/enabled", "true")
            conf.insert_json5("region_name", json.dumps("ground"))
            # F authenticates to N as the far principal; with usrpwd on, it
            # authenticates its own clients too, so G is a user of F's.
            fusers = os.path.join(self.workdir, "far-users.txt")
            with open(fusers, "w") as f:
                f.write("g:g-pw\n")
            conf.insert_json5("transport/auth/usrpwd", json.dumps({"user": self.far, "password": f"{self.far}-pw",
                                                                   "dictionary_file": fusers}))
            self.far_router = (zenoh.open(conf), f"tcp/127.0.0.1:{fport}")
            time.sleep(0.5)
        return live.open_client(self.far_router[1], ("g", "g-pw"))

    def owner(self, name: str = "radio-a", contract: Any = None) -> Any:
        from .owner import Owner

        o = Owner("lab", name, [contract or self.contract], connect=self.near, auth=(name, f"{name}-pw"),
                  tombstone_s=self.tombstone_s)
        o.start()
        self.owners[name] = o
        time.sleep(0.5)
        return o

    def policy(self) -> Any:
        return link.parse_policy(self.policy_text)[0]

    def check_block(self) -> list[dict[str, Any]]:
        """§2.15 on the block N runs (:func:`zk2py.link.check_router_block`:
        each subject that can match the far principal's session checked
        alone, 0.4), sorted as ``lints.json`` sorts findings."""
        return check(self.block, self.policy_text, self.downsampling)

    def g_count(self, key: str) -> int:
        return sum(1 for x in self.gs.of(key) if x.kind == "put")

    def l_count(self, key: str) -> int:
        return sum(1 for x in self.ls.of(key) if x.kind == "put")

    def g_events(self, base: str, name: str, s: Any = None) -> list[str]:
        s = s or self.gs
        prefix = f"{base}/events/{name}/"
        with s.lock:
            return [k for k in s.deliveries if k.startswith(prefix)]

    def close(self) -> None:
        """Close whatever the world holds, a world whose start failed among
        them: subscribers, owners, sessions, then the routers."""
        close_each([x for x in (self.gs, self.ls, self.fls) if x is not None], "the subscribers")
        close_each(list(self.owners.values()), "the owners")
        close_each([x for x in (self.g, self.l, self.fl) if x is not None]
                   + ([self.far_router[0]] if self.far_router else []) + [self.n], "the sessions and routers")


def base_of(name: str = "radio-a") -> str:
    return f"zk2/lab/{name}/{IFACE}"


def traffic(o: Any, name: str = "radio-a", *, streams: tuple[str, ...] = ("all",), seconds: float = 5.0,
            oper: bool = True, events: bool = True) -> None:
    """"The owner's traffic": for 5 s the streams at 20 Hz (the @streams with
    1 KB payloads), oper put 10 times "one every half second" (0.2), peers/x
    put, and fault and change 5 times each within one second."""
    base = base_of(name)
    stop = threading.Event()
    which = ["stream/air/dropped_total", "stream/air/frames_total", "@stream/frames", "@stream/raw"] \
        if streams == ("all",) else list(streams)

    def pump() -> None:
        t_end = time.monotonic() + seconds
        i = 0
        while time.monotonic() < t_end and not stop.is_set():
            for w in which:
                o.publish(f"{base}/{w}", (b"x" * 1024) if w.startswith("@stream") else str(i).encode())
            i += 1
            time.sleep(0.05)

    th = threading.Thread(target=pump, daemon=True)
    th.start()
    if events:
        for i in range(5):
            o.occur(f"{base}/events/fault", f"fault {i}".encode())
            o.occur(f"{base}/events/change", f"change {i}".encode())
            time.sleep(0.1)
    if oper:
        o.set_state(f"{base}/state/peers/x", b"peer x")
        for i in range(10):
            o.set_state(f"{base}/state/oper", f"oper {i}".encode())
            time.sleep(0.5)
    th.join()
    time.sleep(1.0)


# -- §1 ------------------------------------------------------------------------------

def _same_denies(plan_face: dict[str, Any], case: int = 2) -> bool:
    want = json.loads(PLAN_JSON.read_text())["cases"][case]["expect"]["faces"][0]
    return plan_face["deny"] == want["deny"] and [r["key_expr"] for r in (plan_face["downsampling"] or {}).get(
        "rules", [])] == [r["key_expr"] for r in want["downsampling"]["rules"]]


def section1(report: Report) -> None:
    from . import live

    sec = "§1 a face compiled and enforced, the far side a client"
    w = World(policy())
    try:
        ground_rules = {r for pol in w.block["policies"] if "ground" in pol["subjects"] for r in pol["rules"]}
        denies = [d["id"] for d in w.plan["deny"]]
        report.check(sec, "step 1: the block holds the plan: the far principal's policy carries zk, adv, admin and "
                          "the exposure denies, and the downsampling block one rule per crossing stream of "
                          "lab/radio-a at 1 Hz, as plan.json's third case derives them (§2.4, §2.8)",
                     all(f"face-radio-{d}" in ground_rules for d in denies) and denies[:3] == ["zk", "adv", "admin"]
                     and _same_denies(w.plan) and all(r["freq"] == 1.0 for r in w.plan["downsampling"]["rules"]),
                     f"denies {denies}; rules {[r['key_expr'] for r in w.plan['downsampling']['rules']]}")
        selected = {s["id"]: s.get("link_protocols") for s in w.block["subjects"]}
        report.check(sec, "step 1 (0.3): every subject but ground's is selected by the link protocols other than "
                          "unixsock-stream (§2.4)", selected == {**{p: NEAR_PROTOCOLS for p in NEAR}, "ground": None},
                     json.dumps(selected))
        o = w.owner()
        traffic(o)
        b = base_of()
        got = {k: w.g_count(f"{b}/{k}") for k in ("stream/air/dropped_total", "@stream/frames",
                                                    "stream/air/frames_total", "@stream/raw")}
        opers = {x.payload for x in w.gs.of(f"{b}/state/oper") if x.kind == "put"}
        l_got = {k: w.l_count(f"{b}/{k}") for k in got}
        faults, changes = w.g_events(b, "fault"), w.g_events(b, "change")
        report.check(sec, "step 2: L receives every sample, about 100 of each stream; G receives air/dropped_total "
                          "and frames 4 to 6 times each, nothing of air/frames_total nor raw, all 10 puts of oper, "
                          "all 5 occurrences of fault and nothing of change (§2.7 to §2.9)",
                     all(90 <= n <= 110 for n in l_got.values())
                     and 4 <= got["stream/air/dropped_total"] <= 6 and 4 <= got["@stream/frames"] <= 6
                     and got["stream/air/frames_total"] == 0 and got["@stream/raw"] == 0
                     and len([p for p in opers if p.startswith(b"oper ")]) == 10
                     and len(faults) == 5 and not changes,
                     f"G {got}, oper {len(opers)} values, fault {len(faults)}, change {len(changes)}; L {l_got}")
        out = {}
        for who, s in (("G", w.g), ("L", w.l)):
            st = live.get_state(s, f"{b}/state/peers/x")
            res = live.call(s, f"{b}/@op/reset", b"now", first=True)
            out[who] = ([r.payload for r in st.replies], [r.payload for r in res.replies if r.kind == "value"])
        report.check(sec, "step 3: G's GET of peers/x is answered with its value, and its call with the reply: "
                          "requests cross by grant (§2.9); L's are answered too",
                     all(v == ([b"peer x"], [b"now"]) for v in out.values()), str(out))
        g_desc = live.get_descriptor(w.g, o.instance_key)
        report.check(sec, "step 4: L sees the owner's instance and interface tokens; G sees none, and no descriptor "
                          "reaches it (§2.6)",
                     any("/@zk/instance/" in k for k in w.lt.rows) and any("/@zk/alive/" in k for k in w.lt.rows)
                     and not w.gt.rows and not [d for d in g_desc if d.ok],
                     f"L {len(w.lt.rows)} tokens, G {len(w.gt.rows)} tokens, G descriptor {len(g_desc)} replies")
        found = w.check_block()
        report.check(sec, "step 5: a tool checking the block reports no finding (§2.15)", not found, str(found))
    finally:
        w.close()


# -- §2 ------------------------------------------------------------------------------

def section2(report: Report) -> None:
    from . import live

    sec = "§2 a far router in a south region"
    pol = policy(far="ground-router", attach="south-region", region="ground")
    for grant in ("owner", None, "selectors"):
        w = World(pol, far="ground-router", attach="south-region", queryable_grant=grant)
        try:
            o = w.owner()
            b = base_of()
            if grant == "owner":
                report.check(sec, "step 1: the block's gateway is the plan's: N's own peers and clients south, then "
                                  "the region ground (§2.5)",
                             w.plan["attach"]["gateway"] == {"south": [{"filters": [{"modes": ["peer", "client"]}]},
                                                                       {"filters": [{"region_names": ["ground"]}]}]},
                             json.dumps(w.plan["attach"]))
                traffic(o)
                got = {k: w.g_count(f"{b}/{k}") for k in ("stream/air/dropped_total", "@stream/frames",
                                                            "stream/air/frames_total", "@stream/raw")}
                opers = {x.payload for x in w.gs.of(f"{b}/state/oper") if x.kind == "put"}
                faults, changes = w.g_events(b, "fault"), w.g_events(b, "change")
                report.check(sec, "step 1: G, a client of F, receives what §1's G receives",
                             4 <= got["stream/air/dropped_total"] <= 6 and 4 <= got["@stream/frames"] <= 6
                             and got["stream/air/frames_total"] == 0 and got["@stream/raw"] == 0
                             and len([p for p in opers if p.startswith(b"oper ")]) == 10
                             and len(faults) == 5 and not changes,
                             f"G {got}, oper {len(opers)}, fault {len(faults)}, change {len(changes)}")
            else:
                o.set_state(f"{b}/state/peers/x", b"peer x")
                time.sleep(0.5)
            st = live.get_state(w.g, f"{b}/state/peers/x")
            res = live.call(w.g, f"{b}/@op/reset", b"now", first=True)
            answered = ([r.payload for r in st.replies], [r.payload for r in res.replies if r.kind == "value"])
            if grant == "owner":
                report.check(sec, "step 1: with §2.5's declaration grant (0.2), over every queryable G may query as "
                                  "the owner declares it, G sees no token and no descriptor, and its GET and its call "
                                  "are answered",
                             not w.gt.rows and not [d for d in live.get_descriptor(w.g, o.instance_key) if d.ok]
                             and answered == ([b"peer x"], [b"now"]),
                             f"grant {w.queryable_keys}; G tokens {len(w.gt.rows)}; {answered}")
            elif grant is None:
                report.check(sec, "step 2 (control): without the far principal's declare_queryable egress grant, G's "
                                  "GET and its call get no reply: F learns of no queryable (core §8.5)",
                             answered == ([], []), str(answered))
            else:
                report.check(sec, "step 3 (control, 0.2): the grant over G's own GET and call selectors only, "
                                  "…/state/peers/* and …/@op/reset: G's call is answered and its GET is not, the "
                                  "declaration being checked by inclusion against the owner's state/** (core §11.3)",
                             answered == ([], [b"now"]), f"grant {w.queryable_keys}; {answered}")
        finally:
            w.close()
    refused = derive(policy(far="ground-router", attach="router"))
    report.check(sec, "step 4 (control): with attach = \"router\", the generator refuses the policy (router_attach) "
                      "and writes no block",
                 refused.get("refused") == [link.finding("router_attach", "radio", "attach")], json.dumps(refused))


# -- §3 ------------------------------------------------------------------------------

def _two(w: World) -> tuple[Any, Any]:
    oa, ob = w.owner("radio-a"), w.owner("radio-b")
    stop = threading.Event()

    def pump(o: Any, name: str) -> None:
        t_end = time.monotonic() + 5.0
        i = 0
        while time.monotonic() < t_end:
            o.publish(f"{base_of(name)}/stream/air/dropped_total", str(i).encode())
            i += 1
            time.sleep(0.05)

    ts = [threading.Thread(target=pump, args=(o, n)) for o, n in ((oa, "radio-a"), (ob, "radio-b"))]
    for t in ts:
        t.start()
    for t in ts:
        t.join()
    stop.set()
    time.sleep(1.0)
    return w.g_count(f"{base_of('radio-a')}/stream/air/dropped_total"), \
        w.g_count(f"{base_of('radio-b')}/stream/air/dropped_total")


def section3(report: Report) -> None:
    sec = "§3 one rule per key, and a service not listed"
    w = World(policy(["radio-a", "radio-b"]))
    try:
        keys = [r["key_expr"] for r in w.plan["downsampling"]["rules"] if r["key_expr"].endswith("dropped_total")]
        a, b = _two(w)
        report.check(sec, "step 1: the block has a rule for each owner's key, and G receives 4 to 6 samples from "
                          "each owner: each rule has its own timer (§2.8)",
                     keys == [f"{base_of('radio-a')}/stream/air/dropped_total",
                              f"{base_of('radio-b')}/stream/air/dropped_total"] and 4 <= a <= 6 and 4 <= b <= 6,
                     f"rules {keys}; G {a} from radio-a, {b} from radio-b")
    finally:
        w.close()
    wild = [{"id": "radio", "link_protocols": ["unixsock-stream"], "flows": ["egress"], "messages": ["put"],
             "rules": [{"key_expr": "zk2/lab/*/radio.v1/stream/air/dropped_total", "freq": 1.0}]}]
    w = World(policy(["radio-a", "radio-b"]), downsampling_override=wild)
    try:
        a, b = _two(w)
        found = w.check_block()
        report.check(sec, "step 2 (control): one wildcard rule: G receives 4 to 6 samples in all, from the two "
                          "owners together, one timer for every key it covers; a tool reports wildcard_downsampling",
                     4 <= a + b <= 6 and [f["code"] for f in found] == ["wildcard_downsampling"],
                     f"G {a} + {b}; {found}")
    finally:
        w.close()
    w = World(policy(["radio-a"]))
    try:
        a, b = _two(w)
        report.check(sec, "step 3: with radio-a alone listed, G receives 4 to 6 samples from radio-a and about 100 "
                          "from radio-b: an exposed stream of a service not listed is in no rule, and crosses at its "
                          "owner's rate (§2.8)", 4 <= a <= 6 and 90 <= b <= 110, f"G {a} from radio-a, {b} from radio-b")
    finally:
        w.close()


# -- §4 ------------------------------------------------------------------------------

def _events(w: World) -> tuple[int, int, int, int]:
    o = w.owner()
    b = base_of()
    for i in range(5):
        o.occur(f"{b}/events/change", f"change {i}".encode())
        o.occur(f"{b}/events/fault", f"fault {i}".encode())
        time.sleep(0.05)
    time.sleep(1.5)
    return (len(w.g_events(b, "fault")), len(w.g_events(b, "change")),
            len(w.g_events(b, "fault", w.ls)), len(w.g_events(b, "change", w.ls)))


def section4(report: Report) -> None:
    sec = "§4 an event that must not cross"
    w = World(policy())
    try:
        gf, gc, lf, lc = _events(w)
        report.check(sec, "step 1: G receives the 5 occurrences of fault, each on its own …/events/fault/<ulid> key, "
                          "none dropped by the 1 s interval, and none of change; L receives all 10 (§2.9)",
                     gf == 5 and gc == 0 and lf == 5 and lc == 5, f"G fault {gf}, change {gc}; L {lf} + {lc}")
    finally:
        w.close()
    w = World(policy(), deny_override=("events:radio.v1", ["zk2/*/*/radio.v1/stream/change/**"]))
    try:
        gf, gc, _, _ = _events(w)
        report.check(sec, "step 2 (control): the sketch's spelling, zk2/*/*/radio.v1/stream/change/**, in place of "
                          "the plan's events deny: G receives the 5 occurrences of change too",
                     gf == 5 and gc == 5, f"G fault {gf}, change {gc}")
    finally:
        w.close()
    pol = policy(expose="none", interval_s=None)
    w = World(pol)
    try:
        gf, gc, _, _ = _events(w)
        report.check(sec, "step 3: with expose = \"none\", G receives no occurrence at all; the plan denies "
                          "zk2/*/*/*/events/**",
                     gf == 0 and gc == 0 and {"id": "events", "flows": ["egress", "ingress"],
                                              "messages": ["put", "delete", "declare_subscriber"],
                                              "key_exprs": ["zk2/*/*/*/events/**"]} in w.plan["deny"],
                     f"G fault {gf}, change {gc}")
    finally:
        w.close()


# -- §5 ------------------------------------------------------------------------------

def _seed(session: Any, held: dict[str, tuple[int, bytes | None]]) -> None:
    """A late joiner's seed (core §4.2) of the granted state selectors,
    merged into ``held`` by stamp: a reply_del removes a member."""
    from . import live

    for sel in (f"{base_of()}/state/oper", f"{base_of()}/state/peers/*"):
        for r in live.get_state(session, sel).replies:
            stamp = r.stamp_ns or 0
            if r.key in held and held[r.key][0] >= stamp:
                continue
            held[r.key] = (stamp, None if r.deleted else r.payload)


def section5(report: Report) -> None:
    from . import live

    sec = "§5 a state window raised to the outage"
    for window in (6.0, 2.0):
        w = World(policy(), tombstone_s=window)
        try:
            o = w.owner()
            b = base_of()
            o.set_state(f"{b}/state/oper", b"oper 1")
            o.set_state(f"{b}/state/peers/x", b"peer x")
            time.sleep(0.5)
            held: dict[str, tuple[int, bytes | None]] = {}
            _seed(w.g, held)
            standing = sorted(k for k, (_, v) in held.items() if v is not None)
            # The face goes down: G's session closes, for 4 s.
            old = w.gs
            w.gs.close()
            w.g.close()
            t0 = time.monotonic()
            o.delete_state(f"{b}/state/oper")
            o.delete_state(f"{b}/state/peers/x")
            time.sleep(max(0.0, t0 + 4.0 - time.monotonic()))
            w.g = w._open_g()
            w.gs = fr.Subscriber(w.g, w.selectors)
            time.sleep(0.3)
            dels = [r for sel in (f"{b}/state/oper", f"{b}/state/peers/*")
                    for r in live.get_state(w.g, sel).replies if r.deleted]
            _seed(w.g, held)
            after = sorted(k for k, (_, v) in held.items() if v is not None)
            both = [f"{b}/state/oper", f"{b}/state/peers/x"]
            if window == 6.0:
                report.check(sec, "steps 1 to 3: G holds both members; during a 4 s outage it receives nothing; "
                                  "seeding again, its GET answers a reply_del for each, with the deletion's stamp, "
                                  "and the merge removes both (§2.11, core S3)",
                             standing == both and not [x for k in both for x in old.of(k) if x.kind == "delete"]
                             and len(dels) == 2 and after == [],
                             f"standing {len(standing)}, reply_del {len(dels)}, after {after}")
            else:
                found = link.lint(policy(), implements={"radio-a": [IFACE]},
                                  interfaces={IFACE: link.resources_of(w.contract)},
                                  windows={"lab/radio-a": 2})
                report.check(sec, "step 4 (control): with a 2 s window, G's GET answers nothing for the two, and "
                                  "its merge leaves both standing, a lost delete; the linter reports "
                                  "window_under_outage for lab/radio-a",
                             standing == both and not dels and after == both
                             and found == [link.finding("window_under_outage", "radio", "lab/radio-a")],
                             f"reply_del {len(dels)}, standing {after}; {found}")
        finally:
            w.close()


# -- §6 ------------------------------------------------------------------------------

def section6(report: Report) -> None:
    sec = "§6 liveness across the face"
    for expose in ("link", "none"):
        pol = policy() if expose == "link" else policy(expose="none", interval_s=None)
        w = World(pol)
        try:
            ann = {f"{IFACE}/{r['token']}/{r['template']}": r["annotations"] for r in link.resources_of(w.contract)}
            o = w.owner()
            b = base_of()
            o.set_state(f"{b}/state/oper", b"oper")
            stop = threading.Event()

            def pump() -> None:
                while not stop.wait(0.05):
                    o.publish(f"{b}/stream/air/dropped_total", b"1")

            th = threading.Thread(target=pump, daemon=True)
            th.start()
            time.sleep(5.0)
            first = link.liveness_across(w.plan, "lab/radio-a", w.gs, ann)
            evidence = next(x["evidence"] for x in w.plan["liveness"] if x["service"] == "lab/radio-a")
            if expose == "link":
                report.check(sec, "step 1: the plan's evidence is radio.v1/state/oper and "
                                  "radio.v1/stream/air/dropped_total, both fresh at G at 5 s: live",
                             evidence == ["radio.v1/state/oper", "radio.v1/stream/air/dropped_total"]
                             and first[0] == "live", f"evidence {evidence}; {first}")
                time.sleep(5.0)
                stop.set()
                o.close()
                w.owners.clear()
                time.sleep(6.0)
                second = link.liveness_across(w.plan, "lab/radio-a", w.gs, ann)
                report.check(sec, "step 2: 6 s after the owner closes, both are stale: liveness unobservable, never "
                                  "down (core R7)", second[0] == "unobservable", str(second))
            else:
                stop.set()
                got = sum(len(w.gs.of(f"{b}/{k}")) for k in ("state/oper", "stream/air/dropped_total"))
                report.check(sec, "step 3: with expose = \"none\" the plan lists no evidence, G receives nothing of "
                                  "the owner, and liveness is unobservable from the start",
                             evidence == [] and got == 0 and first[0] == "unobservable", f"G {got}; {first}")
        finally:
            w.close()


# -- §7 ------------------------------------------------------------------------------

STREAMS = ("stream/air/dropped_total", "@stream/frames", "stream/air/frames_total", "@stream/raw")


def _receives(w: World, s: Any, t: Tokens) -> dict[str, Any]:
    """What a subscriber received of the owner's traffic, and the tokens its
    liveliness subscriber saw."""
    b = base_of()
    d: dict[str, Any] = {k: sum(1 for x in s.of(f"{b}/{k}") if x.kind == "put") for k in STREAMS}
    d["oper"] = len({x.payload for x in s.of(f"{b}/state/oper") if x.kind == "put" and x.payload.startswith(b"oper ")})
    d["fault"], d["change"] = len(w.g_events(b, "fault", s)), len(w.g_events(b, "change", s))
    d["tokens"] = sorted({k.split("/@zk/", 1)[1].split("/", 1)[0] for k in t.rows})
    return d


def _asks(session: Any, o: Any) -> dict[str, Any]:
    """§7 step 1: GET …/state/peers/x, call …/@op/reset, read
    zk2/lab/*/@zk/** by liveliness and GET the owner's descriptor."""
    from . import live

    b = base_of()
    st = live.get_state(session, f"{b}/state/peers/x")
    res = live.call(session, f"{b}/@op/reset", b"now", first=True)
    pres = live.list_presence(session, "zk2/lab/*/@zk/**")
    return {"get": [r.payload for r in st.replies], "call": [r.payload for r in res.replies if r.kind == "value"],
            "liveliness": len(pres.instances) + len(pres.alive) + len(pres.members) + len(pres.other),
            "descriptor": len([d for d in live.get_descriptor(session, o.instance_key) if d.ok])}


def _puts(w: World, sessions: dict[str, Any]) -> dict[str, int]:
    """§7 step 1: "Then, the owner silent, each puts five times on
    zk2/lab/radio-a/radio.v1/stream/air/dropped_total, a key only lab/radio-a
    may write … and L counts what reaches it"."""
    key = f"{base_of()}/stream/air/dropped_total"
    for who, s in sessions.items():
        for i in range(5):
            s.put(key, f"put {who} {i}".encode())
            time.sleep(0.05)
    time.sleep(1.0)
    return {who: sum(1 for x in w.ls.of(key) if x.payload.startswith(f"put {who} ".encode())) for who in sessions}


def _anonymous(endpoint: str, timeout_s: float = 10.0) -> tuple[str, Any]:
    """A session over the face presenting no credential at all: whether N
    opens it (core §11.3: "A deployment under allow MUST refuse
    unauthenticated sessions at the link")."""
    from . import live

    box: dict[str, Any] = {}

    def run() -> None:
        try:
            box["s"] = live.open_client(endpoint, None)
        except Exception as e:  # noqa: BLE001 - a refusal is the measurement
            box["e"] = f"{type(e).__name__}: {e}"

    th = threading.Thread(target=run, daemon=True)
    th.start()
    th.join(timeout_s)
    if "s" in box:
        return "opened", box["s"]
    return ("refused", box["e"]) if "e" in box else ("no answer", None)


#: A key no contract declares, outside every face deny: under ``allow`` it
#: "stays open" (core §11.2), so a put on it shows a session is live.
PROBE_KEY = "zk2py-probe"
#: How long :func:`_alive` waits for every probe to reach L. It assumes a
#: subscriber's declaration and a put each cross N, on loopback, well
#: within it; a probe that arrives later counts as not arrived.
PROBE_DEADLINE_S = 5.0
#: How often :func:`_alive` puts each probe again while it waits.
PROBE_EVERY_S = 0.1


def _alive(w: World, sessions: dict[str, Any]) -> dict[str, dict[str, int]]:
    """Under ``allow``: each session puts on ``zk2py-probe-<run>/<who>``, a
    key no contract declares, with a suffix unique to this run, and L
    subscribes to it. The puts repeat every :data:`PROBE_EVERY_S` until L
    has one from every session or :data:`PROBE_DEADLINE_S` passes, so no
    fixed sleep has to cover the subscriber's declaration. Returns 1 for a
    session whose probe reached L, else 0, as ``arrived``, beside
    ``attempts``, the puts it took. A session the block denies all of the
    deployment's keys still reaches L here, so "nothing" is the block's,
    not a dead session's."""
    import uuid

    import zenoh

    root = f"{PROBE_KEY}-{uuid.uuid4().hex[:12]}"
    arrived: set[str] = set()
    attempts = {who: 0 for who in sessions}
    lock = threading.Lock()

    def seen(smp: Any) -> None:
        with lock:
            arrived.add(str(smp.key_expr).rsplit("/", 1)[-1])

    sub = w.l.declare_subscriber(f"{root}/**", zenoh.handlers.Callback(seen))
    try:
        deadline = time.monotonic() + PROBE_DEADLINE_S
        while time.monotonic() < deadline:
            with lock:
                waiting = [who for who in sessions if who not in arrived]
            if not waiting:
                break
            for who in waiting:
                sessions[who].put(f"{root}/{who}", b"alive")
                attempts[who] += 1
            time.sleep(PROBE_EVERY_S)
    finally:
        sub.undeclare()
    with lock:
        return {who: {"arrived": int(who in arrived), "attempts": attempts[who]} for who in sessions}


def _run7(posture: str, generator: str = "0.3", *, anonymous: bool = False, block_edit: Any = None,
          alive: bool = False) -> dict[str, Any]:
    """One block of §7 (and of §9 to §11, ``block_edit`` changing it by
    hand): the owner's traffic, then G's and X's requests and puts, then a
    tool's check of the block; with ``alive``, :func:`_alive` for G and X."""
    w = World(policy(), posture=posture, generator=generator, face_local=True, block_edit=block_edit)
    try:
        o = w.owner()
        traffic(o)
        r: dict[str, Any] = {"block": w.block, "found": w.check_block(),
                             "compare": link.compare_block(w.policy(), [w.plan], w.block)}
        r["G"], r["X"], r["L"] = _receives(w, w.gs, w.gt), _receives(w, w.fls, w.flt), _receives(w, w.ls, w.lt)
        r["G"].update(_asks(w.g, o))
        r["X"].update(_asks(w.fl, o))
        sessions = {"G": w.g, "X": w.fl}
        if anonymous:
            r["Y_status"], y = _anonymous(f"unixsock-stream/{w.sock}")
            if r["Y_status"] == "opened":
                sessions["Y"] = y
                r["Y"] = _asks(y, o)
                w.closers.append(y)
            else:
                r["Y_why"] = y
        puts = _puts(w, sessions)
        for who, n in puts.items():
            r.setdefault(who, {})["puts_to_L"] = n
        if alive:
            for who, probe in _alive(w, {"G": w.g, "X": w.fl}).items():
                r[who]["alive"] = probe
        return r
    finally:
        _close_all(w)


def _like_section1_g(d: dict[str, Any]) -> bool:
    """"What §1's G receives": air/dropped_total and frames 4 to 6 times,
    nothing of air/frames_total nor raw, all 10 puts of oper, all 5 fault
    occurrences and nothing of change."""
    return (4 <= d["stream/air/dropped_total"] <= 6 and 4 <= d["@stream/frames"] <= 6
            and d["stream/air/frames_total"] == 0 and d["@stream/raw"] == 0 and d["oper"] == 10
            and d["fault"] == 5 and d["change"] == 0)


def _answered(d: dict[str, Any]) -> bool:
    return d["get"] == [b"peer x"] and d["call"] == [b"now"]


def _nothing(d: dict[str, Any]) -> bool:
    return (all(d[k] == 0 for k in STREAMS + ("oper", "fault", "change", "liveliness", "descriptor"))
            and not d["tokens"] and d["get"] == [] and d["call"] == [])


def _local_share(d: dict[str, Any]) -> bool:
    """§7 expected 2: "about 100 of air/frames_total and of raw, all 5
    occurrences of change, the owner's two tokens, and its GET and call
    answered. The face's downsampling still applies to it"."""
    return (90 <= d["stream/air/frames_total"] <= 110 and 90 <= d["@stream/raw"] <= 110 and d["change"] == 5
            and {"instance", "alive"} <= set(d["tokens"]) and _answered(d)
            and 4 <= d["stream/air/dropped_total"] <= 6 and 4 <= d["@stream/frames"] <= 6)


def _brief(d: dict[str, Any]) -> str:
    keys = {"stream/air/dropped_total": "dropped_total", "@stream/frames": "frames",
            "stream/air/frames_total": "frames_total", "@stream/raw": "raw"}
    parts = [f"{v} {d[k]}" for k, v in keys.items() if k in d]
    parts += [f"{k} {d[k]}" for k in ("oper", "fault", "change") if k in d]
    if "tokens" in d:
        parts.append(f"tokens {d['tokens'] or 'none'}")
    for k in ("liveliness", "descriptor"):
        if k in d:
            parts.append(f"{k} {d[k]}")
    if "get" in d:
        parts.append(f"GET {d['get']}, call {d['call']}")
    if "puts_to_L" in d:
        parts.append(f"puts reaching L {d['puts_to_L']}/5")
    return ", ".join(parts)


def section7(report: Report) -> None:
    sec = "§7 a near principal's credentials on the face"
    runs = {("deny", "0.3"): None, ("deny", "0.2"): None, ("allow", "0.3"): None, ("allow", "no-face"): None,
            ("allow", "own"): None}
    for posture, generator in runs:
        runs[(posture, generator)] = _run7(posture, generator, anonymous=(posture, generator) == ("allow", "0.3"))
    s1, s2, s3, s4, s5 = (runs[k] for k in (("deny", "0.3"), ("deny", "0.2"), ("allow", "0.3"),
                                            ("allow", "no-face"), ("allow", "own")))
    selected = {s["id"]: s.get("link_protocols") for s in s1["block"]["subjects"]}
    report.check(sec, "step 1 (deny): every subject but ground's is selected by tcp, udp, tls, quic, serial, unixpipe, "
                      "vsock and ws", selected == {**{p: NEAR_PROTOCOLS for p in NEAR}, "ground": None},
                 json.dumps(selected))
    report.check(sec, "step 1 (deny): G receives what §1's G receives", _like_section1_g(s1["G"]), _brief(s1["G"]))
    report.check(sec, "step 1 (deny): X matches no subject: it receives nothing at all, sees no token and no "
                      "descriptor, and gets no reply", _nothing(s1["X"]), _brief(s1["X"]))
    report.check(sec, "step 1 (deny): neither's puts reach L; L receives every sample, put and occurrence",
                 s1["G"]["puts_to_L"] == 0 and s1["X"]["puts_to_L"] == 0
                 and all(90 <= s1["L"][k] <= 110 for k in STREAMS) and s1["L"]["oper"] == 10
                 and s1["L"]["fault"] == 5 and s1["L"]["change"] == 5,
                 f"G {s1['G']['puts_to_L']}, X {s1['X']['puts_to_L']}; L {_brief(s1['L'])}")
    report.check(sec, "step 2 (control, deny, 0.2's block): X matches local's subject over the face and gets its "
                      "grants there: about 100 of air/frames_total and of raw, all 5 occurrences of change, the "
                      "owner's two tokens, its GET and call answered; the face's downsampling still applies to it",
                 _local_share(s2["X"]), _brief(s2["X"]))
    report.note(sec, f"step 2 (deny, 0.2's block): G {_brief(s2['G'])}; X's puts reaching L "
                     f"{s2['X']['puts_to_L']}/5 (the text states none for step 2)")
    face = next((s for s in s3["block"]["subjects"] if s["id"] == "face:radio"), None)
    ground_pol = [p for p in s3["block"]["policies"] if "ground" in p["subjects"]]
    report.check(sec, "step 3 (allow): the block has a subject face:radio, selected by unixsock-stream alone, which "
                      "ground's policy lists beside ground",
                 face == {"id": "face:radio", "link_protocols": ["unixsock-stream"]} and len(ground_pol) == 1
                 and ground_pol[0]["subjects"] == ["ground", "face:radio"]
                 and all("face:radio" not in p["subjects"] for p in s3["block"]["policies"] if p not in ground_pol),
                 f"{face}; policies listing ground: {[(p['id'], p['subjects']) for p in ground_pol]}")
    report.check(sec, "step 3 (allow): G and X each receive what §1's G receives, see no token, and have their GET "
                      "and call answered",
                 all(_like_section1_g(s3[w]) and not s3[w]["tokens"] and s3[w]["liveliness"] == 0 and _answered(s3[w])
                     for w in ("G", "X")), f"G {_brief(s3['G'])}; X {_brief(s3['X'])}")
    report.check(sec, "step 3 (allow): neither's puts reach L: ground's deny of other services' writes holds for both",
                 s3["G"]["puts_to_L"] == 0 and s3["X"]["puts_to_L"] == 0,
                 f"G {s3['G']['puts_to_L']}, X {s3['X']['puts_to_L']}")
    y = s3.get("Y_status")
    report.note(sec, "step 3 (allow), a session over the face presenting no credential at all: "
                     + (f"N opens it: {_brief(s3['Y'])}" if y == "opened" else f"{y} at the link ({s3.get('Y_why')})"))
    report.check(sec, "step 4 (control, allow, narrowed, no face subject): X matches no subject and is allowed "
                      "everything: as in step 2, and its five puts reach L",
                 _local_share(s4["X"]) and s4["X"]["puts_to_L"] == 5, _brief(s4["X"]))
    report.note(sec, f"step 4 (allow, no face subject): G {_brief(s4['G'])}")
    report.check(sec, "step 5 (control, allow, the face's subject in a policy of its own holding the plan's denies "
                      "alone): G receives what §1's G receives, and its five puts reach L: ground's own deny of "
                      "other services' writes is undone",
                 _like_section1_g(s5["G"]) and s5["G"]["puts_to_L"] == 5, _brief(s5["G"]))
    report.note(sec, f"step 5 (allow, face subject in its own policy): X {_brief(s5['X'])}")
    report.check(sec, "step 6: a tool finds nothing in steps 1 and 3's blocks (§2.15)",
                 s1["found"] == [] and s3["found"] == [], f"{s1['found']}; {s3['found']}")
    report.check(sec, "step 6: in step 2's, near_subject_on_face for ground-router, local, radio-a and radio-b",
                 s2["found"] == [link.finding("near_subject_on_face", "radio", p)
                                 for p in ("ground-router", "local", "radio-a", "radio-b")], str(s2["found"]))
    report.check(sec, "step 6: in step 4's, face_subject_missing at unixsock-stream",
                 s4["found"] == [link.finding("face_subject_missing", "radio", "unixsock-stream")], str(s4["found"]))
    report.check(sec, "step 6: in step 5's, face_subject_misplaced at the face's subject (0.4)",
                 s5["found"] == [link.finding("face_subject_misplaced", "radio", "face:radio")],
                 f"{s5['found']}; comparison {s5['compare']}")
    report.note(sec, "step 6, the comparison with the plan (zk2py's shape, link.compare_block) for steps 1 to 5: "
                     + "; ".join(f"step {i}: {r['compare'] or 'none'}" for i, r in enumerate((s1, s2, s3, s4, s5), 1)))
    same = _block_under("deny", "no-face") == _block_under("deny", "0.3")
    report.note(sec, f"step 4's block under deny is step 1's: {same} (step 5's under deny is §10's step 6)")
    b2 = noted(report, sec, "step 2's block under allow", lambda: _run7("allow", "0.2"))
    if b2 is not None:
        report.note(sec, f"step 2's block under allow (0.2's, nothing narrowed, no face subject): G {_brief(b2['G'])}; "
                         f"X {_brief(b2['X'])}; tool {b2['found']}")


def _block_under(posture: str, generator: str) -> dict[str, Any]:
    """The block a §7 run would write, without a router."""
    from . import acl

    principals, _ = enrollment(_contract())
    plan_face = derive(policy())["faces"][0]
    return generate_face(policy(), plan_face, acl.generate(principals, posture).block, "ground", generator)


# -- §8 ------------------------------------------------------------------------------

def section8(report: Report) -> None:
    import zenoh

    sec = "§8 two minors that disagree on a stream's exposure"
    pol = policy(["radio-a", "radio-b"])
    m0 = _contract()
    m1 = minor1(tempfile.mkdtemp(prefix="zk2py-link-minor-"))
    alone0 = derive(pol, [m0])["faces"][0]
    for label, given in (("both revisions", [m0, m1]), ("minor 1 alone", [m1])):
        w = World(pol, revisions=given)
        try:
            # "lab/radio-a runs minor 1 and lab/radio-b minor 0".
            oa, ob = w.owner("radio-a", m1), w.owner("radio-b", m0)

            def pump(o: Any, name: str) -> None:
                t_end = time.monotonic() + 5.0
                i = 0
                while time.monotonic() < t_end:
                    for r in ("air/frames_total", "air/dropped_total"):
                        o.publish(f"{base_of(name)}/stream/{r}", str(i).encode())
                    i += 1
                    time.sleep(0.05)

            ts = [threading.Thread(target=pump, args=(o, n)) for o, n in ((oa, "radio-a"), (ob, "radio-b"))]
            for t in ts:
                t.start()
            for t in ts:
                t.join()
            time.sleep(1.0)
            got = {n: {r: w.g_count(f"{base_of(n)}/stream/air/{r}") for r in ("frames_total", "dropped_total")}
                   for n in ("radio-a", "radio-b")}
            l_got = {n: w.l_count(f"{base_of(n)}/stream/air/frames_total") for n in ("radio-a", "radio-b")}
            minors = {n: [c.minor for c in o.contracts] if hasattr(o, "contracts") else None
                      for n, o in (("radio-a", oa), ("radio-b", ob))}
            rules = [r["key_expr"] for r in (w.plan["downsampling"] or {}).get("rules", [])]
            frames = [f"{base_of(n)}/stream/air/frames_total" for n in ("radio-a", "radio-b")]
            denied = [k for k in frames if any(zenoh.KeyExpr(e).includes(zenoh.KeyExpr(k))
                                               for d in w.plan["deny"] for e in d["key_exprs"])]
            detail = (f"G {got}; L frames_total {l_got}; findings {w.plan['findings']}; frames_total denied {denied}; "
                      f"rules {rules}")
            report.note(sec, f"{label}: owners {minors}; {detail}")
            if label == "both revisions":
                report.check(sec, "step 1: the plan warns exposure_disagrees at radio.v1/stream/air/frames_total, and "
                                  "its denies and rules are minor 0's: air/frames_total denied, and no rule for it",
                             w.plan["findings"] == [link.finding("exposure_disagrees", "radio",
                                                                 "radio.v1/stream/air/frames_total", "warning")]
                             and w.plan["deny"] == alone0["deny"] and w.plan["downsampling"] == alone0["downsampling"]
                             and denied == frames and not [k for k in rules if k in frames], detail)
                report.check(sec, "step 1: G receives nothing of air/frames_total from either owner, and 4 to 6 "
                                  "samples of air/dropped_total from each; L receives about 100 of air/frames_total "
                                  "from each",
                             all(got[n]["frames_total"] == 0 and 4 <= got[n]["dropped_total"] <= 6 for n in got)
                             and all(90 <= v <= 110 for v in l_got.values()), detail)
            else:
                report.check(sec, "step 2 (control): from minor 1 alone the plan has a rule for each owner's "
                                  "air/frames_total, and no deny of it",
                             all(k in rules for k in frames) and not denied, detail)
                report.check(sec, "step 2 (control): G receives 4 to 6 of air/frames_total from each owner, "
                                  "lab/radio-b's among them, though the minor it runs keeps it home (F-119)",
                             all(4 <= got[n]["frames_total"] <= 6 for n in got), detail)
        finally:
            w.close()


# -- §9, §10: hand edits of the generator's block --------------------------------------------

def _edits(*fns: Any) -> Any:
    def edit(block: dict[str, Any]) -> dict[str, Any]:
        for fn in fns:
            block = fn(block)
        return block
    return edit


def _subject(sid: str, **props: Any) -> Any:
    """Replace subject ``sid``'s properties (keeping its id), or add it."""
    def edit(block: dict[str, Any]) -> dict[str, Any]:
        for s in block["subjects"]:
            if s["id"] == sid:
                s.clear()
                s.update({"id": sid, **props})
                return block
        block["subjects"].append({"id": sid, **props})
        return block
    return edit


def _narrow(sid: str, protocols: list[str]) -> Any:
    def edit(block: dict[str, Any]) -> dict[str, Any]:
        for s in block["subjects"]:
            if s["id"] == sid:
                s["link_protocols"] = list(protocols)
        return block
    return edit


def _listed(sid: str, *, by: list[str]) -> Any:
    """Subject ``sid`` listed by exactly the policies of the principals in
    ``by`` (``<principal>-policy``), and by no other."""
    def edit(block: dict[str, Any]) -> dict[str, Any]:
        for p in block["policies"]:
            p["subjects"] = [x for x in p["subjects"] if x != sid]
            if p["id"] in {f"{b}-policy" for b in by}:
                p["subjects"].append(sid)
        return block
    return edit


def _drop_subject(sid: str) -> Any:
    def edit(block: dict[str, Any]) -> dict[str, Any]:
        block["subjects"] = [s for s in block["subjects"] if s["id"] != sid]
        for p in block["policies"]:
            p["subjects"] = [x for x in p["subjects"] if x != sid]
        return block
    return edit


def _near_puts(posture: str, edit: Any, who: list[str]) -> dict[str, Any]:
    """§9 step 3: sessions over TCP with ``who``'s credentials each put five
    times on lab/radio-a's air/dropped_total, and L counts what reaches it."""
    from . import live

    w = World(policy(), posture=posture, block_edit=edit)
    try:
        sessions = {n: live.open_client(w.near, (n, f"{n}-pw")) for n in who}
        w.closers += list(sessions.values())
        time.sleep(0.3)
        return {"puts": _puts(w, sessions), "found": w.check_block()}
    finally:
        _close_all(w)


def _load(block: dict[str, Any]) -> str:
    """§9 step 6: an ``access_control`` block loaded as a router's
    configuration is: ``loaded``, or where it was refused and why."""
    import zenoh

    from .owner import free_loopback_port

    conf = zenoh.Config()
    try:
        conf.insert_json5("mode", json.dumps("router"))
        conf.insert_json5("listen/endpoints", json.dumps([f"tcp/127.0.0.1:{free_loopback_port()}"]))
        conf.insert_json5("scouting/multicast/enabled", "false")
        conf.insert_json5("access_control", json.dumps(block))
    except Exception as e:  # noqa: BLE001 - a refusal is the measurement
        return f"refused at insert: {type(e).__name__}: {str(e).splitlines()[0][:160]}"
    try:
        s = zenoh.open(conf)
    except Exception as e:  # noqa: BLE001
        return f"refused at open: {type(e).__name__}: {str(e).splitlines()[0][:160]}"
    s.close()
    return "loaded"


def section9(report: Report) -> None:
    sec = "§9 who a subject binds"
    shared = _subject("local", usernames=["ground", "local"])
    s1 = _run7("deny", block_edit=shared)
    s2 = _run7("deny", block_edit=_edits(shared, _narrow("ground", ["unixsock-stream"])))
    report.check(sec, "step 1 (deny, local's subject bound to ground and local, in local's policy): X matches it and "
                      "gets local's grants across the face, its GET and call answered, the face's downsampling still "
                      "applying", _local_share(s1["X"]), _brief(s1["X"]))
    report.check(sec, "step 1: G receives what §1's G receives (equal combinations are one subject, whose deny is "
                      "evaluated first); neither's puts reach L",
                 _like_section1_g(s1["G"]) and s1["G"]["puts_to_L"] == 0 and s1["X"]["puts_to_L"] == 0,
                 f"G {_brief(s1['G'])}; X's puts {s1['X']['puts_to_L']}")
    report.check(sec, "step 2 (control, ground's subject narrowed to unixsock-stream): the two ground combinations "
                      "differ, G matches both, and any allow wins: G gets local's share too, as X does; neither's "
                      "puts reach L",
                 _local_share(s2["G"]) and _local_share(s2["X"]) and s2["G"]["puts_to_L"] == 0
                 and s2["X"]["puts_to_L"] == 0, f"G {_brief(s2['G'])}; X {_brief(s2['X'])}")
    s3 = _near_puts("allow", _narrow("ground", ["unixsock-stream"]), ["ground"])
    s4 = _near_puts("allow", _narrow("radio-b", [p for p in NEAR_PROTOCOLS if p != "tcp"]), ["radio-b"])
    s5 = _near_puts("allow", None, ["ground", "radio-b"])
    report.check(sec, "step 3 (allow, ground narrowed to unixsock-stream): ground's session over TCP matches no "
                      "subject, and all five puts reach L", s3["puts"] == {"ground": 5}, str(s3["puts"]))
    report.check(sec, "step 4 (allow, radio-b narrowed off tcp too): all five of radio-b's puts over TCP reach L",
                 s4["puts"] == {"radio-b": 5}, str(s4["puts"]))
    report.check(sec, "step 5 (control, the generator's allow block): none of either session's puts reaches L",
                 s5["puts"] == {"ground": 0, "radio-b": 0}, str(s5["puts"]))
    base = {"enabled": True, "default_permission": "deny", "rules": [
        {"id": "r", "messages": ["put"], "flows": ["ingress"], "permission": "allow", "key_exprs": ["zk2/**"]}],
        "policies": [{"id": "p", "rules": ["r"], "subjects": ["x"]}]}
    loads = {name: _load({**base, "subjects": [subject]}) for name, subject in (
        ("link_protocols: []", {"id": "x", "usernames": ["local"], "link_protocols": []}),
        ("usernames: []", {"id": "x", "usernames": [], "link_protocols": ["tcp"]}),
        ("neither list", {"id": "x", "usernames": ["local"], "link_protocols": ["tcp"]}))}
    report.check(sec, "step 6: each empty list is refused at load; the subject without it loads",
                 loads["neither list"] == "loaded" and all(v.startswith("refused") for k, v in loads.items()
                                                          if k != "neither list"), json.dumps(loads))
    no_ground = _block_under("allow", "0.3")
    no_ground = _drop_subject("ground")(no_ground)
    gone = check(no_ground, policy())
    f = link.finding
    report.check(sec, "step 7: steps 1 and 2, near_subject_on_face at local and zk_not_denied",
                 s1["found"] == s2["found"] == link.sort_findings([f("near_subject_on_face", "radio", "local"),
                                                                  f("zk_not_denied", "radio", "@zk")]),
                 f"{s1['found']}; {s2['found']}")
    report.check(sec, "step 7: step 3, narrowed_too_far at ground; step 4 at radio-b; each with no face",
                 s3["found"] == [f("narrowed_too_far", None, "ground")]
                 and s4["found"] == [f("narrowed_too_far", None, "radio-b")], f"{s3['found']}; {s4['found']}")
    report.check(sec, "step 7: the generator's allow block with ground's subject removed: narrowed_too_far at ground, "
                      "and face_subject_misplaced at the face's subject",
                 gone == link.sort_findings([f("narrowed_too_far", None, "ground"),
                                             f("face_subject_misplaced", "radio", "face:radio")]), str(gone))
    report.note(sec, f"step 5's block (the generator's allow block): tool {s5['found'] or 'no finding'}")


def _read_radio_b(posture: str, edit: Any) -> dict[str, Any]:
    """§10 step 1: lab/radio-b publishes air/dropped_total 20 times; a
    session over TCP with radio-a's credentials and L each subscribe to that
    key alone."""
    from . import live

    w = World(policy(), posture=posture, block_edit=edit)
    try:
        ob = w.owner("radio-b")
        ra = live.open_client(w.near, ("radio-a", "radio-a-pw"))
        w.closers.append(ra)
        key = f"{base_of('radio-b')}/stream/air/dropped_total"
        subs = {"radio-a": fr.Subscriber(ra, key), "L": fr.Subscriber(w.l, key)}
        time.sleep(0.5)
        for i in range(20):
            ob.publish(key, str(i).encode())
            time.sleep(0.05)
        time.sleep(1.0)
        got = {k: len(s.of(key)) for k, s in subs.items()}
        for s in subs.values():
            s.close()
        return {"got": got, "found": w.check_block()}
    finally:
        _close_all(w)


def section10(report: Report) -> None:
    sec = "§10 the face's own subject"
    f = link.finding
    wide = {"allow": _subject("face:radio", link_protocols=["unixsock-stream", "tcp"]),
            "deny": _edits(_subject("face:radio", link_protocols=["unixsock-stream", "tcp"]),
                           _listed("face:radio", by=["ground"]))}
    for posture in ("allow", "deny"):
        r1, r2 = _read_radio_b(posture, wide[posture]), _read_radio_b(posture, None)
        report.check(sec, f"step 1 ({posture}): with a face subject selected by tcp beside unixsock-stream in "
                          f"ground's policy, the radio-a session over TCP receives all 20 samples; L all 20",
                     r1["got"] == {"radio-a": 20, "L": 20}, str(r1["got"]))
        report.check(sec, f"step 2 (control, the generator's {posture} block): the radio-a session receives none",
                     r2["got"]["radio-a"] == 0 and r2["got"]["L"] == 20, str(r2["got"]))
        report.check(sec, f"step 7: step 1's block under {posture}, face_subject_beyond_face at the subject",
                     r1["found"] == [f("face_subject_beyond_face", "radio", "face:radio")], str(r1["found"]))
        report.note(sec, f"step 2's block under {posture}: tool {r2['found'] or 'no finding'}")
    s3 = _run7("deny", block_edit=_edits(_subject("face:radio", link_protocols=["unixsock-stream"]),
                                         _listed("face:radio", by=["local"])))
    s4 = _run7("allow", block_edit=_listed("face:radio", by=["local"]))
    s5 = _run7("allow", block_edit=_listed("face:radio", by=[]))
    s6 = _run7("deny", "own")
    for n, r, label in ((3, s3, "deny, a face subject in local's policy"),
                        (4, s4, "allow, the face's subject moved to local's policy")):
        report.check(sec, f"step {n} ({label}): G and X each get local's share there; neither's puts reach L",
                     _local_share(r["G"]) and _local_share(r["X"]) and r["G"]["puts_to_L"] == 0
                     and r["X"]["puts_to_L"] == 0, f"G {_brief(r['G'])}; X {_brief(r['X'])}")
    report.check(sec, "step 5 (allow, the face's subject in no policy): G and X each get local's share there, and all "
                      "five of each one's puts reach L",
                 _local_share(s5["G"]) and _local_share(s5["X"]) and s5["G"]["puts_to_L"] == 5
                 and s5["X"]["puts_to_L"] == 5, f"G {_brief(s5['G'])}; X {_brief(s5['X'])}")
    report.check(sec, "step 6 (deny, the face's subject in a policy of its own holding the plan's denies): G receives "
                      "what §1's G receives; X receives nothing, sees no token and gets no reply; neither's puts "
                      "reach L",
                 _like_section1_g(s6["G"]) and _nothing(s6["X"]) and s6["G"]["puts_to_L"] == 0
                 and s6["X"]["puts_to_L"] == 0, f"G {_brief(s6['G'])}; X {_brief(s6['X'])}")
    misplaced = link.sort_findings([f("face_subject_misplaced", "radio", "face:radio"), f("zk_not_denied", "radio", "@zk")])
    report.check(sec, "step 7: steps 3 to 5, face_subject_misplaced at the face's subject, and zk_not_denied",
                 s3["found"] == s4["found"] == s5["found"] == misplaced,
                 f"{s3['found']}; {s4['found']}; {s5['found']}")
    report.check(sec, "step 7: step 6, face_subject_misplaced alone",
                 s6["found"] == [f("face_subject_misplaced", "radio", "face:radio")], str(s6["found"]))


# -- §11: a subject naming both kinds ----------------------------------------------------

def _close_all(w: World) -> None:
    """Close a run's extra sessions, then the world, logging what failed to
    close once (:func:`close_each`)."""
    close_each(w.closers, "a run's sessions")
    w.close()


def _tls_puts(edit: Any) -> dict[str, Any]:
    """§11 step 4: under allow, three sessions with local's credentials:
    over TLS presenting the CN local, over TLS presenting the CN other, and
    over TCP with no certificate; each puts five times on lab/radio-a's
    air/dropped_total, and L counts what reaches it."""
    from . import live

    w = World(policy(), posture="allow", block_edit=edit, tls=True)
    try:
        sessions = {"tls-cn-local": open_tls(w, "local", "local"), "tls-cn-other": open_tls(w, "local", "other"),
                    "tcp-no-cert": live.open_client(w.near, ("local", "local-pw"))}
        w.closers += list(sessions.values())
        time.sleep(0.3)
        return {"puts": _puts(w, sessions), "found": w.check_block()}
    finally:
        _close_all(w)


def section11(report: Report) -> None:
    sec = "§11 a subject naming both kinds"
    f = link.finding
    off_tcp = _narrow("local", [p for p in NEAR_PROTOCOLS if p != "tcp"])
    step1 = _edits(off_tcp, _subject("ground-local", usernames=["ground"], cert_common_names=["local"]),
                   _listed("ground-local", by=["ground"]))
    step2 = _edits(off_tcp, _subject("local-cn", cert_common_names=["local"], link_protocols=list(NEAR_PROTOCOLS)),
                   _listed("local-cn", by=["local"]))
    s1 = _near_puts("allow", step1, ["local"])
    s2 = _near_puts("allow", step2, ["local"])
    s3 = _near_puts("allow", off_tcp, ["local"])
    s3c = _near_puts("allow", None, ["local"])
    report.check(sec, "step 1 (F-121 (i)): local's session over TCP matches neither local's subject nor the one naming "
                      "ground and local's CN: all five puts reach L", s1["puts"] == {"local": 5}, str(s1["puts"]))
    report.check(sec, "step 2 (F-121 (ii)): the added subject names local as a CN, the session presents it as a user "
                      "name: all five reach L", s2["puts"] == {"local": 5}, str(s2["puts"]))
    report.check(sec, "step 3 (control): local narrowed off tcp and nothing else, all five reach L; under the "
                      "generator's block, none does", s3["puts"] == {"local": 5} and s3c["puts"] == {"local": 0},
                 f"{s3['puts']}; generator's {s3c['puts']}")
    both = _subject("local", usernames=["local"], cert_common_names=["local"], link_protocols=list(NEAR_PROTOCOLS))
    s4, s4c = _tls_puts(both), _tls_puts(None)
    report.check(sec, "step 4 (local's subject naming local as its user name and as its CN): over TLS with the CN "
                      "local, none of its puts reaches L; with the CN other, or over TCP with no certificate, all five "
                      "do", s4["puts"] == {"tls-cn-local": 0, "tls-cn-other": 5, "tcp-no-cert": 5}, str(s4["puts"]))
    report.check(sec, "step 4 (control, the generator's allow block): none of the three sessions' puts reaches L",
                 s4c["puts"] == {"tls-cn-local": 0, "tls-cn-other": 0, "tcp-no-cert": 0}, str(s4c["puts"]))

    def mixed(principal: str, posture: str) -> Any:
        sid = f"ground-{principal}"
        edits = [_subject(sid, usernames=["ground"], cert_common_names=[principal]), _listed(sid, by=[principal])]
        if posture == "deny":
            edits.append(_subject("face:radio", link_protocols=["unixsock-stream"]))
        edits.append(_listed("face:radio", by=["ground", principal]))
        return _edits(*edits)

    def with_zk(block: dict[str, Any]) -> dict[str, Any]:
        for pol in block["policies"]:
            if pol["id"] == "radio-a-policy":
                pol["rules"].append("face-radio-zk")
        return block

    s5 = _run7("deny", block_edit=mixed("radio-a", "deny"))
    s6 = _run7("deny", block_edit=mixed("local", "deny"))
    s7 = _run7("allow", block_edit=mixed("radio-a", "allow"), alive=True)
    s8 = _run7("deny", block_edit=_edits(mixed("radio-a", "deny"), with_zk))
    report.check(sec, "step 5 (deny, ground and radio-a's CN in radio-a's policy, the face's subject in ground's and "
                      "radio-a's): G and X each receive what §1's G receives, and all five of each one's puts reach L",
                 all(_like_section1_g(s5[w]) and s5[w]["puts_to_L"] == 5 for w in ("G", "X")),
                 f"G {_brief(s5['G'])}; X {_brief(s5['X'])}")
    report.check(sec, "step 6 (the same with local): G and X each receive what §1's G receives, and neither's puts "
                      "reach L", all(_like_section1_g(s6[w]) and s6[w]["puts_to_L"] == 0 for w in ("G", "X")),
                 f"G {_brief(s6['G'])}; X {_brief(s6['X'])}")
    # Core §11.2 (0.31): zk2py's generator takes the stricter complement. It
    # denies radio-a, per message and flow, its own keys where its Own
    # grant's rows do not name them (put toward it among them), so link.v1
    # 0.6's second branch of expected 7 is the one it must show, exactly.
    report.check(sec, "step 7 (step 5 under allow; zk2py's complement denies a principal its own keys per message and "
                      "flow, core §11.2's stricter branch): G receives what §1's G receives through ground's own "
                      "subject; X receives nothing at all, no sample, token or descriptor, and neither its GET nor "
                      "its call is answered; neither's puts reach L",
                 _like_section1_g(s7["G"]) and _answered(s7["G"]) and _nothing(s7["X"])
                 and s7["G"]["puts_to_L"] == 0 and s7["X"]["puts_to_L"] == 0,
                 f"G {_brief(s7['G'])}; X {_brief(s7['X'])}")
    report.check(sec, "step 7: X's session is live (its put on a key no contract declares reaches L, as G's does), so "
                      "its nothing is the block's", s7["X"]["alive"]["arrived"] == 1 and s7["G"]["alive"]["arrived"] == 1,
                 f"G {s7['G']['alive']}, X {s7['X']['alive']}")
    report.check(sec, "step 8 (step 5 with radio-a's policy listing the face's zk rule): all five of each one's puts "
                      "reach L", s8["G"]["puts_to_L"] == 5 and s8["X"]["puts_to_L"] == 5,
                 f"G {_brief(s8['G'])}; X {_brief(s8['X'])}")
    srt = link.sort_findings
    report.check(sec, "step 9: step 1, narrowed_too_far at local, and near_subject_on_face at the added subject",
                 s1["found"] == srt([f("narrowed_too_far", None, "local"),
                                     f("near_subject_on_face", "radio", "ground-local")]), str(s1["found"]))
    report.check(sec, "step 9: steps 2 to 4, narrowed_too_far at local",
                 all(r["found"] == [f("narrowed_too_far", None, "local")] for r in (s2, s3, s4)),
                 f"{s2['found']}; {s3['found']}; {s4['found']}")
    for n, r, sid in ((5, s5, "ground-radio-a"), (6, s6, "ground-local")):
        report.check(sec, f"step 9: step {n}, near_subject_on_face at the added subject, face_subject_misplaced at the "
                          f"face's subject, and zk_not_denied",
                     r["found"] == srt([f("near_subject_on_face", "radio", sid),
                                        f("face_subject_misplaced", "radio", "face:radio"),
                                        f("zk_not_denied", "radio", "@zk")]), str(r["found"]))
    report.check(sec, "step 9: step 7, the same, and narrowed_too_far at radio-a",
                 s7["found"] == srt([f("near_subject_on_face", "radio", "ground-radio-a"),
                                     f("face_subject_misplaced", "radio", "face:radio"),
                                     f("zk_not_denied", "radio", "@zk"), f("narrowed_too_far", None, "radio-a")]),
                 str(s7["found"]))
    report.check(sec, "step 9: step 8, near_subject_on_face and face_subject_misplaced, with no zk_not_denied",
                 s8["found"] == srt([f("near_subject_on_face", "radio", "ground-radio-a"),
                                     f("face_subject_misplaced", "radio", "face:radio")]), str(s8["found"]))
    report.note(sec, f"step 3's and step 4's control blocks (the generator's allow block): tool "
                     f"{s3c['found'] or 'no finding'}; {s4c['found'] or 'no finding'}")


#: §13's second face, as scenarios.md writes it, its far principal given.
SAT_FACE = """
[face.sat]
far          = "{far}"
attach       = "client"
select       = {{ link_protocols = ["tls"] }}
class        = "constrained"
expose       = "none"
max_outage_s = 6
batch_bytes  = 1000
link_bps     = 1000000
"""


# -- §12: what a block omits --------------------------------------------------------------

def _omit(key: str) -> Any:
    def edit(block: dict[str, Any]) -> dict[str, Any]:
        block.pop(key, None)
        return block
    return edit


def _set(key: str, value: Any) -> Any:
    def edit(block: dict[str, Any]) -> dict[str, Any]:
        block[key] = value
        return block
    return edit


def _consume_out(rule: dict[str, Any]) -> bool:
    """The console's Consume rule for the samples it receives: an egress
    allow of ``put`` over radio.v1's stream selectors (§12 step 4's
    ``consume-out:tool.console``; zk2py writes one per principal)."""
    return (rule.get("permission") == "allow" and link.rule_flows(rule) == ["egress"]
            and "put" in rule.get("messages", [])
            and any(k.startswith(f"zk2/lab/*/{IFACE}/stream/") for k in rule.get("key_exprs", [])))


def _flows_out(block: dict[str, Any]) -> dict[str, Any]:
    """§12 step 4: "flows taken out of the console's Consume rule for the
    samples it receives …, which ground's and local's policies list, and
    out of every deny of the face"."""
    listed = {r for pol in block["policies"] if {"ground", "local"} & set(pol["subjects"]) for r in pol["rules"]}
    for r in block["rules"]:
        if (r["id"] in listed and _consume_out(r)) or r["id"].startswith("face-radio-"):
            r.pop("flows", None)
    return block


def _g_puts(w: World) -> dict[str, int]:
    """§12 step 4: "the owner silent, G puts five times on …/air/dropped_total
    and five times on …/stream/air/frames_total, and L counts what reaches
    it of each"."""
    out = {}
    for name in ("dropped_total", "frames_total"):
        key = f"{base_of()}/stream/air/{name}"
        for i in range(5):
            w.g.put(key, f"put G {name} {i}".encode())
            time.sleep(0.05)
        time.sleep(1.0)
        out[name] = sum(1 for x in w.ls.of(key) if x.payload.startswith(f"put G {name} ".encode()))
    return out


def _run12_flows(edit: Any) -> dict[str, Any]:
    w = World(policy(), block_edit=edit)
    try:
        o = w.owner()
        traffic(o)
        g = _receives(w, w.gs, w.gt)
        g.update(_asks(w.g, o))
        return {"G": g, "puts": _g_puts(w), "found": w.check_block(), "block": w.block}
    finally:
        _close_all(w)


def _everything(d: dict[str, Any]) -> bool:
    """§12 expected 1: "about 100 of air/frames_total and of raw, all 10
    puts of oper, all 5 occurrences of fault and of change, see the owner's
    two tokens, receive its descriptor, and have their GET and call
    answered. The face's downsampling … still applies: 4 to 6 of
    air/dropped_total and of frames"."""
    return (90 <= d["stream/air/frames_total"] <= 110 and 90 <= d["@stream/raw"] <= 110 and d["oper"] == 10
            and d["fault"] == 5 and d["change"] == 5 and {"instance", "alive"} <= set(d["tokens"])
            and d["descriptor"] >= 1 and _answered(d)
            and 4 <= d["stream/air/dropped_total"] <= 6 and 4 <= d["@stream/frames"] <= 6)


def section12(report: Report) -> None:
    sec = "§12 what a block omits"
    f = link.finding
    s1 = _run7("deny", block_edit=_set("enabled", False))
    s2 = _run7("deny", block_edit=_omit("enabled"))
    s3 = _run7("deny", block_edit=_omit("default_permission"))
    for n, r, label in ((1, s1, "enabled: false"), (2, s2, "the enabled line taken out")):
        report.check(sec, f"step {n} ({label}): nothing is enforced: G and X each receive about 100 of "
                          f"air/frames_total and of raw, all of oper, fault and change, both tokens and the "
                          f"descriptor, their GET and call answered, 4 to 6 of air/dropped_total and frames (the "
                          f"downsampling still applies), and all five of each one's puts reach L",
                     _everything(r["G"]) and _everything(r["X"]) and r["G"]["puts_to_L"] == 5
                     and r["X"]["puts_to_L"] == 5, f"G {_brief(r['G'])}; X {_brief(r['X'])}")
    report.check(sec, "step 3 (the default_permission line taken out): G receives what §1's G receives; X receives "
                      "nothing at all, sees no token and gets no reply; neither's puts reach L",
                 _like_section1_g(s3["G"]) and _nothing(s3["X"]) and s3["G"]["puts_to_L"] == 0
                 and s3["X"]["puts_to_L"] == 0, f"G {_brief(s3['G'])}; X {_brief(s3['X'])}")
    s4, s4c = _run12_flows(_flows_out), _run12_flows(None)
    edited = [r["id"] for r in s4["block"]["rules"] if "flows" not in r]
    report.check(sec, "step 4 (flows taken out of the Consume rule for received samples and of every face deny): G "
                      "receives what §1's G receives and nothing the face denies; all five of its puts on "
                      "air/dropped_total reach L and none on air/frames_total; under the generator's block none of "
                      "either",
                 _like_section1_g(s4["G"]) and s4["puts"] == {"dropped_total": 5, "frames_total": 0}
                 and s4c["puts"] == {"dropped_total": 0, "frames_total": 0}
                 and any(not rid.startswith("face-") for rid in edited) and any(rid.startswith("face-") for rid in edited),
                 f"G {_brief(s4['G'])}; puts {s4['puts']}; generator's {s4c['puts']}; rules without flows {edited}")
    report.check(sec, "step 5: steps 1 and 2, block_disabled at enabled, alone; steps 3 and 4, no finding (a tool "
                      "reads an absent default_permission as deny, and a rule with no flows as both flows)",
                 s1["found"] == s2["found"] == [f("block_disabled", "radio", "enabled")]
                 and s3["found"] == [] and s4["found"] == [],
                 f"{s1['found']}; {s2['found']}; {s3['found']}; {s4['found']}")
    report.note(sec, "step 5, zk2py's comparison with the plan (its shape, link.compare_block): "
                     + "; ".join(f"step {i}: {link.compare_block(link.parse_policy(policy())[0], [derive(policy())['faces'][0]], b) or 'none'}"
                                 for i, b in ((1, s1["block"]), (2, s2["block"]), (3, s3["block"]), (4, s4["block"]))))


# -- §13: two faces, one far principal ------------------------------------------------------

def section13(report: Report) -> None:
    sec = "§13 two faces, one far principal"
    f = link.finding
    out = {}
    for far in ("ground", "ground-router"):
        text = policy() + SAT_FACE.format(far=far)
        warnings = link.parse_policy(text)[1]
        w = World(text, faces="all")
        try:
            o = w.owner()
            traffic(o)
            g = _receives(w, w.gs, w.gt)
            g.update(_asks(w.g, o))
            policies = {p["id"]: [r for r in p["rules"] if r.startswith("face-")] for p in w.block["policies"]}
            out[far] = {"G": g, "warnings": warnings, "found": w.check_block(), "policies": policies}
        finally:
            _close_all(w)
    one, two = out["ground"], out["ground-router"]
    report.check(sec, "step 1: the generator warns far_of_two_faces at ground, and refuses nothing",
                 one["warnings"] == [f("far_of_two_faces", None, "ground", "warning")], str(one["warnings"]))
    gp = one["policies"].get("ground-policy", [])
    report.check(sec, "step 1: ground's policy carries both plans' denies, the radio face's and sat's",
                 any(r.startswith("face-radio-") for r in gp) and any(r.startswith("face-sat-") for r in gp), str(gp))
    report.check(sec, "step 1: over the radio face G receives no sample of air/dropped_total or frames, no put of oper "
                      "and no occurrence of fault; its GET and call are answered; it sees no token and no descriptor",
                 all(one["G"][k] == 0 for k in STREAMS + ("oper", "fault", "change")) and _answered(one["G"])
                 and not one["G"]["tokens"] and one["G"]["descriptor"] == 0, _brief(one["G"]))
    report.check(sec, "step 2 (control, sat's far principal ground-router): no warning; ground-router's policy "
                      "carries sat's denies and ground's the radio face's alone; G receives what §1's G receives",
                 two["warnings"] == [] and any(r.startswith("face-sat-") for r in two["policies"].get("ground-router-policy", []))
                 and all(r.startswith("face-radio-") for r in two["policies"].get("ground-policy", []))
                 and _like_section1_g(two["G"]), f"{two['warnings']}; {two['policies']}; G {_brief(two['G'])}")
    report.check(sec, "step 3: a tool finds nothing in either block", one["found"] == [] and two["found"] == [],
                 f"{one['found']}; {two['found']}")


SECTIONS = {"1": section1, "2": section2, "3": section3, "4": section4, "5": section5, "6": section6,
            "7": section7, "8": section8, "9": section9, "10": section10, "11": section11, "12": section12,
            "13": section13}


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="python -m zk2py.link_scenarios", description=__doc__.split("\n")[0])
    ap.add_argument("--only", action="append", choices=sorted(SECTIONS), help="run only these sections")
    args = ap.parse_args(argv)
    report = Report()
    for name, fn in SECTIONS.items():
        if args.only and name not in args.only:
            continue
        try:
            fn(report)
        except Exception as e:  # noqa: BLE001 - a section that cannot run is a failure
            import traceback

            traceback.print_exc()
            report.check(f"§{name}", "the section ran", False, f"{type(e).__name__}: {e}")
    passed = sum(1 for r in report.rows if r[2])
    failed = len(report.rows) - passed
    print(f"link scenarios: {passed} passed, {failed} failed")
    for sec, name, ok, detail in report.rows:
        if not ok:
            print(f"FAIL [{sec}] {name}: {detail}")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())

"""The ``link.v1`` scenarios (``spec/profiles/link/scenarios.md``, text
0.2) on zenoh-python routers: ``just py-link`` (``python -m
zk2py.link_scenarios [--only 1 …]``).

- **The deployment** is the reference's example, ``examples/zk2/link/``,
  read as given: the ``radio.v1`` contract, the lab router's face policy
  (``radio.face.toml``, each section's changes applied to it) and the
  enrollment (``radio.enrollment.toml``: the two radios, and the console
  tool bound to every stream, state and event of ``radio.v1`` on ``lab/*``
  and calling ``reset`` there, run by ``local``, ``ground`` and
  ``ground-router``), read into ``zk2py.acl``'s principals.
- **The generator** is zk2py's: :func:`zk2py.link.plan`, compiled into N's
  ``access_control`` block under ``deny`` by ``zk2py.acl``, the plan's
  denies in the far principal's policy (§2.4,
  :func:`zk2py.link.compile_face`), the plan's ``downsampling`` block beside
  it, and N's ``gateway`` for a south region
  (:func:`zk2py.link.router_extras`).
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


def policy(services: list[str] | None = None, *, far: str | None = None, attach: str | None = None,
           region: str | None = None, expose: str | None = None, interval_s: Any = "keep") -> str:
    """``radio.face.toml``, with a section's changes: each named key's line
    replaced, ``region`` added after ``attach``, ``interval_s`` removed with
    ``None``."""
    out = []
    changes = {"far": far, "attach": attach, "expose": expose}
    for line in FACE.read_text().splitlines():
        key = line.split("=", 1)[0].strip() if "=" in line and not line.lstrip().startswith("#") else None
        if key == "services" and services is not None:
            line = f"services = {json.dumps(services)}"
        elif key in changes and changes[key] is not None:
            line = f'{key:<12} = "{changes[key]}"'
        elif key == "interval_s" and interval_s != "keep":
            if interval_s is None:
                continue
            line = f"interval_s   = {interval_s}"
        out.append(line)
        if key == "attach" and region is not None:
            out.append(f'region       = "{region}"')
    return "\n".join(out) + "\n"


class Report:
    def __init__(self) -> None:
        self.rows: list[tuple[str, str, bool, str]] = []
        self.lock = threading.Lock()

    def check(self, section: str, name: str, ok: bool, detail: str = "") -> bool:
        with self.lock:
            self.rows.append((section, name, ok, detail))
            print(f"{'PASS' if ok else 'FAIL'} [{section}] {name}" + (f": {detail}" if detail else ""), flush=True)
        return ok


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
    its (consume, call) uses."""
    from . import acl

    doc = tomllib.loads(ENROLLMENT.read_text())
    contracts = {contract.interface: contract}
    implements = {s["address"]: s["implements"] for s in doc.get("service", [])}
    tools: dict[str, tuple[list[Any], list[Any]]] = {}
    for tool in doc.get("tool", []):
        consumes = []
        for b in (tool.get("bindings") or {}).values():
            c = contracts[b["interface"]]
            resources = b.get("resources") or [f"{r['token']}/{r['template']}" for r in c.canonical["resources"]
                                               if r["token"] in BOUND_TOKENS]
            consumes.append(acl.Use(b["interface"], list(b["providers"]), resources))
        calls = []
        for cl in tool.get("calls", []):
            c = contracts[cl["interface"]]
            names = {r["template"] for r in c.canonical["resources"] if r["token"] == "@op"}
            calls.append(acl.Use(cl["interface"], list(cl["providers"]),
                                 [f"@op/{op}" for op in cl["operations"] if op in names]))
        tools[tool["name"]] = (consumes, calls)
    principals = []
    for pr in doc.get("principal", []):
        services = pr.get("services") or []
        consumes, calls = [], []
        for name in pr.get("tools") or []:
            consumes += tools[name][0]
            calls += tools[name][1]
        principals.append(acl.Principal(
            pr["user"], pr["user"], service=services[0] if services else None,
            implements=[contracts[i] for i in implements.get(services[0], [])] if services else [],
            consumes=consumes, calls=calls))
    return principals, tools


def derive(policy_text: str) -> dict[str, Any]:
    """The plan, from the policy, the deployment and the contract (§2.3)."""
    c = _contract()
    return link.plan(policy_text, {"radio-a": [IFACE], "radio-b": [IFACE]}, {IFACE: link.resources_of(c)})


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
    ``face_local``: also a session with ``local``'s credentials over the
    face (§2.4's question)."""

    def __init__(self, policy_text: str, *, far: str = "ground", attach: str = "client",
                 queryable_grant: str | None = "owner", deny_override: tuple[str, list[str]] | None = None,
                 downsampling_override: list[dict[str, Any]] | None = None, tombstone_s: float = 60.0,
                 face_local: bool = False):
        import zenoh

        from . import acl, live
        from .owner import free_loopback_port

        self.contract = _contract()
        p = derive(policy_text)
        if "refused" in p:
            raise ValueError(f"the generator refuses the policy: {p['refused']}")
        self.plan = p["faces"][0]
        self.tombstone_s = tombstone_s
        principals, tools = enrollment(self.contract)
        consumes, calls = tools["console"]
        self.selectors = [s for u in consumes for s in acl.use_selectors(u)]
        block = acl.generate(principals, "deny").block
        block = link.compile_face(self.plan, block, far)
        if deny_override is not None:
            rid, keys = deny_override
            for r in block["rules"]:
                if r["id"] == f"face-{self.plan['name']}-{rid}":
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
                if far in pol["subjects"]:
                    pol["rules"].append("far-queryables")
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
        conf.insert_json5("listen/endpoints", json.dumps([self.near, f"unixsock-stream/{self.sock}"]))
        conf.insert_json5("scouting/multicast/enabled", "false")
        conf.insert_json5("timestamping/enabled", "true")
        conf.insert_json5("transport/auth/usrpwd", json.dumps({"dictionary_file": users}))
        conf.insert_json5("access_control", json.dumps(block))
        for k, v in extras.items():
            conf.insert_json5(k, json.dumps(v))
        self.n = zenoh.open(conf)
        self.closers: list[Any] = []
        self.far_router = None
        self.far, self.attach = far, attach
        self.g = self._open_g()
        self.l = live.open_client(self.near, ("local", "local-pw"))
        self.gs = fr.Subscriber(self.g, self.selectors)
        self.ls = fr.Subscriber(self.l, self.selectors)
        presence = ["zk2/lab/*/@zk/**"]
        self.gt, self.lt = Tokens(self.g, presence), Tokens(self.l, presence)
        self.fl: Any = None
        self.fls: Any = None
        self.flt: Any = None
        if face_local:
            self.fl = live.open_client(f"unixsock-stream/{self.sock}", ("local", "local-pw"))
            self.fls = fr.Subscriber(self.fl, self.selectors)
            self.flt = Tokens(self.fl, presence)
        self.owners: dict[str, Any] = {}
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

    def owner(self, name: str = "radio-a") -> Any:
        from .owner import Owner

        o = Owner("lab", name, [self.contract], connect=self.near, auth=(name, f"{name}-pw"),
                  tombstone_s=self.tombstone_s)
        o.start()
        self.owners[name] = o
        time.sleep(0.5)
        return o

    def block_view(self) -> dict[str, Any]:
        """The block a tool reads for the face (§2.15): the far principal's
        denies, and the downsampling rules."""
        mine = {rid for pol in self.block["policies"] if self.far in pol["subjects"] for rid in pol["rules"]}
        deny = [{"flows": r["flows"], "messages": r["messages"], "key_exprs": r["key_exprs"]}
                for r in self.block["rules"] if r["id"] in mine and r["permission"] == "deny"]
        rules = [x for ds in self.downsampling for x in ds["rules"]]
        return {"face": self.plan["name"], "namespace": "", "deny": deny, "downsampling": rules}

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
        for o in self.owners.values():
            try:
                o.close()
            except Exception:  # noqa: BLE001 - closing anyway
                pass
        for s in (self.gs, self.ls) + ((self.fls,) if self.fls is not None else ()):
            s.close()
        for x in (self.g, self.l, self.n) + ((self.far_router[0],) if self.far_router else ()) \
                + ((self.fl,) if self.fl is not None else ()):
            try:
                x.close()
            except Exception:  # noqa: BLE001
                pass


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
    w = World(policy(), face_local=True)
    try:
        ground_rules = {r for pol in w.block["policies"] if "ground" in pol["subjects"] for r in pol["rules"]}
        denies = [d["id"] for d in w.plan["deny"]]
        report.check(sec, "step 1: the block holds the plan: the far principal's policy carries zk, adv, admin and "
                          "the exposure denies, and the downsampling block one rule per crossing stream of "
                          "lab/radio-a at 1 Hz, as plan.json's third case derives them (§2.4, §2.8)",
                     all(f"face-radio-{d}" in ground_rules for d in denies) and denies[:3] == ["zk", "adv", "admin"]
                     and _same_denies(w.plan) and all(r["freq"] == 1.0 for r in w.plan["downsampling"]["rules"]),
                     f"denies {denies}; rules {[r['key_expr'] for r in w.plan['downsampling']['rules']]}")
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
        found = link.check_block(w.block_view())
        report.check(sec, "step 5: a tool checking the block reports no finding (§2.15)", not found, str(found))
        # §2.4's question (SPEC-FINDINGS F-118): a near principal's
        # credentials on the face, whose policy carries none of the denies.
        fl = {k: sum(1 for x in w.fls.of(f"{b}/{k}") if x.kind == "put")
              for k in ("stream/air/frames_total", "@stream/raw", "stream/air/dropped_total")}
        fl_changes = w.g_events(b, "change", w.fls)
        report.check(sec, "measured (F-118): a session with local's credentials over the face, a principal whose "
                          "policy carries none of the plan's denies, sees the owner's tokens and receives every push: "
                          "the denies hold only for the far principal",
                     any("/@zk/instance/" in k for k in w.flt.rows) and fl["stream/air/frames_total"] >= 90
                     and fl["@stream/raw"] >= 90 and len(fl_changes) == 5,
                     f"tokens {len(w.flt.rows)}, pushes {fl}, change {len(fl_changes)}")
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
        found = link.check_block(w.block_view())
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


SECTIONS = {"1": section1, "2": section2, "3": section3, "4": section4, "5": section5, "6": section6}


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

"""Access control, live (core.md §11, 0.14–0.15; security.md §1–§3): zk2py's
own deployment, compiled by :mod:`zk2py.acl` into zenoh's
``access_control`` block, run on a zenoh-python router R1 that binds
principals by usrpwd.

The deployment (``deployment()``):
- ``own-h1`` and ``own-h2``: zk2py owners of ``h1/tc`` and ``h2/tc``, each
  serving ``zk2py_echo.v1``, ``zk2py_tc.v1`` and ``zk2py_bringup.v1``;
- ``consumer`` (``ops/dash``): Consume ``zk2py_echo.v1``'s ``state/health``
  from ``*/tc``;
- ``caller`` (``ops/ctl``): Call ``@op/echo`` and
  ``@op/interfaces/{if}/set`` on ``*/tc``;
- ``caller-blind`` (``ops/blind``): the same Call with its presence
  removed, security.md §1's "with its liveliness reads removed";
- ``tool``: the Tool shape, Consume ``state/health``, Call
  ``@op/diagnostics``, presence on ``*/tc``, and the admin read (0.15);
- ``S-enrolled`` (user ``spoofer``): an enrolled principal holding nothing
  but the open contract grants, security.md §3 step 3's enrolled ``S``;
- ``S`` (user ``stranger``): authenticated by R1, and no principal of the
  deployment, step 3's other ``S``.
"""

from __future__ import annotations

import json
import os
import tempfile
import threading
import time
from typing import Any

PASSWORD = "pw-{}"
USERS = {"own-h1": "h1tc", "own-h2": "h2tc", "consumer": "dash", "caller": "ctl", "caller-blind": "blind",
         "tool": "tool", "S-enrolled": "spoofer"}
STRANGER = "stranger"


def deployment(repo) -> list:
    from .acl import Principal, Use
    from .contract import load_contract

    contracts = [load_contract(repo / f"impl/python/interop/{n}.v1.toml")
                 for n in ("zk2py_echo", "zk2py_tc", "zk2py_bringup")]
    health = Use("zk2py_echo.v1", ["*/tc"], ["state/health"])
    calls = [Use("zk2py_echo.v1", ["*/tc"], ["@op/echo"]),
             Use("zk2py_tc.v1", ["*/tc"], ["@op/interfaces/{if}/set"])]
    return [
        Principal("own-h1", USERS["own-h1"], service="h1/tc", implements=contracts),
        Principal("own-h2", USERS["own-h2"], service="h2/tc", implements=contracts),
        Principal("consumer", USERS["consumer"], service="ops/dash", consumes=[health]),
        Principal("caller", USERS["caller"], service="ops/ctl", calls=list(calls)),
        Principal("caller-blind", USERS["caller-blind"], service="ops/blind", calls=list(calls), presence=False),
        Principal("tool", USERS["tool"], consumes=[health],
                  calls=[Use("zk2py_tc.v1", ["*/tc"], ["@op/diagnostics"])], inspects=["*/tc"], admin_read=True),
        # security.md §3 step 3 (0.15): S as an enrolled principal, which
        # holds nothing but the open contract grants.
        Principal("S-enrolled", USERS["S-enrolled"]),
    ]


class World:
    """R1 with the generated block, the deployment's sessions, and S."""

    def __init__(self, repo, block: dict[str, Any], workdir: str):
        import zenoh

        from . import live
        from .contract import load_contract
        from .owner import Owner, free_loopback_port

        self.dictionary = os.path.join(workdir, "users.txt")
        with open(self.dictionary, "w") as f:
            for u in [*USERS.values(), STRANGER]:
                f.write(f"{u}:{PASSWORD.format(u)}\n")
        port = free_loopback_port()
        conf = zenoh.Config()
        conf.insert_json5("mode", json.dumps("router"))
        conf.insert_json5("listen/endpoints", json.dumps([f"tcp/127.0.0.1:{port}"]))
        conf.insert_json5("scouting/multicast/enabled", "false")
        conf.insert_json5("timestamping/enabled", "true")
        conf.insert_json5("transport/auth/usrpwd", json.dumps({"dictionary_file": self.dictionary}))
        conf.insert_json5("adminspace", json.dumps({"enabled": True,
                                                    "permissions": {"read": True, "write": False}}))
        conf.insert_json5("access_control", json.dumps(block))
        self.router = zenoh.open(conf)
        self.zid = str(self.router.zid())
        self.endpoint = f"tcp/127.0.0.1:{port}"
        self.sessions: dict[str, Any] = {}
        for pid in ("consumer", "caller", "caller-blind", "tool", "S-enrolled"):
            u = USERS[pid]
            self.sessions[pid] = live.open_client(self.endpoint, (u, PASSWORD.format(u)))
        self.sessions["S"] = live.open_client(self.endpoint, (STRANGER, PASSWORD.format(STRANGER)))
        contracts = [load_contract(repo / f"impl/python/interop/{n}.v1.toml")
                     for n in ("zk2py_echo", "zk2py_tc", "zk2py_bringup")]
        self.owners = {
            pid: Owner(system, "tc", contracts, connect=self.endpoint,
                       auth=(USERS[pid], PASSWORD.format(USERS[pid])))
            for pid, system in (("own-h1", "h1"), ("own-h2", "h2"))
        }
        self.contracts = {c.interface: c for c in contracts}
        self.samples: dict[str, list[tuple[str, bytes]]] = {}
        self._entities: list[Any] = []

    def subscribe(self, pid: str, selector: str, name: str) -> None:
        import zenoh

        got = self.samples.setdefault(name, [])
        self._entities.append(self.sessions[pid].declare_subscriber(selector, zenoh.handlers.Callback(
            lambda s: got.append((str(s.key_expr), s.payload.to_bytes())))))

    def start_owners(self) -> None:
        from . import live

        for o in self.owners.values():
            o.start()
        deadline = time.monotonic() + 5.0
        while time.monotonic() < deadline:
            if len(live.list_presence(self.sessions["tool"], "zk2/*/tc/@zk/alive/**").alive) >= 6:
                break
            time.sleep(0.1)
        time.sleep(0.3)

    def close(self) -> None:
        for e in self._entities:
            try:
                e.undeclare()
            except Exception:  # noqa: BLE001 - closing anyway
                pass
        for o in self.owners.values():
            o.close()
        for s in self.sessions.values():
            s.close()
        self.router.close()


def _codes(res) -> list[str]:
    return sorted((r.envelope or {}).get("code", r.kind) for r in res.replies)


def _keys(res) -> list[str]:
    return sorted(r.key for r in res.replies if r.kind == "value")


def run_python_acl(report, repo) -> None:
    """security.md §1–§3 step 3, and 0.14's measured facts, on generated
    grants. Each posture or variant gets a router of its own."""
    from . import acl, live

    principals = deployment(repo)
    workdir = tempfile.mkdtemp(prefix="zk2py-acl-")
    deny = acl.generate(principals, "deny")
    allow = acl.generate(principals, "allow")
    report.info("acl", f"deny: {len(deny.block['rules'])} rules; allow: {len(allow.block['rules'])} rules, "
                       f"{len(allow.warnings)} complement_partial warnings")
    # §11.2 (0.14): an R2-narrowed grant has no complement by inclusion.
    narrowed = deployment(repo)
    narrowed[2].consumes.append(acl.Use("zk2py_bringup.v1", ["h1/tc"], ["state/tracks/t1"]))
    warned = acl.generate(narrowed, "allow").warnings
    report.check("acl", "§11.2 (0.14): a binding narrowed to one member (tracks/t1) has no complement by "
                        "inclusion under allow, and the generator warns complement_partial",
                 any("complement_partial" in x and "state/tracks/*" in x for x in warned)
                 and not allow.warnings, f"{len(warned)} warnings: {warned[:2]}")
    _deny_full(report, repo, deny.block, workdir)
    _deny_variant(report, repo, acl.generate(principals, "deny", reply_selectors=False).block, workdir, "reply")
    _deny_variant(report, repo, acl.generate(principals, "deny", egress_selectors=False).block, workdir, "egress")
    _allow_full(report, repo, allow.block, workdir)
    _allow_rules_only(report, repo, acl.generate(principals, "allow", allow_rules_under_allow=True).block, workdir)
    _unauthenticated(report, repo, allow.block, workdir)


def _forgeries(w: World) -> dict[str, Any]:
    """own-h1 writing as own-h2: a put, a queryable and a token on h2's keys,
    and a put on a wildcard key."""
    import zenoh

    h1 = w.owners["own-h1"].session
    forged_q = h1.declare_queryable("zk2/h2/tc/zk2py_echo.v1/state/health", zenoh.handlers.Callback(
        lambda q: q.reply("zk2/h2/tc/zk2py_echo.v1/state/health", b"forged")))
    forged_t = h1.liveliness().declare_token("zk2/h2/tc/@zk/instance/" + "f" * 16)
    time.sleep(0.3)
    h1.put("zk2/h2/tc/zk2py_echo.v1/state/health", b"forged-put")
    h1.put("zk2/*/tc/zk2py_echo.v1/state/health", b"wildcard-put")
    time.sleep(0.3)
    return {"q": forged_q, "t": forged_t}


def _deny_full(report, repo, block, workdir) -> None:
    from . import live

    run = "acl: generated grants under deny (security.md §1, §3 step 3; core §11, 0.14)"
    w = World(repo, block, workdir)
    try:
        w.subscribe("consumer", "zk2/*/tc/zk2py_echo.v1/state/health", "named")
        w.subscribe("consumer", "zk2/h1/tc/zk2py_bringup.v1/state/health", "unnamed")
        w.subscribe("caller", "zk2/*/tc/@zk/instance/*", "caller-descriptors")
        w.subscribe("caller-blind", "zk2/*/tc/@zk/instance/*", "blind-descriptors")
        time.sleep(0.3)
        w.start_owners()
        h1, h2 = w.owners["own-h1"], w.owners["own-h2"]
        h1.set_state("zk2/h1/tc/zk2py_echo.v1/state/health", b"v1")
        h1.set_state("zk2/h1/tc/zk2py_bringup.v1/state/health", b"secret")
        time.sleep(0.3)
        report.check(run, "§1: an owner's put on its own key reaches the consumer whose bindings name it",
                     ("zk2/h1/tc/zk2py_echo.v1/state/health", b"v1") in w.samples["named"],
                     str(w.samples["named"]))
        report.check(run, "§1: a subscription the bindings do not name is blocked",
                     not w.samples["unnamed"], str(w.samples["unnamed"]))
        fan = live.get_state(w.sessions["consumer"], "zk2/*/tc/zk2py_echo.v1/state/health")
        report.check(run, "§1: the consumer's fan-in GET over zk2/*/tc/… gets one reply per backend",
                     sorted(r.key for r in fan.replies) == ["zk2/h1/tc/zk2py_echo.v1/state/health",
                                                            "zk2/h2/tc/zk2py_echo.v1/state/health"],
                     str([(r.key, r.payload) for r in fan.replies]))
        held = _forgeries(w)
        try:
            again = live.get_state(w.sessions["consumer"], "zk2/*/tc/zk2py_echo.v1/state/health")
            pres = live.list_presence(w.sessions["tool"], "zk2/h2/tc/@zk/instance/*")
            report.check(run, "§1: a put, a queryable and a token on another principal's keys are blocked, and so "
                              "is a put on a wildcard key, which deny never granted",
                         not any(p in (b"forged-put", b"wildcard-put") for _, p in w.samples["named"])
                         and b"forged" not in [r.payload for r in again.replies]
                         and [i["instance"] for i in pres.instances] == [h2.instance],
                         f"samples {w.samples['named']}; GET {[r.payload for r in again.replies]}; "
                         f"h2 instances {[i['instance'] for i in pres.instances]}")
        finally:
            held["q"].undeclare()
            held["t"].undeclare()
        # Presence within the grants (0.8), and the descriptor rides it (0.14).
        seen = live.list_presence(w.sessions["caller"], "zk2/h1/tc/@zk/**")
        blind = live.list_presence(w.sessions["caller-blind"], "zk2/h1/tc/@zk/**")
        report.check(run, "§1: the caller reads h1's instance and interface tokens; with its presence removed, "
                          "the same read is complete and empty",
                     [i["instance"] for i in seen.instances] == [h1.instance] and len(seen.alive) == 3
                     and blind.complete and blind.count == 0, f"{seen.reading} | {blind.reading}")
        d = live.get_descriptor(w.sessions["caller"], h1.instance_key)
        db = live.get_descriptor(w.sessions["caller-blind"], h1.instance_key)
        subs = sorted(json.loads(p)["instance"] for _, p in w.samples["caller-descriptors"])
        report.check(run, "0.14: presence reads the descriptor: the caller's GET is answered and its subscription "
                          "received both first descriptors; without presence both are silent",
                     len(d) == 1 and d[0].ok and not db and subs == sorted([h1.instance, h2.instance])
                     and not w.samples["blind-descriptors"],
                     f"GET {len(d)} / {len(db)}; subscription {subs} / {len(w.samples['blind-descriptors'])}")
        fetched = {}
        for pid in ("consumer", "caller", "tool"):
            c = w.contracts["zk2py_echo.v1"]
            fetched[pid] = live.retrieve_bundle(w.sessions[pid], c.interface, c.fingerprint).available
        report.check(run, "§1: a contract fetch works for any principal", all(fetched.values()), str(fetched))
        # 0.14: value replies by their own key, refusals by the query's.
        diag = live.call(w.sessions["tool"], "zk2/*/tc/zk2py_tc.v1/@op/diagnostics", b"x", fanout=True)
        sets = live.call(w.sessions["caller"], "zk2/*/tc/zk2py_tc.v1/@op/interfaces/*/set", b"x", fanout=True)
        report.check(run, "with the full grants: a granted fan-out call gets each value, and a granted wildcard "
                          "call to a forbidden operation gets each refusal",
                     _keys(diag) == ["zk2/h1/tc/zk2py_tc.v1/@op/diagnostics", "zk2/h2/tc/zk2py_tc.v1/@op/diagnostics"]
                     and _codes(sets) == ["fanout_forbidden", "fanout_forbidden"], f"{_keys(diag)} {_codes(sets)}")
        # §3 step 3: S's queryable on R1's own key.
        _admin_spoof(report, run, w, "deny")
    finally:
        w.close()


def _admin_spoof(report, run, w: World, posture: str) -> None:
    """security.md §3 step 3 (0.15): "repeat step 1 twice: once with S an
    enrolled principal, and once with S an authenticated session that is
    no principal". And the Tool's admin read (§11.1, 0.15): under deny,
    the tool reads @/*/router and runs S4; a principal without it cannot."""
    import zenoh

    from . import live

    own = f"@/{w.zid}/router"
    spoofers = {"S, enrolled": w.sessions["S-enrolled"], "S, no principal": w.sessions["S"]}
    zids = {name: str(s.zid()) for name, s in spoofers.items()}

    def read() -> tuple[list[tuple[str, str | None]], Any]:
        got = [(a.key, a.replier) for a in live._answers(w.sessions["tool"], "@/*/router",
                                                          zenoh.QueryTarget.ALL, 1.0) if a.ok]
        return got, live.check_s4(w.sessions["tool"])

    for name, session in spoofers.items():
        q = session.declare_queryable(own, zenoh.handlers.Callback(
            lambda qq: qq.reply(own, json.dumps({"plugins": None}), encoding="application/json")))
        time.sleep(0.3)
        try:
            answers, s4 = read()
        finally:
            q.undeclare()
        if name == "S, enrolled" or posture == "deny":
            report.check(run, f"§3 step 3 ({name}): its queryable on @/<R1>/router is refused; only R1's own "
                              "answer arrives, verified, and S4 reads clean",
                         answers == [(own, w.zid)] and s4.verdict == "clean",
                         f"answers {answers}; {s4.verdict}: {s4.detail}")
        else:
            # §11.3: "A session that matches no subject gets no policy"; under
            # allow it gets everything, and only the replier id holds (0.15).
            report.check(run, f"§3 step 3 ({name}) under allow: it matches no subject and gets everything, so its "
                              "answer on R1's key arrives, under its own replier id, unverified, and S4 is not "
                              "clean (§4.2)",
                         (own, zids[name]) in answers and s4.verdict == "unobservable"
                         and (own, zids[name], "a replier other than its key's router") in s4.unverified,
                         f"answers {answers}; {s4.verdict}: {s4.detail}")
    unread = [a for a in live._answers(w.sessions["caller"], "@/*/router", zenoh.QueryTarget.ALL, 1.0)]
    if posture == "deny":
        answers, s4 = read()
        report.check(run, "§11.1 (0.15), the Tool's admin read: under deny the tool reads @/*/router and runs "
                          "S4 (clean); the caller, without it, gets nothing",
                     answers == [(own, w.zid)] and s4.verdict == "clean" and not unread,
                     f"tool {answers}, {s4.verdict}; caller {len(unread)} answers")
    else:
        report.info(run, f"under allow, the caller reads @/*/router too: {len(unread)} answers (the admin "
                         "space is outside the complement's key set, §11.2)")


def _deny_variant(report, repo, block, workdir, which: str) -> None:
    from . import live

    run = (f"acl: deny, security.md §2's generator check, without the consumer selectors in the providers' "
           f"{'ingress reply' if which == 'reply' else 'egress'}")
    w = World(repo, block, workdir)
    try:
        w.start_owners()
        if which == "egress":
            fan = live.get_state(w.sessions["consumer"], "zk2/*/tc/zk2py_echo.v1/state/health")
            one = live.get_state(w.sessions["consumer"], "zk2/h1/tc/zk2py_echo.v1/state/health")
            report.check(run, "the fan-in GET over zk2/*/tc/… gets 0 replies; a concrete GET, inside the "
                              "provider's Own egress, still gets its one",
                         not fan.replies and [r.key for r in one.replies] == ["zk2/h1/tc/zk2py_echo.v1/state/health"],
                         f"fan-in {len(fan.replies)}, concrete {[r.key for r in one.replies]}")
        else:
            diag = live.call(w.sessions["tool"], "zk2/*/tc/zk2py_tc.v1/@op/diagnostics", b"x", fanout=True)
            sets = live.call(w.sessions["caller"], "zk2/*/tc/zk2py_tc.v1/@op/interfaces/*/set", b"x", fanout=True)
            handled = sum(len([c for c in o.calls if c.endswith("/set")]) for o in w.owners.values())
            report.check(run, "0.14: a value reply is checked against its own key: the wildcard diagnostics call "
                              "still gets both values through the providers' Own",
                         _keys(diag) == ["zk2/h1/tc/zk2py_tc.v1/@op/diagnostics",
                                         "zk2/h2/tc/zk2py_tc.v1/@op/diagnostics"], str(_keys(diag)))
            report.check(run, "0.14: a refusal is checked against the query's key: both providers refuse the "
                              "wildcard set call, and no refusal reaches the caller",
                         not sets.replies and handled == 2, f"{len(sets.replies)} replies, {handled} queries reached")
    finally:
        w.close()


def _allow_full(report, repo, block, workdir) -> None:
    from . import live

    run = "acl: generated grants under allow (security.md §2, §3 step 3; core §11.3, 0.14)"
    w = World(repo, block, workdir)
    try:
        w.subscribe("consumer", "zk2/*/tc/zk2py_echo.v1/state/health", "named")
        w.subscribe("consumer", "zk2/h1/tc/zk2py_bringup.v1/state/health", "unnamed")
        time.sleep(0.3)
        w.start_owners()
        h1, h2 = w.owners["own-h1"], w.owners["own-h2"]
        h1.set_state("zk2/h1/tc/zk2py_bringup.v1/state/health", b"secret")
        time.sleep(0.2)
        held = _forgeries(w)
        try:
            again = live.get_state(w.sessions["consumer"], "zk2/*/tc/zk2py_echo.v1/state/health")
            pres = live.list_presence(w.sessions["tool"], "zk2/h2/tc/@zk/instance/*")
            payloads = [p for _, p in w.samples["named"]]
            report.check(run, "§2: a put, a queryable and a token on another principal's keys are blocked, and so "
                              "is a subscription the bindings do not name",
                         b"forged-put" not in payloads and b"forged" not in [r.payload for r in again.replies]
                         and [i["instance"] for i in pres.instances] == [h2.instance] and not w.samples["unnamed"],
                         f"samples {payloads}; GET {[r.payload for r in again.replies]}; "
                         f"h2 instances {[i['instance'] for i in pres.instances]}; unnamed {w.samples['unnamed']}")
            report.check(run, "§2/§11.3: except a put on a wildcard key, which reaches the consumer (R6 discards it "
                              "there)", ("zk2/*/tc/zk2py_echo.v1/state/health", b"wildcard-put") in w.samples["named"],
                         str(w.samples["named"]))
        finally:
            held["q"].undeclare()
            held["t"].undeclare()
        granted = live.get_state(w.sessions["consumer"], "zk2/*/tc/zk2py_echo.v1/state/health")
        ungranted = live.get_state(w.sessions["caller"], "zk2/*/tc/zk2py_echo.v1/state/health")
        report.check(run, "0.14: an ungranted wildcard GET (the caller's) gets nothing back, while the consumer's "
                          "gets both", not ungranted.replies and len(granted.replies) == 2,
                     f"caller {len(ungranted.replies)}, consumer {len(granted.replies)}")
        before = sum(len(o.handled) for o in w.owners.values())
        diag = live.call(w.sessions["consumer"], "zk2/*/tc/zk2py_tc.v1/@op/diagnostics", b"x", fanout=True)
        ran = sum(len(o.handled) for o in w.owners.values()) - before
        report.check(run, "0.14: a wildcard call to a fanout = \"allowed\" operation, never granted to the consumer, "
                          "executes on every provider; only its answers are denied",
                     ran == 2 and not diag.replies, f"{ran} executions, {len(diag.replies)} replies")
        blind = live.list_presence(w.sessions["caller-blind"], "zk2/*/tc/@zk/**")
        seen = live.list_presence(w.sessions["caller"], "zk2/*/tc/@zk/**")
        report.check(run, "presence within the grants: the caller reads both owners' tokens; the blind caller's "
                          "wildcard read is complete and empty",
                     len(seen.instances) == 2 and blind.complete and blind.count == 0,
                     f"{seen.reading} | {blind.reading}")
        stranger = live.get_state(w.sessions["S"], "zk2/*/tc/zk2py_echo.v1/state/health")
        report.check(run, "§11.3: S, authenticated but no principal, matches no deployment subject and reads "
                          "everything under allow", len(stranger.replies) == 2, f"{len(stranger.replies)} replies")
        _admin_spoof(report, run, w, "allow")
    finally:
        w.close()


def _allow_rules_only(report, repo, block, workdir) -> None:
    from . import live

    run = "acl: the deny posture's allow rules, with default_permission allow (security.md §2)"
    w = World(repo, block, workdir)
    try:
        w.subscribe("consumer", "zk2/*/tc/zk2py_echo.v1/state/health", "named")
        time.sleep(0.3)
        w.start_owners()
        w.owners["own-h1"].session.put("zk2/h2/tc/zk2py_echo.v1/state/health", b"forged-put")
        time.sleep(0.3)
        report.check(run, "allow rules alone, under allow, block nothing: own-h1's put on h2's key arrives",
                     ("zk2/h2/tc/zk2py_echo.v1/state/health", b"forged-put") in w.samples["named"],
                     str(w.samples["named"]))
    finally:
        w.close()


def _unauthenticated(report, repo, block, workdir) -> None:
    import zenoh

    from . import live

    run = "acl: §11.3, a deployment under allow refuses unauthenticated sessions at the link"
    w = World(repo, block, workdir)
    try:
        outcomes = {}
        for name, auth in (("no credentials", None), ("a wrong password", (STRANGER, "nope"))):
            try:
                live.open_client(w.endpoint, auth).close()
                outcomes[name] = "connected"
            except zenoh.ZError:
                outcomes[name] = "refused"
        report.check(run, "with usrpwd on R1, a session with no credentials or a wrong password is refused at "
                          "the link", set(outcomes.values()) == {"refused"}, str(outcomes))
    finally:
        w.close()

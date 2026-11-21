"""``link.v2``'s ``scenarios-v2.md`` §2, "Frames per face" (§2.7, §2.8;
``link.v1`` §2.9), live, on an in-process zenoh-python router (zenoh
1.10.1; #609, round 111, the carry of round 110): ``python -m
zk2py.link_v2_frames_live [--only py|ref]``.

**The setup, as §2 writes it.** The contract is ``probe.v1``
(``spec/profiles/link/conformance/contracts/probe.v1.toml``).
``lab/probe-a`` implements it, as the user ``probe-a``. The console, run by
``ground`` and ``local``, binds every stream, state and event of
``probe.v1`` from ``lab/*``. The face policy is the Conventions', its
``services`` ``["probe-a"]``, its face listing one frame,
``probe.v1/frames/link`` every second.

- **py**: N runs zk2py's generated configuration: the grants of
  :func:`zk2py.acl.generate` under ``deny``, the face's denies and its
  downsampling rule compiled in by :func:`zk2py.link.compile_faces` from
  :func:`zk2py.link_v2.plan`'s face, and :func:`zk2py.link_v2.block`'s three
  objects. Expected 1 and 2 are read on that plan.
- **ref**: N runs the fragment ``zenctl acl gen --json5`` writes (a black
  box, run) from the same policy, the same contract and an enrollment the
  runner writes in the reference's format, as
  ``examples/zk2/link/radio.enrollment.toml`` shows it. zenoh loads the
  fragment as the tool wrote it, and the runner inserts what a fragment
  leaves to the deployment: the mode, the endpoints and the dictionary.

**The sessions.** The owner is a client session of N on the near side as
``probe-a``. For 5 s it puts at 20 Hz on ``frames/link``,
``frames/detail``, ``setpoint`` and ``level``, and at 2 Hz on its heartbeat
covering ``probe.v1``. G, a client as ``ground``, attaches over the face;
L, as ``local``, on the near side. Each subscribes to what the console is
granted, one subscriber a resource and one for the heartbeat. The payloads
are counters: what crosses is decided by keys, and no reader decodes one
here.

Working files go in a directory of the run's own, removed at its end
(:func:`zk2py.blackbox.workdir`). Exit 0 when every check passes, 1 when
any fails.
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

from .blackbox import BLACKBOX, workdir
from .link_v2_live import _config
from .stamp_live import _close_all, conventions_policy

REPO = Path(__file__).resolve().parents[3]
PROBE = REPO / "spec" / "profiles" / "link" / "conformance" / "contracts" / "probe.v1.toml"
SERVICE = "lab/probe-a"
BASE = f"zk2/{SERVICE}/probe.v1"
HEARTBEAT = f"zk2/{SERVICE}/freshness.v2/stream/heartbeat/probe.v1"
#: step 3's four keys, by the resource's name in §2
PUT = {"frames/link": f"{BASE}/stream/frames/link", "frames/detail": f"{BASE}/stream/frames/detail",
       "setpoint": f"{BASE}/stream/setpoint", "level": f"{BASE}/stream/level"}
USERS = ("probe-a", "local", "ground")
HZ, SECONDS, BEAT_HZ = 20, 5.0, 2

Result = tuple[str, bool, str]


def policy() -> str:
    """§2's face policy: "the Conventions', its services set to
    ["probe-a"]. Its face lists one frame"."""
    text = conventions_policy().replace('services = ["radio-a"]', 'services = ["probe-a"]')
    if '["probe-a"]' not in text:
        raise RuntimeError("the Conventions' policy names no services = [\"radio-a\"]")
    return text + 'frames = { "probe.v1/frames/link" = { every_s = 1 } }\n'


def enrollment_text() -> str:
    """§2's deployment in the reference's enrollment format, as the example
    enrollment writes one: a principal runs ``services`` or ``tools``, and a
    tool's binding with no ``resources`` takes every stream, state and event
    resource of its interface."""
    return ('[[principal]]\nuser     = "probe-a"\nservices = ["lab/probe-a"]\n\n'
            '[[principal]]\nuser  = "local"\ntools = ["console"]\n\n'
            '[[principal]]\nuser  = "ground"\ntools = ["console"]\n\n'
            '[[service]]\naddress    = "lab/probe-a"\nimplements = ["probe.v1"]\n\n'
            '[[tool]]\nname = "console"\n\n'
            '[tool.bindings.probe]\ninterface = "probe.v1"\nproviders = ["lab/*"]\n')


def console_selectors(contract: Any) -> dict[str, str]:
    """What the console is granted, a selector a resource: every stream,
    state and event of ``probe.v1`` from ``lab/*``, and the heartbeat its
    binding needs (core §11.1, Consume)."""
    out = {f"{r['token']}/{r['template']}": f"zk2/lab/*/probe.v1/{r['token']}/{r['template']}"
           for r in contract.canonical["resources"] if r["token"] in ("stream", "state", "events")}
    out["heartbeat"] = "zk2/lab/*/freshness.v2/stream/heartbeat/probe.v1"
    return out


def zk2py_config(policy_text: str, work: str, near: str, face: str) -> tuple[dict[str, Any], dict[str, Any]]:
    """N's whole configuration as zk2py's generator writes it for §2's
    deployment, and the face's plan."""
    from . import acl, link, link_v2
    from .contract import load_contract

    contract = load_contract(PROBE, spec_dir=REPO / "spec")
    pol, findings = link_v2.parse_policy(policy_text)
    if pol is None:
        raise RuntimeError(f"§2's policy is refused: {findings}")
    bound = sorted(f"{r['token']}/{r['template']}" for r in contract.canonical["resources"]
                   if r["token"] in ("stream", "state", "events"))
    principals = [acl.Principal("probe-a", "probe-a", service=SERVICE, implements=[contract], consumes=[], calls=[])]
    principals += [acl.Principal(u, u, service=None, implements=[],
                                 consumes=[acl.Use("probe.v1", ["lab/*"], bound)], calls=[])
                   for u in ("local", "ground")]
    p = link_v2.plan(policy_text, {"probe-a": ["probe.v1"]}, {"probe.v1": {"revisions": [link.revision_of(contract)]}},
                     users=list(USERS))
    if "refused" in p:
        raise RuntimeError(f"the generator refuses the deployment: {p['refused']}")
    blk = link.compile_faces(pol.as_v1(), p["faces"], acl.generate(principals, "deny").block)
    conf: dict[str, Any] = {"mode": "router", "listen": {"endpoints": [near, face]},
                            "scouting": {"multicast": {"enabled": False}},
                            "transport": {"auth": {"usrpwd": {"dictionary_file": os.path.join(work, "users.txt")}}},
                            "access_control": blk}
    for k, v in link.router_extras(p["faces"][0]).items():
        conf[k] = v
    return link_v2.merge(conf, link_v2.block(pol)), p["faces"][0]


class Counted:
    """One subscriber a selector, counting what each receives by key."""

    def __init__(self, session: Any, selectors: dict[str, str]) -> None:
        import zenoh

        self.lock = threading.Lock()
        self.got: dict[str, int] = {}
        self.refused: dict[str, str] = {}
        self.subs = []
        for name, sel in selectors.items():
            try:
                self.subs.append(session.declare_subscriber(sel, zenoh.handlers.Callback(self._on)))
            except Exception as e:  # noqa: BLE001 - reported
                self.refused[name] = f"{type(e).__name__}: {e}"[:120]

    def _on(self, s: Any) -> None:
        k = str(s.key_expr)
        with self.lock:
            self.got[k] = self.got.get(k, 0) + 1

    def count(self, key: str) -> int:
        with self.lock:
            return self.got.get(key, 0)

    def close(self) -> None:
        for s in self.subs:
            try:
                s.undeclare()
            except Exception:  # noqa: BLE001 - closing anyway
                pass


def traffic(owner: Any) -> dict[str, int]:
    """Step 3: 20 Hz for 5 s on the four keys, and the heartbeat at 2 Hz.
    How many puts each key took."""
    sent = {k: 0 for k in [*PUT.values(), HEARTBEAT]}
    n = int(HZ * SECONDS)
    every = HZ // BEAT_HZ
    t0 = time.monotonic()
    for i in range(n):
        due = t0 + i / HZ
        wait = due - time.monotonic()
        if wait > 0:
            time.sleep(wait)
        for key in PUT.values():
            owner.put(key, f"{i}".encode())
            sent[key] += 1
        if i % every == 0:
            owner.put(HEARTBEAT, f"beat {i}".encode())
            sent[HEARTBEAT] += 1
    return sent


def run(reference: bool) -> list[Result]:
    with workdir("zk2py-frames-") as work:
        try:
            return _run(work, reference)
        except Exception as e:  # noqa: BLE001 - a run that could not finish is a failure
            return [(f"§2{' (the reference block)' if reference else ''}: the run finished", False,
                     f"{type(e).__name__}: {e}")]


def _run(work: str, reference: bool) -> list[Result]:
    import zenoh

    from . import live
    from .contract import load_contract
    from .owner import free_loopback_port

    out: list[Result] = []
    tag = "§2, zenctl acl gen's block" if reference else "§2, zk2py's block"
    policy_text = policy()
    contract = load_contract(PROBE, spec_dir=REPO / "spec")
    face = os.path.join(work, "face.sock")
    near = f"tcp/127.0.0.1:{free_loopback_port()}"
    users = os.path.join(work, "users.txt")
    with open(users, "w") as f:
        f.write("".join(f"{u}:{u}-pw\n" for u in USERS))
    # Step 1: the generator compiles N's block.
    if reference:
        ppath, epath = os.path.join(work, "probe.face.toml"), os.path.join(work, "probe.enrollment.toml")
        with open(ppath, "w") as f:
            f.write(policy_text)
        with open(epath, "w") as f:
            f.write(enrollment_text())
        p = subprocess.run([str(BLACKBOX / "zenctl"), "acl", "gen", "--enrollment", epath, "--contracts", str(PROBE),
                            "--face-policy", ppath, "--json5"], capture_output=True, text=True, timeout=120)
        if p.returncode != 0:
            return [(f"{tag}: zenctl acl gen writes N's block", False,
                     f"exit {p.returncode}; {p.stderr.strip()[-400:]}")]
        nc = _config(p.stdout, {"mode": "router", "listen/endpoints": [near, f"unixsock-stream/{face}"],
                                "scouting/multicast/enabled": False,
                                "transport/auth/usrpwd/dictionary_file": users})
        rules = [r.get("key_expr") for d in json.loads(nc.get_json("downsampling")) or []
                 for r in d.get("rules") or []]
        warned = sorted({c for c in ("timed_downsampled", "frame_unlisted", "keep_alive_not_suppressed")
                         if c in p.stderr})
        out.append((f"{tag}, Expected 2: its one downsampling rule is level's, and it warns timed_downsampled, "
                    "frame_unlisted and keep_alive_not_suppressed",
                    rules == [PUT["level"]]
                    and warned == ["frame_unlisted", "keep_alive_not_suppressed", "timed_downsampled"],
                    f"rules {rules}; warned {warned}"))
    else:
        conf, plan = zk2py_config(policy_text, work, near, f"unixsock-stream/{face}")
        nc = _config(conf)
        denied = sorted(e for d in plan.get("deny", []) for e in d["key_exprs"] if "probe.v1/stream" in e)
        crosses = sorted(e for x in plan.get("liveness", []) for e in x["evidence"])
        out.append((f"{tag}, Expected 1: the plan lets frames/link, status and the heartbeat cross by its evidence, "
                    "and denies frames/detail and setpoint by name, level left to its downsampling rule",
                    denied == ["zk2/*/*/probe.v1/stream/frames/detail/**", "zk2/*/*/probe.v1/stream/setpoint/**"]
                    and crosses == ["freshness.v2/stream/heartbeat/probe.v1", "probe.v1/state/status",
                                    "probe.v1/stream/frames/link"],
                    f"denied {denied}; evidence {crosses}"))
        rules = [r["key_expr"] for r in (plan.get("downsampling") or {}).get("rules", [])]
        warned = sorted(x["code"] for x in plan.get("findings", []))
        out.append((f"{tag}, Expected 2: its one downsampling rule is level's, and it warns timed_downsampled, "
                    "frame_unlisted and keep_alive_not_suppressed",
                    rules == [PUT["level"]]
                    and warned == ["frame_unlisted", "keep_alive_not_suppressed", "timed_downsampled"],
                    f"rules {rules}; warned {warned}"))
    closers: list[Any] = []
    try:
        n = zenoh.open(nc)
        closers.append(n)
        time.sleep(0.3)
        # Step 2: G over the face as ground, L on the near side as local.
        selectors = console_selectors(contract)
        g = live.open_client(f"unixsock-stream/{face}", ("ground", "ground-pw"))
        closers.append(g)
        ell = live.open_client(near, ("local", "local-pw"))
        closers.append(ell)
        cg, cl = Counted(g, selectors), Counted(ell, selectors)
        closers += [cg, cl]
        owner = live.open_client(near, ("probe-a", "probe-a-pw"))
        closers.append(owner)
        time.sleep(1.0)
        # Step 3.
        sent = traffic(owner)
        time.sleep(1.5)
        got_l = {name: cl.count(key) for name, key in [*PUT.items(), ("heartbeat", HEARTBEAT)]}
        got_g = {name: cg.count(key) for name, key in [*PUT.items(), ("heartbeat", HEARTBEAT)]}
        want = {name: sent[key] for name, key in [*PUT.items(), ("heartbeat", HEARTBEAT)]}
        out.append((f"{tag}, Expected 3: L receives every put, 100 on each of the four keys and 10 heartbeats",
                    got_l == want and want == {"frames/link": 100, "frames/detail": 100, "setpoint": 100,
                                               "level": 100, "heartbeat": 10} and not cl.refused,
                    f"L {got_l}; put {want}; refused {cl.refused}"))
        out.append((f"{tag}, Expected 4: G receives every put of frames/link and of the heartbeat: neither is "
                    "downsampled",
                    got_g["frames/link"] == want["frames/link"] and got_g["heartbeat"] == want["heartbeat"],
                    f"G frames/link {got_g['frames/link']} of {want['frames/link']}, heartbeat "
                    f"{got_g['heartbeat']} of {want['heartbeat']}"))
        out.append((f"{tag}, Expected 5: G receives 4 to 6 puts of level, one a second",
                    4 <= got_g["level"] <= 6, f"G level {got_g['level']} of {want['level']}"))
        out.append((f"{tag}, Expected 6: G receives no put of frames/detail or setpoint",
                    got_g["frames/detail"] == 0 and got_g["setpoint"] == 0,
                    f"G frames/detail {got_g['frames/detail']}, setpoint {got_g['setpoint']}; its subscribers "
                    f"refused at the declaration: {sorted(cg.refused) or 'none'}"))
    finally:
        _close_all(closers)
    return out


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="python -m zk2py.link_v2_frames_live", description=__doc__.split("\n")[0])
    ap.add_argument("--only", action="append", choices=("py", "ref"))
    args = ap.parse_args(argv)
    runs = {"py": lambda: run(False), "ref": lambda: run(True)}
    failed = 0
    for name in args.only or list(runs):
        for what, ok, detail in runs[name]():
            print(f"{'PASS' if ok else 'FAIL'} {what}" + (f": {detail}" if detail else ""), flush=True)
            failed += not ok
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())

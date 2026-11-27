"""``link.v1`` 0.21's ``owner_heartbeat_denied``, zk2py's plan beside the
reference's tools (#609, round 126; #1145): ``python -m
zk2py.owner_heartbeat_tools_live``. No session is opened.

Each variant is one enrollment, one face policy and one set of contracts,
written to a working directory of the run's own:

- **The contracts** are ``plan.json`` case 18's interfaces as authoring
  files (``beacon.v2``, ``old.v1``, ``quiet.v2``, each listing ``link.v1``
  beside its ``freshness`` major, since its annotations name
  ``link.exposure``), with ``freshness.v2``'s standard contract; for the
  ``link.v2`` variant, ``v2/plan.json`` case 46's (``census.v1``,
  ``quiet.v1``, ``vital.v1``), their exposed collections packed.
- **The enrollment** names each service's address and interfaces, and
  ``owner_heartbeat = true`` on the services the variant turns on (the
  reference's key, link.v1 0.21's entry: "An enrollment's ``[[service]]``
  takes ``owner_heartbeat = true``").
- **The reference** is ``zenctl acl gen --face-policy --format json`` (its
  notes) and ``zenctl check face`` over a router configuration built from
  ``acl gen --json5``'s fragment (its output), each a black box, run.
- **zk2py** is :func:`zk2py.link.plan` (``link_v2.plan`` for a link.v2
  policy) over the same files, its ``owner_heartbeats`` the services the
  enrollment turns on.

Each variant's warnings, ``(face, service)``, must be the same three
times. Exit 0 when every comparison holds, 1 when any fails.
"""

from __future__ import annotations

import json
import re
import subprocess
import sys
import tomllib
from pathlib import Path
from typing import Any

from .blackbox import BLACKBOX, workdir

REPO = Path(__file__).resolve().parents[3]
FRESHNESS_V2 = REPO / "spec" / "profiles" / "freshness" / "freshness.v2.toml"
WARNING = re.compile(r"face (\S+): owner_heartbeat_denied (\S+):")

Result = tuple[str, bool, str]

RAW = '{ raw = "text/plain" }'


def _contract(name: str, major: int, uses: list[str], resources: dict[str, dict[str, Any]],
              packed: str | None = None) -> str:
    out = [f'[interface]\nname  = "{name}"\nmajor = {major}\nminor = 0\nuses  = {json.dumps(uses)}\n']
    if packed:
        out.append(f'[schemas]\npacked = ["{packed}"]\n')
    for res, r in resources.items():
        lines = [f'[resources."{res}"]', f'kind = "{r["kind"]}"', f'type = {r.get("type", RAW)}']
        if "cardinality" in r:
            lines.append(f'cardinality = {r["cardinality"]}')
            lines.append('params = { peer = "string" }')
        ann = r.get("annotations") or {}
        if ann:
            lines.append("annotations = { " + ", ".join(f'"{k}" = {json.dumps(v)}' for k, v in ann.items()) + " }")
        out.append("\n".join(lines) + "\n")
    return "\n".join(out)


def v1_contracts() -> dict[str, str]:
    """plan.json case 18's three interfaces, as authoring files."""
    link = {"link.exposure": "link"}
    return {
        "beacon.v2.toml": _contract("beacon", 2, ["link.v1", "freshness.v2"], {
            "status": {"kind": "state", "annotations": {"freshness.ttl_s": 120, **link}},
            "level": {"kind": "stream", "annotations": {"freshness.ttl_s": 300, **link}},
            "note": {"kind": "state", "annotations": link}}),
        "old.v1.toml": _contract("old", 1, ["link.v1", "freshness.v1"], {
            "mode": {"kind": "state", "annotations": {"freshness.ttl_s": 60, **link}}}),
        "quiet.v2.toml": _contract("quiet", 2, ["link.v1", "freshness.v2"], {
            "intent": {"kind": "state", "annotations": {"freshness.ttl_s": 0, **link}}}),
    }


#: A packed type for the link.v2 variant's exposed collections, its tag not
#: 0 (core 0.117's E051), checked where it is written.
STATUS_SCHEMA = {"types": {"Status": {"fields": [{"name": "level", "type": "int", "minimum": 0, "maximum": 100}]}}}


def v2_contracts() -> dict[str, str]:
    """v2/plan.json case 46's three interfaces, their exposed collections
    packed (a contract that lists link.v2 exposes only packed resources)."""
    link = {"link.exposure": "link"}
    packed_t = '"packed:Status"'
    return {
        "census.v1.toml": _contract("census", 1, ["link.v2", "freshness.v2"], {
            "status": {"kind": "state", "type": packed_t, "annotations": {"freshness.ttl_s": 600, **link}},
            "peer/{peer}": {"kind": "state", "type": packed_t, "cardinality": 1000,
                            "annotations": {"freshness.ttl_s": 600, **link}},
            "log": {"kind": "state", "annotations": {"freshness.ttl_s": 120}}}, packed="status.json"),
        "quiet.v1.toml": _contract("quiet", 1, ["link.v2", "freshness.v2"], {
            "note": {"kind": "state", "annotations": {"freshness.ttl_s": 60}}}),
        "vital.v1.toml": _contract("vital", 1, ["link.v2", "freshness.v2"], {
            "status": {"kind": "state", "type": packed_t, "annotations": {"freshness.ttl_s": 60, **link}}},
            packed="status.json"),
    }


V1_POLICY = '''system   = "lab"
services = ["b1", "b2"]

[face.rf]
far          = "ground"
attach       = "client"
select       = { link_protocols = ["unixsock-stream"] }
class        = "constrained"
expose       = "link"
interval_s   = 60
max_outage_s = 600
batch_bytes  = 1000
link_bps     = 2400
'''

#: The same deployment with a second face, which pushes nothing: no heartbeat
#: crosses it, so it warns of none.
V1_TWO_FACES = V1_POLICY + '''
[face.sat]
far          = "relay"
attach       = "client"
select       = { link_protocols = ["serial"] }
class        = "constrained"
expose       = "none"
max_outage_s = 600
batch_bytes  = 1000
link_bps     = 2400
'''

V2_POLICY = '''profile  = "link.v2"
system   = "lab"
services = ["node-a"]
binding  = "zenoh-1.10.1"

[transport]
lease_ms             = 240000
keep_alive           = 4
wait_before_close_ms = 240000
sequence             = "16bit"
batch_bytes          = 159

[face.radio]
far              = "ground"
attach           = "client"
select           = { link_protocols = ["unixsock-stream"] }
class            = "constrained"
expose           = "link"
interval_s       = 60
max_outage_s     = 3600
link_bps         = 2400
loss_ppm         = 50000
arq              = false
reply_timeout_ms = 15000
'''


def enrollment(services: dict[str, list[str]], on: dict[str, bool | None], fars: list[str]) -> str:
    """Each listed service a principal of its own, each far principal a
    console's, and ``owner_heartbeat`` written where ``on`` gives it."""
    out = []
    for s in services:
        out.append(f'[[principal]]\nuser     = "{s}"\nservices = ["lab/{s}"]\n')
    for f in fars:
        out.append(f'[[principal]]\nuser  = "{f}"\ntools = ["console"]\n')
    for s, ifs in services.items():
        lines = [f'[[service]]\naddress    = "lab/{s}"\nimplements = {json.dumps(ifs)}']
        if on.get(s) is not None:
            lines.append(f"owner_heartbeat = {'true' if on[s] else 'false'}")
        out.append("\n".join(lines) + "\n")
    out.append('[[tool]]\nname = "console"\n')
    return "\n".join(out)


def _ours(policy: str, services: dict[str, list[str]], on: dict[str, bool | None], contracts: Path,
          v2: bool) -> list[tuple[str, str]]:
    from . import link, link_v2
    from .contract import load_contract

    revisions: dict[str, Any] = {}
    for f in sorted(contracts.glob("*.toml")):
        c = load_contract(f)
        if not c.valid:
            raise RuntimeError(f"{f.name}: {c.codes}")
        revisions.setdefault(c.interface, {"revisions": []})["revisions"].append(link.revision_of(c))
    owner = [s for s, v in on.items() if v]
    got = (link_v2.plan(policy, services, revisions, owner_heartbeats=owner) if v2
           else link.plan(policy, services, revisions, owner_heartbeats=owner))
    if "refused" in got:
        raise RuntimeError(f"zk2py's plan refused: {got['refused']}")
    return sorted((f["face"], f["at"]) for fp in got["faces"] for f in fp["findings"]
                  if f["code"] == "owner_heartbeat_denied")


def _zenctl(*args: str) -> subprocess.CompletedProcess:
    return subprocess.run([str(BLACKBOX / "zenctl"), *args], capture_output=True, text=True, timeout=120)


def variant(name: str, *args: Any, **kw: Any) -> list[Result]:
    """One variant, its three readings; a variant that cannot finish is one
    failure, and the next runs."""
    try:
        return _variant(name, *args, **kw)
    except Exception as e:  # noqa: BLE001 - a run that could not finish is a failure
        return [(f"{name}: the run finished", False, f"{type(e).__name__}: {e}")]


def _variant(name: str, policy: str, services: dict[str, list[str]], on: dict[str, bool | None],
             fars: list[str], files: dict[str, str], want: list[tuple[str, str]], v2: bool = False) -> list[Result]:
    out: list[Result] = []
    with workdir("zk2py-ohb-") as d:
        base = Path(d)
        contracts = base / "contracts"
        contracts.mkdir()
        (contracts / "freshness.v2.toml").write_text(FRESHNESS_V2.read_text(encoding="utf-8"), encoding="utf-8")
        for fname, text in files.items():
            (contracts / fname).write_text(text, encoding="utf-8")
        if v2:
            (contracts / "status.json").write_text(json.dumps(STATUS_SCHEMA), encoding="utf-8")
        pol = base / "face.toml"
        pol.write_text(policy, encoding="utf-8")
        enr = base / "enrollment.toml"
        enr.write_text(enrollment(services, on, fars), encoding="utf-8")
        tomllib.loads(enr.read_text(encoding="utf-8"))
        ours = _ours(policy, services, on, contracts, v2)
        gen = _zenctl("acl", "gen", "--enrollment", str(enr), "--contracts", str(contracts), "--face-policy",
                      str(pol), "--format", "json")
        try:
            notes = [n["text"] for n in json.loads(gen.stdout).get("notes", [])]
        except ValueError:
            notes = []
        gen_json = sorted(WARNING.match(t).groups() for t in notes if WARNING.match(t))
        gen_err = sorted(WARNING.findall(gen.stderr))
        frag = _zenctl("acl", "gen", "--enrollment", str(enr), "--contracts", str(contracts), "--face-policy",
                       str(pol), "--json5")
        router = base / "router.json5"
        router.write_text("{\n" + frag.stdout + "\n}\n", encoding="utf-8")
        chk = _zenctl("check", "face", "--face-policy", str(pol), "--enrollment", str(enr), "--contracts",
                      str(contracts), str(router), "--format", "json")
        chk_all = sorted(WARNING.findall(chk.stdout + chk.stderr))
        chk_json = sorted(WARNING.findall(chk.stdout))
        print(f"NOTE {name}: acl gen exit {gen.returncode}, its JSON notes {gen_json}, its stderr {gen_err}; "
              f"check face exit {chk.returncode}, {chk_all} ({len(chk_json)} in its JSON document); zk2py {ours}")
        w = [tuple(x) for x in want]
        out.append((f"{name}: zk2py's plan warns {w}", ours == w, str(ours)))
        out.append((f"{name}: zenctl acl gen warns the same", [tuple(x) for x in gen_json] == w and
                    [tuple(x) for x in gen_err] == w and gen.returncode == 0, f"{gen_json} {gen_err} "
                                                                             f"exit {gen.returncode}"))
        out.append((f"{name}: zenctl check face warns the same, exit 0 (a plan's warning is no finding)",
                    [tuple(x) for x in chk_all] == w and chk.returncode == 0,
                    f"{chk_all} exit {chk.returncode}: {(chk.stdout + chk.stderr)[-400:]}"))
    return out


def run() -> list[Result]:
    s18 = {"b1": ["beacon.v2", "freshness.v2"], "b2": ["freshness.v2", "old.v1", "quiet.v2"]}
    out: list[Result] = []
    out += variant("case 18's deployment, both services on", V1_POLICY, s18, {"b1": True, "b2": True},
                   ["ground"], v1_contracts(), [("rf", "b1")])
    out += variant("b2 alone on, whose interfaces have no heartbeat", V1_POLICY, s18, {"b2": True},
                   ["ground"], v1_contracts(), [])
    out += variant("case 17's, no service named", V1_POLICY, s18, {}, ["ground"], v1_contracts(), [])
    out += variant("b1 written false", V1_POLICY, s18, {"b1": False, "b2": True}, ["ground"], v1_contracts(), [])
    out += variant("b1 on, freshness.v2 left to the plan to imply", V1_POLICY,
                   {"b1": ["beacon.v2"], "b2": ["old.v1", "quiet.v2"]}, {"b1": True}, ["ground"], v1_contracts(),
                   [("rf", "b1")])
    out += variant("two faces, the second pushing nothing", V1_TWO_FACES, s18, {"b1": True, "b2": True},
                   ["ground", "relay"], v1_contracts(), [("rf", "b1")])
    out += variant("v2/plan.json case 53's, a link.v2 face", V2_POLICY,
                   {"node-a": ["census.v1", "quiet.v1", "vital.v1"]}, {"node-a": True}, ["ground"], v2_contracts(),
                   [], v2=True)
    return out


def main() -> int:
    from . import packed

    assert packed.lint(STATUS_SCHEMA).types["Status"].tag != 0
    try:
        results = run()
    except Exception as e:  # noqa: BLE001 - a run that could not finish is a failure
        results = [("the run finished", False, f"{type(e).__name__}: {e}")]
    for name, ok, why in results:
        print(("PASS " if ok else "FAIL ") + name + (f": {why}" if why and not ok else ""))
    failed = sum(1 for _, ok, _ in results if not ok)
    print(f"{len(results) - failed} passed, {failed} failed")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())

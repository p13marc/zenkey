"""A grant generator from core.md §11.1–§11.2 (0.14) alone: a deployment
compiled into zenoh 1.10.1's ``access_control`` block, under
``default_permission: deny`` or ``allow``.

**The input is zk2py's own.** The spec states the grant shapes, but no
deployment format: the only bindings file in ``examples/zk2/`` calls its
shape "recommended, not normative" (SPEC-FINDINGS F-82). A deployment here
is a list of :class:`Principal`, each bound to one usrpwd username (§11.3:
"Principals are bound by certificate CN or username"):
- ``service`` and ``implements``: the Own grant on ``zk2/<system>/<service>``
  for the contracts it serves;
- ``consumes``: Consume, one :class:`Use` per binding (interface, providers,
  resources);
- ``calls``: Call, the same for operations;
- ``inspects``: presence alone on the services a tool inspects;
- a principal with no ``service`` is the Tool shape (0.14).

**The messages each shape compiles to** are zk2py's reading of §11.1 plus
measurement, since §11.1 names actions, not zenoh's nine messages and two
flows (SPEC-FINDINGS F-83):
- Own: on ingress ``put``, ``delete``, ``declare_queryable``, ``reply`` and
  ``liveliness_token`` on its keys; on egress ``query`` and
  ``declare_subscriber`` on them (0.14).
- Consume: on ingress ``declare_subscriber`` and ``query`` on each selector;
  on egress ``put``, ``delete`` and ``reply`` on it.
- Call: on ingress ``query`` on each operation selector; on egress
  ``reply``.
- Presence, on ``…/@zk/**`` of each provider: liveliness reads, ingress
  ``liveliness_query`` and ``declare_liveliness_subscriber``, egress
  ``liveliness_token`` (measured: a liveliness GET's tokens come back as an
  egress ``liveliness_token``, not a ``reply``); and the descriptor's GET
  and subscription (0.14), ingress ``query`` and ``declare_subscriber``,
  egress ``reply`` and ``put``.
- Contract bundles are open: every principal may ``query``,
  ``declare_queryable`` and ``reply`` on ``zk2/@zk/contract/**``, both
  flows.
- §11.2: every consumer selector that intersects a provider's keys is
  added to that provider's egress ``query`` and ``declare_subscriber``, and
  to its ingress ``reply`` (refusals are checked against the query's key).
- The admin space: no principal declares queryables under ``@/**``; under
  ``allow`` each principal's policy denies ``declare_queryable`` and
  ``reply`` there. Not a subject matching every session: in zenoh 1.10.1
  one undoes the per-user subjects' denies (measured, SPEC-FINDINGS F-87).
  A Tool may read the admin space (``query`` ingress, ``reply`` egress),
  which §11.1 does not grant (SPEC-FINDINGS F-84).

**Under ``allow``** each grant compiles into denies of its complement
(§11.2). Key expressions have no negation, so the complement is taken over
the deployment's own keys (SPEC-FINDINGS F-85): every resource of every
contract a principal serves, as a key expression, and each service's
``@zk/**``. A universe key that a grant includes is left open; one a grant
only intersects is the R2-narrowed case ("no complement by inclusion",
0.14), left open with a ``complement_partial`` warning; any other is denied.
"""

from __future__ import annotations

import json
from dataclasses import dataclass, field
from typing import Any

from .contract import Contract

INGRESS, EGRESS = "ingress", "egress"
MESSAGES = ("put", "delete", "declare_subscriber", "query", "declare_queryable", "reply",
            "liveliness_token", "declare_liveliness_subscriber", "liveliness_query")
CONTRACTS = "zk2/@zk/contract/**"
ADMIN = "@/**"


@dataclass
class Use:
    """One binding (§3.2 R1): ``interface`` from ``providers``
    (``<system>/<service>`` patterns, ``*`` allowed), for ``resources``
    named ``<kind token>/<template>`` (``state/health``, ``@op/echo``)."""

    interface: str
    providers: list[str]
    resources: list[str]


@dataclass
class Principal:
    id: str
    user: str
    service: str | None = None
    implements: list[Contract] = field(default_factory=list)
    consumes: list[Use] = field(default_factory=list)
    calls: list[Use] = field(default_factory=list)
    inspects: list[str] = field(default_factory=list)
    #: a variant for the scenarios: drop the presence a shape carries
    presence: bool = True
    #: Tool's admin-space read (SPEC-FINDINGS F-84)
    admin_read: bool = False


@dataclass
class Generated:
    block: dict[str, Any]
    warnings: list[str] = field(default_factory=list)
    #: (principal id, message, flow) -> sorted keys granted (both postures)
    grants: dict[tuple[str, str, str], list[str]] = field(default_factory=dict)


def template_key(template: str) -> str:
    """A template as a key expression: ``*`` per parameter, ``**`` for a rest."""
    return "/".join("**" if p.endswith("...}") else "*" if p.startswith("{") else p
                    for p in template.split("/"))


def own_keys(service: str, history: bool = False) -> list[str]:
    """§11.1 Own: ``zk2/<system>/<service>/**`` and each verbatim subtree,
    "spelled out because ** never crosses one"."""
    base = f"zk2/{service}"
    keys = [f"{base}/**", f"{base}/*/@stream/**", f"{base}/*/@state/**", f"{base}/*/@op/**", f"{base}/@zk/**"]
    if history:
        keys += [f"{base}/*/stream/**/@adv/**", f"{base}/*/state/**/@adv/**"]
    return keys


def use_selectors(use: Use) -> list[str]:
    return [f"zk2/{p}/{use.interface}/{r.split('/', 1)[0]}/{template_key(r.split('/', 1)[1])}"
            for p in use.providers for r in use.resources]


def presence_selectors(providers: list[str]) -> list[str]:
    return [f"zk2/{p}/@zk/**" for p in providers]


def _ke(text: str):
    import zenoh

    return zenoh.KeyExpr(text)


def _includes(a: str, b: str) -> bool:
    return _ke(a).includes(_ke(b))


def _intersects(a: str, b: str) -> bool:
    return _ke(a).intersects(_ke(b))


def resource_keys(service: str, contracts: list[Contract]) -> list[str]:
    """Every resource a service's contracts declare, as a key expression."""
    out = []
    for c in contracts:
        for r in c.canonical["resources"]:
            out.append(f"zk2/{service}/{c.interface}/{r['token']}/{template_key(r['template'])}")
    return out


def compile_grants(principals: list[Principal], *, egress_selectors: bool = True,
                   reply_selectors: bool = True) -> dict[tuple[str, str, str], set[str]]:
    """The grants of §11.1–§11.2, as (principal, message, flow) -> keys.
    ``egress_selectors``/``reply_selectors``: security.md §2's generator
    check removes the consumer selectors from providers' egress, or from
    their ingress ``reply``."""
    g: dict[tuple[str, str, str], set[str]] = {}

    def add(pid: str, messages: tuple[str, ...], flow: str, keys: list[str]) -> None:
        for m in messages:
            g.setdefault((pid, m, flow), set()).update(keys)

    readers: list[tuple[str, str]] = []  # (principal, selector) a provider may be asked on
    for p in principals:
        # Contract bundles are open.
        add(p.id, ("query", "declare_queryable", "reply"), INGRESS, [CONTRACTS])
        add(p.id, ("query", "reply"), EGRESS, [CONTRACTS])
        if p.service:
            keys = own_keys(p.service)
            add(p.id, ("put", "delete", "declare_queryable", "reply", "liveliness_token"), INGRESS, keys)
            add(p.id, ("query", "declare_subscriber"), EGRESS, keys)
        providers: list[str] = list(p.inspects)
        for use in p.consumes:
            sels = use_selectors(use)
            add(p.id, ("declare_subscriber", "query"), INGRESS, sels)
            add(p.id, ("put", "delete", "reply"), EGRESS, sels)
            readers += [(p.id, s) for s in sels]
            providers += use.providers
        for use in p.calls:
            sels = use_selectors(use)
            add(p.id, ("query",), INGRESS, sels)
            add(p.id, ("reply",), EGRESS, sels)
            readers += [(p.id, s) for s in sels]
            providers += use.providers
        if p.presence and providers:
            sels = presence_selectors(sorted(set(providers)))
            add(p.id, ("liveliness_query", "declare_liveliness_subscriber", "query", "declare_subscriber"),
                INGRESS, sels)
            add(p.id, ("liveliness_token", "reply", "put"), EGRESS, sels)
            readers += [(p.id, s) for s in sels]
        if p.admin_read:
            add(p.id, ("query",), INGRESS, [ADMIN])
            add(p.id, ("reply",), EGRESS, [ADMIN])
    # §11.2: a provider's egress, and its ingress reply, carry every reader
    # selector that intersects what it serves.
    for p in principals:
        if not p.service:
            continue
        keys = own_keys(p.service)
        mine = sorted({s for _, s in readers if any(_intersects(s, k) for k in keys)
                       and not any(_includes(k, s) for k in keys)})
        if egress_selectors:
            add(p.id, ("query", "declare_subscriber"), EGRESS, mine)
        if reply_selectors:
            add(p.id, ("reply",), INGRESS, mine)
    return g


def universe(principals: list[Principal]) -> list[str]:
    """The keys the complement is taken over under ``allow`` (SPEC-FINDINGS
    F-85): each service's resources and its ``@zk/**``, and the contract
    keys."""
    keys: list[str] = []
    for p in principals:
        if p.service:
            keys += resource_keys(p.service, p.implements) + [f"zk2/{p.service}/@zk/**"]
    return sorted(set(keys)) + [CONTRACTS]


def _rules_for(subject: str, grouped: dict[tuple[str, ...], set[tuple[str, str]]], permission: str) -> list[dict]:
    """One rule per key set, carrying every (message, flow) pair that key
    set has; a pair list is split by flow, which a rule's
    ``messages × flows`` needs."""
    rules = []
    for i, (keys, pairs) in enumerate(sorted(grouped.items(), key=lambda kv: kv[0])):
        by_flow: dict[str, list[str]] = {}
        for m, f in sorted(pairs):
            by_flow.setdefault(f, []).append(m)
        for flow, msgs in sorted(by_flow.items()):
            rules.append({"id": f"{subject}-{permission}-{i}-{flow}", "messages": sorted(set(msgs)),
                          "flows": [flow], "permission": permission, "key_exprs": list(keys)})
    return rules


def generate(principals: list[Principal], posture: str, *, egress_selectors: bool = True,
             reply_selectors: bool = True, allow_rules_under_allow: bool = False) -> Generated:
    """The ``access_control`` block for ``posture`` (``deny`` or ``allow``).
    ``allow_rules_under_allow`` compiles the allow rules but sets
    ``default_permission: allow``: security.md §2's "Allow rules alone,
    under allow, block nothing"."""
    grants = compile_grants(principals, egress_selectors=egress_selectors, reply_selectors=reply_selectors)
    out = Generated({}, grants={k: sorted(v) for k, v in grants.items()})
    subjects = [{"id": p.id, "usernames": [p.user]} for p in principals]
    rules: list[dict] = []
    policies: list[dict] = []
    if posture == "deny" or allow_rules_under_allow:
        for p in principals:
            grouped: dict[tuple[str, ...], set[tuple[str, str]]] = {}
            for (pid, m, f), keys in grants.items():
                if pid == p.id and keys:
                    grouped.setdefault(tuple(sorted(keys)), set()).add((m, f))
            r = _rules_for(p.id, grouped, "allow")
            rules += r
            policies.append({"id": f"{p.id}-policy", "rules": [x["id"] for x in r], "subjects": [p.id]})
    else:
        world = universe(principals)
        for p in principals:
            grouped = {}
            for m in MESSAGES:
                for f in (INGRESS, EGRESS):
                    granted = grants.get((p.id, m, f), set())
                    denied = []
                    for u in world:
                        if any(_includes(k, u) for k in granted):
                            continue
                        if any(_intersects(k, u) for k in granted):
                            out.warnings.append(f"complement_partial: {p.id} {m} {f} {u}")
                            continue
                        denied.append(u)
                    if denied:
                        grouped.setdefault(tuple(denied), set()).add((m, f))
            r = _rules_for(p.id, grouped, "deny")
            rules += r
            policies.append({"id": f"{p.id}-policy", "rules": [x["id"] for x in r], "subjects": [p.id]})
    if posture == "allow" and not allow_rules_under_allow:
        # §11.1 (0.12): no principal declares queryables under @/**, denied
        # "to every principal under allow". The deny goes into each
        # principal's policy: a subject matching every session (no
        # attribute, or every link protocol) undoes the per-user subjects'
        # denies in zenoh 1.10.1 (measured, SPEC-FINDINGS F-87). A session
        # matching no subject is not reached (§11.3).
        rules.append({"id": "admin-space-deny", "messages": ["declare_queryable", "reply"], "flows": [INGRESS],
                      "permission": "deny", "key_exprs": [ADMIN]})
        for pol in policies:
            pol["rules"].append("admin-space-deny")
    out.block = {
        "enabled": True,
        "default_permission": "allow" if posture == "allow" or allow_rules_under_allow else "deny",
        "rules": rules,
        "subjects": subjects,
        "policies": policies,
    }
    return out


def to_json5(g: Generated) -> str:
    return json.dumps(g.block, indent=1, sort_keys=True)

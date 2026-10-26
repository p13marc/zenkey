"""A grant generator from core.md §11.1–§11.2 (0.15): a deployment compiled
into zenoh 1.10.1's ``access_control`` block, under
``default_permission: deny`` or ``allow``.

**The input is zk2py's own** (§11.1 "The input", 0.15: "The format is the
generator's own"). A deployment is a list of :class:`Principal`, each bound
to one usrpwd username (§11.3: "Principals are bound by certificate CN or
username"). The input has no CN, so core 0.33's MUST, "A generator MUST
refuse an input that binds one principal by both", holds by construction:
no input can bind one principal by both.
- ``service`` and ``implements``: the Own grant on ``zk2/<system>/<service>``
  for the contracts it serves;
- ``consumes``: Consume, one :class:`Use` per binding (interface, providers,
  resources);
- ``calls``: Call, the same for operations;
- ``inspects``: presence alone on the services a tool inspects;
- ``admin_read``: whether a tool reads the admin space;
- a principal with no ``service`` is the Tool shape (0.14).

Of §11.1's input, zk2py takes neither history nor archives nor a
namespace: its deployments use none.

**Messages and flows** are §11.2's table (0.15), which zk2py had measured
the same way in 0.14:

=========  ==============================================  ====================================
Grant      Ingress (from the holder)                       Egress (toward the holder)
=========  ==============================================  ====================================
Own        put, delete, declare_queryable, reply,          query, declare_subscriber
           liveliness_token
Fan-in     reply                                           query, declare_subscriber
Consume    declare_subscriber, query                       put, delete, reply
Presence   declare_liveliness_subscriber,                  liveliness_token, reply, put
           liveliness_query, query, declare_subscriber
Call       query                                           reply
Contracts  declare_queryable, reply, query                 query, reply
Admin      query                                           reply
=========  ==============================================  ====================================

- Fan-in (§11.2): every reader selector that intersects a provider's keys
  is added to that provider's egress, and to its ingress ``reply``, since
  a refusal is checked against the query's key.
- The admin read (§11.1 Tool, 0.15, 0.50): the tools' three reads,
  ``@/*/router``, ``@/*/router/status/plugins/storage_manager/storages/**``
  and ``@/*/*``, never namespaced. 0.15's ``@/*/router/**`` admitted a GET of
  every declaration record of a router's, which zenoh 1.10.1 answers under
  its routing tables' read lock (§4.2, Appendix B).
- **No allowed ingress query reaches a node's declaration records**
  (§11.1, 0.50; its posture and flow, 0.51, F-256): "Under deny, a
  generator MUST NOT compile an allow rule for an ingress query on a key
  expression that intersects one of those keys of any node".
  :func:`generate` refuses a block that would (:class:`HazardousGrant`),
  and :func:`block_hazards` finds one in any block, zk2py's or another
  generator's.
- No principal declares queryables under ``@/**``. Under ``allow`` the deny
  is a rule in each principal's own policy, never a catch-all subject's
  (§11.2, 0.15: such a subject "makes the per-user subjects lose their
  denies").

**Under ``allow``** each grant compiles into denies of its complement,
taken over "the deployment's own keys: each resource the enrolled
services' contracts declare, each service's …/@zk/**, and the contract
keys" (§11.2, 0.15). A key in that set that a grant includes is left open.
One a grant only intersects is the R2-narrowed case ("no complement by
inclusion", 0.14), left open with a ``complement_partial`` warning. Any
other is denied. Keys no contract declares stay open, the admin space
included.
"""

from __future__ import annotations

import json
from dataclasses import dataclass, field
from typing import Any

from .admin_records import ADMIN_RECORDS
from .contract import Contract

INGRESS, EGRESS = "ingress", "egress"
MESSAGES = ("put", "delete", "declare_subscriber", "query", "declare_queryable", "reply",
            "liveliness_token", "declare_liveliness_subscriber", "liveliness_query")
CONTRACTS = "zk2/@zk/contract/**"
ADMIN = "@/**"
#: §11.1 Tool (0.15, 0.50): the admin read, "query on @/*/router,
#: @/*/router/status/plugins/storage_manager/storages/** and @/*/*, and their
#: reply, never namespaced": S4's two reads (§4.2) and every node's root
#: document, which a mesh view reads.
ADMIN_READ = ["@/*/router", "@/*/router/status/plugins/storage_manager/storages/**", "@/*/*"]
#: 0.15's admin read, "@/*/router and @/*/router/**", which 0.50 narrowed: it
#: admitted a GET of every record of a router's, and refused @/*/*, which
#: neither key includes. Compiled only for security.md §4 step 4.
ADMIN_READ_015 = ["@/*/router", "@/*/router/**"]


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
    #: the Tool's admin read (§11.1, 0.15, 0.50)
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


def resource_key(token: str, template: str) -> str:
    """A resource's keys as a key expression: its template, and for an
    event one chunk more, the occurrence's ULID (core §2.6:
    ``<member>/<ulid>``)."""
    key = f"{token}/{template_key(template)}"
    return f"{key}/*" if token == "events" else key


def use_selectors(use: Use) -> list[str]:
    return [f"zk2/{p}/{use.interface}/{resource_key(*r.split('/', 1))}"
            for p in use.providers for r in use.resources]


def presence_selectors(providers: list[str]) -> list[str]:
    return [f"zk2/{p}/@zk/**" for p in providers]


def _ke(text: str):
    import zenoh

    return zenoh.KeyExpr(text)


def _includes(a: str, b: str) -> bool:
    """Whether key expression ``a`` includes ``b``, by zenoh's own
    ``KeyExpr.includes``."""
    return _ke(a).includes(_ke(b))


def _intersects(a: str, b: str) -> bool:
    """Whether key expressions ``a`` and ``b`` intersect, by zenoh's own
    ``KeyExpr.intersects``."""
    return _ke(a).intersects(_ke(b))


def resource_keys(service: str, contracts: list[Contract]) -> list[str]:
    """Every resource a service's contracts declare, as a key expression."""
    out = []
    for c in contracts:
        for r in c.canonical["resources"]:
            out.append(f"zk2/{service}/{c.interface}/{resource_key(r['token'], r['template'])}")
    return out


def compile_grants(principals: list[Principal], *, egress_selectors: bool = True,
                   reply_selectors: bool = True, presence_egress: bool = True,
                   admin_read: list[str] | None = None) -> dict[tuple[str, str, str], set[str]]:
    """The grants of §11.1–§11.2, as (principal, message, flow) -> keys.
    The shapes compose per principal (0.27): a principal holds the union of
    the grants of what it runs, Own for its service and a tool's beside it.
    ``egress_selectors``/``reply_selectors``: security.md §2's generator
    check removes the consumer selectors from providers' egress, or from
    their ingress ``reply``; ``presence_egress`` (0.27) removes the
    presence selectors from providers' egress. ``admin_read`` is the Tool's
    admin read, :data:`ADMIN_READ` unless security.md §4 step 4 asks 0.15's
    (:data:`ADMIN_READ_015`)."""
    admin_keys = list(ADMIN_READ if admin_read is None else admin_read)
    g: dict[tuple[str, str, str], set[str]] = {}

    def add(pid: str, messages: tuple[str, ...], flow: str, keys: list[str]) -> None:
        for m in messages:
            g.setdefault((pid, m, flow), set()).update(keys)

    readers: list[tuple[str, str]] = []  # (principal, selector) a provider may be asked on
    presence_readers: list[tuple[str, str]] = []  # (principal, presence selector)
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
            # Presence on the services a principal binds, calls or inspects:
            # inspection is presence alone (0.27).
            sels = presence_selectors(sorted(set(providers)))
            add(p.id, ("liveliness_query", "declare_liveliness_subscriber", "query", "declare_subscriber"),
                INGRESS, sels)
            add(p.id, ("liveliness_token", "reply", "put"), EGRESS, sels)
            presence_readers += [(p.id, s) for s in sels]
        if p.admin_read:
            add(p.id, ("query",), INGRESS, admin_keys)
            add(p.id, ("reply",), EGRESS, admin_keys)
    # §11.2: a provider's egress, and its ingress reply, carry every reader
    # selector that intersects what it serves. A presence selector (0.27)
    # joins its egress alone, as `fan-in-presence`: "No refusal answers a
    # descriptor read", and a reply grant on it would let the provider
    # answer under another provider's descriptor key.
    for p in principals:
        if not p.service:
            continue
        keys = own_keys(p.service)

        def foreign(selectors: list[tuple[str, str]]) -> list[str]:
            return sorted({s for _, s in selectors if any(_intersects(s, k) for k in keys)
                           and not any(_includes(k, s) for k in keys)})

        mine, presence = foreign(readers), foreign(presence_readers)
        if egress_selectors:
            add(p.id, ("query", "declare_subscriber"), EGRESS, mine)
        if presence_egress:
            add(p.id, ("query", "declare_subscriber"), EGRESS, presence)
        if reply_selectors:
            add(p.id, ("reply",), INGRESS, mine)
    return g


def universe(principals: list[Principal]) -> list[str]:
    """The keys the complement is taken over under ``allow`` (§11.2, 0.15):
    each service's resources and its ``@zk/**``, and the contract keys."""
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
             reply_selectors: bool = True, allow_rules_under_allow: bool = False,
             presence_egress: bool = True, admin_read: list[str] | None = None,
             hazard_ok: bool = False) -> Generated:
    """The ``access_control`` block for ``posture`` (``deny`` or ``allow``).
    ``allow_rules_under_allow`` compiles the allow rules but sets
    ``default_permission: allow``: security.md §2's "Allow rules alone,
    under allow, block nothing".

    §11.1 (0.50, 0.51): "Under deny, a generator MUST NOT compile an allow
    rule for an ingress query on a key expression that intersects one of
    those keys of any node". A block whose allow rules would is refused
    (:class:`HazardousGrant`), unless ``hazard_ok``: security.md §4 step 4
    compiles 0.15's admin read (``admin_read``) on purpose, beside no
    writer. Under ``allow`` no allow rule is evaluated, and the generator
    compiles no admin read, "so a router there is not protected" (0.51,
    F-256); :func:`block_hazards` says so of such a block.

    Under ``allow``, each principal's complement is taken per message and
    flow over every key of the deployment (:func:`universe`) its grants do
    not name for that message and flow, its own services' keys included:
    core §11.2's stricter branch (0.31, "A generator that denies them too,
    per message and flow, is stricter, and conforms")."""
    grants = compile_grants(principals, egress_selectors=egress_selectors, reply_selectors=reply_selectors,
                            presence_egress=presence_egress, admin_read=admin_read)
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
        # "to every principal under allow"; §11.2 (0.15): "a rule in each
        # principal's policy, never one catch-all subject's". A session
        # matching no subject is not reached (§11.3; security.md §3 step 3).
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
    allowed = [h for h in block_hazards(out.block) if h.kind == "allowed_query"]
    if allowed and not hazard_ok:
        raise HazardousGrant("; ".join(h.why() for h in allowed))
    return out


class HazardousGrant(ValueError):
    """§11.1 (0.50, 0.51): a ``deny`` block whose allow rules admit an
    ingress query on a key expression that intersects a node's declaration
    records, its ``linkstate/*`` or its ``route/successor/**``."""


@dataclass(frozen=True)
class AdminHazard:
    """One way a block admits a GET of a node's declaration records
    (§11.1, 0.50, 0.51; §4.2; Appendix B):
    - ``allowed_query``: under ``deny``, an allow rule a policy lists names
      ``query`` on the ingress flow (named, or zenoh's default of both) and
      a key expression that intersects them (``reaches``, by zenoh 1.10.1's
      own intersection). An egress ``query`` alone is not one: "the GET
      that freezes a node is the asker's, decided at the asker's face on
      ingress" (0.51, F-256);
    - ``allow_posture``: under ``allow`` no allow rule is evaluated, and the
      policy holds no deny of ``query`` that includes ``@/**``, so a GET of
      ``@/**`` from its subjects reaches the records ("a router there is
      not protected", 0.51, F-256);
    - ``disabled``: access control is not enabled (``enabled`` false or
      absent), so every query is admitted."""

    kind: str
    policy: str | None = None
    rule: str | None = None
    flow: str | None = None
    key: str | None = None
    reaches: tuple[str, ...] = ()

    def why(self) -> str:
        if self.kind == "allowed_query":
            return (f"{self.policy}: rule {self.rule} allows query on {self.key} ({self.flow}), which intersects "
                    f"{list(self.reaches)} (§11.1, 0.50, 0.51)")
        if self.kind == "allow_posture":
            return (f"{self.policy}: under allow, no deny of query includes {ADMIN}, so its subjects' GETs reach a "
                    f"node's declaration records (§11.1, 0.51: a router there is not protected)")
        return "access control is not enabled: every query is admitted"


def block_hazards(block: dict[str, Any]) -> list[AdminHazard]:
    """§11.1 (0.50, 0.51) read against a router's ``access_control`` block,
    zk2py's or another generator's: every way it admits a GET of a node's
    declaration records (:class:`AdminHazard`). Empty for a ``deny`` block
    that meets the MUST NOT. Rules no policy lists are inert, and skipped,
    and so is a rule's egress flow (0.51).
    Inclusion and intersection are zenoh 1.10.1's own (:func:`_includes`,
    :func:`_intersects`), never a string test."""
    if not block.get("enabled"):
        return [AdminHazard("disabled")]
    rules = {r["id"]: r for r in block.get("rules") or []}
    out: list[AdminHazard] = []
    for pol in block.get("policies") or []:
        listed = [rules[r] for r in pol.get("rules") or [] if r in rules]
        if block.get("default_permission") == "allow":
            kept_off = any(r.get("permission") == "deny" and "query" in (r.get("messages") or [])
                           and INGRESS in (r.get("flows") or [INGRESS, EGRESS])
                           and any(_includes(k, ADMIN) for k in r.get("key_exprs") or []) for r in listed)
            if not kept_off:
                out.append(AdminHazard("allow_posture", pol.get("id")))
            continue
        for r in listed:
            if r.get("permission") != "allow" or "query" not in (r.get("messages") or []):
                continue
            # 0.51: "an ingress query"; zenoh reads a rule with no flows as both
            if INGRESS not in (r.get("flows") or [INGRESS, EGRESS]):
                continue
            for k in r.get("key_exprs") or []:
                reaches = tuple(p for p in ADMIN_RECORDS if _intersects(k, p))
                if reaches:
                    out.append(AdminHazard("allowed_query", pol.get("id"), r["id"], INGRESS, k, reaches))
    return out


def read_block_json5(text: str) -> dict[str, Any]:
    """A block as another generator writes it, JSON5 at a router
    configuration's top level (``access_control: {…}``, comments included),
    read by zenoh's own configuration loader. Fields zenoh fills in are
    ``null``, which it reads back as absent."""
    import zenoh

    body = text if text.lstrip().startswith("{") else "{" + text + "}"
    return json.loads(zenoh.Config.from_json5(body).get_json("access_control"))


def may_call(g: Generated, principal: str, key: str) -> bool:
    """Whether the generated grants let ``principal`` call ``key``: its
    ingress ``query`` includes the key, and so does its egress ``reply``
    (a value reply is checked against its own key, 0.14). §5.1 (0.16)
    judges O3 from outside only under such grants. A tool cannot observe
    its grants (§11.3), so this reads them from the deployment."""
    asked = g.grants.get((principal, "query", INGRESS), [])
    answered = g.grants.get((principal, "reply", EGRESS), [])
    return any(_includes(k, key) for k in asked) and any(_includes(k, key) for k in answered)


def to_json5(g: Generated) -> str:
    return json.dumps(g.block, indent=1, sort_keys=True)

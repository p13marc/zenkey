"""``link.v1``, text 0.1 (``spec/profiles/link/v1.md``): what crosses a
constrained face.

From the text alone:
- **the face policy** (§2.1): :func:`parse_policy` reads the TOML file and
  reports the linter's findings, each ``{code, severity, face, at}``;
- **``link.exposure``** (§2.2): :func:`exposure`, and whether a resource's
  pushes cross a face (§2.7, §2.8);
- **the plan** (§2.3, §2.5 to §2.12): :func:`plan` derives each face's
  denies, with §2.7's carve-out (:func:`carve`), its downsampling rules,
  its attachment, its window floors and its liveness evidence, or the
  findings that refuse it;
- **the checks**: a deployment's tombstone windows (§2.11,
  :func:`check_windows`) and a router's block (§2.15, :func:`check_block`);
  :func:`lint` is ``lints.json``'s whole reading;
- **a generator** for zenoh-python routers (:func:`compile_face`): the
  plan's denies in the far principal's policy (§2.4), its downsampling
  block, and the near router's ``gateway`` for a south region (§2.5).

Where the text left a choice, zk2py's is named in ``SPEC-FINDINGS.md``
("At link.v1 0.1").
"""

from __future__ import annotations

import tomllib
from dataclasses import dataclass, field
from typing import Any

from . import freshness, templates
from .lexical import is_plain_chunk

#: §0: "The nine messages, in this order".
MESSAGES = ["put", "delete", "declare_subscriber", "query", "declare_queryable", "reply", "liveliness_token",
            "declare_liveliness_subscriber", "liveliness_query"]
PUSH = ["put", "delete", "declare_subscriber"]
FLOWS = ["egress", "ingress"]
#: §2.7: the exposure denies "per kind token K, in the order".
KIND_TOKENS = ["stream", "@stream", "state", "@state", "events"]
#: §2.1: zenoh 1.10.1's link protocol names.
LINK_PROTOCOLS = {"tcp", "udp", "tls", "quic", "serial", "unixpipe", "unixsock-stream", "vsock", "ws"}
U32_MAX = 2**32 - 1
DEFAULT_LEASE_MS = 10_000
#: core S3: a window not given is 60 s (§2.11).
DEFAULT_WINDOW_S = 60
FACE_KEYS = ("far", "attach", "region", "select", "class", "expose", "interval_s", "max_outage_s", "batch_bytes",
             "link_bps", "lease_ms")
_KIND_OF_TOKEN = {"stream": "stream", "@stream": "stream", "state": "state", "@state": "state", "events": "event",
                  "@op": "operation"}


def finding(code: str, face: str | None, at: str, severity: str = "error") -> dict[str, Any]:
    return {"code": code, "severity": severity, "face": face, "at": at}


def _bytes(s: str) -> bytes:
    return s.encode("utf-8", "surrogatepass")


def sort_findings(findings: list[dict[str, Any]]) -> list[dict[str, Any]]:
    """§2.1: "Findings are a set, sorted by face (none first), then at, then
    code, each by its bytes"."""
    unique = {(f["code"], f["severity"], f["face"], f["at"]): f for f in findings}
    return sorted(unique.values(), key=lambda f: (f["face"] is not None, _bytes(f["face"] or ""), _bytes(f["at"]),
                                                  _bytes(f["code"])))


def _int_in(v: Any, lo: int, hi: int) -> bool:
    return type(v) is int and lo <= v <= hi


# -- §2.1 the face policy ----------------------------------------------------------------

@dataclass
class Face:
    name: str
    far: str
    attach: str
    region: str | None
    link_protocols: list[str]
    expose: str
    interval_s: int | None
    max_outage_s: int
    batch_bytes: int
    link_bps: int
    lease_ms: int


@dataclass
class Policy:
    system: str
    services: list[str]
    faces: list[Face]
    findings: list[dict[str, Any]] = field(default_factory=list)

    @property
    def errors(self) -> list[dict[str, Any]]:
        return [f for f in self.findings if f["severity"] == "error"]


def _toml(text: str) -> dict[str, Any] | None:
    from .contract import _beyond_i64, _require_toml_1_0_reader

    _require_toml_1_0_reader()
    try:
        doc = tomllib.loads(text)
    except tomllib.TOMLDecodeError:
        return None
    # §2.1 "toml": "an integer beyond 64 bits, as core §9.1 refuses a
    # contract (E000)".
    return None if _beyond_i64(doc) else doc


def parse_policy(text: str) -> tuple[Policy | None, list[dict[str, Any]]]:
    """Read a face policy (§2.1): the policy when it has no error, and every
    finding, sorted. "Each key is checked alone": a key whose value is wrong
    is reported once, and checks that need it are skipped."""
    doc = _toml(text)
    if doc is None:
        return None, [finding("toml", None, "")]
    out: list[dict[str, Any]] = []
    for k in doc:
        if k not in ("system", "services", "face"):
            out.append(finding("unknown_field", None, k))
    system = doc.get("system")
    if "system" not in doc:
        out.append(finding("missing_field", None, "system"))
    elif type(system) is not str or not is_plain_chunk(system):
        out.append(finding("bad_value", None, "system"))
    services = doc.get("services")
    if "services" not in doc:
        out.append(finding("missing_field", None, "services"))
    elif type(services) is not list or any(type(s) is not str or not is_plain_chunk(s) for s in services) \
            or len(set(services)) != len(services):
        out.append(finding("bad_value", None, "services"))
    faces: list[Face] = []
    good_protocols: dict[str, list[str]] = {}
    if "face" not in doc:
        out.append(finding("missing_field", None, "face"))
    elif type(doc["face"]) is not dict or not doc["face"]:
        out.append(finding("bad_value", None, "face"))
    else:
        for name, body in doc["face"].items():
            if not is_plain_chunk(name) or type(body) is not dict:
                out.append(finding("bad_value", name, ""))
                continue
            f, protos = _face(name, body, out)
            if protos is not None:
                good_protocols[name] = protos
            if f is not None:
                faces.append(f)
    # §2.14: "A policy MUST NOT give two faces a common link protocol."
    names = sorted(good_protocols)
    for i, a in enumerate(names):
        for b in names[i + 1:]:
            if set(good_protocols[a]) & set(good_protocols[b]):
                out += [finding("shared_protocol", a, "select.link_protocols"),
                        finding("shared_protocol", b, "select.link_protocols")]
    out = sort_findings(out)
    if any(f["severity"] == "error" for f in out):
        return None, out
    return Policy(system, list(services), sorted(faces, key=lambda f: _bytes(f.name)), out), out


def _face(name: str, body: dict[str, Any], out: list[dict[str, Any]]) -> tuple[Face | None, list[str] | None]:
    errs_before = len(out)

    def bad(at: str, code: str = "bad_value") -> None:
        out.append(finding(code, name, at))

    for k in body:
        if k not in FACE_KEYS:
            bad(k, "unknown_field")
    far = body.get("far")
    if "far" not in body:
        bad("far", "missing_field")
    elif type(far) is not str or not far:
        bad("far")
    attach = body.get("attach")
    if "attach" not in body:
        bad("attach", "missing_field")
    elif attach == "router":
        bad("attach", "router_attach")
    elif attach not in ("client", "south-region"):
        bad("attach")
    region = body.get("region")
    if "region" in body and (type(region) is not str or not 1 <= len(_bytes(region)) <= 32):
        bad("region")
    elif attach == "south-region" and "region" not in body:
        bad("region", "missing_field")
    elif attach == "client" and "region" in body:
        bad("region")
    protocols: list[str] | None = None
    if "select" not in body:
        bad("select", "missing_field")
    elif type(body["select"]) is not dict:
        bad("select")
    else:
        sel = body["select"]
        for k in sel:
            if k == "interfaces":
                bad("select.interfaces", "select_interfaces")
            elif k != "link_protocols":
                bad(f"select.{k}", "unknown_field")
        lp = sel.get("link_protocols")
        if "link_protocols" not in sel:
            bad("select.link_protocols", "missing_field")
        elif type(lp) is not list or not lp or any(type(p) is not str for p in lp) or len(set(lp)) != len(lp):
            bad("select.link_protocols")
        elif any(p not in LINK_PROTOCOLS for p in lp):
            bad("select.link_protocols", "unknown_protocol")
        else:
            protocols = list(lp)
    if "class" not in body:
        bad("class", "missing_field")
    elif body["class"] != "constrained":
        bad("class")
    expose = body.get("expose")
    if "expose" not in body:
        bad("expose", "missing_field")
    elif expose not in ("link", "none"):
        bad("expose")
    interval = body.get("interval_s")
    if "interval_s" in body and not _int_in(interval, 1, U32_MAX):
        bad("interval_s")
    elif expose == "link" and "interval_s" not in body:
        bad("interval_s", "missing_field")
    elif expose == "none" and "interval_s" in body:
        bad("interval_s")
    ints = {"max_outage_s": U32_MAX, "batch_bytes": 65535, "link_bps": U32_MAX}
    for k, hi in ints.items():
        if k not in body:
            bad(k, "missing_field")
        elif not _int_in(body[k], 1, hi):
            bad(k)
    lease = body.get("lease_ms", DEFAULT_LEASE_MS)
    if "lease_ms" in body and not _int_in(lease, 1, U32_MAX):
        bad("lease_ms")
    # §2.10: "A linter warns when batch_bytes × 24,000 > lease_ms ×
    # link_bps … computed exactly, in integers."
    b, bps = body.get("batch_bytes"), body.get("link_bps")
    if _int_in(b, 1, 65535) and _int_in(bps, 1, U32_MAX) and _int_in(lease, 1, U32_MAX) and b * 24_000 > lease * bps:
        out.append(finding("batch_over_lease", name, "batch_bytes", "warning"))
    if any(f["severity"] == "error" for f in out[errs_before:]):
        return None, protocols
    return Face(name, far, attach, region, protocols or [], expose, interval if expose == "link" else None,
                body["max_outage_s"], b, bps, lease), protocols


# -- §2.2 exposure ------------------------------------------------------------------------

def exposure_of(token: str, annotations: dict[str, Any]) -> str:
    """§2.2: ``host`` (absent, or "host"), ``link``, ``invalid`` (any other
    value, read as host), or ``ignored`` (an operation)."""
    if token == "@op":
        return "ignored"
    if "link.exposure" not in annotations:
        return "host"
    v = annotations["link.exposure"]
    if type(v) is str and v in ("host", "link"):
        return v
    return "invalid"


def has_parameter(template: str) -> bool:
    return any(s.kind != templates.LITERAL for s in templates.parse_template(template).segments)


def exposure(token: str, template: str, annotations: dict[str, Any], expose: str) -> dict[str, str]:
    """exposure.json's reading: the exposure, whether the pushes cross, and
    "the first of these that holds, in this order: requests_only; invalid;
    host; face_closed; templated; exposed"."""
    e = exposure_of(token, annotations)
    if e == "ignored":
        return {"exposure": e, "pushes": "none", "reason": "requests_only"}
    if e in ("invalid", "host"):
        return {"exposure": e, "pushes": "denied", "reason": e}
    if expose != "link":
        return {"exposure": e, "pushes": "denied", "reason": "face_closed"}
    if token in ("stream", "@stream") and has_parameter(template):
        return {"exposure": e, "pushes": "denied", "reason": "templated"}
    return {"exposure": e, "pushes": "downsampled" if token in ("stream", "@stream") else "whole", "reason": "exposed"}


def crossing(token: str, template: str, annotations: dict[str, Any], expose: str) -> bool:
    """§2.7: a stream, a state or an event, exposed, on a face that exposes,
    and not a stream whose template has a parameter."""
    return exposure(token, template, annotations, expose)["reason"] == "exposed"


# -- §2.7 the carve-out --------------------------------------------------------------------

def _pattern(seg: templates.Segment) -> str:
    """§0: "a literal segment as itself, {p} as *, and {p...} as */**"."""
    return seg.text if seg.kind == templates.LITERAL else "*" if seg.kind == templates.PARAM else "*/**"


def _key(root: str, segs: tuple[templates.Segment, ...] | list[templates.Segment], tail: str = "") -> str:
    """``R/P`` with ``tail`` (``/**`` or ``/*/**``) after it; "A ** that a
    pattern ends with is written once before a /** that follows it"."""
    body = "/".join(_pattern(s) for s in segs)
    key = f"{root}/{body}" if body else root
    if tail == "/**" and key.endswith("**"):
        return key
    return key + tail


@dataclass
class _T:
    name: str
    tpl: templates.Template
    crosses: bool


def carve(root: str, items: list[tuple[str, str, bool]]) -> tuple[list[str], set[str]]:
    """§2.7 "The carve-out" of one interface's templates of one kind token:
    ``items`` are (resource name, template, crossing); returns the denied
    key expressions, sorted by their bytes, and the overlapped resources'
    names."""
    ts = [_T(n, templates.parse_template(t), c) for n, t, c in items]
    denied: list[str] = []
    overlapped: set[str] = set()

    def step(prefix: list[templates.Segment], group: list[_T]) -> None:
        n = len(prefix)
        if not any(t.crosses for t in group):
            denied.append(_key(root, prefix, "/**"))
            return
        if all(t.crosses for t in group):
            return
        if any(len(t.tpl.segments) == n and not t.crosses for t in group):
            denied.append(_key(root, prefix))
        longer = [t for t in group if len(t.tpl.segments) > n]
        if not any(t.crosses for t in longer):
            denied.append(_key(root, prefix, "/*/**"))
            return
        kinds = {t.tpl.segments[n].kind for t in longer}
        if len(kinds) > 1:
            own = [t for t in longer if not t.crosses]
            for t in own:
                denied.append(_key(root, t.tpl.segments))
            # "A crossing template is overlapped when a template denied by
            # its own pattern in step 3.3 overlaps it … and it takes
            # precedence over that template (core §2.2)."
            for t in longer:
                if t.crosses and any(templates.overlap(t.tpl, d.tpl) and t.tpl.rank() > d.tpl.rank() for d in own):
                    overlapped.add(t.name)
            return
        groups: dict[tuple[str, str], list[_T]] = {}
        for t in longer:
            s = t.tpl.segments[n]
            groups.setdefault((s.kind, s.text if s.kind == templates.LITERAL else ""), []).append(t)
        for (kind, text), members in groups.items():
            step(prefix + [templates.Segment(kind, text or "_")], members)

    step([], ts)
    return sorted(set(denied), key=_bytes), overlapped


# -- §2.3 the plan ------------------------------------------------------------------------

@dataclass
class Resource:
    iface: str
    token: str
    template: str
    annotations: dict[str, Any]

    @property
    def name(self) -> str:
        return f"{self.iface}/{self.token}/{self.template}"

    def horizon(self) -> freshness.Horizon:
        return freshness.horizon(_KIND_OF_TOKEN[self.token], self.annotations)


def resources_of(contract: Any) -> list[dict[str, Any]]:
    """A loaded contract's resolved resources, as plan.json writes them."""
    return [{"template": r["template"], "token": r["token"], "annotations": dict(r["annotations"])}
            for r in contract.canonical["resources"]]


def plan(policy_text: str, implements: dict[str, list[str]],
         interfaces: dict[str, list[dict[str, Any]]]) -> dict[str, Any]:
    """§2.3: ``{faces: [...]}``, or ``{refused: [findings]}``."""
    policy, findings = parse_policy(policy_text)
    if policy is None:
        return {"refused": findings}
    errors = []
    for s in policy.services:
        if not implements.get(s):
            errors.append(finding("service_unknown", None, s))
            continue
        for i in implements[s]:
            if i not in interfaces:
                errors.append(finding("contract_unknown", None, i))
    if errors:
        return {"refused": sort_findings(findings + errors)}
    return {"faces": [plan_face(policy, f, implements, interfaces) for f in policy.faces]}


def plan_face(policy: Policy, face: Face, implements: dict[str, list[str]],
              interfaces: dict[str, list[dict[str, Any]]]) -> dict[str, Any]:
    listed = sorted({i for s in policy.services for i in implements[s]}, key=_bytes)
    res = {i: [Resource(i, r["token"], r["template"], r.get("annotations") or {}) for r in interfaces[i]]
           for i in listed}
    expose = face.expose
    crosses = {(r.iface, r.token, r.template): crossing(r.token, r.template, r.annotations, expose)
               for i in listed for r in res[i]}
    warnings: list[dict[str, Any]] = []
    overlapped: set[str] = set()
    deny = [{"id": i, "flows": list(FLOWS), "messages": list(MESSAGES), "key_exprs": [k]}
            for i, k in (("zk", "zk2/**/@zk/**"), ("adv", "zk2/**/@adv/**"), ("admin", "@/**"))]
    for k in KIND_TOKENS:
        any_cross = any(crosses[(r.iface, r.token, r.template)] for i in listed for r in res[i] if r.token == k)
        if expose != "link" or not any_cross:
            deny.append({"id": k, "flows": list(FLOWS), "messages": list(PUSH), "key_exprs": [f"zk2/*/*/*/{k}/**"]})
            continue
        for i in listed:
            mine = [r for r in res[i] if r.token == k]
            root = f"zk2/*/*/{i}/{k}"
            if not any(crosses[(r.iface, r.token, r.template)] for r in mine):
                keys = [f"{root}/**"]
            else:
                keys, over = carve(root, [(r.name, r.template, crosses[(r.iface, r.token, r.template)]) for r in mine])
                overlapped |= over
            if keys:
                deny.append({"id": f"{k}:{i}", "flows": list(FLOWS), "messages": list(PUSH), "key_exprs": keys})
    for name in sorted(overlapped, key=_bytes):
        warnings.append(finding("carve_out_overlap", face.name, name, "warning"))

    def crossing_res(r: Resource) -> bool:
        return crosses[(r.iface, r.token, r.template)] and r.name not in overlapped

    rules = []
    if expose == "link":
        for i in listed:
            for r in res[i]:
                if r.token in ("stream", "@stream") and exposure_of(r.token, r.annotations) == "link" \
                        and has_parameter(r.template):
                    warnings.append(finding("templated_exposure", face.name, r.name, "warning"))
                if r.token in ("stream", "@stream") and crossing_res(r):
                    h = r.horizon()
                    if h.horizon == "within" and h.ttl_s < 2 * (face.interval_s or 0):
                        warnings.append(finding("interval_over_horizon", face.name, r.name, "warning"))
        for s in policy.services:
            for i in implements[s]:
                for r in res[i]:
                    if r.token in ("stream", "@stream") and crossing_res(r):
                        rules.append({"key_expr": f"zk2/{policy.system}/{s}/{i}/{r.token}/{r.template}",
                                      "freq": 1 / face.interval_s})
    rules.sort(key=lambda x: _bytes(x["key_expr"]))
    downsampling = None if not rules else {"id": face.name, "link_protocols": list(face.link_protocols),
                                           "flows": ["egress"], "messages": ["put"], "rules": rules}
    windows, liveness = [], []
    for s in policy.services:
        addr = f"{policy.system}/{s}"
        mine = [r for i in implements[s] for r in res[i]]
        if any(r.token in ("state", "@state") for r in mine):
            windows.append({"service": addr, "min_s": face.max_outage_s})
        evidence = sorted({r.name for r in mine if r.token in ("stream", "@stream", "state", "@state")
                           and crossing_res(r) and r.horizon().horizon == "within"}, key=_bytes)
        liveness.append({"service": addr, "evidence": evidence})
    if face.attach == "client":
        attach: dict[str, Any] = {"shape": "client"}
    else:
        attach = {"shape": "south-region", "region": face.region,
                  "gateway": {"south": [{"filters": [{"modes": ["peer", "client"]}]},
                                        {"filters": [{"region_names": [face.region]}]}]}}
    return {"name": face.name, "far": face.far, "attach": attach, "deny": deny, "downsampling": downsampling,
            "windows": sorted(windows, key=lambda w: _bytes(w["service"])),
            "liveness": sorted(liveness, key=lambda w: _bytes(w["service"])),
            "findings": sort_findings(warnings)}


# -- §2.11, §2.15 the checks -----------------------------------------------------------------

def check_windows(plan_doc: dict[str, Any], windows: dict[str, int]) -> list[dict[str, Any]]:
    """§2.11: each listed owner of state's window, configured or 60 s,
    against each face's floor (``window_under_outage``)."""
    out = []
    for f in plan_doc.get("faces", []):
        for w in f["windows"]:
            if windows.get(w["service"], DEFAULT_WINDOW_S) < w["min_s"]:
                out.append(finding("window_under_outage", f["name"], w["service"]))
    return out


def _keyexpr(text: Any):
    import zenoh

    if type(text) is not str:
        return None
    try:
        return zenoh.KeyExpr(text)
    except Exception:  # noqa: BLE001 - "A key expression that is not one"
        return None


def _concrete(text: str) -> bool:
    return not any(c in ("*", "**") or "$*" in c for c in text.split("/"))


def check_block(block: dict[str, Any]) -> list[dict[str, Any]]:
    """§2.15: a router's block for one face against the face it guards:
    ``zk_not_denied``, ``wildcard_downsampling`` and
    ``intersecting_downsampling``."""
    face, ns = block["face"], block.get("namespace") or ""
    target = _keyexpr(f"{ns}/zk2/**/@zk/**" if ns else "zk2/**/@zk/**")
    out = []

    def denied(m: str, fl: str) -> bool:
        return any(m in d.get("messages", []) and fl in d.get("flows", [])
                   and any((ke := _keyexpr(k)) is not None and ke.includes(target) for k in d.get("key_exprs", []))
                   for d in block.get("deny", []))

    # "for one of the 18 pairs of the nine messages and both flows, no deny
    # of the block … includes the namespace's zk2/**/@zk/**".
    if target is None or not all(denied(m, fl) for m in MESSAGES for fl in FLOWS):
        out.append(finding("zk_not_denied", face, "@zk"))
    rules = block.get("downsampling", [])
    kes = [_keyexpr(r.get("key_expr")) for r in rules]
    for j, r in enumerate(rules):
        if kes[j] is None or not _concrete(r["key_expr"]):
            out.append(finding("wildcard_downsampling", face, str(r.get("key_expr"))))
        for i in range(j):
            if kes[i] is not None and kes[j] is not None and kes[i].intersects(kes[j]):
                out.append(finding("intersecting_downsampling", face, r["key_expr"]))
                break
    return out


def lint(policy_text: str, *, implements: dict[str, list[str]] | None = None,
         interfaces: dict[str, list[dict[str, Any]]] | None = None, windows: dict[str, int] | None = None,
         block: dict[str, Any] | None = None) -> list[dict[str, Any]]:
    """lints.json's reading: the policy's findings; when it has no error,
    the plan's (with ``implements``), the windows' (with ``windows``) and
    the block's."""
    policy, out = parse_policy(policy_text)
    if policy is None:
        return out
    out = list(out)
    if implements is not None:
        p = plan(policy_text, implements, interfaces or {})
        if "refused" in p:
            out += p["refused"]
        else:
            for f in p["faces"]:
                out += f["findings"]
            if windows is not None:
                out += check_windows(p, windows)
    if block is not None:
        out += check_block(block)
    return sort_findings(out)


# -- a generator for zenoh-python routers (§2.4, §2.5, §2.8) ------------------------------------

def compile_face(face_plan: dict[str, Any], block: dict[str, Any], far_subject: str) -> dict[str, Any]:
    """Put a face's plan into a router's ``access_control`` block (zk2py's
    generated grants, ``zk2py.acl``): every deny of the plan as a rule in
    the far principal's policy (§2.4: "the far principal's, at least"),
    and no subject of the face's own. Returns the block; the downsampling
    and the gateway are :func:`router_extras`'."""
    out = {**block, "rules": list(block["rules"]), "policies": [dict(p) for p in block["policies"]]}
    ids = []
    for d in face_plan["deny"]:
        rid = f"face-{face_plan['name']}-{d['id']}"
        out["rules"].append({"id": rid, "messages": list(d["messages"]), "flows": list(d["flows"]),
                             "permission": "deny", "key_exprs": list(d["key_exprs"])})
        ids.append(rid)
    for p in out["policies"]:
        if far_subject in p["subjects"]:
            p["rules"] = list(p["rules"]) + ids
    return out


def router_extras(face_plan: dict[str, Any]) -> dict[str, Any]:
    """The plan's other router configuration: the ``downsampling`` items
    (§2.8) and, for a south region, the near router's ``gateway`` (§2.5)."""
    extras: dict[str, Any] = {}
    ds = face_plan.get("downsampling")
    if ds:
        extras["downsampling"] = [{"id": ds["id"], "link_protocols": ds["link_protocols"], "flows": ds["flows"],
                                   "messages": ds["messages"], "rules": ds["rules"]}]
    if face_plan["attach"]["shape"] == "south-region":
        extras["gateway"] = face_plan["attach"]["gateway"]
    return extras

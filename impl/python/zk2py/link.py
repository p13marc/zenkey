"""``link.v1``, text 0.7 (``spec/profiles/link/v1.md``, written against
core 0.32): what crosses a constrained face. Read again against text 0.9
(#609, round 86; #935): its inline history taken out and nine long
sentences split, no rule changed, and none of this module's rules moved.

From the text alone:
- **the face policy** (§2.1): :func:`parse_policy` reads the TOML file and
  reports the linter's findings, each ``{code, severity, face, at}``;
- **``link.exposure``** (§2.2): :func:`exposure`, and whether a resource's
  pushes cross a face (§2.7, §2.8);
- **the plan** (§2.3, §2.5 to §2.12): :func:`plan` derives each face's
  denies, with §2.7's carve-out (:func:`carve`), its downsampling rules,
  its attachment, its window floors and its liveness evidence, or the
  findings that refuse it; every revision of an interface it is given is
  read together (0.3, :func:`merge_revisions`);
- **the checks**: a deployment's tombstone windows (§2.11,
  :func:`check_windows`) and a router's block (§2.15, :func:`check_block`,
  with 0.4's subject checks, :func:`check_subjects` and
  :func:`narrowed_too_far`, and the denies a block holds,
  :func:`subject_denies`); :func:`lint` is ``lints.json``'s whole reading;
- **a generator** for zenoh-python routers (:func:`compile_faces`): the
  plan's denies in the far principal's policy, every other principal's
  subject narrowed off the face, and under ``allow`` the face's own
  subject in the far principal's policy (§2.4, 0.3), its downsampling
  block, and the near router's ``gateway`` for a south region (§2.5).

Where the text left a choice, zk2py's is named in ``SPEC-FINDINGS.md``
("At link.v1 0.1" to "At link.v1 0.7").
"""

from __future__ import annotations

import tomllib
from dataclasses import dataclass, field
from collections.abc import Sequence
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
#: §2.1: zenoh 1.10.1's link protocol names, "§2.1's closed list", in its
#: order (§2.4: a narrowed subject names them "in that list's order").
LINK_PROTOCOL_ORDER = ["tcp", "udp", "tls", "quic", "serial", "unixpipe", "unixsock-stream", "vsock", "ws"]
LINK_PROTOCOLS = set(LINK_PROTOCOL_ORDER)
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
    out += far_of_two_faces(doc.get("face"))
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


def far_of_two_faces(faces: Any) -> list[dict[str, Any]]:
    """§2.1 (0.7, F-125): "Two or more faces of the policy name the same
    far … One finding per such name, with no face; at is the name". It
    "reads every face whose far is a non-empty string, whatever else the
    policy or the face holds", a face whose name is no plain chunk among
    them."""
    if type(faces) is not dict:
        return []
    count: dict[str, int] = {}
    for body in faces.values():
        far = body.get("far") if type(body) is dict else None
        if type(far) is str and far:
            count[far] = count.get(far, 0) + 1
    return [finding("far_of_two_faces", None, far, "warning") for far, n in count.items() if n >= 2]


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
    return _reading(token, template, exposure_of(token, annotations), expose)


def _reading(token: str, template: str, e: str, expose: str) -> dict[str, str]:
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
    """One resource of a listed interface, read over every revision given
    that declares it (§2.3, 0.3). ``annotations`` are the first declaring
    revision's; ``exp`` and ``merged_horizon`` are the revisions' together."""
    iface: str
    token: str
    template: str
    annotations: dict[str, Any]
    #: §2.2 over every declaring revision: "link" only where every one
    #: reads link; else the one revision's reading, or host.
    exp: str = ""
    #: "one reading link and another not" (exposure_disagrees).
    disagrees: bool = False
    #: §2.12 over every declaring revision: "only where every revision that
    #: declares it gives one above 0; its horizon is then the least".
    merged_horizon: freshness.Horizon | None = None

    @property
    def name(self) -> str:
        return f"{self.iface}/{self.token}/{self.template}"

    def exposure(self) -> str:
        return self.exp or exposure_of(self.token, self.annotations)

    def crossing(self, expose: str) -> bool:
        return _reading(self.token, self.template, self.exposure(), expose)["reason"] == "exposed"

    def horizon(self) -> freshness.Horizon:
        if self.merged_horizon is not None:
            return self.merged_horizon
        return freshness.horizon(_KIND_OF_TOKEN[self.token], self.annotations)


def revisions_of(entry: Any) -> list[list[dict[str, Any]]]:
    """plan.json's interface entry: "one revision's resources, or
    {revisions: [resources, ...]}" (0.3)."""
    if isinstance(entry, dict) and "revisions" in entry:
        return [list(r) for r in entry["revisions"]]
    return [list(entry)]


def merge_revisions(iface: str, revisions: list[list[dict[str, Any]]]) -> list[Resource]:
    """§2.3 "Revisions read together" (0.3): "Its resources are those any
    revision declares, paired by kind token and template. A revision that
    does not declare a resource says nothing of it"; "A resource is exposed
    only where every revision that declares it reads link … Where one reads
    link and another does not … it is read as host"; "A resource has a
    horizon (§2.12) only where every revision that declares it gives one
    above 0; its horizon is then the least of them". Resources keep the
    order of their first declaration. One revision is read as itself."""
    if len(revisions) == 1:
        return [Resource(iface, r["token"], r["template"], r.get("annotations") or {}) for r in revisions[0]]
    order: list[tuple[str, str]] = []
    declared: dict[tuple[str, str], list[dict[str, Any]]] = {}
    for rev in revisions:
        for r in rev:
            k = (r["token"], r["template"])
            if k not in declared:
                order.append(k)
                declared[k] = []
            declared[k].append(r.get("annotations") or {})
    out = []
    for token, template in order:
        anns = declared[(token, template)]
        exps = [exposure_of(token, a) for a in anns]
        disagrees = False
        if all(e == "link" for e in exps):
            exp = "link"
        elif any(e == "link" for e in exps):
            exp, disagrees = "host", True
        else:
            # none reads link: host, invalid and ignored are alike to the
            # plan; keep the first's.
            exp = exps[0]
        hs = [freshness.horizon(_KIND_OF_TOKEN[token], a) for a in anns]
        if all(h.horizon == "within" for h in hs):
            h = freshness.Horizon("within", min(x.ttl_s for x in hs))
        else:
            h = freshness.Horizon("none")
        out.append(Resource(iface, token, template, anns[0], exp, disagrees, h))
    return out


def resources_of(contract: Any) -> list[dict[str, Any]]:
    """A loaded contract's resolved resources, as plan.json writes them."""
    return [{"template": r["template"], "token": r["token"], "annotations": dict(r["annotations"])}
            for r in contract.canonical["resources"]]


def plan(policy_text: str, implements: dict[str, list[str]], interfaces: dict[str, Any]) -> dict[str, Any]:
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
              interfaces: dict[str, Any]) -> dict[str, Any]:
    listed = sorted({i for s in policy.services for i in implements[s]}, key=_bytes)
    res = {i: merge_revisions(i, revisions_of(interfaces[i])) for i in listed}
    expose = face.expose
    crosses = {(r.iface, r.token, r.template): r.crossing(expose) for i in listed for r in res[i]}
    warnings: list[dict[str, Any]] = []
    if expose == "link":
        # §2.3 (0.3): "On a face that exposes, a resource the revisions of
        # its interface disagree on, one reading link and another not".
        warnings += [finding("exposure_disagrees", face.name, r.name, "warning")
                     for i in listed for r in res[i] if r.disagrees]
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
                if r.token in ("stream", "@stream") and r.exposure() == "link" and has_parameter(r.template):
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


def check_block(block: dict[str, Any], policy: Policy | None = None) -> list[dict[str, Any]]:
    """§2.15: a router's block for one face against the face it guards:
    ``zk_not_denied`` (over ``deny``, "one subject's", lints.json),
    ``wildcard_downsampling`` and ``intersecting_downsampling``; and, given
    the block's ``default_permission``, ``subjects`` and ``policies``, "all
    three or none" (0.4), the subject checks (:func:`check_subjects`, per
    the block's face) and :func:`narrowed_too_far` (every face at once)."""
    face, ns = block["face"], block.get("namespace") or ""
    target = _keyexpr(f"{ns}/zk2/**/@zk/**" if ns else "zk2/**/@zk/**")
    out = []
    # §2.15 (0.7, F-124): "block_disabled (error, at enabled): the block's
    # access control is not enabled". In lints.json's shape an absent
    # enabled "was not read"; a router's configuration is read by
    # check_router_block, which gives an absent one as false.
    if "enabled" in block:
        if type(block["enabled"]) is not bool:
            raise BlockError(f"enabled is {block['enabled']!r}, not a boolean (zenoh refuses it at load)")
        if not block["enabled"]:
            out.append(finding("block_disabled", face, "enabled"))

    def denied(m: str, fl: str) -> bool:
        return any(m in d.get("messages", []) and fl in rule_flows(d)
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
    if policy is not None and all(k in block for k in ("default_permission", "subjects", "policies")):
        guarded = next((f for f in policy.faces if f.name == face), None)
        if guarded is not None:
            out += check_subjects(guarded, block["subjects"], block["policies"], block["default_permission"])
        out += narrowed_too_far(policy, block["subjects"], block["default_permission"])
    return out


class BlockError(ValueError):
    """A block no router runs: zenoh 1.10.1 refuses it when the
    configuration loads (measured: a subject or a rule with no id, two
    subjects, two rules or two policies with one id, a policy id that is
    not a string, a policy listing a subject or a rule the block does not
    define), or a face the face policy
    does not have. A check says so rather than reading it. A policy with no
    id loads (measured), so a policy needs none."""


def rule_flows(rule: dict[str, Any]) -> list[str]:
    """A rule's flows; one that names none applies to both (zenoh 1.10.1,
    measured: an allow without ``flows`` admits a put on ingress and on
    egress, and a deny without it refuses both)."""
    flows = rule.get("flows")
    return list(FLOWS) if flows is None else list(flows)


def check_ids(subjects: list[dict[str, Any]], policies: list[dict[str, Any]],
              rules: list[dict[str, Any]] | None = None) -> None:
    """Raise :class:`BlockError` for what zenoh refuses at load (each
    measured): a subject with no id, an id two subjects share; given
    ``rules``, a rule with no id, an id two rules share, and a policy
    listing a rule the block does not define; a policy listing a subject
    the block does not define; and an id two policies share."""
    ids: set[str] = set()
    for i, s in enumerate(subjects):
        sid = s.get("id")
        if not isinstance(sid, str) or not sid:
            raise BlockError(f"subject #{i} has no id")
        if sid in ids:
            raise BlockError(f"subject id {sid!r} is repeated")
        ids.add(sid)
    rule_ids: set[str] | None = None
    if rules is not None:
        rule_ids = set()
        for i, r in enumerate(rules):
            rid = r.get("id")
            if not isinstance(rid, str) or not rid:
                raise BlockError(f"rule #{i} has no id")
            if rid in rule_ids:
                raise BlockError(f"rule id {rid!r} is repeated")
            rule_ids.add(rid)
    policy_ids: set[str] = set()
    for i, pol in enumerate(policies):
        pid = pol.get("id")
        # Measured: a policy id that is a list or a number is refused at
        # insert ("invalid type … expected a string"); a null one loads, as
        # an absent one does.
        if pid is not None and not isinstance(pid, str):
            raise BlockError(f"policy #{i} has an id that is not a string: {pid!r}")
        name = pid if pid is not None else f"#{i}"
        if pid is not None:
            if pid in policy_ids:
                raise BlockError(f"policy id {pid!r} is repeated")
            policy_ids.add(pid)
        for sid in pol.get("subjects") or []:
            if sid not in ids:
                raise BlockError(f"policy {name} lists subject {sid!r}, which the block does not define")
        if rule_ids is not None:
            for rid in pol.get("rules") or []:
                if rid not in rule_ids:
                    raise BlockError(f"policy {name} lists rule {rid!r}, which the block does not define")


def _names(subject: dict[str, Any], prop: str) -> list[str]:
    """A subject's list, "a check given one reads it as absent" when empty
    (0.4: "An empty list … is refused when the configuration loads")."""
    return list(subject.get(prop) or [])


def _user_names(subject: dict[str, Any]) -> list[str]:
    return _names(subject, "usernames") + _names(subject, "cert_common_names")


def on_face(subject: dict[str, Any], face: Face) -> bool:
    """§2.15 (0.4): "A subject is on the face when its link_protocols are
    absent or name one of the face's"."""
    lps = _names(subject, "link_protocols")
    return not lps or bool(set(lps) & set(face.link_protocols))


def far_principals(subject: dict[str, Any], far: str) -> bool:
    """§2.15 (0.5): "the far principal's when it names the far principal
    alone in one of its two lists and nothing in the other: its user names
    are all the face's far and it names no CN, or its CNs are all the face's
    far and it names no user name"."""
    users, cns = _names(subject, "usernames"), _names(subject, "cert_common_names")
    return bool(users and not cns and all(n == far for n in users)
                or cns and not users and all(n == far for n in cns))


def is_face_subject(subject: dict[str, Any], face: Face) -> bool:
    """§2.15 (0.4): "a subject on the face that names no user name and no
    CN, whatever its id"."""
    return on_face(subject, face) and not _user_names(subject)


def _listing(subjects: list[dict[str, Any]], policies: list[dict[str, Any]]) -> dict[str, set[int]]:
    """Each subject id -> the policies (by index) that list it; a block
    zenoh would refuse raises :class:`BlockError`."""
    check_ids(subjects, policies)
    out: dict[str, set[int]] = {s["id"]: set() for s in subjects}
    for i, p in enumerate(policies):
        for sid in p.get("subjects") or []:
            out[sid].add(i)
    return out


def far_policies(face: Face, subjects: list[dict[str, Any]], policies: list[dict[str, Any]]) -> set[int]:
    """§2.15 (0.4): "The far principal's policies are those that list one
    of its subjects on the face"."""
    listing = _listing(subjects, policies)
    return {i for s in subjects if on_face(s, face) and far_principals(s, face.far) for i in listing[str(s.get("id"))]}


def check_subjects(face: Face, subjects: list[dict[str, Any]], policies: list[dict[str, Any]],
                   default_permission: str) -> list[dict[str, Any]]:
    """§2.15's subject checks for one face (0.3, F-118; 0.4, F-120):
    - ``near_subject_on_face``: "a subject on the face bound to another
      principal, whatever else it names, the far principal's name among
      them";
    - ``face_subject_beyond_face``: "a face subject whose link_protocols are
      absent, or name one outside the face's";
    - ``face_subject_misplaced``: "a face subject not listed in exactly the
      far principal's policies, at least one";
    - ``face_subject_missing``: "under allow, a link protocol of the face
      that no face subject naming no interface and no zid names in its
      link_protocols"."""
    out = []
    mine = set(face.link_protocols)
    listing = _listing(subjects, policies)
    theirs = far_policies(face, subjects, policies)
    for s in subjects:
        sid = str(s.get("id"))
        if not on_face(s, face):
            continue
        if _user_names(s):
            if not far_principals(s, face.far):
                out.append(finding("near_subject_on_face", face.name, sid))
            continue
        lps = _names(s, "link_protocols")
        if not lps or set(lps) - mine:
            out.append(finding("face_subject_beyond_face", face.name, sid))
        if not theirs or listing[sid] != theirs:
            out.append(finding("face_subject_misplaced", face.name, sid))
    if default_permission == "allow":
        own = [s for s in subjects if is_face_subject(s, face) and not _names(s, "interfaces") and not _names(s, "zids")]
        for lp in face.link_protocols:
            if not any(lp in _names(s, "link_protocols") for s in own):
                out.append(finding("face_subject_missing", face.name, lp))
    return out


#: The two kinds of name a subject names, by its list (§2.15, 0.5), and
#: the kind of a face's far that no subject names.
USER, CN, UNTOLD = "usernames", "cert_common_names", None


def answers(subject: dict[str, Any], name: str, kind: str | None) -> bool:
    """§2.15 (0.5), "Who answers": "A subject that names no interface and
    no zid answers for a user name when it names that user name among its
    user names and no CN, or names no user name and no CN; for a CN, the
    same with the two lists swapped. A face's far that no subject names is
    answered only by a subject that names no user name, CN, interface or
    zid"."""
    if _names(subject, "interfaces") or _names(subject, "zids"):
        return False
    users, cns = _names(subject, USER), _names(subject, CN)
    if not users and not cns:
        return True
    if kind == USER:
        return name in users and not cns
    if kind == CN:
        return name in cns and not users
    return False


def named_kinds(policy: Policy, subjects: list[dict[str, Any]]) -> list[tuple[str, str | None]]:
    """§2.15 (0.5), "The names, by kind": each user name a subject names,
    each CN, "a name in both lists is checked as each", and a face's far
    that no subject names, of a kind the block cannot tell. In order of
    first naming."""
    out: list[tuple[str, str | None]] = []
    for s in subjects:
        for kind in (USER, CN):
            for n in _names(s, kind):
                if (n, kind) not in out:
                    out.append((n, kind))
    named = {n for n, _ in out}
    for f in policy.faces:
        if f.far not in named and (f.far, UNTOLD) not in out:
            out.append((f.far, UNTOLD))
    return out


def narrowed_too_far(policy: Policy, subjects: list[dict[str, Any]], default_permission: str) -> list[dict[str, Any]]:
    """§2.15 (0.4; by kind, 0.5): "under allow, a name the block binds, read
    by its kind, and a link protocol of §2.1's list outside every face whose
    far principal it is not, that no subject answering for that name
    selects" (:func:`answers`). "It reports a name once, whichever of its
    kinds fails", with no face."""
    if default_permission != "allow":
        return []
    out: list[dict[str, Any]] = []
    for name, kind in named_kinds(policy, subjects):
        if any(f["at"] == name for f in out):
            continue
        off = {lp for f in policy.faces if f.far != name for lp in f.link_protocols}
        for lp in LINK_PROTOCOL_ORDER:
            if lp in off:
                continue
            if not any(answers(s, name, kind) and (not _names(s, "link_protocols") or lp in _names(s, "link_protocols"))
                       for s in subjects):
                out.append(finding("narrowed_too_far", None, name))
                break
    return out


def subject_denies(block: dict[str, Any], face: Face) -> dict[str, list[dict[str, Any]]]:
    """§2.15 (0.4), "The denies a face's block holds", over a router's
    ``access_control`` block (rules with ids and permissions, policies
    listing rule and subject ids): for each subject "that can match the far
    principal's session on the face" — on the face, and naming the far
    principal among its user names or CNs, or naming no user name, CN,
    interface or zid — "the deny rules of every policy that lists it", by
    subject id. "With no such subject, the denies are none": an empty
    mapping."""
    check_ids(block.get("subjects", []), block.get("policies", []), block.get("rules", []))
    rules = {r.get("id"): r for r in block.get("rules", [])}
    out: dict[str, list[dict[str, Any]]] = {}
    for s in block.get("subjects", []):
        if not on_face(s, face):
            continue
        names = _user_names(s)
        if names and face.far not in names:
            continue
        if not names and (_names(s, "interfaces") or _names(s, "zids")):
            continue
        held = [rules.get(r) for p in block.get("policies", []) if s["id"] in (p.get("subjects") or [])
                for r in (p.get("rules") or [])]
        out[s["id"]] = [{"flows": rule_flows(r), "messages": list(r.get("messages") or []),
                         "key_exprs": list(r.get("key_exprs") or [])}
                        for r in held if r is not None and r.get("permission") == "deny"]
    return out


def check_router_block(block: dict[str, Any], policy: Policy, face_name: str,
                       downsampling: Sequence[dict[str, Any]] | None = None, namespace: str = "") -> list[dict[str, Any]]:
    """§2.15 over a router's whole ``access_control`` block: each subject's
    denies (:func:`subject_denies`) "checked alone", one ``zk_not_denied``
    when any of them, or none at all, misses a pair; and every other check
    as :func:`check_block` reads lints.json's shape. Sorted. A face the
    policy does not have, or a block zenoh refuses at load, raises
    :class:`BlockError`."""
    face = next((f for f in policy.faces if f.name == face_name), None)
    if face is None:
        raise BlockError(f"the face policy has no face {face_name!r} (it has {[f.name for f in policy.faces]})")
    check_ids(block.get("subjects", []), block.get("policies", []), block.get("rules", []))
    downsampling = list(downsampling or [])
    # §2.15 (0.7), "What a block omits": "enabled absent as false, a
    # block's default_permission absent as deny, and a rule's flows absent
    # as both flows" (rule_flows).
    view = {"face": face_name, "namespace": namespace, "downsampling": downsampling,
            "enabled": block.get("enabled", False),
            "default_permission": block.get("default_permission", "deny"),
            "subjects": [{"id": s["id"], **{k: list(s[k]) for k in SUBJECT_PROPS if s.get(k)}}
                         for s in block.get("subjects", [])],
            "policies": [{"subjects": list(p.get("subjects", []))} for p in block.get("policies", [])]}
    out: list[dict[str, Any]] = []
    for denies in list(subject_denies(block, face).values()) or [[]]:
        for f in check_block({**view, "deny": denies}, policy):
            if f not in out:
                out.append(f)
    return sort_findings(out)


#: zenoh 1.10.1's AclConfigSubjects properties, as lints.json writes a subject.
SUBJECT_PROPS = ("usernames", "cert_common_names", "link_protocols", "interfaces", "zids")


def lint(policy_text: str, *, implements: dict[str, list[str]] | None = None,
         interfaces: dict[str, Any] | None = None, windows: dict[str, int] | None = None,
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
        out += check_block(block, policy)
    return sort_findings(out)


# -- a generator for zenoh-python routers (§2.4, §2.5, §2.8) ------------------------------------

class Refused(ValueError):
    """§2.4: "A principal left with no link protocol could connect over
    none, and a generator refuses it"."""


def _copy(block: dict[str, Any]) -> dict[str, Any]:
    return {**block, "rules": list(block["rules"]), "subjects": [dict(s) for s in block["subjects"]],
            "policies": [dict(p, rules=list(p["rules"]), subjects=list(p["subjects"])) for p in block["policies"]]}


def compile_face(face_plan: dict[str, Any], block: dict[str, Any], far_subject: str) -> dict[str, Any]:
    """0.2's generator, kept for scenarios §7's step 2 control: every deny
    of the plan as a rule in the far principal's policy, no subject
    narrowed and no subject of the face's own. :func:`compile_faces` is
    0.3's."""
    return _with_denies(face_plan, _copy(block), far_subject)


def _with_denies(face_plan: dict[str, Any], out: dict[str, Any], far_subject: str) -> dict[str, Any]:
    ids = []
    for d in face_plan["deny"]:
        rid = f"face-{face_plan['name']}-{d['id']}"
        if not any(r["id"] == rid for r in out["rules"]):
            out["rules"].append({"id": rid, "messages": list(d["messages"]), "flows": list(d["flows"]),
                                 "permission": "deny", "key_exprs": list(d["key_exprs"])})
        ids.append(rid)
    for p in out["policies"]:
        if far_subject in p["subjects"]:
            p["rules"] = list(p["rules"]) + [i for i in ids if i not in p["rules"]]
    return out


def face_subject_id(face: str) -> str:
    """The face's own subject's id. The text names none; its scenarios (§7
    step 3) and Appendix B call the radio's ``face:radio``, and zk2py
    follows."""
    return f"face:{face}"


def compile_faces(policy: Policy, face_plans: list[dict[str, Any]], block: dict[str, Any], *,
                  narrow: bool = True, face_subject: str | None = "far", any_posture: bool = False) -> dict[str, Any]:
    """§2.4 (0.3) over a router's block (zk2py's generated grants, one
    subject per principal, bound by its user name), for each face of
    ``policy`` with its plan:
    - every deny of its plan as a rule in the far principal's policy
      ("Into the far principal's policy"), the far principal's subjects
      being those whose user names or CNs name it;
    - with ``narrow``, "A subject bound to a principal is selected by the
      link protocols of §2.1's closed list outside every face whose far
      principal it is not, in that list's order", the far principal's
      subject narrowed off the other faces only ("and no further");
    - under ``allow``, "a subject of its own, selected by its link
      protocols alone and bound to no principal, which the far principal's
      policy lists beside the far principal's own subject"
      (``face_subject="far"``). The controls of scenarios §7:
      ``face_subject=None`` defines none (step 4), ``"own"`` gives it a
      policy of its own holding the plan's denies alone (step 5), and
      ``any_posture`` adds it under ``deny`` too (zk2py's measurement).
    Raises :class:`Refused` for a principal left with no link protocol, or
    a face whose far principal has no subject."""
    out = _copy(block)
    allow = out.get("default_permission") == "allow"
    faces = {f.name: f for f in policy.faces}

    def names_of(s: dict[str, Any]) -> list[str]:
        return _names(s, "usernames") + _names(s, "cert_common_names")

    def bound_to(name: str) -> list[str]:
        return [s["id"] for s in out["subjects"] if name in names_of(s)]

    for s in out["subjects"]:
        # §2.4 (0.5): "On a router with a face, a generator writes each
        # subject with one of the two lists … and refuses a principal it is
        # given with both".
        if face_plans and _names(s, "usernames") and _names(s, "cert_common_names"):
            raise Refused(f"subject {s.get('id')}: a principal bound by a user name and a CN both, on a router "
                          "with a face")
    for fp in face_plans:
        fars = bound_to(fp["far"])
        if not fars:
            raise Refused(f"face {fp['name']}: no subject is bound to its far principal {fp['far']}")
        for far in fars:
            _with_denies(fp, out, far)
    if narrow:
        for s in out["subjects"]:
            names = names_of(s)
            if not names:
                continue
            off = {lp for fp in face_plans if fp["far"] not in names for lp in faces[fp["name"]].link_protocols}
            if not off:
                continue
            keep = [lp for lp in LINK_PROTOCOL_ORDER if lp not in off
                    and (not s.get("link_protocols") or lp in s["link_protocols"])]
            if not keep:
                raise Refused(f"subject {s['id']}: no link protocol is left outside the faces it is kept off")
            s["link_protocols"] = keep
    if (allow or any_posture) and face_subject is not None:
        for fp in face_plans:
            sid = face_subject_id(fp["name"])
            out["subjects"].append({"id": sid, "link_protocols": list(faces[fp["name"]].link_protocols)})
            if face_subject == "far":
                fars = set(bound_to(fp["far"]))
                for p in out["policies"]:
                    if fars & set(p["subjects"]):
                        p["subjects"].append(sid)
            else:
                ids = [f"face-{fp['name']}-{d['id']}" for d in fp["deny"]]
                out["policies"].append({"id": f"{sid}-policy", "rules": ids, "subjects": [sid]})
    return out


def compare_block(policy: Policy, face_plans: list[dict[str, Any]], block: dict[str, Any]) -> list[str]:
    """§2.15's comparison with the plan, zk2py's shape of it (0.4: "Its
    shape is the generator's"): "Whether a block holds the whole plan, every
    deny and every rule". One line per difference: a deny of the plan
    missing from a policy listing the far principal's subject. Under
    ``allow`` it also names a face whose own subject rides no such policy,
    which 0.4's ``face_subject_misplaced`` now reports from the block
    alone."""
    out = []
    faces = {f.name: f for f in policy.faces}
    check_ids(block.get("subjects", []), block.get("policies", []), block.get("rules", []))
    rules = {r.get("id"): r for r in block.get("rules", [])}

    def names_of(s: dict[str, Any]) -> list[str]:
        return _names(s, "usernames") + _names(s, "cert_common_names")

    for fp in face_plans:
        fars = {s["id"] for s in block.get("subjects", []) if fp["far"] in names_of(s)}
        far_policies = [p for p in block.get("policies", []) if fars & set(p.get("subjects") or [])]
        for p in far_policies:
            held = [r for r in (rules.get(x) for x in (p.get("rules") or []))
                    if r is not None and r.get("permission") == "deny"]
            for d in fp["deny"]:
                if not any(set(d["key_exprs"]) <= set(r.get("key_exprs") or [])
                           and set(d["messages"]) <= set(r.get("messages") or [])
                           and set(d["flows"]) <= set(rule_flows(r)) for r in held):
                    out.append(f"face {fp['name']}: policy {p['id']} lacks the plan's deny {d['id']}")
        if block.get("default_permission") == "allow":
            mine = set(faces[fp["name"]].link_protocols)
            own = [s["id"] for s in block.get("subjects", [])
                   if not any(_names(s, k) for k in ("usernames", "cert_common_names", "interfaces", "zids"))
                   and mine <= set(_names(s, "link_protocols"))]
            if not own:
                out.append(f"face {fp['name']}: no subject of its own")
                continue
            for sid in own:
                if not any(sid in p["subjects"] for p in far_policies):
                    elsewhere = [p["id"] for p in block.get("policies", []) if sid in p["subjects"]]
                    out.append(f"face {fp['name']}: its subject {sid} rides {elsewhere or 'no policy'}, not the far "
                               f"principal's")
    return out


#: core §4.4: "An archive is a service implementing archive.v1".
ARCHIVE_IFACE = "archive.v1"


def declaration_grant(gets: list[tuple[str, list[str]]], calls: list[tuple[str, list[str], str]],
                      archives: list[str] = ()) -> list[str]:  # type: ignore[assignment]
    """§2.5 (0.2), core §8.5 (0.30): the far principal's ``declare_queryable``
    egress grant "over every queryable its side may query, as the owner
    declares it".
    - ``gets`` are (interface, providers) whose state the far side GETs:
      "the owner's state queryables over its interface's state/** and
      @state/** (core S2), with the providers as bound";
    - ``calls`` are (interface, providers, operation template): "the
      operation's queryable over its template's pattern (core §5)";
    - ``archives`` are the archives (addresses, as bound) the far side
      reads last-known state from: "an archive's last-known GET: its
      queryable over its @state forms (core §4.4)", which are under the
      archive's own ``archive.v1/@state`` token, so its state queryable
      there.
    A ``gets`` entry for ``archive.v1`` is an archive read too, and is
    covered by the same keys."""
    keys: list[str] = []
    for iface, providers in gets:
        keys += [f"zk2/{p}/{iface}/{t}/**" for p in providers for t in ("state", "@state")]
    for iface, providers, template in calls:
        pattern = "/".join(_pattern(s) for s in templates.parse_template(template).segments)
        keys += [f"zk2/{p}/{iface}/@op/{pattern}" for p in providers]
    keys += [f"zk2/{a}/{ARCHIVE_IFACE}/@state/**" for a in archives]
    return sorted(set(keys), key=_bytes)


def liveness_across(face_plan: dict[str, Any], address: str, subscriber: Any,
                    annotations: dict[str, dict[str, Any]]) -> tuple[str, str]:
    """§2.12 and §5, "Is this provider live across the face?": ``live``
    while a member of the plan's evidence for ``address`` is fresh by
    ``subscriber``'s receive clock (``freshness.v1`` §2.5), else
    ``unobservable``: no evidence, or none of it fresh. Never down (core
    R7). ``annotations`` are each evidence resource's, by its name."""
    entry = next((x for x in face_plan["liveness"] if x["service"] == address), None)
    if entry is None or not entry["evidence"]:
        return "unobservable", "the plan lists no evidence for it"
    for name in entry["evidence"]:
        iface, token, template = name.split("/", 2)
        key = f"zk2/{address}/{iface}/{token}/{template}"
        v = freshness.judge(_KIND_OF_TOKEN[token], annotations[name], [subscriber.observation(key)])
        if v.verdict == freshness.FRESH:
            return "live", f"{name} is fresh"
    return "unobservable", "none of its evidence is fresh"


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

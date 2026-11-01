"""``link.v2``, text 0.1 (``spec/profiles/link/v2.md``, experimental, written
against core 0.77; #988 L1 and L2): a constrained face, and the transport it
runs. Read cold by zk2py in round 96 (#609).

It restates ``link.v1`` by reference (§0), and :mod:`zk2py.link` keeps those
rules. This module holds what the text adds:
- **the policy** (§2.1, §2.4): :func:`parse_policy` reads a policy with
  ``profile = "link.v2"``, its ``binding``, ``catalog``, ``[transport]`` and
  each face's ``loss_ppm``, ``arq`` and ``reply_timeout_ms``, and every
  finding, ``link.v1``'s codes and the eight this text adds; a policy with no
  ``profile`` is read by :func:`zk2py.link.parse_policy`, finding for finding;
- **the plan** (§2.18): :func:`plan`, ``link.v1``'s per face with its
  ``reply_timeout_ms``, beside the binding, the catalog id and the
  transport; :func:`narrowed`, ``link.v1`` §2.4's narrowing over the
  binding's list;
- **the transport block** (§2.18) and **its checks** (§2.19):
  :func:`emit` and :func:`check_transport`.

The fixtures are ``conformance/v2/`` (``lints.json``, ``plan.json``,
``transport.json``).
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any

from . import link
from .lexical import is_plain_chunk

PROFILE = "link.v2"
#: §2.4: each binding's closed list of link protocols, in its order.
BINDINGS = {
    "zenoh-1.10.1": list(link.LINK_PROTOCOL_ORDER),
    "zm-1.10.2-r1": ["tcp", "udp", "tls", "quic", "serial", "unixpipe", "unixsock-stream", "unixsock-dgram",
                     "vsock", "ws"],
}
U32_MAX = link.U32_MAX
#: §2.1's ``[transport]``: key -> (zenoh's configuration key, required,
#: the default where optional), in the table's order.
TRANSPORT = {
    "lease_ms": ("transport/link/tx/lease", True, None),
    "keep_alive": ("transport/link/tx/keep_alive", True, None),
    "wait_before_close_ms": ("transport/link/tx/queue/congestion_control/block/wait_before_close", True, None),
    "sequence": ("transport/link/tx/sequence_number_resolution", True, None),
    "batch_bytes": ("transport/link/tx/batch_size", False, 65535),
    "open_timeout_ms": ("transport/unicast/open_timeout", False, 10_000),
    "accept_timeout_ms": ("transport/unicast/accept_timeout", False, 10_000),
    "interests_timeout_ms": ("routing/interests/timeout", False, 10_000),
}
SEQUENCES = ("8bit", "16bit", "32bit", "64bit")
#: §2.16: "That window is 64 frames at 8bit, 8,192 at 16bit, 2^27 at 32bit
#: and 2^31 at 64bit".
WINDOW = {"8bit": 64, "16bit": 8192, "32bit": 2**27, "64bit": 2**31}
#: §2.19's table: what a configuration omits, read at the binding's default
#: (wait_before_close in µs).
DEFAULTS = {
    "zenoh-1.10.1": {"lease_ms": 10_000, "keep_alive": 4, "wait_before_close_ms": 5_000_000, "sequence": "32bit",
                     "batch_bytes": 65535, "open_timeout_ms": 10_000, "accept_timeout_ms": 10_000,
                     "interests_timeout_ms": 10_000},
    "zm-1.10.2-r1": {"lease_ms": 10_000, "keep_alive": 4, "wait_before_close_ms": 10_000_000, "sequence": "32bit",
                     "batch_bytes": 65535, "open_timeout_ms": 10_000, "accept_timeout_ms": 10_000,
                     "interests_timeout_ms": 10_000},
}
FACE_KEYS = ("far", "attach", "region", "select", "class", "expose", "interval_s", "max_outage_s", "link_bps",
             "loss_ppm", "arq", "reply_timeout_ms")
MOVED = ("lease_ms", "batch_bytes")


def _int_in(v: Any, lo: int, hi: int) -> bool:
    return type(v) is int and lo <= v <= hi


@dataclass
class FaceV2:
    face: link.Face
    loss_ppm: int
    arq: bool
    reply_timeout_ms: int


@dataclass
class PolicyV2:
    """A link.v2 policy read without an error (``v1`` set for a policy with no
    ``profile``, read by ``link.v1``)."""

    system: str
    services: list[str]
    binding: str
    catalog: str | None
    transport: dict[str, Any] | None
    faces: list[FaceV2]
    findings: list[dict[str, Any]] = field(default_factory=list)
    v1: link.Policy | None = None

    def as_v1(self) -> link.Policy:
        """``link.v1``'s policy over the same faces, each face's batch and
        lease the transport's (§0: "a face's batch_bytes and lease_ms are
        [transport]'s")."""
        return self.v1 or link.Policy(self.system, self.services, [f.face for f in self.faces], self.findings)


def parse_policy(text: str) -> tuple[PolicyV2 | None, list[dict[str, Any]]]:
    """§2.1: the policy when it holds no error, and every finding, sorted as
    ``link.v1`` §2.1 sorts them."""
    doc = link._toml(text)
    if doc is None:
        return None, [link.finding("toml", None, "")]
    if "profile" not in doc:
        # "A policy with no profile is a link.v1 policy. A reader of this text
        # MUST read it as link.v1 §2.1 says, finding for finding."
        p, out = link.parse_policy(text)
        if p is None:
            return None, out
        return PolicyV2(p.system, p.services, "zenoh-1.10.1", None, None,
                        [FaceV2(f, 0, True, 0) for f in p.faces], out, v1=p), out
    out: list[dict[str, Any]] = []
    for k in doc:
        if k not in ("profile", "system", "services", "binding", "catalog", "transport", "face"):
            out.append(link.finding("unknown_field", None, k))
    if doc["profile"] != PROFILE:
        out.append(link.finding("bad_value", None, "profile"))
    system, services = doc.get("system"), doc.get("services")
    if "system" not in doc:
        out.append(link.finding("missing_field", None, "system"))
    elif type(system) is not str or not is_plain_chunk(system):
        out.append(link.finding("bad_value", None, "system"))
    if "services" not in doc:
        out.append(link.finding("missing_field", None, "services"))
    elif type(services) is not list or any(type(s) is not str or not is_plain_chunk(s) for s in services) \
            or len(set(services)) != len(services):
        out.append(link.finding("bad_value", None, "services"))
    binding = doc.get("binding")
    if "binding" not in doc:
        out.append(link.finding("missing_field", None, "binding"))
    elif binding not in BINDINGS:
        out.append(link.finding("bad_value", None, "binding"))
    protocols_of = BINDINGS.get(binding) if type(binding) is str else None
    catalog = doc.get("catalog")
    if "catalog" in doc and not (type(catalog) is str and len(catalog) == 8
                                 and all(c in "0123456789abcdef" for c in catalog)):
        out.append(link.finding("bad_value", None, "catalog"))
    transport = _transport(doc, out)
    faces: list[FaceV2] = []
    good: dict[str, list[str]] = {}
    if "face" not in doc:
        out.append(link.finding("missing_field", None, "face"))
    elif type(doc["face"]) is not dict or not doc["face"]:
        out.append(link.finding("bad_value", None, "face"))
    else:
        for name, body in doc["face"].items():
            if not is_plain_chunk(name) or type(body) is not dict:
                out.append(link.finding("bad_value", name, ""))
                continue
            f, protos = _face(name, body, protocols_of, transport, out)
            if protos is not None:
                good[name] = protos
            if f is not None:
                faces.append(f)
    out += link.far_of_two_faces(doc.get("face"))
    names = sorted(good)
    for i, a in enumerate(names):
        for b in names[i + 1:]:
            if set(good[a]) & set(good[b]):
                out += [link.finding("shared_protocol", a, "select.link_protocols"),
                        link.finding("shared_protocol", b, "select.link_protocols")]
    out = link.sort_findings(out)
    if any(f["severity"] == "error" for f in out):
        return None, out
    full = {k: transport[k] for k in TRANSPORT} if transport is not None else None
    return PolicyV2(system, list(services), binding, catalog, full,
                    sorted(faces, key=lambda f: link._bytes(f.face.name)), out), out


def _transport(doc: dict[str, Any], out: list[dict[str, Any]]) -> dict[str, Any] | None:
    """``[transport]``, each key checked alone: the valid keys' values, the
    optional ones absent at their defaults, or None with no table."""
    if "transport" not in doc:
        out.append(link.finding("transport_missing", None, "transport"))
        return None
    t = doc["transport"]
    if type(t) is not dict:
        out.append(link.finding("bad_value", None, "transport"))
        return None
    got: dict[str, Any] = {}
    for k in t:
        if k not in TRANSPORT:
            out.append(link.finding("unknown_field", None, f"transport.{k}"))
    bounds = {"lease_ms": U32_MAX, "keep_alive": 255, "wait_before_close_ms": U32_MAX, "batch_bytes": 65535,
              "open_timeout_ms": U32_MAX, "accept_timeout_ms": U32_MAX, "interests_timeout_ms": U32_MAX}
    for k, (_, required, default) in TRANSPORT.items():
        if k not in t:
            if required:
                out.append(link.finding("missing_field", None, f"transport.{k}"))
            else:
                got[k] = default
            continue
        v = t[k]
        ok = v in SEQUENCES if k == "sequence" else _int_in(v, 1, bounds[k])
        if not ok:
            out.append(link.finding("bad_value", None, f"transport.{k}"))
        else:
            got[k] = v
    # §2.16: "A linter reports wait_under_lease when wait_before_close_ms <
    # lease_ms, and refuses the policy."
    if "lease_ms" in got and "wait_before_close_ms" in got and got["wait_before_close_ms"] < got["lease_ms"]:
        out.append(link.finding("wait_under_lease", None, "transport.wait_before_close_ms"))
    return got


def _face(name: str, body: dict[str, Any], protocols_of: list[str] | None, transport: dict[str, Any] | None,
          out: list[dict[str, Any]]) -> tuple[FaceV2 | None, list[str] | None]:
    before = len(out)

    def bad(at: str, code: str = "bad_value", severity: str = "error") -> None:
        out.append(link.finding(code, name, at, severity))

    for k in body:
        if k in MOVED:
            bad(k, "moved_field")
        elif k not in FACE_KEYS:
            bad(k, "unknown_field")
    far, attach, region = body.get("far"), body.get("attach"), body.get("region")
    if "far" not in body:
        bad("far", "missing_field")
    elif type(far) is not str or not far:
        bad("far")
    if "attach" not in body:
        bad("attach", "missing_field")
    elif attach == "router":
        bad("attach", "router_attach")
    elif attach not in ("client", "south-region"):
        bad("attach")
    if "region" in body and (type(region) is not str or not 1 <= len(link._bytes(region)) <= 32):
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
        elif protocols_of is None:
            # "a binding v2 does not name: bad_value, and the link protocols
            # are not checked against it" (lints.json)
            pass
        elif any(p not in protocols_of for p in lp):
            bad("select.link_protocols", "unknown_protocol")
        else:
            protocols = list(lp)
    if "class" not in body:
        bad("class", "missing_field")
    elif body["class"] != "constrained":
        bad("class")
    expose, interval = body.get("expose"), body.get("interval_s")
    if "expose" not in body:
        bad("expose", "missing_field")
    elif expose not in ("link", "none"):
        bad("expose")
    if "interval_s" in body and not _int_in(interval, 1, U32_MAX):
        bad("interval_s")
    elif expose == "link" and "interval_s" not in body:
        bad("interval_s", "missing_field")
    elif expose == "none" and "interval_s" in body:
        bad("interval_s")
    for k, lo, hi in (("max_outage_s", 1, U32_MAX), ("link_bps", 1, U32_MAX), ("loss_ppm", 0, 1_000_000),
                      ("reply_timeout_ms", 1, U32_MAX)):
        if k not in body:
            bad(k, "missing_field")
        elif not _int_in(body[k], lo, hi):
            bad(k)
    if "arq" not in body:
        bad("arq", "missing_field")
    elif type(body["arq"]) is not bool:
        bad("arq")
    # The checks that read [transport], "made when the keys it reads are
    # valid, whatever else the policy holds".
    t = transport or {}
    bps = body.get("link_bps") if _int_in(body.get("link_bps"), 1, U32_MAX) else None
    loss = body.get("loss_ppm") if _int_in(body.get("loss_ppm"), 0, 1_000_000) else None
    lease, ka, batch, seq = t.get("lease_ms"), t.get("keep_alive"), t.get("batch_bytes"), t.get("sequence")
    if bps is not None and lease is not None and batch is not None and batch * 24_000 > lease * bps:
        bad("transport.batch_bytes", "batch_over_lease", "warning")          # §2.10
    if loss is not None and lease is not None and ka is not None \
            and 2 * 86_400 * 1_000 * ka * loss ** (ka - 1) > lease * 10 ** (6 * (ka - 1)):
        bad("transport.keep_alive", "keep_alive_flaps", "warning")           # §2.16
    if bps is not None and lease is not None and batch is not None and seq is not None \
            and WINDOW[seq] * 8 * batch * 1_000 <= lease * bps:
        bad("transport.sequence", "sequence_window", "warning")              # §2.16
    for k in ("open_timeout_ms", "accept_timeout_ms"):
        if bps is not None and t.get(k) is not None and t[k] * bps < 6_144_000:
            bad(f"transport.{k}", "open_under_handshake", "warning")         # §2.16
    rt = body.get("reply_timeout_ms")
    if bps is not None and batch is not None and _int_in(rt, 1, U32_MAX) and rt * bps < 128_000 * batch:
        bad("reply_timeout_ms", "reply_timeout_short", "warning")            # §2.1
    if body.get("arq") is False:
        bad("arq", "no_arq", "warning")                                      # §2.1
    if any(f["severity"] == "error" for f in out[before:]) or transport is None \
            or not {"lease_ms", "batch_bytes"} <= set(t):
        return None, protocols
    f = link.Face(name, far, attach, region, protocols or [], expose, interval if expose == "link" else None,
                  body["max_outage_s"], t["batch_bytes"], bps, t["lease_ms"])
    return FaceV2(f, loss, body["arq"], rt), protocols


# -- §2.18 the plan --------------------------------------------------------------------------

def plan(policy_text: str, implements: dict[str, list[str]], interfaces: dict[str, Any]) -> dict[str, Any]:
    """``plan.json``'s ``expect``: the binding, the catalog id, the transport
    (None for a ``link.v1`` policy) and each face's ``link.v1`` plan with its
    ``reply_timeout_ms`` (none for a ``link.v1`` face); or ``{refused}``, the
    policy's findings, or the deployment's errors."""
    policy, findings = parse_policy(policy_text)
    if policy is None:
        return {"refused": findings}
    errors = []
    for s in policy.services:
        if not implements.get(s):
            errors.append(link.finding("service_unknown", None, s))
            continue
        errors += [link.finding("contract_unknown", None, i) for i in implements[s] if i not in interfaces]
    if errors:
        return {"refused": link.sort_findings(findings + errors)}
    v1 = policy.as_v1()
    faces = []
    for f in policy.faces:
        fp = link.plan_face(v1, f.face, implements, interfaces)
        if policy.v1 is None:
            fp["reply_timeout_ms"] = f.reply_timeout_ms
        faces.append(fp)
    return {"binding": policy.binding, "catalog": policy.catalog, "transport": policy.transport, "faces": faces}


def narrowed(policy: PolicyV2, principal: str) -> list[str] | None:
    """§2.4 "The narrowing": "A subject bound to a principal P is selected by
    the binding's list, in its order, less the link protocols of every face
    whose far principal is not P"; None when no face excludes any."""
    off = {lp for f in policy.faces if f.face.far != principal for lp in f.face.link_protocols}
    if not off:
        return None
    return [lp for lp in BINDINGS[policy.binding] if lp not in off]


# -- §2.18 the transport block, §2.19 its checks -------------------------------------------------

def emit(policy: PolicyV2) -> list[dict[str, Any]]:
    """§2.18: each key at zenoh's configuration key, ``wait_before_close`` in
    µs, all eight, in the policy's table's order; nothing for a ``link.v1``
    policy."""
    if policy.transport is None:
        return []
    return [{"key": f"transport.{k}", "path": path,
             "value": policy.transport[k] * 1000 if k == "wait_before_close_ms" else policy.transport[k]}
            for k, (path, _, _) in TRANSPORT.items()]


def block(policy: PolicyV2) -> dict[str, Any]:
    """The emitted keys as the router's top-level ``transport`` and
    ``routing`` objects (§2.18)."""
    out: dict[str, Any] = {}
    for e in emit(policy):
        node = out
        parts = e["path"].split("/")
        for p in parts[:-1]:
            node = node.setdefault(p, {})
        node[parts[-1]] = e["value"]
    return out


def _read(config: dict[str, Any], binding: str) -> dict[str, Any]:
    """A configuration's eight keys, by zenoh's key, one it omits at the
    binding's default (§2.19's table); values as stated, never converted."""
    d = DEFAULTS[binding]
    return {k: config.get(path, d[k]) for k, (path, _, _) in TRANSPORT.items()}


def _same(got: Any, want: Any) -> bool:
    """A configured value equal to the policy's, of its type: a fraction or a
    string is no integer zenoh runs."""
    return type(got) is type(want) and got == want


def check_transport(policy_text: str, router: dict[str, Any], peer: dict[str, Any] | None = None,
                    catalog_doc: Any = None) -> dict[str, Any]:
    """``transport.json``'s ``expect``: what a generator emits, and every
    finding of the router's configuration (§2.19)."""
    import hashlib

    from . import jcs

    policy, findings = parse_policy(policy_text)
    if policy is None:
        return {"emit": [], "findings": findings}
    if policy.transport is None:
        return {"emit": [], "findings": []}
    out: list[dict[str, Any]] = []
    emitted = {e["key"].removeprefix("transport."): e["value"] for e in emit(policy)}
    got = _read(router, policy.binding)
    for k in TRANSPORT:
        if not _same(got[k], emitted[k]):
            out.append(link.finding("transport_differs", None, f"transport.{k}"))
    lease, wait = got["lease_ms"], got["wait_before_close_ms"]
    if type(lease) is int and type(wait) is int and wait < lease * 1000:
        out.append(link.finding("wait_under_lease", None, "transport.wait_before_close_ms"))
    if peer is not None:
        theirs = _read(peer, policy.binding)
        for k in ("lease_ms", "keep_alive"):
            if not _same(theirs[k], got[k]):
                out.append(link.finding("peer_differs", None, f"transport.{k}", "warning"))
    if catalog_doc is not None:
        # §2.19: "The id is the first 4 bytes of the sha256 of the catalog
        # document's JCS bytes (RFC 8785)". transport.json's documents are no
        # catalog core §8.6 reads (an entry's `fingerprint`, an empty
        # lineage), so the id is the JCS bytes' alone, the document read by
        # no rule of §8.6 (F-450); one §8.6 accepts has the same id.
        held = hashlib.sha256(jcs.dumps(catalog_doc)).hexdigest()[:8]
        if policy.catalog is None or held != policy.catalog:
            out.append(link.finding("catalog_mismatch", None, "catalog"))
    return {"emit": emit(policy), "findings": link.sort_findings(out)}


def lint(policy_text: str) -> list[dict[str, Any]]:
    """``lints.json``'s reading: every finding of the policy."""
    return parse_policy(policy_text)[1]

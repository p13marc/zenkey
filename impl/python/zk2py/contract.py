"""Contracts: the authoring format, the lints, resolution, the canonical form
and the fingerprint (core.md §9.1–§9.5, Appendix D).

``load_contract(path)`` returns every diagnostic (§9.2: "exactly the codes
… sorted, with repeats") and, for a valid contract, the canonical form, its
JCS bytes and its fingerprint.
"""

from __future__ import annotations

import datetime
import re
import tomllib
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

from . import jcs
from .lexical import GATE_NAME, IDENT, is_interface_id, is_interface_name, parse_interface_id
from .schemas import JSON, SchemaSet
from .shape import Checker, load_schema
from .templates import REST, Template, TemplateError, overlap, parse_template

FORMAT = "zk2-contract/draft-1"
KINDS = ("stream", "state", "event", "operation")
TYPE_FIELDS = ("type", "attachment", "request", "response", "error", "summary")

# -- Appendix D: authoring fields by kind -----------------------------------

_COMMON = {"kind", "doc", "params", "cardinality", "epoch", "optional", "gate",
           "deprecated", "annotations"}
_PUBLISHED = {"type", "attachment", "encoding", "attachment_encoding",
              "reliability", "congestion", "priority", "express"}
#: Fields each kind takes. ``history`` is handled apart (E019, not E014).
LEGAL = {
    "stream": _COMMON | _PUBLISHED | {"explicit"},
    "state": _COMMON | _PUBLISHED | {"explicit"},
    "event": _COMMON | _PUBLISHED | {"rate", "retention"},
    "operation": _COMMON | {"encoding", "priority", "request", "response", "error", "summary",
                            "idempotent", "fanout", "serving", "replies", "timeout_ms"},
}
REQUIRED = {
    "stream": ("type",),
    "state": ("type",),
    "event": ("type", "rate", "retention"),
    "operation": ("request", "response"),
}

#: Appendix D, interim annotation vocabularies (W105 outside them).
VOCABULARY = {
    "freshness": {"ttl_s"},
    "timing": {"period_ms", "deadline_ms", "lifespan_ms"},
    "telemetry": {"unit", "kind", "buckets", "semantic"},
    "link": {"exposure", "downsample_ms"},
    "arbitration": {"policy"},
    "desired": {"target_param", "target"},
    "alarms": {"severity_default"},
    "media": {"tiers", "tier_param", "frame_clock", "control", "receiver_report"},
    "views": {"document"},
    "redundancy": {"election"},
}

# -- §2.4 / §9.3: built-in defaults -----------------------------------------

#: §2.4's QoS table, by kind token.
QOS_DEFAULTS = {
    "stream": {"reliability": "best_effort", "congestion": "drop", "priority": "data", "express": False},
    "@stream": {"reliability": "best_effort", "congestion": "drop", "priority": "data_low", "express": False},
    "state": {"reliability": "reliable", "congestion": "block", "priority": "data", "express": False},
    "@state": {"reliability": "reliable", "congestion": "block", "priority": "data", "express": False},
    "events": {"reliability": "reliable", "congestion": "block", "priority": "data", "express": False},
}
#: §9.3, operations.
OP_DEFAULTS = {"idempotent": False, "fanout": "forbidden", "serving": "exclusive",
               "replies": "one", "timeout_ms": None, "priority": "interactive_high"}

_RATE = re.compile(r"rare|low|burst\(([1-9][0-9]*)/h\)")
_RETENTION = re.compile(r"([0-9]+)([smhdw])")
_RETENTION_UNIT = {"s": 1, "m": 60, "h": 3600, "d": 86400, "w": 604800}
_GATE = re.compile(r"(build|config|capability):(.*)")


def kind_token(kind: str, explicit: bool) -> str:
    """core.md §1.3 / §2.1: the key token of a resource."""
    if kind == "stream":
        return "@stream" if explicit else "stream"
    if kind == "state":
        return "@state" if explicit else "state"
    if kind == "event":
        return "events"
    return "@op"


# -- results ----------------------------------------------------------------

@dataclass
class Diagnostic:
    code: str
    message: str
    resource: str | None = None  # the template, when the finding is a resource's

    @property
    def is_error(self) -> bool:
        return self.code.startswith("E")


@dataclass
class Contract:
    path: Path
    diagnostics: list[Diagnostic] = field(default_factory=list)
    interface: str | None = None
    minor: int | None = None
    raw: dict[str, Any] | None = None
    schemas: SchemaSet | None = None
    canonical: dict[str, Any] | None = None
    canonical_bytes: bytes | None = None
    fingerprint: str | None = None
    #: template -> resolved type objects, for resources that resolved
    types: dict[str, dict[str, Any]] = field(default_factory=dict)

    @property
    def codes(self) -> list[str]:
        return sorted(d.code for d in self.diagnostics)

    @property
    def valid(self) -> bool:
        return not any(d.is_error for d in self.diagnostics)

    @property
    def templates(self) -> list[str]:
        return list((self.raw or {}).get("resources", {}))


# -- helpers ----------------------------------------------------------------

def _holds_datetime(v: Any) -> bool:
    if isinstance(v, (datetime.datetime, datetime.date, datetime.time)):
        return True
    if isinstance(v, dict):
        return any(_holds_datetime(x) for x in v.values())
    if isinstance(v, list):
        return any(_holds_datetime(x) for x in v)
    return False


def _beyond_i64(v: Any) -> bool:
    if isinstance(v, int) and not isinstance(v, bool):
        return not -(2**63) <= v < 2**63
    if isinstance(v, dict):
        return any(_beyond_i64(x) for x in v.values())
    if isinstance(v, list):
        return any(_beyond_i64(x) for x in v)
    return False


def _gates(value: Any) -> list[str]:
    if value is None:
        return []
    return [value] if isinstance(value, str) else list(value)


def _history(value: Any) -> dict[str, Any] | None:
    """§9.3: ``true`` means ``{depth = 1}``, ``false`` means none."""
    if value is None or value is False:
        return None
    if value is True:
        return {"depth": 1, "miss_detection_ms": None}
    return {"depth": value["depth"], "miss_detection_ms": value.get("miss_detection_ms")}


class _Loader:
    def __init__(self, path: Path, check_file_name: bool, spec_dir: Path | None):
        self.c = Contract(path)
        self.check_file_name = check_file_name
        self.spec_dir = spec_dir

    def diag(self, code: str, message: str, resource: str | None = None) -> None:
        self.c.diagnostics.append(Diagnostic(code, message, resource))

    # -- §9.1: TOML and shape -------------------------------------------

    def parse(self) -> dict[str, Any] | None:
        try:
            text = self.c.path.read_bytes().decode("utf-8")
            doc = tomllib.loads(text)
        except (UnicodeDecodeError, tomllib.TOMLDecodeError) as e:
            self.diag("E000", f"not TOML: {e}")
            return None
        if _beyond_i64(doc):
            # TOML 1.0: "If an integer cannot be represented losslessly [as a
            # 64-bit signed integer], an error must be thrown." Python's
            # tomllib does not; this reader does (SPEC-FINDINGS F-16).
            self.diag("E000", "not TOML 1.0: an integer beyond 64 bits")
            return None
        errors = Checker(load_schema("contract.schema.json", self.spec_dir)).errors(doc)
        if errors:
            # §9.1: E000 is "reported once, and it stops the load".
            self.diag("E000", "outside the format's shape: " + "; ".join(errors[:5]))
            return None
        return doc

    # -- §9.2 lints -----------------------------------------------------

    def run(self) -> Contract:
        doc = self.parse()
        if doc is None:
            return self.c
        self.c.raw = doc
        iface = doc["interface"]
        name, major = iface["name"], iface["major"]
        self.c.minor = iface.get("minor")
        self.c.interface = f"{name}.v{major}"
        # E001: the interface name (§1.2). The major's range is the shape's
        # (uint32, E000).
        if not is_interface_name(name):
            self.diag("E001", f"interface name {name!r}")
        # E002: each `uses` entry is <name>.v<major>.
        self.uses = []
        for u in iface.get("uses", []):
            pid = parse_interface_id(u)
            if pid is None:
                self.diag("E002", f"uses entry {u!r} is not <name>.v<major>")
            else:
                self.uses.append(pid)
        self.profile_names = {u.name for u in self.uses}
        # W104: minor absent.
        if self.c.minor is None:
            self.diag("W104", "[interface] minor is absent")

        sch = doc.get("schemas", {})
        self.schemas = SchemaSet.load(self.c.path.parent, sch.get("jsonschema", []),
                                      sch.get("protobuf", []), sch.get("proto_include"))
        self.c.schemas = self.schemas
        for f in self.schemas.findings:
            self.diag(f.code, f.message)

        self.defaults = doc.get("defaults") or {}
        self.check_defaults()
        resources = doc.get("resources", {})
        self.parsed: dict[str, Template] = {}
        self.tokens: dict[str, str] = {}
        for text, spec in resources.items():
            self.check_resource(text, spec, resources)
        self.check_shapes()
        self.check_second_epochs(resources)
        for role, req in doc.get("requires", {}).items():
            self.check_requirement(role, req)
        self.check_overlaps(resources)

        if self.check_file_name and self.c.path.name != f"{self.c.interface}.toml":
            self.diag("W107", f"file is not named {self.c.interface}.toml")

        if self.c.valid:
            self.canonicalize(doc)
        return self.c

    def check_annotations(self, table: dict[str, Any], where: str, resource: str | None = None) -> None:
        """E020 and W105, once per key of one table (§9.2). A key is
        ``<profile>.<key>`` split at the last dot; the profile is a ``uses``
        name without its major; the value holds no datetime (§9.1)."""
        for k, v in table.items():
            profile, dot, key = k.rpartition(".")
            if not dot or not profile or IDENT.fullmatch(key) is None:
                self.diag("E020", f"{where}: annotation key {k!r} is not <profile>.<key>", resource)
                continue
            if profile not in self.profile_names:
                self.diag("E020", f"{where}: profile {profile!r} is not in uses", resource)
                continue
            if _holds_datetime(v):
                self.diag("E020", f"{where}: annotation {k!r} holds a datetime", resource)
                continue
            vocab = VOCABULARY.get(profile)
            if vocab is not None and key not in vocab:
                self.diag("W105", f"{where}: {k!r} is outside the interim vocabulary", resource)

    def check_history_depth(self, value: Any, where: str, resource: str | None = None) -> None:
        if isinstance(value, dict) and value.get("depth") == 0:
            self.diag("E034", f"{where}: history.depth is 0", resource)

    def check_defaults(self) -> None:
        d = self.defaults
        # [defaults]: every defaultable field is allowed here; a default
        # applies only where its field is legal (§9.3).
        self.check_history_depth(d.get("history"), "[defaults]")
        self.check_annotations(d.get("annotations", {}), "[defaults]")
        for kind in KINDS:
            block = d.get(kind)
            if block is None:
                continue
            where = f"[defaults.{kind}]"
            for f in block:
                if f == "history":
                    if kind in ("event", "operation"):
                        self.diag("E019", f"{where}: history on {kind}")
                elif f != "annotations" and f not in LEGAL[kind]:
                    self.diag("E014", f"{where}: {f} is not a field of {kind}")
            self.check_history_depth(block.get("history"), where)
            self.check_annotations(block.get("annotations", {}), where)

    def resolved(self, kind: str, spec: dict[str, Any], f: str, builtin: Any) -> Any:
        """§9.3: the resource, then [defaults.<kind>], then [defaults], then
        the built-in default, field by field."""
        if f in spec:
            return spec[f]
        block = self.defaults.get(kind) or {}
        if f in block:
            return block[f]
        if f in self.defaults:
            return self.defaults[f]
        return builtin

    def check_resource(self, text: str, spec: dict[str, Any], resources: dict[str, Any]) -> None:
        kind = spec["kind"]
        # E031, replaced_by half: over every template, whatever else is wrong
        # with its resource (§9.2 cascades 1 and 3).
        dep = spec.get("deprecated")
        if dep is not None and dep.get("replaced_by") is not None:
            rb = dep["replaced_by"]
            if rb == text or rb not in resources:
                self.diag("E031", f"{text}: replaced_by {rb!r} names no other template", text)
        # E010 stops the resource's other checks (§9.2 cascade 1).
        try:
            tpl = parse_template(text)
        except TemplateError as e:
            self.diag("E010", f"{text!r}: {e}", text)
            return
        self.parsed[text] = tpl
        explicit = spec.get("explicit") is True and kind in ("stream", "state")
        token = kind_token(kind, explicit)
        self.tokens[text] = token

        # E014 / E019: field legality (§9.1, Appendix D).
        for f in spec:
            if f == "history":
                if kind not in ("stream", "state") or explicit:
                    self.diag("E019", f"{text}: history on {token}", text)
            elif f not in LEGAL[kind]:
                self.diag("E014", f"{text}: {f} is not a field of {kind}", text)
        tparams = tpl.param_names
        if "cardinality" in spec and not tparams:
            self.diag("E014", f"{text}: cardinality without parameters", text)
        # E015: required fields.
        for f in REQUIRED[kind]:
            if f not in spec:
                self.diag("E015", f"{text}: {f} is required on {kind}", text)
        # E011 / E012: parameters (§2.2).
        params = spec.get("params", {})
        rest = {s.text for s in tpl.segments if s.kind == REST}
        for p in tparams:
            if p not in params:
                self.diag("E011", f"{text}: parameter {p!r} missing from params", text)
        for p, ptype in params.items():
            if p not in tparams:
                self.diag("E011", f"{text}: params entry {p!r} is not in the template", text)
            elif (ptype == "path") != (p in rest):
                self.diag("E012", f"{text}: parameter {p!r} of type {ptype}", text)
        # E013: cardinality on a template with parameters.
        if tparams and not spec.get("cardinality"):
            self.diag("E013", f"{text}: cardinality is required and positive", text)
        # E016 / E017: gates (§2.3).
        gates = _gates(spec.get("gate"))
        if "gate" in spec and spec.get("optional") is not True:
            self.diag("E016", f"{text}: gate without optional = true", text)
        for g in gates:
            m = _GATE.fullmatch(g)
            if m is None or GATE_NAME.fullmatch(m.group(2)) is None:
                self.diag("E017", f"{text}: gate {g!r}", text)
        # E022, first condition: epoch names a single-chunk parameter.
        if "epoch" in spec and spec["epoch"] not in tpl.single_chunk_params():
            self.diag("E022", f"{text}: epoch {spec['epoch']!r} is not a single-chunk parameter", text)
        # E026: rate and retention (§2.6).
        if "rate" in spec and _RATE.fullmatch(spec["rate"]) is None:
            self.diag("E026", f"{text}: rate {spec['rate']!r}", text)
        if "retention" in spec:
            m = _RETENTION.fullmatch(spec["retention"])
            if m is None or int(m.group(1)) < 1:
                self.diag("E026", f"{text}: retention {spec['retention']!r}", text)
        # E034: history.depth 0 where history is legal (E019 alone otherwise).
        if kind in ("stream", "state") and not explicit:
            self.check_history_depth(spec.get("history"), text, text)
        # E020 / W105.
        self.check_annotations(spec.get("annotations", {}), text, text)
        # E031, since half: only when minor is present.
        if dep is not None and self.c.minor is not None and dep["since"] > self.c.minor:
            self.diag("E031", f"{text}: deprecated.since {dep['since']} > minor {self.c.minor}", text)

        # Types: E023 / E024 (§9.4), E025 (media_param).
        resolved: dict[str, Any] = {}
        for f in TYPE_FIELDS:
            if f in spec and f in LEGAL[kind]:
                obj, findings = self.schemas.resolve(spec[f])
                for fd in findings:
                    self.diag(fd.code, f"{text}: {f}: {fd.message}", text)
                if obj is not None:
                    resolved[f] = obj
                    mp = obj.get("media_param") if obj["kind"] == "raw" else None
                    if mp is not None and mp not in tpl.single_chunk_params():
                        self.diag("E025", f"{text}: media_param {mp!r} is not a single-chunk parameter", text)
        self.c.types[text] = resolved

        if kind == "operation":
            # E018 / E033 on resolved values (SPEC-FINDINGS F-22).
            serving = self.resolved(kind, spec, "serving", "exclusive")
            idempotent = self.resolved(kind, spec, "idempotent", False)
            if serving == "replicated" and idempotent is not True:
                self.diag("E018", f"{text}: replicated serving without idempotent = true", text)
            replies = self.resolved(kind, spec, "replies", "one")
            if "summary" in spec and replies != "many":
                self.diag("E033", f"{text}: summary without replies = \"many\"", text)
            # W102: a request field named like a template parameter.
            req = resolved.get("request")
            if req is not None and set(self.request_fields(req)) & set(tparams):
                self.diag("W102", f"{text}: request repeats a template parameter", text)
        # W103 judges only types that resolved: an unresolved one is already
        # an error, and whether a JSON Schema type "takes" the encoding is then
        # unknown.
        unresolved = any(f in spec and f in LEGAL[kind] and f not in resolved for f in TYPE_FIELDS)
        if kind == "operation":
            json_any = any(resolved.get(f, {}).get("kind") == JSON
                           for f in ("request", "response", "error", "summary"))
            if "encoding" in spec and not json_any and not unresolved:
                self.diag("W103", f"{text}: encoding on an operation with no JSON Schema type", text)
        elif not unresolved:
            if "encoding" in spec and resolved.get("type", {}).get("kind") != JSON:
                self.diag("W103", f"{text}: encoding on a non-JSON-Schema type", text)
            if "attachment_encoding" in spec and resolved.get("attachment", {}).get("kind") != JSON:
                self.diag("W103", f"{text}: attachment_encoding without a JSON Schema attachment", text)

    def request_fields(self, req: dict[str, Any]) -> list[str]:
        if req["kind"] == JSON:
            hit = self.schemas.json_definition(req)
            if hit is None:
                return []
            art, node = hit
            if isinstance(node, dict) and "$ref" in node:
                target = self.schemas.resolve_ref(art, node["$ref"])
                node = target[1] if target else {}
            props = node.get("properties", {}) if isinstance(node, dict) else {}
            return list(props) if isinstance(props, dict) else []
        if req["kind"] == "protobuf":
            return self.schemas.proto_fields(req)
        return []

    def check_shapes(self) -> None:
        """E021: two templates under one kind token with the same shape; once
        per template after the first of its shape (§2.2, §9.2)."""
        seen: set[tuple[str, str]] = set()
        for text, tpl in self.parsed.items():
            key = (self.tokens[text], tpl.shape)
            if key in seen:
                self.diag("E021", f"{text}: shape {tpl.shape!r} already taken under {key[0]}", text)
            seen.add(key)

    def check_second_epochs(self, resources: dict[str, Any]) -> None:
        """E022, second condition, over the raw resources table: once per
        ``epoch`` template after the first (§8.1, §9.2)."""
        with_epoch = [t for t, s in resources.items() if "epoch" in s]
        for t in with_epoch[1:]:
            self.diag("E022", f"{t}: a second template declares epoch", t)

    def check_requirement(self, role: str, req: dict[str, Any]) -> None:
        """E030 (§3.1, §9.2), and the requirement's annotations (E020/W105)."""
        if IDENT.fullmatch(role) is None:
            self.diag("E030", f"requires.{role}: role name")
        if not is_interface_id(req["interface"]):
            self.diag("E030", f"requires.{role}: interface {req['interface']!r}")
        res = req.get("resources")
        if res is not None:
            if res == []:
                self.diag("E030", f"requires.{role}: resources = []")
            for r in res:
                try:
                    parse_template(r)
                except TemplateError:
                    self.diag("E030", f"requires.{role}: {r!r} is not a template")
        self.check_annotations(req.get("annotations", {}), f"requires.{role}")

    def check_overlaps(self, resources: dict[str, Any]) -> None:
        """W101: two resolved resources under one token whose templates
        overlap, with different types; per pair, over resources without
        errors (§9.2 cascade 4)."""
        bad = {d.resource for d in self.c.diagnostics if d.is_error and d.resource}
        ok = [t for t in self.parsed if t not in bad]

        def types_of(t: str) -> Any:
            r = self.c.types.get(t, {})
            if resources[t]["kind"] == "operation":
                return (r.get("request"), r.get("response"))
            return r.get("type")

        for i, a in enumerate(ok):
            for b in ok[i + 1:]:
                if self.tokens[a] != self.tokens[b]:
                    continue
                if overlap(self.parsed[a], self.parsed[b]) and types_of(a) != types_of(b):
                    self.diag("W101", f"{a!r} and {b!r} overlap with different types", a)

    # -- §9.3 / §9.5: resolution and the canonical form ----------------------

    def canonical_resource(self, text: str, spec: dict[str, Any]) -> dict[str, Any]:
        kind = spec["kind"]
        token = self.tokens[text]
        types = self.c.types[text]
        dep = spec.get("deprecated")
        ann: dict[str, Any] = {}
        ann.update(self.defaults.get("annotations", {}))
        ann.update((self.defaults.get(kind) or {}).get("annotations", {}))
        ann.update(spec.get("annotations", {}))
        out: dict[str, Any] = {
            "token": token,
            "template": text,
            "kind": kind,
            "params": dict(spec.get("params", {})),
            "cardinality": spec.get("cardinality"),
            "epoch": spec.get("epoch"),
            "optional": spec.get("optional", False),
            "gate": sorted(set(_gates(spec.get("gate")))),
            "deprecated": None if dep is None else {
                "since": dep["since"],
                "replaced_by": dep.get("replaced_by"),
                "reason": dep.get("reason"),
            },
            "annotations": ann,
        }
        if kind == "operation":
            json_any = any(types.get(f, {}).get("kind") == JSON
                           for f in ("request", "response", "error", "summary"))
            out.update({
                "request": types["request"],
                "response": types["response"],
                "error": types.get("error"),
                "summary": types.get("summary"),
                "encoding": self.resolved(kind, spec, "encoding", "json") if json_any else None,
            })
            for f, builtin in OP_DEFAULTS.items():
                out[f] = self.resolved(kind, spec, f, builtin)
            return out
        qos = QOS_DEFAULTS[token]
        is_json = types["type"]["kind"] == JSON
        att = types.get("attachment")
        att_json = att is not None and att["kind"] == JSON
        history = None
        if token in ("stream", "state"):
            history = _history(self.resolved(kind, spec, "history", None))
        rate = spec.get("rate") if kind == "event" else None
        retention_s = None
        if kind == "event":
            m = _RETENTION.fullmatch(spec["retention"])
            assert m is not None
            retention_s = int(m.group(1)) * _RETENTION_UNIT[m.group(2)]
        out.update({
            "type": types["type"],
            "attachment": att,
            "encoding": self.resolved(kind, spec, "encoding", "json") if is_json else None,
            "attachment_encoding": (self.resolved(kind, spec, "attachment_encoding", "json")
                                    if att_json else None),
            "reliability": self.resolved(kind, spec, "reliability", qos["reliability"]),
            "congestion": self.resolved(kind, spec, "congestion", qos["congestion"]),
            "priority": self.resolved(kind, spec, "priority", qos["priority"]),
            "express": self.resolved(kind, spec, "express", qos["express"]),
            "history": history,
            "rate": rate,
            "retention_s": retention_s,
        })
        return out

    def canonicalize(self, doc: dict[str, Any]) -> None:
        resources = [self.canonical_resource(t, s) for t, s in doc.get("resources", {}).items()]
        resources.sort(key=lambda r: (r["token"], r["template"]))
        uses = sorted({(u.name, u.major) for u in self.uses})
        requires = {}
        for role, req in doc.get("requires", {}).items():
            res = req.get("resources")
            requires[role] = {
                "interface": req["interface"],
                "resources": None if res is None else sorted(set(res)),
                "cardinality": req.get("cardinality", "one"),
                "optional": req.get("optional", False),
                "annotations": dict(req.get("annotations", {})),
            }
        arts = sorted(self.schemas.artifacts(), key=lambda a: a.id or "")
        canonical = {
            "format": FORMAT,
            "interface": self.c.interface,
            "uses": [f"{n}.v{m}" for n, m in uses],
            "schemas": [{"id": a.id, "kind": a.kind, "name": a.name} for a in arts],
            "resources": resources,
            "requires": requires,
        }
        # §9.5 restrictions: E027 / E028, per value, only when no other error.
        ascii_bad, int_bad = jcs.restriction_violations(canonical)
        for _ in range(ascii_bad):
            self.diag("E027", "a string outside printable ASCII in the canonical form")
        for _ in range(int_bad):
            self.diag("E028", "an integer outside ±(2^53−1) in the canonical form")
        if ascii_bad or int_bad:
            return
        try:
            data = jcs.dumps(canonical)
        except Exception as e:  # noqa: BLE001 - a float JCS cannot write (nan, inf)
            self.diag("E028", f"the canonical form has no JCS bytes: {e}")
            return
        self.c.canonical = canonical
        self.c.canonical_bytes = data
        self.c.fingerprint = jcs.sha256_id(data)


def load_contract(path: str | Path, *, check_file_name: bool = False,
                  spec_dir: Path | None = None) -> Contract:
    """Load one contract file and run every per-file lint (§9.1–§9.5).

    ``check_file_name`` enables W107; the fixtures load without it.
    """
    return _Loader(Path(path), check_file_name, spec_dir).run()

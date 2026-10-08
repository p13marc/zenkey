"""The compatibility classifier (core.md §9.8) and the retention identity
check (core.md §9.7).

§9.8: "Revisions inside a major are checked FULL_TRANSITIVE: a candidate
against every revision in the history, in both directions." A change is
*compatible*, *review* or *breaking*; "the candidate's class is the worst
over both directions and every earlier revision"; a revision that does not
load is *invalid*.

How this module reads "both directions" (SPEC-FINDINGS: both directions):
each rule of §9.8 classifies a transition *from an earlier revision to the
candidate*, and its class already accounts for both reader/writer roles
(the JSON rules say so: "a tightened maximum breaks old writers, and a
loosened maxLength breaks old readers"). Applying the contract table in
reverse too would make ``explicit`` true → false breaking, where
``compat/expect.json`` says review.

A change §9.8 does not list is classed **review** here: a human looks at it
before publication. Every such guess is listed in SPEC-FINDINGS.
"""

from __future__ import annotations

import json
import posixpath
from dataclasses import dataclass, field
from typing import Any, Callable

from google.protobuf import descriptor_pb2

from . import protoc
from .schemas import ANNOTATIONS, JSON, PROTOBUF, RAW, json_pointer, stem

COMPATIBLE, REVIEW, BREAKING, INVALID = "compatible", "review", "breaking", "invalid"
_ORDER = {COMPATIBLE: 0, REVIEW: 1, BREAKING: 2, INVALID: 3}
FIELD_DELETED_UNRESERVED = "field_deleted_unreserved"


def worst(*classes: str) -> str:
    return max(classes, key=_ORDER.__getitem__, default=COMPATIBLE)


@dataclass
class Verdict:
    cls: str = COMPATIBLE
    warnings: list[str] = field(default_factory=list)
    reasons: list[str] = field(default_factory=list)

    def add(self, cls: str, reason: str) -> None:
        self.cls = worst(self.cls, cls)
        if cls != COMPATIBLE:
            self.reasons.append(f"{cls}: {reason}")

    def merge(self, other: Verdict, prefix: str = "") -> None:
        self.cls = worst(self.cls, other.cls)
        self.warnings += other.warnings
        self.reasons += [f"{prefix}{r}" for r in other.reasons]


# ===========================================================================
# JSON Schema payloads (§9.8, over the §7.3 subset)
# ===========================================================================

#: §7.3 bounds; §9.8: "a bound changed … breaking".
BOUNDS = ("minimum", "maximum", "exclusiveMinimum", "exclusiveMaximum",
          "minLength", "maxLength", "minItems", "maxItems")


@dataclass
class JsonWorld:
    """The JSON Schema documents of one revision, by stem.

    A ``$ref``'s file part is resolved by the stem of its last path
    component. In a source tree §9.4 resolves it as a path; in a bundle only
    stems exist, and §9.4's stem uniqueness (E024) makes the stem enough
    (SPEC-FINDINGS: $ref inside a bundle).
    """

    docs: dict[str, Any]

    def deref(self, where: str, node: Any, seen: frozenset = frozenset()) -> tuple[str, Any]:
        """Follow a node that is only a ``$ref`` (plus annotations)."""
        while isinstance(node, dict) and "$ref" in node and set(node) - ANNOTATIONS <= {"$ref"}:
            ref = node["$ref"]
            if (where, ref) in seen:
                break
            seen = seen | {(where, ref)}
            file_part, _, frag = ref.partition("#")
            target = stem(posixpath.basename(file_part)) if file_part else where
            found, nxt = json_pointer(self.docs.get(target), frag)
            if not found:
                break
            where, node = target, nxt
        return where, node

    def normalize(self, where: str, node: Any, depth: int = 0) -> Any:
        """A comparable form: refs inlined (bounded), annotations dropped."""
        where, node = self.deref(where, node)
        if depth > 32 or not isinstance(node, dict):
            return node
        out: dict[str, Any] = {}
        for k, v in node.items():
            if k in ANNOTATIONS:
                continue
            if k in ("properties",) and isinstance(v, dict):
                out[k] = {p: self.normalize(where, s, depth + 1) for p, s in v.items()}
            elif k in ("items", "additionalProperties") and isinstance(v, dict):
                out[k] = self.normalize(where, v, depth + 1)
            elif k in ("prefixItems", "oneOf", "anyOf") and isinstance(v, list):
                out[k] = [self.normalize(where, s, depth + 1) for s in v]
            else:
                out[k] = v
        return out


def _canon(v: Any) -> str:
    return json.dumps(v, sort_keys=True, separators=(",", ":"))


def _multiset(items: list[Any]) -> list[str]:
    return sorted(_canon(x) for x in items)


def _types(v: Any) -> frozenset[str] | None:
    if v is None:
        return None
    return frozenset([v] if isinstance(v, str) else v)


_MISSING = object()


def json_compare(old: JsonWorld, old_where: str, old_node: Any,
                 new: JsonWorld, new_where: str, new_node: Any,
                 path: str = "", seen: set | None = None) -> Verdict:
    """Classify one JSON Schema node's change (§9.8 "JSON Schema payloads")."""
    v = Verdict()
    seen = set() if seen is None else seen
    ow, o = old.deref(old_where, old_node)
    nw, n = new.deref(new_where, new_node)
    key = (ow, id(o), nw, id(n))
    if key in seen:
        return v
    seen.add(key)
    if not isinstance(o, dict) or not isinstance(n, dict):
        if old.normalize(ow, o) != new.normalize(nw, n):
            v.add(BREAKING, f"{path or '/'}: a boolean schema changed")
        return v
    if "$ref" in o or "$ref" in n:
        # A $ref beside other constraints: containment is not decided here.
        if old.normalize(ow, o) != new.normalize(nw, n):
            v.add(REVIEW, f"{path or '/'}: a $ref beside other keywords changed")
        return v

    def get(d: dict[str, Any], k: str) -> Any:
        return d.get(k, _MISSING)

    # type: "integer ↔ number" and any other type change.
    if _types(o.get("type")) != _types(n.get("type")):
        v.add(BREAKING, f"{path or '/'}: type {o.get('type')!r} → {n.get('type')!r}")
    # enum: "an enum value added or removed"; const likewise.
    if "enum" in o or "enum" in n:
        if _multiset(o.get("enum", [])) != _multiset(n.get("enum", [])) or ("enum" in o) != ("enum" in n):
            v.add(BREAKING, f"{path or '/'}: enum changed")
    if get(o, "const") != get(n, "const") or ("const" in o) != ("const" in n):
        v.add(BREAKING, f"{path or '/'}: const changed")
    # "a bound changed: a tightened maximum breaks old writers, and a
    # loosened maxLength breaks old readers" — either way, breaking.
    for b in BOUNDS:
        if (b in o) != (b in n) or o.get(b) != n.get(b):
            v.add(BREAKING, f"{path or '/'}: {b} {o.get(b)!r} → {n.get(b)!r}")

    # properties and required.
    op, np_ = o.get("properties", {}), n.get("properties", {})
    oreq, nreq = set(o.get("required", [])), set(n.get("required", []))
    for name in sorted(set(op) & set(np_)):
        p = f"{path}/{name}"
        if (name in oreq) != (name in nreq):
            v.add(BREAKING, f"{p}: {'optional → required' if name in nreq else 'required → optional'}")
        v.merge(json_compare(old, ow, op[name], new, nw, np_[name], p, seen))
    for name in sorted(set(np_) - set(op)):
        # "Compatible: an optional property added, even to a closed schema";
        # "Breaking: a required property added".
        if name in nreq:
            v.add(BREAKING, f"{path}/{name}: a required property added")
    for name in sorted(set(op) - set(np_)):
        # "Compatible: an optional property … removed". A required one
        # removed is not listed; it breaks new writers → old readers
        # (SPEC-FINDINGS: unlisted JSON Schema changes).
        if name in oreq:
            v.add(BREAKING, f"{path}/{name}: a required property removed")
    loose = (oreq ^ nreq) - set(op) - set(np_)
    if loose:
        v.add(BREAKING, f"{path or '/'}: required names {sorted(loose)} changed")

    # additionalProperties: "Readers tolerate unknown properties, and
    # writers send only what their schema declares": open/closed toggles are
    # compatible; a map's value schema is compared.
    oa, na = o.get("additionalProperties", True), n.get("additionalProperties", True)
    if isinstance(oa, dict) and isinstance(na, dict):
        v.merge(json_compare(old, ow, oa, new, nw, na, f"{path}/additionalProperties", seen))
    elif isinstance(oa, dict) != isinstance(na, dict):
        v.add(REVIEW, f"{path or '/'}: additionalProperties changed between a schema and a boolean")

    # items.
    oi, ni = o.get("items", True), n.get("items", True)
    if oi != ni or isinstance(oi, dict):
        if isinstance(oi, dict) and isinstance(ni, dict):
            v.merge(json_compare(old, ow, oi, new, nw, ni, f"{path}/items", seen))
        elif old.normalize(ow, oi) != new.normalize(nw, ni):
            v.add(BREAKING, f"{path or '/'}: items changed")

    # oneOf / anyOf / prefixItems: "a oneOf branch added" is breaking; "any
    # other change inside oneOf, anyOf or prefixItems" is review.
    for kw in ("oneOf", "anyOf", "prefixItems"):
        ob = [old.normalize(ow, s) for s in o.get(kw, [])]
        nb = [new.normalize(nw, s) for s in n.get(kw, [])]
        same = (_canon(ob) == _canon(nb)) if kw == "prefixItems" else (_multiset(ob) == _multiset(nb))
        if same and (kw in o) == (kw in n):
            continue
        if kw == "oneOf" and kw in o and kw in n and _is_superset(nb, ob):
            v.add(BREAKING, f"{path or '/'}: a oneOf branch added")
        else:
            v.add(REVIEW, f"{path or '/'}: a change inside {kw}")

    # Anything else that differs (a keyword this module does not judge).
    judged = {"type", "enum", "const", *BOUNDS, "properties", "required",
              "additionalProperties", "items", "oneOf", "anyOf", "prefixItems"}
    for k in sorted((set(o) | set(n)) - judged - ANNOTATIONS):
        if _canon(o.get(k)) != _canon(n.get(k)):
            v.add(REVIEW, f"{path or '/'}: {k} changed")
    return v


def _is_superset(new: list[Any], old: list[Any]) -> bool:
    rest = _multiset(new)
    for x in _multiset(old):
        if x not in rest:
            return False
        rest.remove(x)
    return len(rest) > 0


# ===========================================================================
# Protobuf payloads (§9.8: WIRE semantics with renumber detection)
# ===========================================================================

F = descriptor_pb2.FieldDescriptorProto


@dataclass
class ProtoWorld:
    """Every message and enum of one revision's descriptor sets, by
    ``.``-prefixed full name, with its file's syntax."""

    messages: dict[str, tuple[descriptor_pb2.DescriptorProto, str]] = field(default_factory=dict)
    enums: dict[str, tuple[descriptor_pb2.EnumDescriptorProto, str]] = field(default_factory=dict)

    @classmethod
    def from_sets(cls, sets: list[bytes]) -> ProtoWorld:
        w = cls()
        for data in sets:
            for f in protoc.parse_set(data).file:
                syntax = f.syntax or "proto2"
                scope = f".{f.package}" if f.package else ""

                def walk(msgs, enums, scope: str) -> None:
                    for e in enums:
                        w.enums[f"{scope}.{e.name}"] = (e, syntax)
                    for m in msgs:
                        full = f"{scope}.{m.name}"
                        w.messages[full] = (m, syntax)
                        walk(m.nested_type, m.enum_type, full)

                walk(f.message_type, f.enum_type, scope)
        return w


def _real_oneof(m: descriptor_pb2.DescriptorProto, f: Any) -> str | None:
    """The name of a field's oneof, synthetic proto3-optional oneofs aside."""
    if f.HasField("oneof_index") and not f.proto3_optional:
        return m.oneof_decl[f.oneof_index].name
    return None


def json_name_default(name: str) -> str:
    """protoc's default ``json_name``: underscores dropped, the next letter
    upper-cased (SPEC-FINDINGS: default json_name)."""
    out, up = [], False
    for c in name:
        if c == "_":
            up = True
        elif up:
            out.append(c.upper())
            up = False
        else:
            out.append(c)
    return "".join(out)


def _reserved(m: descriptor_pb2.DescriptorProto, number: int) -> bool:
    return any(r.start <= number < r.end for r in m.reserved_range)


def proto_compare_message(old: ProtoWorld, oname: str, new: ProtoWorld, nname: str,
                          seen: set | None = None) -> Verdict:
    v = Verdict()
    seen = set() if seen is None else seen
    if (oname, nname) in seen:
        return v
    seen.add((oname, nname))
    om, _ = old.messages[oname]
    nm, _ = new.messages[nname]
    ofields = {f.number: f for f in om.field}
    nfields = {f.number: f for f in nm.field}
    path = oname.lstrip(".")

    for num in sorted(set(ofields) & set(nfields)):
        of, nf = ofields[num], nfields[num]
        where = f"{path}.{of.name} = {num}"
        if of.type != nf.type:
            # "a field's declared scalar type changes, including int32 →
            # int64 and string → bytes".
            v.add(BREAKING, f"{where}: type {F.Type.Name(of.type)} → {F.Type.Name(nf.type)}")
        if of.label != nf.label:
            v.add(BREAKING, f"{where}: cardinality {F.Label.Name(of.label)} → {F.Label.Name(nf.label)}")
        if _real_oneof(om, of) != _real_oneof(nm, nf):
            v.add(BREAKING, f"{where}: moved into or out of a oneof")
        if of.proto3_optional != nf.proto3_optional:
            v.add(REVIEW, f"{where}: proto3 optional toggled")
        if of.name != nf.name:
            v.add(REVIEW, f"{where}: renamed to {nf.name}")
        elif of.json_name != nf.json_name:
            v.add(REVIEW, f"{where}: json_name {of.json_name!r} → {nf.json_name!r}")
        if of.default_value != nf.default_value:
            v.add(REVIEW, f"{where}: default changed")
        if of.type == nf.type and of.type in (F.TYPE_MESSAGE, F.TYPE_GROUP):
            v.merge(proto_compare_message(old, of.type_name, new, nf.type_name, seen))
        elif of.type == nf.type == F.TYPE_ENUM:
            v.merge(proto_compare_enum(old, of.type_name, new, nf.type_name))

    added = set(nfields) - set(ofields)
    new_by_name = {nfields[n].name: n for n in added}
    for num in sorted(set(ofields) - set(nfields)):
        of = ofields[num]
        if of.name in new_by_name:
            # "it is renumbered: a deletion plus an addition of the same
            # field, which silently drops the data both ways". The same
            # field is the same name (SPEC-FINDINGS: renumber identity); the
            # deletion is then not also a warning (compat/renumber-field).
            moved = new_by_name.pop(of.name)
            added.discard(moved)
            v.add(BREAKING, f"{path}.{of.name}: renumbered {num} → {moved}")
            continue
        # A field deleted: compatible under WIRE semantics; a warning when
        # its number is not reserved.
        if not _reserved(nm, num):
            v.warnings.append(FIELD_DELETED_UNRESERVED)
            v.reasons.append(f"warning: {path}.{of.name} = {num} deleted without reserving {num}")
    # Fields added are compatible.
    return v


def proto_compare_enum(old: ProtoWorld, oname: str, new: ProtoWorld, nname: str) -> Verdict:
    v = Verdict()
    oe, osyn = old.enums[oname]
    ne, nsyn = new.enums[nname]
    path = oname.lstrip(".")
    onames: dict[int, set[str]] = {}
    nnames: dict[int, set[str]] = {}
    for val in oe.value:
        onames.setdefault(val.number, set()).add(val.name)
    for val in ne.value:
        nnames.setdefault(val.number, set()).add(val.name)
    for num in sorted(set(onames) & set(nnames)):
        if onames[num] != nnames[num]:
            v.add(REVIEW, f"{path}: value {num} renamed")
    for num in sorted(set(onames) - set(nnames)):
        v.add(REVIEW, f"{path}: value {num} deleted")
    closed = "proto2" in (osyn, nsyn)
    for num in sorted(set(nnames) - set(onames)):
        # "a value added to a proto3 (open) enum" is compatible; "to a proto2
        # (closed) enum is review".
        v.add(REVIEW if closed else COMPATIBLE, f"{path}: value {num} added")
    return v


def proto_normalized(data: bytes) -> descriptor_pb2.FileDescriptorSet:
    """§9.7: identity compares the FileDescriptorSets "with source info
    dropped and every default json_name dropped"."""
    fds = protoc.parse_set(data)

    def fields(msgs):
        for m in msgs:
            yield from m.field
            yield from m.extension
            yield from fields(m.nested_type)

    for f in fds.file:
        f.ClearField("source_code_info")
        for fld in [*fields(f.message_type), *f.extension]:
            if fld.json_name == json_name_default(fld.name):
                fld.ClearField("json_name")
    return fds


def proto_same_revision(old: list[bytes], new: list[bytes]) -> bool:
    if len(old) != len(new):
        return False
    return all(proto_normalized(a) == proto_normalized(b) for a, b in zip(old, new))


# ===========================================================================
# Payload types inside a contract
# ===========================================================================

@dataclass
class Revision:
    """What the classifier needs of one contract revision: the canonical
    form, and the artifacts it lists by id (JSON documents, or
    FileDescriptorSet bytes)."""

    canonical: dict[str, Any]
    artifacts: dict[str, Any]

    def json_world(self) -> JsonWorld:
        docs = {}
        for s in self.canonical["schemas"]:
            if s["kind"] == JSON and s["id"] in self.artifacts:
                docs[s["name"]] = self.artifacts[s["id"]]
        return JsonWorld(docs)

    def proto_world(self) -> ProtoWorld:
        return ProtoWorld.from_sets([self.artifacts[s["id"]] for s in self.canonical["schemas"]
                                     if s["kind"] == PROTOBUF and s["id"] in self.artifacts])

    def json_stem(self, schema_id: str) -> str:
        return next(s["name"] for s in self.canonical["schemas"] if s["id"] == schema_id)


def type_compare(old: Revision, ot: dict[str, Any] | None,
                 new: Revision, nt: dict[str, Any] | None, where: str) -> Verdict:
    v = Verdict()
    if ot == nt and (ot is None or ot["kind"] == RAW):
        return v
    if ot is None or nt is None:
        v.add(REVIEW, f"{where}: type {'added' if ot is None else 'removed'}")
        return v
    if ot["kind"] != nt["kind"]:
        v.add(BREAKING, f"{where}: type kind {ot['kind']} → {nt['kind']}")
        return v
    if ot["kind"] == RAW:
        v.add(BREAKING, f"{where}: media type {ot['media_type']}/{ot['media_param']} → "
                        f"{nt['media_type']}/{nt['media_param']}")
        return v
    if ot["kind"] == JSON:
        ow, nw = old.json_world(), new.json_world()
        os_, ns = old.json_stem(ot["schema"]), new.json_stem(nt["schema"])
        v.merge(json_compare(ow, os_, ow.docs[os_]["$defs"][ot["name"]],
                             nw, ns, nw.docs[ns]["$defs"][nt["name"]], ot["name"]), f"{where}: ")
        return v
    v.merge(proto_compare_message(old.proto_world(), "." + ot["name"],
                                  new.proto_world(), "." + nt["name"]), f"{where}: ")
    return v


# ===========================================================================
# Contract metadata (§9.8 table)
# ===========================================================================

#: Directional rules of the §9.8 table: (member, old, new) → class.
_TRANSITIONS = {
    ("idempotent", True, False): BREAKING,
    ("fanout", "allowed", "forbidden"): BREAKING,
    ("reliability", "reliable", "best_effort"): REVIEW,
    ("optional", True, False): BREAKING,  # optional → required
    ("congestion", "drop", "block"): REVIEW,
    ("replies", "one", "many"): BREAKING,
}
#: Members whose change is review whatever the direction (§9.8: "priority
#: changed, express toggled").
_ANY_CHANGE_REVIEW = {"priority", "express"}
_TYPE_MEMBERS = ("type", "attachment", "request", "response", "error", "summary")
_EXPLICIT = {("stream", "@stream"), ("state", "@state")}


def contract_compare(old: Revision, new: Revision) -> Verdict:
    """Classify the transition from ``old`` to ``new`` (§9.8)."""
    v = Verdict()
    oc, nc = old.canonical, new.canonical
    if oc["interface"] != nc["interface"]:
        v.add(BREAKING, f"interface {oc['interface']} → {nc['interface']}")
        return v
    if oc["uses"] != nc["uses"]:
        v.add(REVIEW, "uses changed")
    ores = {r["template"]: r for r in oc["resources"]}
    nres = {r["template"]: r for r in nc["resources"]}
    for t in sorted(set(ores) - set(nres)):
        v.add(BREAKING, f"resource {t!r} removed")
    for t in sorted(set(nres) - set(ores)):
        # Not in §9.8's table: an optional resource is compatible, by the
        # table's own "optional role added"; a required one is review.
        v.add(COMPATIBLE if nres[t]["optional"] else REVIEW, f"resource {t!r} added")
    for t in sorted(set(ores) & set(nres)):
        v.merge(_resource_compare(old, ores[t], new, nres[t]))
    oreq, nreq = oc["requires"], nc["requires"]
    for role in sorted(set(nreq) - set(oreq)):
        # "A required role added: breaking. An optional role added:
        # compatible."
        v.add(COMPATIBLE if nreq[role]["optional"] else BREAKING, f"role {role!r} added")
    for role in sorted(set(oreq) - set(nreq)):
        v.add(REVIEW, f"role {role!r} removed")
    for role in sorted(set(oreq) & set(nreq)):
        a, b = oreq[role], nreq[role]
        # "A role's interface or cardinality changed: breaking."
        for m in ("interface", "cardinality"):
            if a[m] != b[m]:
                v.add(BREAKING, f"role {role!r}: {m} {a[m]!r} → {b[m]!r}")
        if a["optional"] and not b["optional"]:
            v.add(BREAKING, f"role {role!r}: optional → required")
        elif a["optional"] != b["optional"]:
            v.add(REVIEW, f"role {role!r}: required → optional")
        for m in ("resources", "annotations"):
            if a[m] != b[m]:
                v.add(REVIEW, f"role {role!r}: {m} changed")
    return v


def _resource_compare(old: Revision, a: dict[str, Any], new: Revision, b: dict[str, Any]) -> Verdict:
    v = Verdict()
    t = a["template"]
    if a["kind"] != b["kind"]:
        v.add(BREAKING, f"{t}: kind {a['kind']} → {b['kind']}")
        return v
    if a["token"] != b["token"]:
        # "explicit false → true: breaking for ambient consumers";
        # "explicit true → false: review (link budgets)".
        if (a["token"], b["token"]) in _EXPLICIT:
            v.add(BREAKING, f"{t}: explicit false → true")
        else:
            v.add(REVIEW, f"{t}: explicit true → false")
    for m in _TYPE_MEMBERS:
        if m in a or m in b:
            v.merge(type_compare(old, a.get(m), new, b.get(m), f"{t}: {m}"))
    skip = {"template", "kind", "token", *_TYPE_MEMBERS}
    for m in sorted((set(a) | set(b)) - skip):
        x, y = a.get(m), b.get(m)
        if x == y:
            continue
        cls = _TRANSITIONS.get((m, x, y))
        if cls is None:
            cls = REVIEW  # §9.8 lists it in neither direction, or not this one
        v.add(cls, f"{t}: {m} {x!r} → {y!r}" + ("" if m in _ANY_CHANGE_REVIEW or (m, x, y) in _TRANSITIONS
                                                 else " (not in §9.8's table)"))
    return v


# ===========================================================================
# FULL_TRANSITIVE
# ===========================================================================

def full_transitive(history: list[Any], candidate: Any,
                    compare: Callable[[Any, Any], Verdict]) -> tuple[Verdict, list[Verdict]]:
    """§9.8: the candidate against every revision in the history; the class
    is the worst of them. Returns the overall verdict and each pairwise one,
    in history order."""
    total = Verdict()
    each = []
    for h in history:
        pv = compare(h, candidate)
        each.append(pv)
        total.merge(pv)
    total.warnings = sorted(total.warnings)
    return total, each

"""The compatibility classifier (core.md §9.8, version 0.4) and the retention
identity check (core.md §9.7).

§9.8: "Revisions inside a major are checked FULL_TRANSITIVE: a candidate
against every revision in the history, in both directions." A change is
*compatible*, *review* or *breaking*; "the candidate's class is the worst
over both directions and every earlier revision"; a revision that does not
load is *invalid*.

Since 0.3, §9.8 states every rule in six tables (interface, resources,
delivery, operations, roles, types) plus the protobuf and JSON Schema lists,
each rule with a name. This module implements those tables and reports the
reference names in its reasons ("Rule names stay informative", CHANGELOG
0.3). Before 0.3, zk2py classed every unlisted change as review; the
amended tables now decide them, and where zk2py's earlier guess differed
it now follows the table. The twelve changes, listed in the README, are:
- required → optional, a required resource added, a parameter type changed,
  an encoding changed: review → breaking;
- best_effort → reliable, ``fanout`` forbidden → allowed, ``replies`` many
  → one, a role removed, a role required → optional, ``deprecated`` added:
  review → compatible;
- a raw ``media_param`` changed: breaking → review;
- ``items`` toggled between absent, true and false: breaking → compatible.

How this module reads "both directions" (SPEC-FINDINGS F-29): each rule
classifies a transition *from an earlier revision to the candidate*, and its
class already accounts for both reader/writer roles.

A change no table lists is classed **review** (SPEC-FINDINGS F-31, F-32,
F-35 record what is still unlisted).
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

    def add(self, cls: str, rule: str, reason: str) -> None:
        """Record one finding: its class, §9.8's rule name, and a reason."""
        self.cls = worst(self.cls, cls)
        if cls != COMPATIBLE:
            self.reasons.append(f"{cls} {rule}: {reason}")

    def warn(self, name: str, reason: str) -> None:
        self.warnings.append(name)
        self.reasons.append(f"warning {name}: {reason}")

    def merge(self, other: Verdict, prefix: str = "") -> None:
        self.cls = worst(self.cls, other.cls)
        self.warnings += other.warnings
        self.reasons += [f"{prefix}{r}" for r in other.reasons]


# ===========================================================================
# JSON Schema payloads (§9.8, over the §7.3 subset)
# ===========================================================================

#: §9.8 ``bound_changed``: "a bound changed, either way".
BOUNDS = ("minimum", "maximum", "exclusiveMinimum", "exclusiveMaximum",
          "minLength", "maxLength", "minItems", "maxItems")


@dataclass
class JsonWorld:
    """The JSON Schema documents of one revision, by stem.

    §9.8: "``$ref``s are followed, across the revision's artifacts." A
    ``$ref``'s file part is resolved by the stem of its last path component:
    in a source tree §9.4 resolves it as a path, but a bundle keeps only
    stems, and §9.4's stem uniqueness (E024) makes the stem enough
    (SPEC-FINDINGS F-11).
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
            if k == "properties" and isinstance(v, dict):
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
    at = path or "/"
    key = (ow, id(o), nw, id(n))
    if key in seen:
        return v
    seen.add(key)
    if not isinstance(o, dict) or not isinstance(n, dict):
        # A boolean schema where a schema object is expected: no rule names
        # this (SPEC-FINDINGS F-32).
        if old.normalize(ow, o) != new.normalize(nw, n):
            v.add(BREAKING, "boolean_schema_changed", f"{at}: a boolean schema changed")
        return v
    if "$ref" in o or "$ref" in n:
        # A $ref beside other keywords: no rule names it (SPEC-FINDINGS F-32).
        if old.normalize(ow, o) != new.normalize(nw, n):
            v.add(REVIEW, "ref_with_siblings_changed", f"{at}: a $ref beside other keywords changed")
        return v

    # "the type set changed, including integer ↔ number (type_changed)".
    if _types(o.get("type")) != _types(n.get("type")):
        v.add(BREAKING, "type_changed", f"{at}: type {o.get('type')!r} → {n.get('type')!r}")
    # "an enum value added or removed (enum_changed)"; "enum values
    # reordered" is compatible, so enum compares as a multiset.
    if ("enum" in o) != ("enum" in n) or _multiset(o.get("enum", [])) != _multiset(n.get("enum", [])):
        v.add(BREAKING, "enum_changed", f"{at}: enum changed")
    # "const changed (const_changed)".
    if o.get("const", _MISSING) != n.get("const", _MISSING):
        v.add(BREAKING, "const_changed", f"{at}: const changed")
    # "a bound changed, either way (bound_changed)".
    for b in BOUNDS:
        if (b in o) != (b in n) or o.get(b) != n.get(b):
            v.add(BREAKING, "bound_changed", f"{at}: {b} {o.get(b)!r} → {n.get(b)!r}")

    # Properties: "an optional property added, even to a closed schema, or
    # removed" is compatible; "a required property added (required_added)
    # or removed (required_removed), or a property optional ↔ required
    # (required_changed)" is breaking.
    op, np_ = o.get("properties", {}), n.get("properties", {})
    oreq, nreq = set(o.get("required", [])), set(n.get("required", []))
    for name in sorted(set(op) & set(np_)):
        p = f"{path}/{name}"
        if (name in oreq) != (name in nreq):
            v.add(BREAKING, "required_changed",
                  f"{p}: {'optional → required' if name in nreq else 'required → optional'}")
        v.merge(json_compare(old, ow, op[name], new, nw, np_[name], p, seen))
    for name in sorted(set(np_) - set(op)):
        if name in nreq:
            v.add(BREAKING, "required_added", f"{path}/{name}: a required property added")
    for name in sorted(set(op) - set(np_)):
        if name in oreq:
            v.add(BREAKING, "required_removed", f"{path}/{name}: a required property removed")
    loose = (oreq ^ nreq) - set(op) - set(np_)
    if loose:
        # A `required` name with no property on either side (SPEC-FINDINGS F-32).
        v.add(BREAKING, "required_changed", f"{at}: required names {sorted(loose)} changed")

    # additionalProperties and items: "changed between absent, true and
    # false" is compatible; "gaining or losing a schema (members_changed)"
    # is review; two schemas are compared.
    for kw in ("additionalProperties", "items"):
        oa, na = o.get(kw, True), n.get(kw, True)
        if isinstance(oa, dict) and isinstance(na, dict):
            v.merge(json_compare(old, ow, oa, new, nw, na, f"{path}/{kw}", seen))
        elif isinstance(oa, dict) != isinstance(na, dict):
            v.add(REVIEW, "members_changed", f"{at}: {kw} gained or lost a schema")

    # oneOf / anyOf / prefixItems: "a oneOf branch added
    # (oneof_branch_added)" is breaking; "any other change inside oneOf,
    # anyOf or prefixItems (undecided_changed)" is review.
    for kw in ("oneOf", "anyOf", "prefixItems"):
        ob = [old.normalize(ow, s) for s in o.get(kw, [])]
        nb = [new.normalize(nw, s) for s in n.get(kw, [])]
        same = (_canon(ob) == _canon(nb)) if kw == "prefixItems" else (_multiset(ob) == _multiset(nb))
        if same and (kw in o) == (kw in n):
            continue
        if kw == "oneOf" and kw in o and kw in n and _is_superset(nb, ob):
            v.add(BREAKING, "oneof_branch_added", f"{at}: a oneOf branch added")
        else:
            v.add(REVIEW, "undecided_changed", f"{at}: a change inside {kw}")

    judged = {"type", "enum", "const", *BOUNDS, "properties", "required",
              "additionalProperties", "items", "oneOf", "anyOf", "prefixItems"}
    for k in sorted((set(o) | set(n)) - judged - ANNOTATIONS):
        if _canon(o.get(k)) != _canon(n.get(k)):
            v.add(REVIEW, "unlisted_changed", f"{at}: {k} changed")
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

    def is_map_entry(self, type_name: str) -> bool:
        hit = self.messages.get(type_name)
        return hit is not None and hit[0].options.map_entry


def _real_oneof(m: descriptor_pb2.DescriptorProto, f: Any) -> str | None:
    """The name of a field's oneof, synthetic proto3-optional oneofs aside."""
    if f.HasField("oneof_index") and not f.proto3_optional:
        return m.oneof_decl[f.oneof_index].name
    return None


def _cardinality(world: ProtoWorld, f: Any) -> str:
    """§9.8 ``cardinality_changed``: "singular, repeated or map"."""
    if f.label == F.LABEL_REPEATED:
        if f.type == F.TYPE_MESSAGE and world.is_map_entry(f.type_name):
            return "map"
        return "repeated"
    return "singular"


def _presence(f: Any, syntax: str, oneof: str | None) -> bool:
    """Explicit presence: a singular field of a message type, of a proto2
    file, declared proto3 ``optional``, or in a oneof."""
    if f.label == F.LABEL_REPEATED:
        return False
    return (f.type in (F.TYPE_MESSAGE, F.TYPE_GROUP) or syntax == "proto2"
            or f.proto3_optional or oneof is not None)


def json_name_default(name: str) -> str:
    """protoc's default ``json_name``: underscores dropped, the next letter
    upper-cased (SPEC-FINDINGS F-28)."""
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
    """§9.8: "Fields are matched by number. A field missing by number but
    present by name is renumbered. Messages are compared recursively, nested
    and referenced ones included, each pair once." """
    v = Verdict()
    seen = set() if seen is None else seen
    if (oname, nname) in seen:
        return v
    seen.add((oname, nname))
    om, osyn = old.messages[oname]
    nm, nsyn = new.messages[nname]
    ofields = {f.number: f for f in om.field}
    nfields = {f.number: f for f in nm.field}
    path = oname.lstrip(".")

    for num in sorted(set(ofields) & set(nfields)):
        of, nf = ofields[num], nfields[num]
        where = f"{path}.{of.name} = {num}"
        if of.type != nf.type:
            v.add(BREAKING, "type_changed", f"{where}: {F.Type.Name(of.type)} → {F.Type.Name(nf.type)}")
        oc, nc = _cardinality(old, of), _cardinality(new, nf)
        if oc != nc:
            v.add(BREAKING, "cardinality_changed", f"{where}: {oc} → {nc}")
        if (of.label == F.LABEL_REQUIRED) != (nf.label == F.LABEL_REQUIRED):
            v.add(BREAKING, "required_label_changed", f"{where}: a label toggled to or from required")
        o1, n1 = _real_oneof(om, of), _real_oneof(nm, nf)
        if o1 != n1:
            v.add(BREAKING, "oneof_changed", f"{where}: moved into or out of a oneof")
        if _presence(of, osyn, o1) != _presence(nf, nsyn, n1):
            v.add(REVIEW, "presence_changed", f"{where}: explicit presence toggled")
        if of.name != nf.name:
            v.add(REVIEW, "field_renamed", f"{where}: renamed to {nf.name}")
        elif of.json_name != nf.json_name:
            v.add(REVIEW, "json_name_changed", f"{where}: json_name {of.json_name!r} → {nf.json_name!r}")
        if of.default_value != nf.default_value:
            # A proto2 default changed: no rule names it (SPEC-FINDINGS F-35).
            v.add(REVIEW, "default_changed", f"{where}: default changed")
        if of.type == nf.type and of.type in (F.TYPE_MESSAGE, F.TYPE_GROUP):
            v.merge(proto_compare_message(old, of.type_name, new, nf.type_name, seen))
        elif of.type == nf.type == F.TYPE_ENUM:
            v.merge(proto_compare_enum(old, of.type_name, new, nf.type_name))

    added = set(nfields) - set(ofields)
    new_by_name = {nfields[n].name: n for n in added}
    for num in sorted(set(ofields) - set(nfields)):
        of = ofields[num]
        if of.name in new_by_name:
            # "it is renumbered … (renumbered)". The deletion is the move's,
            # not also a warning (compat/payload/protobuf/renumber-field;
            # SPEC-FINDINGS F-34).
            moved = new_by_name.pop(of.name)
            added.discard(moved)
            v.add(BREAKING, "renumbered", f"{path}.{of.name}: {num} → {moved}")
            continue
        if of.label == F.LABEL_REQUIRED:
            v.add(BREAKING, "required_field_removed", f"{path}.{of.name} = {num}: a required field deleted")
        if not _reserved(nm, num):
            v.warn(FIELD_DELETED_UNRESERVED, f"{path}.{of.name} = {num} deleted without reserving {num}")
    for num in sorted(added):
        nf = nfields[num]
        if _reserved(om, num):
            v.add(BREAKING, "reserved_reused", f"{path}.{nf.name} = {num}: reuses a reserved number")
        if nf.label == F.LABEL_REQUIRED:
            v.add(BREAKING, "required_field_added", f"{path}.{nf.name} = {num}: a required field added")

    # Nested messages, by name, each pair once.
    nested_new = {m.name for m in nm.nested_type}
    for m in om.nested_type:
        if m.name in nested_new and not m.options.map_entry:
            v.merge(proto_compare_message(old, f"{oname}.{m.name}", new, f"{nname}.{m.name}", seen))
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
            v.add(REVIEW, "enum_value_renamed", f"{path}: value {num} renamed")
    for num in sorted(set(onames) - set(nnames)):
        v.add(REVIEW, "enum_value_removed", f"{path}: value {num} deleted")
    # "a value added to a proto2 (closed) enum (closed_enum_value_added)" is
    # review; to a proto3 (open) enum, compatible. Closed when either
    # revision's file is proto2 (SPEC-FINDINGS F-35).
    closed = "proto2" in (osyn, nsyn)
    for num in sorted(set(nnames) - set(onames)):
        if closed:
            v.add(REVIEW, "closed_enum_value_added", f"{path}: value {num} added")
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
# Revisions
# ===========================================================================

@dataclass
class Revision:
    """What the classifier needs of one contract revision: the canonical
    form, and the artifacts it lists by id (JSON documents, or
    FileDescriptorSet bytes). Built from a loaded contract or a verified
    bundle alike."""

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

    def json_stem(self, schema_id: str) -> str | None:
        return next((s["name"] for s in self.canonical["schemas"] if s["id"] == schema_id), None)


def type_compare(old: Revision, ot: dict[str, Any], new: Revision, nt: dict[str, Any],
                 where: str) -> Verdict:
    """§9.8 "Types", then the payload rules for the schema kind."""
    v = Verdict()
    if ot["kind"] != nt["kind"]:
        v.add(BREAKING, "type_kind_changed", f"{where}: {ot['kind']} → {nt['kind']}")
        return v
    if ot["kind"] == RAW:
        if ot["media_type"] != nt["media_type"]:
            v.add(BREAKING, "media_type_changed", f"{where}: {ot['media_type']} → {nt['media_type']}")
        if ot["media_param"] != nt["media_param"]:
            v.add(REVIEW, "media_param_changed", f"{where}: {ot['media_param']} → {nt['media_param']}")
        return v
    if ot["kind"] == JSON:
        ow, nw = old.json_world(), new.json_world()
        os_, ns = old.json_stem(ot["schema"]), new.json_stem(nt["schema"])
        onode = ((ow.docs.get(os_) or {}).get("$defs") or {}).get(ot["name"]) if os_ else None
        nnode = ((nw.docs.get(ns) or {}).get("$defs") or {}).get(nt["name"]) if ns else None
        if onode is None or nnode is None:
            v.add(REVIEW, "schema_unreadable", f"{where}: an artifact lacks {ot['name']}")
            return v
        v.merge(json_compare(ow, os_, onode, nw, ns, nnode, ""), f"{where}: ")
        return v
    try:
        ow, nw = old.proto_world(), new.proto_world()
    except Exception:  # noqa: BLE001 - bytes that do not decode as a FileDescriptorSet
        v.add(REVIEW, "schema_unreadable", f"{where}: an artifact does not decode")
        return v
    oname, nname = "." + ot["name"], "." + nt["name"]
    if oname not in ow.messages or nname not in nw.messages:
        v.add(REVIEW, "schema_unreadable", f"{where}: an artifact lacks {ot['name']}")
        return v
    v.merge(proto_compare_message(ow, oname, nw, nname), f"{where}: ")
    return v


# ===========================================================================
# Contracts: §9.8's interface, resources, delivery, operations and roles
# ===========================================================================

#: The members the "delivery" and "operations" tables class as review when
#: they change at all.
_REVIEW_IF_CHANGED = {
    "cardinality": "cardinality_changed", "epoch": "epoch_changed",
    "gate": "gate_changed", "annotations": "annotations_changed",
    "congestion": "congestion_changed", "priority": "priority_changed",
    "express": "express_toggled", "history": "history_changed",
    "rate": "rate_changed", "retention_s": "retention_changed",
    "serving": "serving_changed", "timeout_ms": "timeout_changed",
}
_EXPLICIT = {("stream", "@stream"), ("state", "@state")}


def contract_compare(old: Revision, new: Revision) -> Verdict:
    """Classify the transition from ``old`` to ``new`` (§9.8)."""
    v = Verdict()
    oc, nc = old.canonical, new.canonical
    # Interface.
    if oc["interface"] != nc["interface"]:
        v.add(BREAKING, "interface_changed", f"{oc['interface']} → {nc['interface']}")
        return v
    if oc["uses"] != nc["uses"]:
        v.add(REVIEW, "uses_changed", "uses changed")
    # Resources. §9.8 says "matched by kind token and template", but its own
    # explicit_set / explicit_cleared rows (and compat/contract/
    # explicit-true-to-false) need a pairing that survives a token change:
    # zk2py pairs by template, unique within a contract (SPEC-FINDINGS F-40).
    ores = {r["template"]: r for r in oc["resources"]}
    nres = {r["template"]: r for r in nc["resources"]}
    for t in sorted(set(ores) - set(nres)):
        v.add(BREAKING, "resource_removed", f"{t!r} removed")
    for t in sorted(set(nres) - set(ores)):
        if nres[t]["optional"]:
            v.add(COMPATIBLE, "", f"{t!r} added, optional")
        else:
            v.add(BREAKING, "required_resource_added", f"{t!r} added, required")
    for t in sorted(set(ores) & set(nres)):
        v.merge(_resource_compare(old, ores[t], new, nres[t]), f"{t}: ")
    # Roles.
    oreq, nreq = oc["requires"], nc["requires"]
    for role in sorted(set(nreq) - set(oreq)):
        if not nreq[role]["optional"]:
            v.add(BREAKING, "required_role_added", f"role {role!r} added, required")
    # "A role removed: compatible (deployments stop binding it)."
    for role in sorted(set(oreq) & set(nreq)):
        a, b = oreq[role], nreq[role]
        for m in ("interface", "cardinality"):
            if a[m] != b[m]:
                v.add(BREAKING, "role_changed", f"role {role!r}: {m} {a[m]!r} → {b[m]!r}")
        if a["optional"] and not b["optional"]:
            v.add(BREAKING, "role_required", f"role {role!r}: optional → required")
        if a["resources"] != b["resources"] or a["annotations"] != b["annotations"]:
            v.add(REVIEW, "role_resources_changed", f"role {role!r}: resources or annotations changed")
    return v


def _resource_compare(old: Revision, a: dict[str, Any], new: Revision, b: dict[str, Any]) -> Verdict:
    v = Verdict()
    if a["kind"] != b["kind"]:
        v.add(BREAKING, "kind_changed", f"kind {a['kind']} → {b['kind']}")
        return v
    if a["token"] != b["token"]:
        if (a["token"], b["token"]) in _EXPLICIT:
            v.add(BREAKING, "explicit_set", "explicit false → true")
        else:
            v.add(REVIEW, "explicit_cleared", "explicit true → false")
    if a["optional"] != b["optional"]:
        if a["optional"]:
            v.add(BREAKING, "optional_to_required", "optional → required")
        else:
            v.add(BREAKING, "required_to_optional", "required → optional")
    if a["params"] != b["params"]:
        v.add(BREAKING, "params_changed", f"params {a['params']} → {b['params']}")
    for m in ("encoding", "attachment_encoding"):
        if m in a and a.get(m) != b.get(m):
            v.add(BREAKING, "encoding_changed", f"{m} {a.get(m)!r} → {b.get(m)!r}")
    if a["deprecated"] != b["deprecated"]:
        if a["deprecated"] is None:
            v.add(COMPATIBLE, "", "deprecated added")
        else:
            v.add(REVIEW, "deprecated_changed", "deprecated removed or changed")
    for m, rule in _REVIEW_IF_CHANGED.items():
        if m in a and a.get(m) != b.get(m):
            v.add(REVIEW, rule, f"{m} {a.get(m)!r} → {b.get(m)!r}")
    if a["kind"] == "operation":
        _operation_rules(v, a, b)
    else:
        if a["reliability"] == "reliable" and b["reliability"] == "best_effort":
            v.add(REVIEW, "reliability_lowered", "reliable → best_effort")
    # Types: always-present ones follow the type rules; optional ones (an
    # attachment, an error, a summary) are review when added or removed.
    optional_types = {"attachment": "attachment_changed", "error": "error_type_changed",
                      "summary": "summary_type_changed"}
    for m in ("type", "request", "response", "attachment", "error", "summary"):
        if m not in a:
            continue
        x, y = a.get(m), b.get(m)
        if x is None and y is None:
            continue
        if x is None or y is None:
            v.add(REVIEW, optional_types[m], f"{m} {'added' if x is None else 'removed'}")
            continue
        v.merge(type_compare(old, x, new, y, m))
    return v


def _operation_rules(v: Verdict, a: dict[str, Any], b: dict[str, Any]) -> None:
    """§9.8 "Operations"."""
    if a["idempotent"] and not b["idempotent"]:
        v.add(BREAKING, "idempotent_cleared", "idempotent true → false")
    elif b["idempotent"] and not a["idempotent"]:
        v.add(REVIEW, "idempotent_set", "idempotent false → true")
    if a["fanout"] == "allowed" and b["fanout"] == "forbidden":
        v.add(BREAKING, "fanout_forbidden", "fanout allowed → forbidden")
    if a["replies"] == "one" and b["replies"] == "many":
        v.add(BREAKING, "replies_many", "replies one → many")


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
    return total, each

"""Schema artifacts and type references (core.md §7.1, §7.3, §9.4).

A contract's ``[schemas]`` lists JSON Schema and protobuf files. Each listed
file is one *artifact*, identified by the sha256 of its bytes: the JCS bytes
of a JSON Schema document, the FileDescriptorSet bytes of a protobuf file.
This module loads them, reports the per-file lints (E024 stems, E028 at
load, E029, E032, E037), and resolves type references (E023, E024).
"""

from __future__ import annotations

import posixpath
import re
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

from . import jcs, protoc

JSON, PROTOBUF, RAW = "jsonschema", "protobuf", "raw"

# core.md §7.3, the zk2 subset of JSON Schema 2020-12.
#: "Keywords that carry meaning".
MEANING = {
    "type", "properties", "required", "additionalProperties", "items", "prefixItems",
    "enum", "const",
    "minimum", "maximum", "exclusiveMinimum", "exclusiveMaximum",
    "minLength", "maxLength", "minItems", "maxItems",
    "$ref", "oneOf", "anyOf",
}
#: "Annotations, ignored".
ANNOTATIONS = {
    "$schema", "$id", "$defs", "$comment", "title", "description", "default",
    "examples", "format", "deprecated", "readOnly", "writeOnly",
}
# core.md §7.3 (0.5): "Schema positions are the document root; each value
# of properties and $defs; each element of prefixItems, oneOf and anyOf; and
# the value of items and additionalProperties. … Nothing else is a schema
# position": not a refused keyword's content, not data (enum, const, …).
# E037, E032 and the classifier read schema positions only.
_SCHEMA_VALUED = {"additionalProperties", "items"}
_SCHEMA_LISTS = {"prefixItems", "oneOf", "anyOf"}
_SCHEMA_MAPS = {"properties", "$defs"}

#: core.md §9.4 "Raw": each token is a lowercase letter or digit followed by
#: lowercase letters, digits or ``!#$&-^_.+``; the subtype may be ``*``.
_MEDIA_TOKEN = r"[a-z0-9][a-z0-9!#$&\-^_.+]*"
MEDIA_TYPE = re.compile(rf"{_MEDIA_TOKEN}/(?:{_MEDIA_TOKEN}|\*)")


@dataclass
class Artifact:
    kind: str                  # JSON or PROTOBUF
    name: str                  # the stem, or the proto path relative to its root
    id: str | None             # sha256:…, None when the bytes cannot be hashed
    data: Any                  # the JSON document, or the FileDescriptorSet bytes
    listed: str | None = None  # the path as listed in [schemas], for $ref


@dataclass
class Finding:
    code: str
    message: str


def walk_schema(node: Any, visit, path: str = "") -> None:
    """Call ``visit(schema_object, path)`` for every schema object at a
    schema position (§7.3), the root included."""
    if not isinstance(node, dict):
        return
    visit(node, path)
    for kw, val in node.items():
        if kw in _SCHEMA_VALUED:
            walk_schema(val, visit, f"{path}/{kw}")
        elif kw in _SCHEMA_LISTS and isinstance(val, list):
            for i, v in enumerate(val):
                walk_schema(v, visit, f"{path}/{kw}/{i}")
        elif kw in _SCHEMA_MAPS and isinstance(val, dict):
            # A property *named* like a keyword is data (§7.3): only the
            # values of these maps are schemas.
            for k, v in val.items():
                walk_schema(v, visit, f"{path}/{kw}/{k}")


def refused_keywords(doc: Any) -> list[str]:
    """§7.3 "Refused": every keyword in a schema position outside the subset,
    once per keyword per file (E037)."""
    found: list[str] = []

    def visit(obj: dict[str, Any], _path: str) -> None:
        for kw in obj:
            if kw not in MEANING and kw not in ANNOTATIONS and kw not in found:
                found.append(kw)

    walk_schema(doc, visit)
    return found


def json_pointer(doc: Any, pointer: str) -> tuple[bool, Any]:
    """Resolve an RFC 6901 pointer; (found, value)."""
    if pointer == "":
        return True, doc
    if not pointer.startswith("/"):
        return False, None
    node = doc
    for raw in pointer[1:].split("/"):
        part = raw.replace("~1", "/").replace("~0", "~")
        if isinstance(node, dict) and part in node:
            node = node[part]
        elif isinstance(node, list) and part.isdigit() and int(part) < len(node):
            node = node[int(part)]
        else:
            return False, None
    return True, node


def stem(listed: str) -> str:
    """core.md §9.4 (0.5): "the file's stem, the last path segment without a
    final .json"."""
    base = posixpath.basename(listed)
    return base[:-5] if base.endswith(".json") else base


@dataclass
class SchemaSet:
    """The artifacts of one contract, and its type-reference resolver."""

    contract_dir: Path
    json_files: list[Artifact] = field(default_factory=list)
    proto_files: list[Artifact] = field(default_factory=list)
    findings: list[Finding] = field(default_factory=list)
    #: core.md §9.2 cascade 2: "E029 for one schema kind suppresses E023 for
    #: that kind".
    broken: set[str] = field(default_factory=set)
    _proto_index: dict[str, list[Artifact]] = field(default_factory=dict)
    _proto_imported: set[str] = field(default_factory=set)
    _well_known: dict[str, Artifact] = field(default_factory=dict)
    used_well_known: dict[str, Artifact] = field(default_factory=dict)

    # -- loading -------------------------------------------------------

    @classmethod
    def load(cls, contract_dir: Path, jsonschema: list[str], protobuf: list[str],
             proto_include: list[str] | None) -> SchemaSet:
        s = cls(contract_dir)
        taken: set[str] = set()
        for listed in jsonschema:
            # §9.4 (0.5): a stem "MUST be unique among the listed files
            # (E024). A later file with a taken stem is not loaded." E024
            # falls once per such file (§9.2).
            if stem(listed) in taken:
                s._err("E024", f"{listed}: stem {stem(listed)!r} is already taken; not loaded")
                continue
            taken.add(stem(listed))
            s._load_json(listed)
        s._check_refs()
        roots = s._proto_roots(proto_include)
        for listed in protobuf:
            s._load_proto(listed, roots)
        return s

    def _err(self, code: str, message: str) -> None:
        self.findings.append(Finding(code, message))

    def _load_json(self, listed: str) -> None:
        path = self.contract_dir / listed
        try:
            raw = path.read_bytes()
        except OSError as e:
            self._err("E029", f"{listed}: unreadable: {e}")
            self.broken.add(JSON)
            return
        try:
            doc = jcs.loads(raw)
        except jcs.JsonError as e:
            self._err("E029", f"{listed}: not JSON: {e}")
            self.broken.add(JSON)
            return
        # §9.2 E028 "in a JSON Schema artifact … once per artifact", at load.
        _, big = jcs.restriction_violations(doc)
        art_id = None
        if big:
            self._err("E028", f"{listed}: an integer outside ±(2^53−1)")
        else:
            art_id = jcs.jcs_id(doc)
        # §7.3: a refused keyword is E037, once per keyword and file.
        for kw in refused_keywords(doc):
            self._err("E037", f"{listed}: keyword {kw!r} is outside the zk2 subset")
        self.json_files.append(Artifact(JSON, stem(listed), art_id, doc, listed))

    def _check_refs(self) -> None:
        """§9.4: a ``$ref``'s file part resolves relative to the referencing
        file, lexically normalized, and MUST name a listed file; a ``$ref``
        with a scheme is refused; its pointer MUST resolve. Each failure is
        E032 (per ``$ref``)."""
        by_path = {posixpath.normpath(a.listed): a for a in self.json_files}
        for art in self.json_files:
            def visit(obj: dict[str, Any], _path: str, art: Artifact = art) -> None:
                ref = obj.get("$ref")
                if ref is None:
                    return
                if not isinstance(ref, str) or self.resolve_ref(art, ref, by_path) is None:
                    self._err("E032", f"{art.listed}: $ref {ref!r} does not resolve")
            walk_schema(art.data, visit)

    def resolve_ref(self, art: Artifact, ref: str,
                    by_path: dict[str, Artifact] | None = None) -> tuple[Artifact, Any] | None:
        """(artifact, target schema) of a ``$ref`` in ``art``, or None."""
        if by_path is None:
            by_path = {posixpath.normpath(a.listed): a for a in self.json_files}
        file_part, _, fragment = ref.partition("#")
        if ":" in file_part:
            return None  # a scheme (§9.4)
        if file_part:
            target_path = posixpath.normpath(
                posixpath.join(posixpath.dirname(art.listed), file_part))
            target = by_path.get(target_path)
            if target is None:
                return None
        else:
            target = art
        # §9.4 (0.5): the fragment "is a JSON Pointer (RFC 6901), applied as
        # written: ~0 and ~1 are unescaped, and nothing is percent-decoded".
        found, node = json_pointer(target.data, fragment)
        return (target, node) if found else None

    def _proto_roots(self, proto_include: list[str] | None) -> list[str]:
        """§9.4: import roots are ``proto_include``, relative to the
        contract; otherwise ``proto/`` when it exists, else the contract's
        own directory."""
        if proto_include is not None:
            return [posixpath.normpath(r) for r in proto_include]
        return ["proto"] if (self.contract_dir / "proto").is_dir() else ["."]

    def _load_proto(self, listed: str, roots: list[str]) -> None:
        norm = posixpath.normpath(listed)
        if not (self.contract_dir / norm).is_file():
            self._err("E029", f"{listed}: missing")
            self.broken.add(PROTOBUF)
            return
        name = None
        for r in roots:
            if r == ".":
                name = norm
                break
            if norm.startswith(r + "/"):
                name = norm[len(r) + 1:]
                break
        if name is None:
            # §9.4 (0.5): a listed file under no import root has no name and
            # does not compile (E029); the artifact name is
            # the path relative to its import root, which it does not have.
            self._err("E029", f"{listed}: not under an import root {roots}")
            self.broken.add(PROTOBUF)
            return
        try:
            data = protoc.compile_file(name, [self.contract_dir / r for r in roots])
        except protoc.CompileError as e:
            self._err("E029", f"{listed}: does not compile: {e}")
            self.broken.add(PROTOBUF)
            return
        art = Artifact(PROTOBUF, name, jcs.sha256_id(data), data, listed)
        self.proto_files.append(art)
        for f in protoc.parse_set(data).file:
            names = protoc.message_names(f)
            if f.name == name:
                for m in names:
                    self._proto_index.setdefault(m, []).append(art)
            else:
                self._proto_imported |= names

    # -- resolution ----------------------------------------------------

    def artifacts(self) -> list[Artifact]:
        """§9.5: every listed artifact plus every well-known one in use."""
        return [*self.json_files, *self.proto_files, *self.used_well_known.values()]

    def resolve(self, ref: Any) -> tuple[dict[str, Any] | None, list[Finding]]:
        """Resolve one type reference (§7.1, §9.4) into its canonical type
        object (§9.5), or None with the findings that explain why. A
        reference whose schema kind is broken (E029) resolves to None with no
        finding (§9.2 cascade 2)."""
        if isinstance(ref, dict):
            mt = ref.get("raw")
            if not isinstance(mt, str) or MEDIA_TYPE.fullmatch(mt) is None:
                return None, [Finding("E023", f"raw type {mt!r} is not a media type")]
            return {"kind": RAW, "media_type": mt, "media_param": ref.get("media_param")}, []
        assert isinstance(ref, str)
        if ref.startswith("json:"):
            return self._resolve_json(ref)
        return self._resolve_proto(ref)

    def _resolve_json(self, ref: str) -> tuple[dict[str, Any] | None, list[Finding]]:
        if JSON in self.broken:
            return None, []
        body = ref[len("json:"):]
        if "#" in body:
            want_stem, _, name = body.partition("#")
            files = [a for a in self.json_files if a.name == want_stem]
            hits = [a for a in files[:1] if _defines(a, name)]
        else:
            name = body
            hits = [a for a in self.json_files if _defines(a, name)]
        if not hits:
            return None, [Finding("E023", f"{ref}: no listed file defines it")]
        if len(hits) > 1:
            return None, [Finding("E024", f"{ref}: defined by several listed files")]
        art = hits[0]
        return {"kind": JSON, "name": name, "schema": art.id}, []

    def _resolve_proto(self, ref: str) -> tuple[dict[str, Any] | None, list[Finding]]:
        hits = self._proto_index.get(ref, [])
        if len(hits) == 1:
            return {"kind": PROTOBUF, "name": ref, "schema": hits[0].id}, []
        if len(hits) > 1:
            return None, [Finding("E023", f"{ref}: defined by several listed files")]
        if ref in protoc.WELL_KNOWN:
            art = self.well_known(protoc.WELL_KNOWN[ref])
            self.used_well_known[art.name] = art
            return {"kind": PROTOBUF, "name": ref, "schema": art.id}, []
        if PROTOBUF in self.broken:
            return None, []
        if ref in self._proto_imported:
            return None, [Finding("E023", f"{ref}: only an imported file defines it")]
        return None, [Finding("E023", f"{ref}: no listed file defines it")]

    def well_known(self, file: str) -> Artifact:
        if file not in self._well_known:
            data = protoc.compile_well_known(file)
            name = f"google/protobuf/{file}"
            self._well_known[file] = Artifact(PROTOBUF, name, jcs.sha256_id(data), data, None)
        return self._well_known[file]

    # -- lookups used by other lints ----------------------------------

    def json_definition(self, type_obj: dict[str, Any]) -> tuple[Artifact, Any] | None:
        for a in self.json_files:
            if a.id == type_obj["schema"] and _defines(a, type_obj["name"]):
                return a, a.data["$defs"][type_obj["name"]]
        return None

    def proto_fields(self, type_obj: dict[str, Any]) -> list[str]:
        for a in [*self.proto_files, *self.used_well_known.values()]:
            if a.id == type_obj["schema"]:
                hit = protoc.find_message(protoc.parse_set(a.data), type_obj["name"])
                if hit is not None:
                    return [f.name for f in hit[1].field]
        return []


def _defines(art: Artifact, name: str) -> bool:
    defs = art.data.get("$defs") if isinstance(art.data, dict) else None
    return isinstance(defs, dict) and name in defs

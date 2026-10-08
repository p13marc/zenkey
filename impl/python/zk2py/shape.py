"""A JSON Schema checker for the two schemas the spec publishes.

core.md §9.1: the authoring format's shape is ``contract.schema.json``; "an
unknown table or field, a value of the wrong type, or text that is not TOML
is E000". core.md §3.3: the descriptor's schema is ``descriptor.schema.json``
(D000 in the fixtures). Rather than restating either shape by hand, zk2py
checks documents against the published files themselves.

Only the keywords those two files use are implemented. An unknown keyword is
an error in *this* code (``NotImplementedError``), so a schema change that
needs more is noticed rather than silently ignored.

``format`` is asserted for the integer formats the schemas use (``uint32``,
``uint64``): JSON Schema 2020-12 treats ``format`` as an annotation, but
core.md §9.1 bounds ``major`` to 0..2^32−1, and nothing else in the schema
does. See SPEC-FINDINGS (schema integer bounds).
"""

from __future__ import annotations

import datetime
import json
from pathlib import Path
from typing import Any

SPEC_DIR = Path(__file__).resolve().parents[3] / "spec"

_ANNOTATIONS = {"$schema", "$defs", "title", "description", "default", "$comment", "examples"}
_FORMATS = {"uint32": 2**32 - 1, "uint64": 2**64 - 1, "uint": 2**64 - 1}


def load_schema(name: str, spec_dir: Path | None = None) -> dict[str, Any]:
    path = (spec_dir or SPEC_DIR) / name
    return json.loads(path.read_text(encoding="utf-8"))


def _json_type(v: Any) -> str | None:
    if v is None:
        return "null"
    if isinstance(v, bool):
        return "boolean"
    if isinstance(v, int):
        return "integer"
    if isinstance(v, float):
        return "number"
    if isinstance(v, str):
        return "string"
    if isinstance(v, list):
        return "array"
    if isinstance(v, dict):
        return "object"
    if isinstance(v, (datetime.datetime, datetime.date, datetime.time)):
        return None  # a TOML datetime has no JSON type
    return None


def _type_ok(v: Any, t: str) -> bool:
    jt = _json_type(v)
    if t == "number":
        return jt in ("integer", "number")
    return jt == t


class Checker:
    def __init__(self, schema: dict[str, Any]):
        self.root = schema

    def _deref(self, ref: str) -> Any:
        if not ref.startswith("#/"):
            raise NotImplementedError(f"non-local $ref {ref!r}")
        node: Any = self.root
        for part in ref[2:].split("/"):
            node = node[part.replace("~1", "/").replace("~0", "~")]
        return node

    def errors(self, value: Any, schema: Any = None, path: str = "") -> list[str]:
        """Every violation of ``schema`` (default: the root) by ``value``."""
        schema = self.root if schema is None else schema
        if schema is True:
            return []
        if schema is False:
            return [f"{path or '/'}: not allowed"]
        errs: list[str] = []
        for kw, arg in schema.items():
            if kw in _ANNOTATIONS:
                continue
            if kw == "$ref":
                errs += self.errors(value, self._deref(arg), path)
            elif kw == "type":
                types = arg if isinstance(arg, list) else [arg]
                if not any(_type_ok(value, t) for t in types):
                    errs.append(f"{path or '/'}: expected {'|'.join(types)}")
            elif kw == "enum":
                if not any(value == e and type(value) is type(e) for e in arg):
                    errs.append(f"{path or '/'}: {value!r} not in {arg}")
            elif kw == "anyOf":
                if all(self.errors(value, s, path) for s in arg):
                    errs.append(f"{path or '/'}: matches no alternative")
            elif kw == "properties":
                if isinstance(value, dict):
                    for k, s in arg.items():
                        if k in value:
                            errs += self.errors(value[k], s, f"{path}/{k}")
            elif kw == "additionalProperties":
                if isinstance(value, dict):
                    known = schema.get("properties", {})
                    for k, v in value.items():
                        if k not in known:
                            errs += self.errors(v, arg, f"{path}/{k}")
            elif kw == "required":
                if isinstance(value, dict):
                    errs += [f"{path or '/'}: missing {k!r}" for k in arg if k not in value]
            elif kw == "items":
                if isinstance(value, list):
                    for i, v in enumerate(value):
                        errs += self.errors(v, arg, f"{path}/{i}")
            elif kw == "minimum":
                if _json_type(value) in ("integer", "number") and value < arg:
                    errs.append(f"{path or '/'}: below {arg}")
            elif kw == "format":
                bound = _FORMATS.get(arg)
                if bound is None:
                    raise NotImplementedError(f"format {arg!r}")
                if _json_type(value) == "integer" and not 0 <= value <= bound:
                    errs.append(f"{path or '/'}: outside {arg}")
            else:
                raise NotImplementedError(f"JSON Schema keyword {kw!r}")
        return errs

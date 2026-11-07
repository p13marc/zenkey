"""``config.v2``, text 0.1 (``spec/profiles/config/v2.md``, #985 G1): a
configuration declared in its contract, read back in one packed record
(#609, round 102). ``config.v1``'s rules hold by reference (§0); this module
holds what §2 replaces, and reuses :mod:`zk2py.config`'s order of refusals
where v2 keeps v1's rows.

The session-free half, from the text alone:
- **the declaration** (§2.2): ``config.groups`` read as ``derive.json``
  reads it, :func:`read_groups`, into :class:`Group`\\ s, or the reasons in
  order;
- **the derivation** (§3): ``Record``, one ``Set<T>`` per settable group,
  ``Control`` and ``Error``, :func:`derive`, and each one's layout;
- **the contract's lint** (§2.2, §3): :func:`lint`, its findings in the
  text's order;
- **the record** (§2.3, §3): an owner's state written as ``Record``'s JSON
  reading and its bytes, :func:`write_record`, and read back with config.v2's
  refusals after core §7.6's, :func:`read_record`;
- **validation** (§2.5, §2.10, §2.13, §2.18): :func:`check_request`, before
  any act, and :func:`check_desired`;
- **times** (§2.8): :func:`deadline_ms`, :func:`round_up`;
- **the owner** as a state machine, :class:`Core`: calls, device results and
  the owner's clock in; its puts, replies (value and error forms, their as-of
  marks), events and acts out, tokens of 48 bits, runs taken as their seq or
  counter is spent, first answers replayed byte for byte;
- **redaction** (§2.15): no bit of a sensitive value, a redacted edit and
  log line, no device words.

The session half, :class:`ConfigV2Owner`, serves a config contract over
:class:`zk2py.owner.Owner`; :class:`Caller` writes and reads its calls, and
:func:`read_put` reads a pushed record.
"""

from __future__ import annotations

import json
import re
import threading
import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Callable

from . import config as v1
from . import packed, zk2c

REPO = Path(__file__).resolve().parents[3]
PROFILE = REPO / "spec" / "profiles" / "config"
STANDARD_SCHEMA = PROFILE / "config.v2.json"
TEST_CONTRACT = PROFILE / "conformance" / "contracts" / "rfcfg.v1.toml"

PROFILE_ID = "config.v2"
MARKER = packed.MARKER
CLASSES = ("hot", "reach", "contract")
SETTABLE = ("hot", "reach")
SOURCES = ("default", "file", "overlay", "runtime")
OUTCOMES = ("applied", "confirmed", "rolled-back", "partial")
DESIRED_OUTCOMES = ("converged", "refused", "failed")
DESIRED_CODES = ("reach", "contract", "sensitive", "not_found", "invalid_request", "unavailable", "busy",
                 "device_refused")
ERROR_CODES = ("device_refused",)
NAME = re.compile(r"[a-z][a-z0-9]*(_[a-z0-9]+)*")
#: §2.2: "It is none of confirm_s, token, expected_revision,
#: idempotency_key, dry_run, writable, pending and last, which the derived
#: types use."
TAKEN_NAMES = ("confirm_s", "token", "expected_revision", "idempotency_key", "dry_run", "writable", "pending",
               "last")
ENTRY_MEMBERS = ("name", "class", "type", "units", "sensitive")
MAX_GROUPS = 16
MAX_PARAMS = 16
U32 = (1 << 32) - 1
U48 = (1 << 48) - 1
SEQ_MAX = (1 << 24) - 1
COUNTER_MAX = 65535
#: §2.14: "within 300 s".
KEY_TTL_MS = 300_000
RATE_N, RATE_PERIOD_MS = 600, 3_600_000
REDACTED = v1.REDACTED


# -- §2.2 the declaration ----------------------------------------------------------------

@dataclass(frozen=True)
class Param:
    """One parameter: a field of its group's type, ``int`` or ``bool``."""

    name: str
    kind: str                 # "int" or "bool"
    minimum: int | None
    maximum: int | None
    unit: str | None = None
    sensitive: bool = False
    description: str | None = None

    @property
    def width(self) -> int:
        if self.kind == "bool":
            return 1
        assert self.minimum is not None and self.maximum is not None
        return packed.bits_for(self.maximum - self.minimum + 1)

    def field(self, optional: bool) -> dict[str, Any]:
        """The parameter's field as the derived types carry it: its type and
        bounds, no description."""
        out: dict[str, Any] = {"name": self.name, "type": self.kind}
        if self.kind == "int":
            out.update(minimum=self.minimum, maximum=self.maximum)
        if optional:
            out["optional"] = True
        return out

    def declared(self) -> dict[str, Any]:
        """As ``derive.json`` writes a parameter."""
        return {"name": self.name, "kind": self.kind, "minimum": self.minimum, "maximum": self.maximum,
                "width": self.width, "unit": self.unit, "sensitive": self.sensitive}

    def holds(self, v: Any) -> bool:
        """§2.18: of its kind, a JSON integer with no fraction or exponent
        for an int, true or false for a bool, within its bounds."""
        if self.kind == "bool":
            return type(v) is bool
        return type(v) is int and self.minimum <= v <= self.maximum  # type: ignore[operator]


@dataclass(frozen=True)
class Group:
    name: str
    cls: str
    type_name: str
    params: tuple[Param, ...]

    @property
    def settable(self) -> bool:
        return self.cls in SETTABLE

    def param(self, name: str) -> Param | None:
        return next((p for p in self.params if p.name == name), None)

    def declared(self) -> dict[str, Any]:
        return {"name": self.name, "class": self.cls, "type": self.type_name, "settable": self.settable,
                "parameters": [p.declared() for p in self.params]}

    @classmethod
    def of_declared(cls, d: dict[str, Any]) -> "Group":
        """A group as ``derive.json`` (or ``records.json``'s growth) writes it."""
        return cls(d["name"], d["class"], d["type"],
                   tuple(Param(p["name"], p["kind"], p["minimum"], p["maximum"], p["unit"], p["sensitive"])
                         for p in d["parameters"]))


Item = Any  # a Group, or MARKER


@dataclass
class Declaration:
    """``config.groups`` read: the items in order, groups and markers, or the
    reasons it is invalid, each ``{entry, reason}``."""

    items: list[Item] = field(default_factory=list)
    reasons: list[dict[str, Any]] = field(default_factory=list)

    @property
    def valid(self) -> bool:
        return not self.reasons

    @property
    def groups(self) -> list[Group]:
        return [g for g in self.items if isinstance(g, Group)]

    def group(self, name: str) -> Group | None:
        return next((g for g in self.groups if g.name == name), None)

    def index(self, name: str) -> int | None:
        """§2.18: a group's index in the declaration, markers not counted
        (the field's bound, 0 to 15, holds the 16 groups and no marker)."""
        names = [g.name for g in self.groups]
        return names.index(name) if name in names else None

    def behind_marker(self, name: str) -> bool:
        seen = False
        for it in self.items:
            if it == MARKER:
                seen = True
            elif it.name == name:
                return seen
        return False

    def expect(self) -> dict[str, Any]:
        if not self.valid:
            return {"declaration": "invalid", "reasons": self.reasons}
        return {"declaration": "declared",
                "groups": [MARKER if it == MARKER else it.declared() for it in self.items]}

    @classmethod
    def of_declared(cls, items: list[Any]) -> "Declaration":
        return cls([MARKER if it == MARKER else Group.of_declared(it) for it in items])


class Docs:
    """The packed documents a declaration's types are named in, each by its
    stem (core §9.4 "Packed"): ``packed:Name`` names the type in the one
    document that defines it, ``packed:stem#Name`` looks in that stem's
    alone."""

    def __init__(self, docs: list[tuple[str, Any]]):
        self.docs = docs
        self.linted = {s: packed.lint(d) for s, d in docs}

    def resolve(self, ref: Any) -> tuple[str, str] | None:
        """(stem, name) of the one listed type ``ref`` names, or None."""
        if not isinstance(ref, str) or not ref.startswith("packed:"):
            return None
        body = ref[len("packed:"):]
        if "#" in body:
            want, _, name = body.partition("#")
            stems = [s for s, _ in self.docs if s == want]
        else:
            name, stems = body, [s for s, _ in self.docs]
        hits = [s for s in stems if name in self.linted[s].names]
        return (hits[0], name) if len(hits) == 1 else None

    def raw(self, at: tuple[str, str]) -> Any:
        doc = dict(self.docs)[at[0]]
        return doc.get("types", {}).get(at[1]) if isinstance(doc, dict) else None

    def clean(self, at: tuple[str, str]) -> bool:
        return at[1] in self.linted[at[0]].types


def _type_problem(raw: Any) -> str | None:
    """§2.2 "The group type": why a type is no group's, or None."""
    if not isinstance(raw, dict) or not isinstance(raw.get("fields"), list):
        return "not a record"
    fields = raw["fields"]
    if any(f == MARKER for f in fields):
        return "a marker"
    if len(fields) > MAX_PARAMS:
        return f"{len(fields)} fields, past {MAX_PARAMS}"
    for f in fields:
        if not isinstance(f, dict):
            return "an item that is no field"
        if f.get("optional"):
            return f"{f.get('name')!r} optional"
        if f.get("type") not in ("int", "bool"):
            return f"{f.get('name')!r} of type {f.get('type')!r}: int and bool alone at 0.1"
        if "reserved" in f:
            return f"{f.get('name')!r} with a reserved code"
        n = f.get("name")
        if not isinstance(n, str) or NAME.fullmatch(n) is None:
            return f"field name {n!r} not of a group name's form"
        if n in TAKEN_NAMES:
            return f"field name {n!r} is the derived types'"
    return None


def read_groups(value: Any, docs: Docs) -> Declaration:
    """§2.2 "Reading it": ``config.groups``'s value, as TOML reads it into
    JSON, against the packed documents its types are named in. The
    annotation whole first, then each item by its index, markers counted,
    its checks in the text's order, each made alone, one that needs a wrong
    value skipped."""
    out = Declaration()
    if not isinstance(value, list):
        out.reasons.append({"entry": None, "reason": "groups_value"})
        return out
    n_groups = sum(1 for it in value if it != MARKER)
    bad_markers = bool(value) and (value[0] == MARKER or value[-1] == MARKER
                                   or any(a == MARKER and b == MARKER for a, b in zip(value, value[1:])))
    if n_groups == 0 or n_groups > MAX_GROUPS or bad_markers:
        out.reasons.append({"entry": None, "reason": "groups_value"})
        return out
    names: list[str] = []
    types: list[tuple[str, str]] = []
    for i, it in enumerate(value):
        if it == MARKER:
            out.items.append(MARKER)
            continue
        reasons: list[str] = []
        if not isinstance(it, dict):
            out.reasons.append({"entry": i, "reason": "group_value"})
            continue
        if set(it) - set(ENTRY_MEMBERS):
            reasons.append("group_value")
        name = it.get("name")
        if not isinstance(name, str) or NAME.fullmatch(name) is None or name in names:
            reasons.append("group_name")
        if isinstance(name, str):
            names.append(name)
        if it.get("class") not in CLASSES:
            reasons.append("group_class")
        at = docs.resolve(it.get("type"))
        type_ok = at is not None and docs.clean(at) and _type_problem(docs.raw(at)) is None and at not in types
        if at is not None and at not in types:
            types.append(at)
        if not type_ok:
            reasons.append("group_type")
        params: list[Param] = []
        if type_ok:
            assert at is not None
            for f in docs.raw(at)["fields"]:
                params.append(Param(f["name"], f["type"], f.get("minimum"), f.get("maximum"),
                                    description=f.get("description")))
        units = it.get("units")
        if "units" in it:
            if not isinstance(units, dict):
                reasons.append("group_units")
            elif type_ok and any(not any(p.name == k and p.kind == "int" for p in params)
                                 or not isinstance(u, str) or not u for k, u in units.items()):
                reasons.append("group_units")
        sens = it.get("sensitive")
        if "sensitive" in it:
            if not isinstance(sens, list):
                reasons.append("group_sensitive")
            elif type_ok and (any(not isinstance(s, str) or all(p.name != s for p in params) for s in sens)
                              or len(set(map(str, sens))) != len(sens)):
                reasons.append("group_sensitive")
        if reasons:
            out.reasons += [{"entry": i, "reason": r} for r in reasons]
            continue
        units = units or {}
        sens = set(sens or [])
        out.items.append(Group(name, it["class"], at[1],  # type: ignore[index]
                               tuple(Param(p.name, p.kind, p.minimum, p.maximum, units.get(p.name), p.name in sens,
                                           p.description) for p in params)))
    if out.reasons:
        out.items = []
    return out


# -- §3 the derived types -----------------------------------------------------------------

def _int(name: str, lo: int, hi: int, optional: bool = False) -> dict[str, Any]:
    out: dict[str, Any] = {"name": name, "type": "int", "minimum": lo, "maximum": hi}
    if optional:
        out["optional"] = True
    return out


def _bool(name: str, optional: bool = False) -> dict[str, Any]:
    out: dict[str, Any] = {"name": name, "type": "bool"}
    if optional:
        out["optional"] = True
    return out


def _enum(name: str, values: tuple[str, ...], escape: bool, optional: bool = False,
          width: int | None = None) -> dict[str, Any]:
    out: dict[str, Any] = {"name": name, "type": "enum", "values": list(values)}
    if escape:
        out["escape"] = True
    if width is not None:
        out["width"] = width
    if optional:
        out["optional"] = True
    return out


HEADER = [
    _int("instance", 0, U32), _int("revision", 0, U32), _int("floor_s", 1, 65535), _bool("would_be"),
    _int("pending_token", 0, U48, True), {"name": "pending_apply_at", "type": "time", "optional": True},
    {"name": "pending_deadline", "type": "time", "optional": True}, _int("last_token", 0, U48, True),
    _enum("desired", DESIRED_OUTCOMES, True, True), _int("desired_revision", 0, U32, True),
    _enum("desired_code", DESIRED_CODES, True, True), _int("desired_group", 0, 15, True),
    _int("desired_parameter", 0, 15, True),
]


def record_fields(decl: Declaration) -> list[Any]:
    """§3 ``Record``: the header, then each group in the declaration's order,
    a marker for a marker, every field of a group behind one optional, then
    a marker."""
    out: list[Any] = [dict(f) for f in HEADER]
    behind = False
    for it in decl.items:
        if it == MARKER:
            out.append(MARKER)
            behind = True
            continue
        g: Group = it
        if g.settable:
            out += [_bool(f"{g.name}__{b}", behind) for b in ("writable", "pending", "last")]
        for p in g.params:
            out.append(_enum(f"{g.name}__{p.name}__source", SOURCES, False, True))
            if not p.sensitive:
                out.append(_bool(f"{g.name}__{p.name}__differs", behind))
                f = p.field(True)
                f["name"] = f"{g.name}__{p.name}"
                out.append(f)
    out.append(MARKER)
    return out


def set_fields(g: Group) -> list[Any]:
    """§3 ``Set<T>``: each parameter optional, then the request's own."""
    return [p.field(True) for p in g.params] + [
        _int("confirm_s", 1, 65535, True), _int("token", 0, U48, True), _int("expected_revision", 0, U32, True),
        _int("idempotency_key", 0, U32, True), _bool("dry_run")]


CONTROL = [_int("token", 0, U48), _int("confirm_s", 1, 65535, True)]
ERROR = [_enum("code", ERROR_CODES, True, width=4)]


def derive(decl: Declaration) -> dict[str, list[Any]]:
    """The derived types by name, in derivation order: ``Record``,
    ``Set<T>`` per hot or reach group in the groups' order, ``Control``,
    ``Error`` (§3; ``zk2 config derive``'s names, which the lint does not
    compare)."""
    out: dict[str, list[Any]] = {"Record": record_fields(decl)}
    for g in decl.groups:
        if g.settable:
            out[f"Set{g.type_name}"] = set_fields(g)
    out["Control"] = [dict(f) for f in CONTROL]
    out["Error"] = [dict(f) for f in ERROR]
    return out


def records_of(derived: dict[str, list[Any]]) -> dict[str, packed.Record]:
    lt = packed.lint({"types": {n: {"fields": f} for n, f in derived.items()}})
    if lt.findings:
        raise ValueError(f"a derived type does not lint clean: {lt.findings}")
    return lt.types


def layouts(derived: dict[str, list[Any]]) -> dict[str, dict[str, Any]]:
    out = {}
    for n, rec in records_of(derived).items():
        lo, hi = rec.bit_bounds()
        out[n] = {"tag": rec.tag, "sha256": rec.layout_sha256(), "min_bits": lo, "max_bits": hi}
    return out


def _normal(item: Any) -> Any:
    """A field as the lint compares it (§3: "field by field, names and
    descriptions of the types aside"): its name, type, width, optional and
    its type's members, defaults made explicit, its description left out."""
    if item == MARKER or not isinstance(item, dict):
        return item
    try:
        f = packed._field_of(item)  # noqa: SLF001
        width = f.width
    except Exception:  # noqa: BLE001 - a field the codec cannot read differs from every derivation
        return ("unreadable", json.dumps(item, sort_keys=True, default=str))
    out = {"name": item.get("name"), "type": item.get("type"), "width": width,
           "optional": bool(item.get("optional", False))}
    t = item.get("type")
    if t == "int":
        out.update(minimum=item.get("minimum"), maximum=item.get("maximum"), reserved=list(f.reserved))
    elif t == "enum":
        out.update(values=list(item.get("values", [])), escape=bool(item.get("escape", False)))
    elif t == "time":
        out["ms"] = bool(item.get("ms", False))
    elif t == "fixed":
        out.update(minimum=item.get("minimum"), maximum=item.get("maximum"), resolution=item.get("resolution"),
                   reserved=list(f.reserved))
    return out


def same_fields(a: list[Any], b: list[Any]) -> bool:
    return len(a) == len(b) and all(_normal(x) == _normal(y) for x, y in zip(a, b))


# -- §2.2 and §3 the contract's lint ------------------------------------------------------

_RETENTION = re.compile(r"(\d+)([smhd])")
_UNIT_S = {"s": 1, "m": 60, "h": 3600, "d": 86400}


def retention_s(v: Any) -> int | None:
    if isinstance(v, int) and not isinstance(v, bool):
        return v
    m = _RETENTION.fullmatch(v) if isinstance(v, str) else None
    return int(m.group(1)) * _UNIT_S[m.group(2)] if m else None


def standard_schema_id() -> str:
    from . import jcs

    return jcs.jcs_id(jcs.loads(STANDARD_SCHEMA.read_bytes()))


def lint(resources: dict[str, dict[str, Any]], uses: list[str], docs: Docs,
         standard_id: str | None = None) -> list[dict[str, str]]:
    """§2.2 "The contract's lint", on a contract's resources as
    ``derive.json`` resolves them: each by its template, its ``kind``,
    ``optional``, ``params``, merged ``annotations``, a state's or an event's
    ``type``, an operation's ``request``, ``response`` and ``error`` (a
    packed one a ``packed:`` reference, a JSON Schema one ``{kind, name,
    schema}``), ``fanout``, ``idempotent``, ``serving`` and ``replies``, an
    event's ``rate`` and ``retention``. Findings ``{at, reason}`` in order."""
    std = standard_id or standard_schema_id()
    out: list[dict[str, str]] = []
    cfg = resources.get("config")
    if cfg is None or "config.groups" not in (cfg.get("annotations") or {}):
        return [{"at": "config", "reason": "groups_missing"}]
    decl = read_groups(cfg["annotations"]["config.groups"], docs)
    if not decl.valid:
        return [{"at": "config", "reason": "groups_invalid"}]
    derived = derive(decl)
    settable = [g for g in decl.groups if g.settable]

    def packed_of(ref: Any) -> list[Any] | None:
        at = docs.resolve(ref)
        if at is None:
            return None
        raw = docs.raw(at)
        return raw.get("fields") if isinstance(raw, dict) else None

    def one_packed(ref: Any) -> bool:
        return isinstance(ref, str) and docs.resolve(ref) is not None

    def op_shape(r: dict[str, Any], optional: bool) -> bool:
        return (r.get("kind") == "operation" and not r.get("params") and bool(r.get("optional")) == optional
                and r.get("fanout") == "forbidden" and r.get("idempotent") is False
                and r.get("serving") == "exclusive" and r.get("replies") == "one"
                and all(one_packed(r.get(role)) for role in ("request", "response", "error")))

    def derived_ok(r: dict[str, Any], want: dict[str, list[Any]]) -> bool:
        for role, fields in want.items():
            got = packed_of(r.get(role))
            if got is None or not same_fields(got, fields):
                return False
        return True

    expected: list[tuple[str, str, dict[str, list[Any]]]] = [("config", "state", {"type": derived["Record"]})]
    for g in settable:
        expected.append((f"set/{g.name}", "op", {"request": derived[f"Set{g.type_name}"],
                                                 "response": derived["Record"], "error": derived["Error"]}))
    for t in ("confirm", "cancel", "extend", "persist"):
        expected.append((t, "op", {"request": derived["Control"], "response": derived["Record"],
                                   "error": derived["Error"]}))
    expected.append(("change", "event", {}))
    for tmpl, kind, want in expected:
        r = resources.get(tmpl)
        if r is None:
            out.append({"at": tmpl, "reason": "resource_missing"})
        else:
            if kind == "state":
                shape = r.get("kind") == "state" and not r.get("params") and not r.get("optional") \
                    and one_packed(r.get("type"))
            elif kind == "op":
                shape = op_shape(r, tmpl == "persist")
            else:
                t = r.get("type")
                shape = (r.get("kind") == "event" and not r.get("params") and not r.get("optional")
                         and isinstance(t, dict) and t.get("kind") == "jsonschema"
                         and t.get("name") == "ConfigChangeEvent" and t.get("schema") == std
                         and r.get("rate") == "burst(600/h)" and retention_s(r.get("retention")) == 7 * 86400)
            if not shape:
                out.append({"at": tmpl, "reason": "resource_shape"})
            elif want and not derived_ok(r, want):
                out.append({"at": tmpl, "reason": "config_derivation"})
        if tmpl == "config":
            ttl = (cfg.get("annotations") or {}).get("freshness.ttl_s")
            ok = isinstance(ttl, (int, float)) and not isinstance(ttl, bool) and ttl > 0 and "freshness.v2" in uses
            if not ok:
                out.append({"at": "config", "reason": "freshness"})
    sets = {f"set/{g.name}" for g in settable}
    for tmpl in sorted(resources, key=lambda s: s.encode("utf-8")):
        if (tmpl == "set" or tmpl.startswith("set/")) and tmpl not in sets:
            out.append({"at": tmpl, "reason": "set_not_settable"})
    for tmpl in sorted(resources, key=lambda s: s.encode("utf-8")):
        if tmpl != "config" and "config.groups" in (resources[tmpl].get("annotations") or {}):
            out.append({"at": tmpl, "reason": "groups_place"})
    return out


def resolved(contract: Any) -> tuple[dict[str, dict[str, Any]], list[str], Docs]:
    """A loaded :class:`zk2py.contract.Contract` in the shape :func:`lint`
    reads: its resources by template, a packed type as ``packed:<stem>#<name>``,
    and its packed documents by stem."""
    sch = contract.schemas
    docs = Docs([(a.name, a.data) for a in sch.packed_files]) if sch is not None else Docs([])
    stems = {a.id: a.name for a in sch.packed_files} if sch is not None else {}

    def ref(t: Any) -> Any:
        if isinstance(t, dict) and t.get("kind") == "packed":
            return f"packed:{stems.get(t['schema'], '?')}#{t['name']}"
        return t

    out: dict[str, dict[str, Any]] = {}
    for r in contract.canonical["resources"]:
        d = {"kind": r["kind"], "optional": r.get("optional", False), "params": list(r.get("params") or {}),
             "annotations": r.get("annotations") or {}}
        if r["kind"] == "operation":
            d.update(request=ref(r.get("request")), response=ref(r.get("response")), error=ref(r.get("error")),
                     fanout=r.get("fanout"), idempotent=r.get("idempotent"), serving=r.get("serving"),
                     replies=r.get("replies"))
        else:
            d["type"] = ref(r.get("type"))
        if r["kind"] == "event":
            d.update(rate=r.get("rate"), retention=r.get("retention_s"))
        out[r["template"]] = d
    return out, list(contract.canonical.get("uses") or []), docs


def uses_config_v2(contract: Any) -> bool:
    return PROFILE_ID in (contract.canonical.get("uses") or [])


# -- §2.3 the record -----------------------------------------------------------------------

class RecordRefused(ValueError):
    def __init__(self, token: str, why: str = ""):
        super().__init__(f"{token}: {why}" if why else token)
        self.token = token


@dataclass
class Layout:
    """One config contract's declaration and its derived records."""

    decl: Declaration
    derived: dict[str, list[Any]]
    records: dict[str, packed.Record]

    @classmethod
    def of(cls, decl: Declaration) -> "Layout":
        d = derive(decl)
        return cls(decl, d, records_of(d))

    @classmethod
    def of_contract(cls, contract: Any) -> "Layout":
        res, _, docs = resolved(contract)
        decl = read_groups(res["config"]["annotations"]["config.groups"], docs)
        if not decl.valid:
            raise ValueError(f"config.groups is invalid: {decl.reasons}")
        return cls.of(decl)

    @property
    def record(self) -> packed.Record:
        return self.records["Record"]

    def set_type(self, group: str) -> packed.Record:
        g = self.decl.group(group)
        assert g is not None
        return self.records[f"Set{g.type_name}"]

    @property
    def control(self) -> packed.Record:
        return self.records["Control"]

    @property
    def error(self) -> packed.Record:
        return self.records["Error"]


def state_fields(layout: Layout, state: dict[str, Any]) -> dict[str, Any]:
    """An owner's state (``records.json``'s ``state``) as ``Record``'s JSON
    reading: an absent optional field no member, a time in milliseconds
    since the Unix epoch."""
    f: dict[str, Any] = {k: state[k] for k in ("instance", "revision", "floor_s", "would_be")}
    p = state.get("pending")
    if p is not None:
        f["pending_token"] = p["token"]
        if p.get("apply_at") is not None:
            f["pending_apply_at"] = p["apply_at"]
        f["pending_deadline"] = p["deadline"]
    last = state.get("last")
    if last is not None:
        f["last_token"] = last["token"]
    d = state.get("desired")
    if d is not None:
        f["desired"] = d["outcome"]
        f["desired_revision"] = d["observed_revision"]
        r = d.get("refusal")
        if r is not None:
            f["desired_code"] = r["code"]
            if r.get("group") is not None:
                gi = layout.decl.index(r["group"])
                if gi is not None:
                    f["desired_group"] = gi
                    g = layout.decl.groups[gi]
                    if r.get("parameter") is not None:
                        pi = next((i for i, q in enumerate(g.params) if q.name == r["parameter"]), None)
                        if pi is not None:
                            f["desired_parameter"] = pi
    pend = set((p or {}).get("groups") or [])
    lastg = set((last or {}).get("groups") or [])
    for g in layout.decl.groups:
        gs = state["groups"].get(g.name)
        if gs is None:
            continue
        if g.settable:
            f[f"{g.name}__writable"] = bool(gs.get("writable"))
            f[f"{g.name}__pending"] = g.name in pend
            f[f"{g.name}__last"] = g.name in lastg
        for prm in g.params:
            ps = gs["params"][prm.name]
            if ps.get("source") is not None:
                f[f"{g.name}__{prm.name}__source"] = ps["source"]
            if not prm.sensitive:
                f[f"{g.name}__{prm.name}__differs"] = bool(ps.get("differs"))
                if ps.get("value") is not None:
                    f[f"{g.name}__{prm.name}"] = ps["value"]
    return f


def write_record(layout: Layout, state: dict[str, Any], epoch: int = 0) -> bytes:
    return layout.record.write(state_fields(layout, state), epoch)


def fields_state(layout: Layout, f: dict[str, Any]) -> dict[str, Any]:
    """``Record``'s JSON reading as an owner's state, after config.v2's
    refusals (:func:`check_fields`). A group behind a marker with none of
    its fields is absent: an earlier revision's writer's."""
    st: dict[str, Any] = {k: f[k] for k in ("instance", "revision", "floor_s", "would_be")}
    groups = layout.decl.groups
    pend = [g.name for g in groups if f.get(f"{g.name}__pending")]
    lastg = [g.name for g in groups if f.get(f"{g.name}__last")]
    if "pending_token" in f:
        st["pending"] = {"token": f["pending_token"], "groups": pend, "apply_at": f.get("pending_apply_at"),
                         "deadline": f["pending_deadline"], "applied": "pending_apply_at" not in f}
    else:
        st["pending"] = None
    st["last"] = {"token": f["last_token"], "groups": lastg} if "last_token" in f else None
    if "desired" in f:
        refusal = None
        if "desired_code" in f:
            gname = pname = None
            if "desired_group" in f:
                g = groups[f["desired_group"]]
                gname = g.name
                if "desired_parameter" in f:
                    pname = g.params[f["desired_parameter"]].name
            refusal = {"code": f["desired_code"], "group": gname, "parameter": pname}
        st["desired"] = {"outcome": f["desired"], "observed_revision": f["desired_revision"], "refusal": refusal}
    else:
        st["desired"] = None
    gs: dict[str, Any] = {}
    for g in groups:
        if not _group_present(g, f):
            continue
        d: dict[str, Any] = {}
        if g.settable:
            d["writable"] = bool(f.get(f"{g.name}__writable"))
        params = {}
        for prm in g.params:
            src = f.get(f"{g.name}__{prm.name}__source")
            if prm.sensitive:
                params[prm.name] = {"source": src}
            else:
                params[prm.name] = {"source": src, "differs": bool(f.get(f"{g.name}__{prm.name}__differs")),
                                    "value": f.get(f"{g.name}__{prm.name}")}
        d["params"] = params
        gs[g.name] = d
    st["groups"] = gs
    return st


def _group_fields(g: Group) -> list[str]:
    out = [f"{g.name}__{b}" for b in ("writable", "pending", "last")] if g.settable else []
    for p in g.params:
        out.append(f"{g.name}__{p.name}__source")
        if not p.sensitive:
            out += [f"{g.name}__{p.name}__differs", f"{g.name}__{p.name}"]
    return out


def _group_present(g: Group, f: dict[str, Any]) -> bool:
    return any(n in f for n in _group_fields(g))


def check_fields(layout: Layout, f: dict[str, Any]) -> str | None:
    """§2.3's refusals after core §7.6's, the first that applies: ``group``,
    ``pending``, ``last``, ``desired``, ``parameter``; None when the record
    holds together."""
    decl = layout.decl
    groups = decl.groups
    # group: a group behind a marker written in part.
    for g in groups:
        if not decl.behind_marker(g.name) or not _group_present(g, f):
            continue
        bits = [f"{g.name}__{b}" for b in ("writable", "pending", "last")] if g.settable else []
        if any(b not in f for b in bits):
            return "group"
        for p in g.params:
            if not p.sensitive and f"{g.name}__{p.name}" in f and f"{g.name}__{p.name}__differs" not in f:
                return "group"
    # pending
    pend_bits = any(f.get(f"{g.name}__pending") for g in groups if g.settable)
    if ("pending_token" in f) != ("pending_deadline" in f):
        return "pending"
    if ("pending_apply_at" in f or pend_bits) and "pending_token" not in f:
        return "pending"
    if "pending_token" in f and not pend_bits:
        return "pending"
    # last
    last_bits = any(f.get(f"{g.name}__last") for g in groups if g.settable)
    if ("last_token" in f) != last_bits:
        return "last"
    # desired
    if "desired" not in f:
        if any(k in f for k in ("desired_revision", "desired_code", "desired_group", "desired_parameter")):
            return "desired"
    else:
        if "desired_revision" not in f:
            return "desired"
        out = f["desired"]
        if out in ("refused", "failed") and "desired_code" not in f:
            return "desired"
        if out == "converged" and "desired_code" in f:
            return "desired"
        if "desired_group" in f and "desired_code" not in f:
            return "desired"
        if "desired_parameter" in f and "desired_group" not in f:
            return "desired"
        if "desired_group" in f:
            gi = f["desired_group"]
            if gi >= len(groups):
                return "desired"
            if "desired_parameter" in f and f["desired_parameter"] >= len(groups[gi].params):
                return "desired"
    # parameter
    for g in groups:
        for p in g.params:
            if p.sensitive:
                continue
            has_v = f"{g.name}__{p.name}" in f
            has_s = f"{g.name}__{p.name}__source" in f
            if has_v != has_s or (f.get(f"{g.name}__{p.name}__differs") and not has_v):
                return "parameter"
    return None


def read_record(layout: Layout, tag: int, record: bytes, epoch: int = 0) -> dict[str, Any]:
    """A record read as §2.3 says: core §7.6's refusals, then config.v2's.
    Returns the state, or raises :class:`packed.Refused` or
    :class:`RecordRefused`."""
    f = layout.record.read(tag, record, epoch)
    why = check_fields(layout, f)
    if why is not None:
        raise RecordRefused(why, "the record's fields do not hold together (config.v2 §2.3)")
    return fields_state(layout, f)


# -- §2.8 times --------------------------------------------------------------------------

def round_up(ms: int) -> int:
    """§2.8: "round … up to the next whole second"; an instant on a whole
    second stays."""
    return -(-ms // 1000) * 1000


def window_s(confirm_s: int, floor_s: int) -> int:
    return max(confirm_s, floor_s)


def deadline_ms(received_ms: int, confirm_s: int, floor_s: int) -> int:
    """§2.8: the receipt plus the window, rounded up, so a window is never
    shorter than asked."""
    return round_up(received_ms + window_s(confirm_s, floor_s) * 1000)


def time_code(ms: int, epoch: int) -> int:
    """The Record's time field (core §7.6): seconds since the epoch."""
    return (ms - 1000 * epoch) // 1000


# -- §2.5 validating a request -------------------------------------------------------------

@dataclass
class Refusal:
    """A refusal before or after the device: core §5.2's code, its cause on
    ``unavailable``, and ``app``, the ``Error`` code on ``app``; ``message``
    the device's words where they may be sent (§2.5, §2.15)."""

    code: str
    cause: str | None = None
    app: str | None = None
    message: str | None = None

    def expect(self) -> dict[str, Any]:
        return {"code": self.code, "cause": self.cause, "app": self.app}


@dataclass
class Facts:
    """What the rows read of the owner (``validation.json``'s ``state``)."""

    revision: int = 0
    pending_token: int | None = None
    pending_applied: bool = True
    deadline_passed: bool = False
    last_token: int | None = None
    last_sensitive: bool = False
    persist: bool = True
    in_flight: bool = False
    writable: tuple[str, ...] = ()

    @classmethod
    def of(cls, s: dict[str, Any]) -> "Facts":
        p, last = s.get("pending"), s.get("last")
        return cls(s["revision"], None if p is None else p["token"], True if p is None else p["applied"],
                   bool(p and p["deadline_passed"]), None if last is None else last["token"],
                   bool(last and last["set_sensitive"]), s["persist"], s["in_flight"], tuple(s["writable"]))


def parse_request(layout: Layout, op: str, group: str | None, payload: bytes) -> dict[str, Any]:
    """§2.5 "Before the device sees anything": core §7.5's refusals for a
    query, then core §7.6's for the operation's type (``Set<T>`` or
    ``Control``). Raises :class:`zk2c.Refused` or :class:`packed.Refused`."""
    env = zk2c.parse(payload, "query")
    rec = layout.set_type(group) if op == "set" else layout.control  # type: ignore[arg-type]
    return rec.read(env.tag, env.record or b"")


def check_set(layout: Layout, group: str, req: dict[str, Any], f: Facts) -> Refusal | None:
    """§2.5's rows 1, 4 to 7, 9 and 10, after the parse; row 2 reads the
    keys held, and rows 12 to 14 the owner's rate, deadline and device:
    :class:`Core`'s."""
    g = layout.decl.group(group)
    assert g is not None and g.settable
    if f.in_flight:
        return Refusal("busy")
    if group not in f.writable:
        return Refusal("unavailable", cause="config")
    token = req.get("token")
    if f.pending_token is not None and (token != f.pending_token or f.deadline_passed):
        return Refusal("busy")
    if token is not None and f.pending_token is None:
        return Refusal("not_found")
    if "expected_revision" in req and req["expected_revision"] != f.revision:
        return Refusal("invalid_request")
    if token is not None and "confirm_s" in req:
        return Refusal("invalid_request")
    if g.cls == "reach" and "confirm_s" not in req and token is None and not req.get("dry_run"):
        return Refusal("invalid_request")
    return None


def _v1_refusal(r: Any) -> Refusal | None:
    return None if r is None else Refusal(r.code, r.cause)


def check_control(op: str, req: dict[str, Any], f: Facts) -> Refusal | None:
    """§2.10: v1 §2.10's order, after the parse."""
    return _v1_refusal(v1.check_control(op, req, v1.Facts(
        revision=f.revision, in_flight=f.in_flight, pending_token=f.pending_token,  # type: ignore[arg-type]
        pending_applied=f.pending_applied, deadline_passed=f.deadline_passed)))


def check_persist(req: dict[str, Any], f: Facts) -> Refusal | None:
    """§2.13: v1 §2.13's order after its row 1 and the parse."""
    return _v1_refusal(v1.check_persist(req, v1.Facts(
        revision=f.revision, in_flight=f.in_flight, pending_token=f.pending_token,  # type: ignore[arg-type]
        pending_applied=f.pending_applied, deadline_passed=f.deadline_passed,
        last_token=f.last_token, last_sensitive=f.last_sensitive, persist=f.persist)))  # type: ignore[arg-type]


def check_request(layout: Layout, op: str, group: str | None, payload: bytes, f: Facts
                  ) -> tuple[dict[str, Any] | None, Refusal | None]:
    """One request before any act (``validation.json``): persist without a
    layer first, then the parse, then the rows. Returns (the request's JSON
    reading or None, the refusal or None)."""
    if op == "persist" and not f.persist:
        return None, Refusal("unavailable", cause="config")
    try:
        req = parse_request(layout, op, group, payload)
    except (zk2c.Refused, packed.Refused):
        return None, Refusal("invalid_request")
    if op == "set":
        assert group is not None
        return req, check_set(layout, group, req, f)
    if op == "persist":
        return req, check_persist(req, f)
    return req, check_control(op, req, f)


def check_desired(layout: Layout, document: dict[str, Any], writable: tuple[str, ...]) -> dict[str, Any] | None:
    """§2.18: a revision above 32 bits first, ``invalid_request`` naming no
    group; then v1 §2.18's order, groups in the byte order of their names,
    each group's checks, then each parameter's in the byte order of its
    name. A group or parameter the contract does not declare is named None.
    Returns ``{code, group, parameter}`` or None."""
    rev = document.get("revision")
    if type(rev) is int and rev > U32:
        return {"code": "invalid_request", "group": None, "parameter": None}
    values = document.get("values") or {}
    for gname in v1.bytewise(values):
        g = layout.decl.group(gname)
        if g is None:
            return {"code": "not_found", "group": None, "parameter": None}
        for code, hit in (("contract", g.cls == "contract"), ("reach", g.cls == "reach"),
                          ("unavailable", gname not in writable)):
            if hit:
                return {"code": code, "group": gname, "parameter": None}
        for pname in v1.bytewise(values[gname]):
            p = g.param(pname)
            if p is None:
                return {"code": "not_found", "group": gname, "parameter": None}
            if p.sensitive:
                return {"code": "sensitive", "group": gname, "parameter": pname}
            if not p.holds(values[gname][pname]):
                return {"code": "invalid_request", "group": gname, "parameter": pname}
    return None


def error_form(layout: Layout, stamp: zk2c.InBand, r: Refusal) -> bytes:
    """§2.5 "The error form": core §7.5's, ``Error``'s tag, the mark; a
    detail ``Error{code}`` on ``app`` alone; a message where one is sent."""
    detail = layout.error.write({"code": r.app}) if r.code == "app" else None
    return zk2c.write_error(stamp, r.code, r.cause, r.message, detail, layout.error.tag)


def value_form(layout: Layout, stamp: zk2c.InBand, record: bytes) -> bytes:
    """§2.9: the record in core §7.5's value form, ``Record``'s tag."""
    return zk2c.value(stamp, record, layout.record.tag)


# -- the owner, as a state machine ----------------------------------------------------------

UNKNOWN = v1.UNKNOWN


@dataclass
class Running:
    value: Any
    source: str | None


@dataclass
class Wait:
    group: str
    values: dict[str, Any]
    apply_at: int


@dataclass
class Change:
    """A change (v1 §2.6, §2.7): its 48-bit token, the groups that joined,
    its deadline (None without a window), the starting call's metadata
    (core O7), each parameter's value before it and the last value it set,
    what an apply of it moved, and its reach applies still waiting, in the
    order they joined."""

    token: int
    groups: set[str]
    deadline: int | None
    call: dict[str, str]
    before: dict[tuple[str, str], Running] = field(default_factory=dict)
    new: dict[tuple[str, str], Any] = field(default_factory=dict)
    applied: set[tuple[str, str]] = field(default_factory=set)
    waits: list[Wait] = field(default_factory=list)
    sensitive: bool = False


@dataclass
class Kept:
    """A first answer under an idempotency key (§2.14): its group, the
    request's bytes, the answer's bytes as first sent (its mark included),
    and when."""

    group: str
    request: bytes
    form: str        # "value" or "error"
    wire: bytes
    at: int


@dataclass
class Put:
    run: tuple[int, int]
    seq: int
    state: dict[str, Any]
    payload: bytes


@dataclass
class Reply:
    """A reply: its form (``value`` or ``error``), its as-of mark, the
    record's state or the refusal, and its bytes."""

    form: str
    run: tuple[int, int]
    seq: int
    payload: bytes
    state: dict[str, Any] | None = None
    refusal: Refusal | None = None
    replay: bool = False


@dataclass
class Event:
    run: tuple[int, int]
    seq: int
    event: dict[str, Any]


@dataclass
class Step:
    acts: list[str] = field(default_factory=list)
    puts: list[Put] = field(default_factory=list)
    reply: Reply | None = None
    events: list[Event] = field(default_factory=list)
    #: the request as the owner logged it (§2.15), None for one that did
    #: not parse
    logged: dict[str, Any] | None = None
    #: a first answer made now and kept under its key (§2.14)
    kept: Kept | None = None


def _meta(call: dict[str, Any] | None) -> dict[str, str]:
    if not isinstance(call, dict):
        return {}
    return {k: call[k] for k in ("actor", "request_id") if isinstance(call.get(k), str)}


class Core:
    """One ``config.v2`` owner, from the text: calls (their request's bytes),
    device results and the owner's clock (milliseconds since the Unix epoch)
    in; puts, replies, events and device acts out.

    - ``next_run``: the run the owner takes next, ``(width, run)``: at the
      start, when its seq or its token counter is spent (core §4.3, §2.6),
      and at a restart;
    - ``next_counter``: the token counter a run starts at (§2.6: SHOULD be
      drawn at random);
    - ``startup``: ``{group: {parameter: {value, source}}}``; ``overlay``,
      the persisted layer, kept by reference so a restart starts from it;
    - ``device``: a function of an act, ``("apply", group, values)`` or
      ``("undo", {group: {parameter: value}})``, answering ``"ok"``,
      ``{"refused": words}`` or ``{"failed": words, "readback": …}``;
    - ``events_at``: the instants of events published, kept by reference so
      the rate survives a restart."""

    def __init__(self, layout: Layout, *, service: str, next_run: Callable[[], tuple[int, int]],
                 next_counter: Callable[[], int], startup: dict[str, dict[str, dict[str, Any]]],
                 overlay: dict[str, dict[str, Any]] | None = None, floor_s: int = 10, apply_after_ms: int = 1000,
                 persist: bool = True, writable: list[str] | tuple[str, ...] = (), epoch: int = 0,
                 device: Callable[[tuple], Any] | None = None, events_at: list[int] | None = None,
                 seq_start: int = 0, begin: bool = True):
        if not 1 <= floor_s <= 65535:
            raise ValueError("the floor is from 1 to 65,535 s (§2.8)")
        self.layout, self.service = layout, service
        self.next_run, self.next_counter = next_run, next_counter
        self.startup_file = startup
        self.overlay = overlay if overlay is not None else {}
        self.floor_s, self.apply_after_ms, self.persist_layer = floor_s, apply_after_ms, persist
        self.writable = tuple(writable)
        self.epoch = epoch
        self.device = device or (lambda act: "ok")
        self.events_at = events_at if events_at is not None else []
        self.log: list[str] = []
        self.restarts = 0
        self._seq_start = seq_start
        #: the call being answered, for a live owner's :meth:`_emit`
        self.call: Any = None
        self.start_step = self._start() if begin else Step()

    # -- runs, seqs and tokens ------------------------------------------------------------

    def _take_run(self) -> None:
        self.run = self.next_run()
        self.next_seq = 0
        self.last_seq: int | None = None
        self.counter = self.next_counter()

    def _mint_seq(self) -> int:
        if self.next_seq > SEQ_MAX:
            self._take_run()
        seq = self.next_seq
        self.next_seq += 1
        self.last_seq = seq
        return seq

    def mark(self) -> zk2c.InBand:
        """Core §5.1's as-of mark: the run written under, and the last seq
        minted under it, or 0."""
        return zk2c.InBand(self.run[0], self.run[1], 0 if self.last_seq is None else self.last_seq)

    def _mint_token(self) -> int:
        """§2.6: ``run × 65536 + counter``; before the counter would pass
        65,535, a new run, whose counter starts afresh."""
        if self.counter > COUNTER_MAX:
            self._take_run()
        token = self.run[1] * 65536 + self.counter
        self.counter += 1
        return token

    # -- a new instance (§2.12) ----------------------------------------------------------

    def _start(self) -> Step:
        self._take_run()
        if self._seq_start:
            self.next_seq = self._seq_start
            self._seq_start = 0
        self.instance = self.run[1]
        self.revision = 0
        self.pending: Change | None = None
        self.last_change: dict[str, Any] | None = None
        self.keys: dict[int, Kept] = {}
        self.desired: dict[str, Any] | None = None
        self.running: dict[tuple[str, str], Running] = {}
        for g in self.layout.decl.groups:
            for p in g.params:
                v, src = self.startup_value(g.name, p.name)
                self.running[(g.name, p.name)] = Running(v, src)
        self._put_state: dict[str, Any] | None = None
        step = Step()
        self._put(step)
        return step

    def restart(self) -> Step:
        """A new instance: the persisted layer and the events' instants
        survive it; nothing else does (v1 §2.12)."""
        self.restarts += 1
        return self._start()

    def startup_value(self, group: str, param: str) -> tuple[Any, str | None]:
        if param in self.overlay.get(group, {}):
            return self.overlay[group][param], "overlay"
        s = self.startup_file.get(group, {}).get(param)
        if s is None:
            return UNKNOWN, None
        return s["value"], s["source"]

    # -- the record ------------------------------------------------------------------------

    def _sensitive(self, group: str, param: str) -> bool:
        g = self.layout.decl.group(group)
        p = None if g is None else g.param(param)
        return bool(p and p.sensitive)

    def state(self, running: dict[tuple[str, str], Running] | None = None, would_be: bool = False) -> dict[str, Any]:
        """The record, as ``records.json`` writes a state."""
        running = self.running if running is None else running
        order = [g.name for g in self.layout.decl.groups]
        p = self.pending
        pend = None
        if p is not None:
            waits = [w for w in p.waits]
            pend = {"token": p.token, "groups": [n for n in order if n in p.groups],
                    "apply_at": waits[-1].apply_at if waits else None, "deadline": p.deadline,
                    "applied": not waits}
        last = None
        if self.last_change is not None:
            last = {"token": self.last_change["token"],
                    "groups": [n for n in order if n in self.last_change["groups"]]}
        groups: dict[str, Any] = {}
        for g in self.layout.decl.groups:
            d: dict[str, Any] = {}
            if g.settable:
                d["writable"] = g.name in self.writable
            params = {}
            for prm in g.params:
                cur = running[(g.name, prm.name)]
                if prm.sensitive:
                    params[prm.name] = {"source": cur.source}
                    continue
                start, _ = self.startup_value(g.name, prm.name)
                known = cur.value is not UNKNOWN
                differs = known and start is not UNKNOWN and not v1.json_equal(start, cur.value)
                params[prm.name] = {"source": cur.source if known else None, "differs": bool(differs),
                                    "value": cur.value if known else None}
            d["params"] = params
            groups[g.name] = d
        return {"instance": self.instance, "revision": self.revision, "floor_s": self.floor_s,
                "would_be": would_be, "pending": pend, "last": last,
                "desired": None if self.desired is None else json.loads(json.dumps(self.desired)),
                "groups": groups}

    def record_bytes(self, state: dict[str, Any]) -> bytes:
        return write_record(self.layout, state, self.epoch)

    def _put(self, step: Step) -> None:
        """§2.3: on every change of any field, and at nothing else."""
        st = self.state()
        if st == self._put_state:
            return
        seq = self._mint_seq()
        rec = self.record_bytes(st)
        payload = zk2c.write("mutation", self.run[0], self.run[1], seq, self.layout.record.tag, rec)
        self._put_state = st
        step.puts.append(Put(self.run, seq, st, payload))

    def _reply_record(self, step: Step, state: dict[str, Any] | None = None) -> Reply:
        st = self.state() if state is None else state
        m = self.mark()
        r = Reply("value", (m.width, m.run), m.seq, value_form(self.layout, m, self.record_bytes(st)), state=st)
        step.reply = r
        self._emit(r)
        return r

    def _reply_refusal(self, step: Step, r: Refusal) -> Reply:
        m = self.mark()
        rep = Reply("error", (m.width, m.run), m.seq, error_form(self.layout, m, r), refusal=r)
        step.reply = rep
        self._emit(rep)
        return rep

    def _replay(self, step: Step, kept: Kept) -> None:
        """§2.14: the first answer's bytes, its mark included."""
        m = zk2c.parse(kept.wire, "reply_err" if kept.form == "error" else "reply")
        step.reply = Reply(kept.form, (m.width, m.run), m.seq, kept.wire, replay=True)  # type: ignore[arg-type]
        self._emit(step.reply)

    def _emit(self, reply: Reply) -> None:
        """A reply sent as it is made, before the event that follows it
        (§2.17): a live owner's (:class:`LiveCore`); nothing here."""

    # -- events (§2.17) -------------------------------------------------------------------

    def _rate_spent(self, now: int) -> bool:
        return sum(1 for t in self.events_at if now - RATE_PERIOD_MS < t <= now) >= RATE_N

    def _event(self, change: Change, outcome: str, step: Step, now: int) -> None:
        edits = []
        for g in self.layout.decl.groups:
            if g.name not in change.groups:
                continue
            for prm in g.params:
                key = (g.name, prm.name)
                if key not in change.new:
                    continue
                if prm.sensitive:
                    edits.append({"group": g.name, "parameter": prm.name, "redacted": True})
                    continue
                e: dict[str, Any] = {"group": g.name, "parameter": prm.name}
                old = change.before[key].value
                if old is not UNKNOWN:
                    e["old"] = old
                e["new"] = change.new[key]
                edits.append(e)
        ev = {"service": self.service, "instance": self.instance, "revision": self.revision,
              "token": change.token, "outcome": outcome, "edits": edits, **change.call}
        self.events_at.append(now)
        step.events.append(self._publish(ev))

    def _publish(self, ev: dict[str, Any]) -> Event:
        """One change event under the next seq."""
        seq = self._mint_seq()
        return Event(self.run, seq, ev)

    # -- applying and undoing ----------------------------------------------------------------

    def _record_change(self, change: Change, group: str, values: dict[str, Any]) -> None:
        for name, v in values.items():
            key = (group, name)
            if key not in change.before:
                change.before[key] = Running(self.running[key].value, self.running[key].source)
            change.new[key] = v
            if self._sensitive(group, name):
                change.sensitive = True
        change.groups.add(group)

    def _moved(self, change: Change | None, group: str, values: dict[str, Any]) -> None:
        for name, v in values.items():
            self.running[(group, name)] = Running(v, "runtime")
            if change is not None:
                change.applied.add((group, name))
        self.revision += 1

    def _undo(self, change: Change, step: Step) -> str:
        """v1 §2.11: one act over what the change applied, once; nothing
        applied, no act and no revision. Ends the change; returns its
        outcome."""
        self.pending = None
        if not change.applied:
            return "rolled-back"
        restore: dict[str, dict[str, Any]] = {}
        for g, p in sorted(change.applied):
            restore.setdefault(g, {})[p] = change.before[(g, p)].value
        step.acts.append("undo")
        result = self.device(("undo", restore))
        self.revision += 1
        if result == "ok" or result is None:
            for key in change.applied:
                b = change.before[key]
                self.running[key] = Running(b.value, b.source)
            return "rolled-back"
        readback = (result or {}).get("readback") or {}
        for key in change.applied:
            g, p = key
            if p in readback.get(g, {}):
                v, b = readback[g][p], change.before[key]
                same = b.value is not UNKNOWN and v1.json_equal(v, b.value)
                self.running[key] = Running(v, b.source if same else "runtime")
            else:
                self.running[key] = Running(UNKNOWN, None)
        return "partial"

    # -- facts -------------------------------------------------------------------------------

    def facts(self, now: int) -> Facts:
        p = self.pending
        return Facts(revision=self.revision, pending_token=None if p is None else p.token,
                     pending_applied=True if p is None else not p.waits,
                     deadline_passed=p is not None and p.deadline is not None and now >= p.deadline,
                     last_token=None if self.last_change is None else self.last_change["token"],
                     last_sensitive=bool(self.last_change and self.last_change["sensitive"]),
                     persist=self.persist_layer, in_flight=False, writable=self.writable)

    def _logged(self, group: str | None, req: dict[str, Any]) -> dict[str, Any]:
        g = None if group is None else self.layout.decl.group(group)
        out = {k: (REDACTED if g is not None and self._sensitive(group, k) else v)  # type: ignore[arg-type]
               for k, v in req.items()}
        self.log.append(json.dumps({"op": "set" if group else "control", "group": group, "request": out},
                                   sort_keys=True))
        return out

    # -- the calls ---------------------------------------------------------------------------

    def set(self, group: str, payload: bytes, now: int, call: dict[str, Any] | None = None) -> Step:
        """``@op/set/<group>``: the parse, row 1, row 2, then rows 4 to 14,
        then a dry run, a join or a new change."""
        step = Step()
        try:
            req = parse_request(self.layout, "set", group, payload)
        except (zk2c.Refused, packed.Refused):
            self._reply_refusal(step, Refusal("invalid_request"))
            return step
        step.logged = self._logged(group, req)
        f = self.facts(now)
        key = req.get("idempotency_key")
        if key is not None:
            kept = self.keys.get(key)
            if kept is not None and now >= kept.at + KEY_TTL_MS:
                del self.keys[key]
                kept = None
            if kept is not None:
                if kept.group == group and kept.request == bytes(payload):
                    self._replay(step, kept)
                    return step
                self._reply_refusal(step, Refusal("invalid_request"))
                return step

        def keep() -> None:
            if key is not None and step.reply is not None:
                step.kept = self.keys[key] = Kept(group, bytes(payload), step.reply.form, step.reply.payload, now)

        r = check_set(self.layout, group, req, f)
        if r is not None:
            self._reply_refusal(step, r)
            keep()
            return step
        g = self.layout.decl.group(group)
        assert g is not None
        values = {p.name: req[p.name] for p in g.params if p.name in req}
        if req.get("dry_run"):
            would = dict(self.running)
            for name, v in values.items():
                would[(group, name)] = Running(v, "runtime")
            self._reply_record(step, self.state(would, would_be=True))
            keep()
            return step
        joining = self.pending is not None and req.get("token") == self.pending.token
        if not joining and self._rate_spent(now):
            self._reply_refusal(step, Refusal("busy"))
            keep()
            return step
        if g.cls == "reach":
            apply_at = round_up(now + self.apply_after_ms)
            if joining:
                assert self.pending is not None
                if self.pending.deadline is None or apply_at >= self.pending.deadline:
                    self._reply_refusal(step, Refusal("busy"))
                    keep()
                    return step
                change = self.pending
            else:
                change = Change(self._mint_token(), set(), deadline_ms(now, req["confirm_s"], self.floor_s),
                                _meta(call))
                self.pending = change
            self._record_change(change, group, values)
            change.waits.append(Wait(group, dict(values), apply_at))
            self._put(step)
            self._reply_record(step)
            keep()
            return step
        step.acts.append("apply")
        result = self.device(("apply", group, dict(values)))
        if isinstance(result, dict) and "refused" in result:
            sens = any(self._sensitive(group, n) for n in values)
            words = result.get("refused")
            self._reply_refusal(step, Refusal("app", app="device_refused",
                                              message=None if sens or not isinstance(words, str) else words))
            keep()
            return step
        if joining:
            change = self.pending
            assert change is not None
            self._record_change(change, group, values)
            self._moved(change, group, values)
            self._put(step)
            self._reply_record(step)
            keep()
            return step
        change = Change(self._mint_token(), set(), None if "confirm_s" not in req
                        else deadline_ms(now, req["confirm_s"], self.floor_s), _meta(call))
        self._record_change(change, group, values)
        self._moved(change, group, values)
        if change.deadline is not None:
            self.pending = change
            self._put(step)
            self._reply_record(step)
            keep()
            return step
        self.last_change = {"token": change.token, "groups": set(change.groups), "sensitive": change.sensitive,
                            "values": dict(change.new)}
        self._put(step)
        self._reply_record(step)
        keep()
        self._event(change, "applied", step, now)
        return step

    def control(self, op: str, payload: bytes, now: int, call: dict[str, Any] | None = None) -> Step:
        """``confirm``, ``cancel``, ``extend`` and ``persist`` (§2.10,
        §2.13): their checks, then the act, the record put first, then the
        reply, then the event."""
        step = Step()
        req, r = check_request(self.layout, op, None, payload, self.facts(now))
        if req is not None:
            step.logged = self._logged(None, req)
        if r is not None:
            self._reply_refusal(step, r)
            return step
        assert req is not None
        change = self.pending
        if op == "extend":
            assert change is not None
            change.deadline = deadline_ms(now, req["confirm_s"], self.floor_s)
            self._put(step)
            self._reply_record(step)
            return step
        if op == "confirm":
            assert change is not None
            self.pending = None
            self.last_change = {"token": change.token, "groups": set(change.groups), "sensitive": change.sensitive,
                                "values": dict(change.new)}
            self._put(step)
            self._reply_record(step)
            self._event(change, "confirmed", step, now)
            return step
        if op == "cancel":
            assert change is not None
            outcome = self._undo(change, step)
            self._put(step)
            self._reply_record(step)
            self._event(change, outcome, step, now)
            return step
        assert op == "persist" and self.last_change is not None
        for (g, p), v in self.last_change["values"].items():
            self.overlay.setdefault(g, {})[p] = v
            cur = self.running[(g, p)]
            pend = self.pending
            if pend is not None and (g, p) in pend.before:
                b = pend.before[(g, p)]
                if b.value is not UNKNOWN and v1.json_equal(b.value, v):
                    pend.before[(g, p)] = Running(v, "overlay")
                if (g, p) in pend.applied:
                    continue
            if cur.value is not UNKNOWN and v1.json_equal(cur.value, v):
                self.running[(g, p)] = Running(v, "overlay")
        self._put(step)
        self._reply_record(step)
        return step

    def due(self, now: int) -> Step:
        """Every act owed at ``now``: the undo at the deadline, else each
        reach apply whose ``apply_at`` has come, earliest first; each act's
        record put, then its event."""
        step = Step()
        change = self.pending
        if change is None:
            return step
        if change.deadline is not None and now >= change.deadline:
            outcome = self._undo(change, step)
            self._put(step)
            self._event(change, outcome, step, now)
            self._desire_again(now, step)
            return step
        for w in sorted([w for w in change.waits if w.apply_at <= now], key=lambda w: w.apply_at):
            if self.pending is not change:
                break
            step.acts.append("apply")
            result = self.device(("apply", w.group, dict(w.values)))
            change.waits.remove(w)
            if isinstance(result, dict) and "refused" in result:
                outcome = self._undo(change, step)
                self._put(step)
                self._event(change, outcome, step, now)
                break
            self._moved(change, w.group, w.values)
            self._put(step)
        self._desire_again(now, step)
        return step

    def next_due(self) -> int | None:
        p = self.pending
        if p is None:
            return None
        times = [w.apply_at for w in p.waits]
        if p.deadline is not None:
            times.append(p.deadline)
        return min(times) if times else None

    # -- §2.18 desired state ----------------------------------------------------------------

    def desire(self, document: dict[str, Any], now: int) -> Step:
        """A desired document bound to this owner: refused whole, its report
        in the record's desired fields, or converged by one ``set`` without
        a window per group that differs, in the byte order of the group
        names, each its own change; while a change is pending, it waits."""
        self._desired_doc: dict[str, Any] | None = document
        step = Step()
        refusal = check_desired(self.layout, document, self.writable)
        if refusal is not None:
            self.desired = {"outcome": "refused", "observed_revision": document["revision"], "refusal": refusal}
            self._desired_doc = None
            self._put(step)
            return step
        self._converge(now, step)
        return step

    def _desire_again(self, now: int, step: Step) -> None:
        if getattr(self, "_desired_doc", None) is not None and self.pending is None:
            self._converge(now, step)

    def _converge(self, now: int, step: Step) -> None:
        """§2.18 (text 0.2, F-482): "The report changes in a put of its own,
        once the owner has acted on the document. Each converging change's
        put and its event come before it (§2.17)." So each group's change is
        put, then its event published, the record still carrying the earlier
        report; then the report's put."""
        doc = self._desired_doc
        if doc is None or self.pending is not None:
            return
        failed = None
        for gname in v1.bytewise(doc["values"]):
            want = doc["values"][gname]
            if all(v1.json_equal(self.running[(gname, p)].value, v) for p, v in want.items()):
                continue
            if self._rate_spent(now):
                failed = {"code": "busy", "group": gname, "parameter": None}
                break
            step.acts.append("apply")
            result = self.device(("apply", gname, dict(want)))
            if isinstance(result, dict) and "refused" in result:
                failed = {"code": "device_refused", "group": gname, "parameter": None}
                break
            change = Change(self._mint_token(), set(), None, {})
            self._record_change(change, gname, want)
            self._moved(change, gname, want)
            self.last_change = {"token": change.token, "groups": set(change.groups), "sensitive": False,
                                "values": dict(change.new)}
            self._put(step)
            self._event(change, "applied", step, now)
        self.desired = {"outcome": "failed" if failed else "converged", "observed_revision": doc["revision"],
                        "refusal": failed}
        self._desired_doc = None
        self._put(step)

    # -- a re-mint (v1 §2.12, text 0.11) -------------------------------------------------------

    def end_for_remint(self, now: int) -> Step:
        """v1 §2.12 (text 0.11, F-481), step 1 and 2 of a re-mint: "A pending
        change ends first, in the old instance, as ``cancel`` ends it (§2.10).
        Its undo runs and its read-back is put", then "Its event is
        published, ``rolled-back`` or ``partial`` (§2.17). The rate counts
        it." v2 §2.12: "That change's record and event carry the old
        instance number." Nothing pending: an empty step."""
        step = Step()
        change = self.pending
        if change is None:
            return step
        outcome = self._undo(change, step)
        self._put(step)
        self._event(change, outcome, step, now)
        return step


# -- the fixtures' shapes ---------------------------------------------------------------------

def core_of_setup(layout: Layout, setup: dict[str, Any]) -> tuple[Core, list[Any]]:
    """A :class:`Core` as ``lifecycle.json``'s ``setup`` states one: its runs
    and counters in order, its seq start, startup, overlay, floor, apply
    delay, layer and allowlist. Returns the core and the list a step fills
    with its device's results, in order ("a missing result is ok")."""
    runs = iter([(r["width"], r["run"]) for r in setup["runs"]])
    counters = iter(setup["counters"])
    results: list[Any] = []

    def device(act: tuple) -> Any:
        return results.pop(0) if results else "ok"

    core = Core(layout, service=setup["service"], next_run=lambda: next(runs), next_counter=lambda: next(counters),
                startup=setup["startup"], overlay=json.loads(json.dumps(setup["overlay"])),
                floor_s=setup["floor_s"], apply_after_ms=setup["apply_after_ms"], persist=setup["persist"],
                writable=setup["writable"], epoch=setup["epoch"], device=device,
                seq_start=setup.get("seq_start") or 0)
    return core, results


def run_step(core: Core, results: list[Any], s: dict[str, Any]) -> Step:
    """One of ``lifecycle.json``'s steps on ``core``."""
    results[:] = list(s.get("device") or [])
    t, do = s["t"], s["do"]
    if do == "set":
        return core.set(s["group"], bytes.fromhex(s["hex"]), t, s.get("call"))
    if do in ("confirm", "cancel", "extend", "persist"):
        return core.control(do, bytes.fromhex(s["hex"]), t, s.get("call"))
    if do == "due":
        return core.due(t)
    if do == "restart":
        return core.restart()
    raise ValueError(f"no such step: {do!r}")


def read_reply(layout: Layout, sample: str, payload: bytes, epoch: int = 0) -> dict[str, Any]:
    """A reply as ``lifecycle.json`` writes one: ``{value: {run, seq,
    record, hex}}`` read by §7.5's parse and §2.3's reader, or ``{error:
    {run, seq, code, cause, app, message, hex}}``, its ``Error`` detail read
    under the form's tag. Raises on a refused form or record."""
    if sample == "reply":
        env = zk2c.parse(payload, "reply")
        st = read_record(layout, env.tag, env.record or b"", epoch)
        return {"value": {"run": env.run, "seq": env.seq, "record": st, "hex": bytes(payload).hex()}}
    env = zk2c.parse(payload, "reply_err")
    e = env.error
    assert e is not None
    app = None
    if e.detail is not None:
        app = layout.error.read(env.tag, e.detail)["code"]
    return {"error": {"run": env.run, "seq": env.seq, "code": e.code, "cause": e.cause, "app": app,
                      "message": e.message, "hex": bytes(payload).hex()}}


def step_expect(layout: Layout, step: Step, epoch: int = 0) -> dict[str, Any]:
    """A step as ``lifecycle.json``'s ``expect`` writes it, every reply read
    back from its bytes."""
    out: dict[str, Any] = {
        "acts": list(step.acts),
        "puts": [{"run": p.run[1], "seq": p.seq, "record": p.state, "hex": p.payload.hex()} for p in step.puts],
        "reply": None,
        "events": [{"run": e.run[1], "seq": e.seq, "event": e.event} for e in step.events]}
    r = step.reply
    if r is not None:
        out["reply"] = read_reply(layout, "reply" if r.form == "value" else "reply_err", r.payload, epoch)
    return out


# -- zenoh-modem's SX1262 (config.v2-binding.md §15) ---------------------------------------------

SX1262 = REPO / "examples" / "zk2" / "zenoh-modem" / "config" / "sx1262_config.v1.toml"


def sx1262_round_trip() -> dict[str, Any]:
    """``config.v2-binding.md`` §15's round trip, through zk2py's own
    :class:`Core` and codec on ``sx1262_config.v1``: the run 0x5A17, the
    token count from 0x9C41, the catalog's epoch 1,767,225,600, the floor
    15 s, ``apply_at`` 2 s after a reach set, ``writable`` duty and radio.
    The section names the times of day and not the date; the bytes it
    gives date them 2026-11-02 (round 102). Returns each step's request and
    put and reply sizes and bytes, and the record at start."""
    import calendar

    from .contract import load_contract

    k = load_contract(SX1262, check_file_name=True)
    L = Layout.of_contract(k)
    epoch = 1_767_225_600
    startup = {"duty": {"fraction_ppm": {"value": 10_000, "source": "file"},
                        "period_s": {"value": 3_600, "source": "file"}},
               "radio": {"frequency_hz": {"value": 868_100_000, "source": "file"}},
               "modulation": {"spreading_factor": {"value": 7, "source": "file"}}}
    core = Core(L, service="h-c2a95e41d7f0/lora0", next_run=lambda: (16, 0x5A17), next_counter=lambda: 0x9C41,
                startup=startup, floor_s=15, apply_after_ms=2_000, writable=["duty", "radio"], epoch=epoch)

    def at(hms: str) -> int:
        h, m, s = (int(x) for x in hms.split(":"))
        return calendar.timegm((2026, 11, 2, h, m, s, 0, 0, 0)) * 1000

    sd, sr, ctl = L.set_type("duty"), L.set_type("radio"), L.control
    out: dict[str, Any] = {"start": {"record": len(core.start_step.puts[0].payload) - 6,
                                     "put": len(core.start_step.puts[0].payload)}}
    req1 = zk2c.request(sd.write({"fraction_ppm": 9_000, "dry_run": False}), sd.tag)
    s1 = core.set("duty", req1, at("12:00:00"))
    req2 = zk2c.request(sr.write({"frequency_hz": 869_525_000, "confirm_s": 120, "expected_revision": 1,
                                  "idempotency_key": 0x3D2C1B0A, "dry_run": False}), sr.tag)
    s2 = core.set("radio", req2, at("12:01:00"))
    token = s2.reply.state["pending"]["token"] if s2.reply and s2.reply.state else None
    req3 = zk2c.request(ctl.write({"token": token}), ctl.tag)
    s3 = core.control("confirm", req3, at("12:01:01"))
    s4 = core.due(at("12:01:02"))
    s2b = core.set("radio", req2, at("12:01:10"))
    s5 = core.control("confirm", req3, at("12:01:32"))

    def sizes(req: bytes | None, step: Step) -> dict[str, Any]:
        return {"request": None if req is None else len(req),
                "puts": [(p.seq, len(p.payload)) for p in step.puts],
                "reply": None if step.reply is None else (step.reply.form, step.reply.seq, len(step.reply.payload)),
                "acts": list(step.acts), "events": [e.event["outcome"] for e in step.events]}

    out["steps"] = [sizes(req1, s1), sizes(req2, s2), sizes(req3, s3), sizes(None, s4), sizes(req3, s5)]
    out["bytes"] = {"request2": req2.hex(), "reply2": s2.reply.payload.hex() if s2.reply else None,
                    "request5": req3.hex(), "reply5": s5.reply.payload.hex() if s5.reply else None}
    out["retry2_same"] = bool(s2b.reply and s2.reply and s2b.reply.payload == s2.reply.payload)
    out["reply2_is_put"] = bool(s2.reply and s2.puts and s2.reply.payload[1:] == s2.puts[-1].payload[1:])
    out["tokens"] = {"last1": hex(s1.reply.state["last"]["token"]) if s1.reply and s1.reply.state else None,
                     "pending2": hex(token) if token is not None else None}
    return out


# -- the session half: an owner ---------------------------------------------------------------

class LiveCore(Core):
    """:class:`Core` on a :class:`zk2py.owner.Owner`: the run is the owner's,
    every put and event the owner's (so its seqs are the one count over state
    and events, core §4.3), every reply sent as it is made, on the call
    being answered (:attr:`call`), and a token counter spent taking a new run
    whose descriptor states it (§2.6, core §3.3)."""

    def __init__(self, layout: Layout, owner: Any, iface: str, **kw: Any):
        self.owner = owner
        self.cfg_key = f"zk2/{owner.system}/{owner.service}/{iface}/state/config"
        self.event_member = f"zk2/{owner.system}/{owner.service}/{iface}/events/change"
        self._counter_run: Any = None
        #: a test's loss: the next ``drop_replies`` replies are made, kept as
        #: first answers, and never sent; ``dropped`` holds their bytes
        self.drop_replies = 0
        self.dropped: list[bytes] = []
        self.sent: list[Reply] = []
        super().__init__(layout, next_run=lambda: owner.run, begin=False, **kw)

    @property  # type: ignore[override]
    def run(self) -> tuple[int, int]:
        return tuple(self.owner.run)  # type: ignore[return-value]

    @run.setter
    def run(self, _v: Any) -> None:
        pass

    def _take_run(self) -> None:
        self.counter = self.next_counter()
        self._counter_run = self.owner.run
        self.next_seq, self.last_seq = 0, None

    def mark(self) -> zk2c.InBand:
        return self.owner.mark()

    def _mint_token(self) -> int:
        if self._counter_run != self.owner.run:
            # a new run since this counter started (its seq spent, core
            # §4.3): "whose counter starts afresh" (§2.6)
            self.counter, self._counter_run = self.next_counter(), self.owner.run
        if self.counter > COUNTER_MAX:
            with self.owner._writes:  # noqa: SLF001 - a run taken between two writes
                self.owner.take_run()
                self.owner._put_descriptor()  # noqa: SLF001 - core §3.3: before the first write under it
            self.counter, self._counter_run = self.next_counter(), self.owner.run
        token = self.owner.run[1] * 65536 + self.counter
        self.counter += 1
        return token

    def _put(self, step: Step) -> None:
        st = self.state()
        if st == self._put_state:
            return
        self.owner.set_state(self.cfg_key, self.record_bytes(st))
        m = self.owner.mark()
        held = self.owner._held.get(self.cfg_key)  # noqa: SLF001
        self._put_state = st
        step.puts.append(Put((m.width, m.run), m.seq, st, b"" if held is None else held.wire))

    def _publish(self, ev: dict[str, Any]) -> Event:
        self.owner.occur(self.event_member, json.dumps(ev, separators=(",", ":")).encode())
        m = self.owner.mark()
        return Event((m.width, m.run), m.seq, ev)

    def _emit(self, reply: Reply) -> None:
        call = self.call
        self.sent.append(reply)
        if call is None:
            return
        if self.drop_replies > 0 and not reply.replay:
            self.drop_replies -= 1
            self.dropped.append(reply.payload)
            if reply.form == "value":
                call.values += 1
            else:
                call.envelopes += 1
            return
        call.resend(reply.form, reply.payload)


class ConfigV2Owner:
    """A service's config contract over :class:`zk2py.owner.Owner`.

    - Every resource of §3 is exposed, ``persist`` listed unavailable with
      cause ``config`` when there is no persisted layer (§2.1, v1 §2.13); one
      concrete queryable per ``set/<group>``.
    - The record is put before the tokens (§2.3: ``before_tokens``), on every
      change and at nothing else; the owner's heartbeat confirms it
      (``freshness.v2``).
    - Every reply is the record in the value form, or a refusal in the error
      form, sent before the change's event (§2.9, §2.17).
    - Deadlines and reach applies run on a timer thread, at the whole second
      each states (§2.8).
    - A token is the run and a 16-bit counter, each run's counter drawn at
      random, or from ``counter_start`` (a test's hook, scenarios §7 step 5).
    - :meth:`remint` is a restart of the configuration (v1 §2.12, text 0.11,
      core §1.5): a pending change ended first in the old instance, as
      ``cancel`` ends it, its record put and its event published; the device
      returned to its startup values; then a new instance whose record,
      under its new run, is put before its tokens."""

    def __init__(self, system: str, service: str, contract: Any, *, startup: dict[str, dict[str, dict[str, Any]]],
                 overlay: dict[str, dict[str, Any]] | None = None, floor_s: int = 10, apply_after_ms: int = 1000,
                 persist: bool = True, writable: list[str] | None = None, device: Any = None,
                 events_at: list[int] | None = None, epoch: int = 0, offset_ms: int = 0, **owner_kw: Any):
        import secrets

        from .owner import Owner

        self.contract = contract
        self.layout = Layout.of_contract(contract)
        self.iface = contract.interface
        self.device = device or v1.SimDevice()
        unavailable = dict(owner_kw.pop("unavailable", {}) or {})
        if not persist:
            unavailable["@op/persist"] = "config"
        handlers: dict[str, Any] = {}
        for g in self.layout.decl.groups:
            if g.settable:
                handlers[f"@op/set/{g.name}"] = self._on_set(g.name)
        for op in ("confirm", "cancel", "extend", "persist"):
            handlers[f"@op/{op}"] = self._on_control(op)
        self.owner = Owner(system, service, [contract, *owner_kw.pop("contracts", [])], handlers=handlers,
                           unavailable=unavailable, before_tokens=self._before_tokens, **owner_kw)
        self.offset_ms = offset_ms
        self.counter_start: int | None = None
        self.counters: list[int] = []

        def next_counter() -> int:
            c = self.counter_start if self.counter_start is not None else secrets.randbits(16)
            self.counter_start = None
            self.counters.append(c)
            return c

        self.core = LiveCore(self.layout, self.owner, self.iface, service=f"{system}/{service}",
                             next_counter=next_counter, startup=startup, overlay=overlay, floor_s=floor_s,
                             apply_after_ms=apply_after_ms, persist=persist,
                             writable=writable if writable is not None else [g.name for g in self.layout.decl.groups
                                                                             if g.settable],
                             epoch=epoch, device=self.device, events_at=events_at)
        self.lock = threading.RLock()
        self.cv = threading.Condition(self.lock)
        self._stop = threading.Event()
        self._timer: threading.Thread | None = None
        self.steps: list[Step] = []

    # -- life ----------------------------------------------------------------------------

    def now_ms(self) -> int:
        return time.time_ns() // 1_000_000 + self.offset_ms

    @property
    def instance(self) -> str:
        return self.owner.instance

    def _startup_values(self) -> dict[str, dict[str, Any]]:
        out: dict[str, dict[str, Any]] = {}
        for g in self.layout.decl.groups:
            for p in g.params:
                v, _ = self.core.startup_value(g.name, p.name)
                if v is not UNKNOWN:
                    out.setdefault(g.name, {})[p.name] = v
        return out

    def _before_tokens(self, owner: Any) -> None:
        with self.lock:
            self.steps.append(self.core._start())  # noqa: SLF001 - the record before the tokens (§2.3)

    def start(self) -> None:
        self.device.boot(self._startup_values())
        self.owner.start()
        self._timer = threading.Thread(target=self._run_timer, daemon=True)
        self._timer.start()

    def close(self) -> None:
        self._stop.set()
        with self.cv:
            self.cv.notify_all()
        if self._timer is not None:
            self._timer.join(timeout=2.0)
        self.owner.close()

    def remint(self) -> str:
        """Core §1.5's re-mint while running, a restart of the configuration
        (v1 §2.12, text 0.11), "in this order":
        1. a pending change ends in the old instance, as ``cancel`` ends it:
           its undo, its record put, then its event, ``rolled-back`` or
           ``partial``, the rate counting it (:meth:`Core.end_for_remint`);
        2. the device returns to its startup values, one undo where it holds
           others;
        3. the new instance and run, its record put before its tokens:
           revision 0, nothing pending, no last change, no key held.

        Refused (:class:`zk2py.owner.OwnerRefused`), the instance running on,
        where §2.12 says the reference refuses: a parameter set at runtime
        whose startup value is unknown, checked before anything moves, and a
        device that refuses to return to its startup values. An act is never
        in flight here: the owner acts under its lock. Returns the new
        instance id."""
        from .owner import OwnerRefused

        with self.lock:
            unknown = sorted(f"{g}.{p}" for (g, p), r in self.core.running.items()
                             if r.source == "runtime" and self.core.startup_value(g, p)[0] is UNKNOWN)
            if unknown:
                raise OwnerRefused(f"config.v1 §2.12: {', '.join(unknown)} set at runtime, its startup value "
                                   "unknown: no re-mint")
            step = self.core.end_for_remint(self.now_ms())
            self.steps.append(step)
            startup = self._startup_values()
            moved = {g: {p: v for p, v in ps.items() if self.device.values.get(g, {}).get(p) != v}
                     for g, ps in startup.items()}
            moved = {g: ps for g, ps in moved.items() if ps}
            if moved:
                result = self.device(("undo", moved))
                if not (result == "ok" or result is None):
                    self.cv.notify_all()
                    raise OwnerRefused("config.v1 §2.12: the device refuses to return to its startup values: no "
                                       f"re-mint ({result})")
            original = self.owner._declare_instance  # noqa: SLF001

            def declare(plan: Any, instance: str, bundles: bool = False) -> Any:
                self.steps.append(self.core.restart())
                return original(plan, instance, bundles)

            self.owner._declare_instance = declare  # noqa: SLF001 - the record before the new tokens
            try:
                self.owner.new_epoch()
            finally:
                self.owner._declare_instance = original  # noqa: SLF001
            self.cv.notify_all()
            return self.owner.instance

    # -- the calls -----------------------------------------------------------------------

    def _run(self, call: Any, act: Callable[[], Step]) -> None:
        with self.lock:
            self.core.call = call
            try:
                step = act()
            finally:
                self.core.call = None
            self.steps.append(step)
            self.cv.notify_all()

    def _on_set(self, group: str):
        def handle(call: Any) -> None:
            self._run(call, lambda: self.core.set(group, call.wire, self.now_ms(), call.metadata()))
        return handle

    def _on_control(self, op: str):
        def handle(call: Any) -> None:
            self._run(call, lambda: self.core.control(op, call.wire, self.now_ms(), call.metadata()))
        return handle

    def desire(self, document: dict[str, Any]) -> Step:
        with self.lock:
            step = self.core.desire(document, self.now_ms())
            self.steps.append(step)
            self.cv.notify_all()
            return step

    def _run_timer(self) -> None:
        while not self._stop.is_set():
            with self.cv:
                due = self.core.next_due()
                now = self.now_ms()
                if due is None or due > now:
                    wait = 0.5 if due is None else min(0.5, (due - now) / 1000)
                    self.cv.wait(timeout=max(0.001, wait))
                    continue
                try:
                    self.steps.append(self.core.due(now))
                except Exception as e:  # noqa: BLE001 - a session closing under it
                    print(f"zk2py config.v2 owner: due: {type(e).__name__}: {e}", flush=True)
                    self._stop.wait(0.2)


# -- the session half: a caller and a reader ---------------------------------------------------

@dataclass
class Called:
    """One call as a caller saw it: ``value`` (its record's state and mark),
    ``error`` (core §5.2's code, cause, ``Error`` code and message, its
    mark), ``malformed`` (an answer whose form or record a reader refuses),
    or ``silent`` (core O5)."""

    kind: str
    state: dict[str, Any] | None = None
    error: dict[str, Any] | None = None
    mark: zk2c.InBand | None = None
    raw: bytes = b""
    request: bytes = b""
    at_ns: int = 0
    refusal: str | None = None

    @property
    def code(self) -> str | None:
        return None if self.error is None else self.error["code"]

    def short(self) -> str:
        if self.kind == "value" and self.state is not None:
            st = self.state
            return (f"value r{st['revision']} pending={st['pending']} last={st['last']} would_be={st['would_be']} "
                    f"mark={self.mark}")
        if self.kind == "error":
            return f"error {self.error} mark={self.mark}"
        return f"{self.kind} {self.refusal or ''}"


class Caller:
    """C (scenarios-v2.md, Conventions): each request its ``Set<T>`` or
    ``Control`` record in a query (core §7.5), called as core O1 says (one
    concrete key, its first answer), with the call metadata ``{"actor":
    "ops", "request_id": …}`` (core O7); each reply read as the value form
    or the error form, and the record by §2.3's reader. It holds the
    current record from each reply and each push it is handed (§2.19),
    ordered by seq within one run, never one with ``would_be`` set (§2.9);
    a mark of seq 0 is before seq 1, and unordered against the write of seq
    0 (core 0.95 §5.1)."""

    def __init__(self, session: Any, address: str, layout: Layout, iface: str, epoch: int = 0):
        self.session, self.address, self.layout, self.iface, self.epoch = session, address, layout, iface, epoch
        self.held: tuple[zk2c.InBand, dict[str, Any]] | None = None
        self.calls: list[Called] = []

    def key(self, op: str) -> str:
        return f"zk2/{self.address}/{self.iface}/@op/{op}"

    def request(self, op: str, fields: dict[str, Any]) -> bytes:
        if op.startswith("set/"):
            rec = self.layout.set_type(op[4:])
            fields = {"dry_run": False, **fields}
        else:
            rec = self.layout.control
        return zk2c.request(rec.write(fields, self.epoch), rec.tag)

    def call(self, op: str, fields: dict[str, Any] | None = None, *, raw: bytes | None = None,
             actor: str | None = "ops", request_id: str | None = None, timeout: float = 2.0) -> Called:
        from . import live

        payload = raw if raw is not None else self.request(op, fields or {})
        meta = {k: v for k, v in (("actor", actor), ("request_id", request_id)) if v is not None}
        # the query's payload as written, CH included, under zenoh/bytes
        res = live.call(self.session, self.key(op), payload, timeout=timeout, first=True, bare=True,
                        encoding=zk2c.ENCODING, attachment=json.dumps(meta).encode() if meta else None)
        at = time.time_ns()
        out = Called("silent", request=payload, at_ns=at)
        for r in res.replies:
            if r.kind == "value":
                try:
                    st = read_record(self.layout, r.tag or 0, r.payload, self.epoch)
                except (packed.Refused, RecordRefused) as e:
                    out = Called("malformed", raw=r.raw, mark=r.mark, request=payload, at_ns=at, refusal=e.token)
                    break
                out = Called("value", state=st, mark=r.mark, raw=r.raw, request=payload, at_ns=at)
                self.hold(r.mark, st)
                break
            if r.kind == "refused_value":
                out = Called("malformed", raw=r.raw, request=payload, at_ns=at, refusal=r.refusal)
                break
            if r.kind == "error":
                e = dict(r.error or {})
                app = None
                if isinstance(e.get("detail"), (bytes, bytearray)):
                    try:
                        app = self.layout.error.read(r.tag or 0, bytes(e["detail"]))["code"]
                    except packed.Refused as x:
                        app = {"undecodable": x.token}
                out = Called("error", error={"code": e.get("code"), "cause": e.get("cause"), "app": app,
                                             "message": e.get("message")}, mark=r.mark, raw=r.raw,
                             request=payload, at_ns=at)
                break
        self.calls.append(out)
        return out

    def hold(self, mark: zk2c.InBand | None, state: dict[str, Any]) -> bool:
        """Hold ``state`` as the current record unless the one held is newer
        by seq within one run; never a would-be record. Returns whether it
        was taken."""
        if state.get("would_be") or mark is None:
            return False
        if self.held is not None:
            m, _ = self.held
            if m.same_run(mark) and (mark.seq < m.seq):
                return False
        self.held = (mark, state)
        return True


@dataclass
class Delivery:
    at_ns: int
    key: str
    kind: str
    payload: bytes
    encoding: str


class Recorder:
    """S's raw half: every delivery on its selectors, with its arrival."""

    def __init__(self, session: Any, selectors: list[str]):
        import zenoh

        self.rows: list[Delivery] = []
        self.lock = threading.Lock()
        self.subs = [session.declare_subscriber(s, zenoh.handlers.Callback(self._got)) for s in selectors]

    def _got(self, smp: Any) -> None:
        import zenoh

        d = Delivery(time.time_ns(), str(smp.key_expr), "put" if smp.kind == zenoh.SampleKind.PUT else "delete",
                     smp.payload.to_bytes(), str(smp.encoding))
        with self.lock:
            self.rows.append(d)

    def of(self, part: str, since_ns: int = 0) -> list[Delivery]:
        with self.lock:
            return [d for d in self.rows if part in d.key and d.at_ns >= since_ns]

    def close(self) -> None:
        for s in self.subs:
            try:
                s.undeclare()
            except Exception:  # noqa: BLE001 - closing anyway
                pass


def read_put(layout: Layout, payload: bytes, epoch: int = 0) -> tuple[zk2c.InBand | None, dict[str, Any]]:
    """A pushed record read as §2.3 says: core §7.5's mutation, then its
    record. Raises on a refusal."""
    env = zk2c.parse(payload, "put")
    if env.form != "mutation":
        raise RecordRefused("form", f"a {env.form} is no record")
    return env.stamp, read_record(layout, env.tag, env.record or b"", epoch)


# -- §5 what a tool may conclude ---------------------------------------------------------------

def tool_answers(read: dict[str, Any], layout: Layout, *, token: int | None = None, instance: int | None = None,
                 group: str | None = None, survives: tuple[str, str] | None = None,
                 desired_revision: int | None = None) -> dict[str, tuple[str, Any]]:
    """§5's questions, read from a record and the contract (v1 §5 holds): a
    fresh record is a fresh view and status. ``read`` is ``{premise,
    record, fresh, events}``: the premise ``listed`` (a descriptor listing
    config.v2, or across a face the deployment's word), ``not_listed`` or
    ``absent`` (presence's word that no instance is there), the last record
    and whether it is fresh, and the change events heard. An answer is
    ``yes``, ``no``, ``not_yet``, ``unobservable`` or ``not_asked``, never
    ``no`` from silence."""
    out: dict[str, tuple[str, Any]] = {}
    premise = read.get("premise", "listed")
    if premise in ("absent", "not_listed"):
        why = "the owner absent (presence's word)" if premise == "absent" else "its descriptor lists no config.v2"
        for q in ("pending", "landed", "undo_clean", "writable", "survives", "converged"):
            out[q] = ("not_asked", why)
        return out
    rec = read.get("record") if read.get("fresh") else None
    events = read.get("events") or []
    if rec is None:
        out["pending"] = ("unobservable", "no fresh record")
    elif rec.get("pending"):
        out["pending"] = ("yes", rec["pending"])
    else:
        out["pending"] = ("no", None)
    if token is not None:
        mine = [e for e in events if e.get("token") == token]
        landed: tuple[str, Any] = ("unobservable", "no fresh record and no event")
        if any(e["outcome"] in ("applied", "confirmed") for e in mine):
            landed = ("yes", "an event")
        elif any(e["outcome"] in ("rolled-back", "partial") for e in mine):
            landed = ("no", next(e["outcome"] for e in mine if e["outcome"] in ("rolled-back", "partial")))
        elif rec is not None:
            if instance is not None and rec.get("instance") != instance:
                landed = ("no", "a restart: another instance number")
            elif (rec.get("pending") or {}).get("token") == token:
                landed = ("not_yet", "its token is pending")
            elif (rec.get("last") or {}).get("token") == token:
                landed = ("yes", "the last token")
        out["landed"] = landed
        if any(e["outcome"] == "rolled-back" for e in mine):
            out["undo_clean"] = ("yes", "rolled-back")
        elif any(e["outcome"] == "partial" for e in mine):
            out["undo_clean"] = ("no", "partial")
        else:
            out["undo_clean"] = ("unobservable", "no event read")
    if group is not None:
        g = layout.decl.group(group)
        if g is None:
            out["writable"] = ("not_asked", "the contract declares no such group")
        elif g.cls == "contract":
            out["writable"] = ("no", "a contract group")
        elif rec is None:
            out["writable"] = ("unobservable", "no fresh record")
        else:
            out["writable"] = ("yes", None) if rec["groups"][group].get("writable") else ("no", "its bit is clear")
    if survives is not None:
        gname, pname = survives
        g = layout.decl.group(gname)
        p = None if g is None else g.param(pname)
        if rec is None or p is None or p.sensitive:
            out["survives"] = ("unobservable", "no fresh record, or a sensitive or undeclared parameter")
        else:
            ps = rec["groups"][gname]["params"][pname]
            if ps.get("value") is None:
                out["survives"] = ("unobservable", "the value is unknown")
            else:
                out["survives"] = ("no", "differs") if ps.get("differs") else ("yes", ps["value"])
    if rec is None:
        out["converged"] = ("unobservable", "no fresh record")
    else:
        d = rec.get("desired")
        if d is None:
            out["converged"] = ("not_asked", "no desired report")
        elif desired_revision is not None and d["observed_revision"] < desired_revision:
            out["converged"] = ("not_yet", d)
        elif d["outcome"] == "converged":
            out["converged"] = ("yes", d)
        elif d["outcome"] in ("refused", "failed"):
            out["converged"] = ("no", d)
        else:
            out["converged"] = ("unobservable", d)
    return out

"""``config.v1``, text 0.1 (``spec/profiles/config/v1.md``): changing a
running service's configuration without locking it out. Read again
against text 0.6 (#609, round 86; #935): its inline history taken out and
five long sentences split, §2.5's MUST now naming "the checks below", no
rule changed, and none of this module's rules moved. Read again against
text 0.7 (round 87): §2.3, §2.13 and §2.18 rewritten for round 86's F-329
to F-331, no rule changed. Read again against text 0.8 (round 88): §2.18's
instance is ``desired.v1``'s run, and a re-mint is a restart, as
``desired.v1`` 0.10's §2.16 says (F-336); :class:`ConfigOwner` can now be
fed through ``desired.v1`` (``fed``), and re-mint (:meth:`ConfigOwner.remint`)
as a restart or refuse to, as the reference does.

The session-free half, from the text alone:
- **time on the wire** (§2.8): :func:`parse_time`, :func:`write_time`, and
  the window and its deadline, :func:`window`, :func:`deadline`;
- **values** (§2.2, §2.5): a parameter's kind and bounds, an integer
  written without a fraction or an exponent, within 64 bits;
- **the order of refusals**: a ``set``'s (§2.5, rows 1 to 11 in
  :func:`check_set`, 12 to 14 in :class:`Core`), the control operations'
  (§2.10, :func:`check_control`) and ``persist``'s (§2.13,
  :func:`check_persist`), each after the request's own type (the decode,
  :func:`decode_set`, :func:`decode_control`);
- **desired state** refused whole (§2.18, :func:`check_desired`);
- **redaction** (§2.15): :func:`redacted_request`, and every view, status,
  event and envelope :class:`Core` makes;
- **the owner** (§2.3, §2.6 to §2.14, §2.17 to §2.19) as a state machine,
  :class:`Core`, driven by calls, device results and the owner's clock, as
  ``conformance/lifecycle.json`` drives it.

The session half: :class:`ConfigOwner` serves ``config.v1`` over
:class:`zk2py.owner.Owner` (its operations, its view and status put before
the tokens and on every change, its change events, its deadlines on a
thread of its own), with :class:`SimDevice`, a simulated device a test
sets each act's result on; and the reader of §5, :func:`read` and the
``answer_*`` functions.

Where the text left a choice, zk2py's is named in ``SPEC-FINDINGS.md``
("At config.v1 0.1").
"""

from __future__ import annotations

import calendar
import json
import re
import threading
import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Callable

NS = 1_000_000_000
MS = 1_000_000
IFACE = "config.v1"
CONTRACT = Path(__file__).resolve().parents[3] / "spec" / "profiles" / "config" / "config.v1.toml"

CLASSES = ("hot", "reach", "contract")
KINDS = ("bool", "integer", "text")
SOURCES = ("default", "file", "overlay", "runtime")
OUTCOMES = ("applied", "confirmed", "rolled-back", "partial")
I64_MIN, I64_MAX = -(2**63), 2**63 - 1
U64_MAX = 2**64 - 1
#: §2.14: "An owner MUST keep the first answer … for 300 s from that answer."
KEY_TTL_NS = 300 * NS
#: §2.17: the change event's rate, burst(600/h), kept by refusing busy.
RATE_N, RATE_PERIOD_NS = 600, 3600 * NS
#: §2.15: "a request logged by the owner writes the value "<redacted>"".
REDACTED = "<redacted>"


# -- §2.8 time on the wire ------------------------------------------------------------

_RFC3339 = re.compile(r"^(\d{4})-(\d{2})-(\d{2})[Tt ](\d{2}):(\d{2}):(\d{2})(\.\d+)?([Zz]|[+-]\d{2}:\d{2})$")


def parse_time(text: str) -> int:
    """An RFC 3339 date-time to nanoseconds since the epoch: "a reader
    accepts any RFC 3339 date-time" (§2.8), with ``Z`` or an offset, and
    any number of fraction digits (beyond nine, truncated)."""
    m = _RFC3339.match(text)
    if m is None:
        raise ValueError(f"{text!r} is not an RFC 3339 date-time")
    y, mo, d, h, mi, s = (int(m.group(i)) for i in range(1, 7))
    frac = (m.group(7) or ".")[1:]
    sub_ns = int((frac + "000000000")[:9]) if frac else 0
    off = m.group(8)
    secs = calendar.timegm((y, mo, d, h, mi, min(s, 59), 0, 0, 0)) + (1 if s == 60 else 0)
    if off not in ("Z", "z"):
        sign = 1 if off[0] == "+" else -1
        secs -= sign * (int(off[1:3]) * 3600 + int(off[4:6]) * 60)
    return secs * NS + sub_ns


def write_time(ns: int) -> str:
    """§2.8: "An owner writes them to the millisecond,
    ``YYYY-MM-DDTHH:MM:SS.sssZ``", in UTC. A finer instant is truncated to
    its millisecond."""
    secs, rem = divmod(ns, NS)
    return time.strftime("%Y-%m-%dT%H:%M:%S", time.gmtime(secs)) + f".{rem // MS:03d}Z"


def window(confirm_s: int, floor_s: int) -> int:
    """§2.8: "The window is max(confirm_s, confirm_floor_s) seconds"."""
    return max(confirm_s, floor_s)


def deadline(received_ns: int, confirm_s: int, floor_s: int) -> int:
    """§2.8: "the instant the owner received the set (or the extend) plus
    the window"."""
    return received_ns + window(confirm_s, floor_s) * NS


# -- §2.2, §2.5 values ----------------------------------------------------------------

class _Unknown:
    """A value the owner cannot read back: "absence is unknown, never a
    guessed value" (§2.3)."""

    def __repr__(self) -> str:
        return "UNKNOWN"


UNKNOWN = _Unknown()


def is_integer(v: Any) -> bool:
    """§2.5: "An integer is written without a fraction or an exponent … and
    a value beyond 64 bits is out of every parameter's bounds". A decoded
    float (``2.0``, ``2e0``, ``1.5``) is never one; neither is a boolean."""
    return type(v) is int


def kind_of(v: Any) -> str | None:
    """The kind a JSON value is of, or None (``null``, a float, an array, an
    object)."""
    if type(v) is bool:
        return "bool"
    if type(v) is int:
        return "integer"
    if type(v) is str:
        return "text"
    return None


def value_problem(param: dict[str, Any], v: Any) -> str | None:
    """§2.5 row 11's second half: why ``v`` is not a value of ``param``
    ("not of the parameter's kind, or outside its bounds"), or None. The
    reason names the kind and the bounds, never the value (§2.15)."""
    k = param["kind"]
    if kind_of(v) != k:
        return f"is {k}"
    if k == "integer":
        lo, hi = param.get("min", I64_MIN), param.get("max", I64_MAX)
        if not (max(lo, I64_MIN) <= v <= min(hi, I64_MAX)):
            return f"is an integer from {lo} to {hi}" if "min" in param or "max" in param \
                else "is a signed 64-bit integer"
    return None


def json_equal(a: Any, b: Any) -> bool:
    """Equality of two JSON values, kinds included: ``1`` is not ``true``,
    and ``2`` is not ``2.0``."""
    if type(a) is not type(b):
        return False
    if isinstance(a, dict):
        return a.keys() == b.keys() and all(json_equal(a[k], b[k]) for k in a)
    if isinstance(a, list):
        return len(a) == len(b) and all(json_equal(x, y) for x, y in zip(a, b))
    return a == b


def bytewise(names: Any) -> list[str]:
    """§2.5: "the bytewise order of their names (UTF-8)"."""
    return sorted(names, key=lambda n: n.encode("utf-8", "surrogatepass"))


def pointer(*parts: str) -> str:
    """An RFC 6901 JSON pointer: ``~`` written ``~0`` and ``/`` written ``~1``."""
    return "".join("/" + p.replace("~", "~0").replace("/", "~1") for p in parts)


# -- the refusals (core §5.2, §2.5) ---------------------------------------------------

@dataclass
class Refusal:
    """One refusal: core §5.2's code, its cause (``unavailable``), its
    ``ConfigError`` (``app``), and what it is about, as the fixtures write
    it: ``group`` for the key's ``{group}``, a JSON pointer into the
    request, or None for the owner's state or the operation. The message
    names that, never a value (§2.5, §2.15)."""

    code: str
    field: str | None
    message: str
    cause: str | None = None
    app: str | None = None
    detail: str | None = None

    def expect(self) -> dict[str, Any]:
        out: dict[str, Any] = {"code": self.code, "field": self.field}
        if self.cause is not None:
            out["cause"] = self.cause
        if self.app is not None:
            out["app"] = self.app
        return out

    def envelope_kw(self) -> dict[str, Any]:
        """The keywords of :func:`zk2py.envelope.encode`: a cause on
        ``unavailable``, a ``ConfigError`` detail on ``app``."""
        kw: dict[str, Any] = {}
        if self.cause is not None:
            kw["cause"] = self.cause
        if self.code == "app":
            err: dict[str, Any] = {"code": self.app}
            if self.detail is not None:
                err["detail"] = self.detail
            kw["detail"] = err
        return kw


def _bad(field: str | None, message: str) -> Refusal:
    return Refusal("invalid_request", field, message)


# -- the requests' own types (§2.5 "A request outside its type", §2.10) ---------------

def _check_member(name: str, v: Any) -> Refusal | None:
    # The request's own integer members are read as core §3.3 reads an
    # integer, within the unsigned 64-bit range: 30.0, 3e1 or 2^64 is
    # outside the request's type (§2.5, text 0.3).
    if name in ("confirm_s",):
        if not is_integer(v) or not 1 <= v <= U64_MAX:
            return _bad(pointer(name), f"{name} is an integer number of seconds, at least 1 (§2.8)")
    elif name == "expected_revision":
        if not is_integer(v) or not 0 <= v <= U64_MAX:
            return _bad(pointer(name), "expected_revision is an integer, at least 0")
    elif name == "token":
        if type(v) is not str or not 1 <= len(v) <= 64:
            return _bad(pointer(name), "token is a string of 1 to 64 characters (§2.6)")
    elif name == "idempotency_key":
        if type(v) is not str or not 1 <= len(v) <= 128:
            return _bad(pointer(name), "idempotency_key is a string of 1 to 128 characters (§2.14)")
    elif name == "dry_run":
        if type(v) is not bool:
            return _bad(pointer(name), "dry_run is a boolean")
    elif name == "values":
        if not isinstance(v, dict):
            return _bad(pointer(name), "values is an object of parameter values")
    elif name == "answer":
        if v not in ("view", "status"):
            return _bad(pointer(name), "answer is view or status (§2.10)")
    return None


def is_param_value(v: Any) -> bool:
    """A ``ParamValue`` (§3): a boolean, an integer or a string, as JSON
    Schema reads them. ``2.0`` and ``2e0`` are integers to JSON Schema, and
    so are integers beyond 64 bits: of the request's type, and refused at
    row 11 against the parameter's kind (§2.5, 0.2). ``null``, a number with
    a fractional part, an array and an object are not."""
    if type(v) in (bool, int, str):
        return True
    return type(v) is float and v.is_integer()


def decode_set(request: Any) -> Refusal | None:
    """A ``SetRequest`` (§3, §2.5 "A request outside its type", 0.2): an
    object, its members of their types, ``values`` an object of
    ``ParamValue``. A member a reader does not know is ignored (the types
    are open, §3). Several faults: "the first in the bytewise order of the
    member names, ``values`` last, and in ``values`` the first in the
    bytewise order of the parameter names"."""
    if not isinstance(request, dict):
        return _bad("", "the request is a SetRequest object")
    for name in bytewise(n for n in request if n in
                         ("confirm_s", "dry_run", "expected_revision", "idempotency_key", "token")):
        r = _check_member(name, request[name])
        if r is not None:
            return r
    if "values" not in request:
        return _bad(pointer("values"), "a SetRequest carries values")
    r = _check_member("values", request["values"])
    if r is not None:
        return r
    for name in bytewise(request["values"]):
        if not is_param_value(request["values"][name]):
            return _bad(pointer("values", name), f"the value of {name!r} is a boolean, an integer or a string "
                                                 "(ParamValue, §3)")
    return None


def decode_control(request: Any) -> Refusal | None:
    """A ``ControlRequest`` (§3): ``token`` required, ``confirm_s`` and
    ``answer`` optional, each of its type (§2.10)."""
    if not isinstance(request, dict):
        return _bad("", "the request is a ControlRequest object")
    for name in bytewise(n for n in request if n in ("answer", "confirm_s", "token")):
        r = _check_member(name, request[name])
        if r is not None:
            return r
    if "token" not in request:
        return _bad(pointer("token"), "a ControlRequest carries the change's token")
    return None


def redacted_request(group: dict[str, Any], request: dict[str, Any]) -> dict[str, Any]:
    """§2.15: "a request logged by the owner writes the value
    "<redacted>"", for each sensitive parameter's value."""
    sensitive = {p["name"] for p in group["parameters"] if p.get("sensitive")}
    out = dict(request)
    if isinstance(request.get("values"), dict):
        out["values"] = {k: (REDACTED if k in sensitive else v) for k, v in request["values"].items()}
    return out


# -- §2.5, §2.10, §2.13: the order of refusals -----------------------------------------

@dataclass
class Facts:
    """What the order of refusals reads of the owner, at the instant it
    checks (validation.json's ``state``)."""

    revision: int = 0
    in_flight: bool = False
    pending_token: str | None = None
    pending_applied: bool = True
    deadline_passed: bool = False
    last_token: str | None = None
    last_sensitive: bool = False
    persist: bool = True


def find_group(schema: list[dict[str, Any]], name: Any) -> dict[str, Any] | None:
    return next((g for g in schema if g["name"] == name), None)


def find_param(group: dict[str, Any], name: str) -> dict[str, Any] | None:
    return next((p for p in group["parameters"] if p["name"] == name), None)


def _busy_pending(f: Facts) -> Refusal:
    return Refusal("busy", None, f"change {f.pending_token} is pending: read its token from the view or the "
                                 "status (§2.7)")


def check_set(schema: list[dict[str, Any]], group_name: str, request: dict[str, Any], f: Facts) -> Refusal | None:
    """§2.5's rows 1 and 3 to 11, after the request's own type
    (:func:`decode_set`, which the caller has run): the first that applies,
    or None. Row 2, an idempotency key's first answer, reads the owner's
    state and is :class:`Core`'s, as are rows 12 to 14."""
    if f.in_flight:
        return Refusal("busy", None, "an act is in flight: call again once it ends (§2.7)")
    group = find_group(schema, group_name)
    if group is None:
        return Refusal("not_found", "group", f"the schema declares no group {group_name!r} (§2.2)")
    if not group["writable"]:
        return Refusal("unavailable", "group", f"group {group_name!r} is outside the startup allowlist (§2.16)",
                       cause="config")
    token = request.get("token")
    if f.pending_token is not None and (token != f.pending_token or f.deadline_passed):
        if token == f.pending_token:
            return Refusal("busy", None, f"change {f.pending_token} is past its deadline, its undo not yet run "
                                         "(§2.11)")
        return _busy_pending(f)
    if token is not None and f.pending_token is None:
        return Refusal("not_found", pointer("token"), "no change is pending (§2.7)")
    if "expected_revision" in request and request["expected_revision"] != f.revision:
        return _bad(pointer("expected_revision"), "expected_revision is not the revision (§2.3)")
    if group["class"] == "contract":
        return Refusal("app", "group", f"group {group_name!r} is contract: change it in the startup "
                                       "configuration (§2.4)", app="restart-required")
    if token is not None and "confirm_s" in request:
        return _bad(pointer("confirm_s"), "a set joining a change carries no window of its own (§2.7)")
    if group["class"] == "reach" and "confirm_s" not in request and token is None and not request.get("dry_run"):
        return _bad(pointer("confirm_s"), f"group {group_name!r} is reach: a set carries a window or the "
                                          "pending change's token (§2.4)")
    for name in bytewise(request["values"]):
        p = find_param(group, name)
        if p is None:
            return Refusal("not_found", pointer("values", name),
                           f"group {group_name!r} declares no parameter {name!r} (§2.2)")
        why = value_problem(p, request["values"][name])
        if why is not None:
            return _bad(pointer("values", name), f"parameter {name!r} of group {group_name!r} {why} (§2.5)")
    return None


def check_control(op: str, request: dict[str, Any], f: Facts) -> Refusal | None:
    """§2.10's order for ``confirm``, ``cancel`` and ``extend``, after the
    request's own type (:func:`decode_control`)."""
    if f.in_flight:
        return Refusal("busy", None, "an act is in flight: call again once it ends (§2.7)")
    if op == "extend" and "confirm_s" not in request:
        return _bad(pointer("confirm_s"), "extend carries the new window, confirm_s (§2.10)")
    if op != "extend" and "confirm_s" in request:
        return _bad(pointer("confirm_s"), f"{op} carries no window (§2.10)")
    if f.pending_token is None or request["token"] != f.pending_token:
        return Refusal("not_found", pointer("token"), "no change is pending under this token (§2.10)")
    if f.deadline_passed:
        return Refusal("busy", None, f"change {f.pending_token} is past its deadline, its undo not yet run (§2.11)")
    if op == "confirm" and not f.pending_applied:
        return Refusal("busy", None, f"change {f.pending_token} is not applied yet: confirm it once the view "
                                     "shows it applied (§2.9)")
    return None


def persist_unavailable(f: Facts, writable: bool = True) -> Refusal | None:
    """§2.13 row 1: "no persisted layer: unavailable, cause config", which
    the core answers before any handler runs, the request's type included;
    and a layer the owner "cannot write to now … before the request's type
    too" (0.2)."""
    if not f.persist:
        return Refusal("unavailable", None, "this owner has no persisted layer (§2.13)", cause="config")
    if not writable:
        return Refusal("unavailable", None, "this owner cannot write its persisted layer now (§2.13)",
                       cause="config")
    return None


def check_persist(request: dict[str, Any], f: Facts) -> Refusal | None:
    """§2.13's order after row 1 (:func:`persist_unavailable`) and the
    request's own type."""
    if f.in_flight:
        return Refusal("busy", None, "an act is in flight: call again once it ends (§2.7)")
    if "confirm_s" in request:
        return _bad(pointer("confirm_s"), "persist carries no window (§2.13)")
    if f.pending_token is not None and request["token"] == f.pending_token:
        return Refusal("busy", None, f"change {f.pending_token} is pending: confirm it first (§2.13)")
    if f.last_token is None or request["token"] != f.last_token:
        return Refusal("not_found", pointer("token"), "persist takes the last change's token alone (§2.13)")
    if f.last_sensitive:
        return Refusal("forbidden", pointer("token"), "the last change set a sensitive parameter, which is never "
                                                      "persisted (§2.13, §2.15)")
    return None


# -- §2.18 desired state ----------------------------------------------------------------

def check_desired(schema: list[dict[str, Any]], document: dict[str, Any]) -> dict[str, Any] | None:
    """§2.18 "Refused whole": the first problem, groups in the bytewise
    order of their names, each group's checks (``not_found``, ``contract``,
    ``reach``, ``unavailable``), then its parameters in bytewise order
    (``not_found``, ``sensitive``, ``invalid_request``); None when accepted."""
    values = document.get("values") or {}
    for gname in bytewise(values):
        g = find_group(schema, gname)
        if g is None:
            return {"code": "not_found", "group": gname}
        if g["class"] == "contract":
            return {"code": "contract", "group": gname}
        if g["class"] == "reach":
            return {"code": "reach", "group": gname}
        if not g["writable"]:
            return {"code": "unavailable", "group": gname}
        for pname in bytewise(values[gname]):
            p = find_param(g, pname)
            if p is None:
                return {"code": "not_found", "group": gname, "parameter": pname}
            if p.get("sensitive"):
                return {"code": "sensitive", "group": gname, "parameter": pname}
            if value_problem(p, values[gname][pname]) is not None:
                return {"code": "invalid_request", "group": gname, "parameter": pname}
    return None


# -- the owner, as a state machine ------------------------------------------------------

@dataclass
class Param:
    """A running value: its value (or :data:`UNKNOWN`) and where it came
    from (None when unknown)."""

    value: Any
    source: str | None


@dataclass
class Wait:
    """A reach group's apply, armed and not yet made (§2.9)."""

    group: str
    values: dict[str, Any]
    apply_at: int


@dataclass
class Change:
    """A change (§2.6, §2.7): its token, its groups in the order they first
    joined, its deadline (None without a window), the attribution of the
    ``set`` that started it, each parameter's value before it and the
    last value it set, the parameters an apply of it has moved, and its
    reach applies still waiting."""

    token: str
    groups: list[str]
    deadline: int | None
    call: dict[str, str]
    before: dict[tuple[str, str], Param] = field(default_factory=dict)
    new: dict[tuple[str, str], Any] = field(default_factory=dict)
    applied: set[tuple[str, str]] = field(default_factory=set)
    waits: list[Wait] = field(default_factory=list)
    sensitive: bool = False

    @property
    def all_applied(self) -> bool:
        return not self.waits


@dataclass
class Kept:
    """A first answer held under an idempotency key (§2.14)."""

    group: str
    request: dict[str, Any]
    answer: Any                     # a reply dict, or a Refusal
    at: int


@dataclass
class Step:
    """What one call or one instant made: the reply (None when there is
    none, or when the act is held), the device acts started, and the change
    events published."""

    reply: Any = None
    acts: list[str] = field(default_factory=list)
    events: list[dict[str, Any]] = field(default_factory=list)


#: a device result that leaves the act in flight (lifecycle.json's "hold")
HOLD = "hold"


def _call_meta(call: dict[str, Any] | None) -> dict[str, str]:
    """Core O7: the call metadata, ``actor`` and ``request_id`` where they
    are strings."""
    if not isinstance(call, dict):
        return {}
    return {k: call[k] for k in ("actor", "request_id") if isinstance(call.get(k), str)}


class Core:
    """One ``config.v1`` owner, from the text (§2.3, §2.6 to §2.14, §2.17,
    §2.18): calls, device results and the owner's clock in; replies, device
    acts, change events and the view out.

    - ``schema``: the groups as the view carries them, ``writable`` being
      the startup allowlist (§2.16);
    - ``startup``: ``{group: {parameter: {value, source}}}``, the startup
      file's (``file``) or the default (``default``) values;
    - ``overlay``: the persisted layer, ``{group: {parameter: value}}``,
      kept by reference so that it survives a restart (§2.12, §2.13);
    - ``device``: a function of an act, ``("apply", group, values)`` or
      ``("undo", {group: {parameter: value}})``, returning ``"ok"``,
      ``{"refused": words}`` (an apply), ``{"failed": words, "readback":
      {group: {parameter: value}}}`` (an undo), or :data:`HOLD`;
    - ``events_at``: the instants of change events published, kept by
      reference so that the rate survives a restart (§2.17);
    - ``log``: the owner's log lines, each request redacted (§2.15)."""

    def __init__(self, *, service: str, instance: str, schema: list[dict[str, Any]],
                 startup: dict[str, dict[str, dict[str, Any]]], overlay: dict[str, dict[str, Any]] | None = None,
                 floor_s: int = 60, apply_after_ns: int = NS, persist: bool = True,
                 device: Callable[[tuple], Any] | None = None, events_at: list[int] | None = None):
        if floor_s < 1:
            raise ValueError("the floor is at least 1 s (§2.8)")
        if len(schema) > 16:
            raise ValueError("at most 16 groups (§2.2)")
        if apply_after_ns >= floor_s * NS:
            # §2.9: apply_at "MUST state one before the deadline"; a window
            # is never shorter than the floor, which an owner MUST keep above
            # its apply's delay (§2.8 "Above the apply's delay", 0.2).
            raise ValueError("apply_at would not come before the smallest window's deadline (§2.9)")
        self.service, self.schema = service, schema
        self.startup_file = startup
        self.overlay = overlay if overlay is not None else {}
        self.floor_s, self.apply_after_ns, self.persist_layer = floor_s, apply_after_ns, persist
        self.device = device or (lambda act: "ok")
        #: §2.13 row 1 (0.2): whether the persisted layer can be written now
        self.layer_writable = True
        self.events_at = events_at if events_at is not None else []
        self.log: list[str] = []
        self.instance = ""
        self.desired_doc: dict[str, Any] | None = None
        self._start(instance)

    # -- §2.12 a new instance ------------------------------------------------------

    def _start(self, instance: str) -> None:
        """§2.12: "An owner MUST start from its startup values (its persisted
        layer, its startup file, its defaults)"; a new instance id, the
        revision from 0, nothing pending, no last change, no key held."""
        self.instance = instance
        self.revision = 0
        self.pending: Change | None = None
        self.last_change: dict[str, Any] | None = None
        self.minted = 0
        self.keys: dict[str, Kept] = {}
        self.in_flight: tuple[str, Callable[[Any], Step]] | None = None
        self.held_step: Step | None = None
        self.desired: dict[str, Any] | None = None
        self._desired_waiting = False
        #: desired.v1 §2.16: a revert of what this instance's documents set,
        #: under way (:meth:`revert`)
        self._reverting = False
        #: how the last desired act ended, for an owner fed through
        #: desired.v1 (:class:`ConfigOwner`): ``(outcome, refusal)``, outcome
        #: ``converged``, ``refused``, ``failed`` or ``reverted``; None until one
        #: ends, and taken by its reader
        self.desired_end: tuple[str, dict[str, Any] | None] | None = None
        self.running: dict[tuple[str, str], Param] = {}
        for g in self.schema:
            for p in g["parameters"]:
                v, src = self.startup_value(g["name"], p["name"])
                self.running[(g["name"], p["name"])] = Param(v, src)

    def restart(self, instance: str) -> Step:
        """A restart (§2.12): the persisted layer and the published events
        survive it; the new instance says nothing of the old one's pending
        change."""
        self._start(instance)
        return Step()

    def startup_value(self, group: str, param: str) -> tuple[Any, str]:
        """§2.3 "Startup value": "the persisted layer's, else the startup
        file's, else the owner's default"."""
        if param in self.overlay.get(group, {}):
            return self.overlay[group][param], "overlay"
        s = self.startup_file.get(group, {}).get(param)
        if s is None:
            return UNKNOWN, None  # type: ignore[return-value]
        return s["value"], s["source"]

    # -- facts and documents -----------------------------------------------------------

    def facts(self, now: int) -> Facts:
        p = self.pending
        return Facts(
            revision=self.revision, in_flight=self.in_flight is not None,
            pending_token=None if p is None else p.token,
            pending_applied=True if p is None else p.all_applied,
            deadline_passed=p is not None and p.deadline is not None and now >= p.deadline,
            last_token=None if self.last_change is None else self.last_change["token"],
            last_sensitive=bool(self.last_change and self.last_change["sensitive"]),
            persist=self.persist_layer)

    def _sensitive(self, group: str, param: str) -> bool:
        g = find_group(self.schema, group)
        p = None if g is None else find_param(g, param)
        return bool(p and p.get("sensitive"))

    def _pending_doc(self) -> dict[str, Any] | None:
        p = self.pending
        if p is None:
            return None
        return {"token": p.token, "groups": list(p.groups), "deadline": write_time(p.deadline or 0),
                "applied": p.all_applied}

    def view(self, running: dict[tuple[str, str], Param] | None = None) -> dict[str, Any]:
        """The read-back, a ``ConfigView`` (§2.3): per parameter its
        declaration, ``value`` (never a sensitive one, never a guess),
        ``source``, and ``startup`` only where it differs from a known
        running value and the parameter is not sensitive (§2.15)."""
        running = self.running if running is None else running
        groups = []
        for g in self.schema:
            params = []
            for decl in g["parameters"]:
                key = (g["name"], decl["name"])
                cur = running[key]
                p = {k: v for k, v in decl.items() if k not in ("value", "source", "startup")}
                if cur.value is not UNKNOWN and not decl.get("sensitive"):
                    p["value"] = cur.value
                if cur.source is not None:
                    # An unknown value has no source (lifecycle.json case 4,
                    # §2.3, 0.2).
                    p["source"] = cur.source
                start, _ = self.startup_value(*key)
                if not decl.get("sensitive") and cur.value is not UNKNOWN and start is not UNKNOWN \
                        and not json_equal(start, cur.value):
                    p["startup"] = start
                params.append(p)
            groups.append({"name": g["name"], "class": g["class"], "description": g.get("description", ""),
                           "writable": g["writable"], "parameters": params})
        doc: dict[str, Any] = {"service": self.service, "instance": self.instance, "revision": self.revision,
                               "confirm_floor_s": self.floor_s}
        if self.pending is not None:
            doc["pending"] = self._pending_doc()
        if self.last_change is not None:
            doc["last_change"] = {"token": self.last_change["token"], "groups": list(self.last_change["groups"])}
        if self.desired is not None:
            doc["desired"] = dict(self.desired)
        doc["groups"] = groups
        return doc

    def status(self) -> dict[str, Any]:
        return status_of(self.view())

    def _reply_doc(self, request: dict[str, Any]) -> dict[str, Any]:
        """§2.10 "The reply": the view, or the status when asked."""
        if request.get("answer") == "status":
            return {"status": self.status()}
        return {"view": self.view()}

    # -- the device ----------------------------------------------------------------------

    def _act(self, kind: str, act: tuple, step: Step, then: Callable[[Any], None]) -> bool:
        """Start one device act. Returns False when the device holds it in
        flight (§2.7): ``then`` runs when :meth:`complete` brings its
        result."""
        (self.held_step or step).acts.append(kind)
        result = self.device(act)
        if result == HOLD:
            self.in_flight = (kind, then)
            return False
        then(result)
        return True

    def complete(self, result: Any, now: int) -> Step:
        """The act left in flight ends with ``result``; its call's reply, if
        it had one, comes with this step."""
        if self.in_flight is None:
            raise RuntimeError("no act is in flight")
        _, then = self.in_flight
        self.in_flight = None
        step = Step()
        self.held_step = step
        self._now = now
        then(result)
        self.held_step = None
        self._desire_again(now, step)
        return step

    # -- §2.17 events ----------------------------------------------------------------------

    def _rate_spent(self, now: int) -> bool:
        """§2.17: "while 600 change events were published within the last
        hour": an event exactly an hour old no longer counts, as
        observer §5.2's span (§2.17, 0.2)."""
        return sum(1 for t in self.events_at if now - RATE_PERIOD_NS < t <= now) >= RATE_N

    def _end(self, change: Change, outcome: str, step: Step, now: int) -> None:
        """A change ends: its one event (§2.17)."""
        edits = []
        for g in change.groups:
            for name in bytewise({p for (gg, p) in change.new if gg == g}):
                key = (g, name)
                if self._sensitive(g, name):
                    edits.append({"group": g, "parameter": name, "redacted": True})
                    continue
                e: dict[str, Any] = {"group": g, "parameter": name}
                old = change.before[key].value
                if old is not UNKNOWN:
                    e["old"] = old
                e["new"] = change.new[key]
                edits.append(e)
        ev: dict[str, Any] = {"service": self.service, "instance": self.instance, "revision": self.revision,
                              "token": change.token, "outcome": outcome, "edits": edits, **change.call}
        self.events_at.append(now)
        step.events.append(ev)

    def _mint(self) -> str:
        """§2.6: ``<instance>-<n>``, n counting the instance's changes from 1;
        never a token another instance minted, the instance id being new."""
        self.minted += 1
        return f"{self.instance}-{self.minted}"

    # -- applying and undoing ---------------------------------------------------------------

    def _record(self, change: Change, group: str, values: dict[str, Any]) -> None:
        for name, v in values.items():
            key = (group, name)
            if key not in change.before:
                change.before[key] = Param(self.running[key].value, self.running[key].source)
            change.new[key] = v
            if self._sensitive(group, name):
                change.sensitive = True
        if group not in change.groups:
            change.groups.append(group)

    def _moved(self, change: Change | None, group: str, values: dict[str, Any]) -> None:
        for name, v in values.items():
            self.running[(group, name)] = Param(v, "runtime")
            if change is not None:
                change.applied.add((group, name))
        self.revision += 1

    def _undo(self, change: Change, outcome_step: Step, now: int, done: Callable[[], None] | None = None) -> None:
        """§2.11: one act over the change's applied groups, once; nothing
        applied, no act and no revision. Ends the change; while the undo is
        in flight, the change is still pending, and every call is busy
        (§2.7)."""

        def finish(outcome: str) -> None:
            self.pending = None
            self._end(change, outcome, outcome_step if self.held_step is None else self.held_step, self._now)
            if done is not None:
                done()

        if not change.applied:
            finish("rolled-back")
            return
        restore: dict[str, dict[str, Any]] = {}
        for g, p in sorted(change.applied):
            before = change.before[(g, p)]
            restore.setdefault(g, {})[p] = before.value

        def then(result: Any) -> None:
            self.revision += 1
            if result == "ok" or result is None:
                for key in change.applied:
                    b = change.before[key]
                    self.running[key] = Param(b.value, b.source)
                finish("rolled-back")
                return
            readback = (result or {}).get("readback") or {}
            for key in change.applied:
                g, p = key
                if p in readback.get(g, {}):
                    v, b = readback[g][p], change.before[key]
                    same = b.value is not UNKNOWN and json_equal(v, b.value)
                    self.running[key] = Param(v, b.source if same else "runtime")
                else:
                    self.running[key] = Param(UNKNOWN, None)
            finish("partial")

        self._act("undo", ("undo", restore), outcome_step, then)

    # -- the calls ---------------------------------------------------------------------------

    def _row2(self, request: dict[str, Any], group: str, now: int) -> Any:
        """§2.5 row 2: the key's first answer for the same group and an
        equal request; ``invalid_request`` (never kept) for another; None
        when no first answer is held under the key (§2.14)."""
        key = request.get("idempotency_key")
        kept = None if key is None else self.keys.get(key)
        if kept is not None and now >= kept.at + KEY_TTL_NS:
            # §2.14: "At 300 s and after, the key is forgotten".
            del self.keys[key]
            kept = None
        if kept is None:
            return None
        if kept.group == group and _same_request(kept.request, request):
            return kept.answer
        return _bad(pointer("idempotency_key"), "this idempotency key's first answer was to another group or "
                                                "request (§2.14)")

    def set(self, group: str, request: Any, now: int, call: dict[str, Any] | None = None) -> Step:
        """``@op/set/{group}`` (§2.5, §2.9): the request's type, rows 1 to
        14, then a dry run, a join or a new change. A refusal from row 3 on,
        and every reply, is the first answer of an idempotency key (§2.14)."""
        self._now = now
        step = Step()
        r = decode_set(request)
        g = find_group(self.schema, group)
        if r is not None:
            step.reply = r
            return step
        self.log.append(json.dumps({"op": "set", "group": group,
                                    "request": redacted_request(g, request) if g else request}, sort_keys=True))
        f = self.facts(now)
        # Row 1 and row 2 are answered before the key is kept (§2.14: "A
        # refusal before it … is not kept", nor is row 2's own).
        if f.in_flight:
            step.reply = check_set(self.schema, group, request, f)
            return step
        key = request.get("idempotency_key")
        first = self._row2(request, group, now)
        if first is not None:
            step.reply = first
            return step
        r = check_set(self.schema, group, request, f)

        def answer(reply: Any) -> None:
            if key is not None:
                self.keys[key] = Kept(group, dict(request), reply, self._now)
            (self.held_step or step).reply = reply

        if r is not None:
            answer(r)
            return step
        assert g is not None
        values = request["values"]
        if request.get("dry_run"):
            would = dict(self.running)
            for name, v in values.items():
                would[(group, name)] = Param(v, "runtime")
            answer({"applied": self.view(would)})
            return step
        joining = self.pending is not None and request.get("token") == self.pending.token
        # Row 12: the event's rate, joins aside.
        if not joining and self._rate_spent(now):
            answer(Refusal("busy", None, "the change event's rate, burst(600/h), is spent (§2.17)"))
            return step
        if g["class"] == "reach":
            apply_at = now + self.apply_after_ns
            if joining:
                assert self.pending is not None
                if self.pending.deadline is None or apply_at >= self.pending.deadline:
                    # Row 13.
                    answer(Refusal("busy", None, f"group {group!r} could not apply before change "
                                                 f"{self.pending.token}'s deadline: extend it first (§2.9)"))
                    return step
                change = self.pending
            else:
                change = Change(self._mint(), [], deadline(now, request["confirm_s"], self.floor_s),
                                _call_meta(call))
                self.pending = change
            self._record(change, group, values)
            change.waits.append(Wait(group, dict(values), apply_at))
            answer({"pending": {"token": change.token, "apply_at": write_time(apply_at),
                                "deadline": write_time(change.deadline or 0)}})
            return step
        # Hot: the apply now (row 14 at its result). An empty values is an
        # act that sets no value, and moves the revision (§2.5, §2.3, 0.2).

        def then(result: Any) -> None:
            if isinstance(result, dict) and "refused" in result:
                sens = any(self._sensitive(group, n) for n in values)
                words = result.get("refused")
                refusal = Refusal("app", "group", f"the device refused group {group!r} (§2.9)", app="device-refused",
                                  detail=None if sens or not isinstance(words, str) else words)
                partial = result.get("partial") or {}
                if partial:
                    self._restore_partial(group, partial, step, lambda: answer(refusal))
                    return
                answer(refusal)
                return
            if joining:
                change = self.pending
                assert change is not None
                self._record(change, group, values)
                self._moved(change, group, values)
                answer({"applied": self.view()})
                return
            if "confirm_s" in request:
                change = Change(self._mint(), [], deadline(now, request["confirm_s"], self.floor_s), _call_meta(call))
                self._record(change, group, values)
                self._moved(change, group, values)
                self.pending = change
                answer({"applied": self.view()})
                return
            change = Change(self._mint(), [], None, _call_meta(call))
            self._record(change, group, values)
            self._moved(change, group, values)
            self.last_change = {"token": change.token, "groups": list(change.groups), "sensitive": change.sensitive,
                                "values": dict(change.new)}
            answer({"applied": self.view()})
            self._end(change, "applied", self.held_step or step, self._now)

        self._act("apply", ("apply", group, dict(values)), step, then)
        return step

    def _restore_partial(self, group: str, partial: dict[str, Any], step: Step, done: Callable[[], None]) -> None:
        """§2.9 "A hot group left part applied" (text 0.3): a device result
        ``{"refused": words, "partial": {parameter: value}}`` left those
        parameters applied. The owner restores them once, as an undo
        restores, before it answers; the refusal still starts no change. A
        restore that succeeds moves nothing, and is no act on the running
        values; one that fails leaves the read-back the truth, and moves the
        revision by one."""
        restore = {group: {p: self.running[(group, p)].value for p in partial}}

        def then(result: Any) -> None:
            if result == "ok" or result is None:
                done()
                return
            readback = ((result or {}).get("readback") or {}).get(group, {})
            for p in partial:
                before = self.running[(group, p)]
                if p in readback:
                    same = before.value is not UNKNOWN and json_equal(readback[p], before.value)
                    self.running[(group, p)] = before if same else Param(readback[p], "runtime")
                else:
                    self.running[(group, p)] = Param(UNKNOWN, None)
            self.revision += 1
            done()

        self._act("undo", ("undo", restore), step, then)

    def control(self, op: str, request: Any, now: int) -> Step:
        """``confirm``, ``cancel`` and ``extend`` (§2.10)."""
        self._now = now
        step = Step()
        r = decode_control(request) or check_control(op, request, self.facts(now))
        if r is not None:
            step.reply = r
            return step
        change = self.pending
        assert change is not None
        if op == "extend":
            change.deadline = deadline(now, request["confirm_s"], self.floor_s)
            step.reply = self._reply_doc(request)
            return step
        if op == "confirm":
            self.pending = None
            self.last_change = {"token": change.token, "groups": list(change.groups), "sensitive": change.sensitive,
                                "values": dict(change.new)}
            self._end(change, "confirmed", step, now)
            step.reply = self._reply_doc(request)
            self._desire_again(now, step)
            return step

        def done() -> None:
            (self.held_step or step).reply = self._reply_doc(request)
            self._desire_again(self._now, self.held_step or step)

        self._undo(change, step, now, done)
        return step

    def confirm(self, request: Any, now: int) -> Step:
        return self.control("confirm", request, now)

    def cancel(self, request: Any, now: int) -> Step:
        return self.control("cancel", request, now)

    def extend(self, request: Any, now: int) -> Step:
        return self.control("extend", request, now)

    def persist(self, request: Any, now: int) -> Step:
        """``persist`` (§2.13): the last change into the persisted layer;
        its parameters read ``source: overlay``, and ``startup`` leaves
        them. No act, no revision, no event."""
        step = Step()
        r = persist_unavailable(self.facts(now), self.layer_writable) or decode_control(request) \
            or check_persist(request, self.facts(now))
        if r is not None:
            step.reply = r
            return step
        assert self.last_change is not None
        # §2.13 "What it writes" (0.2): "The values the last change set, as
        # it set them: never a value a pending change runs since".
        for (g, p), v in self.last_change["values"].items():
            self.overlay.setdefault(g, {})[p] = v
            cur = self.running[(g, p)]
            pending = self.pending
            if pending is not None and (g, p) in pending.before:
                # The value before the pending change, when it is the value
                # written, is restored at source overlay (§2.13 "After it").
                b = pending.before[(g, p)]
                if b.value is not UNKNOWN and json_equal(b.value, v):
                    pending.before[(g, p)] = Param(v, "overlay")
                if (g, p) in pending.applied:
                    # "keeps its running value and its source, and shows the
                    # value written as its startup".
                    continue
            # "Each parameter the change set that runs the value written
            # reads source: overlay", a reach group not yet applied
            # included.
            if cur.value is not UNKNOWN and json_equal(cur.value, v):
                self.running[(g, p)] = Param(v, "overlay")
        step.reply = self._reply_doc(request)
        return step

    def due(self, now: int) -> Step:
        """Every act owed at ``now``: the undo once the deadline has come
        (the deadline instant belongs to the undo, §2.11), else each reach
        apply whose ``apply_at`` has come, earliest first (§2.9)."""
        self._now = now
        step = Step()
        change = self.pending
        if change is None or self.in_flight is not None:
            return step
        if change.deadline is not None and now >= change.deadline:
            self._undo(change, step, now)
            self._desire_again(now, step)
            return step
        for w in sorted([w for w in change.waits if w.apply_at <= now], key=lambda w: w.apply_at):
            if self.pending is not change or self.in_flight is not None:
                break
            if not self._apply_wait(change, w, step, now):
                break
        self._desire_again(now, step)
        return step

    def _apply_wait(self, change: Change, w: Wait, step: Step, now: int) -> bool:
        def then(result: Any) -> None:
            change.waits.remove(w)
            if isinstance(result, dict) and "refused" in result:
                # §2.9: "A reach group whose apply fails at apply_at … ends
                # the change: the owner runs the undo at once".
                self._undo(change, step, self._now)
                return
            self._moved(change, w.group, w.values)

        return self._act("apply", ("apply", w.group, dict(w.values)), step, then)

    def next_due(self) -> int | None:
        """The next instant :meth:`due` has something to do, for a live
        owner's timer."""
        p = self.pending
        if p is None:
            return None
        times = [w.apply_at for w in p.waits]
        if p.deadline is not None:
            times.append(p.deadline)
        return min(times) if times else None

    # -- §2.18 desired state --------------------------------------------------------------

    def desire(self, document: dict[str, Any], now: int) -> Step:
        """A desired document bound to this owner (§2.18): refused whole
        before anything applies, or converged by one ``set`` without a
        window per group that differs, in bytewise order, each its own
        change; while a change is pending, it waits."""
        self.desired_doc = document
        self._reverting = False
        step = Step()
        refusal = check_desired(self.schema, document)
        if refusal is not None:
            self.desired = {"observed_revision": document["revision"], "outcome": "refused", "refusal": refusal}
            self._desired_waiting = False
            self.desired_end = ("refused", dict(refusal))
            return step
        self._desired_waiting = True
        self._converge(now, step)
        return step

    def revert(self, values: dict[str, dict[str, Any]], now: int) -> Step:
        """``desired.v1`` §2.16 "What a delete reverts it to" (text 0.10), for
        an owner fed through ``desired.v1``: ``values`` are the startup values
        of the parameters the documents it acted on in this run named. They go
        back as it converges (§2.18): one ``set`` without a window per group
        whose running values differ, in the bytewise order of the group names.
        It ends ``reverted``, or ``failed`` with the refusing ``set``'s code,
        the groups before it restored; and its view carries no ``desired``
        after it, either way: no document is bound."""
        self.desired_doc = {"revision": None, "values": values}
        self._reverting = True
        self._desired_waiting = True
        step = Step()
        self._converge(now, step)
        return step

    def _desire_again(self, now: int, step: Step) -> None:
        if self._desired_waiting and self.pending is None and self.in_flight is None:
            self._converge(now, step)

    def _converge(self, now: int, step: Step) -> None:
        doc = self.desired_doc
        assert doc is not None
        if self.pending is not None or self.in_flight is not None:
            return  # "While a change is pending, it waits until the change ends."
        for gname in bytewise(doc["values"]):
            want = doc["values"][gname]
            if all(json_equal(self.running[(gname, p)].value, v) for p, v in want.items()):
                continue
            sub = self.set(gname, {"values": dict(want)}, now)
            step.acts += sub.acts
            step.events += sub.events
            if isinstance(sub.reply, Refusal):
                code = sub.reply.app if sub.reply.code == "app" else sub.reply.code
                refusal = {"code": code, "group": gname}
                if self._reverting:
                    self.desired, self.desired_doc = None, None
                else:
                    self.desired = {"observed_revision": doc["revision"], "outcome": "failed", "refusal": refusal}
                self.desired_end = ("failed", refusal)
                self._desired_waiting = self._reverting = False
                return
            if sub.reply is None:
                return  # held: the convergence goes on when it completes
        if self._reverting:
            self.desired, self.desired_doc = None, None
            self.desired_end = ("reverted", None)
        else:
            self.desired = {"observed_revision": doc["revision"], "outcome": "converged"}
            self.desired_end = ("converged", None)
        self._desired_waiting = self._reverting = False


def _same_request(a: dict[str, Any], b: dict[str, Any]) -> bool:
    """§2.14: "an equal request (every member of SetRequest equal)", equal
    as decoded (text 0.3): "A member at its default is the member absent, so
    "dry_run": false and no dry_run are one request; every other member, and
    each value of values, compares as the JSON value it is"."""
    def decoded(r: dict[str, Any]) -> dict[str, Any]:
        return {m: r[m] for m in ("values", "confirm_s", "token", "expected_revision", "idempotency_key", "dry_run")
                if m in r and not (m == "dry_run" and r[m] is False)}

    da, db = decoded(a), decoded(b)
    return da.keys() == db.keys() and all(json_equal(da[m], db[m]) for m in da)


def status_of(view: dict[str, Any]) -> dict[str, Any]:
    """§2.19: the status is the projection of the view: its instance,
    revision, floor, pending, last change and desired report, and the values
    by group and name, a sensitive or unknown value left out, and a group
    left with none."""
    out: dict[str, Any] = {k: view[k] for k in ("instance", "revision", "confirm_floor_s")}
    for k in ("pending", "last_change", "desired"):
        if k in view:
            out[k] = view[k]
    values: dict[str, dict[str, Any]] = {}
    for g in view["groups"]:
        vs = {p["name"]: p["value"] for p in g["parameters"] if "value" in p and not p.get("sensitive")}
        if vs:
            values[g["name"]] = vs
    out["values"] = values
    return out


def project_reply(reply: Any) -> Any:
    """A reply as lifecycle.json writes it: each document its status
    projection, a refusal as validation.json writes it."""
    if reply is None:
        return None
    if isinstance(reply, Refusal):
        return {"refused": reply.expect()}
    (k, v), = reply.items()
    if k in ("applied", "view"):
        return {k: status_of(v)}
    return {k: v}


def encode_json(doc: Any) -> bytes:
    """A payload: compact JSON, integers as integers (64-bit values
    included, which RFC 8785's double range would refuse)."""
    return json.dumps(doc, separators=(",", ":"), ensure_ascii=False, sort_keys=True).encode()


# -- the session half: a simulated device ------------------------------------------------

class SimDevice:
    """A simulated device (scenarios.md, "Its device is simulated: a test
    sets each act's result, and reads what the device holds"). Each act is
    logged with the owner's clock; ``next_apply`` and ``next_undo`` hold the
    results of the next acts, ``"ok"`` when empty. A failed undo leaves the
    device holding its read-back."""

    def __init__(self, values: dict[str, dict[str, Any]] | None = None):
        self.values: dict[str, dict[str, Any]] = {g: dict(v) for g, v in (values or {}).items()}
        self.next_apply: list[Any] = []
        self.next_undo: list[Any] = []
        self.log: list[tuple[int, str, Any, Any]] = []
        self.lock = threading.Lock()

    def boot(self, values: dict[str, dict[str, Any]]) -> None:
        """What an owner gives its device at start: its startup values."""
        with self.lock:
            self.values = {g: dict(v) for g, v in values.items()}
            self.log.append((time.time_ns(), "boot", None, None))

    def acts(self, kind: str) -> list[tuple[int, str, Any, Any]]:
        with self.lock:
            return [x for x in self.log if x[1] == kind]

    def __call__(self, act: tuple) -> Any:
        with self.lock:
            now = time.time_ns()
            if act[0] == "apply":
                _, group, values = act
                result = self.next_apply.pop(0) if self.next_apply else "ok"
                if result == "ok":
                    self.values.setdefault(group, {}).update(values)
                self.log.append((now, "apply", (group, values), result))
                return result
            _, restore = act
            result = self.next_undo.pop(0) if self.next_undo else "ok"
            if result == "ok":
                for g, ps in restore.items():
                    self.values.setdefault(g, {}).update(ps)
            else:
                readback = (result.get("readback") or {}) if isinstance(result, dict) else {}
                for g, ps in restore.items():
                    for p in ps:
                        if p in readback.get(g, {}):
                            self.values.setdefault(g, {})[p] = readback[g][p]
                        else:
                            self.values.get(g, {}).pop(p, None)
            self.log.append((now, "undo", restore, result))
            return result


# -- the session half: the owner ------------------------------------------------------

class ConfigOwner:
    """A service's ``config.v1`` over :class:`zk2py.owner.Owner`.

    - Every resource is exposed, ``persist`` listed unavailable with cause
      ``config`` when there is no persisted layer (§2.1, §2.13);
      ``set/{group}`` by one queryable over the template.
    - The view and the status are put before the tokens (§2.3, a MUST
      here), on every change of either, and re-put every 30 s by the
      owner's ``freshness.ttl_s`` refresh (freshness.v1 §2.4); never
      deleted.
    - Each change event is an occurrence on ``events/change/<ulid>``
      (§2.17).
    - Deadlines and reach applies run on a timer thread of the owner's
      own, at their instants on its clock (§2.8, §2.9, §2.11).
    - ``desired_key``: a key holding a ``DesiredConfig`` that the
      deployment binds to this owner (§2.18), read by subscription and by
      a GET at start.
    - ``overlay`` and ``events_at`` are shared with a later instance of the
      same service: what survives a restart (§2.12, §2.17).
    - ``fed``: fed through ``desired.v1`` instead (``desired.v1`` §2.16,
      text 0.10; #609, round 88): the slice's key and its author's address,
      ``(slice, author)``. The owner reads its slice as ``desired.v1`` §2.5's
      late joiner does (subscribe, then seed; again at each appearance of its
      author's instance token, read every 500 ms on its own session, which
      holds no liveliness subscriber), judges every version with
      :class:`zk2py.desired.TargetRole` (§2.6, §2.7: once per version in a
      run, a document equal to the last one acted on adopted), acts through
      :meth:`Core.desire`, and on a delete reverts what the documents it
      acted on in this run named (:meth:`Core.revert`). Its report is its
      own, the view's and the status's ``desired`` (§2.16).
    - ``remint``: what :meth:`remint` does (core §1.5): ``"restart"``, a new
      instance from its startup values, its device returned to them first,
      and, fed through ``desired.v1``, a new run that seeds and acts on the
      bound version again (``desired.v1`` §2.16, config.v1 §2.12); or
      ``"refuse"``, as the reference does.

    The calls are serialized: the owner answers one act's call at a time
    (§2.7)."""

    def __init__(self, system: str, service: str, *, schema: list[dict[str, Any]],
                 startup: dict[str, dict[str, dict[str, Any]]], overlay: dict[str, dict[str, Any]] | None = None,
                 floor_s: int = 10, apply_after_s: float = 1.0, persist: bool = True,
                 device: SimDevice | None = None, events_at: list[int] | None = None,
                 desired_key: str | None = None, contracts: list[Any] | None = None,
                 fed: tuple[str, str] | None = None, remint: str = "restart", **owner_kw: Any):
        from .contract import load_contract
        from .owner import Owner

        self.contract = load_contract(CONTRACT)
        if not self.contract.valid:
            raise ValueError(f"{CONTRACT}: {self.contract.codes}")
        self.device = device or SimDevice()
        self.persist_layer = persist
        unavailable = dict(owner_kw.pop("unavailable", {}) or {})
        if not persist:
            unavailable["@op/persist"] = "config"
        handlers = {"@op/set/{group}": self._on_set, "@op/confirm": self._on_control("confirm"),
                    "@op/cancel": self._on_control("cancel"), "@op/extend": self._on_control("extend"),
                    "@op/persist": self._on_persist}
        self.owner = Owner(system, service, [self.contract, *(contracts or [])], handlers=handlers,
                           unavailable=unavailable, **owner_kw)
        self.core = Core(service=f"{system}/{service}", instance=self.owner.instance, schema=schema,
                         startup=startup, overlay=overlay, floor_s=floor_s, apply_after_ns=int(apply_after_s * NS),
                         persist=persist, device=self.device, events_at=events_at)
        self.view_key = f"zk2/{system}/{service}/{IFACE}/state/view"
        self.status_key = f"zk2/{system}/{service}/{IFACE}/state/status"
        self.event_member = f"zk2/{system}/{service}/{IFACE}/events/change"
        self._last_put: tuple[bytes, bytes] = (encode_json(self.core.view()), encode_json(self.core.status()))
        self.owner.initial[f"{IFACE}/state/view"] = self._last_put[0]
        self.owner.initial[f"{IFACE}/state/status"] = self._last_put[1]
        self.desired_key = desired_key
        if remint not in ("restart", "refuse"):
            raise ValueError(f"remint is restart or refuse, not {remint!r}")
        self.remint_policy = remint
        #: desired.v1 §2.16's feed: the slice, its author, the target's state
        #: machine, the act running, and the parameters the documents it acted
        #: on in this run named (a refused one counts for none)
        self.fed = fed
        self.role: Any = None
        if fed is not None:
            from . import desired

            self.role = desired.TargetRole(fed[0])
        self._fed_running = False
        self._named: dict[str, set[str]] = {}
        #: what each seed found, and the acts the feed started (a test's)
        self.seeds: list[int] = []
        self.fed_acts: list[tuple[str, Any]] = []
        #: the feed's own failures, counted and kept, never fatal: a read or
        #: a seed that raised, and the act that raised (``fed_errors``)
        self.fed_errors = 0
        self.fed_last_error: str | None = None
        self.cv = threading.Condition(threading.RLock())
        self._stop = threading.Event()
        self._timer: threading.Thread | None = None
        self._entities: list[Any] = []
        #: every reply sent and every event published, for a test to read
        self.replies: list[bytes] = []
        self.events: list[dict[str, Any]] = []
        #: a test's loss: the next ``drop_replies`` value replies are made
        #: (and kept as first answers) but never sent; ``dropped`` holds them
        self.drop_replies = 0
        self.dropped: list[bytes] = []

    @property
    def instance(self) -> str:
        return self.owner.instance

    def _startup_values(self) -> dict[str, dict[str, Any]]:
        values: dict[str, dict[str, Any]] = {}
        for (g, p), prm in self.core.running.items():
            if prm.value is not UNKNOWN:
                values.setdefault(g, {})[p] = prm.value
        return values

    def start(self) -> None:
        self.device.boot(self._startup_values())
        self.owner.start()
        self._timer = threading.Thread(target=self._run_timer, daemon=True)
        self._timer.start()
        if self.desired_key is not None:
            import zenoh

            from . import live

            s = self.owner.session
            assert s is not None
            self._entities.append(s.declare_subscriber(self.desired_key, zenoh.handlers.Callback(
                lambda smp: self._on_desired(smp.payload.to_bytes()))))
            for r in live.get_state(s, self.desired_key).replies:
                if not r.deleted:
                    self._on_desired(r.payload)
        if self.fed is not None:
            import zenoh

            s = self.owner.session
            assert s is not None
            # desired.v1 §2.5: "declare its subscription to its slice's key
            # first, then make S4's GET of it"
            self._entities.append(s.declare_subscriber(self.fed[0], zenoh.handlers.Callback(self._fed_live)))
            threading.Thread(target=self._fed_poll, daemon=True).start()

    def close(self) -> None:
        self._stop.set()
        with self.cv:
            self.cv.notify_all()
        if self._timer is not None:
            self._timer.join(timeout=2.0)
        for e in self._entities:
            try:
                e.undeclare()
            except Exception:  # noqa: BLE001 - closing anyway
                pass
        self.owner.close()

    # -- after each step ------------------------------------------------------------------

    def _after(self, step: Step) -> None:
        """Events published, and the view and the status put when either
        changed (§2.3), the view first; one revision may name several such
        pairs (§2.3, 0.2: "An owner SHOULD put the view first"). The timer is told of a new due
        instant."""
        try:
            for ev in step.events:
                self.owner.occur(self.event_member, encode_json(ev))
                self.events.append(ev)
            now = (encode_json(self.core.view()), encode_json(self.core.status()))
            if now != self._last_put and self.owner.session is not None:
                self.owner.set_state(self.view_key, now[0])
                self.owner.set_state(self.status_key, now[1])
                self._last_put = now
        finally:
            # An act's end reaches the feed even when a put raises (a
            # session closing under it): the role is never left running.
            self._fed_check()
            self.cv.notify_all()

    def _send(self, call: Any, reply: Any) -> None:
        from .owner import MemberRefused

        if isinstance(reply, Refusal):
            call.refuse(reply.code, reply.message, **reply.envelope_kw())
            return
        data = encode_json(reply)
        if self.drop_replies > 0:
            # Lost on its way: the call counts as answered, so O3's
            # "internal" for a call left without its answer is not sent.
            self.drop_replies -= 1
            self.dropped.append(data)
            call.values += 1
            return
        try:
            call.reply(data, encoding="application/json")
        except MemberRefused:
            call.refuse("internal", "the reply has no member to go on")
            return
        self.replies.append(data)

    @staticmethod
    def _decode(call: Any) -> tuple[Any, Refusal | None]:
        from . import jcs

        try:
            return jcs.loads(call.payload), None
        except jcs.JsonError:
            return None, Refusal("invalid_request", "", "the request does not decode as JSON (core §5.1)")

    def _on_set(self, call: Any) -> None:
        group = (call.bound.get("group") or [None])[0]
        if group is None:
            call.refuse("internal", "no group bound")
            return
        call.name(group=group)
        request, bad = self._decode(call)
        with self.cv:
            step = Step(reply=bad) if bad is not None else self.core.set(group, request, time.time_ns(),
                                                                          call.metadata())
            if step.reply is not None:
                self._send(call, step.reply)
            self._after(step)

    def _on_control(self, op: str):
        def handle(call: Any) -> None:
            request, bad = self._decode(call)
            with self.cv:
                step = Step(reply=bad) if bad is not None else self.core.control(op, request, time.time_ns())
                if step.reply is not None:
                    self._send(call, step.reply)
                self._after(step)
        return handle

    def _on_persist(self, call: Any) -> None:
        request, bad = self._decode(call)
        with self.cv:
            step = Step(reply=bad) if bad is not None else self.core.persist(request, time.time_ns())
            if step.reply is not None:
                self._send(call, step.reply)
            self._after(step)

    def _on_desired(self, payload: bytes) -> None:
        from . import jcs

        try:
            doc = jcs.loads(payload)
        except jcs.JsonError:
            return
        if not isinstance(doc, dict) or not is_integer(doc.get("revision")) or not isinstance(doc.get("values"), dict):
            return
        with self.cv:
            current = self.core.desired_doc
            if current is not None and current.get("revision") == doc["revision"]:
                return  # §2.18: "It tries a failed document again when its revision changes."
            self._after(self.core.desire(doc, time.time_ns()))

    # -- desired.v1 §2.16: fed through desired.v1 ----------------------------------------

    def _fed_live(self, sample: Any) -> None:
        import zenoh

        key = str(sample.key_expr)
        if "*" in key or "$" in key:
            return  # desired.v1 §2.5, core R6
        ts = sample.timestamp
        self._fed_receive(sample.kind == zenoh.SampleKind.DELETE, None if ts is None else str(ts),
                          sample.payload.to_bytes())

    def _fed_receive(self, deleted: bool, stamp: str | None, payload: bytes) -> None:
        from . import desired
        from .desired_live import _sample

        with self.cv:
            if self._stop.is_set():
                return
            word = self.role.receive(_sample(desired.OWNER, deleted, stamp, payload))
            if word in (desired.ACT, desired.REVERT):
                self._fed_act()

    def _fed_seed(self) -> int:
        """S4's GET of the slice (desired.v1 §2.5), each reply judged in the
        order received. Returns how many replies it drew."""
        from . import live

        s = self.owner.session
        if s is None or self._stop.is_set():
            return 0
        st = live.get_state(s, self.fed[0])
        for r in st.replies:
            self._fed_receive(r.deleted, r.stamp, r.payload)
        self.seeds.append(len(st.replies))
        return len(st.replies)

    def _fed_failed(self, what: str, e: BaseException) -> None:
        """The feed's own failure: counted and kept, never fatal (review-bot
        on PR #963)."""
        self.fed_errors += 1
        self.fed_last_error = f"{what}: {e!r}"

    def _fed_seed_retry(self, tries: int = 5) -> int:
        """A seed made again, up to ``tries`` times, 100 ms more apart each
        time, while it draws nothing or raises (desired.v1 §2.5, "A seed that
        draws nothing at an appearance MAY be made again"; the reference's
        four more). A seed that raises is counted, never fatal. Ends at once
        when the owner closes. Returns the replies the last seed drew."""
        got = 0
        for n in range(tries):
            if self._stop.is_set():
                return got
            try:
                got = self._fed_seed()
            except Exception as e:  # noqa: BLE001 - a GET that raises is retried, and counted
                self._fed_failed("seed", e)
                got = 0
            if got or self._stop.wait(0.1 * (n + 1)):
                return got
        return got

    def _fed_poll(self) -> None:
        """desired.v1 §2.5 "When it seeds": at its start, and each time its
        author's instance token appears, read on a period (500 ms, the
        reference's) on the owner's session, which holds no liveliness
        subscriber. A seed that draws nothing at an appearance is made again
        up to four times, 100 ms more apart each time (the reference's). A
        read or a seed that raises is counted (``fed_errors``) and the loop
        goes on; it ends when the owner closes."""
        from . import live

        self._fed_seed_retry(1)
        seen: set[str] | None = None
        while not self._stop.wait(0.5):
            s = self.owner.session
            if s is None:
                return
            try:
                p = live.list_presence(s, f"zk2/{self.fed[1]}/@zk/instance/*", 1.0)
            except Exception as e:  # noqa: BLE001 - one read that raises ends nothing
                if self._stop.is_set():
                    return
                self._fed_failed("presence", e)
                continue
            if p.kind != live.COMPLETE and not p.instances:
                continue
            now = {i["instance"] for i in p.instances}
            if seen is not None and now - seen:
                self._fed_seed_retry()
            seen = now if p.kind == live.COMPLETE else (seen | now if seen is not None else now)

    def _fed_act(self) -> None:
        """Start the act on the role's current version (under ``cv``): a
        document through :meth:`Core.desire`, a delete through
        :meth:`Core.revert`. A payload that does not read as a
        ``DesiredConfig`` is refused ``invalid_request`` and moves nothing."""
        from . import desired, jcs

        v = self.role.current
        assert v is not None
        self._fed_running = True
        if v.delete:
            values: dict[str, dict[str, Any]] = {}
            for g, ps in sorted(self._named.items()):
                for p in sorted(ps):
                    start, _ = self.core.startup_value(g, p)
                    if start is not UNKNOWN:
                        values.setdefault(g, {})[p] = start
            self.fed_acts.append(("revert", values))
            self._fed_step(lambda: self.core.revert(values, time.time_ns()))
            return
        doc = None
        if v.payload is not None and v.payload.value is not None:
            try:
                doc = jcs.loads(desired.canonical_json(v.payload.value))
            except Exception:  # noqa: BLE001 - read as not a document below
                doc = None
        if not isinstance(doc, dict) or not is_integer(doc.get("revision")) or not isinstance(doc.get("values"), dict):
            self.fed_acts.append(("invalid", None))
            self._fed_running = False
            word = self.role.finish(desired.Finish(desired.REFUSED, {"code": "invalid_request"}))
            if word in (desired.ACT, desired.REVERT):
                self._fed_act()
            return
        self.fed_acts.append(("desire", doc))
        self.core.desired_end = None
        self._fed_step(lambda: self.core.desire(doc, time.time_ns()))

    def _fed_step(self, act: Callable[[], Step]) -> None:
        """Run the core's act, then :meth:`_after`, which hands its end to
        :meth:`_fed_check`. An act that raises has an end the owner cannot
        tell: desired.v1 §2.7, "``failed``, code ``apply``", counted, and the
        role is free for the next version."""
        try:
            step = act()
        except Exception as e:  # noqa: BLE001 - the act's end is unknown: failed, apply
            self._fed_failed("act", e)
            self.core._desired_waiting = self.core._reverting = False
            self.core.desired_end = ("failed", {"code": "apply", "detail": f"the act was lost: {e!r}"})
            step = Step()
        try:
            self._after(step)
        except Exception as e:  # noqa: BLE001 - a put that raised; the act's end was handed on (finally)
            self._fed_failed("put", e)

    def _fed_check(self) -> None:
        """After every step (under ``cv``): the running act ended when the
        core says how; the role is told (§2.7 "Two phases"), and acts on, or
        adopts, the version held waiting."""
        from . import desired

        if not self._fed_running or self.core._desired_waiting or self.core.desired_end is None:
            return
        outcome, refusal = self.core.desired_end
        self.core.desired_end = None
        self._fed_running = False
        v = self.role.current
        if v is not None and not v.delete and outcome in ("converged", "failed") and self.fed_acts:
            # §2.16: "each parameter named by a document it acted on in this
            # run"; "A document it refused counts for none".
            kind, doc = self.fed_acts[-1]
            if kind == "desire" and isinstance(doc, dict):
                for g, ps in doc["values"].items():
                    if isinstance(ps, dict):
                        self._named.setdefault(g, set()).update(ps)
        if v is not None and v.delete and outcome == "reverted":
            self._named = {}
        how = {"converged": desired.DONE, "reverted": desired.DONE, "refused": desired.REFUSED,
               "failed": desired.FAILED}[outcome]
        word = self.role.finish(desired.Finish(how, refusal))
        if word in (desired.ACT, desired.REVERT):
            self._fed_act()

    def remint(self) -> str:
        """Core §1.5's re-mint while running. ``config.v1`` §2.12: "A re-mint
        is a restart": the new instance starts from its startup values, the
        revision from 0, nothing pending, its device returned to them first.
        Fed through ``desired.v1``, §2.16 (text 0.10): it "begins a new run,
        so §2.7's restart holds of it: it MUST act on the first version it
        reads, the one it acted on before the re-mint included". zk2py's new
        run reads its slice as at a start (§2.5 "When it seeds"). The new
        instance's view and status are put before its tokens (§2.3), then the
        tokens make before the old ones break (core §8.1). With
        ``remint="refuse"`` it raises :class:`zk2py.owner.OwnerRefused`,
        nothing moved, as the reference does. Returns the new instance id."""
        import secrets

        from .owner import OwnerRefused

        if self.remint_policy == "refuse":
            raise OwnerRefused("config.v1 §2.12: a re-mint is a restart, and this owner refuses one while it runs")
        with self.cv:
            new = secrets.token_hex(8)
            self.core.restart(new)
            self.device.boot(self._startup_values())
            if self.role is not None:
                self.role.restart()
                self._fed_running, self._named = False, {}
            now = (encode_json(self.core.view()), encode_json(self.core.status()))
            self.owner.set_state(self.view_key, now[0])
            self.owner.set_state(self.status_key, now[1])
            self._last_put = now
            self.owner.new_epoch(new)
            self.cv.notify_all()
        if self.fed is not None:
            # A seed that raises, or a close racing the re-mint, never ends
            # the new run silently: counted, made again, and ended by close.
            threading.Thread(target=self._fed_seed_retry, daemon=True).start()
        return new

    def _run_timer(self) -> None:
        with self.cv:
            while not self._stop.is_set():
                nd = self.core.next_due()
                now = time.time_ns()
                if nd is None or self.core.in_flight is not None:
                    self.cv.wait(0.5)
                    continue
                if nd > now:
                    self.cv.wait(min((nd - now) / NS, 0.5))
                    continue
                self._after(self.core.due(now))


# -- §5 what a tool may conclude ---------------------------------------------------------

NOT_ASKED, UNOBSERVABLE = "not_asked", "unobservable"
HORIZON = {"freshness.ttl_s": 60}


@dataclass
class Reading:
    """One reading of a service for §5: the premise (presence and the
    descriptor, or the deployment's word across a face), the view and the
    status with their freshness (``freshness.v1`` at 60 s), and the change
    events heard."""

    premise: str                         # "ok", "not_asked" or "unobservable"
    why: str = ""
    view: dict[str, Any] | None = None
    status: dict[str, Any] | None = None
    view_fresh: bool = False
    status_fresh: bool = False
    events: list[dict[str, Any]] = field(default_factory=list)
    #: observer §4.1 (0.11): the keys the subscriber's recording's bound
    #: forgot, and its note, reported with what it holds (round 92)
    forgotten: int = 0
    bound: str = ""

    def fresh_doc(self) -> dict[str, Any] | None:
        """A fresh view, else a fresh status: what the questions that a
        status can answer read."""
        if self.view_fresh and self.view is not None:
            return self.view
        if self.status_fresh and self.status is not None:
            return self.status
        return None


def read(session: Any, address: str, *, subscriber: Any = None, presence: bool = True,
         lists: bool | None = None, trust: Any = None, timeout: float = 1.0) -> Reading:
    """A reading of ``address`` (``<system>/<service>``) for §5. With
    ``presence``, the premise is presence and the descriptor (observer §3):
    absent or not listing ``config.v1`` is not asked, incomplete or unread
    unobservable. Without (across a face, or a reader that does not read
    presence), ``lists`` is the deployment's word for the descriptor
    (§2.19). The view and the status come from a GET (core S4) and from
    ``subscriber`` (a :class:`zk2py.freshness.Subscriber`), each judged by
    ``freshness.v1`` at 60 s; the events from ``subscriber``."""
    from . import freshness as fr
    from . import live
    from . import seed as sd

    if presence:
        pres = live.list_presence(session, f"zk2/{address}/@zk/**")
        if not pres.instances:
            return Reading(NOT_ASKED if pres.complete else UNOBSERVABLE,
                           "presence shows the service absent" if pres.complete
                           else "the presence read was possibly incomplete")
        inst = pres.instances[0]["instance"]
        d = live.get_descriptor(session, f"zk2/{address}/@zk/instance/{inst}", timeout=timeout)
        if len(d) != 1 or not d[0].ok:
            return Reading(UNOBSERVABLE, "its descriptor is unread")
        doc = json.loads(d[0].payload)
        if not any(e.get("iface") == IFACE for e in doc.get("interfaces") or []):
            return Reading(NOT_ASKED, "its descriptor does not list config.v1")
    elif lists is False:
        return Reading(NOT_ASKED, "the deployment says it does not implement config.v1")
    trust = trust or fr.ClockTrust(word=True)
    out = Reading("ok")
    if subscriber is not None:
        out.forgotten, out.bound = subscriber.forgotten, subscriber.note()
    for which in ("view", "status"):
        key = f"zk2/{address}/{IFACE}/state/{which}"
        observations: list[Any] = []
        # The document as a late joiner holds it: the subscription's samples
        # and the GET's reply merged by core §4.2, the newer by the owner's
        # stamp, across two ids too (observer §4.1, 0.10; round 91). Round
        # 90 took the subscription's last put by arrival against the GET's
        # by time alone.
        arrivals: list[tuple[Any, bool, bytes]] = []
        if subscriber is not None:
            observations.append(subscriber.observation(key))
            arrivals += [(sd.Stamp.of(x.stamp), x.kind == "delete", x.payload) for x in subscriber.of(key)]
        g = fr.get_reading(session, key, timeout=timeout)
        observations.append(g.observation(trust))
        if g.reply in ("put", "delete"):
            arrivals.append((sd.Stamp.of(g.stamp), g.reply == "delete", g.payload))
        held = sd.merged(arrivals)
        latest = None if held is None or held[1] else held[2]
        fresh = fr.judge("state", HORIZON, observations).verdict == fr.FRESH
        docv = json.loads(latest) if latest is not None else None
        setattr(out, which, docv)
        setattr(out, f"{which}_fresh", fresh and docv is not None)
    if subscriber is not None:
        prefix = f"zk2/{address}/{IFACE}/events/change/"
        with subscriber.lock:
            keys = [k for k in subscriber.deliveries if k.startswith(prefix)]
        for k in sorted(keys):
            for x in subscriber.of(k):
                if x.kind == "put":
                    out.events.append(json.loads(x.payload))
    return out


def _premise(r: Reading) -> tuple[str, str] | None:
    return None if r.premise == "ok" else (r.premise, r.why)


def answer_pending(r: Reading) -> tuple[str, Any]:
    """§5 "Is a change pending?"."""
    p = _premise(r)
    if p:
        return p
    doc = r.fresh_doc()
    if doc is None:
        return UNOBSERVABLE, "neither the view nor the status is fresh"
    return ("yes", doc["pending"]) if "pending" in doc else ("no", None)


def answer_landed(r: Reading, token: str | None, instance: str | None) -> tuple[str, str]:
    """§5 "Did my change land?", for a change named by its token, as the
    instance ``instance`` answered the ``set`` that started it."""
    p = _premise(r)
    if p:
        return p
    if token is None:
        return UNOBSERVABLE, "the set drew no reply, so no token is known (core O5)"
    for ev in r.events:
        if ev.get("token") == token:
            return ("yes", ev["outcome"]) if ev["outcome"] in ("applied", "confirmed") else ("no", ev["outcome"])
    doc = r.fresh_doc()
    if doc is None:
        return UNOBSERVABLE, "no fresh view or status"
    if instance is not None and doc.get("instance") != instance:
        return "no", "the instance that held it is gone: the restart rolled it back (§2.12)"
    if doc.get("last_change", {}).get("token") == token:
        return "yes", "its token is the last change"
    if doc.get("pending", {}).get("token") == token:
        return "not_yet", "its token is still pending"
    return UNOBSERVABLE, "the last change is another token, and no event of this one was read"


def answer_undo_clean(r: Reading, token: str) -> tuple[str, str]:
    """§5 "Was its undo clean?", for a change that did not land."""
    p = _premise(r)
    if p:
        return p
    for ev in r.events:
        if ev.get("token") == token:
            if ev["outcome"] == "rolled-back":
                return "yes", "rolled-back"
            if ev["outcome"] == "partial":
                return "no", "partial: the read-back is the truth"
    return UNOBSERVABLE, "no event of it read"


def answer_writable(r: Reading, group: str) -> tuple[str, str]:
    """§5 "Is this group writable?": the view alone carries the class and
    ``writable``."""
    p = _premise(r)
    if p:
        return p
    if not r.view_fresh or r.view is None:
        return UNOBSERVABLE, "no fresh view: the status carries no class and no writable"
    g = find_group(r.view["groups"], group)
    if g is None:
        return NOT_ASKED, "the schema declares no such group"
    if g["writable"] and g["class"] in ("hot", "reach"):
        return "yes", g["class"]
    return "no", "outside the allowlist" if not g["writable"] else "contract"


def answer_survives(r: Reading, group: str, param: str) -> tuple[str, Any]:
    """§5 "Will this value survive a restart?"."""
    p = _premise(r)
    if p:
        return p
    if not r.view_fresh or r.view is None:
        return UNOBSERVABLE, "no fresh view"
    g = find_group(r.view["groups"], group)
    prm = None if g is None else find_param(g, param)
    if prm is None or prm.get("sensitive") or "value" not in prm:
        return UNOBSERVABLE, "the value is unknown, or sensitive"
    if "startup" in prm:
        return "no", {"startup": prm["startup"]}
    return "yes", None


def answer_converged(r: Reading, revision: int) -> tuple[str, Any]:
    """§5 "Has it converged to its desired document?"."""
    p = _premise(r)
    if p:
        return p
    doc = r.fresh_doc()
    if doc is None:
        return UNOBSERVABLE, "no fresh view or status"
    d = doc.get("desired")
    if d is None:
        return NOT_ASKED, "the owner converges no desired state"
    if d["observed_revision"] < revision:
        return "not_yet", d
    if d["outcome"] == "converged":
        return "yes", d
    return "no", d

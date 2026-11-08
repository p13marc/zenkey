"""A minimal zk2 owner on zenoh-python (core.md §2.1, §3.3, §4, §5, §6, §8).

**What it exposes (§8.2 "Exposed", 0.7).** Every resource of its contracts,
except a capability-gated optional one whose capability it does not hold
(implied absent, §3.3), and those the caller lists ``unavailable`` or
withholds:
- **state**: the interface's state queryables (non-``complete``, over
  ``state/**`` and ``@state/**``, S2) and, for a parameterless state, a
  publisher. A parameterless ``raw`` state holds the bytes ``ok`` from the
  start, put before the tokens (§8.2 "State values"); any other state holds
  no value until ``set_state``. A template with no member yet is exposed by
  its template.
- **streams**: a publisher when parameterless, none for a template (each
  member's would come as it appears); **events**: nothing to declare.
- **operations**: a ``complete`` queryable on the concrete key, or over the
  template, or, for the members the caller lists, one per member (O1,
  §5.1). Before any handler runs, a call on a key that is not concrete is
  ``fanout_forbidden`` unless the operation allows fan-out (O2), and, over a
  template, a key whose concrete parameter chunk is not a canonical slug
  names no member and is ``invalid_request``, fan-out or not (§5.1, 0.8).
  Then the operation's handler runs with an :class:`OpCall`: the caller's,
  or the default one, which
  - over a template names no member, since this owner has none: a call
    binding every parameter is ``not_found`` (O-4), and a fan-out
    ``internal`` ("One that names no member … refuses the call");
  - for a ``raw`` request and response echoes the request's bytes;
  - for a JSON Schema request that does not decode as a JSON object answers
    ``invalid_request`` (the decode is the check, O-13), else a JSON value;
  - refuses any other request with ``app``, without a detail since this
    owner declares none (§5.2).

  A handler names at most one member per call, whatever ``replies`` is, and
  one the call selects; anything else is refused to it (§5.1, 0.8). Every
  value goes on the operation's own concrete key, or the named member's;
  every failure is a ``reply_err``, and a call left without its answer is
  ``internal`` (O3). An optional operation not exposed is answered
  ``unavailable`` with its cause (O3).

**A call's forms (core 0.94, #985 C4c).** Every value reply is §7.5's
value form and every refusal its error form, under ``zenoh/bytes``, each
with the owner's as-of mark (:meth:`Owner.mark`): "an owner MUST write each
value reply in §7.5's value form … with its as-of mark", and "MUST answer
a failed call in §7.5's error form". Each CH carries its type's tag
(:mod:`zk2py.calls`). A request is read by §7.5's parse, and its record as
the ``request`` type, a packed one under CH's tag; one either refuses is
``invalid_request``, after the three refusals before any handler (§5.1,
"The request"). Since core 0.99 (#985 C4d) the bare wire is gone: "Under
another ``Encoding`` a query carries no request form", and is refused
``invalid_request`` as a request the parse refuses is, whatever its bytes,
no handler run. An answer sent again is its first bytes, mark included
(:meth:`OpCall.resend`).

**Bring-up (§8.2).** Step 2 runs first (the order of steps 1 and 2 is free),
so a refusal declares nothing: a required resource not exposed, an optional
one neither exposed nor absent as the descriptor says, and a required role
bound to nothing (§3.2). Then step 1 (resources, and the state values),
step 3 (the descriptor's queryable and its first put, then the bundles'
queryables) and step 4 (the instance token, then the interface tokens).

**Re-minting (§8.1).** :meth:`Owner.new_epoch` starts a new instance id
while running, make-before-break: the new descriptor's queryable and first
put, then its tokens, and only then the old ones undeclared. Each instance's
descriptor queryable answers with its own record, so the overlap reads true.

**QoS (§2.4).** Every publisher, an event's included, carries its
resource's four fields (:func:`qos_of`), ``reliability`` through zenoh's
unstable setting, the one opt-in of §0 this owner takes (round 89, F-369).

The session is a zenoh *router* listening on a loopback port, or, with
``connect``, a client of a router: the second is what ``state.md §1`` and
``presence.md §1–§2`` need.

**Runs and seqs (core 0.73, #976).** Every put on a ``stream``, ``state``,
``@state`` or ``events`` key carries core §7.5's envelope, its sample's
``Encoding`` ``zenoh/bytes``, and every delete of a state member carries a
delete envelope as its attachment (§4.3). An ``@stream`` sample keeps its
wire, and so do operations, the descriptor and bundles.
- **The run.** Each instance writes under a run of its own, stated in its
  descriptor's ``run`` (§3.3). With ``run_record`` it is a 16-bit counter
  in that file, written durably before the first sample under the run (0.74,
  F-416); with none, with a record it cannot use, with the counter spent at
  0xFFFF, or for a candidate of a governed interface (``redundancy.v1``), a
  32-bit random draw, drawn again while it equals a run this owner used,
  which meets no reuse, the chance of a repeat accepted (0.74, F-417). A
  re-mint takes a new run; a takeover does not.
- **The seq.** One count per run over every state mutation and event
  occurrence, from 0; before it would pass 16,777,215 the owner takes a new
  run, puts its descriptor stating it, and writes on from 0. Each stream
  key's seq is its own, from 0 at its first sample in a run, wrapping
  modulo 2^24, never restarted within the run, a member's publisher
  declared again included (:meth:`Owner.set_next_seq` sets the next state
  seq, for ``state.md`` §7 step 3).
- **The Zenoh timestamp** is set only on §4.3's table (:func:`keeps_timestamp`):
  a state declaring ``freshness.ttl_s`` above 0, a ``desired.v1`` desired
  resource, a ``telemetry.v1`` metric state, a stream declaring a
  ``timing.v1`` lifespan, a ``health.v1`` fault where its owner sets one,
  and every put of a ``redundancy.v1`` candidate, whose session runs its
  HLC (§8.2's exception). Every other session runs with timestamping
  disabled (§8.2, "Session posture"), unless a test rig asks for ``hlc``:
  a stamp that session adds is its posture, the SHOULD's, never the MUST
  NOT's (0.74, F-414). Each descriptor put carries a Zenoh timestamp the
  owner sets, from :meth:`Owner.mint` (§3.3, 0.74, F-415).
"""

from __future__ import annotations

import json
import secrets
import socket
import sys
import threading
import time
from dataclasses import dataclass
from typing import Any, Callable

import zenoh

from . import bundle, freshness, freshness_v2, snapshot, templates, zk2c
from . import heartbeat as hbeat
from .contract import Contract
from .lexical import parse_interface_id
from .slug import slug

#: Core R1 (0.20): "a provider whose system position is self.system".
SELF_SYSTEM = "self.system"


def resolve_providers(providers: list[str], own_system: str | None) -> list[str]:
    """Core §3.2 R1 (0.20): "A runtime MUST resolve a provider whose system
    position is self.system into the service's own system, once, when the
    service starts … self.system/<service> becomes <system>/<service>, and
    self.system/* becomes <system>/*." Only the system position is a
    keyword. A tool, which has no system of its own (``own_system`` None),
    "MUST refuse a self.system provider" (ValueError)."""
    out = []
    for p in providers:
        system, sep, rest = p.partition("/")
        if system == SELF_SYSTEM and sep:
            if own_system is None:
                raise ValueError(f"{p}: a tool has no system of its own (core R1, 0.20)")
            out.append(f"{own_system}/{rest}")
        else:
            out.append(p)
    return out


def profile_order(profiles: set[str]) -> list[str]:
    """Core §3.3 (0.20): `profiles` "sorted as §9.5 sorts a contract's uses:
    by name as a string, then by major as a number". So views.v2 comes
    before views.v10, and a.v1 before a.b.v1."""
    def key(p: str):
        pid = parse_interface_id(p)
        return (0, pid.name, pid.major, "") if pid is not None else (1, "", 0, p)

    return sorted(profiles, key=key)

#: core.md §2.4 / Appendix D names → zenoh's QoS enums.
_CONGESTION = {"block": "BLOCK", "drop": "DROP"}
_PRIORITY = {"real_time": "REAL_TIME", "interactive_high": "INTERACTIVE_HIGH",
             "interactive_low": "INTERACTIVE_LOW", "data_high": "DATA_HIGH", "data": "DATA",
             "data_low": "DATA_LOW", "background": "BACKGROUND"}
#: core.md S3: the tombstone window, unless the deployment configures another.
TOMBSTONE_WINDOW_S = 60.0
#: core §4.2 (0.100), "Pages": "The page size is 1,024 bytes, unless the
#: deployment configures the owner with another."
PAGE_SIZE = 1024
#: core §2.4's ``reliability`` → zenoh's.
_RELIABILITY = {"reliable": "RELIABLE", "best_effort": "BEST_EFFORT"}


def qos_of(r: dict[str, Any]) -> dict[str, Any]:
    """Core §2.4: "An owner MUST publish with it", the resource's resolved
    QoS, every field: ``reliability``, ``congestion``, ``priority`` and
    ``express``, as zenoh's publisher declaration takes them.

    ``reliability`` is the one field zenoh 1.10.1 sets only through unstable
    API: zenoh-python marks ``Reliability`` unstable, and a publisher
    declared without it is reliable, so a stream's default ``best_effort``
    would go out reliable. zk2py opts into it here, the one unstable
    publisher setting it uses (round 89; §2.4's "Zenoh's stable QoS" and
    §0's opt-in are a finding)."""
    return {"congestion_control": getattr(zenoh.CongestionControl, _CONGESTION[r["congestion"]]),
            "priority": getattr(zenoh.Priority, _PRIORITY[r["priority"]]),
            "express": r["express"],
            "reliability": getattr(zenoh.Reliability, _RELIABILITY[r["reliability"]])}


class OwnerRefused(RuntimeError):
    """The owner must not start (§3.2, §8.2 step 2)."""


class StandbyRefused(OwnerRefused):
    """A write on a governed interface's key while the owner stands by on
    it: "a standby puts and deletes none of its keys" (redundancy.v1 §2.4).
    Its own type, so a caller tells it from a write that failed."""


class BudgetExceeded(RuntimeError):
    """Core §2.7 (0.24): "An owner MUST NOT hold more live members of a
    templated stream, state or event than its bound", and MUST NOT publish
    an event's member beyond its rate."""


class ClockAhead(RuntimeError):
    """Core §4.3 "Ahead" (0.88): a detection shows the owner's clock beyond
    its router's delta, so it "MUST stop the state writes that carry a Zenoh
    timestamp, re-puts included". A write that carries none goes on."""


def free_loopback_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


@dataclass
class _Held:
    payload: bytes | None        # None: deleted (a tombstone); the record
    encoding: str                # the record's Encoding (§7.2)
    stamp: zenoh.Timestamp | None  # the Zenoh timestamp, on §4.3's table only
    at: float                    # monotonic time of the mutation
    #: core §4.3 (0.73): the mutation's in-band stamp, and the bytes it put
    #: (its envelope, or a delete's attachment), which S2's reply carries
    inband: zk2c.InBand | None = None
    wire: bytes | None = None
    #: freshness.v2 (core 0.82 §4.3's table, row 2): the member's last
    #: confirmation, "the later of its last mutation's time and the last
    #: heartbeat's that vouched for it", which its replies carry (S2) and
    #: no put does; None outside such a state
    confirm: zenoh.Timestamp | None = None


def keeps_timestamp(c: Contract, r: dict[str, Any]) -> bool:
    """Core §4.3's table (0.73): whether resource ``r`` of ``c`` carries a
    Zenoh timestamp its owner sets beside the in-band stamp. A
    ``redundancy.v1`` candidate's every put is the table's too, by its
    session's HLC (:attr:`Owner.candidate`); a ``health.v1`` fault carries
    one where its owner sets one (its §2.5), which :meth:`Owner.publish`
    leaves to the caller."""
    from . import timing

    ann = r["annotations"]
    if r["kind"] == "state":
        # Core 0.82's table, row 1 split in two: a horizon's state keeps its
        # Zenoh timestamp on each mutation only where its contract does not
        # use freshness.v2; one that does carries its last confirmation on its
        # replies alone (:attr:`_Held.confirm`).
        if freshness.horizon("state", ann).horizon == "within" and not freshness_v2.uses_v2(c.canonical):
            return True
        if "desired.target_param" in ann:
            return True
        # Core 0.90 (F-462's answer): "A state that is a metric resource of
        # telemetry.v1, or of telemetry.v2 and no frame resource". A state is
        # never a frame resource (core §7.7: a plain stream).
        if "telemetry.kind" in ann and ({"telemetry.v1", "telemetry.v2"} & set(c.canonical["uses"])):
            return True
    if r["kind"] == "stream":
        d = timing.declaration("stream", ann)
        if d.declaration == timing.DECLARED and d.lifespan_ms is not None:
            return True
    return False


def per_key(c: Contract, r: dict[str, Any]) -> bool:
    """Core §4.2 (0.100), "A per-key resource keeps S2's reply per key …
    These are the states §4.3's table lists": a state that declares
    ``freshness.ttl_s`` above 0 of a contract that does not use
    ``freshness.v2``, a ``desired.v1`` desired resource, a ``telemetry.v1``
    or ``telemetry.v2`` metric state, and "every state of an interface whose
    contract uses ``redundancy.v1``" (:func:`keeps_timestamp` reads the
    first three)."""
    if r["token"] not in ("state", "@state"):
        return False
    return keeps_timestamp(c, r) or "redundancy.v1" in c.canonical["uses"]


@dataclass(frozen=True)
class Collection:
    """Core §4.2 (0.100): "one ``state`` or ``@state`` resource of an
    interface it exposes, with its members (§7.5)". ``index`` is "its
    resource's place, from 0, among the interface's resources of the reply
    key's kind token, in the canonical form's order" (§7.5); ``key`` its
    snapshot key, None for "A template whose first chunk is a parameter";
    ``horizon`` whether it is "a resource that declares ``freshness.ttl_s``
    above 0, of a contract that uses ``freshness.v2``"."""

    contract: Any
    resource: dict[str, Any]
    base: str            # zk2/<system>/<service>/<iface>/<token>
    index: int
    key: str | None
    tag: int
    horizon: bool

    def member(self, key: str) -> bool:
        """Whether ``key`` is a member key of this collection's template."""
        if not key.startswith(self.base + "/"):
            return False
        return templates.match(templates.parse_template(self.resource["template"]),
                               key[len(self.base) + 1:].split("/")) is not None


def snapshot_key(base: str, template: str) -> str | None:
    """Core §4.2: "its resource's key up to the template's first parameter
    … A template with no parameter is its own snapshot key", and "A template
    whose first chunk is a parameter has no snapshot key"."""
    lit: list[str] = []
    for chunk in template.split("/"):
        if chunk.startswith("{"):
            break
        lit.append(chunk)
    if not lit:
        return None
    return f"{base}/{'/'.join(lit)}"


#: core §4.3: the kinds an owner writes the envelope on (``@stream`` keeps
#: its wire).
ENVELOPED = ("stream", "state", "@state", "events")


def enveloped(key: str) -> bool:
    """Whether an owner writes ``key``'s samples in §7.5's envelope."""
    p = zk2c.data_key(key)
    return p is not None and p.get("form") == "data" and p.get("token") in ENVELOPED


def _template_key(template: str) -> str:
    """A template as a key expression: ``*`` per parameter, ``**`` for a rest."""
    return "/".join("**" if p.endswith("...}") else "*" if p.startswith("{") else p
                    for p in template.split("/"))


def member_key(base: str, r: dict[str, Any], values: dict[str, Any]) -> str:
    """A member's concrete key: each parameter's value slugged (§1.4), a
    rest parameter's list one chunk per value."""
    tpl = templates.parse_template(r["template"])
    if set(values) != set(tpl.param_names):
        raise MemberRefused(f"a member of {r['template']} names {tpl.param_names}, not {sorted(values)}")
    chunks: list[str] = []
    for seg in tpl.segments:
        if seg.kind == templates.LITERAL:
            chunks.append(seg.text)
            continue
        v = values[seg.text]
        vs = [v] if isinstance(v, str) else list(v)
        if seg.kind == templates.PARAM and len(vs) != 1 or not vs:
            raise MemberRefused(f"{seg.text}: one value per parameter, at least one per rest parameter")
        chunks.extend(slug(x) for x in vs)
    return f"{base}/{r['token']}/{'/'.join(chunks)}"


def replica_only(rows: list[tuple[dict[str, Any], str]]) -> bool:
    """Core §5.1 "Beside replicas": whether an instance's exposed resources
    of one interface (its plan's rows, :meth:`Owner._plan`) are all
    replicated operations, and there is at least one."""
    exposed = [r for r, status in rows if status == "exposed"]
    return bool(exposed) and all(r["kind"] == "operation" and r.get("serving") == "replicated" for r in exposed)


def _refuse(query: zenoh.Query, op: Any, mark: zk2c.InBand, code: str, message: str | None,
            cause: str | None = None) -> bytes:
    """A refusal before any handler runs (§5.1, "The order of refusals"):
    §7.5's error form under ``zenoh/bytes``, the owner's as-of mark, CH's tag
    the operation's ``error`` type's (core 0.94 §5.2). It carries no detail."""
    from . import calls

    wire = calls.error(op, mark, code, cause, message)
    query.reply_err(wire, encoding=calls.FORM_ENCODING)
    return wire


class MemberRefused(RuntimeError):
    """§5.1 (0.8): "naming a second member is refused to the handler", and
    so is naming one the call's key expression does not select, or sending
    a value before naming any."""


class OpCall:
    """One call, as an operation handler sees it (§5.1 "Answering").

    ``bound`` is what the call's key expression binds (each parameter's
    values, None at a wildcard). Over a template, the handler names the one
    member it answers for, :meth:`name`, and every value goes on that
    member's key; a concrete operation's key is named already."""

    def __init__(self, query: zenoh.Query, base: str, r: dict[str, Any], op: Any, mark: Any,
                 bound: dict[str, list[str] | None], key_expr: str):
        """``op``: the operation's :class:`zk2py.calls.Operation`; ``mark``:
        a function giving the owner's as-of mark when a reply is written
        (core §5.1: "The run is the one it writes under when it replies")."""
        from . import calls

        self.query, self.base, self.resource, self.op, self._mark = query, base, r, op, mark
        self.bound, self.key_expr = bound, key_expr
        #: the query's payload as it came, and its Encoding
        self.wire = b"" if query.payload is None else query.payload.to_bytes()
        enc = getattr(query, "encoding", None)
        self.encoding = calls.FORM_ENCODING if enc is None else str(enc)
        #: the request's record (``payload``), its CH's tag, and the token of
        #: the refusal, then invalid_request (§5.1, "The request"): core
        #: 0.99, "Under another Encoding a query carries no request form",
        #: ``encoding``, its bytes never parsed; else §7.5's parse's. A query
        #: with no payload at all carries no Encoding of its own, and is read
        #: as a form: it has no CH (``short``).
        self.payload, self.tag, self.request_refusal = b"", None, None
        if query.payload is not None and not calls.in_forms(self.encoding):
            self.request_refusal = "encoding"
        else:
            try:
                env = zk2c.parse(self.wire, "query")
            except zk2c.Refused as e:
                self.request_refusal = e.token
            else:
                self.payload, self.tag = env.record or b"", env.tag
        #: the request read as its type (:meth:`zk2py.calls.Operation.decode`),
        #: set before the handler runs where it reads
        self.request: Any = None
        #: the request's attachment, the call metadata's carrier (O7)
        self.attachment = None if query.attachment is None else query.attachment.to_bytes()
        self.member_key: str | None = None if r["params"] else f"{base}/{r['token']}/{r['template']}"
        self.values = self.summaries = self.envelopes = 0
        self.refused_to_handler: list[str] = []
        self.failed: str | None = None
        #: every answer this call sent, (``value``, ``summary`` or ``error``,
        #: its bytes), in order
        self.sent: list[tuple[str, bytes]] = []

    def name(self, **values: Any) -> str:
        """Name the member this call answers for; returns its key. A second
        member, or one the call does not select, raises
        :class:`MemberRefused` (§5.1, 0.8: "A template-wide server answers
        for one member per call, whatever replies is")."""
        try:
            if not self.resource["params"]:
                raise MemberRefused("a concrete operation has no member to name")
            key = member_key(self.base, self.resource, values)
            if self.member_key is not None and key != self.member_key:
                raise MemberRefused(f"one member per call: {self.member_key} is named already")
            if not zenoh.KeyExpr(self.key_expr).includes(zenoh.KeyExpr(key)):
                raise MemberRefused(f"the call's key expression does not select {key}")
        except MemberRefused as e:
            self.refused_to_handler.append(str(e))
            raise
        self.member_key = key
        return key

    def _value_wire(self, record: bytes, summary: bool = False) -> bytes:
        from . import calls

        return calls.value(self.op, self._mark(), bytes(record), summary=summary)

    def reply(self, payload: bytes, *, send: bool = True) -> bytes:
        """A value reply on the named member's key (O3): ``payload``, the
        record's bytes as the ``response`` type says, in §7.5's value form
        under ``zenoh/bytes`` with the owner's as-of mark (core 0.94 §5.1).
        Returns the bytes; ``send`` False makes them and counts the call
        answered without sending them (a test's lost reply, config.v1
        scenarios §10)."""
        from . import calls

        if self.member_key is None:
            self.refused_to_handler.append("a value before naming a member")
            raise MemberRefused("name the member this call answers for first")
        if self.resource["replies"] == "one" and (self.values or self.envelopes):
            raise RuntimeError("replies = \"one\": the call is answered already")
        if self.summaries:
            # O6: "each replier MUST end with exactly one summary reply".
            raise RuntimeError("the summary ended this replier: no value after it (O6)")
        wire = self._value_wire(payload)
        if send:
            self.query.reply(self.member_key, wire, encoding=calls.FORM_ENCODING)
        self.values += 1
        self.sent.append(("value", wire))
        return wire

    def reply_value(self, value: Any) -> bytes:
        """A value reply of ``value``, encoded as the ``response`` type says
        (:meth:`zk2py.calls.Operation.encode`: a packed value through §7.6's
        writer, a JSON Schema value as JSON or CBOR)."""
        return self.reply(self.op.encode("response", value))

    def summary(self, payload: bytes) -> bytes:
        """O6: the one summary reply, its attachment the bytes ``summary``:
        "each replier MUST end with exactly one summary reply", so a second
        is refused to the handler, as is one for an operation that declares
        no ``summary``, or whose ``replies`` is not ``many`` (E033). The value
        form, its tag the ``summary`` type's (core 0.94 §5.1)."""
        from . import calls

        if self.member_key is None:
            raise MemberRefused("name the member this call answers for first")
        if self.resource.get("summary") is None or self.resource["replies"] != "many":
            raise RuntimeError("this operation declares no summary (O6, E033)")
        if self.summaries:
            raise RuntimeError("one summary per replier: it is sent already (O6)")
        wire = self._value_wire(payload, summary=True)
        self.query.reply(self.member_key, wire, encoding=calls.FORM_ENCODING, attachment=b"summary")
        self.summaries += 1
        self.sent.append(("summary", wire))
        return wire

    def refuse(self, code: str, message: str | None, *, cause: str | None = None, detail: Any = None,
               send: bool = True) -> bytes:
        """A ``reply_err`` in §7.5's error form under ``zenoh/bytes``, with
        the owner's as-of mark and the ``error`` type's tag (core 0.94
        §5.2). ``detail`` is a value of the operation's ``error`` type,
        encoded as it says: a JSON Schema or packed value, a protobuf
        message's or a raw type's bytes. §5.2: "An operation that declares no
        error type has none to send, and an owner MUST NOT put a detail in
        its error", and "A detail its type cannot encode … MUST NOT be sent
        by an owner": each raises before anything is sent, so the handler
        has failed and its call ends ``internal``, as the reference sends.
        Returns the bytes; ``send`` False makes them without sending."""
        from . import calls

        if detail is not None and self.resource.get("error") is None:
            raise ValueError("this operation declares no error type: an error carries no detail (§5.2)")
        record = None if detail is None else self.op.encode("error", detail)
        wire = calls.error(self.op, self._mark(), code, cause, message, record)
        if send:
            self.query.reply_err(wire, encoding=calls.FORM_ENCODING)
        self.envelopes += 1
        self.sent.append(("error", wire))
        return wire

    def resend(self, kind: str, wire: bytes) -> None:
        """An answer this owner sent before, sent again: "The owner MUST then
        send that answer's bytes unchanged, its mark included" (core 0.94
        §5.1, "An answer sent again"). ``kind`` ``value`` replies on the
        call's member key, ``error`` as a ``reply_err``."""
        from . import calls

        if kind == "error":
            self.query.reply_err(wire, encoding=calls.FORM_ENCODING)
            self.envelopes += 1
        else:
            if self.member_key is None:
                raise MemberRefused("name the member this call answers for first")
            self.query.reply(self.member_key, wire, encoding=calls.FORM_ENCODING)
            self.values += 1
        self.sent.append((kind, wire))

    def metadata(self) -> dict[str, str]:
        """Core O7 (§5.1 "Call metadata"): the attachment read as call
        metadata "only when it is a JSON object whose actor and request_id,
        each optional, are strings where present"; anything else is no
        metadata, and never a reason to refuse."""
        try:
            doc = json.loads(self.attachment) if self.attachment else None
        except ValueError:
            return {}
        if not isinstance(doc, dict) or any(k in doc and not isinstance(doc[k], str)
                                            for k in ("actor", "request_id")):
            return {}
        return {k: doc[k] for k in ("actor", "request_id") if k in doc}

    def finish(self) -> None:
        """O3: "A handler that ends without replying is answered internal";
        with replies = "many", a declared summary is owed too, "after any
        values it sent". A handler that raised is internal unless it had
        answered a one-reply call. §5.1 (0.9) "Sending nothing needs no
        member": a ``many`` handler that sent nothing, named member or not,
        ends "as zero values then completion, when no summary is declared,
        and is answered internal when one is"."""
        r = self.resource
        if self.envelopes or self.summaries:
            # An envelope answers the call; a summary ends its replier (O6),
            # so a handler that raises after it owes nothing more: the call
            # has its values and its summary, and what it then attempted was
            # refused to it, never sent.
            return
        if r["replies"] == "one":
            owed = not self.values
        else:
            owed = (r.get("summary") is not None and not self.summaries) or self.failed is not None
        if owed:
            self.refuse("internal", self.failed or "the handler ended without its answer")


class Owner:
    def __init__(self, system: str, service: str, contracts: list[Contract], *, port: int | None = None,
                 connect: str | None = None, bindings: dict[str, list[str]] | None = None,
                 capabilities: set[str] | None = None, unavailable: dict[str, str] | None = None,
                 withhold: set[str] | None = None, handlers: dict[str, Any] | None = None,
                 members: dict[str, list[dict[str, Any]]] | None = None, hold_s: float = 0.0,
                 auth: tuple[str, str] | None = None, tokenless: set[str] | None = None,
                 router_connect: str | None = None, hostid: Any = None,
                 hostid_ephemeral: bool | None = None, meta_host: str | None = None,
                 state_zid: bool = True, heartbeat: str | None = None, delta_s: float = 0.5,
                 initial: dict[str, bytes] | None = None, hlc: bool | None = None,
                 cardinality: dict[str, dict[str, int]] | None = None, budget: bool = True,
                 tombstone_s: float = TOMBSTONE_WINDOW_S, before_tokens: Any = None,
                 params: dict[str, dict[str, str]] | None = None, standby: set[str] | None = None,
                 lease_ms: int | None = None, namespace: str | None = None,
                 interface_tokens_after_s: float | None = 0.0, run_record: Any = None,
                 first_hop: bool = False):
        """A router listening on ``port`` (a free loopback port by default),
        or, with ``connect``, a client of that router endpoint.
        - ``bindings``: a role's configured providers (R1); a role left out
          is unbound.
        - ``params``: a role's parameter bindings as configured (R2), listed
          in the descriptor for a bound role (R3); an unbound one lists
          ``"params": {}`` (core §3.2).
        - ``capabilities``: the capabilities held (§3.3).
        - ``unavailable``: ``<kind token>/<template>`` → cause, for optional
          resources not exposed and listed so.
        - ``withhold``: resources neither exposed nor listed, which step 2
          refuses (presence.md §2 step 5).
        - ``handlers``: ``<kind token>/<template>`` → a function of an
          :class:`OpCall`, the operation's application handler (the default
          is :meth:`_default_op`).
        - ``members``: ``<kind token>/<template>`` → the members (values by
          parameter, a list for a rest parameter) to serve with a queryable
          each, instead of one over the template.
        - ``hold_s``: how long each operation query stays open after its
          handler returns (operations.md §1).
        - ``auth``: a usrpwd (user, password), the principal it is bound to
          as a client (§11.3).
        - ``tokenless``: the interfaces of its tokenless set (§8.1, U22):
          ``"token": false`` in the descriptor, and no interface token.
          ``archive.v1`` is refused (§4.4, 0.16).
        - ``router_connect``: as a router (no ``connect``), also link to that
          router endpoint, so this owner's own session is a router of the
          deployment (§4.2, "A tool's S1 check").
        - ``hostid``: the process's :class:`zk2py.hostid.Runtime`. With
          ``system`` ``@hostid.v1`` (hostid.v1 §2.3) the system is minted by
          it before the session opens (§2.7), and the descriptor lists
          ``hostid.v1`` (§2.8, core §3.3, 0.19). ``hostid_ephemeral`` is the
          configuration's ``hostid.ephemeral``.
        - ``meta_host``: the descriptor's ``meta.host``; a minted service
          states the host name by default (hostid.v1 §2.13).
        - ``state_zid``: False leaves ``meta.zid`` out (hostid scenarios
          §6 step 3).
        - ``heartbeat``: a router-stamped key the owner subscribes to as its
          clock reference (core §4.3 "Ahead"), with ``delta_s`` the
          router's HLC delta. While its clock reads beyond the delta ahead
          of a heartbeat's stamp, it writes no state that carries a Zenoh
          timestamp (core §4.3, 0.88): :meth:`set_state` and
          :meth:`delete_state` raise :class:`ClockAhead` for a key on §4.3's
          table, or for any key on a session that stamps, and no re-put is
          made (freshness.v1 §2.10). Every other state write goes on, and a
          ``freshness.v2`` heartbeat goes on vouching, its time all ones
          (freshness.v2 §2.10, 0.4).

        - ``cardinality``: per interface, ``<kind token>/<template>`` -> the
          bound this instance states in its descriptor (§3.3), lowering the
          contract's (§2.7).
        - ``budget``: False lets a test owner break §2.7's rule on purpose:
          by default a put of a new live member beyond the bound, or an
          event occurrence beyond its rate, raises :class:`BudgetExceeded`.
        - ``hlc``: whether its session runs zenoh's timestamping. By default
          it does not (core §8.2, 0.73: "The owner's session SHOULD run
          with timestamping disabled"), but for a candidate of a governed
          interface, whose every put the candidate session's HLC stamps
          (``redundancy.v1`` §2.10, §8.2's exception). True is a test rig's
          (``state.md`` §1's second router, timing.v1's stamped steps).
          Without it, a put leaves with the stamp the owner gives it, so an
          offset ``clock`` reaches the router dated ahead (health.v1
          scenarios §4).
        - ``run_record``: a file holding its 16-bit run counter (core §4.3,
          0.73), the last run it issued; each start, and each re-mint,
          takes the next and writes it durably before any sample under it
          (0.74). None, a record it cannot read or write, or a counter spent
          at 0xFFFF, draws a 32-bit run, and leaves the record as it is.
        - ``initial``: state values held at start, by key relative to the
          address (``health.v1/state/status``), put with the others before
          the tokens (§8.2 "State values"; a MUST for health.v1's status,
          health.v1 §2.3).

        - ``before_tokens``: called with the owner after the state values
          held at start are put, before step 3: a restart's own puts and
          deletes (``alarms.v1`` §2.9: "before it answers a GET with it, and
          SHOULD before its new instance token").
        - ``lease_ms``: its session's lease, as its router reads it (zenoh's
          ``transport/link/tx/lease``): a crash behind a silent link is seen
          that long after (``redundancy.v1`` scenarios, "A crash").
        - ``namespace``: its session's zenoh namespace (core §1.6), which
          prefixes every key on the wire (core presence.md §10 step 7).
        - ``interface_tokens_after_s``: at start, how long it waits after its
          instance token before its interface tokens, "as an owner that
          pauses between the two steps would" (operations.md §8 step 6,
          core 0.59; observer §3.2's transient half, text 0.2): its
          descriptor lists the interfaces exposed meanwhile, and no token
          holds them. ``None`` never declares them, a test rig's owner whose
          descriptor claims an exposure no token covers (core §8.1), the
          step's control. :meth:`start` returns once they are declared, or,
          with ``None``, once the instance token is.
        - ``standby``: the interfaces it stands by on at start (core §6,
          0.39; ``redundancy.v1`` §2.4): validated by step 2 as if exposed,
          none of their resources declared, their entries marked
          ``"standby": true``, none of their tokens held. :meth:`take_over`
          serves one in §8.2's order, :meth:`step_down` undoes it in the
          reverse. A standby on a tokenless interface is refused (core
          D012), and so is an interface whose ``redundancy.v1`` declaration
          is invalid, a governed one in the tokenless set included
          (``redundancy.v1`` §2.2, §2.14; core §8.2 step 2, 0.39).

        Every state member whose resource has a ``freshness.ttl_s`` above 0,
        a templated one's members included from their first put, is re-put,
        unchanged under a fresh stamp, once ttl/2 has passed since its last
        put, until its writer closes (:meth:`close_writer`) or the owner
        closes (freshness.v1 §2.4)."""
        for c in contracts:
            if not c.valid or c.canonical is None:
                raise ValueError(f"{c.path}: not a valid contract: {c.codes}")
            # core 0.87 (#981 K3): a frame resource's samples are frames,
            # written by record_frame (core §7.7 "The owner's frames"); its
            # layout must resolve, and no mutation is ever put on its key.
            from . import frame as _frame

            for r in c.canonical["resources"]:
                if r.get("frame") is not None and _frame.layout_of(c, r) is None:
                    raise OwnerRefused(f"{c.interface}: frame resource {r['template']}: its record type does not "
                                       "resolve")
        #: freshness.v2 §2.4 (core 0.82): each interface it implements whose
        #: contract uses freshness.v2 and has a collection is covered by a
        #: heartbeat, and it implements the standard contract beside them
        #: (§3: "once for all of them"), added here when not given.
        self.hb_layouts: dict[str, hbeat.Layout] = {}
        for c in contracts:
            if freshness_v2.uses_v2(c.canonical):
                lo = hbeat.layout_of(c.canonical, c.fingerprint)
                if lo.collections:
                    self.hb_layouts[c.interface] = lo
        if self.hb_layouts and not any(c.interface == freshness_v2.PROFILE for c in contracts):
            contracts = [*contracts, freshness_v2.standard_contract()]
        self.system, self.service, self.contracts = system, service, contracts
        self.connect = connect
        self.bindings = bindings or {}
        self.params = {k: dict(v) for k, v in (params or {}).items()}
        #: the bindings with their self.system providers resolved, at start
        self.resolved_bindings: dict[str, list[str]] = dict(self.bindings)
        self.capabilities = set(capabilities or ())
        self.unavailable = dict(unavailable or {})
        self.withhold = set(withhold or ())
        self.handlers = dict(handlers or {})
        self.members = dict(members or {})
        self.hold_s = hold_s
        self.auth = auth
        self.tokenless = set(tokenless or ())
        #: core 0.96 §8.1, §8.4 "On a first hop" (round 102): every interface
        #: it implements in its tokenless set (the SHOULD), its instance token
        #: alone, and no bundle held (§8.4's SHOULD NOT; §8.2's step 3 declares
        #: no contract queryable). archive.v1 and a governed interface are
        #: refused by step 2, as in any tokenless set (the MUST NOT).
        self.first_hop = first_hop
        if first_hop:
            self.tokenless |= {c.interface for c in self.contracts}
        self.router_connect = router_connect
        self.hostid = hostid
        self.hostid_ephemeral = hostid_ephemeral
        self.meta_host = meta_host
        self.state_zid = state_zid
        #: hostid.v1 §2.8: whether this service's system is minted
        self.minted = False
        self.port = None if connect else (port or free_loopback_port())
        self.endpoint = connect or f"tcp/127.0.0.1:{self.port}"
        #: core.md §1.2: 64 random bits, 16 lowercase hex digits.
        self.instance = secrets.token_hex(8)
        self.session: zenoh.Session | None = None
        #: the clock minting reads, ``Session::new_timestamp`` unless set: the
        #: tick is checked "with a clock the implementation controls"
        #: (state.md §1, 0.8)
        self.clock: Any = None
        self._last: zenoh.Timestamp | None = None
        self._lock = threading.Lock()
        self._held: dict[str, _Held] = {}
        self._publishers: dict[str, Any] = {}
        self._encodings: dict[str, str] = {}
        self._entities: list[Any] = []
        #: every call that reached an operation queryable, by key expression
        self.calls: list[str] = []
        #: the calls a handler ran for (refusals before a handler excluded)
        self.handled: list[str] = []
        #: each call's query as this owner received it: (key, Encoding,
        #: payload), for operations.md §9's capture
        self.queries: list[tuple[str, str, bytes]] = []
        #: freshness.v1 §2.4: each re-put state member's horizon
        self.horizons: dict[str, freshness.Horizon] = {}
        #: members whose writer closed: no re-put until a new writer puts
        self.closed_writers: set[str] = set()
        #: every re-put made, (key, stamp)
        self.reputs: list[tuple[str, zenoh.Timestamp]] = []
        self.heartbeat, self.delta_ns = heartbeat, int(delta_s * 1e9)
        #: core §4.3 "Ahead" (0.88): the guard holds the state writes that
        #: carry a Zenoh timestamp (:meth:`_stamped`)
        self.ahead = False
        #: what the owner reports: the guard's transitions (core §4.3: it
        #: "SHOULD report it")
        self.events: list[str] = []
        self._writes = threading.RLock()
        self._stop = threading.Event()
        self._refresher: threading.Thread | None = None
        #: guards the refresher's start against close() (review of PR #807)
        self._refresher_lock = threading.Lock()
        self.initial = dict(initial or {})
        self.cardinality = {k: dict(v) for k, v in (cardinality or {}).items()}
        self.budget = budget
        #: core S3: the tombstone window, "60 s, unless the deployment
        #: configures the owner with another" (link.v1 §2.11 floors it)
        self.tombstone_s = tombstone_s
        #: §2.7: a templated member's resource, by key
        self._member_of: dict[str, tuple[Any, dict[str, Any]]] = {}
        #: a templated stream member's last publication (monotonic s)
        self._stream_last: dict[str, float] = {}
        #: an event member's occurrences (monotonic s)
        self._occurrences: dict[str, list[float]] = {}
        #: every occurrence put, (key, payload)
        self.events_put: list[tuple[str, bytes]] = []
        #: called with (held, offset in ns) on each of the guard's transitions
        self.on_guard: Any = None
        #: a test's hold on state GETs (state.md §10): when set, a state
        #: query is handed to it, and answered when it calls
        #: :meth:`answer_state` with what :meth:`state_answer` made
        self.state_hold: Any = None
        #: core §4.2 (0.100) "Pages": "The page size is 1,024 bytes, unless
        #: the deployment configures the owner with another."
        self.page_size = PAGE_SIZE
        #: each snapshot this owner built, (key, coll, asof, pages): what a
        #: scenario checks of its answers
        self.snapshots_sent: list[tuple[str, int, int, int]] = []
        self.before_tokens = before_tokens
        #: §8.2's plan, kept for a re-mint (:meth:`new_epoch`)
        self._plan_made: dict[str, list[tuple[dict[str, Any], str]]] = {}
        #: the current instance's descriptor queryable and tokens
        self._instance_entities: list[Any] = []
        #: core §6 (0.39): the interfaces it stands by on now
        self.standing_by: set[str] = set(standby or ())
        self.lease_ms = lease_ms
        self.interface_tokens_after_s = interface_tokens_after_s
        #: monotonic instants of the instance token's and the interface
        #: tokens' declarations at start (operations.md §8 step 6)
        self.instance_token_at: float | None = None
        self.interface_tokens_at: float | None = None
        #: zenoh's session namespace (core §1.6), added on egress
        self.namespace = namespace
        #: step 1's entities, per interface (a step-down undeclares them)
        self._iface_entities: dict[str, list[Any]] = {}
        #: the current instance's interface token, per interface
        self._iface_tokens: dict[str, Any] = {}
        #: each instance's current descriptor, which its queryable answers
        self._docs: dict[str, bytes] = {}
        #: core §8.1's member tokens held, (iface, member) -> (key, token)
        self._member_tokens: dict[tuple[str, str], tuple[str, Any]] = {}
        #: redundancy.v1: a candidate of a governed interface draws 32-bit
        #: runs and runs its session's HLC (core §4.3, §8.2's exception)
        self.candidate = any("redundancy.v1" in c.canonical["uses"] for c in contracts)
        self.hlc = self.candidate if hlc is None else hlc
        #: core §4.2's MUST to set a Zenoh timestamp on §4.3's table: False
        #: is a test rig's break of it ("sets no stamp on its puts: it breaks
        #: §2.6's MUST, as this section means it to", timing.v1 scenarios
        #: §5), never an owner's posture
        self.stamp_table = True
        #: core §4.3 (0.73): the run record, the run written under now, the
        #: runs this owner used, the next state-and-event seq, and each
        #: stream key's next seq in the run
        self.run_record = run_record
        self.run: tuple[int, int] | None = None   # (width, value)
        #: every run this owner took, written under or not: a re-mint that
        #: fails keeps the run it took here on purpose. Nothing was written
        #: under it, but a 16-bit one is already durable in the record, which
        #: moved past it, and a 32-bit one is a draw this process made, so
        #: no later draw takes it and "It draws again while the run equals
        #: one it used" (core §4.3) holds of every run it took.
        self.runs_used: list[tuple[int, int]] = []
        self._seq = 0
        #: the last state-and-event seq minted under the run, None for none:
        #: a reply's as-of mark (core 0.94 §5.1)
        self._minted: int | None = None
        self._stream_seq: dict[str, int] = {}
        self._seq_lock = threading.RLock()
        #: samples a role's subscription heard whose envelope was refused,
        #: (key, token): undecodable (observer §7.3), never handed on
        self.undecodable: list[tuple[str, str]] = []
        #: every resource by key, (contract, resource), for the table
        self._resource_of: dict[str, tuple[Any, dict[str, Any]]] = {}
        #: freshness.v2 §2.4: the members whose bit it clears (it no longer
        #: vouches for their values), the interfaces whose heartbeats run, and
        #: every heartbeat sent, (iface, in-band stamp, body value)
        self.unvouched: set[str] = set()
        self._hb_running: set[str] = set()
        self._hb_thread: threading.Thread | None = None
        self.heartbeats_sent: list[tuple[str, zk2c.InBand, dict[str, Any]]] = []
        #: core §7.7 (0.87): each frame key's writer, the keys whose next frame
        #: is lost (a test hook), and every frame put, (key, what was put)
        self._framers: dict[str, Any] = {}
        self._frames_lost: set[str] = set()
        self._frames_lock = threading.RLock()
        #: covered members whose write the clock guard held (F-461's reading)
        self._guard_unvouched: set[str] = set()
        #: telemetry.v2's meters (§2.19), by frame key: None where none applies
        self._meters: dict[str, Any] = {}
        #: the wall clock less the monotonic one, read at the first frame
        self._wall_less_mono: int | None = None
        self.frames_put: list[tuple[str, dict[str, Any]]] = []
        #: False holds every heartbeat back: a test rig's owner that breaks
        #: §2.4's MUST on purpose
        self.send_heartbeats = True

    # -- core §8.1: member tokens (timing.v1 scenarios §2, round 76) -------

    def hold_member(self, iface: str, member: str, epoch: str | None = None) -> str:
        """Core §8.1: "The owner MUST hold one member token per member" of the
        template that declares ``epoch``, at
        ``…/@zk/member/<iface>/<member>/<epoch>``, ``<member>`` the slugged
        value of the epoch parameter and ``<epoch>`` an instance id (§1.2), a
        fresh one unless given. "A member exists from the owner's first
        declaration of it": the caller holds it before its first sample.
        Returns the token's key; one already held is kept."""
        assert self.session is not None
        with self._writes:
            held = self._member_tokens.get((iface, member))
            if held is not None:
                return held[0]
            key = f"zk2/{self.system}/{self.service}/@zk/member/{iface}/{slug(member)}/{epoch or secrets.token_hex(8)}"
            tok = self.session.liveliness().declare_token(key)
            self._member_tokens[(iface, member)] = (key, tok)
            self._entities.append(tok)
            return key

    def release_member(self, iface: str, member: str) -> bool:
        """The member's entity is gone: its token withdrawn. False when none
        was held."""
        with self._writes:
            held = self._member_tokens.pop((iface, member), None)
            if held is None:
                return False
            if held[1] in self._entities:
                self._entities.remove(held[1])
        try:
            held[1].undeclare()
        except Exception:  # noqa: BLE001 - withdrawn anyway
            pass
        return True

    # -- keys -------------------------------------------------------------

    def prefix(self, c: Contract) -> str:
        return f"zk2/{self.system}/{self.service}/{c.interface}"

    @property
    def instance_key(self) -> str:
        return f"zk2/{self.system}/{self.service}/@zk/instance/{self.instance}"

    # -- §4.3 minting -----------------------------------------------------

    def mint(self) -> zenoh.Timestamp:
        """"The greater of Session::new_timestamp() and the last timestamp it
        issued plus one tick" (§4.3). 0.7: a tick is the smallest step the
        timestamp type takes, and "any larger step keeps S7, such as
        zenoh-python's 1 ns", which is what this builds."""
        assert self.session is not None
        with self._lock:
            ts = self.clock() if self.clock is not None else self.session.new_timestamp()
            last = self._last
            if last is not None and ts.get_time_as_ntp64().as_nanos() <= last.get_time_as_ntp64().as_nanos():
                n = last.get_time_as_ntp64().as_nanos() + 1
                ts = zenoh.Timestamp(zenoh.NTP64(n // 1_000_000_000, n % 1_000_000_000), ts.get_id())
            self._last = ts
            return ts

    def catch_up(self, ntp: zenoh.NTP64) -> None:
        """§4.3's catch-up from a record: every stamp minted after this is
        above ``ntp`` (a restarted owner's last stamp, read from its record)."""
        assert self.session is not None
        with self._lock:
            last = self._last
            if last is None or last.get_time_as_ntp64().as_nanos() < ntp.as_nanos():
                self._last = zenoh.Timestamp(ntp, self.session.new_timestamp().get_id())

    # -- core §4.3 (0.73): runs and seqs ---------------------------------

    def take_run(self) -> tuple[int, int]:
        """A new run (core §4.3): the record's counter plus one, written
        durably first, before any sample under it (0.74, F-416), or a 32-bit
        draw with no record, with the counter spent at 0xFFFF ("and leaves
        the record as it is"), and for a candidate. "It draws again while
        the run equals one it used"; "A random run (below) meets" no reuse,
        "The chance of a repeat is accepted" (0.74, F-417). The seqs start
        again at 0.

        A record it cannot use, unreadable, not JSON, without ``run`` or
        with a ``run`` that is no counter from 0 to 0xFFFF, or one it cannot
        write, is no record: "An owner that cannot keep such a record draws
        32-bit runs." It is left as it is, never rewritten from 0, since the
        counter it lost may be any run already used."""
        import os

        with self._seq_lock:
            run: tuple[int, int] | None = None
            if self.run_record is not None and not self.candidate:
                nxt = self._next_counter(os.fspath(self.run_record))
                if nxt is not None:
                    run = (16, nxt)
            if run is None:
                while True:
                    run = (32, secrets.randbits(32))
                    if run not in self.runs_used:
                        break
            self.run = run
            self.runs_used.append(run)
            self._seq = 0
            self._minted = None
            self._stream_seq = {}
            return run

    @staticmethod
    def _next_counter(path: str) -> int | None:
        """The record's next 16-bit run, written durably to ``path`` before
        it is returned, or None where the owner draws instead: the counter
        spent at 0xFFFF, or a record it cannot use or write. A missing file
        is a fresh record, its first run 0."""
        import os

        try:
            with open(path, encoding="utf-8") as f:
                doc = json.load(f)
        except FileNotFoundError:
            doc = None
        except (OSError, ValueError):  # unreadable, not UTF-8, not JSON
            return None
        if doc is None:
            nxt = 0
        else:
            last = doc.get("run") if isinstance(doc, dict) else None
            if type(last) is not int or not 0 <= last <= 0xFFFF:
                return None
            nxt = last + 1
        if nxt > 0xFFFF:
            return None
        tmp = f"{path}.tmp"
        try:
            with open(tmp, "w", encoding="utf-8") as f:
                json.dump({"run": nxt}, f)
                f.flush()
                os.fsync(f.fileno())
            os.replace(tmp, path)
        except OSError:
            return None
        return nxt

    def run_hex(self) -> str:
        """The run as the descriptor states it (core §3.3)."""
        assert self.run is not None
        return zk2c.InBand(self.run[0], self.run[1], 0).run_hex()

    def set_next_seq(self, n: int) -> None:
        """The next state-and-event seq (``state.md`` §7 step 3: "The test
        sets the owner's next seq")."""
        with self._seq_lock:
            if not 0 <= n <= zk2c.SEQ_MAX:
                raise ValueError("a seq is 24 bits")
            self._seq = n

    def _next_inband(self) -> zk2c.InBand:
        """The in-band stamp of the next state mutation or event occurrence:
        "Before it would pass 16,777,215, the owner takes a new run and puts
        its descriptor stating it. Then it writes under the new run from
        seq 0" (core §4.3)."""
        with self._seq_lock:
            if self.run is None:
                self.take_run()
            if self._seq > zk2c.SEQ_MAX:
                self.take_run()
                self._put_descriptor()
            assert self.run is not None
            ib = zk2c.InBand(self.run[0], self.run[1], self._seq)
            self._minted = self._seq
            self._seq += 1
            return ib

    def stream_envelope(self, key: str, record: bytes) -> bytes:
        """``record`` sealed in core §7.5's envelope on this owner's run and
        ``key``'s next stream seq, for a put made round the writer (a test's
        raw put, ``arbitration.v1`` scenarios §11's X): its seq is taken
        here, as :meth:`publish` takes it, so the key's seqs stay one count
        (core §4.3). Refused on a frame resource's key (core §7.7, 0.87)."""
        if self.is_frame_key(key):
            raise OwnerRefused(f"{key}: a frame resource's key, on which no mutation is put (core §7.7)")
        return zk2c.mutation(self._next_stream_inband(key), record, self.tag_of(key))

    def _next_stream_inband(self, key: str) -> zk2c.InBand:
        """A stream key's next in-band stamp: its own seq, from 0 at its
        first sample in the run, wrapping modulo 2^24, never restarted
        within the run (core §4.3)."""
        with self._seq_lock:
            if self.run is None:
                self.take_run()
            assert self.run is not None
            seq = self._stream_seq.get(key, 0)
            self._stream_seq[key] = (seq + 1) % 2**24
            return zk2c.InBand(self.run[0], self.run[1], seq)

    @staticmethod
    def _tag_for(c: Any, r: dict[str, Any]) -> int:
        """CH's tag bits for a sample of ``r`` (core §7.6, 0.75): a packed
        type's tag, "An implementation that writes a record MUST write the
        type's tag in CH's tag bits (§7.5)"; 0 for every other kind."""
        if r.get("type", {}).get("kind") != "packed":
            return 0
        rec = c.schemas.packed_record(r["type"]) if c.schemas is not None else None
        if rec is None:
            raise OwnerRefused(f"{r['token']}/{r['template']}: packed:{r['type']['name']} has no clean record")
        return rec.tag

    def tag_of(self, key: str) -> int:
        """The tag of the type of the resource ``key`` is a sample of: its
        own resource's, or a member's template's."""
        found = self._resource_of.get(key) or self._member_of.get(key)
        return 0 if found is None else self._tag_for(*found)

    def packed_record(self, key: str) -> Any:
        """The packed record of ``key``'s type, for its writer to encode a
        value with (:meth:`zk2py.packed.Record.write`); None where the type
        is not packed. A member's publisher is declared here, as its first
        put would."""
        if key not in self._resource_of and key not in self._member_of:
            self._member_publisher(key)
        c, r = self._resource_of.get(key) or self._member_of[key]
        if r["type"]["kind"] != "packed" or c.schemas is None:
            return None
        return c.schemas.packed_record(r["type"])

    #: The deployment epoch this owner's writers hold for a packed type's
    #: ``time`` fields, Unix seconds; None where the deployment states none,
    #: which is E = 0 (core 0.76 §7.6).
    packed_epoch: int | None = None

    #: The deployment catalog this owner was given (core 0.79 §8.6), a
    #: :class:`zk2py.catalog.Catalog`, set before :meth:`start`, or None. "An
    #: owner given a catalog counts its `time` fields from the catalog's
    #: epoch, and states its id in its descriptor (§3.3)."
    catalog: Any = None

    def writing_epoch(self) -> int:
        """The epoch this owner's packed ``time`` fields count from: its
        catalog's, else the one its deployment states, else 0 (core §7.6,
        §8.6)."""
        from . import packed

        if self.catalog is not None:
            return self.catalog.epoch
        return packed.deployment_epoch(self.packed_epoch)

    def encode_packed(self, key: str, value: Any) -> bytes:
        """``value``, the JSON reading of ``key``'s packed type, as its
        record (core §7.6 "Writing", "What a writer takes"), under the
        deployment epoch. Raises :class:`zk2py.packed.Refused`: nothing is
        put of a value the text refuses."""

        rec = self.packed_record(key)
        if rec is None:
            raise OwnerRefused(f"{key}: its type is not packed")
        return rec.write(value, self.writing_epoch())

    def _stamped(self, key: str) -> bool:
        """Whether a state write of ``key`` carries a Zenoh timestamp (core
        §4.3 "What it holds", 0.88): "where the table lists its resource",
        and "On a session with timestamping enabled (§8.2), every state
        write carries one"."""
        return self.hlc or self._table(key)

    def _table(self, key: str) -> bool:
        """Whether ``key``'s resource carries a Zenoh timestamp the owner
        sets (core §4.3's table): a candidate's every put, or
        :func:`keeps_timestamp`."""
        if not self.stamp_table:
            return False
        if self.candidate:
            return True
        cr = self._resource_of.get(key) or self._member_of.get(key)
        return cr is not None and keeps_timestamp(*cr)

    # -- §8.2 step 2: the exposure plan ----------------------------------

    def _plan(self) -> dict[str, list[tuple[dict[str, Any], str]]]:
        """Each interface's resources, as (resource, "exposed" | "implied" |
        "listed"), refusing what step 2 refuses."""
        plan: dict[str, list[tuple[dict[str, Any], str]]] = {}
        for c in self.contracts:
            rows = plan.setdefault(c.interface, [])
            for r in c.canonical["resources"]:
                rid = f"{r['token']}/{r['template']}"
                missing = [g for g in r["gate"] if g.startswith("capability:")
                           and g.removeprefix("capability:") not in self.capabilities]
                if r["optional"] and missing:
                    if rid in self.unavailable:
                        raise OwnerRefused(f"{rid} is implied absent, so it is not listed (§3.3)")
                    rows.append((r, "implied"))
                elif rid in self.unavailable:
                    if not r["optional"]:
                        raise OwnerRefused(f"required {rid} cannot be unavailable (§2.3)")
                    rows.append((r, "listed"))
                elif rid in self.withhold:
                    # "Step 2 refuses … a required resource not exposed, an
                    # optional one neither exposed nor absent as the
                    # descriptor says".
                    raise OwnerRefused(f"{'optional' if r['optional'] else 'required'} {rid} is neither "
                                       "exposed nor absent as the descriptor says (§8.2 step 2)")
                else:
                    rows.append((r, "exposed"))
            for role, req in c.canonical["requires"].items():
                if not req["optional"] and not self.bindings.get(role):
                    raise OwnerRefused(f"{c.interface}: required role {role!r} is bound to nothing (§3.2)")
        # §3.3 `cardinality`: "The owner's descriptor MUST NOT raise it": a
        # stated bound is from 1 to the contract's, of a templated resource
        # of that interface (D007 otherwise). Refused before anything is
        # declared, as step 2 refuses (round 89).
        for iface, stated in self.cardinality.items():
            c = next((c for c in self.contracts if c.interface == iface), None)
            for rid, n in stated.items():
                r = None if c is None else next(
                    (r for r in c.canonical["resources"] if f"{r['token']}/{r['template']}" == rid
                     and r["params"]), None)
                if r is None:
                    raise OwnerRefused(f"{iface}: cardinality names {rid}, no templated resource of it (§3.3, D007)")
                if not isinstance(n, int) or isinstance(n, bool):
                    # a descriptor's cardinality is a JSON integer (§3.3's
                    # uint64): "3" is a caller's text, never a bound
                    raise OwnerRefused(f"{iface}: cardinality {rid} = {n!r} is not an integer (§3.3, D000)")
                if not 1 <= n <= r["cardinality"]:
                    raise OwnerRefused(f"{iface}: cardinality {rid} = {n} is not from 1 to the contract's "
                                       f"{r['cardinality']}: a descriptor never raises it (§3.3, D007)")
        # §8.2 step 2 (0.17): "It also refuses an owner whose tokenless set
        # names archive.v1, whether or not the owner implements it (§4.4)".
        if "archive.v1" in self.tokenless:
            raise OwnerRefused("archive.v1 is never in the tokenless set: an archive is found by its token (§4.4)")
        # §8.2 step 2 (0.39), redundancy.v1 §2.2 and §2.14: "A runtime MUST
        # refuse to start an instance whose interface's declaration is
        # invalid", a governed interface in the tokenless set among them.
        from . import redundancy
        for c in self.contracts:
            d = redundancy.declaration_of(c.canonical, tokenless=c.interface in self.tokenless)
            if d.declaration == "invalid":
                raise OwnerRefused(f"{c.interface}: its redundancy.v1 declaration is invalid "
                                   f"({', '.join(d.reasons)}): §2.2, core §8.2 step 2")
        known = {c.interface for c in self.contracts}
        for iface in sorted(self.standing_by):
            if iface not in known:
                raise OwnerRefused(f"{iface}: a standby on an interface it does not implement (core §6)")
            if iface in self.tokenless:
                raise OwnerRefused(f"{iface}: an interface with a standby is never tokenless (core §3.3 D012)")
        return plan

    # -- bring-up (§8.2) --------------------------------------------------

    def resolve_system(self) -> None:
        """hostid.v1 §2.3 and §2.7: read the address as the configuration
        spells it, and mint the system "before that service's session opens".
        A configuration error, or a failure to mint, raises before anything
        is declared."""
        if self.hostid is None and self.hostid_ephemeral is None and not self.system.startswith("@"):
            return
        if self.hostid is None:
            raise ValueError("a system starting with @ needs a hostid runtime")
        config: dict[str, Any] = {"address": f"{self.system}/{self.service}"}
        if self.hostid_ephemeral is not None:
            config["hostid"] = {"ephemeral": self.hostid_ephemeral}
        resolved = self.hostid.configure(config)
        self.system, self.minted = resolved.system, resolved.minted

    def resolve_bindings(self) -> None:
        """Core R1 (0.20): self.system providers are resolved once, at start,
        "at the same time, from the same system" as the address (§8.2)."""
        self.resolved_bindings = {role: resolve_providers(list(provs), self.system)
                                  for role, provs in self.bindings.items()}

    def start(self) -> None:
        self.resolve_system()  # hostid.v1 §2.7: before the session, and before step 1
        self.resolve_bindings()  # core R1 (0.20): from the same system
        plan = self._plan()  # step 2 first: a refusal declares nothing
        if self.session is None:
            self.open_session()
        try:
            # core §4.3 (0.73): "A new run is taken by each new instance",
            # before its descriptor states it and before its first mutation.
            self.take_run()
            self._bring_up(plan)
        except Exception:
            self.close()
            raise

    def open_session(self) -> None:
        """The zenoh session, opened and nothing declared on it: :meth:`start`
        opens it itself, and a runner that holds two starts together opens
        it first (``redundancy.v1`` scenarios §3's ``--wait-go``)."""
        conf = zenoh.Config()
        if self.connect:
            conf.insert_json5("mode", json.dumps("client"))
            conf.insert_json5("connect/endpoints", json.dumps([self.connect]))
            if self.auth is not None:
                conf.insert_json5("transport/auth/usrpwd",
                                  json.dumps({"user": self.auth[0], "password": self.auth[1]}))
        else:
            conf.insert_json5("mode", json.dumps("router"))
            conf.insert_json5("listen/endpoints", json.dumps([self.endpoint]))
            if self.router_connect:
                conf.insert_json5("connect/endpoints", json.dumps([self.router_connect]))
        conf.insert_json5("scouting/multicast/enabled", "false")
        # Core §8.2 (0.73), "Session posture": "The owner's session SHOULD
        # run with timestamping disabled"; a candidate of a governed
        # interface enables it (redundancy.v1 §2.10), and so does a test
        # rig that asks (``hlc``). A router's default is on, so it is
        # written out either way.
        conf.insert_json5("timestamping/enabled", "true" if self.hlc else "false")
        if self.lease_ms is not None:
            conf.insert_json5("transport/link/tx/lease", str(int(self.lease_ms)))
        if self.namespace is not None:
            conf.insert_json5("namespace", json.dumps(self.namespace))
        self.session = zenoh.open(conf)

    def _bring_up(self, plan: dict[str, list[tuple[dict[str, Any], str]]]) -> None:
        s = self.session
        assert s is not None
        if self.heartbeat is not None:
            # Core §4.3: "a reference it holds for the purpose, such as a
            # subscription to a router-stamped heartbeat key".
            self._entities.append(s.declare_subscriber(self.heartbeat, zenoh.handlers.Callback(self._beat)))
        # Step 1: resources, publishers and queryables; none of an interface
        # it stands by on (core §8.2, 0.39: "step 2 does not refuse it").
        for c in self.contracts:
            if c.interface not in self.standing_by:
                self._iface_entities[c.interface] = self._declare_resources(c, plan[c.interface])
        self._put_held(plan)
        if self.before_tokens is not None:
            self.before_tokens(self)
        # Step 3: the descriptor's queryable and its first put (§3.3), then a
        # contract queryable per interface.
        self._plan_made = plan
        self._instance_entities, self.descriptor, self._iface_tokens = self._declare_instance(
            plan, self.instance, bundles=True)
        # Step 4 is the second half of _declare_instance.
        if self.horizons:
            self._ensure_refresher()
        # freshness.v2 §2.4 "When": "from its interface tokens on (core §8.2,
        # step 4), never from a standby".
        self._start_heartbeats([i for i in self.hb_layouts if i not in self.standing_by])

    def _declare_resources(self, c: Contract, rows: list[tuple[dict[str, Any], str]]) -> list[Any]:
        """§8.2 step 1 for one interface: its publishers and queryables, the
        entities returned (and kept in ``_entities`` for :meth:`close`)."""
        s = self.session
        assert s is not None
        start = len(self._entities)
        replica = replica_only(rows)
        for r, status in rows:
            key = f"{self.prefix(c)}/{r['token']}/{r['template']}"
            rid = f"{r['token']}/{r['template']}"
            if replica and r["kind"] == "operation" and status != "exposed" and r.get("serving") != "replicated":
                # §5.1 "Beside replicas": "An instance whose exposed
                # resources of an interface are all replicated operations
                # MUST NOT declare an unavailable queryable on an exclusive
                # operation of it": the instance serving it may be another.
                continue
            if r["kind"] == "operation" and status == "exposed" and rid in self.members:
                # §5.1 "Over a template": "One with a queryable per
                # member answers for each it holds."
                for values in self.members[rid]:
                    mkey = member_key(self.prefix(c), r, values)
                    self._entities.append(s.declare_queryable(
                        mkey, zenoh.handlers.Callback(self._op_handler(self.prefix(c), r, values)),
                        complete=True))
            elif r["kind"] == "operation":
                if status == "exposed":
                    handler = self._op_handler(self.prefix(c), r)
                else:
                    cause = "capability" if status == "implied" else self.unavailable[rid]
                    handler = self._unavailable_handler(r, cause)
                self._entities.append(s.declare_queryable(
                    _template_key(key), zenoh.handlers.Callback(handler), complete=True))
            elif status == "exposed" and r["kind"] in ("state", "stream") and not r["params"]:
                enc = r["type"]["media_type"] if r["type"]["kind"] == "raw" else (
                    "application/json" if r.get("encoding") != "cbor" else "application/cbor") \
                    if r["type"]["kind"] == "jsonschema" else "application/protobuf"
                pub = s.declare_publisher(key, encoding=zk2c.ENCODING if enveloped(key) else enc, **qos_of(r))
                self._entities.append(pub)
                self._publishers[key] = pub
                self._encodings[key] = enc
                self._resource_of[key] = (c, r)
                h = freshness.horizon(r["kind"], r["annotations"])
                if r["kind"] == "state" and h.horizon == "within" and not freshness_v2.uses_v2(c.canonical):
                    self.horizons[key] = h
        for token in ("state", "@state"):
            self._entities.append(s.declare_queryable(
                f"{self.prefix(c)}/{token}/**", zenoh.handlers.Callback(self._state_handler),
                complete=False))
        return self._entities[start:]

    def _put_held(self, plan: dict[str, list[tuple[dict[str, Any], str]]], only: str | None = None) -> None:
        """The state values held at start, put before the tokens (§8.2
        "State values"), of every interface it serves, or of ``only``."""
        for c in self.contracts:
            if c.interface in self.standing_by or (only is not None and c.interface != only):
                continue
            for r, status in plan[c.interface]:
                key = f"{self.prefix(c)}/{r['token']}/{r['template']}"
                if status == "exposed" and r["kind"] == "state" and not r["params"] \
                        and r["type"]["kind"] == "raw":
                    self.set_state(key, b"ok")
        for rel, payload in self.initial.items():
            key = f"zk2/{self.system}/{self.service}/{rel}"
            iface = rel.split("/", 1)[0]
            if iface in self.standing_by or (only is not None and iface != only):
                continue
            self.set_state(key, payload)

    def _declare_instance(self, plan: dict[str, list[tuple[dict[str, Any], str]]], instance: str,
                          bundles: bool = False) -> tuple[list[Any], bytes, dict[str, Any]]:
        """§8.2 steps 3 and 4 for ``instance``: the descriptor's queryable and
        its first put, the contracts' queryables (once, at start: they are
        location-free, §8.4), then the instance token and the interface
        tokens. Returns the entities that belong to this instance, which
        :meth:`new_epoch` undeclares, its descriptor, and its interface
        tokens by interface (none for one it stands by on). It assigns
        nothing of the owner's but the instance's descriptor in ``_docs``:
        on a failure it undeclares what it declared for the instance and
        raises, so the caller keeps its id and entities."""
        s = self.session
        assert s is not None
        mine: list[Any] = []
        tokens: dict[str, Any] = {}
        key = f"zk2/{self.system}/{self.service}/@zk/instance/{instance}"
        doc = self._descriptor(plan, instance)
        self._docs[instance] = doc
        try:
            # §3.3 "The GET": one reply, application/json, no attachment and
            # no timestamp. Each instance's queryable answers with its own
            # current record, so the old one still answers truly during a
            # re-mint's overlap, and a takeover's or step-down's put is what
            # a GET reads after it.
            mine.append(s.declare_queryable(key, zenoh.handlers.Callback(
                lambda q, key=key, instance=instance: q.reply(key, self._docs.get(instance, doc),
                                                              encoding="application/json")), complete=True))
            s.put(key, doc, encoding="application/json", timestamp=self.mint())
            if bundles and not self.first_hop:
                for c in self.contracts:
                    ckey = f"zk2/@zk/contract/{c.interface}/{c.fingerprint.removeprefix('sha256:')}"
                    data = bundle.build(c)
                    self._entities.append(s.declare_queryable(
                        ckey, zenoh.handlers.Callback(lambda q, data=data, ckey=ckey: q.reply(
                            ckey, data, encoding="application/json")), complete=True))
            # Step 4: the instance token, then the interface tokens.
            lv = s.liveliness()
            mine.append(lv.declare_token(key))
            withheld = False
            if bundles:
                self.instance_token_at = time.monotonic()
                withheld = self.interface_tokens_after_s is None
                if not withheld and self.interface_tokens_after_s > 0:
                    time.sleep(self.interface_tokens_after_s)
            for c in self.contracts:
                if not withheld and c.interface not in self.tokenless and c.interface not in self.standing_by \
                        and any(status == "exposed" for _, status in plan[c.interface]):
                    tokens[c.interface] = lv.declare_token(self._alive_key(c, instance))
                    mine.append(tokens[c.interface])
            if bundles and not withheld:
                self.interface_tokens_at = time.monotonic()
        except Exception:
            for e in reversed(mine):
                try:
                    e.undeclare()
                except Exception:  # noqa: BLE001 - rolling back anyway
                    pass
            raise
        self._entities.extend(mine)
        return mine, doc, tokens

    def _alive_key(self, c: Contract, instance: str | None = None) -> str:
        fp16 = c.fingerprint.removeprefix("sha256:")[:16]
        return f"zk2/{self.system}/{self.service}/@zk/alive/{c.interface}/{instance or self.instance}/{fp16}"

    def _contract(self, iface: str) -> Contract:
        for c in self.contracts:
            if c.interface == iface:
                return c
        raise KeyError(f"{iface}: not an interface this owner implements")

    def _put_descriptor(self) -> None:
        """The current instance's descriptor put again (§3.3 "Updates"), and
        answered by its queryable from now on."""
        assert self.session is not None
        doc = self._descriptor(self._plan_made, self.instance)
        self._docs[self.instance] = self.descriptor = doc
        self.session.put(f"zk2/{self.system}/{self.service}/@zk/instance/{self.instance}", doc,
                         encoding="application/json", timestamp=self.mint())

    # -- core §6, §8.2 (0.39): serving later, and stopping --------------------

    def take_over(self, iface: str, puts: Any = None) -> bool:
        """Core §8.2 (0.39), "Serving later": steps 1 to 4 for ``iface``, in
        their order: its resources; step 2 passed at the start; the state
        values held (§8.2's SHOULD, and ``puts`` called with the owner: the
        application's own, ``redundancy.v1`` §2.6), then the descriptor put
        again without the mark; the interface token last. False when it was
        not standing by on it.

        A step that fails is undone at once, every step made reversed in
        §2.9's order (main's "Decided here", text 0.2: zenoh refuses one only
        when its session is closing or broken), and its error raised: what
        it declared, the token included, is found whichever step failed, and
        an undo step's own failure is said, never raised over the first."""
        with self._writes:
            if iface not in self.standing_by or self.session is None or self._stop.is_set():
                return False
            c = self._contract(iface)
            start = len(self._entities)
            tok = None
            try:
                self._iface_entities[iface] = self._declare_resources(c, self._plan_made[iface])
                self.standing_by.discard(iface)
                if puts is not None:
                    puts(self)
                else:
                    self._put_held(self._plan_made, only=iface)
                self._put_descriptor()
                tok = self.session.liveliness().declare_token(self._alive_key(c))
                self._iface_tokens[iface] = tok
                self._instance_entities.append(tok)
                self._entities.append(tok)
            except Exception:
                # what it declared, whichever step failed: step 1's entities
                # so far, and a token declared but not yet recorded
                self._iface_entities.setdefault(iface, [e for e in self._entities[start:] if e is not tok])
                if tok is not None:
                    self._iface_tokens[iface] = tok
                undo = self._step_down_steps(c)
                if undo is not None:
                    print(f"zk2py owner: undoing {iface}'s takeover: {type(undo).__name__}: {undo}",
                          file=sys.stderr, flush=True)
                raise
            if self.horizons:
                self._ensure_refresher()
            self._start_heartbeats([iface] if iface in self.hb_layouts else [])
            return True

    def step_down(self, iface: str) -> bool:
        """Core §8.2 (0.39), "stopping": the reverse order: the interface's
        tokens first, then its resources (publishers and queryables, members'
        included), then the descriptor put marked. It puts and deletes none
        of the interface's keys (``redundancy.v1`` §2.4, §2.9). False when it
        was not serving it. Every step is made whatever one before it gave,
        and the first error is raised after the last."""
        with self._writes:
            if iface in self.standing_by or self.session is None or self._stop.is_set():
                return False
            err = self._step_down_steps(self._contract(iface))
            if err is not None:
                raise err
            return True

    def _step_down_steps(self, c: Contract) -> Exception | None:
        """§2.9's steps for ``c``'s interface, each made whatever the one
        before gave: the token withdrawn, the resources undeclared and their
        state forgotten, marked standby, the descriptor put marked. Returns
        the first error, raising none. Called under ``_writes``."""
        iface = c.interface
        first: Exception | None = None
        # freshness.v2 §2.4: "They stop when the owner stops serving the
        # interface: … when it steps down."
        self._hb_running.discard(iface)
        tok = self._iface_tokens.pop(iface, None)
        if tok is not None:
            if tok in self._instance_entities:
                self._instance_entities.remove(tok)
            first = first or self._undeclare_all([tok])
        prefix = self.prefix(c) + "/"
        pubs = [k for k in self._publishers if k.startswith(prefix)]
        # step 1's publishers are in both lists, a member's in _publishers
        # alone: each undeclared once
        gone: dict[int, Any] = {}
        for e in self._iface_entities.pop(iface, []) + [self._publishers[k] for k in pubs]:
            gone.setdefault(id(e), e)
        first = first or self._undeclare_all(list(gone.values()))
        for k in pubs:
            self._publishers.pop(k, None)
            self.horizons.pop(k, None)
        # redundancy.v1 §2.6, §2.9 (0.2): "At step 2 the reference forgets
        # what it put of them": a later takeover holds nothing of this
        # incumbency it does not put again
        for k in [k for k in self._held if k.startswith(prefix)]:
            self._held.pop(k, None)
        self.standing_by.add(iface)
        try:
            self._put_descriptor()
        except Exception as e:  # noqa: BLE001 - its queryable answers the marked descriptor anyway
            first = first or e
        return first

    def _undeclare_all(self, entities: list[Any]) -> Exception | None:
        """Every entity undeclared, whatever one before gave; the first
        error returned (the entity is withdrawn anyway when the session
        closes)."""
        first: Exception | None = None
        for e in entities:
            try:
                e.undeclare()
            except Exception as x:  # noqa: BLE001
                first = first or x
            if e in self._entities:
                self._entities.remove(e)
        return first

    def _undeclare(self, entities: list[Any]) -> None:
        for e in entities:
            try:
                e.undeclare()
            except Exception:  # noqa: BLE001 - withdrawn anyway when the session closes
                pass
            if e in self._entities:
                self._entities.remove(e)

    def _standing_on(self, key: str) -> str | None:
        """The interface ``key`` belongs to when it stands by on it."""
        for c in self.contracts:
            if c.interface in self.standing_by and key.startswith(self.prefix(c) + "/"):
                return c.interface
        return None

    def _refuse_standby_write(self, key: str) -> None:
        iface = self._standing_on(key)
        if iface is not None:
            raise StandbyRefused(f"{key}: it stands by on {iface}, and a standby puts and deletes none of its "
                                 "keys (redundancy.v1 §2.4)")

    def new_epoch(self, new: str | None = None, before: Callable[[], None] | None = None) -> str:
        """Core §1.5 and §8.1: a new instance id while running, "make-before-
        break": the new descriptor's queryable and first put, then its
        tokens, in §8.2's order, and only then the old ones undeclared, "an
        overlap, never a gap". The session is kept (core §1.5's SHOULD of a
        new session is not taken: ``telemetry.v1`` scenarios §2, "the process
        unchanged"), so the stamps' id stays. ``new`` is the new id, a test's
        choice, else 64 random bits. ``before`` runs once the new run is
        taken and before the new instance declares anything, so what it puts
        is the new instance's, under the new run ("A new instance writes
        under a new run", core §4.3), before its tokens (``config.v1`` §2.3);
        one that raises fails the re-mint as a declaration does. Returns the
        old instance id."""
        assert self.session is not None
        old, old_entities = self.instance, self._instance_entities
        new = new or secrets.token_hex(8)
        # Core §7.7: every open frame put under the run its records were
        # measured in, before the new one.
        self._end_frames()
        # Declared first, assigned after: a declaration that raises leaves
        # the owner on its old id, with nothing of the new one declared.
        with self._writes:
            # Core §1.5, §4.3 (0.73): "A new instance writes under a new
            # run", which its first descriptor states. A declaration that
            # raises leaves the old run too.
            with self._seq_lock:
                held = (self.run, self._seq, dict(self._stream_seq), self._minted)
                self.take_run()
            try:
                if before is not None:
                    before()
                entities, doc, tokens = self._declare_instance(self._plan_made, new)
            except Exception:
                # The old run, its seq and its stream seqs come back; the run
                # the failed re-mint took stays in runs_used (see the field).
                with self._seq_lock:
                    self.run, self._seq, self._stream_seq, self._minted = held[0], held[1], held[2], held[3]
                raise
            self.instance, self._instance_entities, self.descriptor = new, entities, doc
            self._iface_tokens = tokens
        for e in old_entities:
            try:
                e.undeclare()
            except Exception:  # noqa: BLE001 - withdrawn anyway when the session closes
                pass
            if e in self._entities:
                self._entities.remove(e)
        return old

    def _ensure_refresher(self) -> None:
        """Start the re-put thread once, never after :meth:`close` set
        ``_stop``: both read and write ``_refresher`` under one lock."""
        with self._refresher_lock:
            if self._refresher is None and not self._stop.is_set():
                self._refresher = threading.Thread(target=self._refresh_loop, daemon=True)
                self._refresher.start()

    def _descriptor(self, plan: dict[str, list[tuple[dict[str, Any], str]]], instance: str | None = None) -> bytes:
        """§3.3: the record, with `profiles` the union of the `uses`, every
        role listed (an unbound one with empty bindings, an optional one
        with ``optional: true``), `unavailable` the listed resources only
        (exposure is compact), and ``meta.zid`` the session's zid, which an
        owner SHOULD state (0.10): a tool attributes a state stamp by it."""
        assert self.session is not None
        doc = {
            "format": "zk2-descriptor/0.1",
            "service": f"{self.system}/{self.service}",
            "instance": self.instance if instance is None else instance,
            # core §3.3 (0.73): "An owner MUST state its run there".
            **({"run": self.run_hex()} if self.run is not None else {}),
            # core §3.3 (0.79): "An owner given a catalog MUST state its id
            # there"; one with none leaves the member out.
            **({"catalog": self.catalog.id} if self.catalog is not None else {}),
            "interfaces": [{
                "iface": c.interface, "contract": c.fingerprint, "minor": c.minor or 0,
                "token": c.interface not in self.tokenless,
                # core §3.3 (0.39): "Absent is serving"
                **({"standby": True} if c.interface in self.standing_by else {}),
                "unavailable": [{"resource": f"{r['token']}/{r['template']}",
                                 "cause": self.unavailable[f"{r['token']}/{r['template']}"]}
                                for r, status in plan[c.interface] if status == "listed"],
                "cardinality": dict(self.cardinality.get(c.interface, {})),
            } for c in self.contracts],
            "capabilities": sorted(self.capabilities),
            # 0.10: `optional` is true for a role the instance works without,
            # "and absent otherwise"; for a contract's role "it repeats that
            # contract's [requires]".
            "requires": [{
                "role": role, "interface": req["interface"], "declared_by": c.interface,
                # R3 (0.20): "a self.system provider resolved".
                "bindings": list(self.resolved_bindings.get(role, [])),
                "params": dict(self.params.get(role, {})) if self.resolved_bindings.get(role) else {},
                **({"optional": True} if req["optional"] else {}),
            } for c in self.contracts for role, req in c.canonical["requires"].items()],
            # Core §3.3 (0.19): "the union of two sets": the contracts' uses,
            # and the derivation-only profiles the instance follows.
            "profiles": profile_order({u for c in self.contracts for u in c.canonical["uses"]}
                                      | ({"hostid.v1"} if self.minted else set())),
            "meta": {**({"zid": str(self.session.zid())} if self.state_zid else {}),
                     **({"host": self.meta_host} if self.meta_host is not None
                        else {"host": socket.gethostname()} if self.minted else {})},
        }
        return json.dumps(doc, separators=(",", ":")).encode()

    # -- state mutations (S1–S3) ------------------------------------------

    def set_state(self, key: str, payload: bytes) -> Any:
        """Put a state value, its in-band stamp in its envelope (S1, core
        §4.3, 0.73), with a Zenoh timestamp on §4.3's table. Returns that
        timestamp, or the in-band stamp where the table keeps none. Every
        change starts the re-put interval again (freshness.v1 §2.4). A put
        after :meth:`close_writer` is a new writer's."""
        with self._writes:
            self._refuse_standby_write(key)
            self._member_publisher(key)
            if self.ahead and self._stamped(key):
                self._unvouch_held(key)
                raise ClockAhead(f"{key}: the clock guard holds the state writes that carry a Zenoh timestamp "
                                 "(core §4.3)")
            held = self._held.get(key)
            if key in self._member_of and (held is None or held.payload is None):
                # §2.7: a new live member. A key with a value is live.
                live = [k for k, h in self._held.items() if h.payload is not None
                        and self._member_of.get(k, (None, None))[1] is self._member_of[key][1]]
                self._check_budget(key, len(live))
            # S1 (0.73): its in-band stamp in its envelope, and a Zenoh
            # timestamp on §4.3's table alone (§8.2's MUST NOT).
            enc = self._encodings[key]
            stamp = self.mint() if self._table(key) else None
            ib, wire = None, payload
            if enveloped(key):
                ib = self._next_inband()
                wire = zk2c.mutation(ib, payload, self.tag_of(key))
            self._publishers[key].put(wire, encoding=zk2c.ENCODING if ib is not None else enc,
                                      **({"timestamp": stamp} if stamp is not None else {}))
            held = _Held(payload, enc, stamp, time.monotonic(), ib, wire)
            if self._covered(key):
                # core 0.82 §4.3's table: the put carries no Zenoh timestamp;
                # its time is the member's confirmation until a heartbeat
                # vouches for it later.
                held.confirm = self.mint()
            self._held[key] = held
            if key in self.closed_writers or key in self._guard_unvouched:
                # a new writer's put: a writer closed had cleared its bit; or
                # the first put the guard lets through after holding one
                self.unvouched.discard(key)
            self.closed_writers.discard(key)
            self._guard_unvouched.discard(key)
            return stamp if stamp is not None else ib

    def _unvouch_held(self, key: str) -> None:
        """F-461's reading (round 98): a ``freshness.v2`` member whose write
        the guard holds, on a session that stamps every write (a
        ``redundancy.v1`` candidate's, core §8.2), holds a value it can no
        longer keep current. Its bit is cleared until a put of it goes
        through (``freshness.v2`` §2.4, "Only a value it holds current"),
        so the heartbeat that goes on (§2.10) never vouches for the value
        the application asked to replace."""
        if self._covered(key) is not None and self._held.get(key) is not None:
            self.unvouched.add(key)
            self._guard_unvouched.add(key)

    def delete_state(self, key: str, elapsed_s: float = 0.0) -> Any:
        """Delete a state key, stamped (S1); answered with ``reply_del`` within
        the tombstone window (S3). "A deleted member is not re-put".

        ``elapsed_s`` dates the tombstone that long ago: a delete made again
        for what is left of the window counted from the delete it repeats
        (desired.v1 §2.11, 0.3, "not a window more")."""
        with self._writes:
            self._refuse_standby_write(key)
            self._member_publisher(key)
            if self.ahead and self._stamped(key):
                self._unvouch_held(key)
                raise ClockAhead(f"{key}: the clock guard holds the state writes that carry a Zenoh timestamp "
                                 "(core §4.3)")
            # S1 (0.73): a delete's in-band stamp is its attachment.
            stamp = self.mint() if self._table(key) else None
            ib, att = None, None
            if enveloped(key):
                ib = self._next_inband()
                att = zk2c.deletion(ib)
            kw: dict[str, Any] = {}
            if stamp is not None:
                kw["timestamp"] = stamp
            if att is not None:
                kw["attachment"] = att
            self._publishers[key].delete(**kw)
            held = _Held(None, self._encodings[key], stamp, time.monotonic() - max(0.0, elapsed_s), ib, att)
            if self._covered(key):
                # core §4.3 "Minting" (0.85): "An owner mints one at each
                # mutation of such a member, though its put carries none": a
                # delete's reply_del carries it, its last confirmation (S2)
                held.confirm = self.mint()
            self._held[key] = held
            return stamp if stamp is not None else ib

    def remove_member(self, key: str) -> None:
        """A templated member removed (``state.md`` §13 step 3): its
        publisher undeclared, so that its next put declares it again. Its
        stream seq is kept: "An owner MUST NOT restart a stream key's seq
        within a run, even where it declares the key's publisher again"
        (core §4.3, 0.73)."""
        with self._writes:
            if key not in self._member_of:
                raise KeyError(f"{key}: no templated member of this owner")
            pub = self._publishers.pop(key, None)
            self._encodings.pop(key, None)
            self._member_of.pop(key, None)
            if pub is not None:
                try:
                    pub.undeclare()
                except Exception:  # noqa: BLE001 - withdrawn when the session closes
                    pass
                if pub in self._entities:
                    self._entities.remove(pub)

    def _member_publisher(self, key: str) -> None:
        """A publisher for a member of a templated state resource, declared
        at its first put (``checks/disk`` of ``checks/{check}``), with the
        resource's QoS and Encoding. The key is concrete, its parameter
        chunks slugged by the caller (§1.4). It runs under ``_writes``
        (reentrant: :meth:`set_state` and :meth:`delete_state` hold it), so
        the refresher never sees a member's horizon before its first put is
        recorded."""
        with self._writes:
            self._declare_member(key)

    def _declare_member(self, key: str) -> None:
        if key in self._publishers:
            return
        assert self.session is not None
        if "*" in key or "$" in key:
            raise ValueError(f"{key}: a member key is concrete (R6)")
        for c in self.contracts:
            for r in c.canonical["resources"]:
                if r["kind"] not in ("state", "stream") or not r["params"]:
                    continue
                pattern = zenoh.KeyExpr(f"{self.prefix(c)}/{r['token']}/{_template_key(r['template'])}")
                if pattern.intersects(zenoh.KeyExpr(key)):
                    self._member_of[key] = (c, r)
                    enc = r["type"]["media_type"] if r["type"]["kind"] == "raw" else (
                        "application/json" if r.get("encoding") != "cbor" else "application/cbor") \
                        if r["type"]["kind"] == "jsonschema" else "application/protobuf"
                    pub = self.session.declare_publisher(key, encoding=zk2c.ENCODING if enveloped(key) else enc,
                                                         **qos_of(r))
                    self._entities.append(pub)
                    self._publishers[key], self._encodings[key] = pub, enc
                    # freshness.v1 §2.4 applies to every member of a state
                    # with a horizon, a templated one's too.
                    # freshness.v2 §2.4: "An owner SHOULD NOT put a member
                    # again to confirm it: its heartbeat does."
                    h = freshness.horizon(r["kind"], r["annotations"])
                    if r["kind"] == "state" and h.horizon == "within" and not freshness_v2.uses_v2(c.canonical):
                        self.horizons[key] = h
                        self._ensure_refresher()
                    return
        raise KeyError(f"{key}: no state resource of this owner holds it")

    # -- core §2.7, the population budget -----------------------------------

    def bound_of(self, c: Any, r: dict[str, Any]) -> int | None:
        """§2.7 "The bound": the cardinality this instance states, when from
        1 to the contract's, else the contract's; None for no ceiling."""
        return self.bound_and_source(c, r)[0]

    def bound_and_source(self, c: Any, r: dict[str, Any]) -> tuple[int | None, str]:
        """The bound and where it comes from: ``descriptor`` or ``contract``."""
        contract = r["cardinality"]
        stated = self.cardinality.get(c.interface, {}).get(f"{r['token']}/{r['template']}")
        if stated is not None and 1 <= stated <= contract:
            b, source = stated, "descriptor"
        else:
            b, source = contract, "contract"
        return (None if b == 4294967295 else b), source

    def _refusal(self, key: str, c: Any, r: dict[str, Any], live: int) -> BudgetExceeded:
        """§2.7 "Keeping it" (0.25): "A refused write publishes nothing, and
        its writer is told". The refusal names the resource, the bound and
        its source (scenarios budget.md §4)."""
        b, source = self.bound_and_source(c, r)
        return BudgetExceeded(f"{key}: refused, {c.interface} {r['token']}/{r['template']} holds {live} live members, "
                              f"at its bound {b}, the {source}'s (core §2.7)")

    def _check_budget(self, key: str, live_others: int) -> None:
        c, r = self._member_of[key]
        b = self.bound_of(c, r)
        if self.budget and b is not None and live_others >= b:
            raise self._refusal(key, c, r, live_others)

    #: core §2.6 (0.103): "where the owner does not trust its clock", its
    #: occurrences' time is unknown; set by a runner that plays such an owner
    clock_untrusted: bool = False

    def event_time(self, at_ms: int | None = None) -> dict[str, int] | None:
        """An occurrence's time as form 4 carries it (core §2.6, §7.5):
        ``{"s", "ms"}`` from the deployment epoch, or None, the unknown time.
        "The time is the owner's clock's, from the deployment epoch (§7.6).
        It is unknown while §4.3's clock guard holds, as a heartbeat's is,
        and where the owner does not trust its clock." ``at_ms`` (Unix ms)
        is "An occurrence seen earlier": an owner that relays one "writes the
        time it occurred". A time outside t32's span is unknown too."""
        if at_ms is None:
            if self.ahead or self.clock_untrusted or self.session is None:
                return None
            at_ms = self._now_ns() // 1_000_000
        total = at_ms - self.writing_epoch() * 1000
        s_, ms = divmod(total, 1000)
        return {"s": s_, "ms": ms} if 0 <= s_ < zk2c.UNKNOWN_T32 else None

    def occur(self, member: str, payload: bytes, at_ms: int | None = None, unknown: bool = False) -> str:
        """An event occurrence (core §2.6, 0.103): "An owner MUST publish each
        occurrence once, with a one-shot put, on
        ``…/<iface>/events/<template>/<occurrence>``, its last chunk the
        occurrence chunk (§1.2) of the run and the seq the occurrence took
        (§4.3). The owner MUST write the put's payload as an event (§7.5's
        form 4), with the occurrence's time." The seq is the owner's one
        count over state and events (§4.3), so a key is unique while S7
        holds. ``at_ms`` dates an occurrence seen earlier, the reference's
        ``put_at`` (state.md §8); ``unknown`` writes the unknown time; the
        clock guard and an untrusted clock write it too (:meth:`event_time`).
        By default the owner keeps §2.7: no new member while ``bound``
        members have an occurrence within the retention, and no more than
        ``rate`` occurrences of a member within its period."""
        from . import budget as bg
        from .lexical import occurrence_chunk

        assert self.session is not None
        found = None
        for c in self.contracts:
            for r in c.canonical["resources"]:
                pattern = zenoh.KeyExpr(f"{self.prefix(c)}/{r['token']}/{_template_key(r['template'])}")
                if r["kind"] == "event" and pattern.intersects(zenoh.KeyExpr(member)):
                    found = (c, r)
        if found is None:
            raise KeyError(f"{member}: no event resource of this owner holds it")
        c, r = found
        now = time.monotonic()
        with self._writes:
            if self.budget:
                n, period = bg.parse_rate(r["rate"])
                recent = [x for x in self._occurrences.get(member, []) if now - x < period / 1e9]
                if len(recent) >= n:
                    raise BudgetExceeded(f"{member}: refused, an occurrence beyond its rate {r['rate']}: "
                                         f"{len(recent) + 1} of one member within its period (core §2.7)")
                if r["params"] and member not in self._occurrences_live(c, r, now):
                    b = self.bound_of(c, r)
                    live = self._occurrences_live(c, r, now)
                    if b is not None and len(live) >= b:
                        raise self._refusal(member, c, r, len(live))
            # Core §2.6 (0.103): the occurrence takes the owner's next seq,
            # under its run, and its key ends in their occurrence chunk.
            ib = self._next_inband()
            key = f"{member}/{occurrence_chunk(ib.width, ib.run, ib.seq)}"
            when = None if unknown else self.event_time(at_ms)
            # §7.2: a raw type's media type, a JSON Schema type's JSON (or
            # CBOR where the resource says so), protobuf's.
            enc = r["type"]["media_type"] if r["type"]["kind"] == "raw" else (
                ("application/cbor" if r.get("encoding") == "cbor" else "application/json")
                if r["type"]["kind"] == "jsonschema" else "application/protobuf")
            # §2.4: with the event's QoS, every field. Each occurrence has a
            # key of its own (its run and seq), so no publisher outlives one put. A
            # reliable event, the default, goes on Session.put with the three
            # stable fields: zenoh's own reliability is reliable (a publisher
            # declared without it reads back RELIABLE). Session.put takes no
            # reliability, so a best_effort event alone goes on a publisher
            # declared for that put. Either way the occurrence is counted
            # only once it is put: one that raises publishes nothing, and
            # its member keeps its rate (§2.7).
            qos = qos_of(r)
            # Core §7.5 (0.101, adopted by 0.103): form 4, `CH · t32 · ms16 ·
            # record`, CH's width bit the key's run's and its tag the
            # record's type's, under zenoh/bytes.
            wire, enc = zk2c.event(ib.width, when, payload, self._tag_for(c, r)), zk2c.ENCODING
            ts = {"timestamp": self.mint()} if self.candidate else {}
            if r["reliability"] == "reliable":
                self.session.put(key, wire, encoding=enc, congestion_control=qos["congestion_control"],
                                 priority=qos["priority"], express=qos["express"], **ts)
            else:
                pub = self.session.declare_publisher(key, encoding=enc, **qos)
                try:
                    pub.put(wire, **ts)
                finally:
                    try:
                        pub.undeclare()
                    except Exception:  # noqa: BLE001 - put already; withdrawn when the session closes
                        pass
            self._occurrences.setdefault(member, []).append(now)
            self.events_put.append((key, payload))
            return key

    def _occurrences_live(self, c: Any, r: dict[str, Any], now: float) -> set[str]:
        """§2.7: an event member is live "for the retention after its last
        occurrence"."""
        pattern = zenoh.KeyExpr(f"{self.prefix(c)}/{r['token']}/{_template_key(r['template'])}")
        return {m for m, ts in self._occurrences.items()
                if ts and now - ts[-1] < r["retention_s"] and pattern.intersects(zenoh.KeyExpr(m))}

    def close_writer(self, key: str) -> None:
        """freshness.v1 §2.4: "The re-puts stop … when its writer for that
        member closes." The owner still holds the value and answers GETs
        with it, under its last stamp; nobody confirms it. Under
        ``freshness.v2`` (round 97) the member's bit clears: its heartbeat
        stops vouching for it (``alarms.v1`` 0.11 §2.7: "It clears the
        alarm's bit and lets the alarm go stale"; ``health.v1`` 0.8 §2.3)."""
        with self._writes:
            self.closed_writers.add(key)
            if self._covered(key) is not None:
                self.unvouched.add(key)

    def reopen_writer(self, key: str) -> None:
        """The member's writer is back (``alarms.v1`` §2.7, 0.2: "An
        evaluation that finds the condition again confirms it again"): its
        re-puts resume, the next one at once if its interval ran out. Under
        ``freshness.v2`` its bit is set again: "An evaluation that finds the
        condition again sets the bit again" (``alarms.v1`` 0.11)."""
        with self._writes:
            self.closed_writers.discard(key)
            if self._covered(key) is not None:
                self.unvouched.discard(key)

    # -- freshness.v1 §2.4 and core §4.3 -----------------------------------

    def _now_ns(self) -> int:
        """The owner's clock, as it stamps (``clock`` when set)."""
        assert self.session is not None
        ts = self.clock() if self.clock is not None else self.session.new_timestamp()
        return ts.get_time_as_ntp64().as_nanos()

    def _beat(self, sample: zenoh.Sample) -> None:
        """Core §4.3: a heartbeat's router stamp against the owner's clock."""
        if sample.timestamp is None or self.session is None:
            return
        off = self._now_ns() - sample.timestamp.get_time_as_ntp64().as_nanos()
        changed = None
        with self._writes:
            if off > self.delta_ns and not self.ahead:
                self.ahead = changed = True
                self.events.append(f"ahead by {off / 1e9:.3f} s, beyond the delta: the state writes that carry a Zenoh timestamp held (core §4.3)")
            elif off <= self.delta_ns and self.ahead:
                self.ahead, changed = False, False
                self.events.append(f"within the delta again ({off / 1e9:.3f} s): state writes released")
        if changed is not None and self.on_guard is not None:
            # Core §4.3 (0.23): the report is a stream sample, which the
            # guard does not hold (health.v1 §2.5).
            self.on_guard(changed, off)

    # -- freshness.v2 §2.4: the heartbeats (core §7.5's form 6) ------------------

    def _covered(self, key: str) -> hbeat.Collection | None:
        """The collection of a covered interface ``key`` is a member of."""
        for iface, lo in self.hb_layouts.items():
            prefix = f"zk2/{self.system}/{self.service}/{iface}/"
            if key.startswith(prefix):
                for col in lo.collections:
                    if freshness_v2._member_of(lo, col, key):
                        return col
        return None

    def vouch(self, key: str, on: bool) -> None:
        """Set or clear a member's bit (§2.4 "Only a value it holds
        current"): an owner that can no longer vouch for a value "clears its
        bit and lets the member go stale". A put does not set it again."""
        with self._writes:
            if self._covered(key) is None:
                raise KeyError(f"{key}: no member of a collection a heartbeat of this owner covers")
            (self.unvouched.discard if on else self.unvouched.add)(key)

    def _start_heartbeats(self, ifaces: list[str]) -> None:
        if not ifaces:
            return
        with self._refresher_lock:
            if self._stop.is_set():
                return
            self._hb_running.update(ifaces)
            if self._hb_thread is None:
                self._hb_thread = threading.Thread(target=self._heartbeat_loop, daemon=True)
                self._hb_thread.start()

    def heartbeat_body(self, iface: str, now: zenoh.Timestamp | None) -> dict[str, Any]:
        """The value of ``iface``'s next body (§2.4): each exposed
        collection's map bit, its digest over the members held live, a bit
        for each it vouches for; under the clock guard (§2.10, 0.4) the bits
        as at any other time, and the time unknown: "An owner MUST write all
        ones as its heartbeat's time while its guard holds"."""
        lo = self.hb_layouts[iface]
        exposed = {f"{r['token']}/{r['template']}" for r, status in self._plan_made.get(iface, [])
                   if status == "exposed"}
        prefix = f"zk2/{self.system}/{self.service}/{iface}/"
        members = {k: freshness_v2.OwnerView(k, h.inband, k not in self.unvouched)
                   for k, h in self._held.items() if k.startswith(prefix) and h.payload is not None}
        known = None if now is None or self.ahead else hbeat.time_of(
            now.get_time_as_ntp64().as_nanos(), self.writing_epoch())
        return freshness_v2.owner_body(lo, exposed, members, known)

    def _put_heartbeat(self, iface: str) -> None:
        """One heartbeat covering ``iface``: a put on its heartbeat key (§3),
        on the stream's publisher with the contract's QoS, its payload form
        6, CH the covered revision's tag, the run and seq the key's own (core
        §4.3 "Streams"). Each member it vouches for is confirmed at its time
        (:attr:`_Held.confirm`)."""
        lo = self.hb_layouts[iface]
        key = hbeat.heartbeat_key(self.system, self.service, iface)
        with self._writes:
            if self._stop.is_set() or self.session is None or iface not in self._hb_running:
                return
            if key not in self._publishers:
                self._member_publisher(key)
            now = self.mint()
            value = self.heartbeat_body(iface, now)
            ib = self._next_stream_inband(key)
            self._publishers[key].put(zk2c.heartbeat(ib, hbeat.write(lo, value), lo.tag),
                                      encoding=zk2c.ENCODING)
            self.heartbeats_sent.append((iface, ib, value))
            # core §4.3 "Minting" (0.88): "The heartbeat still mints that
            # timestamp" while the guard holds, so each member it vouches
            # for is confirmed at it, from the clock ahead (freshness.v2
            # §2.10: "Its replies carry a last confirmation minted from the
            # clock ahead").
            prefix = f"zk2/{self.system}/{self.service}/{iface}/"
            for k, h in self._held.items():
                if k.startswith(prefix) and h.payload is not None and k not in self.unvouched \
                        and self._covered(k) is not None and f"{self._covered(k).resource}" in value["sections"]:
                    h.confirm = now

    def _heartbeat_loop(self) -> None:
        """Each running interface's heartbeat at its period
        (:func:`zk2py.freshness_v2.period_ns`), the first at once; "It sends
        them with no member live too"."""
        due: dict[str, int] = {}
        while not self._stop.is_set():
            now = time.monotonic_ns()
            for iface in sorted(self._hb_running):
                if not self.send_heartbeats:
                    continue
                if due.get(iface, 0) <= now:
                    try:
                        self._put_heartbeat(iface)
                    except Exception as e:  # noqa: BLE001 - the next period tries again
                        print(f"zk2py owner: heartbeat {iface}: {type(e).__name__}: {e}", file=sys.stderr, flush=True)
                    due[iface] = now + (freshness_v2.period_ns(self.hb_layouts[iface]) or 10**9)
            for iface in [i for i in due if i not in self._hb_running]:
                due.pop(iface)
            nxt = min(due.values(), default=time.monotonic_ns() + 50_000_000)
            self._stop.wait(max(0.0, min((nxt - time.monotonic_ns()) / 1e9, 0.05)))

    def _refresh_loop(self) -> None:
        """freshness.v1 §2.4: re-put each held member once ttl/2 has passed
        since its last put, less a margin for the scheduler (a fifth of
        ttl/2, at most 200 ms), so that two puts are never more than ttl/2
        apart. A deleted member, one whose writer closed, and every member
        whose re-put carries a Zenoh timestamp while the clock guard holds,
        are not re-put (§2.10; core §4.3, 0.88). When the guard
        releases, the members whose interval ran out are re-put at the next
        tick."""
        def horizons() -> list[tuple[str, freshness.Horizon]]:
            # A member's horizon is registered under _writes, with its first
            # put (_member_publisher): the snapshot is taken under it too.
            with self._writes:
                return list(self.horizons.items())

        def tick() -> float:
            refresh = min((h.refresh_ns for _, h in horizons() if h.refresh_ns), default=10**9)
            return min(max(refresh / 10 / 1e9, 0.01), 0.1)

        while not self._stop.wait(tick()):
            for key, h in horizons():
                due = h.refresh_ns - min(h.refresh_ns // 5, 200_000_000)
                with self._writes:
                    held = self._held.get(key)
                    if self._stop.is_set() or self.session is None or held is None or held.payload is None \
                            or key in self.closed_writers:
                        continue
                    if self.ahead and self._stamped(key):
                        # Core 0.90 "A candidate's" and freshness.v2 0.5
                        # §2.10 (F-461's answer): the guard holds the re-put
                        # as it holds a put, and a covered member's bit is
                        # cleared until a write of it goes on (review-bot on
                        # round 98). A freshness.v2 member is never in
                        # self.horizons (its §2.4: no re-put to confirm it),
                        # so this clears nothing today; it keeps the three
                        # held writes alike, as the reference's StateWriter
                        # does.
                        self._unvouch_held(key)
                        continue
                    if (time.monotonic() - held.at) * 1e9 < due:
                        continue
                    # "the owner re-puts the member's current value,
                    # unchanged: the same payload, Encoding and attachment",
                    # under "a fresh timestamp the owner set" (core S1, §4.3).
                    # Core §4.2 (0.73): "A re-put is a mutation … It
                    # carries a new seq (S1)", and on the table a Zenoh
                    # timestamp minted above every one the owner issued.
                    stamp = self.mint() if self._table(key) else None
                    ib, wire = None, held.payload
                    if enveloped(key):
                        ib = self._next_inband()
                        wire = zk2c.mutation(ib, held.payload, self.tag_of(key))
                    self._publishers[key].put(wire, encoding=zk2c.ENCODING if ib is not None else held.encoding,
                                              **({"timestamp": stamp} if stamp is not None else {}))
                    self._held[key] = _Held(held.payload, held.encoding, stamp, time.monotonic(), ib, wire)
                    self.reputs.append((key, stamp if stamp is not None else ib))

    # -- core §7.7: frames (0.87, #981 K3) ---------------------------------------------

    #: Whether this owner's wall clock is known: False writes every frame's
    #: time as unknown, all 42 bits set (core §7.7 "Times": "or all ones
    #: while its clock is unknown"), a test's choice.
    frame_clock_known: bool = True

    def is_frame_key(self, key: str) -> bool:
        """Whether ``key`` is a frame resource's (core §7.7: its template has
        no parameter, so its key is the resource's own)."""
        found = self._resource_of.get(key)
        return found is not None and found[1].get("frame") is not None

    def _framer(self, key: str) -> Any:
        from . import frame

        if not self.is_frame_key(key):
            raise OwnerRefused(f"{key}: no frame resource of this owner (core §7.7)")
        with self._frames_lock:
            if key not in self._framers:
                c, r = self._resource_of[key]
                lo = frame.layout_of(c, r)
                assert lo is not None
                self._framers[key] = frame.Framer(lo, self.writing_epoch())
            return self._framers[key]

    def record_frame(self, key: str, values: dict[str, Any], at_ns: int | None = None) -> list[dict[str, Any]]:
        """One record of a frame resource, §7.6's JSON reading (an absent
        optional field left out), measured now or at the monotonic instant
        ``at_ns``: gathered into the key's open frame, and every frame it
        makes ready put at once (:class:`zk2py.frame.Framer`). Returns the
        frames put, each ``{run, seq, records, bytes, lost}``. Raises
        :class:`zk2py.frame.Refused` or :class:`zk2py.packed.Refused` for a
        record the text refuses: nothing is gathered of it."""
        self._refuse_standby_write(key)
        fr_ = self._framer(key)
        mono = time.monotonic_ns() if at_ns is None else at_ns
        wall = self._frame_wall(mono)
        with self._frames_lock:
            meter = self._meter(key)
            run = None
            if meter is not None:
                # telemetry.v2 §2.19: "Within one run, an owner MUST NOT write
                # a counter field's count below one it wrote before on that
                # key", the count before §2.20's reduction, under the run the
                # record is put in (an open frame is put before a new run).
                with self._seq_lock:
                    if self.run is None:
                        self.take_run()
                    run = self.run
                meter.check(run, values)
            ready = fr_.record(values, mono, wall)
            if meter is not None:
                meter.gather(run, values)
            return [self._put_metered(key, fr_, r, meter) for r in ready]

    def _put_metered(self, key: str, fr_: Any, ready: Any, meter: Any) -> dict[str, Any]:
        """A ready frame put, its records' counts accepted by the meter once
        the put went through, and forgotten if it raised (review-bot on round
        98: "Accept after the put succeeds")."""
        n = len(ready.records)
        try:
            out = self._put_frame(key, fr_, ready)
        except BaseException:
            if meter is not None:
                meter.unsent(n)
            raise
        if meter is not None:
            meter.put(n)
        return out

    def _meter(self, key: str) -> Any:
        """``telemetry.v2``'s meter of a frame key (§2.19), or None where its
        contract does not use telemetry.v2 or its ``telemetry.series`` is not
        declared. A counter field's count is written modulo its width by
        :func:`zk2py.packed.encode` (§2.20, core §7.6)."""
        from . import telemetry as tm, telemetry_v2 as t2

        if key in self._meters:
            return self._meters[key]
        c, r = self._resource_of[key]
        meter = None
        if t2.uses_v2(c.canonical):
            d = t2.contract_declarations(c).get(r["template"])
            if isinstance(d, t2.FrameDeclaration) and d.declaration == tm.DECLARED:
                meter = t2.Meter(d)
        self._meters[key] = meter
        return meter

    def end_frame(self, key: str) -> dict[str, Any] | None:
        """Core §7.7's MAY (0.87): "An owner MAY end a frame before it holds
        records records, and puts it then." The frame put, or None with
        none open."""
        fr_ = self._framer(key)
        with self._frames_lock:
            ready = fr_.end()
            return None if ready is None else self._put_metered(key, fr_, ready, self._meter(key))

    def _frame_wall(self, mono: int) -> dict[str, int] | None:
        """A record's wall time (core §7.7 "Times") for its instant ``mono``
        on the monotonic clock: the wall clock less the monotonic one, read
        once at the owner's first frame, so that two records' instants
        differ by their monotonic interval to the millisecond; None while
        the clock is unknown (:attr:`frame_clock_known`)."""
        if not self.frame_clock_known:
            return None
        if self._wall_less_mono is None:
            self._wall_less_mono = time.time_ns() - time.monotonic_ns()
        wall_ms = (mono + self._wall_less_mono) // 1_000_000 - self.writing_epoch() * 1000
        return {"s": wall_ms // 1000, "ms": wall_ms % 1000} if wall_ms >= 0 else None

    def raw_frame(self, key: str, values: dict[str, Any]) -> dict[str, Any]:
        """``scenarios-v2.md``'s *raw* step: "puts a frame on the owner's key
        from the owner's session, under its run and its next seq. It
        bypasses any check the owner's runtime makes": one full record, its
        time now, each count reduced to its field's width (core §7.6), and
        no meter (telemetry.v2 §2.19). The framer's own record before is
        left as it was."""
        fr_ = self._framer(key)
        lo = fr_.layout
        with self._frames_lock, self._seq_lock:
            if self.run is None:
                self.take_run()
            run = self.run
            assert run is not None
            seq = self._stream_seq.get(key, 0)
            self._stream_seq[key] = (seq + 1) % 2**24
        value = {"time": self._frame_wall(time.monotonic_ns()),
                 "records": [{"full": True, "values": dict(values),
                              "absent": [f.name for f in lo.fields if f.name not in values]}]}
        payload = zk2c.write("frame", run[0], run[1], seq, lo.tag, lo.write(value, self.writing_epoch()))
        if key not in self._publishers:
            self._member_publisher(key)
        self._publishers[key].put(payload, encoding=zk2c.ENCODING)
        out = {"run": zk2c.InBand(run[0], run[1], 0).run_hex(), "seq": seq, "records": value,
               "bytes": len(payload), "lost": False, "raw": True}
        self.frames_put.append((key, out))
        return out

    def lose_frame(self, key: str) -> None:
        """``frames.md``'s lost frame, a test hook: "O takes the frame's seqs
        and puts nothing, as a put a network dropped". The next frame put on
        ``key`` is lost so."""
        self._framer(key)
        with self._frames_lock:
            self._frames_lost.add(key)

    def _end_frames(self) -> None:
        """Every open frame put, before a new run (:meth:`new_epoch`): a
        frame's records were measured under the run it is put in."""
        for key in list(self._framers):
            self.end_frame(key)

    def _put_frame(self, key: str, fr_: Any, ready: Any) -> dict[str, Any]:
        """A ready frame put on ``key`` (core §7.7): its value written under
        the run it is put in, record 0 the next seq of the key, "and the
        frame after it the seq that follows its last record's", in form 5
        under the frame's tag, ``zenoh/bytes``, on the stream's publisher
        with its QoS and no Zenoh timestamp (no row of §4.3's table)."""
        with self._seq_lock:
            if self.run is None:
                self.take_run()
            assert self.run is not None
            run = self.run
            value = fr_.seal(ready, run)
            n = len(value["records"])
            seq = self._stream_seq.get(key, 0)
            self._stream_seq[key] = (seq + n) % 2**24
        lost = key in self._frames_lost
        try:
            body = fr_.layout.write(value, self.writing_epoch())
            payload = zk2c.write("frame", run[0], run[1], seq, fr_.layout.tag, body)
            if not lost:
                if key not in self._publishers:
                    self._member_publisher(key)
                self._publishers[key].put(payload, encoding=zk2c.ENCODING)
        except BaseException:
            # A frame never sent gives its seqs back, where no later put took
            # the ones after them, so that its reader reads no gap.
            with self._seq_lock:
                if self._stream_seq.get(key) == (seq + n) % 2**24:
                    self._stream_seq[key] = seq
            raise
        if lost:
            self._frames_lost.discard(key)
        out = {"run": zk2c.InBand(run[0], run[1], 0).run_hex(), "seq": seq, "records": value,
               "bytes": len(payload), "lost": lost}
        self.frames_put.append((key, out))
        return out

    def publish(self, key: str, payload: bytes, timestamp: zenoh.Timestamp | None = None,
                attachment: bytes | None = None) -> None:
        """Put a stream sample on its declared publisher: on a ``stream``
        key in §7.5's envelope under its own seq, its Encoding
        ``zenoh/bytes``, and on an ``@stream`` key as itself, with the
        resource's Encoding (§7.2, §4.3, 0.73); stamped ``timestamp`` when
        given (a test's, or a ``health.v1`` fault's, its owner's choice),
        else on §4.3's table by the owner, else not at all (§8.2); and with
        ``attachment`` when given (its bytes as the resource's attachment
        encoding writes them, §7.2). A frame resource's key takes
        none: "An owner MUST NOT put a mutation (§7.5) on a frame resource's
        key" (core §7.7, 0.87), so it is refused (:meth:`record_frame`)."""
        self._refuse_standby_write(key)
        if self.is_frame_key(key):
            raise OwnerRefused(f"{key}: a frame resource's key, on which no mutation is put (core §7.7): "
                               "write its records with record_frame")
        if key not in self._publishers:
            self._member_publisher(key)
        if key in self._member_of:
            # §2.7: a stream member is live "for an hour after the owner last
            # published on it".
            now = time.monotonic()
            c, r = self._member_of[key]
            live = [k for k, last in self._stream_last.items() if k != key and now - last < 3600
                    and self._member_of.get(k, (None, None))[1] is self._member_of[key][1]]
            if key not in self._stream_last or now - self._stream_last[key] >= 3600:
                self._check_budget(key, len(live))
            self._stream_last[key] = now
        kw: dict[str, Any] = {"encoding": self._encodings[key]}
        if timestamp is None and self._table(key):
            # Core §4.3's table (0.73): a stream declaring a timing.v1
            # lifespan keeps a Zenoh timestamp, which the owner stamps
            # (timing.v1 §2.6), as does a candidate's every put.
            timestamp = self.mint()
        if timestamp is not None:
            kw["timestamp"] = timestamp
        if attachment is not None:
            kw["attachment"] = attachment
        if enveloped(key):
            # Core §4.3: a stream key's own seq, in §7.5's envelope, under
            # zenoh/bytes; an @stream sample keeps its wire.
            payload = zk2c.mutation(self._next_stream_inband(key), payload, self.tag_of(key))
            kw["encoding"] = zk2c.ENCODING
        self._publishers[key].put(payload, **kw)

    # -- the consumer's side (§3.2) ---------------------------------------

    def role_keys(self, role: str, iface: str, rid: str) -> list[str]:
        """The key expression of ``rid`` (``<kind token>/<template>``) at
        each provider ``role`` is bound to, through the bindings as resolved
        at start (R1, 0.20): ``<system>/*`` stays a wildcard over services.

        A resource named with its parameters (``stream/axis/{axis}``) is
        subscribed by its template's selector, each parameter a wildcard, so
        what is declared is its literal prefix (core §8.5, 0.78,
        :func:`zk2py.face.resource_scope`); a concrete resource by its exact
        key. A parameter the role binds (R2) is its value, ``self.system``
        and ``self.service`` resolved, never a wildcard. Nothing wider than
        ``rid`` is asked."""
        from . import face

        own = {"self.system": self.system, "self.service": self.service}
        bound = {k: own.get(v, v) for k, v in self.params.get(role, {}).items()}
        return [f"zk2/{p}/{iface}/{face.resource_scope(rid, bound)}" for p in self.resolved_bindings.get(role, [])]

    def subscribe_role(self, role: str, iface: str, rid: str, sink: Any, discarded: Any = None,
                       record: Any = None, epoch: int | None = None, across: Any = None,
                       horizon_s: float | None = None) -> list[Any]:
        """Subscribe through a role's resolved bindings, handing ``sink``
        each sample's (key, payload). A sample whose key expression is not
        concrete is discarded (R6), and its key handed to ``discarded``.

        With ``record``, the :class:`zk2py.packed.Record` of the resource's
        packed type in the revision this consumer holds, each put is read
        through it (core §7.6, observer §7.3 at 0.16): ``sink`` gets its
        JSON reading, and one the layout refuses, or that came in no
        envelope, is undecodable, counted with its refusal named and never
        handed on. Its ``time`` fields are read under ``epoch``, the
        deployment's, 0 where none is stated. A delete carries no record, so
        it hands ``sink`` (key, None), never bytes; its envelope, in the
        delete's attachment, is read as every delete's is. A put on an events
        key that carries no event, a mutation or a payload with no envelope,
        is undecodable, ``form``, never handed on (core 0.104 §2.6,
        :func:`zk2py.live.no_event`).

        ``across``, a :class:`zk2py.far_side.FarSide` (core §8.5, 0.91):
        where the face's medium repairs no lost frame, each subscription with
        a horizon, ``horizon_s`` (:func:`zk2py.far_side.resource_horizon_s`
        of the provider's resource), is declared again after two horizons of
        silence while the session's transport stayed up
        (:class:`zk2py.far_side.Redeclaring`). Returns the subscriptions,
        each undeclared by ``.undeclare()``."""
        from . import far_side, packed
        from .live import no_event

        assert self.session is not None

        def received(sample: zenoh.Sample) -> None:
            key = str(sample.key_expr)
            if "*" not in key:
                # Observer §7.3 (0.14) and core §4.3 (0.73): the envelope's
                # record is the payload; a refused one is undecodable, never
                # handed on, and counted.
                read = zk2c.read_sample(sample)
                if read.refused:
                    self.undecodable.append((key, read.refusal))
                    return
                if sample.kind == zenoh.SampleKind.PUT:
                    # Core §2.6 (0.104): "A consumer MUST NOT deliver a put on
                    # an events key whose payload is no event ... It counts it
                    # undecodable, refused as form."
                    token = no_event(key, read)
                    if token is not None:
                        self.undecodable.append((key, token))
                        return
                if record is not None:
                    if sample.kind == zenoh.SampleKind.DELETE:
                        sink(key, None)
                        return
                    got = packed.read_sample(record, read, epoch)
                    if got.undecodable:
                        self.undecodable.append((key, got.refusal))
                        return
                    sink(key, got.value)
                    return
                sink(key, read.record if read.record is not None else b"")
            elif discarded is not None:
                discarded(key)

        subs = [far_side.subscribe(self.session, k, received, horizon_s, across)
                for k in self.role_keys(role, iface, rid)]
        self._entities += subs
        return subs

    # -- handlers ---------------------------------------------------------

    def _state_handler(self, query: zenoh.Query) -> None:
        """S2 (core 0.100): "It MUST answer a GET over them per collection,
        as 'Answering a state GET' below says" (:meth:`state_answer`)."""
        if self.state_hold is not None:
            self.state_hold(query)
            return
        self.answer_state(query, self.state_answer(query), finish=False)

    def collections(self) -> list[Collection]:
        """Each collection this owner answers by snapshot (core §4.2, 0.100):
        every exposed ``state`` and ``@state`` resource of an interface it
        serves that is no per-key resource (:func:`per_key`)."""
        out: list[Collection] = []
        plan = getattr(self, "_plan_made", None) or {}
        for c in self.contracts:
            if c.interface in self.standing_by:
                continue
            exposed = {(r["token"], r["template"]) for r, status in plan.get(c.interface, []) if status == "exposed"}
            for token in ("state", "@state"):
                rows = [r for r in c.canonical["resources"] if r["token"] == token]
                for i, r in enumerate(rows):
                    if (token, r["template"]) not in exposed or per_key(c, r):
                        continue
                    base = f"{self.prefix(c)}/{token}"
                    horizon = freshness.horizon("state", r["annotations"]).horizon == "within" \
                        and freshness_v2.uses_v2(c.canonical)
                    out.append(Collection(c, r, base, i, snapshot_key(base, r["template"]), self._tag_for(c, r),
                                          horizon))
        return out

    def _resource_key(self, key: str) -> tuple[Any, dict[str, Any]] | None:
        return self._resource_of.get(key) or self._member_of.get(key)

    def _live_or_windowed(self, now: float) -> list[tuple[str, _Held]]:
        return [(k, h) for k, h in list(self._held.items())
                if h.payload is not None or now - h.at <= self.tombstone_s]

    def state_answer(self, query: zenoh.Query | str) -> list[tuple[Any, ...]]:
        """The answer to a state GET, made now (core §4.2, 0.100, "Answering
        a state GET"), as items :meth:`answer_state` sends:

        - ``("reply", key, held)``: "A per-key resource keeps S2's reply per
          key … An owner MUST answer each member of a per-key resource that a
          GET reaches with its reply, or with its ``reply_del`` within the
          window";
        - ``("snapshot", key, pages)``: a collection read's snapshot, "on its
          snapshot key, in pages"; a member read's "snapshot of that member
          alone, on the member's own key … one page, whose one entry has the
          empty id"; or, for "a member read of one concrete key that it holds
          no member at, live or deleted within the window", "a snapshot of no
          entry on that key".

        A collection read "is a GET whose selector includes the snapshot key
        followed by ``/**``"; a member read reaches a collection's member
        keys and is no collection read of it. Every snapshot of one answer
        is built at once, under the writes' lock, "from one state of the
        collection"."""
        sel = str(query if isinstance(query, str) else query.key_expr)
        asked = zenoh.KeyExpr(sel)
        concrete = not any(templates.is_wild(c) for c in sel.split("/"))
        out: list[tuple[Any, ...]] = []
        with self._writes:
            now = time.monotonic()
            held = self._live_or_windowed(now)
            cols = self.collections()
            snap_cols = {(id(c.contract), c.resource["token"], c.resource["template"]) for c in cols}
            for key, h in held:
                cr = self._resource_key(key)
                if cr is not None and (id(cr[0]), cr[1]["token"], cr[1]["template"]) in snap_cols:
                    continue
                if asked.intersects(zenoh.KeyExpr(key)):
                    out.append(("reply", key, h))
            header: dict[str, Any] | None = None
            for col in cols:
                members = [(k, h) for k, h in held if self._resource_key(k) is not None
                           and self._resource_key(k)[1] is col.resource]  # type: ignore[index]
                if col.key is not None and asked.includes(zenoh.KeyExpr(col.key + "/**")):
                    header = header or self._snapshot_header()
                    entries = [(k[len(col.key) + 1:] if k != col.key else "", k, h) for k, h in members]
                    out.append(("snapshot", col.key, self._pages(col, header, entries)))
                    continue
                reached = [(k, h) for k, h in members if asked.intersects(zenoh.KeyExpr(k))]
                for k, h in sorted(reached, key=lambda x: x[0].encode()):
                    header = header or self._snapshot_header()
                    out.append(("snapshot", k, self._pages(col, header, [("", k, h)])))
                if concrete and not reached and col.member(sel) and self._winner(col, sel):
                    header = header or self._snapshot_header()
                    out.append(("snapshot", sel, self._pages(col, header, [])))
        return out

    def _winner(self, col: Collection, key: str) -> bool:
        """Whether ``key`` is ``col``'s member by core §2.2's ranking among
        its interface's templates of its kind token."""
        rows = [r for r in col.contract.canonical["resources"] if r["token"] == col.resource["token"]]
        won = templates.resolve([templates.parse_template(r["template"]) for r in rows],
                                key[len(col.base) + 1:].split("/"))
        return won is not None and won[0].text == col.resource["template"]

    def _snapshot_header(self) -> dict[str, Any]:
        """One answer's run, asof and time (core §4.2, 0.100): "Its run is
        the one the owner writes under"; "An owner MUST write as asof a seq
        of its run at or below which it has recorded every state mutation it
        minted" — this owner mints and records each state mutation under
        the writes' lock, which its caller holds, so the last seq minted is
        such a seq; "A run that has minted none takes one for its first
        snapshot, as an occurrence does". "Its time is the owner's clock as
        it builds the snapshot, minted as a last confirmation is", counted
        from the deployment epoch (the unknown time outside 32 bits)."""
        with self._seq_lock:
            if self.run is None:
                self.take_run()
            minted = self._minted
        if minted is None:
            minted = self._next_inband().seq
        assert self.run is not None
        return {"run_width": self.run[0], "run": self.run[1], "asof": minted, "time": self._time(self.mint())}

    def _time(self, ts: zenoh.Timestamp | None) -> dict[str, int] | None:
        """A timestamp's instant as a snapshot carries it: "to the
        millisecond, counted from the deployment epoch (§7.6)"."""
        if ts is None:
            return None
        return hbeat.time_of(ts.get_time_as_ntp64().as_nanos(), self.writing_epoch())

    def _entry(self, col: Collection, header: dict[str, Any], entry_id: str, key: str, h: _Held
               ) -> tuple[bool, dict[str, Any]] | None:
        """One member as a snapshot lists it (core §4.2 "What a snapshot
        lists", §7.5): a live entry, "the record that mutation put", with its
        confirmation where the collection has a horizon; or a tombstone. Its
        own run where its last mutation was under another than the
        header's."""
        ib = h.inband
        if ib is None:
            return None
        own = (ib.width, ib.run) != (header["run_width"], header["run"])
        e: dict[str, Any] = {"id": entry_id, "run_width": ib.width if own else None, "run": ib.run if own else None,
                             "seq": ib.seq}
        if h.payload is None:
            return True, e
        conf: dict[str, Any] = {"confirmation": "none"}
        if col.horizon:
            # "It is 1 while the owner vouches for the member …, and
            # otherwise 2, with the member's last confirmation".
            if key not in self.unvouched:
                conf = {"confirmation": "time"}
            else:
                conf = {"confirmation": "conf", "conf": self._time(h.confirm or h.stamp)}
        e.update(conf)
        e["record"] = h.payload.hex()
        return False, e

    def _pages(self, col: Collection, header: dict[str, Any], members: list[tuple[str, str, _Held]]) -> list[bytes]:
        """The snapshot's pages, cut "by §7.5's paging rule, under its page
        size" (:func:`zk2py.snapshot.cut`): each member its id, its key and
        what the owner holds of it."""
        snap: dict[str, Any] = {"tag": col.tag, **header, "coll": col.index, "live": [], "tombs": []}
        for entry_id, key, h in members:
            got = self._entry(col, header, entry_id, key, h)
            if got is None:
                continue
            tomb, e = got
            snap["tombs" if tomb else "live"].append(e)
        return snapshot.cut(snap, self.page_size)

    def answer_state(self, query: zenoh.Query, answer: list[tuple[Any, ...]], finish: bool = True) -> None:
        """Send an answer :meth:`state_answer` made, and, for a held query,
        end it (its final reply). A per-key reply carries "the in-band stamp
        of the mutation it represents: a reply as the bytes the mutation
        put, a ``reply_del`` as the delete's attachment", and "the
        mutation's Zenoh timestamp, where the mutation carried one" (S2). A
        snapshot's pages go "as a reply to the one GET", each under
        ``zenoh/bytes``, with no Zenoh timestamp."""
        for item in answer:
            if len(item) == 2:
                item = ("reply", *item)  # an answer made before core 0.100's items
            what, key, x = item
            if what == "snapshot":
                for page in x:
                    query.reply(key, page, encoding=zk2c.ENCODING)
                first = snapshot.read_page(x[0])
                self.snapshots_sent.append((key, first["coll"], first["asof"], len(x)))
                continue
            held = x
            kw: dict[str, Any] = {} if held.stamp is None else {"timestamp": held.stamp}
            if held.payload is not None:
                if held.inband is not None:
                    query.reply(key, held.wire, encoding=zk2c.ENCODING, **kw)
                else:
                    query.reply(key, held.payload, encoding=held.encoding, **kw)
            else:
                if held.inband is not None:
                    kw["attachment"] = held.wire
                query.reply_del(key, **kw)
        if finish:
            query.drop()

    def _operation(self, r: dict[str, Any]) -> Any:
        """``r``'s :class:`zk2py.calls.Operation`, by the contract that
        declares it, its packed ``time`` fields under this owner's epoch."""
        from . import calls

        for c in self.contracts:
            if any(x is r for x in c.canonical["resources"]):
                return calls.Operation.of_resource(r, c.schemas, self.writing_epoch())
        return calls.Operation.of_resource(r, None, self.writing_epoch())

    def mark(self) -> zk2c.InBand:
        """The as-of mark of a reply this owner writes now (core 0.94 §5.1):
        "The run is the one it writes under when it replies (§4.3). The seq
        is the last it minted for state and events under that run, or 0 when
        it has minted none. A reply mints no seq"."""
        with self._seq_lock:
            if self.run is None:
                self.take_run()
            assert self.run is not None
            return zk2c.InBand(self.run[0], self.run[1], 0 if self._minted is None else self._minted)

    def _unavailable_handler(self, r: dict[str, Any], cause: str):
        op = self._operation(r)

        def handle(query: zenoh.Query) -> None:
            asked = str(query.key_expr)
            self.calls.append(asked)
            # §5.1 (0.9) "The order of refusals": fanout_forbidden (O2)
            # first, then unavailable (O3), then a key that names no member.
            if any(templates.is_wild(c) for c in asked.split("/")) and r["fanout"] != "allowed":
                _refuse(query, op, self.mark(), "fanout_forbidden", f"{r['template']} is fanout = \"forbidden\": "
                                                                    "call one concrete key (O2)")
                return
            # O3: "MUST answer a call to an optional operation it does not
            # expose with unavailable and its cause".
            _refuse(query, op, self.mark(), "unavailable", f"{r['template']} is not exposed here", cause=cause)

        return handle

    @staticmethod
    def _read_request(call: "OpCall") -> str | None:
        """Core §5.1, "The request" (0.99): "An owner reads a query under
        zenoh/bytes by §7.5's parse. Under another Encoding a query carries
        no request form … Such a query, a request the parse refuses, and one
        whose record does not decode as the operation's request type are
        refused with invalid_request. The record is read on the contract's
        encoding (§7.2), and a packed one under its CH's tag (§7.6)." Sets
        ``call.request``; returns why it is refused, or None. A protobuf or
        raw record is handed to the handler as its bytes: its decode is the
        handler's."""
        from . import calls, packed

        if call.request_refusal == "encoding":
            return (f"a query under {call.encoding} carries no request form, whatever its bytes: only "
                    f"{calls.FORM_ENCODING} does (core §5.1, \"The request\")")
        if call.request_refusal is not None:
            return f"the request is no request form: {call.request_refusal} (core §7.5)"
        kind = call.op.kind("request")
        if kind not in ("packed", "jsonschema"):
            call.request = call.payload
            return None
        try:
            call.request = call.op.decode("request", call.tag or 0, call.payload)
        except (packed.Refused, calls.Refused) as e:
            return f"the request does not decode as its type: {e.token} (core §5.1, §7.6)"
        return None

    def _op_handler(self, base: str, r: dict[str, Any], member: dict[str, Any] | None = None):
        """The queryable callback for one exposed operation of the interface
        at ``base``: over its template, or, with ``member`` (its values by
        parameter), on that member's concrete key (§5.1 "Over a template").

        Before any handler runs: O2's ``fanout_forbidden``, then, over a
        template, a key whose concrete parameter chunk is not a canonical
        slug is ``invalid_request`` (§5.1, 0.8). Then the operation's
        handler (the caller's, from ``handlers``, or the default below), and
        after it O3: a call left without its answer is ``internal``. Every
        answer is in §7.5's forms (core 0.94), and a request that is no form,
        or whose record does not decode as its type, is ``invalid_request``
        after the three refusals above and before the handler (§5.1)."""
        op = self._operation(r)
        tpl = templates.parse_template(r["template"])
        app = self.handlers.get(f"{r['token']}/{r['template']}")

        def handle(query: zenoh.Query) -> None:
            asked = str(query.key_expr)
            self.calls.append(asked)
            try:
                # O2: a call on a key that is not concrete. §5.1 (0.9) "The
                # order of refusals": fanout_forbidden first, "whatever its
                # other chunks hold"; then a key that names no member. (An
                # operation not exposed has its own queryable, which keeps
                # the same order: _unavailable_handler.)
                if any(templates.is_wild(c) for c in asked.split("/")) and r["fanout"] != "allowed":
                    _refuse(query, op, self.mark(), "fanout_forbidden",
                            f"{r['template']} is fanout = \"forbidden\": call one concrete key (O2)")
                    return
                if member is not None:
                    bound: dict[str, list[str] | None] = {
                        k: [v] if isinstance(v, str) else list(v) for k, v in member.items()}
                elif r["params"]:
                    chunks = asked.split("/")
                    chunks = chunks[chunks.index(r["token"]) + 1:] if r["token"] in chunks else chunks
                    got = templates.bind(tpl, chunks)
                    if got is None:
                        # §5.1 (0.8): "A concrete parameter chunk that is not
                        # a canonical slug (§1.4) names no member … refuses
                        # such a call invalid_request before any handler
                        # runs, fan-out or not".
                        _refuse(query, op, self.mark(), "invalid_request", "the key names no member of the template")
                        return
                    bound = got
                else:
                    bound = {}
                call = OpCall(query, base, r, op, self.mark, bound, asked)
                # a test's capture of each query "as the server receives it"
                # (operations.md §9): its key, its Encoding and its payload
                self.queries.append((asked, call.encoding, call.wire))
                why = self._read_request(call)
                if why is not None:
                    call.refuse("invalid_request", why)
                    return
                if member is not None:
                    call.name(**member)
                self.handled.append(asked)
                try:
                    (app or self._default_op)(call)
                except Exception as e:  # noqa: BLE001 - a handler's bug is internal, never silence
                    call.failed = f"{type(e).__name__}: {e}"
                call.finish()
            finally:
                if self.hold_s > 0:
                    # operations.md §1 (0.8): hold the query open a while
                    # after replying, so a caller that waits for completion
                    # shows it.
                    threading.Timer(self.hold_s, query.drop).start()

        return handle

    def _default_op(self, call: OpCall) -> None:
        """The default handler: a template-wide operation names no member,
        since this owner has none, so a call naming one is ``not_found``
        and a fan-out over the template is ``internal`` (§5.1: "One that
        names no member has no key to reply on, and refuses the call").
        Otherwise a raw operation echoes, a JSON one answers an object, and
        any other is refused ``app`` with no detail (§5.2)."""
        r = call.resource
        if r["params"] and call.member_key is None:
            if all(v is not None for v in call.bound.values()):
                call.refuse("not_found", "no such member")
            else:
                call.refuse("internal", "this server names no member for the call")
            return
        req, resp = r["request"]["kind"], r["response"]["kind"]
        if req == "raw" and resp == "raw":
            call.reply(call.payload)
        elif req == "jsonschema" and resp == "jsonschema":
            # the request is read already (_read_request): an object, or
            # invalid_request
            if not isinstance(call.request, dict):
                call.refuse("invalid_request", "the request does not decode as the request type")
                return
            call.reply_value({"state": "up"})
        else:
            # §5.2 (0.7): any operation may refuse with app; with no error
            # type there is no detail. Since core 0.94 a packed operation
            # type travels in the forms like any other (F-471's answer); the
            # default handler has no value of it to send.
            kinds = sorted({req, resp} - {"raw", "jsonschema"})
            what = " or ".join(kinds) if kinds else f"{req} to {resp}"
            call.refuse("app", f"this owner serves no {what} operation")

    def close(self) -> None:
        # freshness.v1 §2.4: "The re-puts stop when the owner stops". _stop
        # is set under the refresher's lock, so no refresher starts after it.
        with self._refresher_lock:
            self._stop.set()
            refresher = self._refresher
            beater = self._hb_thread
        if refresher is not None and refresher is not threading.current_thread():
            refresher.join(timeout=2.0)
        # freshness.v2 §2.4: the heartbeats stop "when its service closes",
        # before its tokens go.
        self._hb_running.clear()
        if beater is not None and beater is not threading.current_thread():
            beater.join(timeout=2.0)
        # Under _writes (an RLock, which take_over's and step_down's own
        # puts re-enter): a write, a takeover or a step-down in flight ends
        # first, and none begins after, since each checks _stop under it.
        # The lists are taken whole, so no other thread mutates what is
        # undeclared here.
        with self._writes:
            entities, self._entities = list(self._entities), []
            self._instance_entities, self._iface_entities, self._iface_tokens = [], {}, {}
        for e in reversed(entities):
            try:
                e.undeclare()
            except Exception:  # noqa: BLE001 - closing anyway
                pass
        if self.session is not None:
            self.session.close()
            self.session = None

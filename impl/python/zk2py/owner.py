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
  every failure is a ``reply_err`` carrying the §5.2 envelope, and a call
  left without its answer is ``internal`` (O3). An optional operation not
  exposed is answered ``unavailable`` with its cause (O3).

**Bring-up (§8.2).** Step 2 runs first (the order of steps 1 and 2 is free),
so a refusal declares nothing: a required resource not exposed, an optional
one neither exposed nor absent as the descriptor says, and a required role
bound to nothing (§3.2). Then step 1 (resources, and the state values),
step 3 (the descriptor's queryable and its first put, then the bundles'
queryables) and step 4 (the instance token, then the interface tokens).

The session is a zenoh *router* listening on a loopback port, or, with
``connect``, a client of a router: the second is what ``state.md §1`` and
``presence.md §1–§2`` need. Timestamping (the HLC) is enabled, as §4.3
requires of "a session that serves state".
"""

from __future__ import annotations

import json
import secrets
import socket
import threading
import time
from dataclasses import dataclass
from typing import Any

import zenoh

from . import bundle, envelope, templates
from .contract import Contract
from .slug import slug

#: core.md §2.4 / Appendix D names → zenoh's QoS enums.
_CONGESTION = {"block": "BLOCK", "drop": "DROP"}
_PRIORITY = {"real_time": "REAL_TIME", "interactive_high": "INTERACTIVE_HIGH",
             "interactive_low": "INTERACTIVE_LOW", "data_high": "DATA_HIGH", "data": "DATA",
             "data_low": "DATA_LOW", "background": "BACKGROUND"}
#: core.md S3: the tombstone window, unless the deployment configures another.
TOMBSTONE_WINDOW_S = 60.0


class OwnerRefused(RuntimeError):
    """The owner must not start (§3.2, §8.2 step 2)."""


def free_loopback_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


@dataclass
class _Held:
    payload: bytes | None        # None: deleted (a tombstone)
    encoding: str
    stamp: zenoh.Timestamp
    at: float                    # monotonic time of the mutation


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


def _value_encoding(r: dict[str, Any]) -> str:
    """A response value's Encoding (§7.2): a raw type's media type; a JSON
    Schema type's JSON, or CBOR where the resource says so; protobuf's."""
    t = r["response"]
    if t["kind"] == "raw":
        return t["media_type"]
    if t["kind"] == "jsonschema":
        return "application/cbor" if r.get("encoding") == "cbor" else "application/json"
    return "application/protobuf"


def _refuse(query: zenoh.Query, enc: str, code: str, message: str, **kw: Any) -> None:
    query.reply_err(envelope.encode(enc, code, message, **kw), encoding=enc)


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

    def __init__(self, query: zenoh.Query, base: str, r: dict[str, Any], enc: str,
                 bound: dict[str, list[str] | None], key_expr: str):
        self.query, self.base, self.resource, self.enc = query, base, r, enc
        self.bound, self.key_expr = bound, key_expr
        self.payload = b"" if query.payload is None else query.payload.to_bytes()
        self.member_key: str | None = None if r["params"] else f"{base}/{r['token']}/{r['template']}"
        self.values = self.summaries = self.envelopes = 0
        self.refused_to_handler: list[str] = []
        self.failed: str | None = None

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

    def reply(self, payload: bytes, encoding: str | None = None) -> None:
        """A value reply on the named member's key (O3)."""
        if self.member_key is None:
            self.refused_to_handler.append("a value before naming a member")
            raise MemberRefused("name the member this call answers for first")
        if self.resource["replies"] == "one" and (self.values or self.envelopes):
            raise RuntimeError("replies = \"one\": the call is answered already")
        self.query.reply(self.member_key, payload, encoding=encoding or _value_encoding(self.resource))
        self.values += 1

    def summary(self, payload: bytes, encoding: str | None = None) -> None:
        """O6: the one summary reply, its attachment the bytes ``summary``."""
        if self.member_key is None:
            raise MemberRefused("name the member this call answers for first")
        self.query.reply(self.member_key, payload, encoding=encoding or "application/octet-stream",
                         attachment=b"summary")
        self.summaries += 1

    def refuse(self, code: str, message: str, **kw: Any) -> None:
        """A ``reply_err`` carrying the §5.2 envelope."""
        _refuse(self.query, self.enc, code, message, **kw)
        self.envelopes += 1

    def finish(self) -> None:
        """O3: "A handler that ends without replying is answered internal";
        with replies = "many", a declared summary is owed too, "after any
        values it sent". A handler that raised is internal unless it had
        answered a one-reply call. §5.1 (0.9) "Sending nothing needs no
        member": a ``many`` handler that sent nothing, named member or not,
        ends "as zero values then completion, when no summary is declared,
        and is answered internal when one is"."""
        r = self.resource
        if self.envelopes:
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
                 auth: tuple[str, str] | None = None):
        """A router listening on ``port`` (a free loopback port by default),
        or, with ``connect``, a client of that router endpoint.
        - ``bindings``: a role's configured providers (R1); a role left out
          is unbound.
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
          as a client (§11.3)."""
        for c in contracts:
            if not c.valid or c.canonical is None:
                raise ValueError(f"{c.path}: not a valid contract: {c.codes}")
        self.system, self.service, self.contracts = system, service, contracts
        self.connect = connect
        self.bindings = bindings or {}
        self.capabilities = set(capabilities or ())
        self.unavailable = dict(unavailable or {})
        self.withhold = set(withhold or ())
        self.handlers = dict(handlers or {})
        self.members = dict(members or {})
        self.hold_s = hold_s
        self.auth = auth
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
        return plan

    # -- bring-up (§8.2) --------------------------------------------------

    def start(self) -> None:
        plan = self._plan()  # step 2 first: a refusal declares nothing
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
        conf.insert_json5("scouting/multicast/enabled", "false")
        # §4.3: "A session that serves state MUST enable Zenoh's HLC."
        conf.insert_json5("timestamping/enabled", "true")
        self.session = zenoh.open(conf)
        try:
            self._bring_up(plan)
        except Exception:
            self.close()
            raise

    def _bring_up(self, plan: dict[str, list[tuple[dict[str, Any], str]]]) -> None:
        s = self.session
        assert s is not None
        # Step 1: resources, publishers and queryables.
        for c in self.contracts:
            for r, status in plan[c.interface]:
                key = f"{self.prefix(c)}/{r['token']}/{r['template']}"
                rid = f"{r['token']}/{r['template']}"
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
                    pub = s.declare_publisher(
                        key, encoding=enc,
                        congestion_control=getattr(zenoh.CongestionControl, _CONGESTION[r["congestion"]]),
                        priority=getattr(zenoh.Priority, _PRIORITY[r["priority"]]),
                        express=r["express"])
                    self._entities.append(pub)
                    self._publishers[key] = pub
                    self._encodings[key] = enc
            for token in ("state", "@state"):
                self._entities.append(s.declare_queryable(
                    f"{self.prefix(c)}/{token}/**", zenoh.handlers.Callback(self._state_handler),
                    complete=False))
        # The state values held at start, put before the tokens (§8.2 "State
        # values").
        for c in self.contracts:
            for r, status in plan[c.interface]:
                key = f"{self.prefix(c)}/{r['token']}/{r['template']}"
                if status == "exposed" and r["kind"] == "state" and not r["params"] \
                        and r["type"]["kind"] == "raw":
                    self.set_state(key, b"ok")
        # Step 3: the descriptor's queryable and its first put (§3.3), then a
        # contract queryable per interface.
        self.descriptor = self._descriptor(plan)
        self._entities.append(s.declare_queryable(
            self.instance_key, zenoh.handlers.Callback(self._descriptor_handler), complete=True))
        s.put(self.instance_key, self.descriptor, encoding="application/json", timestamp=self.mint())
        for c in self.contracts:
            key = f"zk2/@zk/contract/{c.interface}/{c.fingerprint.removeprefix('sha256:')}"
            data = bundle.build(c)
            self._entities.append(s.declare_queryable(
                key, zenoh.handlers.Callback(lambda q, data=data, key=key: q.reply(
                    key, data, encoding="application/json")), complete=True))
        # Step 4: the instance token, then the interface tokens.
        lv = s.liveliness()
        self._entities.append(lv.declare_token(self.instance_key))
        for c in self.contracts:
            if any(status == "exposed" for _, status in plan[c.interface]):
                fp16 = c.fingerprint.removeprefix("sha256:")[:16]
                self._entities.append(lv.declare_token(
                    f"zk2/{self.system}/{self.service}/@zk/alive/{c.interface}/{self.instance}/{fp16}"))

    def _descriptor(self, plan: dict[str, list[tuple[dict[str, Any], str]]]) -> bytes:
        """§3.3: the record, with `profiles` the union of the `uses`, every
        role listed (an unbound one with empty bindings, an optional one
        with ``optional: true``), `unavailable` the listed resources only
        (exposure is compact), and ``meta.zid`` the session's zid, which an
        owner SHOULD state (0.10): a tool attributes a state stamp by it."""
        assert self.session is not None
        doc = {
            "format": "zk2-descriptor/0.1",
            "service": f"{self.system}/{self.service}",
            "instance": self.instance,
            "interfaces": [{
                "iface": c.interface, "contract": c.fingerprint, "minor": c.minor or 0,
                "token": True,
                "unavailable": [{"resource": f"{r['token']}/{r['template']}",
                                 "cause": self.unavailable[f"{r['token']}/{r['template']}"]}
                                for r, status in plan[c.interface] if status == "listed"],
                "cardinality": {},
            } for c in self.contracts],
            "capabilities": sorted(self.capabilities),
            # 0.10: `optional` is true for a role the instance works without,
            # "and absent otherwise"; for a contract's role "it repeats that
            # contract's [requires]".
            "requires": [{
                "role": role, "interface": req["interface"], "declared_by": c.interface,
                "bindings": list(self.bindings.get(role, [])), "params": {},
                **({"optional": True} if req["optional"] else {}),
            } for c in self.contracts for role, req in c.canonical["requires"].items()],
            "profiles": sorted({u for c in self.contracts for u in c.canonical["uses"]}),
            "meta": {"zid": str(self.session.zid())},
        }
        return json.dumps(doc, separators=(",", ":")).encode()

    # -- state mutations (S1–S3) ------------------------------------------

    def set_state(self, key: str, payload: bytes) -> zenoh.Timestamp:
        """Put a state value, stamped by the owner (S1)."""
        stamp = self.mint()
        enc = self._encodings[key]
        self._publishers[key].put(payload, encoding=enc, timestamp=stamp)
        self._held[key] = _Held(payload, enc, stamp, time.monotonic())
        return stamp

    def delete_state(self, key: str) -> zenoh.Timestamp:
        """Delete a state key, stamped (S1); answered with ``reply_del`` within
        the tombstone window (S3)."""
        stamp = self.mint()
        self._publishers[key].delete(timestamp=stamp)
        self._held[key] = _Held(None, self._encodings[key], stamp, time.monotonic())
        return stamp

    # -- handlers ---------------------------------------------------------

    def _descriptor_handler(self, query: zenoh.Query) -> None:
        # §3.3 "The GET": one reply, application/json, no attachment and no
        # timestamp.
        query.reply(self.instance_key, self.descriptor, encoding="application/json")

    def _state_handler(self, query: zenoh.Query) -> None:
        """S2: "every matching live key, plus a reply_del for each matching
        key it deleted within the window", each with its mutation's
        timestamp (S2, S3)."""
        asked = zenoh.KeyExpr(str(query.key_expr))
        now = time.monotonic()
        for key, held in list(self._held.items()):
            if not asked.intersects(zenoh.KeyExpr(key)):
                continue
            if held.payload is not None:
                query.reply(key, held.payload, encoding=held.encoding, timestamp=held.stamp)
            elif now - held.at <= TOMBSTONE_WINDOW_S:
                query.reply_del(key, timestamp=held.stamp)

    def _unavailable_handler(self, r: dict[str, Any], cause: str):
        enc = envelope.envelope_encoding(r)

        def handle(query: zenoh.Query) -> None:
            asked = str(query.key_expr)
            self.calls.append(asked)
            # §5.1 (0.9) "The order of refusals": fanout_forbidden (O2)
            # first, then unavailable (O3), then a key that names no member.
            if any(templates.is_wild(c) for c in asked.split("/")) and r["fanout"] != "allowed":
                _refuse(query, enc, "fanout_forbidden", f"{r['template']} is fanout = \"forbidden\": "
                                                        "call one concrete key (O2)")
                return
            # O3: "MUST answer a call to an optional operation it does not
            # expose with unavailable and its cause".
            _refuse(query, enc, "unavailable", f"{r['template']} is not exposed here", cause=cause)

        return handle

    def _op_handler(self, base: str, r: dict[str, Any], member: dict[str, Any] | None = None):
        """The queryable callback for one exposed operation of the interface
        at ``base``: over its template, or, with ``member`` (its values by
        parameter), on that member's concrete key (§5.1 "Over a template").

        Before any handler runs: O2's ``fanout_forbidden``, then, over a
        template, a key whose concrete parameter chunk is not a canonical
        slug is ``invalid_request`` (§5.1, 0.8). Then the operation's
        handler (the caller's, from ``handlers``, or the default below), and
        after it O3: a call left without its answer is ``internal``."""
        enc = envelope.envelope_encoding(r)
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
                    _refuse(query, enc, "fanout_forbidden", f"{r['template']} is fanout = \"forbidden\": "
                                                            "call one concrete key (O2)")
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
                        _refuse(query, enc, "invalid_request", "the key names no member of the template")
                        return
                    bound = got
                else:
                    bound = {}
                call = OpCall(query, base, r, enc, bound, asked)
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
            call.reply(call.payload, encoding=r["response"]["media_type"])
        elif req == "jsonschema" and resp == "jsonschema":
            try:
                if not isinstance(json.loads(call.payload or b"null"), dict):
                    raise ValueError("not an object")
            except ValueError:
                call.refuse("invalid_request", "the request does not decode as the request type")
                return
            call.reply(json.dumps({"state": "up"}).encode(), encoding="application/json")
        else:
            # §5.2 (0.7): any operation may refuse with app; with no error
            # type there is no detail.
            call.refuse("app", "this owner decodes no protobuf schema")

    def close(self) -> None:
        for e in reversed(self._entities):
            try:
                e.undeclare()
            except Exception:  # noqa: BLE001 - closing anyway
                pass
        self._entities.clear()
        if self.session is not None:
            self.session.close()
            self.session = None

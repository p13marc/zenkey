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
  template (O1). A call on a key that is not concrete is ``fanout_forbidden``
  unless the operation allows fan-out (O2). On a template, a key that names
  no member is ``invalid_request`` and a well-formed one ``not_found``, since
  this owner has no members (O-4). Otherwise:
  - a ``raw`` request and response echo the request's bytes;
  - a JSON Schema request that does not decode as a JSON object is
    ``invalid_request`` (the decode is the check, O-13), else the answer is
    a JSON value;
  - any other request is refused with ``app``, without a detail since this
    owner declares none (§5.2);
  - every reply goes on the operation's own concrete key, and every failure
    is a ``reply_err`` carrying the §5.2 envelope (O3). An optional
    operation not exposed is answered ``unavailable`` with its cause (O3).

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


class Owner:
    def __init__(self, system: str, service: str, contracts: list[Contract], *, port: int | None = None,
                 connect: str | None = None, bindings: dict[str, list[str]] | None = None,
                 capabilities: set[str] | None = None, unavailable: dict[str, str] | None = None,
                 withhold: set[str] | None = None):
        """A router listening on ``port`` (a free loopback port by default),
        or, with ``connect``, a client of that router endpoint.
        - ``bindings``: a role's configured providers (R1); a role left out
          is unbound.
        - ``capabilities``: the capabilities held (§3.3).
        - ``unavailable``: ``<kind token>/<template>`` → cause, for optional
          resources not exposed and listed so.
        - ``withhold``: resources neither exposed nor listed, which step 2
          refuses (presence.md §2 step 5)."""
        for c in contracts:
            if not c.valid or c.canonical is None:
                raise ValueError(f"{c.path}: not a valid contract: {c.codes}")
        self.system, self.service, self.contracts = system, service, contracts
        self.connect = connect
        self.bindings = bindings or {}
        self.capabilities = set(capabilities or ())
        self.unavailable = dict(unavailable or {})
        self.withhold = set(withhold or ())
        self.port = None if connect else (port or free_loopback_port())
        self.endpoint = connect or f"tcp/127.0.0.1:{self.port}"
        #: core.md §1.2: 64 random bits, 16 lowercase hex digits.
        self.instance = secrets.token_hex(8)
        self.session: zenoh.Session | None = None
        self._last: zenoh.Timestamp | None = None
        self._lock = threading.Lock()
        self._held: dict[str, _Held] = {}
        self._publishers: dict[str, Any] = {}
        self._encodings: dict[str, str] = {}
        self._entities: list[Any] = []
        self.calls: list[str] = []

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
            ts = self.session.new_timestamp()
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
                if r["kind"] == "operation":
                    if status == "exposed":
                        handler = self._op_handler(key, r)
                    else:
                        cause = "capability" if status == "implied" else \
                            self.unavailable[f"{r['token']}/{r['template']}"]
                        handler = self._unavailable_handler(r, cause)
                    self._entities.append(s.declare_queryable(
                        _template_key(f"{self.prefix(c)}/{r['token']}/{r['template']}"),
                        zenoh.handlers.Callback(handler), complete=True))
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
        role listed (an unbound one with empty bindings), and `unavailable`
        the listed resources only (exposure is compact)."""
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
            "requires": [{
                "role": role, "interface": req["interface"], "declared_by": c.interface,
                "bindings": list(self.bindings.get(role, [])), "params": {},
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
            # O3: "MUST answer a call to an optional operation it does not
            # expose with unavailable and its cause".
            query.reply_err(envelope.encode(enc, "unavailable", f"{r['template']} is not exposed here",
                                            cause=cause), encoding=enc)

        return handle

    def _op_handler(self, key: str, r: dict[str, Any]):
        enc = envelope.envelope_encoding(r)
        tpl = templates.parse_template(r["template"])
        prefix_chunks = len(key.split("/")) - len(r["template"].split("/"))

        def refuse(query: zenoh.Query, code: str, message: str) -> None:
            query.reply_err(envelope.encode(enc, code, message), encoding=enc)

        def handle(query: zenoh.Query) -> None:
            asked = str(query.key_expr)
            self.calls.append(asked)
            # O2: a call on a key that is not concrete.
            if "*" in asked and r["fanout"] != "allowed":
                refuse(query, "fanout_forbidden", f"{r['template']} is fanout = \"forbidden\": "
                                                  "call one concrete key (O2)")
                return
            if r["params"]:
                # O-4: a key that names no member is malformed; a well-formed
                # member this owner lacks is not found. It has no members.
                chunks = asked.split("/")[prefix_chunks:]
                if templates.match(tpl, chunks) is None:
                    refuse(query, "invalid_request", "the key names no member of the template")
                else:
                    refuse(query, "not_found", "no such member")
                return
            body = b"" if query.payload is None else query.payload.to_bytes()
            req, resp = r["request"]["kind"], r["response"]["kind"]
            if req == "raw" and resp == "raw":
                query.reply(key, body, encoding=r["response"]["media_type"])
            elif req == "jsonschema" and resp == "jsonschema":
                try:
                    if not isinstance(json.loads(body or b"null"), dict):
                        raise ValueError("not an object")
                except ValueError:
                    refuse(query, "invalid_request", "the request does not decode as the request type")
                    return
                query.reply(key, json.dumps({"state": "up"}).encode(), encoding="application/json")
            else:
                # §5.2 (0.7): any operation may refuse with app; with no
                # error type there is no detail.
                refuse(query, "app", "this owner decodes no protobuf schema")

        return handle

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

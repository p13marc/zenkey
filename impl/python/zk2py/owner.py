"""A minimal zk2 owner on zenoh-python (core.md §2.1, §3.3, §4, §5, §6, §8).

It serves what a contract's parameterless resources need, and nothing more:
- **state** of a ``raw`` type holds the bytes ``ok``, published once with a
  timestamp the owner minted (S1, §4.3), and answered by a non-``complete``
  queryable over the interface's ``state/**`` and ``@state/**`` (S2, §2.1);
- **operations** get a ``complete`` queryable on their concrete key (O1):
  - a call on a key that is not concrete is refused with
    ``fanout_forbidden`` unless the operation declares ``fanout =
    "allowed"`` (O2);
  - a ``raw`` request and response echo the request's bytes;
  - a JSON Schema request is parsed, and answered with a JSON value, or
    refused with ``invalid_request``;
  - any other request is refused with ``app``;
  - every reply goes on the operation's own concrete key, and every failure
    is a ``reply_err`` carrying the §5.2 envelope (O3);
- the **descriptor** (§3.3) and each interface's **bundle** (§8.4) are
  answered by ``complete`` queryables, one reply each, ``application/json``;
- **presence** (§8.1): the instance token, then one interface token per
  interface.

Bring-up follows §8.2's order: resources; the check that every required
resource is exposed (refusal otherwise, and for an unbound required role,
§3.2); the descriptor's and the bundles' queryables; then the tokens.

The session is a zenoh *router* listening on a loopback port, so a client
(the Rust ``consume`` example) can connect to it. Timestamping (the HLC) is
enabled, as §4.3 requires of "a session that serves state".
"""

from __future__ import annotations

import json
import secrets
import socket
import threading
from dataclasses import dataclass
from typing import Any

import zenoh

from . import bundle, envelope
from .contract import Contract

#: core.md §2.4 / Appendix D names → zenoh's QoS enums.
_RELIABILITY = {"reliable": "RELIABLE", "best_effort": "BEST_EFFORT"}
_CONGESTION = {"block": "BLOCK", "drop": "DROP"}
_PRIORITY = {"real_time": "REAL_TIME", "interactive_high": "INTERACTIVE_HIGH",
             "interactive_low": "INTERACTIVE_LOW", "data_high": "DATA_HIGH", "data": "DATA",
             "data_low": "DATA_LOW", "background": "BACKGROUND"}


class OwnerRefused(RuntimeError):
    """The owner must not start (§3.2, §8.2 step 2)."""


def free_loopback_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


@dataclass
class _Held:
    payload: bytes
    encoding: str
    stamp: zenoh.Timestamp


class Owner:
    def __init__(self, system: str, service: str, contracts: list[Contract], *, port: int | None = None,
                 connect: str | None = None, bindings: dict[str, list[str]] | None = None):
        """A router listening on ``port`` (a free loopback port by default),
        or, with ``connect``, a client of that router endpoint. ``bindings``
        maps a role to its configured providers (R1); a role left out is
        unbound."""
        for c in contracts:
            if not c.valid or c.canonical is None:
                raise ValueError(f"{c.path}: not a valid contract: {c.codes}")
        self.system, self.service, self.contracts = system, service, contracts
        self.connect = connect
        self.bindings = bindings or {}
        self.port = None if connect else (port or free_loopback_port())
        self.endpoint = connect or f"tcp/127.0.0.1:{self.port}"
        #: core.md §1.2: 64 random bits, 16 lowercase hex digits.
        self.instance = secrets.token_hex(8)
        self.session: zenoh.Session | None = None
        self._last: zenoh.Timestamp | None = None
        self._lock = threading.Lock()
        self._held: dict[str, _Held] = {}
        self._publishers: dict[str, Any] = {}
        self._entities: list[Any] = []
        self.calls: list[str] = []  # keys of calls served, for a test

    # -- keys -------------------------------------------------------------

    def prefix(self, c: Contract) -> str:
        return f"zk2/{self.system}/{self.service}/{c.interface}"

    @property
    def instance_key(self) -> str:
        return f"zk2/{self.system}/{self.service}/@zk/instance/{self.instance}"

    # -- §4.3 minting -----------------------------------------------------

    def mint(self) -> zenoh.Timestamp:
        """"The greater of Session::new_timestamp() and the last timestamp it
        issued plus one tick, with its session's zid as the id" (§4.3). A
        tick is taken as one nanosecond, NTP64's finest step that zenoh-python
        can build (SPEC-FINDINGS F-66)."""
        assert self.session is not None
        with self._lock:
            ts = self.session.new_timestamp()
            last = self._last
            if last is not None and ts.get_time_as_ntp64().as_nanos() <= last.get_time_as_ntp64().as_nanos():
                n = last.get_time_as_ntp64().as_nanos() + 1
                ts = zenoh.Timestamp(zenoh.NTP64(n // 1_000_000_000, n % 1_000_000_000), ts.get_id())
            self._last = ts
            return ts

    # -- bring-up (§8.2) --------------------------------------------------

    def start(self) -> None:
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
            self._bring_up()
        except Exception:
            self.close()
            raise

    def _bring_up(self) -> None:
        s = self.session
        assert s is not None
        exposed: dict[str, set[str]] = {}
        # 1. Resources: publishers and queryables.
        for c in self.contracts:
            served = exposed.setdefault(c.interface, set())
            for r in c.canonical["resources"]:
                if r["params"]:
                    continue  # templated: not served by this minimal owner
                key = f"{self.prefix(c)}/{r['token']}/{r['template']}"
                if r["kind"] == "state" and r["type"]["kind"] == "raw":
                    pub = s.declare_publisher(
                        key, encoding=r["type"]["media_type"],
                        congestion_control=getattr(zenoh.CongestionControl, _CONGESTION[r["congestion"]]),
                        priority=getattr(zenoh.Priority, _PRIORITY[r["priority"]]),
                        express=r["express"])
                    self._entities.append(pub)
                    self._publishers[key] = pub
                    stamp = self.mint()
                    self._held[key] = _Held(b"ok", r["type"]["media_type"], stamp)
                    served.add(f"{r['token']}/{r['template']}")
                elif r["kind"] == "operation":
                    q = s.declare_queryable(key, zenoh.handlers.Callback(self._op_handler(key, r)),
                                            complete=True)
                    self._entities.append(q)
                    served.add(f"{r['token']}/{r['template']}")
            for token in ("state", "@state"):
                q = s.declare_queryable(f"{self.prefix(c)}/{token}/**",
                                        zenoh.handlers.Callback(self._state_handler), complete=False)
                self._entities.append(q)
        # 2. Every required resource exposed; every required role bound.
        for c in self.contracts:
            for r in c.canonical["resources"]:
                if not r["optional"] and f"{r['token']}/{r['template']}" not in exposed[c.interface]:
                    raise OwnerRefused(f"{c.interface}: required {r['token']}/{r['template']} is not exposed")
            for role, req in c.canonical["requires"].items():
                if not req["optional"] and not self.bindings.get(role):
                    raise OwnerRefused(f"{c.interface}: required role {role!r} is bound to nothing (§3.2)")
        # The state values, put once through their publishers (§2.4's QoS),
        # stamped (S1).
        for key, held in self._held.items():
            self._publishers[key].put(held.payload, encoding=held.encoding, timestamp=held.stamp)
        # 3. The descriptor's queryable, and a contract queryable per interface.
        self.descriptor = self._descriptor(exposed)
        self._entities.append(s.declare_queryable(
            self.instance_key, zenoh.handlers.Callback(self._descriptor_handler), complete=True))
        for c in self.contracts:
            key = f"zk2/@zk/contract/{c.interface}/{c.fingerprint.removeprefix('sha256:')}"
            data = bundle.build(c)
            self._entities.append(s.declare_queryable(
                key, zenoh.handlers.Callback(lambda q, data=data, key=key: q.reply(
                    key, data, encoding="application/json")), complete=True))
        # §3.3 "put on every change", carrying the owner's timestamp.
        s.put(self.instance_key, self.descriptor, encoding="application/json", timestamp=self.mint())
        # 4. The instance token, then the interface tokens.
        lv = s.liveliness()
        self._entities.append(lv.declare_token(self.instance_key))
        for c in self.contracts:
            fp16 = c.fingerprint.removeprefix("sha256:")[:16]
            self._entities.append(lv.declare_token(
                f"zk2/{self.system}/{self.service}/@zk/alive/{c.interface}/{self.instance}/{fp16}"))

    def _descriptor(self, exposed: dict[str, set[str]]) -> bytes:
        """§3.3: the record, with `profiles` the union of the `uses`, every
        role listed (an unbound one with empty bindings), nothing
        unavailable beyond what the missing capabilities imply."""
        assert self.session is not None
        doc = {
            "format": "zk2-descriptor/0.1",
            "service": f"{self.system}/{self.service}",
            "instance": self.instance,
            "interfaces": [{
                "iface": c.interface, "contract": c.fingerprint, "minor": c.minor or 0,
                "token": True, "unavailable": [], "cardinality": {},
            } for c in self.contracts],
            "capabilities": [],
            "requires": [{
                "role": role, "interface": req["interface"], "declared_by": c.interface,
                "bindings": list(self.bindings.get(role, [])), "params": {},
            } for c in self.contracts for role, req in c.canonical["requires"].items()],
            "profiles": sorted({u for c in self.contracts for u in c.canonical["uses"]}),
            "meta": {"zid": str(self.session.zid())},
        }
        return json.dumps(doc, separators=(",", ":")).encode()

    # -- handlers ---------------------------------------------------------

    def _descriptor_handler(self, query: zenoh.Query) -> None:
        # §3.3 "The GET": one reply, application/json, no attachment and no
        # timestamp.
        query.reply(self.instance_key, self.descriptor, encoding="application/json")

    def _state_handler(self, query: zenoh.Query) -> None:
        """S2: "every matching live key", each with "the timestamp of the
        mutation it represents"."""
        asked = zenoh.KeyExpr(str(query.key_expr))
        for key, held in list(self._held.items()):
            if asked.intersects(zenoh.KeyExpr(key)):
                query.reply(key, held.payload, encoding=held.encoding, timestamp=held.stamp)

    def _op_handler(self, key: str, r: dict[str, Any]):
        enc = envelope.envelope_encoding(r)

        def refuse(query: zenoh.Query, code: str, message: str) -> None:
            query.reply_err(envelope.encode(enc, code, message), encoding=enc)

        def handle(query: zenoh.Query) -> None:
            asked = str(query.key_expr)
            self.calls.append(asked)
            # O2: a call on a key that is not concrete.
            if asked != key and r["fanout"] != "allowed":
                refuse(query, "fanout_forbidden", f"{r['template']} is fanout = \"forbidden\": "
                                                  "call one concrete key (O2)")
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
                    refuse(query, "invalid_request", "the request is not a JSON object")
                    return
                query.reply(key, json.dumps({"state": "up"}).encode(), encoding="application/json")
            else:
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

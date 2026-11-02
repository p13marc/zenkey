"""Core §5.1 and §5.2 (0.94, #985 C4c): a call's forms by its operation's
types, the codec beside :mod:`zk2py.zk2c`'s bytes (``conformance/envelope/
calls.json``; ``scenarios/operations.md`` §9).

**What 0.94 adopts.** "A call travels in §7.5's forms, whatever its types'
kind: a JSON Schema, protobuf, raw or packed record is carried alike. A
form's record is encoded as its type says, on the contract's ``encoding``
(§7.2)."
- **The request:** "a caller MUST write its request as §7.5's request,
  ``CH · record``, under the ``Encoding`` ``zenoh/bytes``. CH's width bit is
  0, and its tag is the ``request`` type's" (:func:`request`).
- **The value:** "an owner MUST write each value reply in §7.5's value
  form, under the ``Encoding`` ``zenoh/bytes``, with its as-of mark. CH's
  tag is the ``response`` type's, or the ``summary`` type's on a summary
  reply" (:func:`value`).
- **The error:** "An owner MUST answer a failed call in §7.5's error form,
  under the ``Encoding`` ``zenoh/bytes``, with its as-of mark. CH's tag is
  the operation's ``error`` type's, or 0 when it declares none"
  (:func:`error`). "The detail is the bytes of a value of the operation's
  ``error`` type, encoded as that type says": JSON or CBOR on the
  operation's ``encoding``, a protobuf message, a raw type's own bytes
  ("never base64"), and a packed type's record (:meth:`Operation.encode`).

**The tag.** "Only the ``packed`` kind defines a tag (§7.6)", so every
other kind's is 0 (:meth:`Operation.tag`). A packed record is read "under
its CH's tag (§7.6)", whose first check is that tag; a record of any other
kind is read with no check of the tag, as §7.5's parser "reports the tag of
every form but a delete, and checks none".

**Reading** (:func:`read`): §7.5's parse of the sample's payload, then its
record as the type: a request's as the ``request`` type, a value's as the
``response`` type or the ``summary`` type on a summary reply, an error's
detail as the ``error`` type. "A detail the type refuses leaves the error
the owner's, its detail undecodable": the error is kept, its detail
``{"refused": token}``. "An error with no detail reads no record, so its
tag is not checked" (calls.json's description, and 0.94's entry: "A packed
detail is read under CH's tag, and an error with no detail reads no
record").

**The bare wire** (core §5.1, a MAY): "An implementation MAY read a call's
sample whose ``Encoding`` is not ``zenoh/bytes`` as the bare wire. It tells
the two wires apart by the ``Encoding`` alone". :func:`bare` says which a
sample is, and the readers of :mod:`zk2py.live` and :mod:`zk2py.owner` take
the MAY; "None writes the bare wire", and no writer here does.
"""

from __future__ import annotations

import json
from dataclasses import dataclass
from typing import Any

from . import packed, zk2c

#: An operation's four types, by role (core §5.1, §9.1).
ROLES = ("request", "response", "summary", "error")

#: core §5.1: "a call's sample whose Encoding is not zenoh/bytes" is the
#: bare wire; zk2c's ENCODING, "compared as an exact string".
FORM_ENCODING = zk2c.ENCODING


def bare(encoding: Any) -> bool:
    """Whether a call's sample under ``encoding`` is the bare wire (core
    §5.1: "It tells the two wires apart by the Encoding alone")."""
    return str(encoding) != FORM_ENCODING


class Refused(ValueError):
    """A record its type refuses, with the token of the refusal: §7.6's
    reader's or writer's for a packed type, ``decode`` for a JSON Schema
    record that is not JSON or CBOR, and ``value`` for a value its type
    cannot carry (a detail of bytes for a JSON Schema type, a value for a
    protobuf or raw one: core §5.2, "A detail its type cannot encode")."""

    def __init__(self, token: str, why: str = ""):
        super().__init__(f"{token}: {why}" if why else token)
        self.token = token


@dataclass(frozen=True)
class Operation:
    """One operation's types, as its contract's canonical form states them
    (core §9.5), and the packed record of each packed one.
    - ``types``: role → the canonical type, or None (no ``summary``, no
      ``error``);
    - ``records``: role → the :class:`zk2py.packed.Record`, for a packed
      type only;
    - ``encoding``: ``json`` or ``cbor``, the contract's ``encoding`` that a
      JSON Schema record is read and written on (§7.2), ``json`` where the
      operation states none;
    - ``epoch``: the deployment epoch a packed ``time`` field counts from
      (core §7.6, §8.6), 0 where none is stated."""

    template: str
    types: dict[str, dict[str, Any] | None]
    records: dict[str, Any]
    encoding: str = "json"
    epoch: int = 0

    @classmethod
    def of(cls, contract: Any, template: str, epoch: int | None = None) -> "Operation":
        """The operation ``template`` of a loaded :class:`zk2py.contract.Contract`."""
        for r in contract.canonical["resources"]:
            if r["kind"] == "operation" and r["template"] == template:
                return cls.of_resource(r, contract.schemas, epoch)
        raise KeyError(f"{contract.iface}: no operation {template!r}")

    @classmethod
    def of_resource(cls, r: dict[str, Any], schemas: Any, epoch: int | None = None) -> "Operation":
        """An operation from its canonical resource and its contract's
        :class:`zk2py.schemas.SchemaSet`."""
        types = {role: r.get(role) for role in ROLES}
        records: dict[str, Any] = {}
        for role, t in types.items():
            if t is not None and t.get("kind") == "packed":
                rec = schemas.packed_record(t) if schemas is not None else None
                if rec is None:
                    raise ValueError(f"{r['template']}: packed:{t.get('name')} has no clean record")
                records[role] = rec
        return cls(r["template"], types, records, r.get("encoding") or "json", packed.deployment_epoch(epoch))

    def kind(self, role: str) -> str | None:
        t = self.types.get(role)
        return None if t is None else t["kind"]

    def tag(self, role: str) -> int:
        """CH's tag for ``role``'s form (core §7.5): a packed type's tag, 0
        for every other kind and for an ``error`` the operation does not
        declare ("It is 0 for an operation that declares none")."""
        rec = self.records.get(role)
        return 0 if rec is None else rec.tag

    def encode(self, role: str, value: Any) -> bytes:
        """``value`` as ``role``'s record: a packed value through §7.6's
        writer (:class:`zk2py.packed.Refused` for one with no code); a JSON
        Schema value as JSON, or CBOR on ``encoding = "cbor"``; the bytes of
        a protobuf message or a raw type as they are. Bytes for a JSON Schema
        type, and anything but bytes for a protobuf or raw one, are refused
        ``value`` (core §5.2: "A detail its type cannot encode … MUST NOT be
        sent")."""
        kind = self.kind(role)
        if kind is None:
            raise Refused("value", f"the operation declares no {role} type")
        if kind == "packed":
            return self.records[role].write(value, self.epoch)
        if kind == "jsonschema":
            if isinstance(value, (bytes, bytearray, memoryview)):
                raise Refused("value", "bytes for a JSON Schema type: its value is encoded, never passed as bytes")
            if self.encoding == "cbor":
                from . import cbor

                return cbor.dumps(value)
            return json.dumps(value, separators=(",", ":"), ensure_ascii=False).encode("utf-8")
        if not isinstance(value, (bytes, bytearray, memoryview)):
            raise Refused("value", f"a {kind} type's record is its bytes, never a value")
        return bytes(value)

    def decode(self, role: str, tag: int, record: bytes) -> Any:
        """``record`` read as ``role``'s type, a packed one under ``tag``,
        CH's (core §7.6's reader, :class:`zk2py.packed.Refused`); a JSON
        Schema one on ``encoding`` (``decode`` where it is not strict JSON,
        or not one CBOR item); a protobuf or raw one as its bytes."""
        kind = self.kind(role)
        if kind == "packed":
            return self.records[role].read(tag, record, self.epoch)
        if kind == "jsonschema":
            if self.encoding == "cbor":
                from . import cbor

                try:
                    return cbor.loads(record)
                except cbor.CborError as e:
                    raise Refused("decode", f"not one CBOR item: {e}") from None
            from . import jcs

            try:
                return jcs.loads(record)
            except jcs.JsonError as e:
                raise Refused("decode", f"not strict JSON: {e}") from None
        return bytes(record)


def value_role(summary: bool) -> str:
    """The type a value's record is: the ``summary`` type's on a summary
    reply (O6), else the ``response`` type's."""
    return "summary" if summary else "response"


def request(op: Operation, record: bytes) -> bytes:
    """A request, a query's payload: ``CH · record``, its width bit 0 and
    its tag the ``request`` type's (core §5.1)."""
    return zk2c.request(record, op.tag("request"))


def value(op: Operation, stamp: zk2c.InBand, record: bytes, summary: bool = False) -> bytes:
    """A value, a reply's payload: ``CH · run · seq · record``, ``stamp`` the
    replying instance's as-of mark, the tag the ``response`` type's or the
    ``summary`` type's on a summary reply (core §5.1)."""
    return zk2c.value(stamp, record, op.tag(value_role(summary)))


def error(op: Operation, stamp: zk2c.InBand, code: str, cause: str | None = None, message: str | None = None,
          detail: bytes | None = None) -> bytes:
    """An error, a reply error's payload: ``CH · run · seq · error``, its tag
    the ``error`` type's, 0 where the operation declares none (core §5.2).
    Refuses (:class:`zk2py.zk2c.Refused`) what §7.5's writer refuses,
    ``cause`` then ``detail``, and ``detail`` for a detail on an operation
    that declares no ``error`` type ("an owner MUST NOT put a detail in its
    error"). ``detail`` is the record's bytes (:meth:`Operation.encode`)."""
    zk2c.check_error(code, cause, detail is not None)
    if detail is not None and op.types.get("error") is None:
        raise zk2c.Refused("detail", "the operation declares no error type: an owner has no detail to send (§5.2)")
    return zk2c.write_error(stamp, code, cause, message, detail, op.tag("error"))


# -- calls.json's cases ---------------------------------------------------------------------


def write_case(op: Operation, case: dict[str, Any]) -> bytes:
    """One of ``calls.json``'s ``writes``: its ``form``, its stamp and its
    record (``value``, a packed value; ``record``, a JSON Schema record's
    bytes in hex) or its ``error``. Raises :class:`zk2py.zk2c.Refused` or
    :class:`zk2py.packed.Refused` for the writer's refusal, §7.5's checks on
    the error's byte first, then the detail's record."""
    form = case["form"]

    def record(role: str) -> bytes:
        if "value" in case:
            return op.encode(role, case["value"])
        return bytes.fromhex(case["record"])

    if form == "request":
        return request(op, record("request"))
    st = case["stamp"]
    stamp = zk2c.InBand(st["run_width"], st["run"], st["seq"])
    if form in ("value", "summary"):
        return value(op, stamp, record(value_role(form == "summary")), summary=form == "summary")
    if form != "error":
        raise ValueError(f"no such form: {form!r}")
    e = case["error"]
    d = e.get("detail")
    zk2c.check_error(e["code"], e.get("cause"), d is not None)
    if d is not None and op.types.get("error") is None:
        raise zk2c.Refused("detail", "the operation declares no error type (§5.2)")
    detail = None
    if d is not None:
        detail = op.encode("error", d) if op.kind("error") == "packed" else bytes.fromhex(d)
    return error(op, stamp, e["code"], e.get("cause"), e.get("message"), detail)


def _sample_role(sample: str, summary: bool) -> str:
    if sample == "query":
        return "request"
    if sample == "reply":
        return value_role(summary)
    raise ValueError(sample)


@dataclass(frozen=True)
class Read:
    """A call's sample read (:func:`read`): the parsed form, and its
    record read as its type (``value``), or for an error the error with its
    detail read (``detail``, or ``detail_refusal`` where the type refuses
    it: the error is still the owner's)."""

    env: zk2c.Envelope
    value: Any = None
    detail: Any = None
    detail_refusal: str | None = None

    def expect(self) -> dict[str, Any]:
        """As ``calls.json``'s ``reads`` write it."""
        e = self.env
        out: dict[str, Any] = {"run_width": e.width, "run": e.run, "seq": e.seq, "tag": e.tag}
        if e.error is not None:
            x = e.error
            det: Any = None
            if x.detail is not None:
                det = {"refused": self.detail_refusal} if self.detail_refusal is not None else self.detail
            out["error"] = {"code": x.code, "cause": x.cause, "message": x.message, "detail": det}
        else:
            out["value"] = self.value
        return out


def _shown(op: Operation, role: str, v: Any) -> Any:
    """A record's reading as the fixtures show it: a packed or JSON value
    as read, a protobuf or raw record's bytes in hex."""
    return v.hex() if op.kind(role) in ("protobuf", "raw") and isinstance(v, bytes) else v


def read(op: Operation, sample: str, payload: bytes, summary: bool = False) -> Read:
    """``payload`` of a ``query``, a ``reply`` or a ``reply_err`` read as
    ``op``'s form: §7.5's parse (:class:`zk2py.zk2c.Refused`), then the
    record as its type under CH's tag (:class:`zk2py.packed.Refused` or
    :class:`Refused`). An error's detail its type refuses is kept as
    ``detail_refusal``, never raised; an error with no detail reads no
    record. The ``Encoding`` is the caller's to check first (:func:`bare`)."""
    env = zk2c.parse(payload, sample)
    if env.error is not None:
        if env.error.detail is None or op.types.get("error") is None:
            return Read(env, detail=None if env.error.detail is None else env.error.detail.hex())
        try:
            return Read(env, detail=_shown(op, "error", op.decode("error", env.tag, env.error.detail)))
        except (packed.Refused, Refused) as e:
            return Read(env, detail_refusal=e.token)
    role = _sample_role(sample, summary)
    return Read(env, value=_shown(op, role, op.decode(role, env.tag, env.record or b"")))


def read_case(op: Operation, case: dict[str, Any]) -> dict[str, Any]:
    """One of ``calls.json``'s ``reads``: the stamp, the tag and the record
    read, or ``{"refused": token}``, §7.5's first, then §7.6's."""
    try:
        return read(op, case["sample"], bytes.fromhex(case["hex"]), bool(case.get("summary"))).expect()
    except (zk2c.Refused, packed.Refused, Refused) as e:
        return {"refused": e.token}

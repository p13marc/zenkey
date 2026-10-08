"""The error envelope decoder (core.md §5.2).

"A tool decodes an envelope by its reply's Encoding, without the contract.
It MUST refuse: an unknown encoding (``encoding``); malformed bytes, a
duplicate or unknown member, or a missing ``code`` or ``message``
(``decode``); an unknown code, the empty string included (``code``);
``unavailable`` without a valid cause, or a cause on any other code
(``cause``); a detail on any code but ``app`` (``detail``). A protobuf
decoder ignores unknown fields, as protobuf does."
"""

from __future__ import annotations

import base64
from typing import Any

from . import cbor, jcs

CODES = ("invalid_request", "not_found", "unavailable", "forbidden", "fanout_forbidden",
         "busy", "internal", "app")
CAUSES = ("build", "config", "capability")
MEMBERS = {"code", "message", "cause", "detail"}

JSON_ENCODING = "application/json"
CBOR_ENCODING = "application/cbor"
#: §5.2: ``application/protobuf`` "with the schema suffix zk2.core.v1.Error",
#: in Zenoh's ``<encoding>;<schema>`` text form.
PROTOBUF_ENCODING = "application/protobuf;zk2.core.v1.Error"


class EnvelopeError(ValueError):
    def __init__(self, tag: str, message: str = ""):
        super().__init__(f"{tag}: {message}" if message else tag)
        self.tag = tag


def decode(encoding: str, payload: bytes) -> dict[str, Any]:
    """Decode one envelope into ``{code, message, cause, detail}``, raising
    :class:`EnvelopeError` with the refusal tag. A protobuf ``detail`` is
    ``{"bytes_hex": …}``, as the fixtures show it; a CBOR byte string inside
    a detail is shown the same way."""
    if encoding == JSON_ENCODING:
        env = _from_document(_json(payload))
    elif encoding == CBOR_ENCODING:
        env = _from_document(_cbor(payload))
    elif encoding == PROTOBUF_ENCODING:
        env = _protobuf(payload)
    else:
        # §5.2 "Transport errors": only these three encodings carry an
        # envelope; any other reply error is the transport's.
        raise EnvelopeError("encoding", encoding)
    _validate(env)
    return env


def _json(payload: bytes) -> Any:
    try:
        return jcs.loads(payload)
    except jcs.JsonError as e:
        raise EnvelopeError("decode", str(e)) from e


def _cbor(payload: bytes) -> Any:
    """§5.2 (0.5), CBOR: one data item and nothing after it; indefinite
    lengths accepted; a tag decodes as its content, ``undefined`` as null;
    "A map key that is not text, an integer outside 64 bits, or a float that
    is not finite is decode." Applied at any depth of the item."""
    try:
        doc = cbor.loads(payload)
    except cbor.CborError as e:
        raise EnvelopeError("decode", str(e)) from e
    _cbor_domain(doc)
    return doc


def _cbor_domain(v: Any) -> None:
    if isinstance(v, dict):
        for k, x in v.items():
            if not isinstance(k, str):
                raise EnvelopeError("decode", f"a map key that is not text: {k!r}")
            _cbor_domain(x)
    elif isinstance(v, list):
        for x in v:
            _cbor_domain(x)
    elif isinstance(v, bool) or v is None:
        pass
    elif isinstance(v, int):
        # "Outside 64 bits": outside both i64 and u64, -2^63 to 2^64-1
        # (SPEC-FINDINGS F-58). CBOR's major type 0 never exceeds 2^64-1.
        if not -(2**63) <= v <= 2**64 - 1:
            raise EnvelopeError("decode", f"an integer outside 64 bits: {v}")
    elif isinstance(v, float):
        if v != v or v in (float("inf"), float("-inf")):
            raise EnvelopeError("decode", "a float that is not finite")


def _bytes_view(v: Any) -> Any:
    """§5.2 (0.5): "A byte string inside a detail reads as base64 text (RFC
    4648 §4, padded), the JSON form of bytes (§7.2)"."""
    if isinstance(v, bytes):
        return base64.b64encode(v).decode("ascii")
    if isinstance(v, list):
        return [_bytes_view(x) for x in v]
    if isinstance(v, dict):
        return {k: _bytes_view(x) for k, x in v.items()}
    return v


def _from_document(doc: Any) -> dict[str, Any]:
    """The JSON and CBOR envelope: an object with text keys, only the four
    members, ``code`` and ``message`` required and strings."""
    if not isinstance(doc, dict) or not all(isinstance(k, str) for k in doc):
        raise EnvelopeError("decode", "not an object with text keys")
    unknown = set(doc) - MEMBERS
    if unknown:
        raise EnvelopeError("decode", f"unknown members {sorted(unknown)}")
    for m in ("code", "message"):
        if m not in doc:
            raise EnvelopeError("decode", f"missing {m}")
        if not isinstance(doc[m], str):
            raise EnvelopeError("decode", f"{m} is not a string")
    cause = doc.get("cause")
    if cause is not None and not isinstance(cause, str):
        raise EnvelopeError("decode", "cause is not a string or null")
    return {"code": doc["code"], "message": doc["message"], "cause": cause,
            "detail": _bytes_view(doc.get("detail"))}


def _varint(data: bytes, pos: int) -> tuple[int, int]:
    value = shift = 0
    while True:
        if pos >= len(data):
            raise EnvelopeError("decode", "truncated varint")
        b = data[pos]
        pos += 1
        value |= (b & 0x7F) << shift
        if not b & 0x80:
            return value, pos
        shift += 7
        if shift >= 64:
            raise EnvelopeError("decode", "varint too long")


def _skip(data: bytes, pos: int, wire: int, field: int) -> int:
    if wire == 0:
        return _varint(data, pos)[1]
    if wire == 1:
        pos += 8
    elif wire == 5:
        pos += 4
    elif wire == 2:
        n, pos = _varint(data, pos)
        pos += n
    elif wire == 3:
        # A group: skip until its matching end-group tag.
        while True:
            key, pos = _varint(data, pos)
            if key & 7 == 4:
                if key >> 3 != field:
                    raise EnvelopeError("decode", "mismatched end group")
                return pos
            pos = _skip(data, pos, key & 7, key >> 3)
    else:
        raise EnvelopeError("decode", f"wire type {wire}")
    if pos > len(data):
        raise EnvelopeError("decode", "truncated field")
    return pos


def _protobuf(data: bytes) -> dict[str, Any]:
    """``zk2.core.v1.Error`` (core/error.proto): proto3, ``code`` = 1,
    ``message`` = 2, ``optional cause`` = 3, ``optional detail`` = 4.
    Missing ``code``/``message`` decode as ``""`` (proto3 defaults)."""
    fields: dict[int, bytes] = {}
    pos = 0
    while pos < len(data):
        key, pos = _varint(data, pos)
        number, wire = key >> 3, key & 7
        if number == 0:
            raise EnvelopeError("decode", "field number 0")
        if number in (1, 2, 3, 4):
            if wire != 2:
                raise EnvelopeError("decode", f"field {number} has wire type {wire}")
            n, pos = _varint(data, pos)
            if pos + n > len(data):
                raise EnvelopeError("decode", f"field {number} is truncated")
            fields[number] = data[pos:pos + n]  # the last occurrence wins
            pos += n
        else:
            pos = _skip(data, pos, wire, number)
    text: dict[int, str] = {}
    for n in (1, 2, 3):
        if n in fields:
            try:
                text[n] = fields[n].decode("utf-8")
            except UnicodeDecodeError as e:
                raise EnvelopeError("decode", f"field {n} is not UTF-8") from e
    return {
        "code": text.get(1, ""),
        "message": text.get(2, ""),
        "cause": text.get(3),
        "detail": {"bytes_hex": fields[4].hex()} if 4 in fields else None,
    }


def _validate(env: dict[str, Any]) -> None:
    if env["code"] not in CODES:
        raise EnvelopeError("code", repr(env["code"]))
    if env["code"] == "unavailable":
        if env["cause"] not in CAUSES:
            raise EnvelopeError("cause", f"unavailable needs a cause, got {env['cause']!r}")
    elif env["cause"] is not None:
        raise EnvelopeError("cause", f"a cause on {env['code']}")
    if env["detail"] is not None and env["code"] != "app":
        raise EnvelopeError("detail", f"a detail on {env['code']}")

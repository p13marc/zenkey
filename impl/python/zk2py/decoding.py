"""Observer §7.3's decode order as zk2py's tools take it (#609, the mend
after #926 part 5; ``observer.md`` text 0.12).

The text, as 0.12 states it: "a payload whose declared type the tool holds
is decoded by that type, on the wire its sample's `Encoding` names (below).
Any other payload is sniffed." And of the payloads sniffed: "A payload
whose declared type the tool does not hold is sniffed (§8.1). That is a
foreign key's, or a zk2 key's whose contract is not in hand or is
unavailable (§2). The tool says which, naming where it stopped resolving
the key."

- **A held JSON Schema type** is read on the wire its sample's ``Encoding``
  names: JSON or CBOR (core §7.2). An ``Encoding`` that names neither,
  zenoh's default ``zenoh/bytes`` or another type's, does not pick the
  wire, and the type is read on its contract's ``encoding``
  (:func:`wire_of`, :func:`decode_json_type`). ``config.v1``'s reader
  (:func:`zk2py.config.read`) reads its view, status and events so, since
  its profile does not say how its reader reads a sample's bytes.
- **A profile's reader** decides for itself where its profile says how it
  reads a sample's bytes (observer §7.3, 0.12, F-409): ``telemetry.v1``'s
  on its resource's ``encoding`` (:func:`zk2py.telemetry.decode`), and
  ``alarms.v1``'s and ``desired.v1``'s reports as strict JSON
  (:func:`zk2py.alarms.decode`, :mod:`zk2py.desired`). Neither reads the
  sample's ``Encoding``.
- **Any other payload is sniffed** (:func:`sniff`): a structural rendering
  of its bytes, never its declared type, with what the tool could not
  establish (:func:`unheld`, observer §8.1): a foreign key, or a zk2 key
  whose contract the tool does not hold, and the step where it stopped
  resolving the key. zk2py's echo tool holds no contract, so it sniffs
  every payload (:mod:`zk2py.echo_tool`).
"""

from __future__ import annotations

from typing import Any

from . import cbor, jcs, keys

#: zenoh's predefined encodings for JSON and CBOR (core §7.2): the names a
#: sample's ``Encoding`` gives the two wires of a JSON Schema type.
JSON_ENCODINGS = ("application/json", "text/json")
CBOR_ENCODINGS = ("application/cbor",)

#: How much of a payload a sniffed rendering shows as text or hex.
SHOWN_BYTES = 256


def named(encoding: str | None) -> str:
    """An ``Encoding``'s name, the part of its text before any ``;`` and
    the suffix after it (core §7.2: an owner adds no schema suffix but on
    the error envelope)."""
    return (encoding or "").split(";", 1)[0].strip()


def wire_of(encoding: str | None, declared: str | None = "json") -> str:
    """The wire a held JSON Schema type is read on (observer §7.3): JSON or
    CBOR as its sample's ``Encoding`` names it, else its contract's
    ``encoding`` (``json`` by default, core §7.2)."""
    n = named(encoding)
    if n in JSON_ENCODINGS:
        return "json"
    if n in CBOR_ENCODINGS:
        return "cbor"
    return "cbor" if declared == "cbor" else "json"


class Undecodable(ValueError):
    """A payload that does not decode as its held type, on the wire it was
    read from: honest rendering's case (observer §7.3), never sniffed."""

    def __init__(self, wire: str, reason: str) -> None:
        super().__init__(f"read as {wire.upper()}: {reason}")
        self.wire = wire
        self.reason = reason


def decode_json_type(payload: bytes, encoding: str | None, declared: str | None = "json") -> Any:
    """A held JSON Schema type's payload decoded on the wire its sample's
    ``Encoding`` names (:func:`wire_of`): strict JSON (:func:`zk2py.jcs.loads`)
    or one CBOR item (:func:`zk2py.cbor.loads`). Raises :class:`Undecodable`,
    naming the wire it was read from."""
    wire = wire_of(encoding, declared)
    try:
        return jcs.loads(payload) if wire == "json" else cbor.loads(payload)
    except (ValueError, RecursionError) as e:
        raise Undecodable(wire, str(e) or type(e).__name__) from e


def _plain(v: Any) -> Any:
    """A CBOR item as JSON can show it: bytes as ``h'…'`` text, a key that
    is not text as its spelling, a non-finite float as its name."""
    if isinstance(v, bytes):
        return f"h'{v.hex()}'"
    if isinstance(v, float) and (v != v or v in (float("inf"), float("-inf"))):
        return repr(v)
    if isinstance(v, dict):
        return {(k if isinstance(k, str) else repr(_plain(k))): _plain(x) for k, x in v.items()}
    if isinstance(v, list):
        return [_plain(x) for x in v]
    return v


def _text(payload: bytes) -> str | None:
    try:
        t = payload.decode("utf-8")
    except UnicodeDecodeError:
        return None
    return t if all(c.isprintable() or c in "\t\r\n" for c in t) else None


def sniff(payload: bytes, encoding: str | None = None) -> dict[str, Any]:
    """A structural rendering of a payload whose declared type the tool does
    not hold (observer §7.3, §8.1): what its bytes are, never a type.

    In order: nothing; strict JSON; one CBOR item, tried first when the
    sample's ``Encoding`` names CBOR, and otherwise only when it is a map
    or an array, so that a byte or two of text is never read as a small
    integer; UTF-8 text of printable characters; else hexadecimal. Text
    and hexadecimal show at most :data:`SHOWN_BYTES` bytes and say so."""
    b = bytes(payload)
    if not b:
        return {"as": "empty"}
    n = named(encoding)

    def as_cbor(structural: bool) -> dict[str, Any] | None:
        try:
            v = cbor.loads(b)
        except (ValueError, RecursionError):
            return None
        if structural and not isinstance(v, (dict, list)):
            return None
        return {"as": "cbor", "value": _plain(v)}

    if n in CBOR_ENCODINGS:
        got = as_cbor(False)
        if got is not None:
            return got
    try:
        return {"as": "json", "value": jcs.loads(b)}
    except (ValueError, RecursionError):
        pass
    got = as_cbor(True)
    if got is not None:
        return got
    text = _text(b)
    if text is not None:
        # Cut at a character, never inside one: ``shown`` is the bytes the
        # text shows, at most SHOWN_BYTES (review-bot on PR #994).
        value = text if len(b) <= SHOWN_BYTES else b[:SHOWN_BYTES].decode("utf-8", "ignore")
        out: dict[str, Any] = {"as": "text", "value": value}
        shown = len(value.encode("utf-8"))
    else:
        out = {"as": "hex", "value": b[:SHOWN_BYTES].hex()}
        shown = min(len(b), SHOWN_BYTES)
    if len(b) > SHOWN_BYTES:
        out["shown"] = shown
    return out


#: :func:`unheld`'s answers: which payload whose declared type the tool
#: does not hold this is (observer §7.3, 0.12).
FOREIGN = "foreign"
NOT_IN_HAND = "contract_not_in_hand"
UNAVAILABLE = "contract_unavailable"
NO_CONTRACT_TYPE = "no_contract_type"


def unheld(key: str, *, unavailable: bool = False) -> dict[str, Any]:
    """What a tool says of a payload whose declared type it does not hold
    (observer §7.3, 0.12): which case it is, and the step where it stopped
    resolving the key, as ``declared_type``'s fields: ``held`` false,
    ``which``, ``stopped`` and ``says``.

    - **A foreign key** (``foreign``): the key does not parse as zk2 (core
      §1.1), so resolution stops at the grammar. A fact, not an error
      (observer §8.1).
    - **A zk2 data key whose contract is not in hand**
      (``contract_not_in_hand``): resolution stops at the contract. zk2py's
      echo tool reads no descriptor and holds no contract, so the revision
      of the key's interface that declares its type is never in hand.
    - **A zk2 data key whose contract is unavailable**
      (``contract_unavailable``, with ``unavailable``): the tool asked, and
      no holder served the bundle, or it failed verification. That is
      "contract unavailable", never "no contract" (observer §2; core §8.4).
    - **A zk2 key of a control form** (``no_contract_type``; core §1.1's
      ``instance``, ``alive``, ``member`` or ``contract``): no interface's
      contract declares its payload, so resolution stops at the key's form.

    It reads the key alone, and retrieves nothing."""
    p = keys.parse(key)
    if p is None:
        return {"held": False, "which": FOREIGN, "stopped": "grammar",
                "says": "not a zk2 key (core §1.1): no declared type, so a payload on it is sniffed (observer "
                        "§7.3, §8.1)"}
    if p.get("form") != "data":
        return {"held": False, "which": NO_CONTRACT_TYPE, "stopped": "form",
                "says": f"a zk2 {p['form']} key (core §1.1), whose payload no interface's contract declares, so a "
                        "payload on it is sniffed (observer §7.3)"}
    where = f"{p['iface']}'s {p['token']}/{'/'.join(p['resource'])}"
    if unavailable:
        return {"held": False, "which": UNAVAILABLE, "stopped": "contract", "iface": p["iface"],
                "says": f"a zk2 key: the contract declaring {where} is unavailable (observer §2; core §8.4), so a "
                        "payload on it is sniffed, not decoded by its declared type (observer §7.3)"}
    return {"held": False, "which": NOT_IN_HAND, "stopped": "contract", "iface": p["iface"],
            "says": f"a zk2 key: the contract declaring {where} is not in hand (this tool reads no descriptor and "
                    "holds no contract), so a payload on it is sniffed, not decoded by its declared type "
                    "(observer §7.3)"}

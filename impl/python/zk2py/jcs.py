"""Strict JSON reading, RFC 8785 (JCS) bytes, and sha256 ids.

core.md §9.5: the canonical bytes are the RFC 8785 serialization, and a
fingerprint is ``sha256:`` + the lowercase hex sha256 of those bytes. §9.6:
a bundle is JSON "with no duplicate member", and a schema or extra is
identified by the sha256 of its JCS bytes.
"""

from __future__ import annotations

import hashlib
import json
from typing import Any

import rfc8785

#: core.md §9.5: every integer is within ±(2^53−1).
INT_MAX = 2**53 - 1


class JsonError(ValueError):
    """Bytes that are not strict JSON: malformed, a duplicate member, a
    non-finite number, or not UTF-8."""


def _no_duplicates(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    out: dict[str, Any] = {}
    for k, v in pairs:
        if k in out:
            raise JsonError(f"duplicate member {k!r}")
        out[k] = v
    return out


def _no_constant(name: str) -> Any:
    raise JsonError(f"{name} is not JSON")


def loads(data: bytes | str) -> Any:
    """Parse JSON strictly: RFC 8259 syntax, no duplicate member, no
    NaN/Infinity. Raises :class:`JsonError`."""
    if isinstance(data, bytes):
        try:
            data = data.decode("utf-8")
        except UnicodeDecodeError as e:
            raise JsonError(f"not UTF-8: {e}") from e
    try:
        value = json.loads(data, object_pairs_hook=_no_duplicates, parse_constant=_no_constant)
    except JsonError:
        raise
    except ValueError as e:
        raise JsonError(str(e)) from e
    _check_finite(value)
    return value


def _check_finite(value: Any) -> None:
    # json.loads turns an overflowing literal such as 1e400 into inf.
    if isinstance(value, float) and (value != value or value in (float("inf"), float("-inf"))):
        raise JsonError("number out of the double range")
    if isinstance(value, dict):
        for v in value.values():
            _check_finite(v)
    elif isinstance(value, list):
        for v in value:
            _check_finite(v)


def dumps(value: Any) -> bytes:
    """RFC 8785 bytes. Raises ``rfc8785.CanonicalizationError`` (an integer
    outside ±(2^53−1), a non-finite float, a non-JSON value)."""
    return rfc8785.dumps(value)


def sha256_hex(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def sha256_id(data: bytes) -> str:
    """``sha256:`` + 64 lowercase hex digits (core.md §1.2, §9.5)."""
    return "sha256:" + sha256_hex(data)


def jcs_id(value: Any) -> str:
    return sha256_id(dumps(value))


def is_json_integer(value: Any) -> bool:
    """A JSON integer. Python's bool is an int subclass; it is not one."""
    return isinstance(value, int) and not isinstance(value, bool)


def restriction_violations(value: Any) -> tuple[int, int]:
    """Count §9.5's restriction violations in a JSON value: strings (keys
    included) outside printable ASCII 0x20–0x7E (E027), and integers outside
    ±(2^53−1) (E028).

    A float whose JCS form is an integer literal beyond the bound (``1e16``
    serializes as ``10000000000000000``) is counted as an integer too: once
    serialized, every reader parses it as one, and a bundle verifier would
    refuse it with ``restrictions``. See SPEC-FINDINGS F-21.
    """
    ascii_bad = 0
    int_bad = 0

    def bad_string(s: str) -> bool:
        return any(not (0x20 <= ord(c) <= 0x7E) for c in s)

    def walk(v: Any) -> None:
        nonlocal ascii_bad, int_bad
        if isinstance(v, str):
            ascii_bad += bad_string(v)
        elif isinstance(v, bool) or v is None:
            pass
        elif isinstance(v, int):
            int_bad += abs(v) > INT_MAX
        elif isinstance(v, float):
            if v.is_integer() and abs(v) > INT_MAX and abs(v) < 1e21:
                # ECMAScript renders integral doubles below 1e21 as integer
                # literals; at or above 1e21 they become exponent forms.
                int_bad += 1
        elif isinstance(v, dict):
            for k, x in v.items():
                ascii_bad += bad_string(k)
                walk(x)
        elif isinstance(v, list):
            for x in v:
                walk(x)

    walk(value)
    return ascii_bad, int_bad

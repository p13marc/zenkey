"""Lexical rules (core.md §1.2), shared by keys, templates, contracts and descriptors."""

from __future__ import annotations

import re
from dataclasses import dataclass

#: core.md §1.2: "A plain chunk is ``[a-z0-9]([a-z0-9._-]*[a-z0-9])?``."
PLAIN_CHUNK = re.compile(r"[a-z0-9](?:[a-z0-9._-]*[a-z0-9])?")

#: core.md §1.2: an interface name segment is ``[a-z][a-z0-9_]*``.
NAME_SEGMENT = re.compile(r"[a-z][a-z0-9_]*")

#: core.md §1.2: the last name segment "is not ``v`` followed by digits".
VERSION_SEGMENT = re.compile(r"v[0-9]+")

#: core.md §1.2: ``<major>`` is decimal, 0 to 2^32-1, without leading zeros.
MAJOR = re.compile(r"0|[1-9][0-9]*")
MAJOR_MAX = 2**32 - 1

#: core.md §1.2: an instance id is 16 lowercase hex digits.
INSTANCE_ID = re.compile(r"[0-9a-f]{16}")

#: core.md §1.2: a fingerprint is ``sha256:`` + 64 lowercase hex digits.
HEX64 = re.compile(r"[0-9a-f]{64}")
FINGERPRINT = re.compile(r"sha256:[0-9a-f]{64}")

#: core.md §1.2: a ULID chunk is 26 characters of Crockford's base32 in
#: lowercase: digits, and lowercase letters except i, l, o, u.
#: (The spec does not restrict the first character to 0-7, the 128-bit
#: bound of a real ULID; see SPEC-FINDINGS F-01.)
ULID = re.compile(r"[0-9a-hjkmnp-tv-z]{26}")

#: core.md §2.2: a parameter name is ``[a-z][a-z0-9_]*``; §9.2 E030 uses the
#: same pattern for a role, and the descriptor schema for a role and a param.
IDENT = re.compile(r"[a-z][a-z0-9_]*")

#: core.md §2.3: a gate's ``<n>`` is ``[a-z0-9][a-z0-9_.-]*``.
GATE_NAME = re.compile(r"[a-z0-9][a-z0-9_.-]*")


def is_plain_chunk(s: str) -> bool:
    return PLAIN_CHUNK.fullmatch(s) is not None


@dataclass(frozen=True)
class InterfaceId:
    """``<name>.v<major>`` (core.md §1.2)."""

    name: str
    major: int

    def __str__(self) -> str:
        return f"{self.name}.v{self.major}"


def is_interface_name(name: str) -> bool:
    """core.md §1.2 / §9.2 E001: one or more ``.``-joined ``[a-z][a-z0-9_]*``
    segments, whose last segment is not ``v<digits>``."""
    segments = name.split(".")
    if not all(NAME_SEGMENT.fullmatch(s) for s in segments):
        return False
    return VERSION_SEGMENT.fullmatch(segments[-1]) is None


def parse_interface_id(s: str) -> InterfaceId | None:
    """Parse ``<name>.v<major>``, or return None (core.md §1.2).

    The major is the text after the *last* ``.v``; the name before it must
    then pass :func:`is_interface_name`. So ``nav.v1.v2`` is refused (its name
    ``nav.v1`` ends in a version segment) and ``nav.v02`` is refused (leading
    zero), as ``keys.json`` expects.
    """
    head, sep, major = s.rpartition(".v")
    if not sep or not MAJOR.fullmatch(major) or not is_interface_name(head):
        return None
    value = int(major)
    if value > MAJOR_MAX:
        return None
    return InterfaceId(head, value)


def is_interface_id(s: str) -> bool:
    return parse_interface_id(s) is not None

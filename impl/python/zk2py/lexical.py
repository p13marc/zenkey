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

#: core.md §1.2 (0.101): Crockford's base32 in lowercase, "the digits, and
#: the lowercase letters but ``i``, ``l``, ``o`` and ``u``", in value order.
CROCKFORD = "0123456789abcdefghjkmnpqrstvwxyz"

#: core.md §1.2 (0.103): "A key whose last chunk is not one [an occurrence
#: chunk], a 26-character ULID's among them, is not a zk2 key." The ULID
#: chunk's definition is gone (C2d); this shape is kept only so that a reader
#: can name what it refuses (``keys.json``'s three ULID cases).
ULID = re.compile(r"[0-9a-hjkmnp-tv-z]{26}")

#: core.md §1.2 (0.101): "An occurrence chunk (events, §2.6) is a run and a
#: seq (§4.3), the run's bits then the seq's 24 … A 16-bit run's chunk is 40
#: bits, in 8 characters. A 32-bit run's chunk is 56 bits, in 12 characters,
#: the first ``0`` or ``1``."
OCCURRENCE = re.compile(r"[0-9a-hjkmnp-tv-z]{8}|[01][0-9a-hjkmnp-tv-z]{11}")

#: the seq's bits in an occurrence chunk (core §4.3: "an unsigned 24-bit
#: integer")
SEQ_BITS = 24


@dataclass(frozen=True)
class Occurrence:
    """An occurrence chunk's stamp (core §1.2): the run's width in bits, 16
    or 32, the run and the seq. ``keys.json`` writes it ``{run_width, run,
    seq}``."""

    run_width: int
    run: int
    seq: int

    def as_json(self) -> dict[str, int]:
        return {"run_width": self.run_width, "run": self.run, "seq": self.seq}


def parse_occurrence(chunk: str) -> Occurrence | None:
    """An occurrence chunk's run and seq, or None for a chunk that is none
    (core §1.2): "The decode is exact and reads no contract." Its width is
    its length's: 8 characters a 16-bit run's, 12 a 32-bit run's."""
    if OCCURRENCE.fullmatch(chunk) is None:
        return None
    value = 0
    for ch in chunk:
        value = value * 32 + CROCKFORD.index(ch)
    width = 16 if len(chunk) == 8 else 32
    return Occurrence(width, value >> SEQ_BITS, value & ((1 << SEQ_BITS) - 1))


def occurrence_chunk(run_width: int, run: int, seq: int) -> str:
    """The occurrence chunk of ``run`` and ``seq`` (core §1.2): "the run's
    bits then the seq's 24, in Crockford's base32 in lowercase … most
    significant first, and zero-filled at the top", 8 characters for a
    16-bit run and 12 for a 32-bit run. Refuses a run or a seq out of its
    field."""
    if run_width not in (16, 32):
        raise ValueError("a run is 16 or 32 bits")
    if not 0 <= run < 2**run_width:
        raise ValueError(f"run {run} does not fit {run_width} bits")
    if not 0 <= seq < 2**SEQ_BITS:
        raise ValueError(f"seq {seq} does not fit 24 bits")
    value = (run << SEQ_BITS) | seq
    n = 8 if run_width == 16 else 12
    return "".join(CROCKFORD[(value >> (5 * (n - 1 - i))) & 31] for i in range(n))

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

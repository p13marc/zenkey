"""The injective slug and its decoder (core.md §1.4)."""

from __future__ import annotations

from .lexical import is_plain_chunk

PREFIX = "x-"
#: core.md §1.4 rule 3: "The empty value is ``x-_x``."
EMPTY = "x-_x"

_LITERAL = frozenset(b"abcdefghijklmnopqrstuvwxyz0123456789")
_HEX = "0123456789abcdef"


def slug(value: str) -> str:
    """Write a parameter value as its chunk (core.md §1.4)."""
    # Rule 1: a plain chunk that does not start with the reserved prefix is
    # its own chunk.
    if is_plain_chunk(value) and not value.startswith(PREFIX):
        return value
    # Rule 3: the empty value.
    if value == "":
        return EMPTY
    # Rule 2: ``x-`` then the UTF-8 bytes; each byte outside [a-z0-9] is
    # ``_xHH`` (lowercase hex, no closing underscore), except ``.`` and ``-``,
    # which stay literal unless they are the value's last byte.
    data = value.encode("utf-8")
    out = [PREFIX]
    last = len(data) - 1
    for i, b in enumerate(data):
        if b in _LITERAL or (b in b".-" and i != last):
            out.append(chr(b))
        else:
            out.append(f"_x{b:02x}")
    return "".join(out)


def unslug(chunk: str) -> str | None:
    """Read a chunk back as its value, or None when the chunk is not one the
    encoder produces (core.md §1.4: the decoder "MUST accept exactly the
    canonical chunks the encoder produces")."""
    if not chunk.startswith(PREFIX):
        return chunk if is_plain_chunk(chunk) else None
    if chunk == EMPTY:
        return ""
    body = chunk[len(PREFIX):]
    data = bytearray()
    i = 0
    while i < len(body):
        c = body[i]
        if c == "_":
            hh = body[i + 2:i + 4]
            if body[i + 1:i + 2] != "x" or len(hh) != 2 or any(h not in _HEX for h in hh):
                return None
            data.append(int(hh, 16))
            i += 4
        else:
            if not c.isascii():
                return None
            data.append(ord(c))
            i += 1
    try:
        value = data.decode("utf-8")
    except UnicodeDecodeError:
        return None
    # Canonical check: only the encoder's own output is accepted, which
    # refuses ``x-eth0`` (eth0 is its own chunk), upper-case hex, an escaped
    # byte that should have been literal, and a literal trailing ``.``/``-``.
    return value if slug(value) == chunk else None

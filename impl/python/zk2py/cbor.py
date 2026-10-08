"""A small, strict CBOR (RFC 8949) decoder for the error envelope (core.md §5.2).

Strict means: exactly one data item and no trailing bytes, no duplicate map
key, well-formed lengths, valid UTF-8 text. Tags are decoded to their
content (a tagged item is not refused; the spec says nothing about tags).
Byte strings decode to ``bytes``.
"""

from __future__ import annotations

import math
import struct
from typing import Any


class CborError(ValueError):
    pass


_BREAK = object()


class _Reader:
    def __init__(self, data: bytes):
        self.data = data
        self.pos = 0

    def take(self, n: int) -> bytes:
        if self.pos + n > len(self.data):
            raise CborError("truncated")
        out = self.data[self.pos:self.pos + n]
        self.pos += n
        return out

    def argument(self, info: int) -> int | None:
        if info < 24:
            return info
        if info == 24:
            return self.take(1)[0]
        if info == 25:
            return struct.unpack(">H", self.take(2))[0]
        if info == 26:
            return struct.unpack(">I", self.take(4))[0]
        if info == 27:
            return struct.unpack(">Q", self.take(8))[0]
        if info == 31:
            return None  # indefinite length
        raise CborError(f"reserved additional information {info}")

    def item(self, allow_break: bool = False) -> Any:
        head = self.take(1)[0]
        major, info = head >> 5, head & 0x1F
        if major == 7:
            return self.simple(info, allow_break)
        arg = self.argument(info)
        if major == 0:
            return self._definite(arg)
        if major == 1:
            return -1 - self._definite(arg)
        if major in (2, 3):
            if arg is None:
                chunks = []
                while (c := self.item(allow_break=True)) is not _BREAK:
                    if type(c) is not (bytes if major == 2 else str):
                        raise CborError("bad chunk in an indefinite string")
                    chunks.append(c)
                return b"".join(chunks) if major == 2 else "".join(chunks)
            raw = self.take(arg)
            if major == 2:
                return raw
            try:
                return raw.decode("utf-8")
            except UnicodeDecodeError as e:
                raise CborError("text is not UTF-8") from e
        if major == 4:
            out = []
            if arg is None:
                while (x := self.item(allow_break=True)) is not _BREAK:
                    out.append(x)
            else:
                out = [self.item() for _ in range(arg)]
            return out
        if major == 5:
            pairs = []
            if arg is None:
                while (k := self.item(allow_break=True)) is not _BREAK:
                    pairs.append((k, self.item()))
            else:
                pairs = [(self.item(), self.item()) for _ in range(arg)]
            return _Map(pairs)
        if major == 6:
            self._definite(arg)
            return self.item()
        raise CborError("unreachable")

    def _definite(self, arg: int | None) -> int:
        if arg is None:
            raise CborError("indefinite length on an integer or tag")
        return arg

    def simple(self, info: int, allow_break: bool) -> Any:
        if info == 20:
            return False
        if info == 21:
            return True
        if info in (22, 23):
            return None  # null; undefined is read as null
        if info == 25:
            return _half(self.take(2))
        if info == 26:
            return struct.unpack(">f", self.take(4))[0]
        if info == 27:
            return struct.unpack(">d", self.take(8))[0]
        if info == 31 and allow_break:
            return _BREAK
        raise CborError(f"unsupported simple value {info}")


class _Map(list):
    """A decoded map, kept as its pairs so duplicates and key types can be
    judged by the caller."""


def _half(b: bytes) -> float:
    h = struct.unpack(">H", b)[0]
    exp, frac = (h >> 10) & 0x1F, h & 0x3FF
    sign = -1.0 if h & 0x8000 else 1.0
    if exp == 0:
        return sign * math.ldexp(frac, -24)
    if exp == 31:
        return sign * (math.inf if frac == 0 else math.nan)
    return sign * math.ldexp(frac + 1024, exp - 25)


def loads(data: bytes) -> Any:
    """Decode one CBOR data item. Maps become dicts and must have unique
    keys; a non-text key is kept as is (the caller decides)."""
    r = _Reader(data)
    item = r.item()
    if r.pos != len(data):
        raise CborError("trailing bytes")
    return _to_python(item)


def _to_python(v: Any) -> Any:
    if isinstance(v, _Map):
        out: dict[Any, Any] = {}
        for k, x in v:
            k = _to_python(k)
            if isinstance(k, (list, dict)):
                raise CborError("a map key that is a container")
            if k in out:
                raise CborError(f"duplicate map key {k!r}")
            out[k] = _to_python(x)
        return out
    if isinstance(v, list):
        return [_to_python(x) for x in v]
    return v

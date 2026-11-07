"""Core §7.5's heartbeat (core 0.81, #978 F1): its tag, its collections, its
body and the digest, session-free. ``freshness.v2`` (text 0.1, core 0.82)
says who sends one and what a reader concludes: :mod:`zk2py.freshness_v2`.

**The tag** is "the low 4 bits of the last byte of the covered revision's
fingerprint (§9.5). An implementation that writes a heartbeat MUST write its
covered revision's tag in CH" (:func:`tag_of`).

**The collections** (:func:`layout_of`): "a ``state`` or ``@state`` resource
whose resolved ``freshness.ttl_s`` is above 0", in "the canonical form's
resource order: by kind token, then template". One "is templated when its
template has a parameter. Its cardinality c then bounds its live members";
its live count takes ⌈log₂(c + 1)⌉ bits, ``c.bit_length()``.

**The body** (:func:`write_bits`, :func:`write`, :func:`read`)::

    body       time · map · a section for each map bit set, in map order · pad
    time       42 bits: 32 of seconds from the deployment epoch, 10 of ms
    map        1 bit for each collection, in order
    section    digest · members
    digest     32 bits
    members    one member: vouched, 1 bit
               templated:  live · sel · (bitmap | count · ranks)

``count`` takes ⌈log₂(live + 1)⌉ bits, ``live.bit_length()``, and each rank
⌈log₂ live⌉, ``(live - 1).bit_length()``. "An implementation that writes a
templated section MUST write the list only where it is shorter than the
bitmap": a tie keeps the bitmap. A value read keeps the ``form`` it was read
in, and writes back in it: "Every body an implementation accepts MUST write
back to the same bytes."

**Reading** refuses "for the first of these that applies, in this order":
``short`` and ``code`` as the fields are read, then ``trailing``, then
``pad`` (:class:`Refused`).

**The digest** (:func:`digest`): the XOR over the live members of the first
4 bytes, big-endian, of the sha256 of "its base-relative key, in UTF-8; a
zero byte; a byte holding its run's width bit …; its run, then its seq,
big-endian". "An implementation MUST compute a digest so."

A body's value is the fixtures' (``conformance/heartbeat/bodies.json``):
``{"time": {"s", "ms"} | None, "sections": {resource: section}}``, a
resource named ``<kind token>/<template>``; a section of one member
``{"digest": hex8, "vouched": bool}``, a templated one's ``{"digest", "live",
"vouched": [ranks], "form"?}``.
"""

from __future__ import annotations

import hashlib
from dataclasses import dataclass
from typing import Any, Iterable

#: core §7.5: "All 42 bits set say the owner's clock is unknown."
UNKNOWN_S, UNKNOWN_MS = 2**32 - 1, 1023
TIME_BITS, SECOND_BITS, MS_BITS, DIGEST_BITS = 42, 32, 10, 32
#: freshness.v2 §3: the standard contract's interface, and its one resource
PROFILE = "freshness.v2"
RESOURCE = "stream/heartbeat/{iface}"


class Refused(ValueError):
    """A body core §7.5 refuses, with its token: ``short``, ``code``,
    ``trailing`` or ``pad``."""

    def __init__(self, token: str, why: str):
        super().__init__(f"{token}: {why}")
        self.token = token


@dataclass(frozen=True)
class Collection:
    """One collection of a covered revision (core §7.5)."""

    resource: str            # "<kind token>/<template>"
    token: str               # "state" or "@state"
    template: str
    params: tuple[str, ...]
    cardinality: int | None  # None: no parameter, one member
    ttl_s: int

    @property
    def templated(self) -> bool:
        return self.cardinality is not None

    @property
    def live_bits(self) -> int | None:
        """⌈log₂(c + 1)⌉, None for a collection of one member."""
        return None if self.cardinality is None else self.cardinality.bit_length()

    def expect(self) -> dict[str, Any]:
        """``layouts.json``'s collection object."""
        return {"resource": self.resource, "cardinality": self.cardinality, "live_bits": self.live_bits,
                "ttl_s": self.ttl_s}


@dataclass(frozen=True)
class Layout:
    """A covered revision's heartbeat layout: its interface id, fingerprint,
    tag and collections in map order."""

    interface: str
    fingerprint: str
    tag: int
    collections: tuple[Collection, ...]

    def collection(self, resource: str) -> Collection:
        for c in self.collections:
            if c.resource == resource:
                return c
        raise KeyError(f"{resource}: no collection of {self.interface}")

    @property
    def shortest_ttl_s(self) -> int | None:
        """freshness.v2 §2.4: the heartbeats are "at most half the shortest
        horizon among that interface's collections apart"; None with no
        collection, which has no heartbeat."""
        return min((c.ttl_s for c in self.collections), default=None)


def tag_of(fingerprint: str) -> int:
    """CH's tag: the low 4 bits of the fingerprint's last byte."""
    digest = fingerprint.removeprefix("sha256:")
    return bytes.fromhex(digest)[-1] & 0x0F


def layout_of(canonical: dict[str, Any], fingerprint: str) -> Layout:
    """The collections of a canonical form (core §9.5): each ``state`` or
    ``@state`` resource whose resolved ``freshness.ttl_s`` is a horizon above
    0 (freshness.v1 §2.1), by kind token then template. A value that is no
    horizon is not "above 0", so it makes no collection."""
    from . import freshness

    out = []
    for r in sorted(canonical["resources"], key=lambda r: (r["token"], r["template"])):
        if r["token"] not in ("state", "@state"):
            continue
        h = freshness.horizon("state", r["annotations"])
        if h.horizon != "within":
            continue
        params = tuple(r.get("params") or {})
        out.append(Collection(f"{r['token']}/{r['template']}", r["token"], r["template"], params,
                              r["cardinality"] if params else None, h.ttl_s))
    return Layout(canonical["interface"], fingerprint, tag_of(fingerprint), tuple(out))


def heartbeat_key(system: str, service: str, iface: str) -> str:
    """freshness.v2 §3: ``zk2/<system>/<service>/freshness.v2/stream/heartbeat/<iface>``,
    the covered interface's id a plain chunk."""
    return f"zk2/{system}/{service}/{PROFILE}/stream/heartbeat/{iface}"


# -- the digest ----------------------------------------------------------------------

def member_value(key: str, width: int, run: int, seq: int) -> int:
    """A live member's 32-bit value: the first 4 bytes, big-endian, of the
    sha256 of its key, a zero byte, its run's width bit, its run and seq."""
    if width not in (16, 32):
        raise ValueError("a run is 16 or 32 bits")
    data = key.encode("utf-8") + b"\x00" + bytes([1 if width == 32 else 0]) \
        + run.to_bytes(width // 8, "big") + seq.to_bytes(3, "big")
    return int.from_bytes(hashlib.sha256(data).digest()[:4], "big")


def digest(members: Iterable[tuple[str, int, int, int]]) -> int:
    """The XOR of each live member's value, (key, width, run, seq); 0 with
    none live."""
    d = 0
    for key, width, run, seq in members:
        d ^= member_value(key, width, run, seq)
    return d


# -- bits ------------------------------------------------------------------------------

class _Bits:
    """A body's bits, most significant first, read in order."""

    def __init__(self, data: bytes):
        self.data, self.pos, self.total = data, 0, len(data) * 8

    def take(self, n: int, what: str) -> int:
        if self.pos + n > self.total:
            raise Refused("short", f"the body ends before {what} ({n} bits at bit {self.pos}, {self.total} held)")
        v = 0
        for i in range(self.pos, self.pos + n):
            v = (v << 1) | ((self.data[i >> 3] >> (7 - (i & 7))) & 1)
        self.pos += n
        return v


def _rank_bits(live: int) -> int:
    """⌈log₂ live⌉: 0 for one live member."""
    return (live - 1).bit_length() if live > 0 else 0


def _list_shorter(live: int, count: int) -> bool:
    """Whether a list of ``count`` ranks of ``live`` is shorter than the
    bitmap's ``live`` bits."""
    return live.bit_length() + count * _rank_bits(live) < live


def write_bits(layout: Layout, value: dict[str, Any]) -> str:
    """The body's bits before the pad, as a string of 0 and 1. Raises
    :class:`ValueError` for a value no body holds."""
    out: list[str] = []

    def put(v: int, n: int, what: str) -> None:
        if not 0 <= v < (1 << n):
            raise ValueError(f"{what}: {v} does not fit {n} bits")
        if n:
            out.append(format(v, f"0{n}b"))

    t = value.get("time")
    if t is None:
        put(UNKNOWN_S, SECOND_BITS, "the time's seconds")
        put(UNKNOWN_MS, MS_BITS, "the time's milliseconds")
    else:
        if not 0 <= t["ms"] <= 999:
            raise ValueError(f"{t['ms']} ms: a known time's milliseconds are 0 to 999")
        put(t["s"], SECOND_BITS, "the time's seconds")
        put(t["ms"], MS_BITS, "the time's milliseconds")
    sections = value.get("sections", {})
    unknown = set(sections) - {c.resource for c in layout.collections}
    if unknown:
        raise ValueError(f"sections of no collection: {sorted(unknown)}")
    for c in layout.collections:
        put(1 if c.resource in sections else 0, 1, f"{c.resource}'s map bit")
    for c in layout.collections:
        if c.resource not in sections:
            continue
        sec = sections[c.resource]
        put(int(sec["digest"], 16), DIGEST_BITS, f"{c.resource}'s digest")
        if not c.templated:
            put(1 if sec["vouched"] else 0, 1, f"{c.resource}'s vouched bit")
            continue
        live, ranks = sec["live"], list(sec["vouched"])
        assert c.cardinality is not None and c.live_bits is not None
        if live > c.cardinality:
            raise ValueError(f"{c.resource}: {live} live, above its cardinality {c.cardinality}")
        if ranks != sorted(set(ranks)) or any(not 0 <= r < live for r in ranks):
            raise ValueError(f"{c.resource}: ranks {ranks} are not ascending ranks below {live}")
        put(live, c.live_bits, f"{c.resource}'s live count")
        form = sec.get("form") or ("list" if _list_shorter(live, len(ranks)) else "bitmap")
        if form == "bitmap":
            put(0, 1, "sel")
            out.append("".join("1" if i in ranks else "0" for i in range(live)))
        elif form == "list":
            put(1, 1, "sel")
            put(len(ranks), live.bit_length(), f"{c.resource}'s count")
            for r in ranks:
                put(r, _rank_bits(live), f"{c.resource}'s rank")
        else:
            raise ValueError(f"{c.resource}: no such form {form!r}")
    return "".join(out)


def write(layout: Layout, value: dict[str, Any]) -> bytes:
    """The body's bytes: its bits, then zero bits to the next whole byte."""
    bits = write_bits(layout, value)
    bits += "0" * (-len(bits) % 8)
    return int(bits, 2).to_bytes(len(bits) // 8, "big") if bits else b""


def read(layout: Layout, data: bytes) -> dict[str, Any]:
    """A body read with its covered revision's layout, or :class:`Refused`
    for the first reason that applies, in §7.5's order. A templated
    section's value names the ``form`` it was read in."""
    b = _Bits(bytes(data))
    s = b.take(SECOND_BITS, "the time's seconds")
    ms = b.take(MS_BITS, "the time's milliseconds")
    if ms >= 1000 and not (s == UNKNOWN_S and ms == UNKNOWN_MS):
        raise Refused("code", f"a millisecond code of {ms}, unused")
    out: dict[str, Any] = {"time": None if ms >= 1000 else {"s": s, "ms": ms}, "sections": {}}
    marked = [c for c in layout.collections if b.take(1, f"{c.resource}'s map bit")]
    for c in marked:
        d = b.take(DIGEST_BITS, f"{c.resource}'s digest")
        sec: dict[str, Any] = {"digest": f"{d:08x}"}
        if not c.templated:
            sec["vouched"] = bool(b.take(1, f"{c.resource}'s vouched bit"))
        else:
            assert c.cardinality is not None and c.live_bits is not None
            live = b.take(c.live_bits, f"{c.resource}'s live count")
            if live > c.cardinality:
                raise Refused("code", f"{c.resource}: a live count of {live}, above its cardinality {c.cardinality}")
            if b.take(1, f"{c.resource}'s sel") == 0:
                sec.update(form="bitmap", live=live,
                           vouched=[i for i in range(live) if b.take(1, f"{c.resource}'s bitmap")])
            else:
                count = b.take(live.bit_length(), f"{c.resource}'s count")
                if count > live:
                    raise Refused("code", f"{c.resource}: a count of {count}, above its {live} live")
                ranks: list[int] = []
                for _ in range(count):
                    r = b.take(_rank_bits(live), f"{c.resource}'s rank")
                    if r >= live or (ranks and r <= ranks[-1]):
                        raise Refused("code", f"{c.resource}: a rank of {r} after {ranks}, of {live} live")
                    ranks.append(r)
                sec.update(form="list", live=live, vouched=ranks)
        out["sections"][c.resource] = sec
    used = (b.pos + 7) // 8
    if len(b.data) > used:
        raise Refused("trailing", f"{len(b.data) - used} bytes after the pad")
    if b.pos % 8 and b.data[-1] & ((1 << (8 - b.pos % 8)) - 1):
        raise Refused("pad", "a pad bit is 1")
    return out


def instant_of(time: dict[str, int] | None, epoch_s: int) -> int | None:
    """A heartbeat's time read back to an instant in Unix milliseconds,
    counted from the deployment epoch ``epoch_s`` (core §7.6, ``freshness.v2``
    0.6 §2.4: the catalog's epoch, else 0); None for the unknown time."""
    if time is None:
        return None
    return (epoch_s + time["s"]) * 1000 + time["ms"]


def time_of(now_ns: int, epoch_s: int) -> dict[str, int] | None:
    """The time field of an owner's clock ``now_ns`` (Unix ns), counted from
    the deployment epoch ``epoch_s`` (core §7.6): None, the unknown time,
    where it falls outside the 32 bits of seconds."""
    total_ms = now_ns // 1_000_000 - epoch_s * 1000
    s, ms = divmod(total_ms, 1000)
    return {"s": s, "ms": ms} if 0 <= s <= UNKNOWN_S else None

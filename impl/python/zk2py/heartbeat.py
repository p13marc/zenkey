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

**Another form on a heartbeat's key** (core 0.109, F-540's answer;
:func:`put_of`): "A heartbeat's key and an owner heartbeat's carry form 6,
and no other form. Both are ``stream`` keys, and the parse reads a key's
kind token alone. So it accepts a mutation or a frame on either. ... An
implementation that reads a heartbeat or an owner heartbeat MUST refuse a
put of another form on that key, naming ``form``, before it reads a tag or a
body." "A put with no envelope on either key is refused the same way."
:func:`put_of` is the gate of both readings: zk2py's heartbeat subscribers
and :func:`zk2py.owner_heartbeat.read` go through it.

**The digest** (:func:`digest`): the XOR over the live members of the first
4 bytes, big-endian, of the sha256 of "its base-relative key, in UTF-8; a
zero byte; a byte holding its run's width bit …; its run, then its seq,
big-endian". "An implementation MUST compute a digest so."

**The time's three readings** (core 0.113; :func:`read_time`,
:func:`time_bits`, :func:`reading_of`): "Two of its codes carry no instant.
Unknown. All 42 bits set say the owner's clock is unknown. Ahead. The seconds
all ones, with the millisecond code 1022, say the owner's clock is ahead: its
clock guard holds (§4.3, "Ahead"). It is the unknown time with its last bit
clear." "Unused. Every other millisecond code from 1000 up is unused: 1000 to
1021 under any seconds, and 1022 or 1023 under seconds that are not all
ones." "An implementation that reads a heartbeat's time MUST read it as an
instant, as unknown or as ahead ... An implementation MUST NOT read the ahead
time as unknown, nor as an instant." Here an instant is ``{"s", "ms"}``, the
unknown time None, and the ahead time :data:`AHEAD`, a value of its own:
no code path that asks ``time is None`` takes it for unknown, and none that
reads ``time["s"]`` takes it for an instant without raising.

A body's value is the fixtures' (``conformance/heartbeat/bodies.json``):
``{"time": {"s", "ms"} | None | AHEAD, "sections": {resource: section}}``, a
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
#: core §7.5 (0.113): "The seconds all ones, with the millisecond code 1022,
#: say the owner's clock is ahead": ``0x3fffffffffe`` in 42 bits, "the unknown
#: time with its last bit clear". The value of a time read so, and written so.
AHEAD_MS = 1022
AHEAD = "ahead"
#: the three readings of a heartbeat's time (:func:`reading_of`)
INSTANT, UNKNOWN = "instant", "unknown"
TIME_BITS, SECOND_BITS, MS_BITS, DIGEST_BITS = 42, 32, 10, 32
#: freshness.v2 §3: the standard contract's interface, and its one resource
PROFILE = "freshness.v2"
RESOURCE = "stream/heartbeat/{iface}"


class Refused(ValueError):
    """A body core §7.5 refuses, with its token: ``short``, ``code``,
    ``trailing`` or ``pad``; or a put on a heartbeat's key that
    :func:`put_of` refuses: the parse's token, or ``form``."""

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
    """A body's bits, most significant first, read in order. ``start`` and
    ``end`` bound a run of bits within ``data``, as an owner heartbeat's
    group's sections are (core 0.108): ``noun`` names what ends."""

    def __init__(self, data: bytes, start: int = 0, end: int | None = None, noun: str = "the body"):
        self.data, self.pos, self.total, self.noun = data, start, len(data) * 8 if end is None else end, noun

    def take(self, n: int, what: str) -> int:
        if self.pos + n > self.total:
            raise Refused("short", f"{self.noun} ends before {what} ({n} bits at bit {self.pos}, {self.total} held)")
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


def _put(out: list[str], v: int, n: int, what: str) -> None:
    if not 0 <= v < (1 << n):
        raise ValueError(f"{what}: {v} does not fit {n} bits")
    if n:
        out.append(format(v, f"0{n}b"))


def reading_of(t: Any) -> str:
    """Which of §7.5's three readings a time's value is: :data:`INSTANT`,
    :data:`UNKNOWN` or :data:`AHEAD`. A value that is none of them raises:
    nothing falls through to unknown, nor to an instant."""
    if t is None:
        return UNKNOWN
    if isinstance(t, str):
        if t == AHEAD:
            return AHEAD
        raise ValueError(f"{t!r}: no reading of a heartbeat's time")
    if isinstance(t, dict) and type(t.get("s")) is int and type(t.get("ms")) is int:
        return INSTANT
    raise ValueError(f"{t!r}: no reading of a heartbeat's time")


def time_bits(t: dict[str, int] | str | None) -> str:
    """The 42-bit time: 32 bits of seconds, then 10 of milliseconds; all
    ones for the unknown time, None; the seconds all ones and the code 1022
    for the ahead time, :data:`AHEAD`."""
    out: list[str] = []
    reading = reading_of(t)
    if reading == UNKNOWN:
        _put(out, UNKNOWN_S, SECOND_BITS, "the time's seconds")
        _put(out, UNKNOWN_MS, MS_BITS, "the time's milliseconds")
    elif reading == AHEAD:
        _put(out, UNKNOWN_S, SECOND_BITS, "the time's seconds")
        _put(out, AHEAD_MS, MS_BITS, "the time's milliseconds")
    else:
        assert isinstance(t, dict)
        if not 0 <= t["ms"] <= 999:
            raise ValueError(f"{t['ms']} ms: a known time's milliseconds are 0 to 999")
        _put(out, t["s"], SECOND_BITS, "the time's seconds")
        _put(out, t["ms"], MS_BITS, "the time's milliseconds")
    return "".join(out)


def sections_bits(layout: Layout, sections: dict[str, Any]) -> str:
    """"The map, then a section for each map bit set": a heartbeat's body
    between its time and its pad, which an owner heartbeat's group holds
    whole (core 0.108)."""
    out: list[str] = []

    def put(v: int, n: int, what: str) -> None:
        _put(out, v, n, what)

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


def write_bits(layout: Layout, value: dict[str, Any]) -> str:
    """The body's bits before the pad, as a string of 0 and 1. Raises
    :class:`ValueError` for a value no body holds."""
    return time_bits(value.get("time")) + sections_bits(layout, value.get("sections", {}))


def write(layout: Layout, value: dict[str, Any]) -> bytes:
    """The body's bytes: its bits, then zero bits to the next whole byte."""
    bits = write_bits(layout, value)
    bits += "0" * (-len(bits) % 8)
    return int(bits, 2).to_bytes(len(bits) // 8, "big") if bits else b""


def read_time(b: _Bits) -> dict[str, int] | str | None:
    """The 42-bit time, read whole, then judged (core 0.113): an instant;
    None, the unknown time, for all 42 bits set; :data:`AHEAD` for the
    seconds all ones with the millisecond code 1022. "Every other millisecond
    code from 1000 up is unused: 1000 to 1021 under any seconds, and 1022 or
    1023 under seconds that are not all ones" (``code``). Seconds all ones
    under a code below 1000 are an instant, the last second the field
    holds."""
    s = b.take(SECOND_BITS, "the time's seconds")
    ms = b.take(MS_BITS, "the time's milliseconds")
    if ms < 1000:
        return {"s": s, "ms": ms}
    if s == UNKNOWN_S and ms == UNKNOWN_MS:
        return None
    if s == UNKNOWN_S and ms == AHEAD_MS:
        return AHEAD
    raise Refused("code", f"a millisecond code of {ms} under seconds {s}, unused")


def read_sections(layout: Layout, b: _Bits) -> dict[str, Any]:
    """The map and each marked collection's section, field by field:
    ``short`` where the bits end before a field, ``code`` for a field's
    unused code, the first field that draws a refusal deciding."""
    out: dict[str, Any] = {}
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
        out[c.resource] = sec
    return out


def read(layout: Layout, data: bytes, time_reader: Any = None) -> dict[str, Any]:
    """A body read with its covered revision's layout, or :class:`Refused`
    for the first reason that applies, in §7.5's order. A templated
    section's value names the ``form`` it was read in. ``time_reader`` reads
    the 42-bit time in :func:`read_time`'s place: a reader built before
    core 0.113, for the fixtures' ``earlier`` members, handed in rather
    than swapped into this module."""
    b = _Bits(bytes(data))
    out: dict[str, Any] = {"time": (time_reader or read_time)(b)}
    out["sections"] = read_sections(layout, b)
    used = (b.pos + 7) // 8
    if len(b.data) > used:
        raise Refused("trailing", f"{len(b.data) - used} bytes after the pad")
    if b.pos % 8 and b.data[-1] & ((1 << (8 - b.pos % 8)) - 1):
        raise Refused("pad", "a pad bit is 1")
    return out


def put_of(payload: bytes, key_token: str | None = None, enveloped: bool = True) -> Any:
    """A put on a heartbeat's key or on an owner heartbeat's, read as far as
    its form (core §7.5, 0.109): the envelope form 6 carries, or
    :class:`Refused`.

    1. "A put with no envelope on either key is refused the same way":
       ``enveloped`` False, a put whose Encoding carries none, is ``form``.
       Its bytes are never parsed as an envelope.
    2. "The envelope's parse comes first": its refusal is the parse's token
       (``short`` for no byte at all, ``sample_kind``, ``key_kind`` ...).
    3. "then the put's form (``form``)": a mutation or a frame, which the
       parse accepts on a ``stream`` key, "is no heartbeat, and no owner
       heartbeat". It is refused "before it reads a tag or a body": a
       mutation whose tag is not 0 is ``form`` on an owner heartbeat's key,
       never ``tag``.

    Nothing here chooses a layout: the caller holds the key, and reads the
    body it returns as a heartbeat's or as an owner heartbeat's. ``key_token``
    is the key's kind token where the caller gives the parse one."""
    from . import zk2c

    if not enveloped:
        raise Refused("form", "a put with no envelope on a heartbeat's key carries no heartbeat")
    try:
        env = zk2c.parse(bytes(payload), "put", key_token)
    except zk2c.Refused as e:
        raise Refused(e.token, str(e).split(": ", 1)[1]) from None
    if env.form != "heartbeat":
        raise Refused("form", f"a {env.form} on a heartbeat's key: it carries form 6, and no other form")
    return env


def read_put(layout: Layout, payload: bytes, key_token: str | None = None, enveloped: bool = True) -> dict[str, Any]:
    """A put on a heartbeat's key read whole: :func:`put_of`, then its body
    with the covered revision's layout. The value is the body's, with CH's
    ``tag`` and whether it is the layout's (``tag_matches``): "The tag is a
    check", and what a reader concludes from another tag is
    ``freshness.v2``'s (§2.5), which reads no body under one."""
    env = put_of(payload, key_token, enveloped)
    return {**read(layout, env.record or b""), "tag": env.tag, "tag_matches": env.tag == layout.tag}


def instant_of(time: dict[str, int] | str | None, epoch_s: int) -> int | None:
    """A heartbeat's time read back to an instant in Unix milliseconds,
    counted from the deployment epoch ``epoch_s`` (core §7.6, ``freshness.v2``
    0.6 §2.4: the catalog's epoch, else 0); None where it carries none, the
    unknown time and the ahead time. The two are told apart by
    :func:`reading_of`, never here: "It carries no instant." """
    if reading_of(time) != INSTANT:
        return None
    assert isinstance(time, dict)
    return (epoch_s + time["s"]) * 1000 + time["ms"]


def shown(time: dict[str, int] | str | None, epoch_s: int) -> dict[str, Any]:
    """A heartbeat's time as a tool shows it (``freshness.v2`` 0.12 §5): "A
    tool that shows one shows the ahead time as ahead: never as unknown, and
    never as an instant. It shows the unknown time as unknown, never as the
    epoch." ``{"clock": "instant", "unix_ms"}``, ``{"clock": "unknown"}`` or
    ``{"clock": "ahead"}``."""
    reading = reading_of(time)
    if reading == INSTANT:
        return {"clock": INSTANT, "unix_ms": instant_of(time, epoch_s)}
    return {"clock": reading}


def time_of(now_ns: int | None, epoch_s: int, guard: bool = False) -> dict[str, int] | str | None:
    """The time field an owner writes (core §7.5, ``freshness.v2`` 0.12
    §2.10 and §2.4). "An owner MUST write core §7.5's ahead time as its
    heartbeat's time while its guard holds, and as its owner heartbeat's
    ... Whatever its clock reads. The ahead time carries no instant. An
    owner writes it for an instant the epoch codes and for one it does not":
    ``guard`` decides first, :data:`AHEAD`. Otherwise the clock ``now_ns``
    (Unix ns), counted from the deployment epoch ``epoch_s`` (core §7.6):
    None, the unknown time, "while its clock reads such an instant", one
    "before the epoch, or 2^32 s or more after it", and for a clock that
    reads none (``now_ns`` None)."""
    if guard:
        return AHEAD
    if now_ns is None:
        return None
    total_ms = now_ns // 1_000_000 - epoch_s * 1000
    s, ms = divmod(total_ms, 1000)
    return {"s": s, "ms": ms} if 0 <= s <= UNKNOWN_S else None

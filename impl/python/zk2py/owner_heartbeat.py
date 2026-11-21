"""Core §7.5's owner heartbeat (core 0.108, #1110): form 6's second layout,
"a put an owner sends for several interfaces it serves at once", with "one
**group** for each interface it covers". Session-free: the form, its parse
and write, its refusals and its round trip. Written from §7.5 alone, before
``conformance/heartbeat/owner.json`` was opened (round 112).

**The layout** is its key's to say: "Nothing in the bytes tells the two
apart. An implementation MUST NOT choose the layout by a payload's bytes."
Nothing here reads a payload to decide: a caller that holds an owner
heartbeat's key calls :func:`read`, and one that holds a heartbeat's calls
:func:`zk2py.heartbeat.read`. At core 0.108 no text on this main names the
key (``freshness.v2`` 0.9 does, later), so no function here takes one.

**The bytes**::

    payload    CH · run · seq · body        CH's tag 0
    body       time · groups · that many groups · pad
    time       42 bits, as a heartbeat's
    groups     4 bits: the count of groups, from 1 to 15
    group      index · tag · size · sections
    index      10 bits: the covered revision's index in the catalog
    tag        4 bits: the covered revision's tag
    size       an integer: the count of bits in the group's sections
    sections   map · a section for each map bit set, in map order
    pad        zero bits up to the next whole byte, fewer than 8

"The size is an integer as a snapshot's is: unsigned LEB128, in its
shortest form, at most 5 bytes and at most 2^32 − 1. Each of its bytes is
the next 8 bits of the body, wherever they fall."

**Reading takes two steps.**

1. :func:`read` (a put's payload) and :func:`read_body` (its body): "the
   groups without any contract, as an envelope is parsed". "The envelope's
   parse comes first", then, in this order: ``tag`` (CH's tag is not 0);
   reading the body's fields in order, ``short``, ``code``, ``order`` and
   ``leb128``; then ``trailing``; then ``pad``. "Each field is read whole,
   then judged, and the first field that draws a refusal decides. So an
   unused code is refused before a later field cut short, and an index out
   of order before its own group's size." A group's sections are kept as
   their bits, unread. A refusal here refuses the owner heartbeat
   (:class:`Refused`).
2. :func:`read_group`: one group's sections "with its covered revision's
   collections", its index read "through the catalog its owner names, as
   §8.6 says". In this order: ``unknown_catalog`` or ``unknown_index``;
   reading the sections' fields in order, ``short`` and ``code``; then
   ``trailing``, "the sections hold a bit after their last field". "A group
   has no pad of its own." A refusal here is that group's alone
   (:class:`GroupRead` holds it): "An implementation MUST read each other
   group as it would with the refused one left out", which
   :func:`read_groups` does by reading each from its own bits.

   "A reader reads the groups it has a use for, and skips every other by
   its size, one whose revision it does not hold among them": a group whose
   entry names a revision the reader holds no layout of is ``skipped``,
   never refused.

   "The tag is a check … Another tag refuses nothing here: the sections
   read the same": :attr:`GroupRead.tag_matches`.

**Writing** (:func:`write`, :func:`write_body`, :func:`body_bits`): "CH's
tag 0, from 1 to 15 groups in ascending order of their indices, and each
group's size the count of bits in its sections". The writer orders the
groups it is given by index and refuses two of one index, none, more than
15, an index beyond 10 bits and a tag beyond 4. "An implementation MUST
write every owner heartbeat it accepts back to the same bytes"
(:func:`write_back`).

zk2py's readings of what the text leaves to a reader, each in
``SPEC-FINDINGS.md`` where it is a finding:
- a put on an owner heartbeat's key whose form is not 6 is refused
  ``form``, as zk2py's heartbeat subscriber refuses one on a heartbeat's
  key. §7.5 names no refusal for it;
- a size of 0 is read: a revision with no collection has an empty map;
- the writer orders unordered groups rather than refusing them.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any, Iterable, Mapping, Sequence

from . import catalog as cat
from . import heartbeat, snapshot, zk2c

#: "groups 4 bits", "index 10 bits", "tag 4 bits".
COUNT_BITS, INDEX_BITS, TAG_BITS = 4, 10, 4
#: "from 1 to 15 groups".
MAX_GROUPS = 15
#: "at most 5 bytes and at most 2^32 − 1".
SIZE_MAX_BYTES, SIZE_MAX = 5, 2 ** 32 - 1

#: The refusals of the first reading, which refuse the owner heartbeat.
TOKENS = ("tag", "short", "code", "order", "leb128", "trailing", "pad")
#: The refusals of the second, which refuse one group alone.
GROUP_TOKENS = ("unknown_catalog", "unknown_index", "short", "code", "trailing")


class Refused(ValueError):
    """An owner heartbeat the first reading refuses, or a group the second
    does, with its token."""

    def __init__(self, token: str, why: str):
        super().__init__(f"{token}: {why}")
        self.token = token


@dataclass(frozen=True)
class Group:
    """One group as the first reading holds it: its index, its tag, and its
    sections' bits, unread, as a string of 0 and 1. Its size is their
    count."""

    index: int
    tag: int
    bits: str

    @property
    def size(self) -> int:
        return len(self.bits)


@dataclass(frozen=True)
class OwnerHeartbeat:
    """An owner heartbeat's body read with no contract: its time (a
    heartbeat's ``{"s", "ms"}``, None for the unknown time) and its groups in
    the order read, ascending by index."""

    time: dict[str, int] | None
    groups: tuple[Group, ...]

    def group(self, index: int) -> Group | None:
        return next((g for g in self.groups if g.index == index), None)


@dataclass(frozen=True)
class Put:
    """A put's payload read: its envelope's stamp and the body."""

    stamp: zk2c.InBand
    body: OwnerHeartbeat


@dataclass(frozen=True)
class GroupRead:
    """The second reading of one group. Exactly one of:
    - ``refused``: the token that refuses this group alone;
    - ``skipped``: its entry names a revision the reader holds no layout of;
    - ``sections``: its sections' value, as a heartbeat's body's.
    ``entry`` is the catalog's ``(iface, contract)`` where §8.6 read the
    index, and ``tag_matches`` whether the group's tag is that revision's."""

    index: int
    tag: int
    entry: tuple[str, str] | None = None
    refused: str | None = None
    why: str | None = None
    skipped: bool = False
    tag_matches: bool | None = None
    sections: dict[str, Any] | None = None


# -- the first reading: the groups, with no contract --------------------------------

class _Bits(heartbeat._Bits):
    """The body's bits; a refusal here is the owner heartbeat's."""

    def take(self, n: int, what: str) -> int:
        try:
            return super().take(n, what)
        except heartbeat.Refused as e:
            raise Refused(e.token, str(e).split(": ", 1)[1]) from None

    def raw(self, n: int, what: str) -> str:
        """``n`` bits as they are, unread."""
        if self.pos + n > self.total:
            raise Refused("short", f"the body ends before {what} ({n} bits at bit {self.pos}, {self.total} held)")
        out = "".join(str((self.data[i >> 3] >> (7 - (i & 7))) & 1) for i in range(self.pos, self.pos + n))
        self.pos += n
        return out

    def size(self, what: str) -> int:
        """A size: "a reader takes a byte at a time, and stops at the fifth.
        A fifth byte with its high bit set is refused, whatever follows it."
        A byte the body does not hold is ``short``; then, the integer read
        whole, ``leb128``."""
        value = shift = 0
        for i in range(SIZE_MAX_BYTES):
            b = self.take(8, f"{what}'s byte {i + 1}")
            value |= (b & 0x7F) << shift
            shift += 7
            if not b & 0x80:
                if i and b == 0:
                    raise Refused("leb128", f"{what} is not in its shortest form: its last byte is 0")
                if value > SIZE_MAX:
                    raise Refused("leb128", f"{what} is {value}, above 2^32 − 1")
                return value
        raise Refused("leb128", f"{what}'s fifth byte has its high bit set")


def read_body(body: bytes) -> OwnerHeartbeat:
    """The first reading of a body, after its envelope: the fields in order,
    each read whole, then judged; then ``trailing``, then ``pad``."""
    b = _Bits(bytes(body))
    try:
        time = heartbeat.read_time(b)
    except heartbeat.Refused as e:
        raise Refused(e.token, str(e).split(": ", 1)[1]) from None
    count = b.take(COUNT_BITS, "the count of groups")
    if count == 0:
        raise Refused("code", "a count of 0 groups, unused")
    groups: list[Group] = []
    for n in range(count):
        index = b.take(INDEX_BITS, f"group {n}'s index")
        if groups and index <= groups[-1].index:
            raise Refused("order", f"group {n}'s index {index} is not above {groups[-1].index}")
        tag = b.take(TAG_BITS, f"group {n}'s tag")
        size = b.size(f"group {n}'s size")
        groups.append(Group(index, tag, b.raw(size, f"group {n}'s sections")))
    used = (b.pos + 7) // 8
    if len(b.data) > used:
        raise Refused("trailing", f"{len(b.data) - used} bytes after the pad")
    if b.pos % 8 and b.data[-1] & ((1 << (8 - b.pos % 8)) - 1):
        raise Refused("pad", "a pad bit is 1")
    return OwnerHeartbeat(time, tuple(groups))


def read(payload: bytes, key_token: str | None = None) -> Put:
    """The first reading of a put's payload on an owner heartbeat's key.
    "The envelope's parse comes first": its refusals are the parse's
    (:class:`zk2py.zk2c.Refused`'s token, raised here as :class:`Refused`).
    Then ``tag``, then the body's."""
    try:
        env = zk2c.parse(bytes(payload), "put", key_token)
    except zk2c.Refused as e:
        raise Refused(e.token, str(e).split(": ", 1)[1]) from None
    if env.form != "heartbeat":
        # No sentence of §7.5 names this refusal: zk2py's own, as its
        # heartbeat subscriber's is on a heartbeat's key.
        raise Refused("form", f"a {env.form} on an owner heartbeat's key")
    if env.tag != 0:
        raise Refused("tag", f"CH's tag is {env.tag}, and an owner heartbeat's is 0")
    stamp = env.stamp
    assert stamp is not None
    return Put(stamp, read_body(env.record or b""))


# -- the second reading: one group's sections, with its revision --------------------

Layouts = Mapping[tuple[str, str], heartbeat.Layout]


def read_sections(layout: heartbeat.Layout, bits: str) -> dict[str, Any]:
    """A group's sections read with its covered revision's collections, the
    bits its size counts and no other: ``short``, ``code``, then
    ``trailing``. Raises :class:`Refused`."""
    # The bits sit at the high end of ``data``, and the reader stops at their count.
    data = (int(bits, 2) << (-len(bits) % 8)).to_bytes((len(bits) + 7) // 8, "big") if bits else b""
    b = heartbeat._Bits(data, 0, len(bits), "the sections")
    try:
        sections = heartbeat.read_sections(layout, b)
    except heartbeat.Refused as e:
        raise Refused(e.token, str(e).split(": ", 1)[1]) from None
    if b.pos < len(bits):
        raise Refused("trailing", f"{len(bits) - b.pos} bits after the sections' last field")
    return sections


def read_group(group: Group, named: str | None, held: Sequence[cat.Catalog], layouts: Layouts) -> GroupRead:
    """The second reading of one group: its index "through the catalog its
    owner names, as §8.6 says" (``named``, the id the owner names, None for
    an owner that names none; ``held``, the catalogs the reader holds), then
    its sections with the revision that entry names (``layouts``, by
    ``(iface, contract)``). Never raises for the group's bytes: a refusal is
    the group's alone."""
    try:
        entry = cat.resolve(list(held), named, group.index).entry
    except cat.Refused as e:
        return GroupRead(group.index, group.tag, refused=e.token, why=str(e))
    assert entry is not None
    layout = layouts.get(entry)
    if layout is None:
        return GroupRead(group.index, group.tag, entry=entry, skipped=True)
    try:
        sections = read_sections(layout, group.bits)
    except Refused as e:
        return GroupRead(group.index, group.tag, entry=entry, refused=e.token, why=str(e),
                         tag_matches=group.tag == layout.tag)
    return GroupRead(group.index, group.tag, entry=entry, tag_matches=group.tag == layout.tag, sections=sections)


def read_groups(body: OwnerHeartbeat, named: str | None, held: Sequence[cat.Catalog],
                layouts: Layouts) -> list[GroupRead]:
    """Every group's second reading, each from its own bits: "A refused
    group says nothing of the groups beside it"."""
    return [read_group(g, named, held, layouts) for g in body.groups]


# -- writing -------------------------------------------------------------------------

def group_of(index: int, layout: heartbeat.Layout, sections: dict[str, Any]) -> Group:
    """A group an owner writes for the revision at ``index`` of its catalog:
    that revision's tag, and its sections' bits."""
    return Group(index, layout.tag, heartbeat.sections_bits(layout, sections))


def _size_bits(n: int) -> str:
    """A size's LEB128 bytes as bits, "the next 8 bits of the body, wherever
    they fall"."""
    try:
        raw = snapshot.leb128(n)
    except snapshot.Refused as e:
        raise ValueError(str(e)) from None
    return "".join(format(byte, "08b") for byte in raw)


def body_bits(time: dict[str, int] | None, groups: Iterable[Group]) -> str:
    """The body's bits before its pad. The groups are written "in ascending
    order of their indices"; :class:`ValueError` for what no owner heartbeat
    holds: no group, more than 15, an index twice, an index or a tag beyond
    its field, sections that are no bits."""
    ordered = sorted(groups, key=lambda g: g.index)
    if not 1 <= len(ordered) <= MAX_GROUPS:
        raise ValueError(f"{len(ordered)} groups: an owner heartbeat holds from 1 to 15")
    out = [heartbeat.time_bits(time), format(len(ordered), f"0{COUNT_BITS}b")]
    before: int | None = None
    for g in ordered:
        if not 0 <= g.index < 1 << INDEX_BITS:
            raise ValueError(f"index {g.index} does not fit 10 bits")
        if not 0 <= g.tag < 1 << TAG_BITS:
            raise ValueError(f"tag {g.tag} does not fit 4 bits")
        if before is not None and g.index == before:
            raise ValueError(f"index {g.index} twice: no index comes twice")
        if set(g.bits) - {"0", "1"}:
            raise ValueError(f"group {g.index}: its sections are bits")
        before = g.index
        out += [format(g.index, f"0{INDEX_BITS}b"), format(g.tag, f"0{TAG_BITS}b"), _size_bits(len(g.bits)), g.bits]
    return "".join(out)


def write_body(time: dict[str, int] | None, groups: Iterable[Group]) -> bytes:
    """The body's bytes: its bits, then zero bits to the next whole byte."""
    bits = body_bits(time, groups)
    bits += "0" * (-len(bits) % 8)
    return int(bits, 2).to_bytes(len(bits) // 8, "big")


def write(stamp: zk2c.InBand, time: dict[str, int] | None, groups: Iterable[Group]) -> bytes:
    """A put's payload: form 6, CH's tag 0, the run and the seq, the body."""
    return zk2c.write("heartbeat", stamp.width, stamp.run, stamp.seq, 0, write_body(time, groups))


def write_back(put: Put) -> bytes:
    """"An implementation MUST write every owner heartbeat it accepts back
    to the same bytes.\""""
    return write(put.stamp, put.body.time, put.body.groups)


def size_of(run_width: int, sections_bits: Iterable[int]) -> int:
    """§7.5's "Sizes": "6 or 8 bytes over its body … Its body is 46 bits,
    then 14 bits and a size for each group, over the groups' sections", in
    bytes, the pad included."""
    bits = heartbeat.TIME_BITS + COUNT_BITS
    for n in sections_bits:
        bits += INDEX_BITS + TAG_BITS + 8 * snapshot.leb128_size(n) + n
    return (6 if run_width == 16 else 8) + (bits + 7) // 8

"""Core §7.5's snapshot (core 0.97, #977 E1): form 7, a collection's state in
a GET's reply, in pages. Session-free: no rule at 0.97 has an owner write
one or a reader read one (§4.2), so this module reads, writes and cuts pages
for ``conformance/snapshot/bodies.json`` and for zk2py's own tools.

**The page** is ``CH · run · asof · body``: §7.5's envelope, its asof in the
seq's place, parsed by :func:`zk2py.zk2c.parse` on a put ("A put carries a
mutation, a heartbeat, a frame or a snapshot"). The body::

    body        time · page · coll · live · tombs
    time        t32 · ms16
    page        an integer: the page's index × 2, plus 1 when a later page follows
    coll        an integer: the collection's index
    live        an integer, their count · that many live entries
    tombs       an integer, their count · that many tombstones
    live entry  id · flags · run, when flags say · seq · conf, when flags say · length · record
    tombstone   id · flags · run, when flags say · seq
    id          an integer, its length in bytes · that many bytes

"An integer is unsigned LEB128 … at most 5 bytes and at most 2^32 − 1. It is
in its shortest form" (:func:`leb128`). The flags byte: bit 7 an own run,
bits 6–5 the confirmation (0 none, 1 at the snapshot's time, 2 at ``conf``),
bit 4 the own run's width, bits 3–0 unused.

**Reading** (:func:`read_body`, :func:`read_page`) refuses "for the first of
these that applies, reading its fields in order": ``short``, ``leb128``,
``time``, ``id``, ``order``, ``flags``, ``run``, ``trailing``, and "Where
one field meets two of them, the first in this list applies"
(:class:`Refused`). **Writing** (:func:`write_page`) refuses, with the
reading's token, every page the reading would refuse ("An implementation
MUST NOT write a page that this reading refuses"), and "Every page an
implementation accepts MUST write back to the same bytes".

**Pages** (:func:`cut`): "The entries are the snapshot's live entries and
tombstones together, in the byte order of their ids … A page takes the next
entry when it holds none yet, or when its bytes with that entry, from CH to
its last tombstone, are within the page size. Otherwise the next page takes
it." :func:`cut` measures each candidate page by writing it;
:func:`cut_by_arithmetic`, a second reading, sums §7.5's sizes field by
field and never writes a page. The two must agree.

A page's value is the fixtures': ``{"tag", "run_width", "run", "asof",
"time", "index", "more", "coll", "live", "tombs"}``, a time ``{"s", "ms"}``
or None for the unknown time; a live entry ``{"id", "run_width", "run",
"seq", "confirmation", ["conf"], "record"}`` (``run_width`` and ``run`` None
for the header's, ``confirmation`` ``none``, ``time`` or ``conf``, the
record in hex); a tombstone ``{"id", "run_width", "run", "seq"}``. A
snapshot is a page without ``index`` and ``more``.
"""

from __future__ import annotations

from typing import Any, Iterable

from . import zk2c
from .lexical import is_plain_chunk

#: core §7.5: "An integer is at most 5 bytes and at most 2^32 − 1."
U32_MAX = 2**32 - 1
LEB128_MAX_BYTES = 5
#: "t32 all ones with ms16 0 says the owner's clock is unknown."
UNKNOWN_T32 = 0xFFFFFFFF
SEQ_MAX = zk2c.SEQ_MAX
#: the confirmation's two bits, by name
CONFIRMATIONS = ("none", "time", "conf")
#: the refusals, in §7.5's order: "Where one field meets two of them, the
#: first in this list applies."
TOKENS = ("short", "leb128", "time", "id", "order", "flags", "run", "trailing")

FLAG_RUN, FLAG_WIDE, FLAG_UNUSED = 0x80, 0x10, 0x0F


class Refused(ValueError):
    """A page core §7.5's reading refuses, or one its writer will not write,
    with its token (:data:`TOKENS`)."""

    def __init__(self, token: str, why: str):
        super().__init__(f"{token}: {why}")
        self.token = token


# -- the integers and the fields --------------------------------------------

def leb128(n: int) -> bytes:
    """``n`` as §7.5's integer: unsigned LEB128, the least significant group
    first, in its shortest form. Raises :class:`Refused` (``leb128``) above
    2^32 − 1."""
    if not isinstance(n, int) or n < 0:
        raise ValueError(f"an integer is unsigned: {n!r}")
    if n > U32_MAX:
        raise Refused("leb128", f"{n} is above 2^32 − 1")
    out = bytearray()
    while True:
        b = n & 0x7F
        n >>= 7
        if n:
            out.append(b | 0x80)
        else:
            out.append(b)
            return bytes(out)


def leb128_size(n: int) -> int:
    """How many bytes :func:`leb128` takes for ``n``: one per 7 bits."""
    return max(1, (n.bit_length() + 6) // 7)


class _Body:
    """A body read a field at a time."""

    def __init__(self, data: bytes):
        self.data, self.at = data, 0

    def take(self, n: int, what: str) -> bytes:
        if self.at + n > len(self.data):
            raise Refused("short", f"{what} needs {n} bytes, {len(self.data) - self.at} left")
        out = self.data[self.at:self.at + n]
        self.at += n
        return out

    def integer(self, what: str) -> int:
        """§7.5: "a reader takes a byte at a time, and stops at the fifth. A
        fifth byte with its high bit set is refused, whatever follows it."
        Then "one above 2^32 − 1, or one not in its shortest form"."""
        value = shift = 0
        for i in range(LEB128_MAX_BYTES):
            b = self.take(1, f"{what}'s byte {i + 1}")[0]
            value |= (b & 0x7F) << shift
            shift += 7
            if not b & 0x80:
                if i and b == 0:
                    raise Refused("leb128", f"{what} is not in its shortest form: its last byte is 0")
                if value > U32_MAX:
                    raise Refused("leb128", f"{what} is {value}, above 2^32 − 1")
                return value
        raise Refused("leb128", f"{what}'s fifth byte has its high bit set")

    def fixed(self, n: int, what: str) -> int:
        return int.from_bytes(self.take(n, what), "big")


def _time_of(t32: int, ms16: int, what: str) -> dict[str, int] | None:
    """§7.5's time read: None for the unknown time; ``time`` for an unused
    one, "An ms16 above 999 is unused, and so is any ms16 but 0 with t32 all
    ones"."""
    if t32 == UNKNOWN_T32:
        if ms16 == 0:
            return None
        raise Refused("time", f"{what}: t32 all ones and ms16 {ms16}, no time, known or unknown")
    if ms16 > 999:
        raise Refused("time", f"{what}: ms16 {ms16} is above 999")
    return {"s": t32, "ms": ms16}


def _time_bytes(t: dict[str, int] | None, what: str) -> bytes:
    """A time's 6 bytes, or :class:`Refused` (``time``) for one the reading
    would not read back: an ms above 999, and a known time of t32 all ones,
    "which reads as unknown"."""
    if t is None:
        return UNKNOWN_T32.to_bytes(4, "big") + b"\x00\x00"
    s, ms = t["s"], t["ms"]
    if not (isinstance(s, int) and 0 <= s <= U32_MAX and isinstance(ms, int) and 0 <= ms <= 0xFFFF):
        raise ValueError(f"{what}: t32 is 32 bits and ms16 16: {t!r}")
    if ms > 999:
        raise Refused("time", f"{what}: {ms} ms is above 999")
    if s == UNKNOWN_T32:
        raise Refused("time", f"{what}: a known time of t32 all ones reads back as the unknown time")
    return s.to_bytes(4, "big") + ms.to_bytes(2, "big")


def id_ok(raw: bytes) -> bool:
    """§7.5: "An id is empty, or plain chunks (§1.2) joined by ``/``." A
    plain chunk is ASCII, so bytes that are no ASCII are no id."""
    if not raw:
        return True
    try:
        text = raw.decode("ascii")
    except UnicodeDecodeError:
        return False
    return all(is_plain_chunk(c) for c in text.split("/"))


def member_key(reply_key: str, entry_id: str) -> str:
    """§7.5: "The member's key is the reply's key, then ``/`` and the id. An
    empty id names the reply's key itself.\""""
    return reply_key if entry_id == "" else f"{reply_key}/{entry_id}"


# -- reading -----------------------------------------------------------------

def _run_width(flags: int) -> int:
    return 32 if flags & FLAG_WIDE else 16


def _flags_check(flags: int, tomb: bool, what: str) -> int:
    """§7.5's ``flags``: "a bit from 3 to 0 set, bit 4 without bit 7,
    confirmation 3, or a tombstone's confirmation other than 0". Returns the
    confirmation's code."""
    if flags & FLAG_UNUSED:
        raise Refused("flags", f"{what}: flags {flags:02x} set a bit from 3 to 0")
    if flags & FLAG_WIDE and not flags & FLAG_RUN:
        raise Refused("flags", f"{what}: flags {flags:02x} set bit 4 without bit 7")
    conf = (flags >> 5) & 0x3
    if conf == 3:
        raise Refused("flags", f"{what}: confirmation 3 is unused")
    if tomb and conf:
        raise Refused("flags", f"{what}: a tombstone's confirmation is {conf}, not 0")
    return conf


def _entry(body: _Body, header: tuple[int, int], tomb: bool, n: int, before: bytes | None,
           live_ids: set[bytes]) -> tuple[bytes, dict[str, Any]]:
    what = f"{'tombstone' if tomb else 'live entry'} {n}"
    length = body.integer(f"{what}'s id length")
    raw = body.take(length, f"{what}'s id")
    if not id_ok(raw):
        raise Refused("id", f"{what}'s id {raw!r} is neither empty nor plain chunks joined by /")
    if before is not None and raw <= before:
        raise Refused("order", f"{what}'s id {raw!r} is not above {before!r}, the one before it")
    if tomb and raw in live_ids:
        raise Refused("order", f"{what}'s id {raw!r} is a live entry's too")
    flags = body.take(1, f"{what}'s flags")[0]
    conf = _flags_check(flags, tomb, what)
    entry: dict[str, Any] = {"id": raw.decode("ascii"), "run_width": None, "run": None}
    if flags & FLAG_RUN:
        width = _run_width(flags)
        run = body.fixed(width // 8, f"{what}'s own {width}-bit run")
        if (width, run) == header:
            raise Refused("run", f"{what}'s own run is the header's, {width}-bit {run}")
        entry["run_width"], entry["run"] = width, run
    entry["seq"] = body.fixed(3, f"{what}'s seq")
    if tomb:
        return raw, entry
    entry["confirmation"] = CONFIRMATIONS[conf]
    if conf == 2:
        t32 = body.fixed(4, f"{what}'s conf")
        ms16 = body.fixed(2, f"{what}'s conf")
        entry["conf"] = _time_of(t32, ms16, f"{what}'s conf")
    size = body.integer(f"{what}'s record length")
    entry["record"] = body.take(size, f"{what}'s record").hex()
    return raw, entry


def read_body(data: bytes, run_width: int, run: int) -> dict[str, Any]:
    """A snapshot's body read "after its envelope, with the header's run":
    ``{"time", "index", "more", "coll", "live", "tombs"}``, or
    :class:`Refused` for the first refusal reading its fields in order."""
    body = _Body(bytes(data))
    header = (run_width, run)
    t32 = body.fixed(4, "the time")
    ms16 = body.fixed(2, "the time")
    out: dict[str, Any] = {"time": _time_of(t32, ms16, "the time")}
    page = body.integer("the page")
    out["index"], out["more"] = page >> 1, bool(page & 1)
    out["coll"] = body.integer("the collection's index")
    live_ids: set[bytes] = set()
    out["live"] = []
    before: bytes | None = None
    for n in range(body.integer("the live count")):
        before, e = _entry(body, header, False, n, before, live_ids)
        live_ids.add(before)
        out["live"].append(e)
    out["tombs"] = []
    before = None
    for n in range(body.integer("the tombstone count")):
        before, e = _entry(body, header, True, n, before, live_ids)
        out["tombs"].append(e)
    if body.at != len(body.data):
        raise Refused("trailing", f"{len(body.data) - body.at} bytes after the last tombstone")
    return out


def read_page(data: bytes, key_token: str | None = None) -> dict[str, Any]:
    """A page's bytes, CH to its last tombstone: §7.5's parse on a put, which
    must be a snapshot (:class:`zk2py.zk2c.Refused` otherwise), then its
    body's reading (:class:`Refused`)."""
    env = zk2c.parse(data, "put", key_token)
    if env.form != "snapshot":
        raise zk2c.Refused("sample_kind", f"a {env.form}, where a snapshot's page was asked for")
    assert env.width is not None and env.run is not None and env.seq is not None
    body = read_body(env.record or b"", env.width, env.run)
    return {"tag": env.tag, "run_width": env.width, "run": env.run, "asof": env.seq, **body}


# -- writing -----------------------------------------------------------------

def id_bytes(entry_id: Any, what: str = "an entry") -> bytes:
    """An id's bytes, in the one encoding every writer, measure and order
    here takes: ASCII, since "An id is empty, or plain chunks (§1.2) joined
    by ``/``" and a plain chunk is ASCII (:func:`id_ok`). An id that is no
    string, holds a character outside ASCII, or is no plain chunks is
    refused ``id``, never an encoding error (the mend after round 103)."""
    if not isinstance(entry_id, str):
        raise Refused("id", f"{what}'s id {entry_id!r} is no string")
    try:
        raw = entry_id.encode("ascii")
    except UnicodeEncodeError:
        raise Refused("id", f"{what}'s id {entry_id!r} holds a character outside ASCII, which no plain chunk "
                            "does") from None
    if not id_ok(raw):
        raise Refused("id", f"{what}'s id {entry_id!r} is neither empty nor plain chunks joined by /")
    return raw


def _own_run(e: dict[str, Any], header: tuple[int, int], what: str) -> tuple[int, bytes]:
    """An entry's flags bits 7 and 4, and its own run's bytes."""
    width, run = e.get("run_width"), e.get("run")
    if width is None and run is None:
        return 0, b""
    if width not in (16, 32) or not isinstance(run, int) or not 0 <= run < 2**width:
        raise ValueError(f"{what}: an own run is 16 or 32 bits wide and fits them: {width!r}, {run!r}")
    if (width, run) == header:
        raise Refused("run", f"{what}'s own run is the header's, {width}-bit {run}")
    return FLAG_RUN | (FLAG_WIDE if width == 32 else 0), run.to_bytes(width // 8, "big")


def _seq_bytes(e: dict[str, Any], what: str) -> bytes:
    seq = e["seq"]
    if not isinstance(seq, int) or not 0 <= seq <= SEQ_MAX:
        raise ValueError(f"{what}: a seq is 24 bits: {seq!r}")
    return seq.to_bytes(3, "big")


def _write_entry(e: dict[str, Any], header: tuple[int, int], tomb: bool, n: int, before: bytes | None,
                 live_ids: set[bytes]) -> tuple[bytes, bytes]:
    """One entry's bytes, its fields checked in the reading's order."""
    what = f"{'tombstone' if tomb else 'live entry'} {n}"
    raw = id_bytes(e["id"], what)
    if before is not None and raw <= before:
        raise Refused("order", f"{what}'s id {e['id']!r} is not above the one before it")
    if tomb and raw in live_ids:
        raise Refused("order", f"{what}'s id {e['id']!r} is a live entry's too")
    out = bytearray(leb128(len(raw)) + raw)
    if tomb:
        if e.get("confirmation", "none") != "none":
            raise Refused("flags", f"{what}: a tombstone's confirmation is 0")
        conf = 0
    else:
        if e["confirmation"] not in CONFIRMATIONS:
            raise Refused("flags", f"{what}: no confirmation {e['confirmation']!r}")
        conf = CONFIRMATIONS.index(e["confirmation"])
    bits, run = _own_run(e, header, what)
    out.append(bits | conf << 5)
    out += run + _seq_bytes(e, what)
    if tomb:
        return raw, bytes(out)
    if conf == 2:
        out += _time_bytes(e.get("conf"), f"{what}'s conf")
    record = bytes.fromhex(e["record"])
    out += leb128(len(record)) + record
    return raw, bytes(out)


def _check_header(page: dict[str, Any]) -> tuple[int, int]:
    width, run, asof, tag = page["run_width"], page["run"], page["asof"], page["tag"]
    if width not in (16, 32) or not isinstance(run, int) or not 0 <= run < 2**width:
        raise ValueError(f"a header run is 16 or 32 bits wide and fits them: {width!r}, {run!r}")
    if not isinstance(asof, int) or not 0 <= asof <= SEQ_MAX:
        raise ValueError(f"asof is 24 bits: {asof!r}")
    if not isinstance(tag, int) or not 0 <= tag <= 15:
        raise ValueError(f"a tag is four bits: {tag!r}")
    return width, run


def write_body(page: dict[str, Any]) -> bytes:
    """A page's body, after its envelope, each field checked in the
    reading's order: "An implementation MUST NOT write a page that this
    reading refuses." Raises :class:`Refused` with the reading's token."""
    header = _check_header(page)
    out = bytearray(_time_bytes(page["time"], "the time"))
    index = page["index"]
    if not isinstance(index, int) or index < 0:
        raise ValueError(f"a page's index is from 0: {index!r}")
    out += leb128(index * 2 + (1 if page["more"] else 0))
    out += leb128(page["coll"])
    live_ids: set[bytes] = set()
    before: bytes | None = None
    out += leb128(len(page["live"]))
    for n, e in enumerate(page["live"]):
        before, b = _write_entry(e, header, False, n, before, live_ids)
        live_ids.add(before)
        out += b
    before = None
    out += leb128(len(page["tombs"]))
    for n, e in enumerate(page["tombs"]):
        before, b = _write_entry(e, header, True, n, before, live_ids)
        out += b
    return bytes(out)


def write_page(page: dict[str, Any]) -> bytes:
    """A page's bytes, CH to its last tombstone: §7.5's envelope, form 7
    under the collection's type's tag, asof in the seq's place, then the
    body (:func:`write_body`)."""
    body = write_body(page)
    return zk2c.write("snapshot", page["run_width"], page["run"], page["asof"], page["tag"], body)


def write_back(page: dict[str, Any]) -> bytes:
    """A page read, written again: "Every page an implementation accepts MUST
    write back to the same bytes"."""
    return write_page(page)


# -- pages -------------------------------------------------------------------

HEADER_FIELDS = ("tag", "run_width", "run", "asof", "time", "coll")


def entries(snapshot: dict[str, Any]) -> list[tuple[bytes, bool, dict[str, Any]]]:
    """The snapshot's entries, "live entries and tombstones together, in the
    byte order of their ids": ``(id bytes, tombstone, entry)``. The snapshot
    is checked first as one page of every entry, so a snapshot the reading
    would refuse is refused with its token."""
    write_body({**snapshot, "index": 0, "more": False})
    merged = [(id_bytes(e["id"]), False, e) for e in snapshot["live"]]
    merged += [(id_bytes(e["id"]), True, e) for e in snapshot["tombs"]]
    merged.sort(key=lambda x: x[0])
    return merged


def page_of(snapshot: dict[str, Any], held: Iterable[tuple[bytes, bool, dict[str, Any]]], index: int,
            more: bool) -> dict[str, Any]:
    """One page: the snapshot's CH, run, asof, time and coll, its own index
    and more bit, and the entries it holds, each list in its ids' order."""
    held = list(held)
    page = {k: snapshot[k] for k in HEADER_FIELDS}
    page.update({"index": index, "more": more, "live": [e for _, t, e in held if not t],
                 "tombs": [e for _, t, e in held if t]})
    return page


def cut(snapshot: dict[str, Any], page_bytes: int) -> list[bytes]:
    """§7.5's paging rule, measured by writing: "A page takes the next entry
    when it holds none yet, or when its bytes with that entry, from CH to its
    last tombstone, are within the page size. Otherwise the next page takes
    it." "A snapshot with no entry is one page, with none." Raises
    :class:`Refused` for a snapshot the reading would refuse."""
    merged = entries(snapshot)
    groups: list[list[tuple[bytes, bool, dict[str, Any]]]] = []
    current: list[tuple[bytes, bool, dict[str, Any]]] = []
    for e in merged:
        if current and len(write_page(page_of(snapshot, current + [e], len(groups), False))) > page_bytes:
            groups.append(current)
            current = []
        current.append(e)
    groups.append(current)
    return [write_page(page_of(snapshot, g, i, i + 1 < len(groups))) for i, g in enumerate(groups)]


def _entry_size(e: dict[str, Any], tomb: bool) -> int:
    """An entry's bytes by §7.5's sizes: its id's length and bytes, flags, an
    own run's 2 or 4, the seq's 3; a live entry adds a conf's 6, its record's
    length and bytes."""
    raw = id_bytes(e["id"])
    n = leb128_size(len(raw)) + len(raw) + 1 + 3
    if e.get("run_width") is not None:
        n += e["run_width"] // 8
    if tomb:
        return n
    rec = len(e["record"]) // 2
    return n + (6 if e["confirmation"] == "conf" else 0) + leb128_size(rec) + rec


def page_size(snapshot: dict[str, Any], index: int, live: list[int], tombs: list[int]) -> int:
    """A page's bytes by arithmetic: the envelope's 6 or 8, the time's 6, the
    page, coll and the two counts as LEB128, and its entries' sizes."""
    envelope = 1 + snapshot["run_width"] // 8 + 3
    return (envelope + 6 + leb128_size(index * 2) + leb128_size(snapshot["coll"])
            + leb128_size(len(live)) + sum(live) + leb128_size(len(tombs)) + sum(tombs))


def cut_by_arithmetic(snapshot: dict[str, Any], page_bytes: int) -> list[tuple[int, int]]:
    """The paging rule read a second way: each entry's size summed into the
    page it would join, no page written. Returns each page's (entries, size).
    "Its more bit never changes its size": index × 2 and index × 2 + 1 take
    as many LEB128 bytes."""
    merged = entries(snapshot)
    out: list[tuple[int, int]] = []
    live: list[int] = []
    tombs: list[int] = []
    for _, tomb, e in merged:
        size = _entry_size(e, tomb)
        trial = (live, tombs + [size]) if tomb else (live + [size], tombs)
        if (live or tombs) and page_size(snapshot, len(out), *trial) > page_bytes:
            out.append((len(live) + len(tombs), page_size(snapshot, len(out), live, tombs)))
            live, tombs = [], []
            trial = ([], [size]) if tomb else ([size], [])
        live, tombs = trial
    out.append((len(live) + len(tombs), page_size(snapshot, len(out), live, tombs)))
    return out


def complete(pages: list[dict[str, Any]]) -> dict[str, Any] | None:
    """§7.5's "Complete": "one page of each index from 0 to a page whose more
    bit is 0, all with one CH, run, asof, time and coll". Returns the
    snapshot, its entries in the order of the pages, or None while it is not
    complete."""
    by_index: dict[int, dict[str, Any]] = {}
    for p in pages:
        by_index.setdefault(p["index"], p)
    if 0 not in by_index:
        return None
    first = by_index[0]
    got: list[dict[str, Any]] = []
    i = 0
    while True:
        p = by_index.get(i)
        if p is None or any(p[k] != first[k] for k in HEADER_FIELDS):
            return None
        got.append(p)
        if not p["more"]:
            break
        i += 1
    snap = {k: first[k] for k in HEADER_FIELDS}
    snap["live"] = [e for p in got for e in p["live"]]
    snap["tombs"] = [e for p in got for e in p["tombs"]]
    return snap


# -- a second reading of the paging rule, cross-checked -----------------------

_CHUNK = "abcdefghijklmnopqrstuvwxyz0123456789"
_INNER = _CHUNK + "._-"


def _random_id(rng: Any) -> str:
    chunks = []
    for _ in range(rng.choice((1, 1, 1, 2, 3))):
        n = rng.choice((1, 1, 2, 3, 8, 20))
        c = rng.choice(_CHUNK)
        if n > 1:
            c += "".join(rng.choice(_INNER) for _ in range(n - 2)) + rng.choice(_CHUNK)
        chunks.append(c)
    return "/".join(chunks)


def random_snapshot(rng: Any) -> dict[str, Any]:
    """A snapshot §7.5's reading accepts, drawn from ``rng``: either run
    width, ids of one to three plain chunks (the empty id now and then),
    live entries and tombstones interleaved, own runs of either width,
    every confirmation, records from 0 to 300 bytes, and up to 300 entries,
    so that counts and lengths cross 128 and indices 64."""
    width = rng.choice((16, 32))
    run = rng.randrange(2**width)
    n = rng.choice((0, 1, 3, 20, 130, 300))
    ids = sorted({_random_id(rng) for _ in range(n)} | ({""} if rng.random() < 0.1 else set()),
                 key=id_bytes)
    horizon = rng.random() < 0.6

    def stamp() -> dict[str, Any]:
        if rng.random() < 0.8:
            return {"run_width": None, "run": None}
        w = rng.choice((16, 32))
        r = rng.randrange(2**w)
        while (w, r) == (width, run):
            r = rng.randrange(2**w)
        return {"run_width": w, "run": r}

    def when() -> dict[str, int] | None:
        return None if rng.random() < 0.1 else {"s": rng.randrange(UNKNOWN_T32), "ms": rng.randrange(1000)}

    live, tombs = [], []
    for i in ids:
        if i and rng.random() < 0.2:
            tombs.append({"id": i, **stamp(), "seq": rng.randrange(SEQ_MAX + 1)})
            continue
        conf = rng.choice(("time", "conf")) if horizon else "none"
        e: dict[str, Any] = {"id": i, **stamp(), "seq": rng.randrange(SEQ_MAX + 1), "confirmation": conf}
        if conf == "conf":
            e["conf"] = when()
        e["record"] = rng.randbytes(rng.choice((0, 1, 9, 110, 127, 128, 300))).hex()
        live.append(e)
    return {"tag": rng.randrange(16), "run_width": width, "run": run, "asof": rng.randrange(SEQ_MAX + 1),
            "time": when(), "coll": rng.choice((0, 4, 127, 128, U32_MAX)), "live": live, "tombs": tombs}


def cross_check(count: int, seed: int = 103) -> dict[str, int]:
    """:func:`cut` beside :func:`cut_by_arithmetic` on ``count`` random
    snapshots, each under a random page size: the same pages, each page's
    size as measured, each page read back and written back to its bytes, and
    the pages complete to the snapshot's entries. Returns the counts, and
    each disagreement as ``failed``."""
    import random

    rng = random.Random(seed)
    out = {"snapshots": 0, "pages": 0, "failed": 0, "over_64_pages": 0, "single_over": 0}
    for _ in range(count):
        snap = random_snapshot(rng)
        size = rng.choice((1, 16, 40, 64, 220, 1024, 4096, rng.randrange(1, 3000)))
        cut_pages = cut(snap, size)
        arith = cut_by_arithmetic(snap, size)
        read = [read_page(p) for p in cut_pages]
        held = [len(p["live"]) + len(p["tombs"]) for p in read]
        whole = complete(list(reversed(read)))
        ok = (arith == list(zip(held, [len(p) for p in cut_pages]))
              and all(write_back(r) == p for r, p in zip(read, cut_pages))
              and whole is not None and whole["live"] == snap["live"] and whole["tombs"] == snap["tombs"]
              and all(len(p) <= size or h <= 1 for p, h in zip(cut_pages, held)))
        out["snapshots"] += 1
        out["pages"] += len(cut_pages)
        out["failed"] += not ok
        out["over_64_pages"] += len(cut_pages) > 64
        out["single_over"] += sum(1 for p, h in zip(cut_pages, held) if len(p) > size and h == 1)
    return out

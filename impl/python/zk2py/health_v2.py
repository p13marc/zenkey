"""``health.v2`` (profile text 0.1, core 0.113, ``freshness.v2`` 0.12, #986):
``health.v1`` by reference, in packed records. Session-free. Each part was
written from ``spec/profiles/health/v2.md``, with v1's text beside it, and
committed before its file of ``conformance/v2/`` was opened (round 120).

"`health.v2` keeps every rule of `health.v1` and changes three things": "The
records are packed, each of one size", "The words are numbers", and "A
heartbeat answers the clock question alone". The rules v1 keeps are
:mod:`zk2py.health`'s, read here "as written, except where §2 replaces it":
"Where it names a level, it reads the level of the same name in lower case.
``UNSPECIFIED`` reads ``unknown``."

**§3, the standard schema** (:func:`schema`, :func:`record`): the packed file
``health.v2.json``, whose artifact id is :data:`SCHEMA_ID`. "``Status`` and
``Check`` have one layout, so one tag, 3. A ``Fault``'s is 11 (core §7.6). A
status's put has CH ``0x03`` under a 16-bit run, and ``0x13`` under a 32-bit
run. A fault's has ``0x0b`` or ``0x1b``." "The tag tells a status from a
check not at all. The key does."

**§2.3 and §2.6, the records** (:func:`write`, :func:`payload`,
:func:`read`, :func:`read_put`): a record written from its level, its word
and its time, and read back to them; "A record a reader refuses is unknown
too. Core §7.6's reading refuses a record by its tag, its length or its pad.
A reader MUST read a status whose record it refuses as unknown, never as
``ok``, and never at a level read from its bits" (:class:`Read`,
:func:`level_of`).

**§2.5, the fault's argument** (:func:`clock_ahead_argument`): "the offset it
measured, in whole seconds, rounded up. An offset of 65,534 s or more is
written 65,534. Where it measured none, it is ``not_available``."
"""

from __future__ import annotations

from dataclasses import dataclass
from fractions import Fraction
from functools import lru_cache
from pathlib import Path
from typing import Any

from . import jcs, packed
from . import health as v1

PROFILE = "health.v2"
IFACE = "health.v2"
REPO = Path(__file__).resolve().parents[3]
CONTRACT = REPO / "spec" / "profiles" / "health" / "health.v2.toml"
SCHEMA_PATH = REPO / "spec" / "profiles" / "health" / "health.v2.json"
#: §3: "The packed file's artifact id is …" (core §9.4)
SCHEMA_ID = "sha256:d270e1487bb6b2688aa34e5cb55804a14edc61a64e5fdd88d167b21ca4eaa983"
#: §3: "Revision 2.0 is …, published in ../.history/health.v2/ (core §9.7).
#: Its heartbeat's tag is 1 (core §7.5)."
FINGERPRINT = "sha256:b447f613bea7bdc24703500280a20bf0cf20fb27cbb38ecd25f5164b0ae005b1"
HEARTBEAT_TAG = 1

STATUS, CHECK, FAULT = "Status", "Check", "Fault"
#: §3 "The tags."
TAGS = {STATUS: 3, CHECK: 3, FAULT: 11}
#: §3 "The sizes. With its time, a status and a check are 6 bytes, and a
#: fault 8. With none, they are 10 and 26 bits: 2 and 4 bytes."
BITS = {STATUS: (10, 42), CHECK: (10, 42), FAULT: (26, 58)}
#: each type's word (§0 "Word: a status's reason, a check's detail or a
#: fault's code"), and its time
WORD_FIELD = {STATUS: "reason", CHECK: "detail", FAULT: "code"}
TIME_FIELD = {STATUS: "since", CHECK: "at", FAULT: "at"}

#: §0 "Level: one of ok, degraded and failed, in that order. unknown is not
#: a level (§2.6)." §2.6: "It is the record's code 0, v1's UNSPECIFIED."
UNKNOWN = "unknown"
LEVELS = ("ok", "degraded", "failed")
CODES = (UNKNOWN,) + LEVELS
OUT_OF_RANGE = "out_of_range"
NOT_AVAILABLE = "not_available"
#: §2.5: "An offset of 65,534 s or more is written 65,534."
ARGUMENT_MAX = 65_534


def artifact_id(document: Any) -> str:
    """Core §9.4's artifact id of a schema document, as ``desired.v2``'s is
    computed."""
    from . import desired

    return desired.artifact_id(document)


@lru_cache(maxsize=1)
def schema() -> Any:
    """The standard schema's document, strict JSON."""
    return jcs.loads(SCHEMA_PATH.read_bytes())


@lru_cache(maxsize=3)
def record(name: str) -> packed.Record:
    """One of the schema's three types, for core §7.6's codec."""
    return packed.record(schema(), name)


# -- writing (§2.3, §3) ------------------------------------------------------------------

def time_or_absent(at_ms: int | None, epoch: int | None = None) -> int | None:
    """``since`` or ``at`` as an owner writes it (§2.3): "on the owner's
    clock, in whole seconds from the deployment epoch (core §7.6)", so the
    instant's second, in Unix milliseconds as core §7.6's JSON reading has a
    time; or None, absent: "It is optional. An owner whose clock reads no
    instant the epoch codes leaves it absent. A device with no real-time
    clock at boot reads 1970. Under a catalog's later epoch that instant has
    no code, and core §7.6 forbids a record written for it." """
    if at_ms is None:
        return None
    e = packed.deployment_epoch(epoch)
    if not 1000 * e <= at_ms < 1000 * (e + (1 << 32)):
        return None
    return at_ms - (at_ms - 1000 * e) % 1000


def value_of(name: str, level: str, word: Any = 0, at_ms: int | None = None, argument: Any = NOT_AVAILABLE,
             epoch: int | None = None) -> dict[str, Any]:
    """A record's JSON reading as core §7.6 writes it: its ``level`` (one of
    :data:`CODES`), its word (a number from 0 to 62, or ``out_of_range``,
    §2.10), its time where the epoch codes it, and a fault's ``argument``
    ("It is ``not_available`` where the code takes none")."""
    out: dict[str, Any] = {"level": level, WORD_FIELD[name]: word}
    if name == FAULT:
        out["argument"] = argument
    t = time_or_absent(at_ms, epoch)
    if t is not None:
        out[TIME_FIELD[name]] = t
    return out


def write(name: str, value: dict[str, Any], epoch: int | None = None) -> bytes:
    """The record's bytes (core §7.6 "Writing"); :class:`packed.Refused` for
    a value the type has no code for."""
    return record(name).write(value, epoch)


def payload(name: str, value: dict[str, Any], run: int, seq: int, width: int = 16,
            epoch: int | None = None) -> bytes:
    """A put's payload: the record in core §7.5's envelope, CH's tag the
    type's: "A status's put has CH 0x03 under a 16-bit run, and 0x13 under a
    32-bit run. A fault's has 0x0b or 0x1b." """
    return packed.seal(record(name), value, run, seq, width, epoch)


def clock_ahead_argument(offset_s: Any) -> Any:
    """A ``clock_ahead`` fault's ``argument`` (§2.5): "the offset it
    measured, in whole seconds, rounded up. An offset of 65,534 s or more is
    written 65,534. Where it measured none, it is ``not_available``." The
    changelog's reason for rounding up: "so that an offset above 0 never
    reads 0". ``offset_s`` is None for none measured, or the seconds as an
    integer, a :class:`fractions.Fraction` or a decimal string, read
    exactly; a float is read by its shortest decimal text. A guard holds
    only for a clock ahead, so an offset below 0 is no offset measured of a
    clock ahead: it raises."""
    if offset_s is None:
        return NOT_AVAILABLE
    if isinstance(offset_s, bool):
        raise ValueError("an offset is a number of seconds")
    q = Fraction(repr(offset_s)) if isinstance(offset_s, float) else Fraction(offset_s)
    if q < 0:
        raise ValueError(f"{offset_s}: an offset below 0 is no clock ahead")
    whole = -((-q.numerator) // q.denominator)   # the ceiling, exactly
    return min(whole, ARGUMENT_MAX)


def clock_ahead_fault(offset_s: Any, at_ms: int | None = None, epoch: int | None = None) -> dict[str, Any]:
    """§2.5: "When its guard begins to hold, an owner MUST publish one fault
    with the code ``clock_ahead`` and the level ``failed``", its argument
    the offset. Its ``at`` "is the owner's clock, which is ahead during a
    drift"."""
    return value_of(FAULT, "failed", 1, at_ms, clock_ahead_argument(offset_s), epoch)


# -- reading (§2.6, §3) ------------------------------------------------------------------

@dataclass(frozen=True)
class Read:
    """One record read (§2.6): its JSON reading, or the token for which core
    §7.6 refuses it. "Core §7.6's reading refuses a record by its tag, its
    length or its pad", "never by a code" (§3 "No unused code"). ``level``
    is one of :data:`LEVELS`, or None where the record states none: at
    ``unknown``, or refused."""

    name: str
    value: dict[str, Any] | None
    refusal: str | None = None
    why: str = ""

    @property
    def refused(self) -> bool:
        return self.refusal is not None

    @property
    def level(self) -> str | None:
        """The level it states: never one "read from its bits" where the
        record is refused, and never ``ok`` for ``unknown``."""
        if self.value is None:
            return None
        lv = self.value.get("level")
        return lv if lv in LEVELS else None

    @property
    def unknown(self) -> str | None:
        """Why it states no level (§2.11 step 4): "A record at ``unknown``
        gives ``unknown_level``, and one the reader refuses gives
        ``undecodable``"; None where it states one."""
        if self.value is None:
            return "undecodable"
        return None if self.level is not None else "unknown_level"

    @property
    def word(self) -> Any:
        """Its word's number, 0 to 62, or ``out_of_range``; None where the
        record is refused."""
        return None if self.value is None else self.value.get(WORD_FIELD[self.name])

    @property
    def at_ms(self) -> int | None:
        """Its ``since`` or ``at``, in Unix milliseconds; None where it is
        absent (§5: "An absent one is shown as absent, never as the epoch")
        or the record refused."""
        return None if self.value is None else self.value.get(TIME_FIELD[self.name])

    def expect(self) -> dict[str, Any]:
        return {"refused": self.refusal} if self.value is None else dict(self.value)


def read(name: str, tag: int, data: bytes, epoch: int | None = None) -> Read:
    """A record's bytes read under CH's ``tag`` as core §7.6 reads one. "A
    marker read as 1 ends this text's reading: the record's level and words
    stand" (§3)."""
    try:
        return Read(name, record(name).read(tag, bytes(data), epoch))
    except packed.Refused as e:
        return Read(name, None, e.token, str(e).split(": ", 1)[-1])


def read_put(name: str, payload_: bytes, epoch: int | None = None, enveloped: bool = True) -> Read:
    """A put's payload read as one record: core §7.5's parse, then §7.6's
    reading under its tag. A payload the parse refuses, one with no
    envelope and one of another form state no level either: each is
    undecodable, "as for v1's payload that does not decode" (§2.6)."""
    from . import zk2c

    if not enveloped:
        return Read(name, None, "no_envelope", "a packed record travels in core §7.5's envelope")
    try:
        env = zk2c.parse(bytes(payload_), "put")
    except zk2c.Refused as e:
        return Read(name, None, e.token, str(e).split(": ", 1)[-1])
    if env.form != "mutation":
        return Read(name, None, "no_reading", f"a {env.form} is no record of {name}")
    return read(name, env.tag, env.record or b"", epoch)


def level_of(r: Read | None) -> v1.Level | None:
    """A record's level as :func:`zk2py.health.judge` takes one: the level's
    name; :class:`zk2py.health.Unknown` with ``unknown_level`` for
    ``unknown`` and ``undecodable`` for a refused record; None for no record
    read."""
    if r is None:
        return None
    return r.level if r.level is not None else v1.Unknown(r.unknown or "unknown_level")


# -- §2.10, §4: the words ----------------------------------------------------------------
#
# Written from v2.md §2.10 and §4 and committed before tables.json was opened.

REASON, DETAIL, CODE = "reason", "detail", "code"
KINDS = (REASON, DETAIL, CODE)
#: §4: the three keys, one for each kind of word
KEYS = {REASON: "health.reasons", DETAIL: "health.details", CODE: "health.codes"}
#: a record type's kind of word
KIND_OF = {STATUS: REASON, CHECK: DETAIL, FAULT: CODE}
#: §2.10's table of numbers: "0 none given; 1 to 31 the profile's; 32 to 62
#: the service's own; 63 no table's: the record's out_of_range"
PROFILE_LAST, OWN_FIRST, OWN_LAST, TABLE_MAX = 31, 32, 62, 31
#: §2.10 "The profile's tables. A number in them means what the table says,
#: from any owner."
PROFILE_TABLES: dict[str, dict[int, str]] = {
    REASON: {1: "starting", 2: "serving", 3: "stopping", 4: "check", 5: "upstream_lost", 6: "fallback",
             7: "backlog", 8: "misconfigured"},
    DETAIL: {1: "ok", 2: "unreachable", 3: "timeout", 4: "refused", 5: "cannot_run"},
    CODE: {1: "clock_ahead"},
}
#: §2.10's codes table: a code's level, and what its argument counts
PROFILE_CODE_LEVEL = {"clock_ahead": "failed"}
#: "A name is [a-z][a-z0-9_]*, as v1 §2.10's codes are."
NAME = v1.CODE_FORM
#: a contract's declaration is invalid "for the first of these"
NOT_USED, VALUE, DISAGREES = "not_used", "value", "disagrees"
#: an instance's table: "Where two hold two different lists, it has none (differ)."
DIFFER = "differ"


def is_name(x: Any) -> bool:
    return isinstance(x, str) and x.isascii() and NAME.fullmatch(x) is not None


def is_table(value: Any) -> bool:
    """§4: "a list of at most 31 names, each [a-z][a-z0-9_]*, none twice".
    A list of no name is one."""
    return (isinstance(value, list) and len(value) <= TABLE_MAX and all(is_name(x) for x in value)
            and len(set(value)) == len(value))


@dataclass(frozen=True)
class Declared:
    """One contract's table of one kind (§2.10 "A contract's table"): the
    list its resources carry, or none; and why its declaration is invalid,
    where it is: "Its declaration is invalid, and it holds none, for the
    first of these"."""

    table: tuple[str, ...] | None
    invalid: str | None = None

    def expect(self) -> dict[str, Any]:
        if self.invalid is not None:
            return {"invalid": self.invalid}
        return {"table": None if self.table is None else list(self.table)}


def contract_table(kind: str, uses: Any, annotations: Any) -> Declared:
    """A contract's table of ``kind``, from its ``uses`` and each resource's
    resolved annotations (a list of mappings, one per resource). "A resource
    that does not carry the key takes no part." In the text's order:

    1. ``not_used``: "a resource carries the key, and the contract's
       ``uses`` does not list ``health.v2``";
    2. ``value``: "a resource's value is not a list of at most 31 names,
       none twice";
    3. ``disagrees``: "two resources carry two different lists"."""
    key = KEYS[kind]
    carried = [a[key] for a in annotations if isinstance(a, dict) and key in a]
    if not carried:
        return Declared(None)
    if PROFILE not in (uses or []):
        return Declared(None, NOT_USED)
    if not all(is_table(v) for v in carried):
        return Declared(None, VALUE)
    if any(v != carried[0] for v in carried[1:]):
        return Declared(None, DISAGREES)
    return Declared(tuple(carried[0]))


def contract_tables(canonical: dict[str, Any]) -> dict[str, Declared]:
    """A revision's three tables, read from its canonical form: "The
    canonical form carries it on each (core §9.5), which is where a reader
    of a bundle finds it." """
    ann = [r.get("annotations") or {} for r in canonical.get("resources", [])]
    return {k: contract_table(k, canonical.get("uses"), ann) for k in KINDS}


@dataclass(frozen=True)
class Table:
    """An instance's table of one kind (§2.10 "An instance's table"): "the
    one table among the contracts it implements: the revisions its
    descriptor names". None where no named revision in hand holds one;
    ``differ`` where "two hold two different lists, it has none"."""

    names: tuple[str, ...] | None
    differ: bool = False

    def expect(self) -> dict[str, Any]:
        return {"table": None, "reason": DIFFER} if self.differ else \
            {"table": None if self.names is None else list(self.names)}


def instance_table(kind: str, named: Any, held: dict[str, Any]) -> Table:
    """The instance's table of ``kind``. ``named`` is the revisions its
    descriptor names, or the deployment's word across a face, each by its
    fingerprint; None where the reader holds neither: "A reader that holds
    neither the descriptor nor the deployment's word shows numbers".
    ``held`` maps a fingerprint to what the reader holds of that revision:
    its canonical form, or its :class:`Declared` table of this kind.

    "A reader reads it from those of the named revisions it holds. A
    revision it does not hold adds no table, and neither does one it holds
    that the descriptor does not name." A contract whose declaration is
    invalid "holds none", so it adds none."""
    if named is None:
        return Table(None)
    lists: list[tuple[str, ...]] = []
    for fp in named:
        rev = held.get(fp)
        if rev is None:
            continue
        d = rev if isinstance(rev, Declared) else contract_tables(rev)[kind]
        if d.table is not None and d.table not in lists:
            lists.append(d.table)
    if not lists:
        return Table(None)
    if len(lists) > 1:
        return Table(None, True)
    return Table(lists[0])


def write_word(kind: str, word: str | None, own: Table | None = None) -> Any:
    """A word as an owner writes it (§2.10 "Writing a word"), in the text's
    order: "No word given: 0. A name of the profile's table of that kind:
    its number. A name of its instance's own table: 32 plus its index. Any
    other word: ``out_of_range``. The words stay in the owner's log." An
    instance whose tables differ has none, so its "owner then writes no word
    from 32 to 62"."""
    if word is None:
        return 0
    for number, name in PROFILE_TABLES[kind].items():
        if name == word:
            return number
    if own is not None and own.names is not None and word in own.names:
        return OWN_FIRST + own.names.index(word)
    return OUT_OF_RANGE


@dataclass(frozen=True)
class Word:
    """A word as a reader shows it (§2.10 "Reading a word", §5): ``shown``
    is ``none`` (0, "shown as no word"), ``name`` (with whose table names
    it, ``profile`` or ``own``), ``number`` ("as its number where none
    does") or ``out_of_range`` ("a word no table names: never as a name, and
    never as no word")."""

    shown: str
    number: int | None = None
    name: str | None = None
    whose: str | None = None

    def expect(self) -> dict[str, Any]:
        if self.shown == "name":
            return {"name": self.name}
        if self.shown == "number":
            return {"number": self.number}
        return {"word": None} if self.shown == "none" else {"out_of_range": True}

    def text(self) -> str:
        return {"none": "no word", "name": str(self.name), "number": str(self.number),
                "out_of_range": "a word no table names"}[self.shown]


def read_word(kind: str, number: Any, own: Table | None = None) -> Word:
    """A record's word read (§2.10): "0: no word. 1 to 31: the profile's
    name, or the number where this text names none. 32 to 62: the name at
    that index of the instance's own table, or the number where the reader
    holds no table or the table ends before it. ``out_of_range``: a word no
    table names. A reader shows that, never a name." ``own`` is
    :func:`instance_table`'s: never a table "read from a revision its
    owner's descriptor does not name"."""
    if number == OUT_OF_RANGE:
        return Word("out_of_range")
    if isinstance(number, bool) or not isinstance(number, int) or not 0 <= number <= OWN_LAST:
        raise ValueError(f"{number!r}: a word is a number from 0 to 62, or out_of_range")
    if number == 0:
        return Word("none", 0)
    if number <= PROFILE_LAST:
        name = PROFILE_TABLES[kind].get(number)
        return Word("number", number) if name is None else Word("name", number, name, "profile")
    index = number - OWN_FIRST
    if own is None or own.names is None or index >= len(own.names):
        return Word("number", number)
    return Word("name", number, own.names[index], "own")


def growth(earlier: Any, later: Any) -> dict[str, Any]:
    """A table's change from one revision to a later one (§2.10 "A table
    grows by appending": "A later revision SHOULD append names, and SHOULD
    NOT remove, reorder or rename one. Then every number keeps its word
    across revisions."). ``keeps`` is whether every number the earlier table
    names keeps its word: the earlier list is a prefix of the later.
    ``change`` names what the later one did: ``same``, ``appended``,
    ``removed`` (a name gone and none in its place), ``renamed`` (a name
    gone and another at its index) or ``reordered`` (every name kept, one at
    another index). None is no table: the list of no name."""
    a, b = list(earlier or []), list(later or [])
    if b[:len(a)] == a:
        return {"keeps": True, "change": "same" if len(a) == len(b) else "appended"}
    gone = [x for x in a if x not in b]
    if not gone:
        return {"keeps": False, "change": "reordered"}
    renamed = any(i < len(b) and b[i] not in a for i, x in enumerate(a) if x in gone)
    return {"keeps": False, "change": "renamed" if renamed else "removed"}


# -- §2.2, §2.4: a late joiner's reading of the two snapshots ----------------------------
#
# Written from v2.md §2.2, §2.4 and §3 and committed before snapshots.json
# was opened.

#: §3 "In a snapshot the checks are collection 0 and the status collection
#: 1: the contract's state resources in the canonical form's order (core
#: §7.5). A snapshot's CH carries the tag 3 for either."
CHECKS_COLL, STATUS_COLL = 0, 1
#: why a reading is not one instant, the first of §2.2's three items that
#: fails
INCOMPLETE, RUN, ASOF, ABOVE_ASOF = "incomplete", "run", "asof", "above_asof"


def _entry_run(e: dict[str, Any], snap: dict[str, Any]) -> tuple[int, int]:
    """An entry's run: its own where it states one, else its snapshot's."""
    if e.get("run") is not None:
        return (e["run_width"], e["run"])
    return (snap["run_width"], snap["run"])


def one_instant(status: dict[str, Any] | None, checks: dict[str, Any] | None) -> tuple[bool, str | None]:
    """§2.2 "One instant": "A reading is one instant when all of these
    hold: both snapshots are complete (core §7.5); they are under one run,
    in width and value, and one asof; no live entry and no tombstone of that
    run has a seq above that asof. An entry that states another run is an
    earlier run's, and breaks nothing." Each argument is a complete snapshot
    as :func:`zk2py.snapshot.complete` gives one, or None for one that was
    not answered. One whose pages were not all read is given with
    ``complete`` False, and what its pages held (:func:`held`). Returns
    whether, and the first item that fails."""
    if status is None or checks is None or not (status.get("complete", True) and checks.get("complete", True)):
        return False, INCOMPLETE
    if (status["run_width"], status["run"]) != (checks["run_width"], checks["run"]):
        return False, RUN
    if status["asof"] != checks["asof"]:
        return False, ASOF
    run, asof = (status["run_width"], status["run"]), status["asof"]
    for snap in (status, checks):
        for e in list(snap["live"]) + list(snap["tombs"]):
            if _entry_run(e, snap) == run and e["seq"] > asof:
                return False, ABOVE_ASOF
    return True, None


def held(pages: list[dict[str, Any]]) -> dict[str, Any] | None:
    """One collection's snapshot as a late joiner holds it after its GET:
    the complete snapshot, ``complete`` True, where it holds "a page of each
    index from 0 to a page whose more bit is 0" (core §7.5); else the
    entries of the pages it read, under the first page's header,
    ``complete`` False; None for no page read."""
    from . import snapshot

    if not pages:
        return None
    whole = snapshot.complete(pages)
    if whole is not None:
        return dict(whole, complete=True)
    first = pages[0]
    out = {k: first[k] for k in snapshot.HEADER_FIELDS}
    out["live"] = [e for p in pages for e in p["live"]]
    out["tombs"] = [e for p in pages for e in p["tombs"]]
    out["complete"] = False
    return out


def worst_of(levels: Any) -> str | None:
    """The worst of some levels, None for none."""
    got = [lv for lv in levels if lv in LEVELS]
    return max(got, key=LEVELS.index) if got else None


@dataclass(frozen=True)
class Joined:
    """A late joiner's reading of one service's two snapshots (§2.4 "How a
    late joiner reads"):
    - ``status``: the status's record read, None where its snapshot holds
      no live entry; ``status_deleted`` where it holds a tombstone;
    - ``confirmation``: the status entry's, ``time`` ("vouched for at the
      snapshot's time"), ``conf`` ("its last confirmation") or ``none``;
    - ``checks``: each current check's record read, by its name;
      ``retired``: "each one retired within the window as a tombstone";
    - ``instant``: whether the reading is one instant, and ``why_not``;
    - ``worst``: the worst current check's level, "those the owner answers
      with a record at a level" (§2.11 step 5);
    - ``breaks``: "A status better than a check breaks the rule";
      ``report``: how a tool reports it, ``finding`` ("from one reading
      that is one instant") or ``two_readings`` ("From any other reading it
      SHOULD see the break in two readings a grace apart")."""

    status: Read | None
    status_deleted: bool
    confirmation: str | None
    checks: dict[str, Read]
    retired: tuple[str, ...]
    instant: bool
    why_not: str | None
    worst: str | None
    breaks: bool
    report: str | None


def check_name(entry_id: str) -> str:
    """A check's name from its entry's id: "It is the {check} parameter in
    the key, slugged like any value (core §1.4)"."""
    from . import slug

    name = slug.unslug(entry_id)
    return entry_id if name is None else name


def late_joiner(status: dict[str, Any] | None, checks: dict[str, Any] | None, epoch: int | None = None) -> Joined:
    """Both snapshots of one GET over ``health.v2/state/**`` read (§2.4,
    §2.2). Each record is read under its snapshot's tag: "A snapshot's CH
    carries the tag 3 for either", so one under another tag is refused,
    ``tag``, and read as unknown (§2.6). A check at ``unknown``, and one
    whose record is refused, "bound nothing"."""
    st: Read | None = None
    deleted, confirmation = False, None
    if status is not None:
        for e in status["live"]:
            st = read(STATUS, status["tag"], bytes.fromhex(e["record"]), epoch)
            confirmation = e.get("confirmation")
        if st is None and status["tombs"]:
            deleted = True
    got: dict[str, Read] = {}
    retired: tuple[str, ...] = ()
    if checks is not None:
        for e in checks["live"]:
            got[check_name(e["id"])] = read(CHECK, checks["tag"], bytes.fromhex(e["record"]), epoch)
        retired = tuple(check_name(e["id"]) for e in checks["tombs"])
    instant, why = one_instant(status, checks)
    worst = worst_of(r.level for r in got.values())
    breaks = (st is not None and st.level is not None and worst is not None
              and LEVELS.index(worst) > LEVELS.index(st.level))
    return Joined(st, deleted, confirmation, got, retired, instant, why, worst, breaks,
                  None if not breaks else ("finding" if instant else "two_readings"))


# -- §2.11, §2.12: is this service healthy? ----------------------------------------------
#
# Written from v2.md §2.1, §2.4, §2.6 to §2.8, §2.11 and §2.12, with v1
# §2.11's five steps beside them, and committed before judgements.json was
# opened.

V1, V2 = v1.IFACE, IFACE
FRESH, STALE, UNOBSERVABLE, NOT_ASKED = "fresh", v1.STALE, v1.UNOBSERVABLE, v1.NOT_ASKED


def major_of(interfaces: Any) -> str | None:
    """§2.11 step 2, §2.12: "A descriptor whose ``interfaces`` holds
    ``health.v2`` is read by this text. One that holds ``health.v1`` and not
    ``health.v2`` is read by ``health.v1``'s. One that holds neither: not
    asked, ``not_listed``. One that holds both breaks §2.3. A reader reads
    ``health.v2``." The interfaces are the descriptor's list: no interface
    token plays a part (§2.7)."""
    names = set(interfaces or [])
    return V2 if V2 in names else V1 if V1 in names else None


@dataclass(frozen=True)
class Reading:
    """One reading of one service, for §2.11's steps:
    - ``presence``: ``present``, ``absent``, ``incomplete`` or
      ``across_face``;
    - ``interfaces``: the descriptor's ``interfaces``, None for a descriptor
      not read; read only when present;
    - ``face``: ``{"status_crosses": bool}``, read only across a face, where
      "the deployment's word stands for the descriptor"; its ``major`` where
      that word names ``health.v1``;
    - ``freshness``: ``freshness.v2``'s verdict over every observation of
      ``state/status`` and its reason, as judged: ``fresh``, ``stale``,
      ``unobservable`` or ``not_asked``;
    - ``status``: the status's record read, a :class:`Read`, or None for no
      value read;
    - ``checks``: None when not read, else each current check's
      :class:`Read` by name."""

    presence: str
    interfaces: Any = None
    face: dict[str, Any] | None = None
    freshness: tuple[str, str] | None = None
    status: Read | None = None
    checks: dict[str, Read] | None = None


@dataclass(frozen=True)
class Answer:
    """"The answer is a verdict, a reason, and, for healthy and unhealthy,
    the level it rests on. The verdicts and the reasons are v1's." ``major``
    is the major the reading is judged under (§2.12), None where no
    descriptor and no deployment's word names one."""

    verdict: str
    reason: str
    level: str | None = None
    major: str | None = None

    def expect(self) -> dict[str, Any]:
        return {"verdict": self.verdict, "reason": self.reason, "level": self.level}


def judge(r: Reading) -> Answer:
    """§2.11: v1's five steps "read over this major's interface and
    records", "the first of these steps that answers"."""
    across = r.presence == "across_face"
    major: str | None = None
    # 1. Presence (§2.1: "Absent is presence's word, not a level").
    if r.presence == "absent":
        return Answer(NOT_ASKED, "absent")
    if r.presence == "incomplete":
        return Answer(UNOBSERVABLE, "presence_incomplete")
    if across:
        major = (r.face or {}).get("major", V2)
        if not (r.face or {}).get("status_crosses"):
            return Answer(UNOBSERVABLE, "face_closed", None, major)
    elif r.presence == "present":
        # 2. The descriptor (§2.7): never the interface token.
        if r.interfaces is None:
            return Answer(UNOBSERVABLE, "no_descriptor")
        major = major_of(r.interfaces)
        if major is None:
            return Answer(NOT_ASKED, "not_listed")
    else:
        raise ValueError(f"presence {r.presence!r}")
    # 3. The status's freshness, freshness.v2's verdict, as judged.
    # With no observation at all the verdict is freshness's own for none,
    # unobservable, `no_observation`. No case of judgements.json reaches
    # this step without a status (changed after the file was opened, and
    # for no case: the first writing guessed stale, `no_delivery`).
    verdict, reason = r.freshness if r.freshness is not None else (UNOBSERVABLE, "no_observation")
    if verdict == STALE:
        if across and reason == "no_delivery":
            return Answer(UNOBSERVABLE, "nothing_crossed", None, major)
        return Answer(STALE, reason, None, major)
    if verdict == UNOBSERVABLE:
        return Answer(UNOBSERVABLE, reason, None, major)
    if verdict == NOT_ASKED:
        if reason == "deleted":
            return Answer(UNOBSERVABLE, "deleted", None, major)   # "The owner broke §2.3"
        return Answer(NOT_ASKED, reason, None, major)             # last_known, core S6
    if verdict != FRESH:
        raise ValueError(f"freshness {verdict!r}")
    # 4. The status's level, then 5. the checks: "Only current checks count:
    # those the owner answers with a record at a level."
    worst = worst_of(c.level for c in (r.checks or {}).values())
    st = r.status
    if st is not None and st.level is not None:
        if worst is not None and LEVELS.index(worst) > LEVELS.index(st.level):
            return Answer(v1.UNHEALTHY, "inconsistent", worst, major)
        if st.level == "ok":
            return Answer(v1.HEALTHY, "ok", "ok", major)
        return Answer(v1.UNHEALTHY, st.level, st.level, major)
    # "With the status unknown: a current check DEGRADED or FAILED:
    # unhealthy, check, at the worst check's level"; "otherwise:
    # unobservable, unknown_level, or undecodable" (§2.6: never ok).
    if worst is not None and worst != "ok":
        return Answer(v1.UNHEALTHY, "check", worst, major)
    return Answer(UNOBSERVABLE, "unknown_level" if st is None else (st.unknown or "unknown_level"), None, major)


# -- §2.5, §5: is this service's clock ahead? --------------------------------------------
#
# Written from v2.md §2.5 and §5, core §7.5 and freshness.v2 §2.5, and
# committed before clock.json was opened.

#: what one heartbeat says of its owner's clock: core §7.5's three readings
#: where it is a read heartbeat, and ``not_read`` where it is not
INSTANT, AHEAD, TIME_UNKNOWN, NOT_READ = "instant", "ahead", "unknown", "not_read"
#: §2.5 "an owner heartbeat of the status's class, host"
STATUS_CLASS = "host"
YES, NO = "yes", "no"


@lru_cache(maxsize=1)
def standard_layout() -> Any:
    """The standard contract's heartbeat layout: "The status is the
    contract's one collection, of one member. Its section is a 32-bit digest
    and one bit, in a body of 10 bytes" (§2.4), its tag :data:`HEARTBEAT_TAG`."""
    from . import heartbeat
    from .contract import load_contract

    c = load_contract(CONTRACT)
    return heartbeat.layout_of(c.canonical, c.fingerprint)


class ClockReader:
    """What one reader's heartbeats of one service say of its clock (§2.5
    "Which heartbeats count"): "A heartbeat covering ``health.v2`` counts,
    and an owner heartbeat of the status's class, ``host`` (``freshness.v2``
    §2.4), each where it is a read heartbeat. Its sections play no part: the
    time is the owner's word of its clock. A heartbeat of another revision,
    a refused one and an older one are not read, and count for nothing."

    It holds no view and follows nothing: a read heartbeat is "one read at
    step 4" (``freshness.v2`` §2.5), which steps 1 to 3 decide, the tag, the
    body and the sequence; a read owner heartbeat is "one read at step 3,
    whatever became of its groups", so it needs no catalog. Each key has its
    own sequence. ``layout`` is the revision of ``health.v2`` the reader
    holds, the standard contract's by default."""

    def __init__(self, layout: Any = None) -> None:
        self.layout = standard_layout() if layout is None else layout
        self._last: dict[str, Any] = {}        # by key: the last read put's stamp
        #: every put heard, in arrival order: (key, what it says)
        self.heard: list[tuple[str, str]] = []
        #: the last read heartbeat's reading, None while none was read
        self.last: str | None = None
        #: which step stopped the last put that was not read: ``revision``,
        #: ``refused`` or ``older`` (clock.json's ``unread``)
        self.why: str | None = None

    def _sequence(self, key: str, stamp: Any) -> bool:
        """Step "Older": "Within one run, its seq is not after the last read
        heartbeat's, by core §4.3's serial order"."""
        from . import freshness_v2 as f2

        last = self._last.get(key)
        if last is not None and last.same_run(stamp) and not f2.serial_newer(stamp.seq, last.seq):
            return False
        self._last[key] = stamp
        return True

    def _count(self, key: str, time: Any) -> str:
        from . import heartbeat

        reading = {heartbeat.INSTANT: INSTANT, heartbeat.AHEAD: AHEAD,
                   heartbeat.UNKNOWN: TIME_UNKNOWN}[heartbeat.reading_of(time)]
        self.heard.append((key, reading))
        self.last = reading
        return reading

    def _not_read(self, key: str, why: str) -> str:
        self.heard.append((key, NOT_READ))
        self.why = why
        return NOT_READ

    def heartbeat(self, payload_: bytes, enveloped: bool = True) -> str:
        """A put on the heartbeat key covering ``health.v2``."""
        from . import heartbeat

        key = "heartbeat"
        try:
            env = heartbeat.put_of(payload_, "stream", enveloped)
        except heartbeat.Refused:
            return self._not_read(key, "refused")
        return self.heartbeat_parts(env.tag, env.stamp, env.record or b"")

    def heartbeat_parts(self, tag: int, stamp: Any, body: bytes) -> str:
        """The same, as its envelope gave it: CH's tag, the stamp and the
        body."""
        from . import heartbeat

        key = "heartbeat"
        if tag != self.layout.tag:
            return self._not_read(key, "revision")     # step 1, another revision
        try:
            value = heartbeat.read(self.layout, body)
        except heartbeat.Refused:
            return self._not_read(key, "refused")      # step 2, refused
        if not self._sequence(key, stamp):
            return self._not_read(key, "older")        # step 3, older
        return self._count(key, value["time"])         # step 4, read: "whatever its sections say"

    def owner_heartbeat(self, payload_: bytes, cls: str = STATUS_CLASS, enveloped: bool = True) -> str:
        """A put on an owner heartbeat key of this service. Only the
        ``host`` class's counts: "an owner heartbeat of the status's class".
        One of the ``link`` class says nothing here."""
        from . import owner_heartbeat as oh

        key = f"heartbeats/{cls}"
        if cls != STATUS_CLASS:
            return self._not_read(key, "class")
        try:
            put = oh.read(payload_, "stream", enveloped)
        except oh.Refused:
            return self._not_read(key, "refused")      # step 1, refused whole
        if not self._sequence(key, put.stamp):
            return self._not_read(key, "older")        # step 2, older
        return self._count(key, put.body.time)         # step 3, read: "whatever became of its groups"

    def last_read(self, stamp: Any, key: str = "heartbeat") -> None:
        """Start from a reader that last read ``stamp`` on ``key``."""
        self._last[key] = stamp

    def answer(self, presence: str = "present", interfaces: Any = (V2,)) -> tuple[str, str]:
        return clock_question(presence, interfaces, [what for _, what in self.heard])


def clock_question(presence: str, interfaces: Any, heard: Any) -> tuple[str, str]:
    """§5 "Is this service's clock ahead?" (§2.5 "How a reader reads it"),
    from what one reader heard of one service, in arrival order. Each item
    of ``heard`` is what a put says: :data:`AHEAD`, :data:`INSTANT` or
    :data:`TIME_UNKNOWN` for a read heartbeat that counts, by its time; and
    anything else for what answers nothing: :data:`NOT_READ`, a fault, a
    ``clock_ahead`` fault, a put of the status.

    "Whether it is asked is §2.11's first two steps: an absent service, and
    one whose descriptor lists neither major, are not asked. In every other
    case the heartbeats answer: a descriptor not read and a face closed to
    the status change nothing." ``interfaces`` is the descriptor's list,
    None for one not read.

    "The last heartbeat it read answers: its time is ahead: yes, ``ahead``;
    its time is an instant: no, ``confirmed``; its time is unknown:
    unobservable, ``time_unknown``; none read: unobservable,
    ``nothing_heard``." "Nothing else answers. A put of the status and the
    faults' silence say nothing of the clock. Nor does the fault."

    A service whose descriptor lists ``health.v1`` and not ``health.v2`` "is
    read by ``health.v1``'s" text (§2.11 step 2), and "Only this major
    answers yes from it: ``health.v1`` reads it as it read the unknown time"
    (§2.12): its question is :func:`zk2py.health.clock_question`'s, over the
    same hearing in v1's words. (Added after clock.json was opened, and
    pinned by no case: its case of such a descriptor "expects nothing
    more". The first writing answered such a service by this major's
    rule.)"""
    if presence == "absent":
        return NOT_ASKED, "absent"
    if presence == "present" and interfaces is not None:
        major = major_of(interfaces)
        if major is None:
            return NOT_ASKED, "not_listed"
        if major == V1:
            words = {INSTANT: "heartbeat", AHEAD: "heartbeat_held", TIME_UNKNOWN: "heartbeat_held"}
            return v1.clock_question(presence, {"lists": True}, [words.get(x, x) for x in heard])
    last = next((x for x in reversed(list(heard)) if x in (AHEAD, INSTANT, TIME_UNKNOWN)), None)
    if last == AHEAD:
        return YES, "ahead"
    if last == INSTANT:
        return NO, "confirmed"
    if last == TIME_UNKNOWN:
        return UNOBSERVABLE, "time_unknown"
    return UNOBSERVABLE, "nothing_heard"

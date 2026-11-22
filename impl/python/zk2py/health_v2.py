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

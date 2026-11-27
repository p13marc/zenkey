"""``alarms.v2`` (profile text 0.1, core 0.115, ``freshness.v2`` 0.12, #984):
``alarms.v1`` by reference, an alarm in a slot. Session-free. Each part was
written from ``spec/profiles/alarms/v2.md``, with v1's text beside it, and
committed before its file of ``conformance/v2/`` was opened (round 124).

"Every rule of ``alarms.v1`` holds for an owner, a reader and a tool of this
major, as written, except where §2 replaces it" (§0). The rules v1 keeps are
:mod:`zk2py.alarms`'s: its recipe and its sanitizing step (§2.2, §2.3), its
answers and their reasons (§2.14), "anything firing" (§2.15's last item).

**§3, the standard contract and its schemas** (:func:`schema`,
:func:`record`, :func:`schema_ids`): revision 2.0 is :data:`FINGERPRINT`,
"Its heartbeat's tag is 7"; the packed file's artifact id is
:data:`SCHEMA_ID` and the JSON Schema's :data:`WORDS_SCHEMA_ID`; "The tag is
11 (core §7.6). A record's put has CH ``0x0b`` under a 16-bit run, and
``0x1b`` under a 32-bit run."
"""

from __future__ import annotations

from functools import lru_cache
from pathlib import Path
from typing import Any

from . import jcs, packed

PROFILE = "alarms.v2"
IFACE = "alarms.v2"
REPO = Path(__file__).resolve().parents[3]
HERE = REPO / "spec" / "profiles" / "alarms"
CONTRACT = HERE / "alarms.v2.toml"
SCHEMA_PATH = HERE / "alarms.v2.json"
WORDS_SCHEMA_PATH = HERE / "schemas" / "alarms.v2.json"

#: §3: "Revision 2.0 is …, published in ../.history/alarms.v2/ (core §9.7).
#: Its heartbeat's tag is 7 (core §7.5)."
FINGERPRINT = "sha256:2292477cadf555dd09b07499ca048add4e48c63d60a209c13eaf543988c32cb7"
HEARTBEAT_TAG = 7
#: §3 "The schemas. The packed file's artifact id is …, and the JSON
#: Schema's … (core §9.4)."
SCHEMA_ID = "sha256:fe59e748fc957322428ac633a0908d09aa8b6c66952f7a9cdd8608db715a2b2a"
WORDS_SCHEMA_ID = "sha256:a3515ad47772ac778ccfbad3850a086ac18c28a9975eb7c8288c3437db9b99de"
#: §3 "The tag is 11 (core §7.6)."
RECORD_TAG = 11
ALARM = "Alarm"
WORDS = "alarms.v2.Words"

#: §3's templates, and "In a snapshot the alarms are collection 0 and the
#: words collection 1: the contract's state resources in the canonical
#: form's order (core §7.5). A member's id is its slot."
ALARMS, WORDS_PREFIX = "alarms", "words"
COLLECTION = {ALARMS: 0, WORDS_PREFIX: 1}
#: §2.7 "alarms/{slot} is the contract's one collection, of cardinality 256,
#: at a 900 s horizon."
CARDINALITY = 256
TTL_S = 900


def artifact_id(document: Any) -> str:
    """Core §9.4's artifact id of a schema document: the sha256 of its JCS
    bytes, as every other profile's schema id is computed in zk2py."""
    return jcs.jcs_id(document)


@lru_cache(maxsize=1)
def schema() -> Any:
    """The standard packed schema's document, strict JSON."""
    return jcs.loads(SCHEMA_PATH.read_bytes())


@lru_cache(maxsize=1)
def words_schema() -> Any:
    """The JSON Schema of the words, strict JSON."""
    return jcs.loads(WORDS_SCHEMA_PATH.read_bytes())


@lru_cache(maxsize=1)
def record() -> packed.Record:
    """``Alarm``, for core §7.6's codec."""
    return packed.record(schema(), ALARM)


def schema_ids() -> dict[str, str]:
    """Each standard schema's artifact id, computed from its file."""
    return {"packed": artifact_id(schema()), "jsonschema": artifact_id(words_schema())}


#: The ids the profile publishes: §3's two.
PUBLISHED_IDS = (SCHEMA_ID, WORDS_SCHEMA_ID)


def is_published(artifact: str | None) -> bool:
    return artifact in PUBLISHED_IDS


def ch(width: int) -> int:
    """§3: a record's put's CH, "``0x0b`` under a 16-bit run, and ``0x1b``
    under a 32-bit run": core §7.5's mutation, its run-width bit, the tag."""
    return (0x10 if width == 32 else 0x00) | record().tag


# -- §2.4 and §3, the record ---------------------------------------------------------------

NOT_AVAILABLE = "not_available"
FIRING, RESOLVED = "firing", "resolved"
STATES = (FIRING, RESOLVED)
#: §2.4 "``info``, ``warning`` or ``critical``, in that order, as v1's. The
#: record's escape is a severity a later text names"; its JSON reading is
#: null (core §7.6).
SEVERITIES = ("info", "warning", "critical")
#: §3's listing: rule 0 to 254, labels below 2^24, argument 0 to 1,048,574.
RULE_MAX, LABELS_MAX, ARGUMENT_MAX = 254, 16_777_215, 1_048_574
#: §3's table of sizes, by what the record carries.
SIZES = {"table": (67, 9), "labels": (91, 12), "no_at": (35, 5), "words": (131, 17)}


class Refused(ValueError):
    """An owner's refusal to write a record (§2.4 "The owner's record"),
    by token: core §7.6's writer's (``unknown``, ``missing``, ``value``,
    ``range``), or this text's own."""

    def __init__(self, token: str, message: str = "") -> None:
        super().__init__(f"{token}: {message}" if message else token)
        self.token = token


#: §2.4's reader refusals beyond core §7.6's: "``key`` is present and
#: ``rule`` is a table's (``key_with_rule``); ``key`` is absent and ``rule``
#: is ``not_available`` (``key_missing``)."
KEY_WITH_RULE, KEY_MISSING = "key_with_rule", "key_missing"
#: §2.4: an owner MUST NOT write "``labels`` or an ``argument`` other than
#: ``not_available`` under a ``rule`` of ``not_available``". The text names
#: no token. zk2py's first writing had two of its own; records.json's
#: description names one for both, ``by_words_fields``.
BY_WORDS_FIELDS = "by_words_fields"
LABELS_BY_WORDS = ARGUMENT_BY_WORDS = BY_WORDS_FIELDS


def by_words(value: dict[str, Any]) -> bool:
    return value.get("rule") == NOT_AVAILABLE


def undecodable_by_key(value: dict[str, Any]) -> str | None:
    """§2.4 "Undecodable", after core §7.6's reading: ``key_with_rule`` or
    ``key_missing``, else None."""
    if "key" in value and not by_words(value):
        return KEY_WITH_RULE
    if "key" not in value and by_words(value):
        return KEY_MISSING
    return None


def write(value: dict[str, Any], epoch: int | None = None) -> bytes:
    """An ``Alarm``'s bytes, as an owner writes it (§2.4, §3). ``value`` is
    the record's JSON reading (core §7.6): ``at`` in Unix milliseconds, whole
    seconds of them; ``key`` the alert key's 64 bits as an integer.

    Refuses, before any bit, what a reader of this text reads as
    undecodable (``key_with_rule``, ``key_missing``), ``labels`` or an
    ``argument`` other than ``not_available`` under a ``rule`` of
    ``not_available``; then whatever core §7.6's writer refuses."""
    if not isinstance(value, dict):
        raise Refused("value", "a record's value is a JSON object")
    if by_words(value) and "labels" in value:
        raise Refused(LABELS_BY_WORDS, "labels under a rule of not_available")
    if by_words(value) and value.get("argument", NOT_AVAILABLE) != NOT_AVAILABLE:
        raise Refused(ARGUMENT_BY_WORDS, "an argument under a rule of not_available")
    why = undecodable_by_key(value)
    if why is not None and "rule" in value:
        raise Refused(why, "a reader of alarms.v2 reads it as undecodable")
    try:
        return record().write(value, epoch)
    except packed.Refused as e:
        raise Refused(e.token, str(e)) from e


def payload(value: dict[str, Any], run: int, seq: int, width: int = 16, epoch: int | None = None) -> bytes:
    """A put's payload on ``alarms/{slot}``: the record in core §7.5's
    mutation envelope, CH's tag 11."""
    from . import zk2c

    return zk2c.write("mutation", width, run, seq, record().tag, write(value, epoch))


def bits(value: dict[str, Any], epoch: int | None = None) -> int:
    """The record's bits before its pad (§3's "Bits")."""
    write(value, epoch)
    return len(record().write_bits(value, epoch))


class Read:
    """A record read by a reader of this text (§2.4): its JSON reading, or
    the first reason it is undecodable, in §2.4's order: core §7.6's
    ``tag``, ``length`` and ``pad``, then ``key_with_rule`` and
    ``key_missing``. "It never reads such a record at a severity or in a
    state."

    "Under ``not_available``, a reader ignores a ``labels`` field and an
    ``argument`` that an owner wrote against this text": :attr:`value` keeps
    what the bytes say, and :meth:`meaning` drops them."""

    def __init__(self, value: dict[str, Any] | None, undecodable: str | None = None, why: str = "") -> None:
        self.value = value
        self.undecodable = undecodable
        self.why = why

    @property
    def reads(self) -> bool:
        return self.undecodable is None

    def meaning(self) -> dict[str, Any] | None:
        """The fields a reader reads, an alarm by words' ``labels`` and
        ``argument`` ignored."""
        if self.value is None:
            return None
        v = dict(self.value)
        if by_words(v):
            v.pop("labels", None)
            v["argument"] = NOT_AVAILABLE
        return v

    @property
    def state(self) -> str | None:
        return None if self.value is None else self.value["state"]

    @property
    def severity(self) -> str | None:
        """The severity, or None: an undecodable record, or the escape, "no
        severity known"."""
        return None if self.value is None else self.value["severity"]

    def __eq__(self, other: object) -> bool:
        return isinstance(other, Read) and (self.value, self.undecodable) == (other.value, other.undecodable)

    def __repr__(self) -> str:
        return f"Read({self.value!r}, {self.undecodable!r})"


def read(tag: int, data: bytes, epoch: int | None = None) -> Read:
    """A record under CH's ``tag``, read as §2.4 says."""
    try:
        v = record().read(tag, data, packed.deployment_epoch(epoch))
    except packed.Refused as e:
        return Read(None, e.token, str(e))
    why = undecodable_by_key(v)
    if why is not None:
        return Read(None, why, "the record's key disagrees with its rule (§2.4)")
    return Read(v)


def read_put(data: bytes, epoch: int | None = None) -> Read:
    """A put's payload on ``alarms/{slot}``: core §7.5's envelope, then
    :func:`read`. An envelope refused, or a form other than a mutation, is
    undecodable by its own token; a payload in no envelope has no tag
    (observer §7.3), so a JSON document put there reads ``tag``."""
    from . import zk2c

    try:
        env = zk2c.parse(data, "put")
    except zk2c.Refused as e:
        return Read(None, e.token, str(e))
    if env.form != "mutation":
        return Read(None, "no_reading", f"a {env.form} is no record of Alarm (core §7.7)")
    return read(env.tag, env.record or b"", epoch)

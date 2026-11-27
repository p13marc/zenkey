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

from dataclasses import dataclass
from fractions import Fraction
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


# -- §4, the vocabulary: a contract's table, an instance's -----------------------------------

#: §4: "``alarms.rules`` | a list of at most 255 rules".
RULES_KEY = "alarms.rules"
RULES_MAX = 255
#: §4 "``labels_width``: a rule's labels take more than 24 bits".
LABELS_WIDTH_MAX = 24
#: A contract's declaration is invalid, and it holds none, "for the first of
#: these" (§4).
NOT_USED, VALUE, RULE_NAME, LABEL, LABELS_WIDTH, HOST_LABEL, PLACEHOLDER, DISAGREES = (
    "not_used", "value", "rule_name", "label", "labels_width", "host_label", "placeholder", "disagrees")
FINDINGS = (NOT_USED, VALUE, RULE_NAME, LABEL, LABELS_WIDTH, HOST_LABEL, PLACEHOLDER, DISAGREES)
#: An instance's table: "Where two hold two different lists, it has none
#: (``differ``)".
DIFFER = "differ"
#: "**A rule** is a table with these members, and no other".
RULE_MEMBERS = {"name", "summary", "category", "argument", "labels"}
ARGUMENT_PLACEHOLDER = "argument"


def _is_str(x: Any) -> bool:
    return isinstance(x, str)


def _is_int(x: Any) -> bool:
    return isinstance(x, int) and not isinstance(x, bool)


def _decimal(x: Any) -> Fraction | None:
    """"two decimal strings as core §7.6 writes them": ``-?(0|[1-9][0-9]*)(\\.[0-9]+)?``."""
    return packed.read_decimal(x) if isinstance(x, str) else None


def _label_shape(lab: Any) -> bool:
    """§4 "A label is a table holding ``name``, a string, and one of:
    ``values``: a list of one or more strings …; ``minimum`` and
    ``maximum``: two integers, the first not above the second." zk2py reads
    "one of" as exactly one, and the table as holding nothing else."""
    if not isinstance(lab, dict) or not _is_str(lab.get("name")):
        return False
    keys = set(lab) - {"name"}
    if keys == {"values"}:
        v = lab["values"]
        return isinstance(v, list) and len(v) >= 1 and all(_is_str(x) for x in v)
    if keys == {"minimum", "maximum"}:
        lo, hi = lab["minimum"], lab["maximum"]
        return _is_int(lo) and _is_int(hi) and lo <= hi
    return False


def _rule_shape(rule: Any) -> bool:
    """§4's rule: ``name`` and ``summary`` strings, required; ``category`` a
    string; ``argument`` ``{ minimum, resolution }``, two decimal strings,
    "the resolution above 0"; ``labels`` "a list of one or more labels"."""
    if not isinstance(rule, dict) or not set(rule) <= RULE_MEMBERS:
        return False
    if not (_is_str(rule.get("name")) and _is_str(rule.get("summary"))):
        return False
    if "category" in rule and not _is_str(rule["category"]):
        return False
    if "argument" in rule:
        arg = rule["argument"]
        if not isinstance(arg, dict) or set(arg) != {"minimum", "resolution"}:
            return False
        lo, res = _decimal(arg["minimum"]), _decimal(arg["resolution"])
        if lo is None or res is None or res <= 0:
            return False
    if "labels" in rule:
        labs = rule["labels"]
        if not isinstance(labs, list) or len(labs) < 1 or not all(_label_shape(x) for x in labs):
            return False
    return True


def label_count(lab: dict[str, Any]) -> int:
    """A label's number of values: its ``values``, or the integers from
    ``minimum`` to ``maximum``."""
    return len(lab["values"]) if "values" in lab else lab["maximum"] - lab["minimum"] + 1


def label_width(lab: dict[str, Any]) -> int:
    """"A label of n values takes ⌈log₂ n⌉ bits, so a label of one value
    takes none.\""""
    n = label_count(lab)
    return (n - 1).bit_length()


def _bad_name(name: str, *bad: str) -> bool:
    return name == "" or any(b in name for b in bad)


def placeholders(summary: str) -> list[str] | None:
    """The names of a summary's placeholders, in order, or None where it
    breaks §4: "A ``{`` opens a placeholder, which ends at the next ``}``.
    The text between them is its name." A ``{`` with no ``}`` after it, or a
    ``}`` outside a placeholder, is ``placeholder``."""
    names: list[str] = []
    i = 0
    while i < len(summary):
        c = summary[i]
        if c == "}":
            return None
        if c == "{":
            j = summary.find("}", i + 1)
            if j < 0:
                return None
            names.append(summary[i + 1:j])
            i = j + 1
            continue
        i += 1
    return names


def _rule_finding(rules: list[dict[str, Any]], finding: str) -> bool:
    """Whether the list (of shape-valid rules) draws ``finding``."""
    from . import alarms as v1

    if finding == RULE_NAME:
        names = [r["name"] for r in rules]
        return any(_bad_name(n, "\n") for n in names) or len(set(names)) != len(names)
    for r in rules:
        labs = r.get("labels", [])
        if finding == LABEL:
            names = [lab["name"] for lab in labs]
            if any(_bad_name(n, "\n", "=", "{", "}") or n == ARGUMENT_PLACEHOLDER for n in names):
                return True
            if len(set(names)) != len(names):
                return True
            for lab in labs:
                vals = lab.get("values", [])
                if any("\n" in v for v in vals) or len(set(vals)) != len(vals):
                    return True
        elif finding == LABELS_WIDTH:
            if sum(label_width(lab) for lab in labs) > LABELS_WIDTH_MAX:
                return True
        elif finding == HOST_LABEL:
            if any(v1.is_host_scoped(lab["name"]) for lab in labs):
                return True
        elif finding == PLACEHOLDER:
            names = placeholders(r["summary"])
            allowed = {lab["name"] for lab in labs} | ({ARGUMENT_PLACEHOLDER} if "argument" in r else set())
            if names is None or any(n not in allowed for n in names):
                return True
    return False


@dataclass(frozen=True)
class Declared:
    """One contract's table (§4 "A contract's table"): the list its
    resources carry, or none; and why its declaration is invalid, where it
    is."""

    rules: tuple[Any, ...] | None
    invalid: str | None = None

    def expect(self) -> dict[str, Any]:
        if self.invalid is not None:
            return {"invalid": self.invalid}
        return {"table": None if self.rules is None else list(self.rules)}


def contract_table(uses: Any, annotations: Any) -> Declared:
    """A contract's table from its ``uses`` and each resource's resolved
    annotations (one mapping per resource). "A resource that does not carry
    the key takes no part." Invalid, holding none, for the first of §4's
    eight: ``not_used`` (``uses`` does not list ``alarms.v2``), ``value``,
    ``rule_name``, ``label``, ``labels_width``, ``host_label``,
    ``placeholder``, ``disagrees`` (two resources carry two different
    lists). Each of the five between is read over every list carried."""
    carried = [a[RULES_KEY] for a in annotations if isinstance(a, dict) and RULES_KEY in a]
    if not carried:
        return Declared(None)
    if PROFILE not in (uses or []):
        return Declared(None, NOT_USED)
    if not all(isinstance(v, list) and len(v) <= RULES_MAX and all(_rule_shape(r) for r in v) for v in carried):
        return Declared(None, VALUE)
    for finding in (RULE_NAME, LABEL, LABELS_WIDTH, HOST_LABEL, PLACEHOLDER):
        if any(_rule_finding(v, finding) for v in carried):
            return Declared(None, finding)
    if any(v != carried[0] for v in carried[1:]):
        return Declared(None, DISAGREES)
    return Declared(tuple(carried[0]))


def contract_table_of(canonical: dict[str, Any]) -> Declared:
    """A revision's table, from its canonical form: each resource carries
    its resolved annotations (core §9.5)."""
    ann = [r.get("annotations") or {} for r in canonical.get("resources", [])]
    return contract_table(canonical.get("uses"), ann)


@dataclass(frozen=True)
class Table:
    """An instance's table (§4 "An instance's table"): "the one table among
    the contracts it implements". ``rules`` None where none is held;
    ``differ`` where two named revisions in hand hold two different
    lists."""

    rules: tuple[Any, ...] | None
    differ: bool = False

    def expect(self) -> dict[str, Any]:
        return {"table": None, "reason": DIFFER} if self.differ else \
            {"table": None if self.rules is None else list(self.rules)}

    def rule(self, index: Any) -> dict[str, Any] | None:
        if self.rules is None or not _is_int(index) or not 0 <= index < len(self.rules):
            return None
        return self.rules[index]

    def index_of(self, name: str) -> int | None:
        for i, r in enumerate(self.rules or ()):
            if r["name"] == name:
                return i
        return None


NO_TABLE = Table(None)


def instance_table(named: Any, held: dict[str, Any]) -> Table:
    """The instance's table. ``named`` is the revisions its descriptor names,
    or the deployment's word across a face, each by fingerprint; None where
    the reader holds neither ("shows numbers"). ``held`` maps a fingerprint
    to what the reader holds of it: its canonical form or its
    :class:`Declared`. "A reader reads it from those of the named revisions
    it holds. A revision it does not hold adds no table, and neither does
    one it holds that the descriptor does not name." An invalid declaration
    holds none, so it adds none."""
    if named is None:
        return NO_TABLE
    lists: list[tuple[Any, ...]] = []
    for fp in named:
        rev = held.get(fp)
        if rev is None:
            continue
        d = rev if isinstance(rev, Declared) else contract_table_of(rev)
        if d.rules is not None and d.rules not in lists:
            lists.append(d.rules)
    if not lists:
        return NO_TABLE
    if len(lists) > 1:
        return Table(None, True)
    return Table(lists[0])


# -- §2.4 and §4, writing and reading by a table -------------------------------------------

#: A record read back by a table is **unnamed** "for the first of these"
#: (§2.4): ``no_table``, ``rule``, ``labels``.
NO_TABLE_REASON, RULE_REASON, LABELS_REASON = "no_table", "rule", "labels"


def label_text(lab: dict[str, Any], code: int) -> str:
    """"A label's text is its value, or its integer in decimal: ``-`` for a
    negative one, and no leading zero.\""""
    return lab["values"][code] if "values" in lab else str(lab["minimum"] + code)


def _int_code(lab: dict[str, Any], text: str) -> int | None:
    import re

    if not re.fullmatch(r"-?(0|[1-9][0-9]*)", text) or text == "-0":
        return None
    v = int(text)
    return v - lab["minimum"] if lab["minimum"] <= v <= lab["maximum"] else None


def code_of(lab: dict[str, Any], text: Any) -> int | None:
    if not isinstance(text, str):
        return None
    if "values" in lab:
        return lab["values"].index(text) if text in lab["values"] else None
    return _int_code(lab, text)


def pack_labels(labs: list[dict[str, Any]], codes: list[int]) -> int:
    """"``labels`` holds the codes of the rule's labels in list order, the
    first in the most significant bits, with no bit between them.\""""
    v = 0
    for lab, c in zip(labs, codes):
        v = (v << label_width(lab)) | c
    return v


def unpack_labels(labs: list[dict[str, Any]], value: int) -> list[int] | None:
    """The codes in ``value``, or None where it is no such integer: at or
    above 2^w, or holding "a code a label does not have"."""
    w = sum(label_width(lab) for lab in labs)
    if not _is_int(value) or not 0 <= value < (1 << w):
        return None
    codes: list[int] = []
    shift = w
    for lab in labs:
        shift -= label_width(lab)
        c = (value >> shift) & ((1 << label_width(lab)) - 1)
        if c >= label_count(lab):
            return None
        codes.append(c)
    return codes


def argument_code(rule: dict[str, Any], reading: Any) -> Any:
    """§4 "The argument": "An owner writes a reading as round((reading −
    ``minimum``) / ``resolution``), a tie to the even code, as core §7.6
    writes a ``fixed``. A reading whose code is outside 0 to 1,048,574 is
    written ``not_available``, and so is none." "A rule with no
    ``argument`` writes ``not_available``." A reading is a decimal string or
    a JSON number, read by its shortest decimal text."""
    if "argument" not in rule or reading is None:
        return NOT_AVAILABLE
    x = _reading(reading)
    if x is None:
        return NOT_AVAILABLE
    lo, res = _decimal(rule["argument"]["minimum"]), _decimal(rule["argument"]["resolution"])
    assert lo is not None and res is not None
    code = packed.round_half_even((x - lo) / res)
    return code if 0 <= code <= ARGUMENT_MAX else NOT_AVAILABLE


def _reading(reading: Any) -> Fraction | None:
    if isinstance(reading, bool):
        return None
    if isinstance(reading, int):
        return Fraction(reading)
    if isinstance(reading, float):
        return packed.read_decimal(repr(reading)) or Fraction(repr(reading))
    if isinstance(reading, str):
        return packed.read_decimal(reading)
    return None


def argument_reading(rule: dict[str, Any], code: Any) -> str | None:
    """"The argument reads as ``minimum`` + code × ``resolution``, in exact
    decimal arithmetic, in its shortest form"; None for ``not_available``,
    and under a rule with no argument ("A reader ignores any other code
    under such a rule")."""
    if "argument" not in rule or not _is_int(code):
        return None
    lo, res = _decimal(rule["argument"]["minimum"]), _decimal(rule["argument"]["resolution"])
    assert lo is not None and res is not None
    return packed.shortest(lo + code * res)


def summary_of(rule: dict[str, Any], labels: dict[str, str], argument: str | None) -> str:
    """"The summary is the rule's template with each placeholder replaced":
    ``{argument}`` by the argument's reading, "or by ``?`` where the
    record's is ``not_available``", ``{<label>}`` by that label's text."""
    out: list[str] = []
    s = rule["summary"]
    i = 0
    while i < len(s):
        if s[i] == "{":
            j = s.index("}", i + 1)
            name = s[i + 1:j]
            out.append(("?" if argument is None else argument) if name == ARGUMENT_PLACEHOLDER else labels[name])
            i = j + 1
        else:
            out.append(s[i])
            i += 1
    return "".join(out)


def table_alert_key(rule: dict[str, Any], labels: dict[str, str]) -> str:
    """"The alert key of a table's alarm is v1's recipe over the rule's
    ``name`` and each label's name and text.\""""
    from . import alarms as v1

    return v1.derive(rule["name"], labels).key


def key_int(alert_key: str) -> int:
    """§3: the record's ``key``, "the alert key's 16 hexadecimal digits read
    as one number"."""
    return int(alert_key, 16)


def key_hex(key: int) -> str:
    return f"{key:016x}"


@dataclass(frozen=True)
class Written:
    """An alarm's fields as an owner writes them (§2.4): by its table, or by
    words with its words document."""

    rule: Any
    labels: int | None
    argument: Any
    key: int | None
    words: dict[str, Any] | None = None

    @property
    def by_words(self) -> bool:
        return self.rule == NOT_AVAILABLE

    def fields(self) -> dict[str, Any]:
        out: dict[str, Any] = {"rule": self.rule, "argument": self.argument}
        if self.labels is not None:
            out["labels"] = self.labels
        if self.key is not None:
            out["key"] = self.key
        return out


def words_document(rule: str, labels: dict[str, str] | None, summary: str, category: str | None) -> dict[str, Any]:
    """An ``alarms.v2.Words`` document: "``rule``, ``summary``, ``category``
    and ``labels`` mean what v1 §2.4 says, host-scoped labels included"."""
    doc: dict[str, Any] = {"rule": rule, "summary": summary}
    if labels:
        doc["labels"] = dict(labels)
    if category is not None:
        doc["category"] = category
    return doc


def write_by(table: Table, rule: str, labels: dict[str, str] | None, reading: Any = None,
             summary: str | None = None, category: str | None = None) -> Written:
    """An alarm written as §2.4 says. "By the table" where "the rule's name
    is a table's rule, the alarm's discriminating labels are exactly the
    labels that rule declares, and each one's text has a code": ``rule`` the
    index, ``labels`` the codes, ``argument`` by §4, no ``key``. "Host-scoped
    labels are not written." Otherwise by words: ``rule`` and ``argument``
    ``not_available``, no ``labels``, ``key`` the alert key (v1's recipe,
    through the sanitizing step), and its words document, ``summary`` and
    ``category`` the alarm's own."""
    from . import alarms as v1

    disc = v1.discriminating(labels)
    i = table.index_of(rule)
    if i is not None:
        r = table.rules[i]  # type: ignore[index]
        labs = r.get("labels", [])
        if set(disc) == {lab["name"] for lab in labs}:
            codes = [code_of(lab, disc[lab["name"]]) for lab in labs]
            if all(c is not None for c in codes):
                return Written(i, pack_labels(labs, codes) if labs else None,  # type: ignore[arg-type]
                               argument_code(r, reading), None)
    key = v1.derive(rule, labels).key
    return Written(NOT_AVAILABLE, None, NOT_AVAILABLE, key_int(key),
                   words_document(rule, labels, summary if summary is not None else "", category))


@dataclass(frozen=True)
class Named:
    """A record read back by a table (§2.4, §4): its words and its alert
    key, or unnamed and why. An alarm by words is named by its ``key``
    alone here: its words are :func:`take_words`'s."""

    rule: str | None = None
    labels: dict[str, str] | None = None
    summary: str | None = None
    category: str | None = None
    argument: str | None = None
    alert_key: str | None = None
    unnamed: str | None = None
    by_words: bool = False

    @property
    def named(self) -> bool:
        return self.alert_key is not None

    def shown(self) -> str:
        """§5: a named alarm "by its rule's name, its labels and its
        summary"; an unnamed one "by its numbers, as in ``rule 1, labels
        2``"; an alarm by words with no words "by its alert key"."""
        if self.rule is not None:
            return self.summary or self.rule
        return self.alert_key or ""


def name_record(table: Table, fields: dict[str, Any]) -> Named:
    """A record that reads, named by the owner's table (§2.4 "Unnamed is
    not undecodable"): unnamed "for the first of these": ``no_table``,
    ``rule`` (no rule at the index), ``labels`` ("present where the rule
    declares no label, or absent where it declares one, or holds a code a
    label does not have"). An alarm by words is named by its ``key``."""
    if fields.get("rule") == NOT_AVAILABLE:
        return Named(alert_key=key_hex(fields["key"]), by_words=True)
    if table.rules is None:
        return Named(unnamed=NO_TABLE_REASON)
    r = table.rule(fields.get("rule"))
    if r is None:
        return Named(unnamed=RULE_REASON)
    labs = r.get("labels", [])
    has = "labels" in fields
    if has != bool(labs):
        return Named(unnamed=LABELS_REASON)
    codes = unpack_labels(labs, fields["labels"]) if labs else []
    if codes is None:
        return Named(unnamed=LABELS_REASON)
    texts = {lab["name"]: label_text(lab, c) for lab, c in zip(labs, codes)}
    arg = argument_reading(r, fields.get("argument"))
    return Named(rule=r["name"], labels=texts, summary=summary_of(r, texts, arg), category=r.get("category"),
                 argument=arg, alert_key=table_alert_key(r, texts))


def numbers_of(fields: dict[str, Any]) -> str:
    """§5's "``rule 1, labels 2``": an unnamed alarm shown by its numbers."""
    out = f"rule {fields.get('rule')}"
    if "labels" in fields:
        out += f", labels {fields['labels']}"
    return out


# -- §4, a table's growth --------------------------------------------------------------------

SAME, KEPT, MOVED = "same", "kept", "moved"


def growth(earlier: list[Any], later: list[Any]) -> str:
    """"A table grows as ``same``, ``kept`` or ``moved``" (the changelog's
    words; §4: "A later revision SHOULD append rules, and SHOULD NOT remove
    or reorder one, nor change a rule's name or its labels … A rule's
    summary, category and argument MAY change in place"). ``same``: the two
    lists equal; ``kept``: every index of the earlier keeps its rule's name
    and labels in the later; ``moved``: any other."""
    if list(earlier) == list(later):
        return SAME
    if len(later) < len(earlier):
        return MOVED
    for a, b in zip(earlier, later):
        if a["name"] != b["name"] or a.get("labels") != b.get("labels"):
            return MOVED
    return KEPT

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


# -- §2.2, §2.12 and §4, the keys --------------------------------------------------------------

TOKEN = "state"


def alert_key(rule: str, labels: dict[str, str] | None) -> dict[str, Any]:
    """An alarm's alert key over its words (§0: "v1 §2.2's and §2.3's
    recipe over that rule's name and those labels as text. It is 64 bits,
    written as 16 hexadecimal digits"), as digits and as the integer a
    record's ``key`` carries (§3)."""
    from . import alarms as v1

    k = v1.derive(rule, labels).key
    return {"alert_key": k, "key": key_int(k)}


def slot_key(system: str, service: str, slot: int, prefix: str = ALARMS) -> str | None:
    """§2.2: ``zk2/<system>/<service>/alarms.v2/state/alarms/<slot>``, the
    slot "written in decimal with no leading zero, as core §2.2 writes a
    ``uint``"; ``words/<slot>`` beside it. None where the system or the
    service is no name, or the slot no ``uint``."""
    from . import keys
    from .lexical import is_plain_chunk

    if not (is_plain_chunk(system) and is_plain_chunk(service)) or not _is_int(slot) or slot < 0:
        return None
    key = f"zk2/{system}/{service}/{IFACE}/{TOKEN}/{prefix}/{slot}"
    return key if keys.parse(key) is not None else None


def parse_slot_chunk(chunk: str, bound: int = CARDINALITY) -> int | None:
    """A key's last chunk read as a slot: decimal, no leading zero, "below
    the instance's bound: 256, or the cardinality its descriptor states"
    (§2.2). "A member whose last chunk is no such number is on no slot":
    None."""
    import re

    if re.fullmatch(r"0|[1-9][0-9]*", chunk) is None:
        return None
    v = int(chunk)
    return v if v < bound else None


def slot_of(key: str, bound: int = CARDINALITY, prefix: str = ALARMS) -> dict[str, Any] | None:
    """The owner and the slot a key of ``prefix`` names, or None: another
    major, token or template, or a last chunk on no slot."""
    from . import keys

    p = keys.parse(key)
    if p is None or p["form"] != "data" or p["iface"] != IFACE or p["token"] != TOKEN:
        return None
    if len(p["resource"]) != 2 or p["resource"][0] != prefix:
        return None
    slot = parse_slot_chunk(p["resource"][1], bound)
    if slot is None:
        return None
    return {"system": p["system"], "service": p["service"], "slot": slot}


def collection_of(system: str, service: str) -> str | None:
    """§2.12 "A reference names no key": "A reader finds the alarm by a
    collection read of its owner's
    ``zk2/<system>/<service>/alarms.v2/state/alarms/**``". None where the
    reference's system or service is no name."""
    from .lexical import is_plain_chunk

    if not (is_plain_chunk(system) and is_plain_chunk(service)):
        return None
    return f"zk2/{system}/{service}/{IFACE}/{TOKEN}/{ALARMS}/**"


# -- §2.2, §2.8 and §2.9, the owner's slots ----------------------------------------------------

REFUSED = "refused"


@dataclass
class Held:
    """What an instance holds on one slot (§0 "Holds: an instance holds a
    record on a slot from its put of it, until it puts another identity's
    there"): the identity and whether its record fires."""

    identity: str
    firing: bool


class Slots:
    """The slots one instance holds, for §2.2's rule: "The identity takes:
    1. the slot on which its instance holds a record of that identity,
    firing or resolved; 2. else the lowest slot on which its instance holds
    no firing record: one it holds nothing on, or one whose record is
    resolved." The bound is 256, or the descriptor's (§2.8). "Where every
    slot fires, the raise is refused." An identity is its alert key, or any
    value two unnamed records share."""

    def __init__(self, bound: int = CARDINALITY) -> None:
        self.bound = bound
        self.held: dict[int, Held] = {}

    def slot_of(self, identity: str) -> int | None:
        for s, h in self.held.items():
            if h.identity == identity:
                return s
        return None

    def take(self, identity: str) -> int | None:
        """The slot §2.2 gives ``identity`` now, or None where every slot
        below the bound fires for another."""
        s = self.slot_of(identity)
        if s is not None:
            return s
        for s in range(self.bound):
            h = self.held.get(s)
            if h is None or not h.firing:
                return s
        return None

    def raise_(self, identity: str) -> int | str:
        """A raise (or an escalation, or a refresh): the slot it is put on,
        or :data:`REFUSED` at the bound. "It MUST NOT resolve a firing alarm
        to make room, nor put over one" (§2.8)."""
        s = self.take(identity)
        if s is None:
            return REFUSED
        self.held[s] = Held(identity, True)
        return s

    def resolve(self, identity: str) -> int | None:
        """A resolve: "the record, and no delete" on the identity's slot,
        which it then holds resolved; None where the identity holds no
        firing record (nothing to resolve)."""
        s = self.slot_of(identity)
        if s is None or not self.held[s].firing:
            return None
        self.held[s] = Held(identity, False)
        return s

    def firing(self) -> dict[int, str]:
        return {s: h.identity for s, h in sorted(self.held.items()) if h.firing}


# A restart's adoption (§2.9), slot by slot: "For each slot it finds, in this order".
FREE_RESOLVED, FREE_UNREADABLE, PUT_RESOLVED, ADOPT, PAST_BOUND, FREE_TWICE = (
    "free_resolved", "free_unreadable", "put_resolved", "adopt", "past_bound", "free_duplicate")


@dataclass(frozen=True)
class Kept:
    """One slot of an owner's persistent record (§2.9 "What the record
    holds": "by slot, each record as put, its alarm's words and its alert
    key, a resolved one included"). ``record`` is the record's JSON reading,
    or None where it does not read; ``rule`` and ``labels`` the words (the
    rule's name and each label's text, host-scoped ones of an alarm by words
    included); ``alert_key`` as stored."""

    slot: int
    record: dict[str, Any] | None
    rule: str | None
    labels: dict[str, str] | None
    alert_key: str | None
    summary: str | None = None
    category: str | None = None


def derives(k: Kept) -> bool:
    """Whether a kept slot's words derive its alert key: v1's recipe, through
    the sanitizing step, over the words gives the stored alert key; for an
    alarm by words, the record's ``key`` too."""
    from . import alarms as v1

    if k.rule is None or k.alert_key is None:
        return False
    key = v1.derive(k.rule, k.labels).key
    if key != k.alert_key:
        return False
    if k.record is not None and by_words(k.record) and k.record.get("key") != key_int(key):
        return False
    return True


@dataclass(frozen=True)
class Adoption:
    """What a restart does with one kept slot: ``action``, and for a put the
    record put (written by the new table where it names the words, else by
    words)."""

    slot: int
    action: str
    record: dict[str, Any] | None = None


@dataclass(frozen=True)
class Found:
    """What a restart finds of one kept slot, as §2.9's steps read it: the
    identity, whether it fires, whether its entry reads, whether its words
    derive its alert key, and whether the new instance still evaluates its
    rule."""

    slot: int
    identity: str | None
    firing: bool
    reads: bool
    derives: bool
    evaluated: bool


def adoption(found: list[Found], bound: int = CARDINALITY) -> dict[int, str]:
    """§2.9's steps for each slot, in slot order: resolved, free; it does
    not read or its words do not derive its key, free; then "One identity
    firing on two slots of the record … The owner adopts it on the lowest of
    them, by the steps above, and puts nothing on the others: they are
    free"; "Past a lowered bound, a slot at or above the new bound is
    neither put nor resolved" (``past_bound``); a rule no longer evaluated,
    put resolved; else adopted."""
    out: dict[int, str] = {}
    seen: set[str] = set()
    for f in sorted(found, key=lambda x: x.slot):
        if not f.firing:
            out[f.slot] = FREE_RESOLVED
        elif not f.reads or not f.derives or f.identity is None:
            out[f.slot] = FREE_UNREADABLE
        elif f.identity in seen:
            out[f.slot] = FREE_TWICE
        elif f.slot >= bound:
            seen.add(f.identity)
            out[f.slot] = PAST_BOUND
        else:
            seen.add(f.identity)
            out[f.slot] = ADOPT if f.evaluated else PUT_RESOLVED
    return out


def adopt(kept: list[Kept], evaluates: Any, table: Table, bound: int = CARDINALITY) -> list[Adoption]:
    """:func:`adoption` over an owner's persistent record, and each put's
    record: "By name, never by index": written by the new table where it
    names the words, else by words, its ``at`` kept. ``evaluates`` is a
    predicate on a rule's name, or the set of the names the new instance
    evaluates."""
    ev = evaluates if callable(evaluates) else (lambda name, s=set(evaluates): name in s)
    found = []
    for k in kept:
        r = k.record
        reads = r is not None and undecodable_by_key(r) is None
        found.append(Found(k.slot, k.alert_key, r is not None and r.get("state") == FIRING if r else True, reads,
                           reads and derives(k), k.rule is not None and ev(k.rule)))
    acts = adoption(found, bound)
    out: list[Adoption] = []
    for k in sorted(kept, key=lambda x: x.slot):
        act = acts[k.slot]
        if act not in (ADOPT, PUT_RESOLVED):
            out.append(Adoption(k.slot, act))
            continue
        assert k.record is not None and k.rule is not None
        r = k.record
        w = write_by(table, k.rule, k.labels, None, k.summary, k.category)
        rec = {key: v for key, v in r.items() if key not in ("rule", "labels", "argument", "key")}
        rec.update(w.fields())
        if not w.by_words and not by_words(r) and _declares_argument(table, k.rule):
            # The record's argument code is kept where the new table's rule
            # declares an argument: the text says only "its at kept".
            rec["argument"] = r.get("argument", NOT_AVAILABLE)
        if act == PUT_RESOLVED:
            rec["state"] = RESOLVED
        out.append(Adoption(k.slot, act, rec))
    return out


def restarted(found: list[Found], bound: int = CARDINALITY) -> Slots:
    """The new instance's slots after its adoption: "The new instance holds
    what it put, and nothing else": each adopted alarm firing, each one put
    resolved."""
    new = Slots(bound)
    for slot, act in adoption(found, bound).items():
        f = next(x for x in found if x.slot == slot)
        if act in (ADOPT, PUT_RESOLVED):
            assert f.identity is not None
            new.held[slot] = Held(f.identity, act == ADOPT)
    return new


def _declares_argument(table: Table, rule: str) -> bool:
    i = table.index_of(rule)
    return i is not None and table.rules is not None and "argument" in table.rules[i]


# -- §2.6, the reader's slot for one identity --------------------------------------------------

@dataclass(frozen=True)
class SlotSeen:
    """One slot a reader holds for an identity: its slot, and the run and
    seq of the put (or snapshot entry) that carried it. A run is compared in
    width and value: "So "00a1" and "000000a1" are two runs" (core §3.3),
    so it is held as its hex digits, or as a (width, value) pair."""

    slot: int
    run: Any
    seq: int


def reader_slot(seen: list[SlotSeen], present_run: Any) -> int | None:
    """§2.6 "One identity on two slots": "A reader takes the record under
    the run its owner's present descriptor states (core §3.3). Where no slot
    is under that run, it takes none … Two under that one run break §2.2.
    The reader takes the later by seq … A slot is taken only under the
    present run. So a reader with no run stated, as across a face, takes no
    slot." Read for one slot too: one under another run is taken by
    none."""
    if present_run is None:
        return None
    under = [x for x in seen if x.run == present_run]
    if not under:
        return None
    return max(under, key=lambda x: x.seq).slot


def doubles(seen: dict[str, list[SlotSeen]], present_run: Any) -> list[str]:
    """The alert keys two slots of the present run name: §2.6's "Two under
    that one run break §2.2 … a tool MAY report the break (§5)"."""
    if present_run is None:
        return []
    return sorted(k for k, xs in seen.items() if sum(1 for x in xs if x.run == present_run) > 1)


def one_slot_each(slots: dict[int, tuple[str | None, int]], present_run: int | None, established: bool) -> str:
    """§5 "Does this owner hold each identity on one slot?": yes, "No two
    slots of its present run name one alert key"; no where two do;
    unobservable where "its alarms are not established". ``slots`` maps a
    slot to its alarm's alert key (None for an unnamed one, "not compared")
    and its record's run."""
    if not established:
        return "unobservable"
    keys_ = [k for k, run in slots.values() if k is not None and run == present_run]
    return "yes" if len(keys_) == len(set(keys_)) else "no"


# -- §2.4 and §5, a slot's words ----------------------------------------------------------------

#: §2.4 "Otherwise it takes none, for the first of these", and §5's third
#: question's tokens.
NO_RECORD, UNDECODABLE, TABLE_RULE, NO_WORDS, UNREADABLE, MISMATCH = (
    "no_record", "undecodable", "table", "no_words", "unreadable", "mismatch")
STRICT, SANITIZED = "strict", "sanitized"


def words_payload(doc: dict[str, Any], run: int, seq: int, width: int = 16) -> bytes:
    """A words put's payload: "strict JSON, in the envelope a state put
    carries (core §7.5)", CH's tag 0, "a JSON type has no tag"."""
    from . import zk2c

    return zk2c.write("mutation", width, run, seq, 0, jcs.dumps(doc))


def in_words_schema(value: Any) -> bool:
    """"inside the ``Words`` schema" (§2.4), by core §7.3's subset."""
    from . import alarms as v1

    root = words_schema()
    return v1.validate(value, {"$ref": f"#/$defs/{WORDS}"}, root)


def read_words(value: Any) -> dict[str, Any] | None:
    """A words document's value, or None where it does not read: "strict
    JSON as v1 §2.4 reads a payload, inside the ``Words`` schema". ``value``
    is the payload's bytes (its envelope's record, the JSON) or the decoded
    value."""
    from . import alarms as v1

    if isinstance(value, (bytes, bytearray)):
        try:
            value = v1.strict_json(bytes(value))
        except v1.NotStrict:
            return None
    if not isinstance(value, dict) or not in_words_schema(value):
        return None
    return value


def read_words_put(data: bytes) -> dict[str, Any] | None:
    """A put on ``words/{slot}``: its envelope (CH 0x00), then
    :func:`read_words` over the record's bytes."""
    from . import zk2c

    try:
        env = zk2c.parse(data, "put")
    except zk2c.Refused:
        return None
    if env.form != "mutation" or env.tag != 0:
        return None
    return read_words(env.record or b"")


@dataclass(frozen=True)
class Taken:
    """§2.4 "A reader takes words by the key" and §5's "Do this slot's
    words derive its record's key?": ``taken`` with how (``strict`` or
    ``sanitized``), or the first reason it takes none."""

    taken: bool
    how: str | None = None
    reason: str | None = None

    def answer(self) -> dict[str, str]:
        """§5's verdict: yes (strict, sanitized), no (mismatch), not asked
        (table, no_words, no_record), unobservable (unreadable,
        undecodable)."""
        if self.taken:
            return {"answer": "yes", "reason": self.how or ""}
        if self.reason == MISMATCH:
            return {"answer": "no", "reason": MISMATCH}
        if self.reason in (TABLE_RULE, NO_WORDS, NO_RECORD):
            return {"answer": "not_asked", "reason": self.reason}
        return {"answer": "unobservable", "reason": self.reason or ""}


def take_words(record: Read | None, words: Any = None, words_heard: bool | None = None) -> Taken:
    """Whether a reader takes a slot's words as the words of the slot's
    alarm. ``record`` is the slot's record as read (None where it holds
    none); ``words`` the words it read: a document, the payload's JSON bytes,
    or None where it read none. In §2.4's order: ``no_record``,
    ``undecodable``, ``table``, ``no_words``, ``unreadable``, ``mismatch``.
    Taken where "v1's recipe over the words, through the sanitizing step,
    gives the record's ``key``": ``strict`` where the strict recipe does
    (v1 §2.3's framed input), else ``sanitized``."""
    from . import alarms as v1

    if record is None:
        return Taken(False, reason=NO_RECORD)
    if not record.reads:
        return Taken(False, reason=UNDECODABLE)
    assert record.value is not None
    if not by_words(record.value):
        return Taken(False, reason=TABLE_RULE)
    if words is None if words_heard is None else not words_heard:
        return Taken(False, reason=NO_WORDS)
    doc = read_words(words)
    if doc is None:
        return Taken(False, reason=UNREADABLE)
    key = key_hex(record.value["key"])
    if v1.derive(doc["rule"], doc.get("labels")).key != key:
        return Taken(False, reason=MISMATCH)
    strict = v1.strict_key(doc["rule"], doc.get("labels"))
    return Taken(True, STRICT if strict == key else SANITIZED)


class SlotView:
    """What a tool shows of one slot as its puts arrive, in either order
    (§2.4 "The two puts ride two priorities, so a reader can receive the
    record first … It then shows the alarm by its alert key until its words
    arrive"; §5): by its words (a table's or taken), by its alert key, by its
    numbers where unnamed, or nothing."""

    def __init__(self, table: Table = NO_TABLE) -> None:
        self.table = table
        self.record: Read | None = None
        self.words: Any = None

    def put_record(self, r: Read) -> None:
        self.record = r if r.reads else None

    def put_words(self, w: Any) -> None:
        self.words = w

    def shown(self) -> dict[str, Any]:
        """``{"words": {rule, labels, summary, category?}}``,
        ``{"alert_key": …}``, ``{"unnamed": reason, "numbers": …}``, or
        ``{"nothing": …}`` where no record reads."""
        if self.record is None or self.record.value is None:
            return {"nothing": NO_RECORD}
        v = self.record.value
        if not by_words(v):
            n = name_record(self.table, v)
            if n.unnamed is not None:
                return {"unnamed": n.unnamed, "numbers": numbers_of(v)}
            out: dict[str, Any] = {"rule": n.rule, "labels": n.labels, "summary": n.summary}
            if n.category is not None:
                out["category"] = n.category
            return {"words": out, "alert_key": n.alert_key}
        t = take_words(self.record, self.words)
        if not t.taken:
            return {"alert_key": key_hex(v["key"])}
        doc = read_words(self.words)
        assert doc is not None
        return {"words": doc, "alert_key": key_hex(v["key"])}


# -- §2.6 and §2.16, reading a slot's puts by identity ------------------------------------------

RAISED, ESCALATED, REFRESHED, CONFIRMED, RETIRED, UNREADABLE_PUT = (
    "raised", "escalated", "refreshed", "confirmed", "retired", "unreadable")
REBOUND, REPLACED, DROPPED = "rebound", "replaced", "dropped"
DELETE = "delete"
#: §2.4's fields, "Equal is equal in every field §2.4 names".
FIELDS = ("state", "severity", "rule", "at", "labels", "argument", "key")


def identity_of(table: Table, value: dict[str, Any]) -> tuple[str, Any]:
    """A record's identity for §2.6's "One identity": its alert key where
    the reader has one (a record by words its ``key``, a named table's rule
    the key the reader derives), else ("unnamed", (rule, labels or None))."""
    n = name_record(table, value)
    if n.alert_key is not None:
        return ("key", n.alert_key)
    return ("unnamed", (value.get("rule"), value.get("labels")))


def same_identity(table: Table, a: dict[str, Any], b: dict[str, Any]) -> bool:
    """§2.6: "each has an alert key, and the two are equal …; or either is
    unnamed, and their ``rule`` and their ``labels`` are equal, an absent
    ``labels`` equal only to an absent one"."""
    ia, ib = identity_of(table, a), identity_of(table, b)
    if ia[0] == "key" and ib[0] == "key":
        return ia[1] == ib[1]
    return (a.get("rule"), "labels" in a, a.get("labels")) == (b.get("rule"), "labels" in b, b.get("labels"))


def _equal(a: dict[str, Any], b: dict[str, Any]) -> bool:
    return all((f in a, a.get(f)) == (f in b, b.get(f)) for f in FIELDS)


def _against(held: dict[str, Any], put: dict[str, Any]) -> str:
    """The same identity's rows of §2.6's table."""
    hf, pf = held["state"] == FIRING, put["state"] == FIRING
    if hf and pf:
        if put["severity"] != held["severity"]:
            return ESCALATED
        if ("at" in put, put.get("at")) != ("at" in held, held.get("at")):
            return RAISED
        return CONFIRMED if _equal(held, put) else REFRESHED
    if hf and not pf:
        return RESOLVED
    if pf:
        return RAISED
    if ("at" in put, put.get("at")) != ("at" in held, held.get("at")):
        return RESOLVED
    return CONFIRMED if _equal(held, put) else REFRESHED


@dataclass(frozen=True)
class Step:
    """One put's reading on a slot (§2.6): the held record's fate where it
    was another identity's (``rebound``, ``replaced``, ``dropped``), and the
    put's own transition."""

    held: str | None
    put: str
    #: the identity whose record the held fate is about, and the put's
    held_identity: Any = None
    identity: Any = None

    def expect(self) -> list[str]:
        return ([self.held] if self.held else []) + [self.put]


_DERIVE = object()


class SlotReader:
    """One slot's samples "in the owner's order", each read "against the
    record it holds on that slot, as this table and the fixture do. … It
    holds each record with the run of the put that carried it: its
    envelope's, or its snapshot entry's (core §7.5)." A run is compared in
    width and value (core §3.3). Each record's alert key is the one the
    reader has for it: derived through the owner's table by default, or
    handed in (None for an unnamed alarm)."""

    def __init__(self, table: Table = NO_TABLE) -> None:
        self.table = table
        self.held: dict[str, Any] | None = None
        self.key: str | None = None
        self.run: Any = None

    def _key(self, value: dict[str, Any], given: Any) -> str | None:
        return name_record(self.table, value).alert_key if given is _DERIVE else given

    def step(self, sample: Any, run: Any = None, alert_key: Any = _DERIVE) -> Step:
        """``sample`` is :data:`DELETE` (a delete or a tombstone), a
        :class:`Read`, or a record's JSON reading."""
        held, held_key, held_run = self.held, self.key, self.run
        if sample == DELETE:
            self.held, self.key, self.run = None, None, None
            return Step(None, RESOLVED if held is not None and held["state"] == FIRING else RETIRED, None, held_key)
        r = sample if isinstance(sample, Read) else Read(sample)
        if not r.reads or r.value is None:
            self.held, self.key, self.run = None, None, None
            return Step(None, UNREADABLE_PUT)
        put = r.value
        key = self._key(put, alert_key)
        self.held, self.key, self.run = put, key, run
        if held is None:
            return Step(None, RAISED if put["state"] == FIRING else RESOLVED, None, key)
        if held_key is not None and key is not None:
            same = held_key == key
        else:
            same = (put.get("rule"), "labels" in put, put.get("labels")) == \
                (held.get("rule"), "labels" in held, held.get("labels"))
        if same:
            return Step(None, _against(held, put), None, key)
        fate = DROPPED if held_run != run else (REBOUND if held["state"] == FIRING else REPLACED)
        return Step(fate, RAISED if put["state"] == FIRING else RESOLVED, held_key, key)


def read_slot(samples: list[Any], table: Table = NO_TABLE) -> list[list[str]]:
    rd = SlotReader(table)
    return [rd.step(s, run).expect() for s, run in samples]


@dataclass(frozen=True)
class Notice:
    """One report of a notifier (§2.16): the alarm's reference (system,
    service, alert key), or for an unnamed alarm its owner, its slot and its
    ``at``; its answer after the change; the severity while it fires; the
    change that moved it."""

    reference: tuple[Any, ...]
    answer: str
    severity: str | None
    change: str
    unnamed: bool = False


class Notifier:
    """§2.16 over one owner's slots: "``rebound`` is a resolve to report",
    "A record ``replaced`` or ``dropped`` is no change of its alarm to
    report by itself", "A notice's identity is the reference, never the key
    and never the slot … So it tells nothing twice when a service moves
    from ``alarms.v1``, or when an alarm comes back on another slot." v1
    §2.16 holds: a change is the answer moving between yes and no, or the
    severity moving while yes; a refresh and a confirmation are none; a
    first answer is reported where it is yes."""

    def __init__(self, system: str, service: str, table: Table = NO_TABLE) -> None:
        self.system, self.service = system, service
        self.table = table
        self.readers: dict[int, SlotReader] = {}
        self.told: dict[tuple[Any, ...], tuple[str, str | None]] = {}

    def _ref(self, slot: int, value: dict[str, Any], key: str | None) -> tuple[tuple[Any, ...], bool]:
        if key is not None:
            return (self.system, self.service, key), False
        return (self.system, self.service, slot, value.get("at")), True

    def _tell(self, ref: tuple[Any, ...], unnamed: bool, answer: str, severity: str | None,
              change: str) -> list[Notice]:
        now = (answer, severity if answer == "yes" else None)
        last = self.told.get(ref)
        if last is None and answer != "yes":
            return []
        if last == now:
            return []
        self.told[ref] = now
        return [Notice(ref, answer, now[1], change, unnamed)]

    def put(self, slot: int, sample: Any, run: Any = None, alert_key: Any = _DERIVE) -> list[Notice]:
        rd = self.readers.setdefault(slot, SlotReader(self.table))
        before, before_key = rd.held, rd.key
        st = rd.step(sample, run, alert_key)
        out: list[Notice] = []
        if st.held == REBOUND and before is not None:
            ref, un = self._ref(slot, before, before_key)
            out += self._tell(ref, un, "no", None, REBOUND)
        if rd.held is None:
            if sample == DELETE and before is not None:
                ref, un = self._ref(slot, before, before_key)
                out += self._tell(ref, un, "no", None, st.put)
            return out
        ref, un = self._ref(slot, rd.held, rd.key)
        firing = rd.held["state"] == FIRING
        out += self._tell(ref, un, "yes" if firing else "no", rd.held["severity"] if firing else None, st.put)
        return out


# -- §2.7 and §3, confirmation by the heartbeat, and a snapshot's page -------------------------

ALARMS_RESOURCE = "state/alarms/{slot}"


@lru_cache(maxsize=1)
def standard():
    """The standard contract, loaded."""
    from .contract import load_contract

    return load_contract(CONTRACT)


def heartbeat_layout(canonical: dict[str, Any] | None = None, fingerprint: str | None = None) -> Any:
    """The heartbeat's layout for revision 2.0 (``freshness.v2`` §2.4, core
    §7.5): one section, ``alarms/{slot}``, the words having no horizon; tag
    7. A copy with another ``freshness.ttl_s`` is handed in whole."""
    from . import heartbeat

    k = standard()
    return heartbeat.layout_of(canonical or k.canonical, fingerprint or k.fingerprint)


@dataclass
class SlotHeld:
    """What a reader's view holds of one slot: the record's JSON reading,
    its put's in-band stamp, when it was taken (ns), and its last heartbeat
    confirmation since (ns), or None."""

    record: dict[str, Any]
    width: int
    run: int
    seq: int
    at_ns: int
    confirmed_ns: int | None = None


class OwnerAlarms:
    """One reader's view of one owner's alarms, read against its heartbeats
    (§2.7): "Its heartbeat carries a digest, a live count of 9 bits, and one
    bit or one rank for each live member (core §7.5, ``freshness.v2``
    §2.4)." "Neither a count nor a bit is a state": the record says it.
    "Ranks follow key order, in bytes: slot 10 ranks before slot 2 (core
    §7.5). A rank is a place among the live members, never a slot."

    The heartbeat is read by :class:`zk2py.freshness_v2.View`, over the
    slots' keys under ``system``/``service``."""

    def __init__(self, system: str, service: str, layout: Any = None, ttl_s: int = TTL_S) -> None:
        from . import freshness_v2 as f2

        self.system, self.service = system, service
        self.layout = layout or heartbeat_layout()
        self.ttl_s = ttl_s
        self.view = f2.View(self.layout, [ALARMS_RESOURCE], f"{system}/{service}")
        self.slots: dict[int, SlotHeld] = {}

    def key(self, slot: int) -> str:
        k = slot_key(self.system, self.service, slot)
        assert k is not None
        return k

    def put(self, slot: int, record: dict[str, Any], width: int, run: int, seq: int, at_ns: int) -> bool:
        """A put heard on ``alarms/<slot>``; False where the view did not
        take it (an older put of one run)."""
        from . import zk2c

        if not self.view.mutate(self.key(slot), zk2c.InBand(width, run, seq), at_ns, True):
            return False
        self.slots[slot] = SlotHeld(dict(record), width, run, seq, at_ns)
        return True

    def heartbeat(self, tag: int, width: int, run: int, seq: int, body: bytes, at_ns: int) -> dict[str, Any]:
        """One heartbeat read over the view, in ``freshness.v2`` §2.5's steps:
        what it says (its live count, the digest in step or not), the view's
        count of firing records beside it, and the slots it confirmed, by
        rank."""
        from . import heartbeat, zk2c

        got = self.view.heartbeat(tag, zk2c.InBand(width, run, seq), body, at_ns)
        out: dict[str, Any] = {"firing": self.firing_count(), "held": len(self.slots)}
        try:
            value = heartbeat.read(self.layout, body)
            sec = value["sections"].get(ALARMS_RESOURCE)
            out["live"] = None if sec is None else sec["live"]
        except heartbeat.Refused as e:
            out["live"] = None
            out["refused"] = e.token
        if "read" not in got:
            out["read"] = got
            out["confirmed"] = []
            return out
        sec_r = got["read"]["sections"][ALARMS_RESOURCE]
        if isinstance(sec_r, dict):
            confirmed = [self._slot(k) for k in sec_r["confirmed"]]
            for s in confirmed:
                self.slots[s].confirmed_ns = at_ns
            out["in_step"] = True
            out["confirmed"] = confirmed
        else:
            out["in_step"] = False
            out["section"] = sec_r
            out["confirmed"] = []
        return out

    def _slot(self, key: str) -> int:
        s = slot_of(key)
        assert s is not None
        return s["slot"]

    def firing_count(self) -> int:
        return sum(1 for h in self.slots.values() if h.record.get("state") == FIRING)

    def ranks(self) -> list[int]:
        """The live slots in rank order: their keys' byte order."""
        return [self._slot(k) for k in self.view.live_keys(self.layout.collection(ALARMS_RESOURCE))]

    def answer(self, slot: int, now_ns: int) -> dict[str, str]:
        """"Is this alarm firing?" of one slot, by its record and its age
        alone (§2.7, §2.14 steps 5 and 6): a resolved record **no**,
        ``resolved``, "whatever its age"; a firing one **yes**, ``firing``,
        within 900 s of its last confirmation (its put, or the last
        heartbeat that vouched for it), else **unobservable**, ``stale``:
        "neither firing nor resolved"."""
        h = self.slots.get(slot)
        if h is None:
            return {"answer": "unobservable", "reason": "not_read"}
        if h.record.get("state") == RESOLVED:
            return {"answer": "no", "reason": RESOLVED}
        last = max(h.at_ns, h.confirmed_ns or h.at_ns)
        if now_ns - last <= self.ttl_s * 1_000_000_000:
            return {"answer": "yes", "reason": FIRING}
        return {"answer": "unobservable", "reason": "stale"}


def read_page(data: bytes, epoch: int | None = None) -> dict[str, Any]:
    """A snapshot's page of the alarms or the words (§3: "In a snapshot the
    alarms are collection 0 and the words collection 1 … A member's id is
    its slot"), read entry by entry: each live entry's slot, its stamp, its
    confirmation and its record read as §2.4 says (or its words as §2.4
    reads them), and each tombstone."""
    from . import snapshot

    page = snapshot.read_page(data)
    out: dict[str, Any] = {"run_width": page["run_width"], "run": page["run"], "asof": page["asof"],
                           "index": page["index"], "more": page["more"], "coll": page["coll"],
                           "time": page["time"], "live": [], "tombs": []}
    for e in page["live"]:
        slot = parse_slot_chunk(e["id"])
        rec = bytes.fromhex(e["record"])
        entry: dict[str, Any] = {"id": e["id"], "slot": slot, "seq": e["seq"], "confirmation": e["confirmation"]}
        if e.get("run") is not None:
            entry["run_width"], entry["run"] = e["run_width"], e["run"]
        if "conf" in e:
            entry["conf"] = e["conf"]
        if page["coll"] == COLLECTION[ALARMS]:
            r = read(page["tag"], rec, epoch)
            entry["record"] = r.value if r.reads else {"undecodable": r.undecodable}
        else:
            entry["words"] = read_words(rec)
        out["live"].append(entry)
    for e in page["tombs"]:
        t = {"id": e["id"], "slot": parse_slot_chunk(e["id"]), "seq": e["seq"]}
        if e.get("run") is not None:
            t["run_width"], t["run"] = e["run_width"], e["run"]
        out["tombs"].append(t)
    return out


# -- §2.5, the acknowledgement identity: ``at`` --------------------------------------------------

#: §2.5's table: which acts take a new ``at``, and which keep it.
RAISE, ESCALATE, REFRESH, REFRESH_WORDS, CONFIRM, RESOLVE = (
    "raise", "escalate", "refresh", "refresh_words", "confirm", "resolve")
NEW_AT = {RAISE, ESCALATE, RESOLVE}
KEPT_AT = {REFRESH, REFRESH_WORDS, CONFIRM}


def coded(at_ms: int | None, epoch: int | None = None) -> bool:
    """Whether an instant has a code under the epoch: whole seconds from it,
    0 to 2^32 − 1 (core §7.6)."""
    if at_ms is None:
        return False
    e = packed.deployment_epoch(epoch)
    return 1000 * e <= at_ms < 1000 * (e + (1 << 32))


class AtClock:
    """An owner's ``at`` for each identity (§2.5): "**New** is the owner's
    clock, in whole seconds. Where that is not later than the greatest
    ``at`` the owner knows it wrote for that identity, new is that ``at``
    plus one second." "While its clock reads no instant the epoch codes, an
    owner writes no ``at``. So does one whose new ``at`` would have no
    code." Instants are Unix milliseconds."""

    def __init__(self, epoch: int | None = None) -> None:
        self.epoch = epoch
        self.greatest: dict[Any, int] = {}

    def new(self, identity: Any, clock_ms: int | None) -> int | None:
        if clock_ms is None or not coded(clock_ms, self.epoch):
            return None
        at = clock_ms - clock_ms % 1000
        g = self.greatest.get(identity)
        if g is not None and at <= g:
            at = g + 1000
        if not coded(at, self.epoch):
            return None
        self.greatest[identity] = at
        return at

    def act(self, identity: Any, act: str, clock_ms: int | None, held: int | None) -> int | None:
        """The ``at`` an act writes: a new one at a transition, "the one it
        holds at a refresh" (and a confirmation puts nothing)."""
        if act in NEW_AT:
            return self.new(identity, clock_ms)
        if act in KEPT_AT:
            return held
        raise ValueError(f"no act {act!r}")

    def knows(self, identity: Any, at_ms: int | None) -> None:
        """An ``at`` the owner knows it wrote for an identity, as an adopted
        record's."""
        if at_ms is not None and at_ms > self.greatest.get(identity, -1):
            self.greatest[identity] = at_ms


def ack_applies(acked_at: int | None, at: int | None) -> bool:
    """"An acknowledgement applies while the alarm's ``at`` is at most the
    acknowledged one"; "With no ``at``, no acknowledgement applies." An
    acknowledgement made against no ``at`` applies to none."""
    if at is None or acked_at is None:
        return False
    return at <= acked_at


def from_v1(timestamp_ms: int, epoch: int | None = None) -> int | None:
    """§2.9 "From ``alarms.v1``": "Its ``at`` is the document's ``timestamp``
    in whole seconds (§2.5)"; none where that instant has no code under the
    epoch."""
    at = timestamp_ms - timestamp_ms % 1000
    return at if coded(at, epoch) else None


# -- §2.1, §2.11, §2.13 to §2.15 and §5, what a reader concludes ------------------------------------

V1 = "alarms.v1"


def major_of(interfaces: Any) -> str | None:
    """§2.1 "A reader MUST read a service's alarms by the major its
    descriptor lists, and by this text where it lists both"; §2.14 step 2:
    "One that holds ``alarms.v1`` and not ``alarms.v2`` is read by
    ``alarms.v1``'s. One that holds neither: not asked, ``not_listed``."
    None for neither."""
    names = set(interfaces or ())
    if IFACE in names:
        return IFACE
    if V1 in names:
        return V1
    return None


def judge(r: Any, question: str | None = None, horizon: Any = None) -> Any:
    """§2.14 "v1 §2.14's six steps hold, read over this major's records":
    :func:`zk2py.alarms.judge` over a :class:`zk2py.alarms.Reading` whose
    ``descriptor`` ``lists`` says the descriptor lists ``alarms.v2`` (or,
    for an owner read by v1's text, ``alarms.v1``), with step 5's one answer
    more: "The alarm's record was ``rebound`` (§2.6): **no**, ``rebound``."
    ``latest`` is then ``rebound``; "A record ``replaced`` or ``dropped``
    leaves the reader holding nothing of its alarm", so ``latest`` None.
    "Is this firing alarm past its horizon" answers ``rebound`` "beside a
    resolved record: not asked, ``not_firing``". Step 6 is freshness.v2's
    at 900 s, its observations as :mod:`zk2py.freshness` reads them."""
    from . import alarms as v1

    q = v1.Q_FIRING if question is None else question
    h = v1.HORIZON if horizon is None else horizon
    if r.latest == REBOUND:
        probe = v1.Reading(**{**r.__dict__, "latest": RESOLVED})
        a = v1.judge(probe, q, h)
        if (a.answer, a.reason) == (v1.NO, RESOLVED):
            return v1.Answer(v1.NO, REBOUND, "its slot was retaken by another identity under the same run (§2.6)")
        return a
    return v1.judge(r, q, h)


@dataclass(frozen=True)
class Read2:
    """The reader's latest collection read of ``alarms.v2/state/alarms/**``
    (§2.15 "Established"): whether one was made; whether it ran to its final
    reply with no error reply; whether it drew a complete snapshot of the
    alarms (every page); and the snapshot's run, as its digits, or None."""

    made: bool = True
    complete: bool = True
    snapshot: bool = True
    run: Any = None


def established(presence: str, descriptor: dict[str, Any] | None, face: dict[str, Any] | None,
                read: Read2 | None, lost: bool = False, gap: bool = False, runs: Any = None) -> Any:
    """§2.15 "The order", the first that answers:
    1. v1 §2.14's steps 1 and 2, the descriptor read as §2.14 reads it;
    2. the read: none made, ``no_seed``; one that ended at its timeout or
       with an error reply, ``seed_incomplete``;
    3. since the read, when the reader subscribes: a delivery lost,
       ``window_lost``; a gap in the owner's presence, ``owner_gap`` (not
       asked across a face);
    4. the snapshot: no page of it, or a page missing,
       ``snapshot_incomplete``; its run one that no present descriptor
       states, where one states a run, ``snapshot_run``;
    5. otherwise ``snapshot``: established.

    ``descriptor`` is ``{"lists": bool}``, whether it lists ``alarms.v2``
    (v1's shape); ``runs`` the runs the owner's present descriptors state,
    as digits (None or empty where none states one). "Across a face the
    deployment's word stands for the descriptor, and no run is compared."
    """
    from . import alarms as v1

    if presence == "absent":
        return v1.Verdict(v1.NOT_ASKED, "absent")
    if presence == "incomplete":
        return v1.Verdict(v1.UNOBSERVABLE, "presence_incomplete")
    across = presence == "across_face"
    if across:
        if not (face or {}).get("reachable"):
            return v1.Verdict(v1.UNOBSERVABLE, "face_closed")
    elif presence == "present":
        if descriptor is None:
            return v1.Verdict(v1.UNOBSERVABLE, "no_descriptor")
        if not descriptor.get("lists"):
            return v1.Verdict(v1.NOT_ASKED, "not_listed")
    else:
        raise ValueError(f"presence {presence!r}")
    if read is None or not read.made:
        return v1.Verdict(v1.UNOBSERVABLE, "no_seed")
    if not read.complete:
        return v1.Verdict(v1.UNOBSERVABLE, "seed_incomplete")
    if lost:
        return v1.Verdict(v1.UNOBSERVABLE, "window_lost")
    if gap and not across:
        return v1.Verdict(v1.UNOBSERVABLE, "owner_gap")
    if not read.snapshot:
        return v1.Verdict(v1.UNOBSERVABLE, "snapshot_incomplete")
    if not across and runs and read.run not in set(runs):
        return v1.Verdict(v1.UNOBSERVABLE, "snapshot_run")
    return v1.Verdict(v1.ESTABLISHED, "snapshot")


def any_firing(at_least: str, owners: Any, scope_whole: bool = True) -> dict[str, Any]:
    """§2.15 "Anything firing at or above a severity is v1's, over the
    owners so established. An unnamed alarm counts. A firing alarm whose
    severity is the escape is v1's 'yes without a severity'": owners of
    either major, one set (§2.17)."""
    from . import alarms as v1

    return v1.any_firing(at_least, owners, scope_whole)

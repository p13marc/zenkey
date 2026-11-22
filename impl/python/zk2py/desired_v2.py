"""``desired.v2`` (profile text 0.1, core 0.112, #986): ``desired.v1`` by
reference, "on a wire a far target can afford". Session-free. Each part was
written from ``spec/profiles/desired/v2.md`` and committed before its file
of ``conformance/v2/`` was opened (round 118).

"`desired.v2` keeps every rule of `desired.v1` and changes three things":
the version "is the run and the seq its put already carries"; a target's
"re-read names the version it holds", or it "reads only when that
heartbeat's digest moves"; and "The report is a packed record". The rules
v1 keeps are :mod:`zk2py.desired`'s, read here "as written, except where §2
replaces it".

**§3, the standard schema** (:data:`REPORT`): the packed file
``desired.v2.json``, whose ``Report`` "is the profile's in any artifact
whose id is one of these, and in no other". "Its tag is 12 (core §7.6). So a
report's put has CH ``0x0c`` under a 16-bit run of its target, and ``0x1c``
under a 32-bit run."

**§2.10, the report.** :func:`write_report` and :func:`put_payload` write
one; :func:`read_report` reads one: "Its bytes are a packed record's, read
as core §7.6 reads one, its tag CH's. A record that reading refuses is
**unreadable**, for the refusal core §7.6 names: ``tag``, ``length``,
``code``, ``extension`` or ``pad``", "Then the checks above. A report with a
finding is **malformed**" (:func:`consistency`, the eight in order,
``escape`` "alone").

**§2.10, the two tables.** :func:`place_index` and :func:`line_index` write
a refusal's place and line "as this says"; :func:`show_place` and
:func:`show_line` read an index back with the table a reader holds.
"""

from __future__ import annotations

from dataclasses import dataclass
from functools import lru_cache
from pathlib import Path
from typing import Any

from . import jcs, packed
from . import desired as v1

PROFILE = "desired.v2"
#: §4: the three keys this major adds to v1's ``desired.target_param``.
REPORT_KEY, POINTERS_KEY, DETAILS_KEY = "desired.report", "desired.pointers", "desired.details"
REPO = Path(__file__).resolve().parents[3]
SCHEMA_PATH = REPO / "spec" / "profiles" / "desired" / "desired.v2.json"
#: §3's table: "Artifact id | Revision | What it holds".
PUBLISHED = (("sha256:1ac893a1cb14a102cc1b01ea526a317b19390a8ed6c40707f08754f4429532dd", "The first"),)
PUBLISHED_IDS = tuple(i for i, _ in PUBLISHED)
CURRENT = PUBLISHED[-1][0]
REPORT_TYPE = "Report"
#: §3 "Its tag is 12 (core §7.6)."
REPORT_TAG = 12
#: §2.4 "It is at most 4,294,967,295. A report carries it in 32 bits".
MAX_REVISION = 4_294_967_295
MAX_SEQ = 16_777_215

BASELINE, DESIRED, CALL = "baseline", "desired", "call"
NOT_ACTED, CONVERGED, REFUSED, FAILED, REVERTED = "not_acted", "converged", "refused", "failed", "reverted"
IN_NONE, IN_OBSERVED, IN_OTHER = "none", "observed", "other"
#: §2.10 "`code`, with `refused` or `failed`: the profile's five codes, as in
#: v1."
CODES = ("undecodable", "reach", "sensitive", "invalid", "apply")
OUT_OF_RANGE = "out_of_range"
#: §2.10: "A code of the type's own is written as the escape, and a tool
#: says "the type's own"."
TYPES_OWN = "the type's own"

OBSERVED_FIELDS = ("observed_run16", "observed_run32", "observed_seq", "revision")
IN_FORCE_FIELDS = ("in_force_run16", "in_force_run32", "in_force_seq", "in_force_revision")


def artifact_id(document: Any) -> str:
    """Core §7.1's artifact id of a schema document: sha256 over its JCS
    bytes, as v1's is computed."""
    return v1.artifact_id(document)


def is_published(artifact: str | None) -> bool:
    """§3: the type "is the profile's in any artifact whose id is one of
    these, and in no other, whatever its types are named"."""
    return artifact in PUBLISHED_IDS


@lru_cache(maxsize=1)
def load_schema() -> dict[str, Any]:
    return jcs.loads(SCHEMA_PATH.read_bytes())


@lru_cache(maxsize=1)
def report_record() -> packed.Record:
    """The profile's ``Report`` as core §7.6's codec holds it."""
    return packed.record(load_schema(), REPORT_TYPE)


# -- §2.4: a version ------------------------------------------------------------------------------

@dataclass(frozen=True, order=True)
class Run:
    """Core §4.3's run: "of 16 or 32 bits ... Two runs of different widths
    are two runs, whatever their values.\""""

    width: int
    value: int

    @property
    def text(self) -> str:
        """§2.4: "written as a descriptor writes one (core §3.3): 4 lowercase
        hexadecimal digits for a 16-bit run, 8 for a 32-bit run"."""
        return format(self.value, "04x" if self.width == 16 else "08x")

    @classmethod
    def parse(cls, text: Any) -> "Run | None":
        if not isinstance(text, str) or len(text) not in (4, 8):
            return None
        if any(ch not in "0123456789abcdef" for ch in text):
            return None
        return cls(16 if len(text) == 4 else 32, int(text, 16))


@dataclass(frozen=True)
class Version:
    """§2.4: "one mutation of a document's key, named by its in-band stamp:
    its run, with its width, and its seq". "Two versions are the same when
    their runs are the same, in width and value, and their seqs are equal.\""""

    run: Run
    seq: int

    @property
    def text(self) -> str:
        """§2.4: "its run, then ``/``, then its seq ... The seq is in
        decimal, without leading zeros. ``0007/12`` is seq 12 of the 16-bit
        run 7, and ``00000007/12`` a version of another run.\""""
        return f"{self.run.text}/{self.seq}"

    @classmethod
    def parse(cls, text: Any) -> "Version | None":
        if not isinstance(text, str) or text.count("/") != 1:
            return None
        r, s = text.split("/")
        run = Run.parse(r)
        if run is None or not s.isascii() or not s.isdigit() or (len(s) > 1 and s[0] == "0"):
            return None
        seq = int(s)
        return cls(run, seq) if seq <= MAX_SEQ else None

    @classmethod
    def of(cls, width: int, run: int, seq: int) -> "Version":
        return cls(Run(width, run), seq)


def stated(report: dict[str, Any], fields: tuple[str, ...] = OBSERVED_FIELDS) -> Version | None:
    """§2.10 ``run_fields``: "A version is **stated** by its seq and exactly
    one of its two runs." The version the four fields state, or None."""
    r16, r32, seq = report.get(fields[0]), report.get(fields[1]), report.get(fields[2])
    if seq is None or (r16 is None) == (r32 is None):
        return None
    return Version(Run(16, r16) if r16 is not None else Run(32, r32), seq)


def version_fields(v: Version, revision: int | None, fields: tuple[str, ...] = OBSERVED_FIELDS) -> dict[str, Any]:
    """§2.10 "The observed version ... is ``observed_seq`` and one of
    ``observed_run16`` and ``observed_run32``, by its run's width.
    ``revision`` is its document's own, where its type carries one.\""""
    out: dict[str, Any] = {fields[0] if v.run.width == 16 else fields[1]: v.run.value, fields[2]: v.seq}
    if revision is not None:
        out[fields[3]] = revision
    return out


# -- §2.10: the two tables ------------------------------------------------------------------------

#: "A resource without the key has the table ``[""]``", and "an empty table".
DEFAULT_POINTERS: tuple[str, ...] = ("",)
DEFAULT_DETAILS: tuple[str, ...] = ()
MAX_TABLE = 255


def is_json_pointer(p: Any) -> bool:
    """§2.10: "Each is a JSON Pointer (RFC 6901): empty, or starting with
    ``/``, with every ``~`` followed by ``0`` or ``1``.\""""
    if not isinstance(p, str):
        return False
    if p and not p.startswith("/"):
        return False
    i = p.find("~")
    while i != -1:
        if p[i + 1:i + 2] not in ("0", "1"):
            return False
        i = p.find("~", i + 1)
    return True


def pointers_ok(table: Any) -> bool:
    """§2.10: "``desired.pointers`` is an array of 1 to 255 strings. Its
    first is ``""``, the document itself. Each is a JSON Pointer ... None is
    there twice.\""""
    return (isinstance(table, list) and 1 <= len(table) <= MAX_TABLE and all(is_json_pointer(p) for p in table)
            and table[0] == "" and len(set(table)) == len(table))


def details_ok(table: Any) -> bool:
    """§2.10: "``desired.details`` is an array of at most 255 strings, each
    of at least one character.\""""
    return (isinstance(table, list) and len(table) <= MAX_TABLE
            and all(isinstance(x, str) and len(x) >= 1 for x in table))


def place_index(pointers: Any, place: str | None) -> int | str | None:
    """§2.10, a refusal's place as the report writes it.

    - "A refusal that points nowhere carries no ``pointer``": None.
    - "A place the table holds is written as its index."
    - "A place it does not hold is written as its nearest listed ancestor:
      the longest listed pointer, other than ``""``, that the place
      continues with a ``/``. With none, it is written ``out_of_range``. So
      index 0 says the document itself is refused, never "somewhere"."
    """
    if place is None:
        return None
    table = list(DEFAULT_POINTERS if pointers is None else pointers)
    if place in table:
        return table.index(place)
    best: int | None = None
    for i, p in enumerate(table):
        if p != "" and place.startswith(p + "/") and (best is None or len(p) > len(table[best])):
            best = i
    return OUT_OF_RANGE if best is None else best


def line_index(details: Any, line: str | None) -> int | str | None:
    """§2.10: "A line the table does not hold is written ``out_of_range``.
    The words stay in the target's log. A refusal with no line carries no
    ``detail``.\""""
    if line is None:
        return None
    table = list(DEFAULT_DETAILS if details is None else details)
    return table.index(line) if line in table else OUT_OF_RANGE


def show_index(table: Any, index: Any) -> Any:
    """§2.10 "A reader reads an index with the revision of the target's
    contract that its descriptor names ... An index past the table it holds
    is a later revision's: it shows the number. A tool with neither shows
    numbers." ``table`` None is a tool with neither. ``out_of_range`` reads
    as itself: the word "no table names" (§3)."""
    if index is None or index == OUT_OF_RANGE:
        return index
    if table is None or not 0 <= index < len(table):
        return index
    return table[index]


def show_place(pointers: Any, index: Any, held: bool = True) -> Any:
    """A ``pointer`` read back: with the table of the target's revision,
    ``[""]`` where that revision's resource has no key; ``held`` False for a
    tool that holds no revision."""
    return show_index((list(DEFAULT_POINTERS) if pointers is None else pointers) if held else None, index)


def show_line(details: Any, index: Any, held: bool = True) -> Any:
    return show_index((list(DEFAULT_DETAILS) if details is None else details) if held else None, index)


def code_field(code: str | None) -> str | None:
    """§2.10: "A code of the type's own is written as the escape": the
    packed reading's ``null`` (core §7.6)."""
    return code if code in CODES else None


def show_code(code: Any) -> str:
    """§2.10: "a tool says "the type's own"" of the escape."""
    return TYPES_OWN if code is None else code


# -- §2.10: writing a report ----------------------------------------------------------------------

def baseline_report() -> dict[str, Any]:
    """§2.12: "the baseline's record: ``source`` ``baseline``,
    ``not_acted``. ... it is the report any role starts with." 20 bits."""
    return {"source": BASELINE, "outcome": NOT_ACTED, "last_known": False, "in_force": IN_NONE}


def write_bits(report: dict[str, Any]) -> str:
    """The record's bits before its pad (§3's layout)."""
    return report_record().write_bits(report)


def write_report(report: dict[str, Any]) -> bytes:
    """A report's record, by core §7.6's writer. Raises
    :class:`zk2py.packed.Refused` for a value the layout has no code for."""
    return report_record().write(report)


def put_payload(report: dict[str, Any], run: int, seq: int, width: int = 16) -> bytes:
    """The report's put: core §7.5's mutation, CH's tag the type's. "So a
    report's put has CH ``0x0c`` under a 16-bit run of its target, and
    ``0x1c`` under a 32-bit run." The run and seq are the *target's*: the
    report is its own state (core S1–S3)."""
    return packed.seal(report_record(), report, run, seq, width)


# -- §2.10: reading a report ----------------------------------------------------------------------

#: "A record that reading refuses is unreadable, for the refusal core §7.6
#: names".
UNREADABLE = ("tag", "length", "code", "extension", "pad")
#: "A reader checks these, in this order, and a report that breaks one is
#: malformed".
FINDINGS = ("escape", "run_fields", "observed_outcome", "refusal", "source", "in_force", "revision_on_revert",
            "last_known")


def consistency(r: dict[str, Any]) -> list[str]:
    """§2.10 "A target's report MUST be consistent": the findings a report
    draws, in the text's order.

    - ``escape``: "``source`` or ``outcome`` is the escape, a value a later
      text names. A reader reports it alone, and checks nothing more."
    - ``run_fields``: "The finding is any of a version's four fields present
      with no version stated. It is checked of the observed fields, and of
      the ``in_force`` ones."
    """
    source, outcome, in_force = r.get("source"), r.get("outcome"), r.get("in_force")
    if source is None or outcome is None:
        return ["escape"]
    out: list[str] = []
    observed, other = stated(r), stated(r, IN_FORCE_FIELDS)
    if any((observed if fields is OBSERVED_FIELDS else other) is None and any(r.get(f) is not None for f in fields)
           for fields in (OBSERVED_FIELDS, IN_FORCE_FIELDS)):
        out.append("run_fields")
    # "an observed version stated with not_acted; or converged, refused or
    # failed with none stated"
    if (observed is not None and outcome == NOT_ACTED) or (
            observed is None and outcome in (CONVERGED, REFUSED, FAILED)):
        out.append("observed_outcome")
    # "code, pointer or detail with an outcome other than refused or failed;
    # or one of those two without code"
    refusing = outcome in (REFUSED, FAILED)
    has_code = "code" in r  # the escape is a code too: present, read as null
    if (not refusing and (has_code or "pointer" in r or "detail" in r)) or (refusing and not has_code):
        out.append("refusal")
    # "desired with no observed version stated; converged with baseline;
    # reverted with desired"
    if ((source == DESIRED and observed is None) or (outcome == CONVERGED and source == BASELINE)
            or (outcome == REVERTED and source == DESIRED)):
        out.append("source")
    # "in_force: any of these"
    in_force_field = any(r.get(f) is not None for f in IN_FORCE_FIELDS)
    if (
        # "observed or other with a source other than desired, or with failed"
        (in_force in (IN_OBSERVED, IN_OTHER) and (source != DESIRED or outcome == FAILED))
        # "converged with desired, and an in_force that is not observed"
        or (outcome == CONVERGED and source == DESIRED and in_force != IN_OBSERVED)
        # "observed with no observed version stated"
        or (in_force == IN_OBSERVED and observed is None)
        # "other with no in_force version stated, or with one that is the
        # observed version"
        or (in_force == IN_OTHER and (other is None or other == observed))
        # "an in_force field present with none or observed"
        or (in_force in (IN_NONE, IN_OBSERVED) and in_force_field)
    ):
        out.append("in_force")
    if outcome == REVERTED and r.get("revision") is not None:
        out.append("revision_on_revert")
    if r.get("last_known") is True and observed is None:
        out.append("last_known")
    return out


@dataclass(frozen=True)
class ReportReading:
    """§2.10 "How a reader reads one": ``refusal`` names why a record is
    unreadable; ``findings`` why a readable one is malformed. "A tool answers
    nothing from an unreadable or a malformed report (§2.17)."""

    report: dict[str, Any] | None
    refusal: str | None = None
    findings: tuple[str, ...] = ()

    @property
    def readable(self) -> bool:
        return self.refusal is None

    @property
    def valid(self) -> bool:
        return self.refusal is None and not self.findings

    def expect(self) -> dict[str, Any]:
        if self.refusal is not None:
            return {"readable": False, "refused": self.refusal}
        return {"readable": True, "report": self.report, "findings": list(self.findings)}


def read_report(tag: int, record: bytes) -> ReportReading:
    """A report's record under CH's ``tag``. "A later text's fields, behind
    the marker, are ignored (core §7.6)."""
    try:
        value = report_record().read(tag, record)
    except packed.Refused as e:
        return ReportReading(None, e.token)
    return ReportReading(value, None, tuple(consistency(value)))


def read_put(payload: bytes) -> ReportReading:
    """A report's put: core §7.5's parse, then the record under its CH's
    tag. An envelope the parse refuses is unreadable by its own token."""
    from . import zk2c

    try:
        env = zk2c.parse(payload, "put")
    except zk2c.Refused as e:
        return ReportReading(None, e.token)
    if env.form != "mutation":
        return ReportReading(None, "form")
    return read_report(env.tag, env.record or b"")


def reads_back(record: bytes, tag: int = REPORT_TAG) -> bool:
    """§2.10: "Every report reads back to the same bytes. The checks leave
    one record for one report." True for a record that reads with no finding
    and writes back byte for byte."""
    got = read_report(tag, record)
    return got.valid and got.report is not None and write_report(got.report) == record


# -- reports.json's format: "the profile's reading" -----------------------------------------------
#
# Written after reports.json was opened (round 118): the file's description
# names a nested form of a report, ``{source, outcome, last_known, observed?,
# in_force?, refusal?}``, which the text does not hold. These two functions
# are that format and nothing of §2.10's rules.

def fields_of(reading: dict[str, Any]) -> dict[str, Any]:
    """reports.json's nested reading as the record's fields: "observed and
    in_force are {version, revision?} ...; in_force absent is the field
    ``none``, equal to observed in its version is ``observed``, and any other
    is ``other`` with the four in_force fields; refusal is {code, pointer?,
    detail?}, code null for a code of the type's own". Raises
    :class:`zk2py.packed.Refused` (``value``) for a version that does not
    read."""
    out: dict[str, Any] = {"source": reading["source"], "outcome": reading["outcome"],
                           "last_known": reading["last_known"]}

    def version(x: dict[str, Any]) -> Version:
        v = Version.parse(x["version"])
        if v is None:
            raise packed.Refused("value", f"{x['version']!r} is no version (§2.4)")
        return v

    observed = reading.get("observed")
    in_force = reading.get("in_force")
    if in_force is None:
        out["in_force"] = IN_NONE
    elif observed is not None and version(in_force) == version(observed):
        out["in_force"] = IN_OBSERVED
    else:
        out["in_force"] = IN_OTHER
    if observed is not None:
        out.update(version_fields(version(observed), observed.get("revision")))
    if out["in_force"] == IN_OTHER:
        assert in_force is not None
        out.update(version_fields(version(in_force), in_force.get("revision"), IN_FORCE_FIELDS))
    refusal = reading.get("refusal")
    if refusal is not None:
        out["code"] = refusal["code"]
        for k in ("pointer", "detail"):
            if refusal.get(k) is not None:
                out[k] = refusal[k]
    return out


def reading_of_fields(fields: dict[str, Any]) -> dict[str, Any]:
    """A consistent record's fields as reports.json's nested reading."""
    out: dict[str, Any] = {"source": fields["source"], "outcome": fields["outcome"],
                           "last_known": fields["last_known"]}
    observed = stated(fields)
    if observed is not None:
        out["observed"] = {"version": observed.text}
        if fields.get("revision") is not None:
            out["observed"]["revision"] = fields["revision"]
    if fields["in_force"] == IN_OBSERVED and observed is not None:
        out["in_force"] = dict(out["observed"])
    elif fields["in_force"] == IN_OTHER:
        other = stated(fields, IN_FORCE_FIELDS)
        if other is not None:
            out["in_force"] = {"version": other.text}
            if fields.get("in_force_revision") is not None:
                out["in_force"]["revision"] = fields["in_force_revision"]
    if "code" in fields:
        out["refusal"] = {"code": fields["code"]}
        for k in ("pointer", "detail"):
            if k in fields:
                out["refusal"][k] = fields[k]
    return out


# -- §2.5 to §2.10: the target --------------------------------------------------------------------
#
# Written from §2.4 to §2.10 before conformance/v2/order.json was opened
# (round 118).

OWNER, ARCHIVE = v1.OWNER, v1.ARCHIVE
UNSTAMPED, SAME_VERSION, CONFIRMED, OLDER, HELD = "unstamped", "same_version", "confirmed", "older", "held"
ADOPTED, ACT, REVERT, DONE, CALLED, RESTARTED = "adopted", "act", "revert", "done", "called", "restarted"
#: §2.8: "Never under another run ... It is no verdict (``no_verdict``)."
NO_VERDICT = "no_verdict"
#: §2.5's answers to a conditional read: "Answered unchanged, by the held
#: stamp it sent, the target judges nothing"; "A held stamp it did not send
#: is no answer (core §4.2, ``held``)". zk2py's own words for the two: the
#: text gives the step no token.
UNCHANGED, NOT_SENT = "unchanged", "not_sent"
#: §2.6 "What it remembers": "for at least the 16 runs it most lately acted
#: on or adopted a version of". zk2py remembers exactly that many.
REMEMBERED_RUNS = 16


@dataclass(frozen=True)
class Sample:
    """One delivery of the slice (§2.5 "What a read draws"), from either
    path: a put or a live entry (``put``), a delete or a tombstone
    (``delete``), each with its version, or None for "a sample whose payload
    carries no in-band stamp"; from the author (``owner``) or an archive's
    last-known (``archive``, §2.9)."""

    origin: str
    delete: bool
    version: Version | None
    payload: v1.Payload | None = None
    revision: int | None = None


@dataclass(frozen=True)
class NoEntry:
    """§2.5: "A snapshot with no entry is the author's word that it holds no
    document for the target." It "names no version" (§2.6): its header's run
    and its asof."""

    run: Run
    asof: int


@dataclass
class _Held:
    """What the target admitted: a version (a put with its document, or a
    delete), or a snapshot with no entry (``version`` None, ``no_entry``
    set)."""

    version: Version | None
    delete: bool
    payload: v1.Payload | None
    revision: int | None
    last_known: bool
    no_entry: NoEntry | None = None

    @property
    def document(self) -> bool:
        """§2.8: "A target that holds a document's version"."""
        return self.version is not None and not self.delete


@dataclass
class Finish:
    """How the running act ended (v1 §2.7 "Two phases"): ``done``,
    ``refused`` or ``failed``, with the refusal's code, its place (a JSON
    Pointer, or None for nowhere) and its line (or None)."""

    how: str
    code: str | None = None
    place: str | None = None
    line: str | None = None


class TargetRole:
    """One target's role on a desired resource of this major, §2.6 to §2.10,
    as a pure state machine. v1's :class:`zk2py.desired.TargetRole` with "a
    stamp, its id or its time" read "the version, its run and its seq" (§0),
    and §2.8's snapshot with no entry.

    ``pointers`` and ``details`` are its report resource's two tables
    (§2.10), by which a refusal's place and line are written."""

    def __init__(self, pointers: Any = None, details: Any = None, memory: int = REMEMBERED_RUNS) -> None:
        self.pointers = list(DEFAULT_POINTERS if pointers is None else pointers)
        self.details = list(DEFAULT_DETAILS if details is None else details)
        self.memory = memory
        self._fresh()

    def _fresh(self) -> None:
        #: The version last acted on or adopted: the running act's while one
        #: runs (v1 §2.7: "records it as acted on when the act starts").
        self.current: _Held | None = None
        self.running = False
        self.waiting: _Held | None = None
        self.source = BASELINE
        self.outcome = NOT_ACTED
        #: (version, revision) of §2.10's observed version and of the one
        #: wholly in force.
        self.observed: tuple[Version, int | None] | None = None
        self.in_force: tuple[Version, int | None] | None = None
        self.refusal: dict[str, Any] | None = None
        self.last_known = False
        self.report_puts = 1
        #: §2.6: "per run, the highest seq it acted on or adopted a version
        #: of in this process", the most lately used last; the flag says the
        #: seq is a snapshot's asof (§2.8 "What it records").
        self.remembered: dict[Run, tuple[int, bool]] = {}
        #: The stamp of the conditional read outstanding (§2.5), if any.
        self.asked: Version | None = None

    # -- §2.6: what it remembers --------------------------------------------

    def _remember(self, run: Run, seq: int, asof: bool = False) -> None:
        m = self.remembered
        old = m.pop(run, None)
        if old is not None and (old[0] > seq or (old[0] == seq and old[1])):
            seq, asof = old
        m[run] = (seq, asof)
        while len(m) > self.memory:
            del m[next(iter(m))]  # "A run it forgot beyond them is read as a new one"

    # -- §2.6: the order ------------------------------------------------------

    @staticmethod
    def _judge(v: Version, held: _Held) -> str | None:
        """Steps 3 and 4 against one held thing. Against a version: "ordered
        by seq" in its run, else admitted (None). Against a snapshot with no
        entry that waits (§2.8): "of its run, at or below the asof, it is
        older"."""
        if held.no_entry is not None:
            return OLDER if v.run == held.no_entry.run and v.seq <= held.no_entry.asof else None
        assert held.version is not None
        if v.run != held.version.run:
            return None
        if v.seq == held.version.seq:
            return SAME_VERSION
        return OLDER if v.seq < held.version.seq else None

    def _same(self, held: _Held, origin: str) -> str:
        """Step 3 "equal": "the same version ... One equal delivery is not:
        its author's own sample or answer of a version the target holds from
        an archive, or holds waiting from one, confirms it"."""
        if origin == OWNER and held.last_known:
            held.last_known = False
            if held is self.current and not self.running:
                self.last_known = False
                self.report_puts += 1
            return CONFIRMED
        return SAME_VERSION

    def receive(self, s: Sample) -> str:
        """§2.6, one version, "against the one it holds and the runs it
        remembers. While an act runs ... against the running act's version,
        and then against the one it holds waiting"."""
        if s.version is None:
            return UNSTAMPED  # step 1: "Nothing it holds changes."
        v = s.version
        cur = self.current
        if cur is not None and cur.version is not None:
            word = self._judge(v, cur)
            if word == SAME_VERSION:
                return self._same(cur, s.origin)
            if word == OLDER:
                return OLDER
        # Step 4: "never an older version of a run the target remembers ...
        # Lower is older ... Equal is the same version. Higher is admitted."
        # And §2.8: "The target remembers the asof as the seq it acted on
        # under that run. So a version of that run at or below the asof is
        # older".
        latest = self.remembered.get(v.run)
        if latest is not None:
            if v.seq < latest[0] or (v.seq == latest[0] and latest[1]):
                return OLDER
            if v.seq == latest[0]:
                return SAME_VERSION
        if self.running and self.waiting is not None:
            word = self._judge(v, self.waiting)
            if word == SAME_VERSION:
                return self._same(self.waiting, s.origin)
            if word == OLDER:
                return OLDER
        held = _Held(v, s.delete, None if s.delete else s.payload, None if s.delete else s.revision,
                     s.origin == ARCHIVE)
        if self.running:
            self.waiting = held  # "A version admitted so waits (held), in place of the one that waited."
            return HELD
        return self._start(held)

    # -- §2.8: a snapshot with no entry -------------------------------------------

    @staticmethod
    def _judge_no_entry(n: NoEntry, held: _Held | None) -> str | None:
        """§2.8 against one held thing: None when the snapshot is admitted.

        - "A target that holds a document's version of the snapshot's run,
          with a seq at or below the snapshot's asof, reverts".
        - "A snapshot of the held run whose asof is below the held version's
          seq was built before that version. It is older".
        - "A target MUST NOT revert on a snapshot with no entry under another
          run than its held version's. ... A target that holds no document's
          version has nothing to revert: no verdict either."
        """
        if held is None or not held.document:
            return NO_VERDICT
        assert held.version is not None
        if held.version.run != n.run:
            return NO_VERDICT
        return OLDER if n.asof < held.version.seq else None

    def receive_no_entry(self, n: NoEntry) -> str:
        """§2.8: "While an act runs, a snapshot with no entry is judged
        against the running act's version, then against the one held
        waiting, each as above. Admitted by both, it waits as a delete
        does.\""""
        word = self._judge_no_entry(n, self.current)
        if word is not None:
            return word
        if self.running and self.waiting is not None:
            word = self._judge_no_entry(n, self.waiting)
            if word is not None:
                return word
        held = _Held(None, True, None, None, False, n)
        if self.running:
            self.waiting = held
            return HELD
        return self._start(held)

    # -- §2.7: acting -----------------------------------------------------------

    def _start(self, h: _Held) -> str:
        """An admitted version, nothing running: adopted when its document
        equals that of the last version acted on or adopted (step 5), "a
        delete is equal to a delete" (v1 §2.7), else acted on. A snapshot
        with no entry is acted on: it was admitted against a document."""
        prev = self.current
        if h.no_entry is not None:
            self.current = h
            # §2.8 "What it records": "the asof as the seq it acted on under
            # that run", recorded as an act is, when it starts.
            self._remember(h.no_entry.run, h.no_entry.asof, asof=True)
            self.running = True
            return REVERT
        assert h.version is not None
        if prev is not None and v1.same_document(prev.payload, h.payload):
            # v1 §2.7 "The report moves": observed to the adopted version,
            # in_force with it "when it named the previous one"; outcome,
            # refusal and source stay. After a snapshot with no entry: "A
            # delete it admits next is a delete after a delete, adopted
            # (§2.7), and gives the report its version."
            new = (h.version, self._revision(h))
            if self.in_force is not None and self.observed is not None and self.in_force[0] == self.observed[0]:
                self.in_force = new
            self.current = h
            self._remember(h.version.run, h.version.seq)
            self.observed = new
            self.last_known = h.last_known
            self.report_puts += 1
            return ADOPTED
        self.current = h
        self._remember(h.version.run, h.version.seq)
        self.running = True
        return REVERT if h.delete else ACT

    @staticmethod
    def _revision(h: _Held) -> int | None:
        """§2.4: "A delete has no document, so its version has no revision";
        and of a revision above 4,294,967,295, "The report of that refusal
        carries no revision: it has no code.\""""
        if h.delete or h.revision is None or h.revision > MAX_REVISION:
            return None
        return h.revision

    def must_refuse(self) -> bool:
        """§2.4: "A target MUST refuse whole a document whose revision is
        above 4,294,967,295, with the code ``invalid``." True while the
        running act is of such a document."""
        h = self.current
        return bool(self.running and h is not None and h.document and h.revision is not None
                    and h.revision > MAX_REVISION)

    def finish(self, f: Finish) -> str:
        """§2.7, §2.8, §2.10: the running act ended. The report moves, then
        what waits, if anything, is acted on or adopted."""
        if not self.running or self.current is None:
            raise ValueError("no act runs")
        h = self.current
        self.running = False
        how, code = f.how, f.code
        revert = h.delete  # a delete, a tombstone, or a snapshot with no entry
        if self.must_refuse_of(h) and how != REFUSED:
            how, code = REFUSED, "invalid"
        if how == REFUSED and revert:
            how = FAILED  # v1 §2.8: "A revert is never refused; one that could not finish is failed."
        if how == DONE:
            self.outcome, self.refusal = (REVERTED if revert else CONVERGED), None
            self.source = BASELINE if revert else DESIRED
        elif how in (REFUSED, FAILED):
            self.outcome = how
            self.refusal = {"code": code_field(code or (FAILURE_CODE if how == FAILED else None))}
            p, ln = place_index(self.pointers, f.place), line_index(self.details, f.line)
            if p is not None:
                self.refusal["pointer"] = p
            if ln is not None:
                self.refusal["detail"] = ln
            if how == FAILED:
                self.source = BASELINE if revert else DESIRED
        else:
            raise ValueError(f"an unknown end {f.how!r}")
        # §2.10 "The observed version is the last the target acted on or
        # adopted in this process." "A revert on a snapshot with no entry has
        # no observed version (§2.8)."
        self.observed = None if h.version is None else (h.version, self._revision(h))
        if how == DONE:
            self.in_force = None if revert else self.observed
        elif how == FAILED:
            self.in_force = None
        self.last_known = h.last_known
        self.report_puts += 1
        if self.waiting is not None:
            w, self.waiting = self.waiting, None
            return self._start(w)
        return DONE

    @staticmethod
    def must_refuse_of(h: _Held) -> bool:
        return bool(h.document and h.revision is not None and h.revision > MAX_REVISION)

    def call(self) -> str:
        """§2.10: "A call makes ``source`` ``call`` and ``in_force``
        ``none``. The rest stays.\""""
        self.source, self.in_force = CALL, None
        self.report_puts += 1
        return CALLED

    def restart(self) -> str:
        """§2.6: "A new process forgets every run, as it forgets every act"."""
        self._fresh()
        return RESTARTED

    # -- §2.5: the conditional re-read ---------------------------------------------

    def stamp_to_send(self) -> Version | None:
        """§2.5 "The stamp it sends is that of the latest version it admitted
        (§2.6): the one it holds waiting, else the running act's, else the
        last it acted on or adopted. A target that holds no version sends
        none. Core §4.2 forbids any other stamp."

        The latest thing admitted may be a snapshot with no entry, which
        "names no version" (§2.6): zk2py then sends none, while it waits,
        while it is acted on and after it. Core §4.2 allows only "the in-band
        stamp of the mutation it holds for that member", and "Never a
        snapshot's asof" (round 118's third answer)."""
        latest = self.waiting if (self.running and self.waiting is not None) else self.current
        return None if latest is None else latest.version

    def ask(self) -> Version | None:
        """Make a read of the slice: the stamp it carries, None for a read
        that is not conditional."""
        self.asked = self.stamp_to_send()
        return self.asked

    def unchanged(self, stamp: Version) -> str:
        """§2.5 "Answered unchanged, by the held stamp it sent, the target
        judges nothing. Its author holds that version still. It is the
        author's own word of the version, so it confirms a last-known one
        (§2.9)." "A held stamp it did not send is no answer (core §4.2,
        ``held``)." The read it answers has then ended."""
        sent, self.asked = self.asked, None
        if sent is None or stamp != sent:
            return NOT_SENT
        for held in (self.waiting if self.running else None, self.current):
            if held is not None and held.version == stamp:
                return CONFIRMED if self._same(held, OWNER) == CONFIRMED else UNCHANGED
        return UNCHANGED

    # -- §2.10: the report ---------------------------------------------------------

    def report(self) -> dict[str, Any]:
        """§2.10: the report after the last step, as its record's fields."""
        out: dict[str, Any] = {"source": self.source, "outcome": self.outcome, "last_known": self.last_known}
        if self.in_force is None:
            out["in_force"] = IN_NONE
        elif self.observed is not None and self.in_force[0] == self.observed[0]:
            out["in_force"] = IN_OBSERVED
        else:
            out["in_force"] = IN_OTHER
        if self.observed is not None:
            out.update(version_fields(*self.observed))
        if out["in_force"] == IN_OTHER:
            assert self.in_force is not None
            out.update(version_fields(self.in_force[0], self.in_force[1], IN_FORCE_FIELDS))
        if self.refusal is not None:
            out.update(self.refusal)
        return out


#: v1 §2.10: "`apply` for a failure".
FAILURE_CODE = "apply"


# -- order.json's steps (its format, written after the file was opened) ---------------------------

def sample_of(x: dict[str, Any]) -> Sample | NoEntry:
    """order.json's ``sample``: a put's ``document`` (decoded JSON) or
    ``bytes`` (hex), a delete, or "a member's snapshot with no entry, its
    header's run and its asof"."""
    if x["kind"] == "no_entry":
        run = Run.parse(x["snapshot"]["run"])
        assert run is not None
        return NoEntry(run, int(x["snapshot"]["asof"]))
    version = None if x["version"] is None else Version.parse(x["version"])
    if x["kind"] == "delete":
        return Sample(x["from"], True, version)
    payload = v1.Payload(value=x["document"]) if "document" in x else v1.Payload(data=bytes.fromhex(x["bytes"]))
    rev = x.get("revision")
    return Sample(x["from"], False, version, payload, None if rev is None else int(rev))


def run_steps(steps: list[dict[str, Any]]) -> tuple[list[str], dict[str, Any]]:
    """order.json: one role's steps -> each step's word, and the report
    after the last, as its record's fields. A step's Refusal carries
    indices, so the role's tables play no part."""
    t = TargetRole()
    words: list[str] = []
    for s in steps:
        if "sample" in s:
            x = sample_of(s["sample"])
            words.append(t.receive_no_entry(x) if isinstance(x, NoEntry) else t.receive(x))
        elif "finish" in s:
            f = s["finish"]
            if f == "done":
                words.append(t.finish(Finish(DONE)))
            elif f == "check":
                # "the running act's put ends as §2.4's bound judges it"
                words.append(t.finish(Finish(REFUSED, "invalid") if t.must_refuse() else Finish(DONE)))
            else:
                (how, refusal), = f.items()
                words.append(t.finish(Finish(how, refusal.get("code"))))
                _indices(t, how, refusal)
        elif s.get("call"):
            words.append(t.call())
        elif s.get("restart"):
            words.append(t.restart())
        else:
            raise ValueError(f"an unknown step {s!r}")
    return words, t.report()


def _indices(t: TargetRole, how: str, refusal: dict[str, Any]) -> None:
    """Put a step's indices in the refusal the act's end just recorded. The
    refusal is still that act's unless an act that waited has started, which
    changes no report field."""
    if t.refusal is None:
        return
    t.refusal = {"code": refusal.get("code") if refusal.get("code") in CODES or refusal.get("code") is None
                 else None}
    if refusal.get("code") is None and how == FAILED and "code" not in refusal:
        t.refusal["code"] = FAILURE_CODE
    for k in ("pointer", "detail"):
        if refusal.get(k) is not None:
            t.refusal[k] = refusal[k]


# -- §2.5: the watch ------------------------------------------------------------------------------
#
# Written from §2.5 before conformance/v2/watch.json was opened (round 118).

#: The decisions §2.5's rule names: ``no_section``, ``unmoved``, ``read``,
#: ``outstanding``, and ``older`` (step 2). Step 1 names no token:
#: ``not_read`` is zk2py's.
NOT_READ, NO_SECTION, UNMOVED, READ, OUTSTANDING = "not_read", "no_section", "unmoved", "read", "outstanding"
SEQ_MOD = 1 << 24


@dataclass(frozen=True)
class Section:
    """§0: "the part of a heartbeat that covers one collection: its digest
    and its live count (core §7.5)"."""

    digest: int
    live: int


def serial_after(new: int, old: int) -> bool:
    """Core §4.3 "Serial order": "s′ is newer when (s′ − s) mod 2^24 is from
    1 to 2^23 − 1"."""
    return 1 <= (new - old) % SEQ_MOD <= (1 << 23) - 1


class Watch:
    """§2.5 "The watch": "A far target can let its author's heartbeat say
    when to read, in place of a period." One target's watch of one author's
    desired collection, session-free.

    "The target keeps the section it expects: a digest and a live count, or
    none. It starts with none." It reads the section "for one thing: whether
    it moved": nothing here confirms a version, and nothing here ages one
    ("Never a confirmation")."""

    def __init__(self) -> None:
        self.expects: Section | None = None
        #: The last read heartbeat's run and seq (step 3), by the key it came
        #: on: "A group of an owner heartbeat is read as a heartbeat is, its
        #: sequence the owner heartbeat key's." One key where the caller
        #: names none.
        self.lasts: dict[str, tuple[Run, int]] = {}
        self.outstanding = False
        #: "the section of the heartbeat that made it read", None for a read
        #: no heartbeat made (a seed, a period).
        self.made_by: Section | None = None

    @property
    def last(self) -> tuple[Run, int] | None:
        """The last read heartbeat's run and seq, on the one key of a
        caller that names none."""
        return self.lasts.get("")

    def heartbeat(self, run: Run, seq: int, section: Section | None, read: bool = True, key: str = "") -> str:
        """One heartbeat of the author's, or one group of its owner
        heartbeat: "It takes the first of these that answers".

        ``read`` False is a heartbeat ``freshness.v2`` §2.5's steps do not
        read for another reason than its age: "of another revision
        (``revision``), a refused one (``refused``)", and "A group whose
        sections freshness.v2 §2.5 does not read". ``section`` None is "The
        collection's map bit is 0", and "An owner heartbeat with no group
        for the author's interface"."""
        # 1. "Not read. After a heartbeat of another revision or a refused
        # one, the target expects none. It makes no read for it, and the
        # heartbeat moves no sequence."
        if not read:
            self.expects = None
            return NOT_READ
        # 2. "Older. In the last read one's run, its seq is not after that
        # one's, by core §4.3's serial order. Nothing changes, and it moves no
        # sequence."
        last = self.lasts.get(key)
        if last is not None and last[0] == run and not serial_after(seq, last[1]):
            return OLDER
        # 3. "Out of sequence. Any other heartbeat is read, and is the last
        # read one from then on. One of another run than the last read one's,
        # or whose seq is not that one's plus 1 modulo 2^24, follows a change
        # the target did not see. The target expects none, and goes on to
        # step 4."
        self.lasts[key] = (run, seq)
        if last is not None and (last[0] != run or seq != (last[1] + 1) % SEQ_MOD):
            self.expects = None
        # 4. "No section. The collection's map bit is 0. The target expects
        # none, and makes no read for it."
        if section is None:
            self.expects = None
            return NO_SECTION
        # 5. "Unmoved. The section is the one it expects. Nothing is read."
        if section == self.expects:
            return UNMOVED
        # 6. "Moved. The section differs, or the target expects none. It
        # makes one conditional read of its slice (read). Where a read is
        # outstanding it makes none (outstanding)."
        if self.outstanding:
            return OUTSTANDING
        self.outstanding = True
        self.made_by = section
        return READ

    def reading(self) -> bool:
        """A read no heartbeat made begins: the seed, "A seed made again", a
        period's. False where one is outstanding ("A target MUST NOT read
        its slice while a read of it is outstanding")."""
        if self.outstanding:
            return False
        self.outstanding = True
        self.made_by = None
        return True

    def unchanged(self, still_holds: bool) -> None:
        """"Unchanged: the target expects the section of the heartbeat that
        made it read, where it still holds the version it sent. Where it
        holds another by then, it expects none." A read no heartbeat made
        has no such section: what the target expects stays, unless it holds
        another version."""
        self.outstanding = False
        if not still_holds:
            self.expects = None
        elif self.made_by is not None:
            self.expects = self.made_by
        self.made_by = None

    def whole(self) -> None:
        """"Whole: it judges the entry (§2.6), and expects none. The next
        heartbeat makes it read once more, and that read settles what it
        expects.\""""
        self.outstanding = False
        self.expects = None
        self.made_by = None

    def no_answer(self) -> None:
        """"No answer, a timeout or a held stamp it did not send: what it
        expects does not change. The next heartbeat is its next trigger.\""""
        self.outstanding = False
        self.made_by = None

    def own_change(self, key: str, old: tuple[Version, bool] | None, new: tuple[Version, bool]) -> bool:
        """"Its own change costs nothing. A target that admits a version of
        its slice heard live MAY move the digest it expects itself", each
        version given with whether it is a delete. "It moves it only where
        it expects a section and holds a version." True when it moved."""
        from . import heartbeat

        if self.expects is None or old is None:
            return False

        def value(v: Version) -> int:
            return heartbeat.member_value(key, v.run.width, v.run.value, v.seq)

        (was, was_delete), (now, now_delete) = old, new
        digest, live = self.expects.digest, self.expects.live
        if not was_delete and not now_delete:    # "A put over a put XORs the old value out and the new one in."
            digest ^= value(was) ^ value(now)
        elif not was_delete and now_delete:      # "A delete XORs the old value out, and the live count falls by 1."
            digest ^= value(was)
            live -= 1
        elif was_delete and not now_delete:      # "A put over a delete XORs the new value in, and the count rises by 1."
            digest ^= value(now)
            live += 1
        self.expects = Section(digest, live)
        return True


# -- §2.2, §2.10: the declarations ----------------------------------------------------------------
#
# Written from §2.2 and §2.10 before conformance/v2/declarations.json was
# opened (round 118).

#: §2.2 "Only under freshness.v2": "The declaration is invalid,
#: ``horizon_v1``. It is checked last, after v1 §2.2's seven reasons, and
#: skipped where the resource is ``not_state``."
HORIZON_V1 = "horizon_v1"
FRESHNESS_V2 = "freshness.v2"


def declare(kind: str, template: str, annotations: dict[str, Any], params: dict[str, str],
            uses: Any) -> v1.Declaration:
    """§2.2: "v1 §2.2 holds, and ``desired.target_param`` means what it means
    there", with "Its horizon" replaced: "A desired resource that declares
    ``freshness.ttl_s`` above 0 MUST be of a contract that lists
    ``freshness.v2`` in ``uses``." ``uses`` is the contract's."""
    from . import freshness as fr

    d = v1.declare(kind, template, annotations, params)
    if d.declaration == v1.NOT_THIS or "not_state" in d.reasons:
        return d
    above = fr.KEY in annotations and fr.horizon("state", annotations).horizon == "within"
    if above and FRESHNESS_V2 not in (uses or ()):
        return v1.Declaration(v1.INVALID, d.reasons + (HORIZON_V1,), horizon=d.horizon)
    return d


#: §2.10 "A declaration is invalid for each of these, in this order."
REPORT_REASONS = ("not_state", "value", "not_a_role", "not_a_param", "rest_param", "param_not_role", "not_report",
                  "pointers", "details", "two_reports")
ROLE, PARAM = "role", "param"


def report_value(value: Any) -> tuple[str, str] | None:
    """§2.10 "Its value is a string, in one of two forms. A name holds at
    least one character, and no brace. A value in neither form names
    nothing": ``(role, name)`` for "a role's name", ``(param, name)`` for
    "a parameter in braces, as ``{topic}``", else None."""
    if not isinstance(value, str):
        return None
    if value.startswith("{") and value.endswith("}") and len(value) >= 2:
        name = value[1:-1]
        return (PARAM, name) if name and "{" not in name and "}" not in name else None
    return (ROLE, value) if value and "{" not in value and "}" not in value else None


@dataclass(frozen=True)
class ReportResource:
    """One resource of a target's contract as §2.10's declaration reads it:
    its kind token, its template, its resolved annotations, and whether its
    type is "the profile's ``Report`` (§3)", by its artifact's id."""

    token: str
    template: str
    annotations: dict[str, Any]
    is_report: bool

    @property
    def holds_key(self) -> bool:
        return REPORT_KEY in self.annotations

    @property
    def named(self) -> tuple[str, str] | None:
        return report_value(self.annotations.get(REPORT_KEY)) if self.holds_key else None


@dataclass(frozen=True)
class ReportDeclaration:
    declaration: str
    reasons: tuple[str, ...] = ()
    #: declared: ``role`` with the role's name, or ``param`` with the
    #: parameter's.
    form: str | None = None
    name: str | None = None
    pointers: tuple[str, ...] = DEFAULT_POINTERS
    details: tuple[str, ...] = DEFAULT_DETAILS


def _report_resource(r: ReportResource) -> bool:
    """§0 "Report resource: a state resource of a target's contract whose
    resolved annotations hold ``desired.report``.\""""
    return r.holds_key and r.token in ("state", "@state")


def declare_report(r: ReportResource, roles: Any, others: Any = ()) -> ReportDeclaration:
    """§2.10 "The report resource". ``roles`` are the requirements of the
    resource's contract, by name; ``others`` the contract's other resources.

    "A resource without the key is no report resource. On it,
    ``desired.pointers`` and ``desired.details`` mean nothing." "After
    ``value``, no later check reads the value. After ``not_a_param`` or
    ``rest_param``, ``param_not_role`` is skipped\""""
    from . import templates

    if not r.holds_key:
        return ReportDeclaration(v1.NOT_THIS)
    t = templates.parse_template(r.template)
    single = [s.text for s in t.segments if s.kind == templates.PARAM]
    rest = [s.text for s in t.segments if s.kind == templates.REST]
    reasons: list[str] = []
    if r.token not in ("state", "@state"):
        reasons.append("not_state")
    named = r.named
    if named is None:
        reasons.append("value")
    else:
        form, name = named
        skip = False
        if form == ROLE and name not in roles:
            reasons.append("not_a_role")
        if form == PARAM and name not in single and name not in rest:
            reasons.append("not_a_param")
            skip = True
        if form == PARAM and name in rest:
            reasons.append("rest_param")
            skip = True
        # "a role's name ... The resource has no parameter"; "a parameter in
        # braces ...: the resource's one parameter"
        if not skip and any(p != name or form == ROLE for p in t.param_names):
            reasons.append("param_not_role")
    if not r.is_report:
        reasons.append("not_report")
    ann = r.annotations
    if POINTERS_KEY in ann and not pointers_ok(ann[POINTERS_KEY]):
        reasons.append("pointers")
    if DETAILS_KEY in ann and not details_ok(ann[DETAILS_KEY]):
        reasons.append("details")
    # "two_reports: another report resource of the contract names the same
    # role, or one of the two names a parameter." It reads the value, so it
    # is skipped after `value`; another whose value names nothing names no
    # role.
    if named is not None:
        for o in others:
            if o is r or not _report_resource(o) or o.named is None:
                continue
            if PARAM in (named[0], o.named[0]) or named[1] == o.named[1]:
                reasons.append("two_reports")
                break
    if reasons:
        return ReportDeclaration(v1.INVALID, tuple(reasons))
    assert named is not None
    return ReportDeclaration(v1.DECLARED, (), named[0], named[1],
                             tuple(ann.get(POINTERS_KEY, DEFAULT_POINTERS)),
                             tuple(ann.get(DETAILS_KEY, DEFAULT_DETAILS)))


# -- §2.10: which reply is a role's report --------------------------------------------------------
#
# Written from §2.10 "A tool finds a target's report" before
# conformance/v2/slices.json was opened (round 118).

FOUND_ONE, FOUND_UNREADABLE, FOUND_NONE = "one", "unreadable", "none"
TWO_REPORTS = "two_reports"


@dataclass(frozen=True)
class ReportFound:
    """§2.10: what a tool found of one role's report. ``found``: ``one`` (a
    reply read on the member's key, readable; it may still be malformed),
    ``unreadable`` (two resources name the role, ``why`` ``two_reports``; or
    a record core §7.6 refuses, ``why`` its token), or ``none`` (no resource
    names it; one names it and is invalid, its ``reasons`` beside; or no
    reply)."""

    found: str
    why: str | None = None
    key: str | None = None
    reading: ReportReading | None = None
    reasons: tuple[str, ...] = ()
    declaration: ReportDeclaration | None = None

    def as_reading(self) -> ReportReading | None:
        """§2.17 step 3's report: None for none read; an unreadable one
        reads as such."""
        if self.found == FOUND_NONE:
            return None
        if self.found == FOUND_UNREADABLE:
            return self.reading if self.reading is not None else ReportReading(None, self.why or TWO_REPORTS)
        return self.reading


def names_role(r: ReportResource, role: str) -> bool:
    """§2.10 "The report resources of a role are the resources of the
    contract that declares the role whose ``desired.report`` names it: by
    its name, or by a parameter in braces." A report resource is "a state
    resource" (§0)."""
    if not _report_resource(r) or r.named is None:
        return False
    form, name = r.named
    return form == PARAM or name == role


def report_key(target: str, iface: str, r: ReportResource, role: str) -> str:
    """§2.10: "That key is the resource's, its parameter filled with the
    role's name", "slugged as any parameter value is (core §1.4)"."""
    from . import slug, templates

    t = templates.parse_template(r.template)
    chunks = [s.text if s.kind == templates.LITERAL else slug.slug(role) for s in t.segments]
    return f"zk2/{target}/{iface}/{r.token}/" + "/".join(chunks)


def find_report(target: str, iface: str, resources: Any, roles: Any, role: str,
                replies: Any) -> ReportFound:
    """§2.10 "A tool finds a target's report by type identity and by the
    annotation, never by a resource's name." ``resources`` are those of the
    target's contract that declares ``role``, ``roles`` that contract's
    requirements, and ``replies`` S4's GET's, ``(key, payload)``, already
    merged per key ("Two instances' reports on one key are one reply after a
    tool's merge")."""
    resources = list(resources)
    naming = [r for r in resources if names_role(r, role)]
    if not naming:
        return ReportFound(FOUND_NONE)                    # "None names it: no report was read."
    if len(naming) >= 2:
        return ReportFound(FOUND_UNREADABLE, TWO_REPORTS)  # "The target's contract says two things of one role."
    r = naming[0]
    decl = declare_report(r, roles, resources)
    if decl.declaration != v1.DECLARED:
        return ReportFound(FOUND_NONE, reasons=decl.reasons, declaration=decl)
    key = report_key(target, iface, r, role)
    mine = [payload for k, payload in replies if k == key]
    if not mine:
        return ReportFound(FOUND_NONE, key=key, declaration=decl)  # "With no reply, none was read."
    reading = read_put(mine[-1])
    if not reading.readable:
        return ReportFound(FOUND_UNREADABLE, reading.refusal, key, reading, declaration=decl)
    return ReportFound(FOUND_ONE, None, key, reading, declaration=decl)


# -- §2.17, §5: what a tool concludes of one target ---------------------------------------------
#
# Written from §2.17 and §5 before conformance/v2/conclusions.json was
# opened (round 118).

YES, NO, NOT_YET, UNOBSERVABLE, NOT_ASKED = v1.YES, v1.NO, v1.NOT_YET, v1.UNOBSERVABLE, v1.NOT_ASKED
Q_CONVERGED, Q_REFUSED, Q_LAST_KNOWN, Q_IN_FORCE = v1.Q_CONVERGED, v1.Q_REFUSED, v1.Q_LAST_KNOWN, v1.Q_IN_FORCE
QUESTIONS = v1.QUESTIONS
Answer = v1.Answer


@dataclass(frozen=True)
class Reading:
    """One reading of one target's role (v1's :class:`zk2py.desired.Reading`
    with this major's report and versions):

    - ``report``: None when none was read, else its reading (§2.10);
    - ``slice``: whether the tool has the slice, which needs "a revision of
      the author's interface in hand" (§2.17 "The slice from the target
      alone goes");
    - ``author_current``: None for no GET made, ``incomplete``, ``silent``
      (no reply), a :class:`Version` (its snapshot entry's, "a live entry's,
      or a tombstone's"), or a :class:`NoEntry`."""

    target_presence: str
    descriptor: tuple[bool, bool] | None
    report: ReportReading | None
    report_freshness: str
    author_presence: str
    author_current: Any
    deployment: tuple[bool, bool] | None = None
    slice: bool = True


def conclude(r: Reading) -> dict[str, v1.Answer]:
    """§2.17: "v1 §2.17 holds, with these changes", "by the first of these
    that answers"."""
    if r.target_presence == v1.ABSENT:
        return v1._all(NOT_ASKED, "target_absent")
    if r.target_presence == v1.INCOMPLETE:
        return v1._all(UNOBSERVABLE, "presence_incomplete")
    across = r.target_presence == v1.ACROSS_FACE
    said = r.deployment if across else r.descriptor
    if said is None:
        return v1._all(UNOBSERVABLE, "no_descriptor")
    lists, bound = said
    if not lists:
        return v1._all(NOT_ASKED, "not_a_target")
    if not bound:
        out = v1._all(NOT_ASKED, "unbound")
        out[Q_IN_FORCE] = Answer(NO, "unbound")
        return out
    # 3. "Step 3, its report, is found as §2.10 says. No slice is needed to
    # find it."
    if r.report is None:
        return v1._all(UNOBSERVABLE, "nothing_crossed" if across else "no_report")
    if not r.report.valid:
        return v1._all(UNOBSERVABLE, "report_invalid")
    if r.report_freshness == "stale":
        return v1._all(UNOBSERVABLE, "report_stale")
    if r.report_freshness == "unobservable":
        return v1._all(UNOBSERVABLE, "report_unobservable")
    rep = r.report.report or {}
    outcome, source = rep["outcome"], rep["source"]
    out = {}
    # 5. "Step 5 reads the report's fields. not_acted is v1's "no observed"."
    if outcome == NOT_ACTED:
        out[Q_REFUSED] = Answer(NOT_ASKED, "not_acted")
        out[Q_LAST_KNOWN] = Answer(NOT_ASKED, "not_acted")
    else:
        out[Q_REFUSED] = Answer(YES if outcome in (REFUSED, FAILED) else NO, outcome)
        out[Q_LAST_KNOWN] = Answer(YES, "last_known") if rep.get("last_known") else Answer(NO, "current")
    # §5: "in_force observed or other is v1's in_force; none with source
    # desired is partial."
    if source == DESIRED:
        out[Q_IN_FORCE] = Answer(YES, "desired" if rep["in_force"] in (IN_OBSERVED, IN_OTHER) else "partial")
    else:
        out[Q_IN_FORCE] = Answer(NO, source)
    out[Q_CONVERGED] = _converged(r, rep)
    return {q: out[q] for q in QUESTIONS}


def _converged(r: Reading, rep: dict[str, Any]) -> v1.Answer:
    """§2.17 steps 6 and 7, "by run and seq"."""
    outcome = rep["outcome"]
    observed = stated(rep)
    if r.author_presence == v1.ABSENT:
        return Answer(NOT_ASKED, "author_absent")
    if r.author_presence == v1.INCOMPLETE:
        return Answer(UNOBSERVABLE, "author_presence_incomplete")
    # "A tool with no revision of the author's interface in hand has no
    # slice. ... It answers the first not asked, author_absent, where the
    # author is absent, and otherwise unobservable, author_not_read." No
    # slice, no GET: v1's "no GET made".
    cur = r.author_current if r.slice else None
    if cur is None:
        return Answer(UNOBSERVABLE, "author_not_read")
    if cur == "incomplete":
        return Answer(UNOBSERVABLE, "author_read_incomplete")
    if cur == "silent":
        return Answer(UNOBSERVABLE, "author_silent")
    if isinstance(cur, NoEntry):
        # "A snapshot with no entry, under the run of the report's observed
        # version, is the author's word that it holds no document. By the
        # report's outcome: yes, reverted; any other, not yet, behind."
        # "otherwise, under another run or with no observed version:
        # unobservable, author_silent."
        if observed is not None and observed.run == cur.run:
            return Answer(YES, "reverted") if outcome == REVERTED else Answer(NOT_YET, "behind")
        return Answer(UNOBSERVABLE, "author_silent")
    # 7. v1: "no observed: not yet, not_acted". A report that reverted on a
    # snapshot with no entry states no observed version either, and the text
    # names no other answer for it: zk2py reads v1's line as written.
    if observed is None:
        return Answer(NOT_YET, "not_acted")
    assert isinstance(cur, Version)
    if observed.run != cur.run:
        return Answer(NOT_YET, "other_epoch")
    if observed.seq == cur.seq:
        return Answer(YES if outcome in (CONVERGED, REVERTED) else NO, outcome)
    return Answer(NOT_YET, "behind") if observed.seq < cur.seq else Answer(UNOBSERVABLE, "ahead")

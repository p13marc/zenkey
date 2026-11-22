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

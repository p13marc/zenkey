"""``redundancy.v2`` text 0.1: one instance serves, and its followers know
it by its run (#609, round 128; #987).

Written from ``spec/profiles/redundancy/v2.md`` and core 0.119 before
``conformance/v2/`` was run. Every rule of :mod:`zk2py.redundancy` (v1)
holds by reference, "except where §2 replaces it". Session-free:
- the declaration (§2.2): v1's four reasons with ``majors`` before them and
  ``explicit_stream`` after them, each checked alone
  (:func:`declaration`);
- an address's governance from its candidates' revisions, of either major
  (§2.2 "Across revisions", §2.15; :func:`governance`);
- what a follower reads of a descriptor: its run, and the governed
  interface's entry and mark (§2.10, §2.14; :func:`read_descriptor`);
- a sample's run by the form that carries it (§2.10's table;
  :func:`sample_run`);
- the fence over one sample (§2.10 "The rule", §2.17; :func:`decide`);
- the hold for one read, over a follower's sequence of arrivals (§2.10 "An
  unattributed sample"; :class:`Hold`);
- a candidate's draw again (§2.10 "A run no other states";
  :func:`draw_again`).

:mod:`zk2py.redundancy_v2_live` drives the follower on a bus, and
:class:`zk2py.redundancy_live.Candidate` is a candidate of either major.
"""

from __future__ import annotations

import json
import re
from dataclasses import dataclass, field
from typing import Any, Iterable

from . import redundancy as v1
from . import zk2c

PROFILE = "redundancy.v2"
V1 = "redundancy.v1"
KEY = v1.KEY
CLAIM, EXTERNAL = v1.CLAIM, v1.EXTERNAL
ELECTIONS = v1.ELECTIONS

# -- §2.2 the declaration ---------------------------------------------------------

#: §2.2's reasons in the text's order: "This text adds one before them and
#: one after them", v1's four between.
REASONS = ("majors", "not_used", "value", "disagrees", "tokenless", "explicit_stream")


def majors_of(uses: Iterable[str]) -> set[int]:
    """The majors of ``redundancy`` a contract's ``uses`` lists."""
    return v1._majors(list(uses), "redundancy")


def governing_major(uses: Iterable[str]) -> int | None:
    """The major that governs an interface, read as §2.2 and v1 §2.2 read it:
    2 where ``uses`` lists ``redundancy.v2`` (``majors`` makes it invalid,
    but "The interface is read as governed for the checks after it"), 1
    where it lists ``redundancy.v1`` alone, None for neither."""
    m = majors_of(uses)
    if 2 in m:
        return 2
    if 1 in m:
        return 1
    return None


def declaration(uses: Iterable[str], resources: Iterable[dict[str, Any]], tokenless: bool = False) -> v1.Declaration:
    """§2.2 over a contract's ``uses`` and its resources, each
    ``{kind, explicit, annotations}`` with its resolved annotations (core
    §9.3).
    - "Governed is a contract's uses listing redundancy.v2. A contract whose
      uses lists redundancy.v1 and no redundancy.v2 is v1's, and not this
      text's": ``not_governed`` here.
    - "v1's four hold, in v1's order, each checked alone. This text adds one
      before them and one after them": ``majors`` ("uses lists
      redundancy.v2 and another major of redundancy … The interface is read
      as governed for the checks after it"), then v1's ``not_used``,
      ``value``, ``disagrees`` and ``tokenless``, then ``explicit_stream``
      ("the interface is governed, and declares a stream resource with
      explicit = true")."""
    uses = list(uses)
    res = [r if isinstance(r, dict) else {} for r in resources]
    anns = [r.get("annotations") if isinstance(r.get("annotations"), dict) else {} for r in res]
    majors = majors_of(uses)
    governed = 2 in majors
    if not governed and majors:
        # another major's: "that major's text then deciding"
        return v1.Declaration("not_governed")
    carried = [a for a in anns if KEY in a]
    if not governed and not carried:
        return v1.Declaration("not_governed")
    reasons: list[str] = []
    if governed and majors - {2}:
        reasons.append("majors")
    if not governed:
        reasons.append("not_used")
    valid: list[str] = []
    bad = False
    for a in anns:
        if KEY not in a:
            valid.append(CLAIM)
        elif isinstance(a[KEY], str) and a[KEY] in ELECTIONS:
            valid.append(a[KEY])
        else:
            bad = True
    if bad:
        reasons.append("value")
    if len(set(valid)) > 1:
        reasons.append("disagrees")
    if governed and tokenless:
        reasons.append("tokenless")
    if governed and any(r.get("kind") == "stream" and r.get("explicit") is True for r in res):
        reasons.append("explicit_stream")
    if reasons:
        return v1.Declaration("invalid", reasons=tuple(reasons))
    return v1.Declaration("governed", election=valid[0] if valid else CLAIM)


def declaration_of(canonical: dict[str, Any], tokenless: bool = False) -> v1.Declaration:
    """§2.2 over a contract's canonical form (core §9.5): each resource's
    kind, its ``explicit`` and its annotations."""
    return declaration(canonical.get("uses") or [],
                       [{"kind": r.get("kind"), "explicit": r.get("explicit", False),
                         "annotations": r.get("annotations") or {}} for r in canonical.get("resources") or []],
                       tokenless)


def either_declaration(canonical: dict[str, Any], tokenless: bool = False) -> tuple[int | None, v1.Declaration]:
    """The declaration a reader of both majors reads (§2.17): this text's
    for a contract listing ``redundancy.v2``, v1's for one listing
    ``redundancy.v1`` alone, and v1's ``not_used`` reading for one listing
    neither. Returns (the major read, the declaration)."""
    m = governing_major(canonical.get("uses") or [])
    if m == 2:
        return 2, declaration_of(canonical, tokenless)
    return (1 if m == 1 else None), v1.declaration_of(canonical, tokenless)


def lint(canonical: dict[str, Any]) -> list[tuple[str, str]]:
    """§2.2: "A contract linter SHOULD warn of explicit_stream, as v1 §2.2
    says of value and disagrees", for either major's governed contract:
    (reason, the resource it names or "" for the interface)."""
    m = governing_major(canonical.get("uses") or [])
    if m is None:
        return []
    out: list[tuple[str, str]] = []
    rs = canonical.get("resources") or []
    for r in rs:
        a = r.get("annotations") or {}
        if KEY in a and not (isinstance(a[KEY], str) and a[KEY] in ELECTIONS):
            out.append(("value", f"{r.get('token', r.get('kind'))}/{r.get('template')}"))
    d = declaration_of(canonical) if m == 2 else v1.declaration_of(canonical)
    if "disagrees" in d.reasons:
        out.append(("disagrees", ""))
    if m == 2:
        for r in rs:
            if r.get("kind") == "stream" and r.get("explicit") is True:
                out.append(("explicit_stream", f"{r.get('token', '@stream')}/{r.get('template')}"))
    return out


# -- §2.2 "Across revisions", §2.15: an address's governance ------------------------------

NO_CONTRACT, NOT_GOVERNED, DISAGREE = "no_contract", "not_governed", "disagree"


def governance(revisions: Iterable[dict[str, Any]]) -> str:
    """The governance a tool reads from every candidate's revision, of
    either major (§2.2, §2.15; v1 §2.15's third premise). Each revision is
    ``{declaration: governed|not_governed|invalid|unread, major?,
    election?}``. "A revision unread or invalid gives no election, a
    revision not governed beside a governed one disagrees, and so do two
    elections." "A redundancy.v1 candidate and a redundancy.v2 one, with one
    election, are one election": the major takes no part."""
    revs = list(revisions)
    if not revs:
        return NO_CONTRACT
    kinds = [r.get("declaration") for r in revs]
    if any(k not in ("governed", "not_governed") for k in kinds):
        return NO_CONTRACT
    elections = {r.get("election") for r in revs if r.get("declaration") == "governed"}
    if not elections:
        return NOT_GOVERNED
    if "not_governed" in kinds or len(elections) > 1:
        return DISAGREE
    return elections.pop()


# -- §2.10, §2.14: what a follower reads of a descriptor -------------------------------------

_RUN = re.compile(r"[0-9a-f]{4}|[0-9a-f]{8}")


@dataclass(frozen=True, order=True)
class Run:
    """A run, by width and value (core §4.3: "Two runs of different widths
    are two runs"), spelled as a descriptor spells it."""

    width: int
    value: int

    @classmethod
    def parse(cls, text: Any) -> Run | None:
        """"4 or 8 lowercase hexadecimal digits (core §3.3)"; anything else
        is no run (D000, D002)."""
        if not isinstance(text, str) or _RUN.fullmatch(text) is None:
            return None
        return cls(len(text) * 4, int(text, 16))

    @classmethod
    def of(cls, stamp: zk2c.InBand | None) -> Run | None:
        return None if stamp is None else cls(stamp.width, stamp.run)

    def __str__(self) -> str:
        return f"{self.value:0{self.width // 4}x}"


@dataclass(frozen=True)
class DescriptorReading:
    """What a follower reads of one descriptor: its run (None where it
    states none or spells it otherwise), whether an entry is the governed
    interface's, and that entry's mark and findings (v1 §2.14)."""

    run: Run | None
    listed: bool
    mark: str | None
    findings: tuple[str, ...]

    def as_json(self) -> dict[str, Any]:
        return {"run": None if self.run is None else str(self.run), "listed": self.listed, "mark": self.mark,
                "findings": list(self.findings)}


def read_descriptor(doc: Any, iface: str, governance_: str) -> DescriptorReading:
    """§2.10: "A follower MUST read a descriptor's run and mark exactly as the
    fixture does." "The entry it reads is the first of the descriptor's
    interfaces that is not an object, or whose iface is the governed
    interface's. One that is not an object is malformed. No later entry is
    read for the mark. The run is read whatever the entries hold." The mark
    and its findings are v1 §2.14's (:func:`zk2py.redundancy.mark`), under
    the governance the reader knows."""
    if not isinstance(doc, dict):
        return DescriptorReading(None, False, None, ())
    run = Run.parse(doc.get("run"))
    entries = doc.get("interfaces")
    entry: Any = None
    found = False
    if isinstance(entries, list):
        for e in entries:
            if not isinstance(e, dict) or e.get("iface") == iface:
                entry, found = e, True
                break
    if not found:
        return DescriptorReading(run, False, None, ())
    m, findings = v1.mark(entry, governance_)
    return DescriptorReading(run, True, m, tuple(findings))


def descriptor_run(payload: bytes | str | None) -> Run | None:
    """The run of a descriptor's bytes, None where they do not read."""
    if payload is None:
        return None
    try:
        doc = json.loads(payload)
    except (ValueError, TypeError):
        return None
    return Run.parse(doc.get("run")) if isinstance(doc, dict) else None


# -- §2.10: a sample's run ------------------------------------------------------------------

#: §2.10's table: the forms whose run is an envelope's, a put's payload
ENVELOPE_FORMS = ("mutation", "frame", "heartbeat", "owner_heartbeat")
REPLY_FORMS = ("snapshot", "held_stamp")
CALL_FORMS = ("value", "error")


def sample_run(form: str, data: bytes | None, chunk: str | None = None) -> Run | None:
    """§2.10's table: "mutation, frame, heartbeat, owner heartbeat (a put's
    payload): its envelope's; delete: its attachment's; snapshot, held stamp
    (a GET's reply): its header's, for every entry the snapshot lists;
    event: its key's occurrence chunk's (core §1.2); value, error (a call's
    reply): the replying instance's as-of mark (core §5.1)". "A sample with
    no envelope has no run": ``none``, and bytes §7.5's parse refuses."""
    if form == "none" or data is None:
        return None
    carrier = ("put" if form in ENVELOPE_FORMS or form in REPLY_FORMS or form == "event"
               else "delete" if form == "delete" else "reply" if form == "value"
               else "reply_err" if form == "error" else None)
    if carrier is None:
        return None
    try:
        env = zk2c.parse(bytes(data), carrier, last_chunk=chunk if form == "event" else None)
    except (zk2c.Refused, ValueError):
        return None
    return Run.of(env.stamp)


# -- §2.10 "The rule", §2.17 -----------------------------------------------------------------

ACCEPT, DISCARD, PENDING = "accept", "discard", "pending"
INCUMBENT, NO_INCUMBENT, NO_RUN, NOT_INCUMBENT, UNATTRIBUTED, NO_STAMP = (
    "incumbent", "no_incumbent", "no_run", "not_incumbent", "unattributed", "no_stamp")


@dataclass
class Known:
    """What a follower holds of one instance whose descriptor it read: the
    major of ``redundancy`` its revision uses (None where not known), every
    run its descriptors stated, and for a ``redundancy.v1`` revision its
    ``meta.zid`` where read."""

    major: int | None = None
    runs: set[Run] = field(default_factory=set)
    zid: str | None = None


@dataclass
class View:
    """A follower's state (§2.10): the instances holding the interface's
    token, and what it read of each instance's descriptors."""

    holders: set[str] = field(default_factory=set)
    instances: dict[str, Known] = field(default_factory=dict)

    @classmethod
    def of(cls, j: dict[str, Any]) -> View:
        inst = {}
        for i, k in (j.get("instances") or {}).items():
            runs = {r for r in (Run.parse(x) for x in k.get("runs") or []) if r is not None}
            inst[i] = Known(k.get("major"), runs, k.get("zid", k.get("meta_zid")))
        return cls(set(j.get("holders") or []), inst)

    def incumbent(self) -> str | None:
        """"During a split, it is the lowest holder the follower sees" (v1
        §2.3's order)."""
        return v1.lowest(self.holders)

    def owner_of(self, run: Run) -> list[str]:
        """Every instance whose descriptors stated ``run``."""
        return sorted((i for i, k in self.instances.items() if run in k.runs), key=v1.id_key)


@dataclass(frozen=True)
class Decision:
    decision: str
    reason: str
    run: Run | None

    def as_json(self) -> dict[str, Any]:
        return {"decision": self.decision, "reason": self.reason, "run": None if self.run is None else str(self.run)}


def decide(view: View, run: Run | None, stamp: str | None = None, read_ended: bool = False) -> Decision:
    """§2.10's rule over one sample, "the first of these that holds":
    1. no incumbent: ``no_incumbent``;
    2. the incumbent's revision uses ``redundancy.v1``: v1 §2.10 by the
       stamp, "Its run is not read": ``no_stamp``, ``unattributed`` (no
       ``meta.zid`` held: "v1 holds no sample for a read"), the stamp's id
       the incumbent's ``meta.zid`` by value (observer §4.4), else
       ``not_incumbent``;
    3. no run: ``no_run``;
    4. one of the incumbent's runs: accepted;
    5. one of another instance's that the follower holds: ``not_incumbent``;
    6. unattributed: pending until a read of the descriptors asked after it
       arrived has ended (``read_ended``), then discarded.
    "An incumbent whose major is not known, its descriptor not held yet, is
    read by run, from step 3 on." "A run two instances state is the
    incumbent's, by step 4." ``run`` is the sample's by §2.10's table, and
    is reported whichever step decides."""
    from .live import _zid_value

    inc = view.incumbent()
    if inc is None:
        return Decision(DISCARD, NO_INCUMBENT, run)
    k = view.instances.get(inc)
    if k is not None and k.major == 1:
        if stamp is None:
            return Decision(DISCARD, NO_STAMP, run)
        if k.zid is None or _zid_value(k.zid) is None:
            return Decision(DISCARD, UNATTRIBUTED, run)
        if _zid_value(stamp) == _zid_value(k.zid):
            return Decision(ACCEPT, INCUMBENT, run)
        return Decision(DISCARD, NOT_INCUMBENT, run)
    if run is None:
        return Decision(DISCARD, NO_RUN, run)
    if k is not None and run in k.runs:
        return Decision(ACCEPT, INCUMBENT, run)
    if any(run in o.runs for i, o in view.instances.items() if i != inc):
        return Decision(DISCARD, NOT_INCUMBENT, run)
    return Decision(DISCARD if read_ended else PENDING, UNATTRIBUTED, run)


# -- §2.10 "An unattributed sample": the hold for one read ----------------------------------------


@dataclass
class Held:
    """One held sample: its id, key, kind, run, seq, and whether it arrived
    before the outstanding read was asked."""

    id: str
    key: str
    kind: str
    run: Run | None
    seq: int | None
    before_read: bool = False
    value: Any = None
    #: its Zenoh timestamp's id, which decides it under a ``redundancy.v1``
    #: incumbent learnt while it is held (step 2)
    stamp: str | None = None


@dataclass
class Step:
    """What one arrival did (``fences.json``'s sequences): the samples
    accepted, discarded (with reasons), held and superseded, and whether a
    read was asked."""

    accepted: list[str] = field(default_factory=list)
    discarded: list[dict[str, str]] = field(default_factory=list)
    held: list[str] = field(default_factory=list)
    superseded: list[str] = field(default_factory=list)
    asks_read: bool = False

    def as_json(self) -> dict[str, Any]:
        return {"accepted": self.accepted, "discarded": self.discarded, "held": self.held,
                "superseded": self.superseded, "asks_read": self.asks_read}


class Hold:
    """A follower of ``redundancy.v2`` incumbents over its arrivals
    (§2.10):
    - "A follower MUST NOT discard an unattributed sample before a read of
      the address's descriptors, asked after the sample arrived, has ended."
    - "It decides the sample by the rule as soon as a descriptor it reads
      or hears states the run. It decides it again when that read ends.
      Still unattributed then, it discards it."
    - "It asks one such read at a time. A sample that arrives while one
      runs waits for the next, which the follower asks as that one ends."
    - "A token decides nothing": a token changes the holders, and decides
      no held sample.
    - "A state member. Meanwhile the follower holds one unattributed value
      of the member: the one core §4.3's consumer rule would keep. That is
      the higher seq within one run, and the later arrival across two. At
      an equal seq within one run it keeps the value it holds."
    - "Every other sample is held alone: a stream sample, an event and a
      heartbeat, two on one key included."
    ``stamp`` decides a ``redundancy.v1`` incumbent's sample, step 2."""

    def __init__(self, view: View) -> None:
        self.view = view
        self.held: list[Held] = []
        #: whether a read of the descriptors is outstanding
        self.reading = False

    def _decide_held(self, step: Step, at_read_end: bool) -> None:
        keep: list[Held] = []
        for h in self.held:
            d = decide(self.view, h.run, h.stamp, read_ended=at_read_end and h.before_read)
            if d.decision == ACCEPT:
                step.accepted.append(h.id)
                self.on_accept(h, d)
            elif d.decision == DISCARD:
                step.discarded.append({"id": h.id, "reason": d.reason})
                self.on_discard(h, d)
            else:
                keep.append(h)
        self.held = keep

    # hooks for a live follower
    def on_accept(self, h: Held, d: Decision) -> None:  # pragma: no cover - a live follower's
        pass

    def on_discard(self, h: Held, d: Decision) -> None:  # pragma: no cover - a live follower's
        pass

    def _ask(self, step: Step) -> None:
        if self.held and not self.reading:
            self.reading = True
            for h in self.held:
                h.before_read = True
            step.asks_read = True

    def sample(self, sid: str, key: str, kind: str, run: Run | None, seq: int | None, stamp: str | None = None,
               value: Any = None) -> Step:
        step = Step()
        d = decide(self.view, run, stamp)
        if d.decision == ACCEPT:
            step.accepted.append(sid)
            self.on_accept(Held(sid, key, kind, run, seq, value=value, stamp=stamp), d)
            return step
        if d.decision == DISCARD:
            step.discarded.append({"id": sid, "reason": d.reason})
            self.on_discard(Held(sid, key, kind, run, seq, value=value, stamp=stamp), d)
            return step
        new = Held(sid, key, kind, run, seq, value=value, stamp=stamp)
        if kind == "state":
            old = next((h for h in self.held if h.kind == "state" and h.key == key), None)
            if old is not None:
                if old.run == new.run and (new.seq or 0) <= (old.seq or 0):
                    step.superseded.append(new.id)
                    self._ask(step)
                    return step
                step.superseded.append(old.id)
                self.held.remove(old)
        self.held.append(new)
        step.held.append(new.id)
        self._ask(step)
        return step

    def token(self, instance: str, held: bool) -> Step:
        if held:
            self.view.holders.add(instance)
        else:
            self.view.holders.discard(instance)
        return Step()

    def descriptor(self, instance: str, run: Run | None, major: int | None = None, zid: str | None = None) -> Step:
        """A descriptor's put heard, or one descriptor read: its run kept "for
        as long as it follows the address", and each held sample it
        attributes decided."""
        step = Step()
        self._learn(instance, run, major, zid)
        self._decide_held(step, at_read_end=False)
        return step

    def _learn(self, instance: str, run: Run | None, major: int | None, zid: str | None) -> None:
        k = self.view.instances.setdefault(instance, Known())
        if run is not None:
            k.runs.add(run)
        if major is not None:
            k.major = major
        if zid is not None:
            k.zid = zid

    def read_ends(self, descriptors: Iterable[tuple[str, Run | None, int | None, str | None]] = ()) -> Step:
        """The outstanding read's end, with the descriptors it read: every
        held sample decided, those that arrived before the read was asked
        and are still unattributed discarded; then the next read asked if
        any is still held."""
        step = Step()
        for inst, run, major, zid in descriptors:
            self._learn(inst, run, major, zid)
        self.reading = False
        self._decide_held(step, at_read_end=True)
        self._ask(step)
        return step


# -- §2.10 "A run no other states" ----------------------------------------------------------------


def draw_again(own: str | Run, read: Iterable[str | Run | None]) -> bool:
    """"A candidate SHOULD draw a new run before it serves, when it reads its
    own run in the descriptor of another instance of its address." "Two
    runs are one where their widths and their values are." A spelling that
    is no run matches none."""
    mine = own if isinstance(own, Run) else Run.parse(own)
    if mine is None:
        return False
    return any((r if isinstance(r, Run) else Run.parse(r)) == mine for r in read)

"""``redundancy.v1`` text 0.1: one instance serves, the others stand by.

Written from ``spec/profiles/redundancy/v1.md`` alone, with core 0.39's
standby mark (core §3.3, §6). Session-free: the declaration (§2.2), an
entry's mark (§2.4, §2.14), one read's election (§2.5, §2.11), a heal
(§2.9), the state machine a runtime drives (§2.5 to §2.11, the fixture
``lifecycle.json``), and a tool's four questions (§2.15, §5).
:mod:`zk2py.redundancy_live` drives :class:`Machine` on a bus.
"""

from __future__ import annotations

import re
from dataclasses import dataclass, field
from typing import Any, Iterable

PROFILE = "redundancy.v1"
KEY = "redundancy.election"
CLAIM, EXTERNAL = "claim", "external"
ELECTIONS = (CLAIM, EXTERNAL)

_PROFILE_ID = re.compile(r"(.+)\.v(0|[1-9][0-9]*)")

# -- §2.2 the declaration -------------------------------------------------------


@dataclass(frozen=True)
class Declaration:
    """§2.2: ``not_governed``; ``governed`` with its election; or
    ``invalid`` with its reasons, in the text's order."""

    declaration: str
    election: str | None = None
    reasons: tuple[str, ...] = ()

    def as_json(self) -> dict[str, Any]:
        if self.declaration == "governed":
            return {"declaration": "governed", "election": self.election}
        if self.declaration == "invalid":
            return {"declaration": "invalid", "reasons": list(self.reasons)}
        return {"declaration": "not_governed"}

    @property
    def governed(self) -> bool:
        return self.declaration == "governed"


def _majors(uses: Iterable[str], name: str) -> set[int]:
    out = set()
    for u in uses:
        m = _PROFILE_ID.fullmatch(u) if isinstance(u, str) else None
        if m is not None and m.group(1) == name:
            out.add(int(m.group(2)))
    return out


def declaration(uses: Iterable[str], annotations: Iterable[dict[str, Any]], tokenless: bool = False) -> Declaration:
    """§2.2 over a contract's ``uses`` and each resource's resolved
    annotations (core §9.3). "Governed is a contract's uses listing
    redundancy.v1"; "one listing another major of redundancy is that
    major's". The reasons, "checked in this order, each alone": not_used,
    value, disagrees, tokenless. "Each resource's election is its resolved
    value, or claim where it carries none"; "A resource found value takes
    no part" in disagrees; "An interface with no resource is claim"."""
    uses = list(uses)
    anns = [a if isinstance(a, dict) else {} for a in annotations]
    majors = _majors(uses, "redundancy")
    governed = 1 in majors
    if not governed and majors:
        return Declaration("not_governed")
    carried = [a for a in anns if KEY in a]
    if not governed and not carried:
        return Declaration("not_governed")
    reasons: list[str] = []
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
    if reasons:
        return Declaration("invalid", reasons=tuple(reasons))
    return Declaration("governed", election=valid[0] if valid else CLAIM)


def declaration_of(canonical: dict[str, Any], tokenless: bool = False) -> Declaration:
    """§2.2 over a contract's canonical form, where "the canonical form
    carries it on each" resource (core §9.5)."""
    return declaration(canonical.get("uses") or [],
                       [r.get("annotations") or {} for r in canonical.get("resources") or []], tokenless)


# -- §2.4, §2.14 an entry's mark ----------------------------------------------------

STANDBY, SERVING = "standby", "serving"


def mark(entry: Any, governance: str) -> tuple[str | None, list[str]]:
    """An interface entry's mark, and §2.14's findings, in the fixture's
    order: ``malformed`` first and alone (core D000: not an object, or
    ``standby`` or ``token`` present and no boolean, null included), then
    ``standby_tokenless`` (core D012), ``governed_tokenless`` (token false,
    governed under either election), ``not_governed`` (a mark on an
    interface no profile governs). ``governance`` is ``claim``,
    ``external``, ``not_governed`` or ``no_contract``. "standby absent is
    false, token absent is true"."""
    if not isinstance(entry, dict):
        return None, ["malformed"]
    for k in ("standby", "token"):
        if k in entry and not isinstance(entry[k], bool):
            return None, ["malformed"]
    standby = entry.get("standby", False)
    token = entry.get("token", True)
    findings = []
    if standby and not token:
        findings.append("standby_tokenless")
    if not token and governance in ELECTIONS:
        findings.append("governed_tokenless")
    if standby and governance == "not_governed":
        findings.append("not_governed")
    return (STANDBY if standby else SERVING), findings


# -- reads ----------------------------------------------------------------------------

#: An instance's descriptor as a read shows it (``elections.json``).
UNREAD, NOT_LISTED = "unread", "not_listed"


@dataclass(frozen=True)
class Seen:
    """One instance a read found: its id, whether it holds the governed
    interface's token (at any revision), and its descriptor: ``unread``,
    ``not_listed``, ``standby`` or ``serving``."""

    id: str
    token: bool
    descriptor: str

    @property
    def candidate(self) -> bool:
        """§2.3: "an instance of the address whose descriptor lists the
        governed interface, marked standby or not"."""
        return self.descriptor in (STANDBY, SERVING)


@dataclass(frozen=True)
class Read:
    """One read of the address (§0): complete by core §8.1 (0.37, 0.38), and
    the instances it found. A read not made is ``Read(False, ())``."""

    complete: bool
    instances: tuple[Seen, ...] = ()

    @classmethod
    def of(cls, j: dict[str, Any]) -> Read:
        return cls(bool(j["complete"]), tuple(Seen(i["id"], bool(i["token"]), i["descriptor"])
                                              for i in j.get("instances") or []))

    def ids(self) -> list[str]:
        return [i.id for i in self.instances]

    def holders(self) -> set[str]:
        return {i.id for i in self.instances if i.token}

    def get(self, ident: str) -> Seen | None:
        for i in self.instances:
            if i.id == ident:
                return i
        return None


def id_key(ident: str) -> int:
    """§2.3: "Instance ids compare as their 16 lowercase hexadecimal digits
    … character by character, which is their order as unsigned 64-bit
    integers"."""
    return int(ident, 16)


def lowest(ids: Iterable[str]) -> str | None:
    s = sorted(set(ids), key=id_key)
    return s[0] if s else None


# -- §2.5, §2.11 one read's election -------------------------------------------------


@dataclass(frozen=True)
class Decision:
    """``take_over``, or ``stand_by`` with why (``incumbent``,
    ``incomplete``, ``own_claim_missing``, ``descriptor_unread``,
    ``lower_candidate``) and the instance it names, where it names one."""

    decision: str
    why: str | None = None
    instance: str | None = None

    def as_json(self) -> dict[str, Any]:
        out: dict[str, Any] = {"decision": self.decision}
        if self.why is not None:
            out["why"] = self.why
        if self.instance is not None:
            out["instance"] = self.instance
        return out


def elect(own: str, read: Read, election: str = CLAIM) -> Decision:
    """§2.5's steps 2 to 4 over one read, from ``own``'s view; under
    ``external`` with its lock held, §2.11's: steps 2 and 3, no step 4.
    - Step 2: another instance holding the token, "whether or not the read
      is complete, since a read cannot invent a token": stand by, naming
      the lowest such.
    - Step 3: the read not complete; its own claim missing; under claim,
      a lower instance whose descriptor was not read.
    - Step 4: a lower candidate stands it by. "own's own descriptor reading
      plays no part"."""
    others = [i.id for i in read.instances if i.token and i.id != own]
    if others:
        return Decision("stand_by", "incumbent", lowest(others))
    if not read.complete:
        return Decision("stand_by", "incomplete")
    if read.get(own) is None:
        return Decision("stand_by", "own_claim_missing")
    if election == EXTERNAL:
        return Decision("take_over")
    below = sorted((i for i in read.instances if id_key(i.id) < id_key(own) and i.descriptor != NOT_LISTED),
                   key=lambda i: id_key(i.id))
    if below:
        first = below[0]
        return Decision("stand_by", "descriptor_unread" if first.descriptor == UNREAD else "lower_candidate", first.id)
    return Decision("take_over")


# -- §2.9 the heal -------------------------------------------------------------------


def heal(own: str, election: str, first: Iterable[str], second: Iterable[str]) -> tuple[bool, str]:
    """§2.9: whether an incumbent ``own`` steps down, over the holders of
    two reads a grace apart. (True, the lower holder) or (False, why:
    ``external``, ``not_holding``, ``no_split``, ``lowest``). "A read's
    completeness plays no part: a read cannot invent a holder"."""
    h1, h2 = set(first), set(second)
    if election == EXTERNAL:
        return False, "external"
    if own not in h2:
        return False, "not_holding"
    if len(h1) < 2 or len(h2) < 2:
        return False, "no_split"
    low = lowest(h2)
    assert low is not None
    if id_key(low) < id_key(own):
        return True, low
    return False, "lowest"


# -- §2.5 to §2.11 the state machine --------------------------------------------------

UNCLAIMED, CLAIMING, CAMPAIGNING, STANDING_BY, TAKING_OVER = (
    "unclaimed", "claiming", "campaigning", "standing_by", "taking_over")
SERVING_STATE, HEAL_FIRST, HEAL_GRACE, HEAL_SECOND, STEPPING_DOWN = (
    "serving", "heal_first", "heal_grace", "heal_second", "stepping_down")

#: The events a runtime hands the machine (``lifecycle.json``).
START, FINISH, WITHDRAWN, TIMER = "start", "finish", "withdrawn", "timer"


@dataclass
class Machine:
    """One candidate ``own`` of a governed interface (``lifecycle.json``).
    :meth:`step` takes one event and returns the step its runtime does:
    ``claim``, ``read``, ``grace``, ``stand_by`` (why, instance, retry),
    ``take_over``, ``step_down`` (why, instance) or ``nothing``. The role
    is ``unclaimed`` before the claim's finish, ``standby`` while
    campaigning, standing by or taking over, ``serving`` while serving,
    healing or stepping down."""

    own: str
    election: str = CLAIM
    state: str = UNCLAIMED
    #: under external: the lock held, as last heard
    lock: bool = False
    #: the stand_by in force armed the retry timer
    retry: bool = False
    #: an appearance heard while taking over (the heal starts once it serves)
    appeared: bool = False
    #: a withdrawal heard while stepping down (it campaigns once it stands by)
    withdrawn: bool = False
    #: the heal's first read's holders, own counted
    first: set[str] = field(default_factory=set)
    #: the instance the last stand_by or step_down named
    named: str | None = None
    why: str | None = None
    #: its former ids, after a re-mint (F-209): its own, never another's
    former: set[str] = field(default_factory=set)

    def remint(self, new: str) -> None:
        """A re-mint in its own process (core §1.5, §8.1): its id is ``new``
        from now on, and its former ids are its own. The fixture's machine
        has no such event (F-209): without it, the new id's token is
        another holder's appearance, and the overlap a split."""
        self.former.add(self.own)
        self.own = new

    def _ours(self, holders: set[str]) -> set[str]:
        return (holders - self.former) | {self.own}

    @property
    def role(self) -> str:
        if self.state in (UNCLAIMED, CLAIMING):
            return "unclaimed"
        if self.state in (CAMPAIGNING, STANDING_BY, TAKING_OVER):
            return "standby"
        return "serving"

    @property
    def external(self) -> bool:
        return self.election == EXTERNAL

    def _stand_by(self, why: str, instance: str | None = None, retry: bool | None = None) -> dict[str, Any]:
        """``retry is false for incumbent and no_lock, true otherwise``."""
        self.state, self.why, self.named = STANDING_BY, why, instance
        self.retry = retry if retry is not None else why not in ("incumbent", "no_lock")
        out: dict[str, Any] = {"step": "stand_by", "why": why, "retry": self.retry}
        if instance is not None:
            out["instance"] = instance
        return out

    def _campaign(self) -> dict[str, Any]:
        """"a read, or under external without the lock stand_by no_lock"."""
        if self.external and not self.lock:
            return self._stand_by("no_lock")
        self.state = CAMPAIGNING
        return {"step": "read"}

    def _may_campaign(self) -> bool:
        """A withdrawal campaigns "under external only with the lock"."""
        return not self.external or self.lock

    def step(self, event: Any) -> dict[str, Any]:
        nothing: dict[str, Any] = {"step": "nothing"}
        if isinstance(event, dict):
            (kind, value), = event.items()
        else:
            kind, value = event, None
        if kind == START:
            if self.state != UNCLAIMED:
                return nothing
            self.state = CLAIMING
            return {"step": "claim"}
        if kind == FINISH:
            return self._finish()
        if kind == "read":
            r = value if isinstance(value, Read) else Read.of(value)
            return self._read(r)
        if kind == "appeared":
            if value == self.own or value in self.former or self.external:
                return nothing
            if self.state == SERVING_STATE:
                self.state = HEAL_FIRST
                return {"step": "read"}
            if self.state == TAKING_OVER:
                self.appeared = True
            return nothing
        if kind == WITHDRAWN:
            if self.state == STANDING_BY and self._may_campaign():
                return self._campaign()
            if self.state == STEPPING_DOWN:
                self.withdrawn = True
            return nothing
        if kind == TIMER:
            if self.state == STANDING_BY and self.retry:
                return self._campaign()
            if self.state == HEAL_GRACE:
                self.state = HEAL_SECOND
                return {"step": "read"}
            return nothing
        if kind == "lock":
            if not self.external:
                return nothing
            taken = bool(value) and not self.lock
            self.lock = bool(value)
            if taken and self.state == STANDING_BY:
                return self._campaign()
            if not self.lock and self.state == SERVING_STATE:
                self.state = STEPPING_DOWN
                self.why, self.named = "lock_lost", None
                return {"step": "step_down", "why": "lock_lost"}
            return nothing
        return nothing

    def _finish(self) -> dict[str, Any]:
        if self.state == CLAIMING:
            return self._campaign()
        if self.state == TAKING_OVER:
            self.state = SERVING_STATE
            appeared, self.appeared = self.appeared, False
            if self.external and not self.lock:
                self.state = STEPPING_DOWN
                self.why, self.named = "lock_lost", None
                return {"step": "step_down", "why": "lock_lost"}
            if not self.external and appeared:
                self.state = HEAL_FIRST
                return {"step": "read"}
            return {"step": "nothing"}
        if self.state == STEPPING_DOWN:
            withdrawn, self.withdrawn = self.withdrawn, False
            self.state, self.retry = STANDING_BY, False
            if withdrawn and self._may_campaign():
                return self._campaign()
            return {"step": "nothing"}
        return {"step": "nothing"}

    def _read(self, r: Read) -> dict[str, Any]:
        if self.state == CAMPAIGNING:
            if self.external and not self.lock:
                return self._stand_by("no_lock")
            d = elect(self.own, r, self.election)
            if d.decision == "take_over":
                self.state, self.why, self.named = TAKING_OVER, None, None
                return {"step": "take_over"}
            assert d.why is not None
            return self._stand_by(d.why, d.instance)
        if self.state == HEAL_FIRST:
            holders = self._ours(r.holders())
            if len(holders) >= 2:
                self.first, self.state = holders, HEAL_GRACE
                return {"step": "grace"}
            self.state = SERVING_STATE
            return {"step": "nothing"}
        if self.state == HEAL_SECOND:
            down, who = heal(self.own, self.election, self.first, self._ours(r.holders()))
            if down:
                self.state, self.why, self.named = STEPPING_DOWN, "lower_holder", who
                return {"step": "step_down", "why": "lower_holder", "instance": who}
            self.state = SERVING_STATE
            return {"step": "nothing"}
        return {"step": "nothing"}


def given_back(step: dict[str, Any], others_heard: Iterable[str], incomplete_heal: bool = False) -> list[Any]:
    """The events a runtime hands its machine again from what its watch
    holds after a step (F-199), since ``lifecycle.json``'s machine drops
    both while it campaigns ("Every other event in every other state is
    nothing"):
    - after a ``take_over``, each other holder its watch hears is an
      ``appeared``: a holder that appeared during the read before it would
      otherwise leave a split no heal starts, the lower one's heal keeping
      it serving and the higher never reading;
    - after a ``stand_by`` on an incumbent, a watch hearing no holder is a
      ``withdrawn``: an incumbent the read showed and that went while it ran
      would otherwise leave it standing by on nobody, no retry armed;
    - after a heal's read that was not complete sent it back to serving
      (``incomplete_heal``), each other holder its watch still hears is an
      ``appeared`` again, which the runtime hands back after its retry
      interval: "a read not made shows none" (§2.9), so a heal whose read
      could not be made would let the split go for good."""
    others = sorted(set(others_heard), key=id_key)
    if step.get("step") == "take_over" or (step.get("step") == "nothing" and incomplete_heal):
        return [{"appeared": o} for o in others]
    if step.get("step") == "stand_by" and step.get("why") == "incumbent" and not others:
        return [WITHDRAWN]
    return []


# -- §2.15, §5 what a tool concludes ---------------------------------------------------

YES, NO, UNOBSERVABLE, NOT_ASKED, NOT_THIS = "yes", "no", "unobservable", "not_asked", "not_this_profiles"


@dataclass(frozen=True)
class Answer:
    answer: str
    reason: str
    count: int | None = None
    exact: bool | None = None

    def as_json(self) -> dict[str, Any]:
        out: dict[str, Any] = {"answer": self.answer, "reason": self.reason}
        if self.count is not None:
            out["count"], out["exact"] = self.count, bool(self.exact)
        return out


@dataclass(frozen=True)
class Reading:
    """§2.15's input: the governance (``claim``, ``external``,
    ``not_governed``, ``no_contract``, ``disagree``), whether across a
    constrained face, and two reads a grace apart."""

    governance: str
    first: Read
    second: Read
    across_face: bool = False

    @classmethod
    def of(cls, j: dict[str, Any]) -> Reading:
        return cls(j["governance"], Read.of(j["first"]), Read.of(j["second"]), bool(j.get("across_face", False)))


def _premise(r: Reading, present: bool) -> tuple[str, str] | None:
    """§2.15 "The premises, for every question, in this order"."""
    if r.across_face:
        return UNOBSERVABLE, "across_face"
    if not present:
        return (NOT_ASKED, "absent") if r.second.complete else (UNOBSERVABLE, "presence_incomplete")
    if r.governance == "not_governed":
        return NOT_THIS, "not_governed"
    if r.governance == "no_contract":
        return UNOBSERVABLE, "no_contract"
    if r.governance == "disagree":
        return UNOBSERVABLE, "revisions_disagree"
    return None


def _no_candidate(read: Read) -> tuple[str, str]:
    if any(i.descriptor == UNREAD for i in read.instances):
        return UNOBSERVABLE, "no_descriptor"
    return NOT_ASKED, "no_candidate"


def one_incumbent(r: Reading) -> Answer:
    """§2.15 "Is the address served by exactly one incumbent?"."""
    p = _premise(r, bool(r.second.instances))
    if p:
        return Answer(*p)
    h1, h2 = r.first.holders(), r.second.holders()
    if len(h1) >= 2 and len(h2) >= 2:
        return Answer(NO, "split")
    if len(h2) >= 2:
        return Answer(UNOBSERVABLE, "in_transition")
    if len(h2) == 1:
        return Answer(YES, "one") if r.second.complete else Answer(UNOBSERVABLE, "presence_incomplete")
    if not r.second.complete:
        return Answer(UNOBSERVABLE, "presence_incomplete")
    if not any(i.candidate for i in r.second.instances):
        return Answer(*_no_candidate(r.second))
    if h1:
        return Answer(UNOBSERVABLE, "in_transition")
    if not r.first.complete:
        return Answer(UNOBSERVABLE, "presence_incomplete")
    return Answer(NO, "vacant")


def vacancy(r: Reading) -> Answer:
    """§2.15 "Is there a vacancy?"."""
    p = _premise(r, bool(r.second.instances))
    if p:
        return Answer(*p)
    if r.second.holders():
        return Answer(NO, "held")
    if not r.second.complete:
        return Answer(UNOBSERVABLE, "presence_incomplete")
    if not any(i.candidate for i in r.second.instances):
        return Answer(*_no_candidate(r.second))
    if not r.first.holders() and r.first.complete:
        return Answer(YES, "lasting")
    return Answer(YES, "vacant")


def standbys(r: Reading) -> Answer:
    """§2.15 "Does any instance stand by, and how many?": the count exact
    "when the read is complete and every descriptor in it was read, and
    otherwise a lower bound". On a premise's answer, count 0, not exact."""
    p = _premise(r, bool(r.second.instances))
    if p:
        return Answer(*p, count=0, exact=False)
    sb = [i for i in r.second.instances if i.descriptor == STANDBY and not i.token]
    unread = any(i.descriptor == UNREAD for i in r.second.instances)
    if sb:
        return Answer(YES, "standing_by", len(sb), r.second.complete and not unread)
    if not r.second.complete:
        return Answer(UNOBSERVABLE, "presence_incomplete", 0, False)
    if unread:
        return Answer(UNOBSERVABLE, "no_descriptor", 0, False)
    return Answer(NO, "none", 0, True)


def twin(r: Reading, x: str) -> Answer:
    """§2.15 "Is this instance a standby, or a twin?", of ``x``."""
    x2 = r.second.get(x)
    p = _premise(r, x2 is not None)
    if p:
        return Answer(*p)
    assert x2 is not None
    x1 = r.first.get(x)
    same = x1 is not None and x1.token == x2.token and x1.descriptor == x2.descriptor
    h1, h2 = r.first.holders(), r.second.holders()
    if x1 is not None and x1.token and x2.token and len(h1) >= 2 and len(h2) >= 2:
        return Answer(YES, "split_lowest" if lowest(h2) == x else "split")
    if x2.descriptor == UNREAD:
        return Answer(UNOBSERVABLE, "no_descriptor")
    if x2.descriptor == NOT_LISTED:
        if not x2.token:
            return Answer(NOT_ASKED, "not_a_candidate")
        return Answer(YES, "unlisted") if same else Answer(UNOBSERVABLE, "in_transition")
    if x2.token:
        if h2 - {x}:
            return Answer(UNOBSERVABLE, "in_transition")
        if x2.descriptor == STANDBY:
            return Answer(YES, "marked_holding") if same else Answer(UNOBSERVABLE, "in_transition")
        return Answer(NO, "incumbent") if r.second.complete else Answer(UNOBSERVABLE, "presence_incomplete")
    if x2.descriptor == STANDBY:
        return Answer(NO, "standby") if r.second.complete else Answer(UNOBSERVABLE, "presence_incomplete")
    if same and r.first.complete and r.second.complete:
        return Answer(YES, "unmarked")
    if same:
        return Answer(UNOBSERVABLE, "presence_incomplete")
    return Answer(UNOBSERVABLE, "in_transition")


def conclude(r: Reading) -> dict[str, Any]:
    """§5's four questions: of the address, and of each instance named in
    either read."""
    named = list(dict.fromkeys(r.first.ids() + r.second.ids()))
    return {"one_incumbent": one_incumbent(r), "vacancy": vacancy(r), "standbys": standbys(r),
            "twins": {x: twin(r, x) for x in named}}

"""``arbitration.v1``: one commander at a time, in the binding's order
(profile text 0.3, draft, written against core 0.56 and ``timing.v1`` text
0.3; ``spec/profiles/arbitration/v1.md``).

Read cold from the text, its fixtures and its scenarios alone (#609, round
77; text 0.2 in round 78, text 0.3 in round 79; text 0.4 in round 88, its
inline history taken out and fifteen long sentences split, no rule
changed, and none of this module's rules moved). The session-free half:
- :func:`declaration` reads one role against the revision of its required
  interface held (§2.2, §2.3), as ``conformance/declarations.json`` does:
  not arbitrated, arbitrated (its policy, its stream, its deadline and
  period), unread, or invalid with its reasons in their order; the
  deadline is :func:`zk2py.timing.declaration`'s, never re-read here;
  :func:`tool_declaration` is §2.14's over every revision a tool holds;
- :func:`ranking` is §2.4, as ``conformance/rankings.json`` does: the
  providers in rank order, the addresses no entry matches, the wildcard and
  the shadowed entries;
- :class:`Arbiter` is §2.5 to §2.11, one consumer's arbitrated role driven
  by the events its runtime hands it, as ``conformance/arbitrations.json``
  drives it: each sample's outcome, eligibility, withdrawals and
  re-admission, a move of the watch session held back until its round trip
  stands, selection, switches and their causes, the dead-man, and the
  counts;
- :func:`conclude` is §5's three questions over a tool's reading, as
  ``conformance/conclusions.json`` does, ``no_descriptor`` with no
  consumer's descriptor read (text 0.2, F-272); :func:`view_governed` and
  :func:`listened_ns` are what a tool's view reads that no descriptor
  carries, and when it has listened (§2.14, text 0.3);
- :func:`lint`, §2.2's linter SHOULD (checks 1 to 7), which has no core
  code.

Times are integers of nanoseconds on the consumer's monotonic clock; the
fixtures' milliseconds are read exactly (:func:`zk2py.timing.ms_to_ns`).

The session half (an actuator on a bus, its commanders, a tool's view) is
:mod:`zk2py.arbitration_live`.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any, Iterable

from . import keys, templates
from . import timing as tm
from .slug import slug

PROFILE = "arbitration.v1"
POLICY = "arbitration.policy"
#: §4: the one policy 1.0 defines, and the two it reserves.
PRIORITY = "priority"
RESERVED = ("freshest", "lease")
#: §2.2's eight reasons, in their order.
REASONS = ("not_used", "manifest_role", "policy_unsupported", "value", "cardinality", "resource", "no_deadline",
           "params_unbound")
NOT_ARBITRATED, ARBITRATED, UNREAD, INVALID = "not_arbitrated", "arbitrated", "unread", "invalid"
STREAM_TOKENS = ("stream", "@stream")
# §2.10's causes.
START, SAMPLE, DEADLINE, WITHDRAWN = "start", "sample", "deadline", "withdrawn"
# §2.5's outcomes.
WILD, UNRESOLVED, UNBOUND, DELETED, EXPIRED, NOT_INCUMBENT, DELIVERED, NOT_ACTIVE = (
    "wild", "unresolved", "unbound", "deleted", "expired", "not_incumbent", "delivered", "not_active")
#: the outcomes counted per provider (4 to 7)
PER_PROVIDER = (DELIVERED, NOT_ACTIVE, DELETED, EXPIRED, NOT_INCUMBENT)


# -- §2.2, §2.3 the declaration -----------------------------------------------------------

@dataclass(frozen=True)
class Declaration:
    declaration: str
    policy: str | None = None
    #: the role's stream, ``<token>/<template>``
    resource: str | None = None
    deadline_ms: int | None = None
    period_ms: int | None = None
    reasons: tuple[str, ...] = ()

    @property
    def valid(self) -> bool:
        return self.declaration == ARBITRATED

    def expect(self) -> dict[str, Any]:
        if self.declaration == ARBITRATED:
            return {"declaration": ARBITRATED, "policy": self.policy, "resource": self.resource,
                    "deadline_ms": self.deadline_ms, "period_ms": self.period_ms}
        if self.declaration == INVALID:
            return {"declaration": INVALID, "reasons": list(self.reasons)}
        return {"declaration": self.declaration}


def _arbitration_majors(uses: Iterable[str]) -> list[str]:
    return [u for u in uses if u.startswith("arbitration.v")]


def consumed(resources: list[dict[str, Any]], named: list[str] | None) -> list[dict[str, Any]]:
    """§2.2 "What a role consumes": every resource of the revision held whose
    template the requirement's ``resources`` names, or every resource when it
    names none (core §3.1)."""
    return [r for r in resources if named is None or r["template"] in named]


def declaration(role: dict[str, Any], provider: dict[str, Any] | None,
                params: dict[str, str] | None = None) -> Declaration:
    """§2.2 over one role (declarations.json's ``role``: ``declared_by``
    ``{uses}`` or None for a manifest's, ``resources`` or None, its
    ``cardinality`` and ``annotations``), the revision held (``provider``:
    its resources' tokens, templates and resolved annotations, or None) and
    the parameter bindings as configured."""
    ann = role.get("annotations") or {}
    if POLICY not in ann:
        return Declaration(NOT_ARBITRATED)
    by = role.get("declared_by")
    uses = list(by.get("uses", [])) if isinstance(by, dict) else None
    majors = _arbitration_majors(uses or [])
    if uses is not None and majors and PROFILE not in uses:
        return Declaration(NOT_ARBITRATED)  # "Another major … is that major's"
    policy = ann[POLICY]
    reasons: list[str] = []
    if uses is not None and not majors:
        reasons.append("not_used")
    if by is None:
        reasons.append("manifest_role")
    if isinstance(policy, str) and policy in RESERVED:
        reasons.append("policy_unsupported")
    elif not (isinstance(policy, str) and policy == PRIORITY):
        reasons.append("value")
    if role.get("cardinality", "one") != "many":
        reasons.append("cardinality")
    if provider is None:
        # "No revision held … reads checks 1 to 5 only"
        return Declaration(INVALID, reasons=tuple(reasons)) if reasons else Declaration(UNREAD)
    mine = consumed(provider.get("resources", []), role.get("resources"))
    stream = mine[0] if len(mine) == 1 and mine[0]["token"] in STREAM_TOKENS else None
    deadline = period = None
    if stream is None:
        reasons.append("resource")
    else:
        # "as timing.v1 reads the revision held": its declaration, the
        # deadline below the period none
        td = tm.declaration(stream["token"], stream.get("annotations") or {})
        deadline = td.judged_deadline_ms
        period = td.period_ms
        if deadline is None:
            reasons.append("no_deadline")
        tpl = templates.parse_template(stream["template"])
        if any(s.text not in (params or {}) for s in tpl.segments if s.kind != templates.LITERAL):
            reasons.append("params_unbound")
    if reasons:
        return Declaration(INVALID, reasons=tuple(reasons))
    assert stream is not None
    return Declaration(ARBITRATED, policy, f"{stream['token']}/{stream['template']}", deadline, period)


def declaration_of(case: dict[str, Any]) -> Declaration:
    return declaration(case["role"], case.get("provider"), case.get("params") or {})


def role_of(canonical: dict[str, Any], role: str) -> dict[str, Any]:
    """A contract's requirement as :func:`declaration` reads it."""
    req = canonical["requires"][role]
    return {"declared_by": {"uses": list(canonical.get("uses", []))}, "resources": req.get("resources"),
            "cardinality": req.get("cardinality", "one"), "annotations": dict(req.get("annotations") or {})}


def provider_of(canonical: dict[str, Any]) -> dict[str, Any]:
    """A revision of the required interface as :func:`declaration` reads it."""
    return {"interface": canonical["interface"],
            "resources": [{"token": r["token"], "template": r["template"],
                           "annotations": dict(r.get("annotations") or {})} for r in canonical["resources"]]}


#: §2.14's token for revisions that give two declarations
REVISIONS_DISAGREE = "revisions_disagree"
NO_CONTRACT = "no_contract"
NO_DESCRIPTOR = "no_descriptor"  # §2.14, text 0.2, F-272


def tool_declaration(role: dict[str, Any] | None, revisions: list[dict[str, Any]],
                     params: dict[str, str] | None = None) -> str | Declaration:
    """§2.14: the declaration a tool reads, over every revision of the
    required interface it holds: "the one they all give; two that differ
    are unobservable, revisions_disagree; none held leaves it unread". A
    role whose declaring contract is not held (``role`` None) is
    ``no_contract``."""
    if role is None:
        return NO_CONTRACT
    if not revisions:
        return declaration(role, None, params)
    got = [declaration(role, p, params) for p in revisions]
    first = got[0].expect()
    return got[0] if all(g.expect() == first for g in got) else REVISIONS_DISAGREE


# -- §2.4 the ranking -------------------------------------------------------------------------

def entry_valid(entry: str) -> bool:
    """§0: an entry is ``<system>/<service>``, each chunk a plain chunk or
    ``*`` (core §1.2, D009), a ``self.system`` already resolved (a plain
    chunk either way). Anything else matches no address, so a runtime
    refuses it rather than leave the address silently unbound (review-bot on
    PR #912)."""
    from .lexical import is_plain_chunk

    chunks = entry.split("/") if isinstance(entry, str) else []
    return len(chunks) == 2 and all(c == "*" or is_plain_chunk(c) for c in chunks)


def is_wild(entry: str) -> bool:
    return "*" in entry.split("/", 1)


def matches(entry: str, address: str) -> bool:
    """§0: an exact entry matches its own address, a wildcard every address
    whose chunks equal its chunks where they are not ``*``."""
    e, a = entry.split("/", 1), address.split("/", 1)
    return len(e) == len(a) == 2 and all(x == "*" or x == y for x, y in zip(e, a))


def covers(earlier: str, later: str) -> bool:
    """An earlier entry that matches every address a later one does: "each
    chunk of the earlier `*` or the same as the later's"."""
    e, l_ = earlier.split("/", 1), later.split("/", 1)
    return all(x == "*" or x == y for x, y in zip(e, l_))


def address_key(address: str) -> tuple[bytes, bytes]:
    """§2.4: "by system, then by service, each compared byte by byte"."""
    system, service = address.split("/", 1)
    return system.encode(), service.encode()


def rank_of(bindings: list[str], address: str) -> tuple[int, bytes, bytes] | None:
    """An address's rank: its first matching entry, then its system and its
    service; None when no entry matches (``unbound``)."""
    for i, e in enumerate(bindings):
        if matches(e, address):
            return (i, *address_key(address))
    return None


def wildcards(bindings: list[str]) -> list[int]:
    return [i for i, e in enumerate(bindings) if is_wild(e)]


def shadowed(bindings: list[str]) -> list[int]:
    return [j for j, e in enumerate(bindings) if any(covers(bindings[i], e) for i in range(j))]


def ranking(bindings: list[str], heard: Iterable[str]) -> dict[str, Any]:
    """rankings.json's ``expect``: every exact entry's address, heard or not,
    and every heard address some entry matches, once each, in rank order;
    the heard addresses no entry matches; the wildcard and shadowed
    entries."""
    heard = list(heard)
    addrs = {e for e in bindings if not is_wild(e)} | {a for a in heard if rank_of(bindings, a) is not None}
    ranked = sorted(addrs, key=lambda a: rank_of(bindings, a))  # type: ignore[arg-type, return-value]
    unbound = sorted({a for a in heard if rank_of(bindings, a) is None}, key=address_key)
    return {"ranked": ranked, "unbound": unbound, "wildcards": wildcards(bindings), "shadowed": shadowed(bindings)}


# -- §2.5 to §2.11 the arbiter -----------------------------------------------------------------

@dataclass(frozen=True)
class Switch:
    at_ns: int
    from_: str | None
    to: str | None
    cause: str
    #: the instant of ``from_``'s last arrival, as it was before the event
    last_ns: int | None = None

    def expect(self) -> dict[str, Any]:
        return {"from": self.from_, "to": self.to, "cause": self.cause}


@dataclass
class Decision:
    """What one event decided: its switches (at most a deadline one, then the
    event's own), the active provider after it and its due instant, a
    sample's outcome, and the addresses to read again."""

    switches: list[Switch]
    active: str | None
    due_ns: int | None
    outcome: str | None = None
    reread: list[str] = field(default_factory=list)
    #: the provider of a sample, when it has one
    provider: str | None = None

    def expect(self) -> dict[str, Any]:
        out: dict[str, Any] = {"switches": [s.expect() for s in self.switches], "active": self.active,
                               "due": None if self.due_ns is None else tm.ns_to_ms(self.due_ns)}
        if self.outcome is not None:
            out["outcome"] = self.outcome
        if self.reread:
            out["reread"] = list(self.reread)
        return out


@dataclass
class _Provider:
    #: its last arrival since its last re-admission (or the start), for eligibility
    last: int | None = None
    #: its last arrival, never forgotten: a switch's ``last_ms``
    seen: int | None = None
    withdrawn: bool = False
    #: the instances known holding the required interface's token here
    known: set[str] = field(default_factory=set)


class Arbiter:
    """One consumer's arbitrated role (§2.5 to §2.11), the events handed to
    it in order. ``interface`` is the required interface's id, ``token`` and
    ``template`` its stream's, ``params`` the role's parameter bindings
    resolved, ``deadline_ms`` the deadline of the revision held (§2.3), and
    ``bindings`` the entries in their configured order (§2.4)."""

    def __init__(self, interface: str, token: str, template: str, params: dict[str, str], deadline_ms: int,
                 bindings: list[str]) -> None:
        self.interface, self.token = interface, token
        self.template = templates.parse_template(template)
        self.params = dict(params)
        self.deadline_ns = deadline_ms * tm.MS
        bad = [e for e in bindings if not entry_valid(e)]
        if bad:
            raise ValueError(f"not a binding entry: {', '.join(map(repr, bad))} (§0, core D009)")
        self.bindings = list(bindings)
        self.providers: dict[str, _Provider] = {}
        self.active: str | None = None
        self.started = False
        #: a move of the watch session not yet followed by a standing round trip
        self.pending = False
        self.kept: list[tuple[str, Any]] = []
        self.counts: dict[str, Any] = {"providers": {}, WILD: 0, UNRESOLVED: 0, UNBOUND: 0}
        self.switches: list[Switch] = []

    # -- what it holds --------------------------------------------------------------------

    def _p(self, address: str) -> _Provider:
        return self.providers.setdefault(address, _Provider())

    def eligible(self, address: str, t: int) -> bool:
        """§2.6: not withdrawn, and its last arrival ``a`` with ``t ≤ a +
        deadline``."""
        p = self.providers.get(address)
        return p is not None and not p.withdrawn and p.last is not None and t <= p.last + self.deadline_ns

    def best(self, t: int) -> str | None:
        """§2.9: the eligible provider of highest rank, or None."""
        cands = [a for a in self.providers if self.eligible(a, t)]
        return min(cands, key=lambda a: rank_of(self.bindings, a)) if cands else None  # type: ignore[arg-type, return-value]

    def due(self) -> int | None:
        """The active provider's due instant, its last arrival plus the
        deadline: the runtime's timer fires after it (§2.9)."""
        if self.active is None:
            return None
        last = self.providers[self.active].last
        return None if last is None else last + self.deadline_ns

    def _rank_sorted(self, addrs: Iterable[str]) -> list[str]:
        return sorted(set(addrs), key=lambda a: rank_of(self.bindings, a) or (len(self.bindings), *address_key(a)))

    # -- deciding ---------------------------------------------------------------------------

    def _switch(self, t: int, to: str | None, cause: str, out: list[Switch],
                last_before: dict[str, int | None] | None = None) -> None:
        frm = self.active
        last = None
        if frm is not None:
            last = (last_before or {}).get(frm, self.providers[frm].seen)
        s = Switch(t, frm, to, cause, last)
        out.append(s)
        self.switches.append(s)
        self.active = to

    def _time_first(self, t: int, out: list[Switch]) -> None:
        """§2.9 "An event decides the time first": the active provider's miss
        before ``t``, cause deadline."""
        if self.active is not None and not self.eligible(self.active, t):
            self._switch(t, self.best(t), DEADLINE, out)

    def _select(self, t: int, out: list[Switch], arrival_of: str | None,
                last_before: dict[str, int | None] | None = None) -> None:
        new = self.best(t)
        if new != self.active:
            self._switch(t, new, SAMPLE if arrival_of is not None and new == arrival_of else WITHDRAWN, out,
                         last_before)

    def _decision(self, out: list[Switch], **kw: Any) -> Decision:
        return Decision(out, self.active, self.due(), **kw)

    # -- events -------------------------------------------------------------------------------

    def start(self, t: int) -> Decision:
        """§2.11: the dead-man at start, cause start."""
        self.started = True
        out: list[Switch] = []
        self._switch(t, None, START, out)
        return self._decision(out)

    def classify(self, key: str, delete: bool = False, expired: bool = False,
                 incumbent: bool = True) -> tuple[str, str | None]:
        """§2.5's first outcome that holds, and the sample's provider when it
        has one (outcomes 4 to 7; ``None`` for an arrival's own, decided by
        :meth:`sample`)."""
        if "*" in key:
            return WILD, None
        p = keys.parse(key)
        if p is None or p["form"] != "data" or p["iface"] != self.interface or p["token"] != self.token:
            return UNRESOLVED, None
        b = templates.match(self.template, list(p["resource"]))
        if b is None or any(len(v) != 1 or self.params.get(name) != v[0] for name, v in b.items()):
            return UNRESOLVED, None
        address = f"{p['system']}/{p['service']}"
        if rank_of(self.bindings, address) is None:
            return UNBOUND, None
        if delete:
            return DELETED, address
        if expired:
            return EXPIRED, address
        if not incumbent:
            return NOT_INCUMBENT, address
        return "arrival", address

    def _count(self, outcome: str, address: str | None) -> None:
        if address is None:
            self.counts[outcome] += 1
            return
        c = self.counts["providers"].setdefault(address, {k: 0 for k in PER_PROVIDER})
        c[outcome] += 1

    def sample(self, t: int, key: str, delete: bool = False, expired: bool = False,
               incumbent: bool = True) -> Decision:
        out: list[Switch] = []
        self._time_first(t, out)
        outcome, address = self.classify(key, delete, expired, incumbent)
        if outcome != "arrival":
            self._count(outcome, address)
            return self._decision(out, outcome=outcome, provider=address)
        assert address is not None
        p = self._p(address)
        if not p.withdrawn:
            p.last = t
        p.seen = t
        self._select(t, out, address)
        outcome = DELIVERED if self.active == address else NOT_ACTIVE
        self._count(outcome, address)
        return self._decision(out, outcome=outcome, provider=address)

    def _token(self, token: str) -> tuple[str, str] | None:
        """An interface token of the required interface at an address some
        entry matches: (address, instance); None for any other."""
        p = keys.parse(token)
        if p is None or p["form"] != "alive" or p["iface"] != self.interface:
            return None
        address = f"{p['system']}/{p['service']}"
        if rank_of(self.bindings, address) is None:
            return None
        return address, p["instance"]

    def _readmit(self, p: _Provider) -> None:
        # "Re-admission alone makes no provider eligible, since it forgot its
        # arrivals"
        p.withdrawn = False

    def _withdraw(self, address: str, p: _Provider) -> None:
        p.withdrawn = True
        p.last = None

    def declared(self, t: int, token: str) -> Decision:
        out: list[Switch] = []
        self._time_first(t, out)
        tok = self._token(token)
        if tok is not None:
            if self.pending:
                self.kept.append(("declared", tok))
            else:
                p = self._p(tok[0])
                p.known.add(tok[1])
                if p.withdrawn:
                    self._readmit(p)
        self._select(t, out, None)
        return self._decision(out)

    def withdrawn(self, t: int, token: str) -> Decision:
        out: list[Switch] = []
        self._time_first(t, out)
        tok = self._token(token)
        reread: list[str] = []
        last_before: dict[str, int | None] = {}
        if tok is not None:
            if self.pending:
                self.kept.append(("withdrawn", tok))
            else:
                p = self._p(tok[0])
                p.known.discard(tok[1])
                if not p.known:
                    last_before[tok[0]] = p.seen
                    self._withdraw(tok[0], p)
                    reread.append(tok[0])
        self._select(t, out, None, last_before)
        return self._decision(out, reread=reread)

    def read(self, t: int, address: str, complete: bool, held: Iterable[str]) -> Decision:
        """§2.7 "Read again": what a read shows held is known held from then
        on, a complete read replacing what was, one not complete adding; any
        then known held re-admits; a read never withdraws."""
        out: list[Switch] = []
        self._time_first(t, out)
        held = list(held)
        if rank_of(self.bindings, address) is not None:
            if self.pending:
                self.kept.append(("read", (address, complete, held)))
            else:
                p = self._p(address)
                p.known = set(held) if complete else p.known | set(held)
                if p.known and p.withdrawn:
                    self._readmit(p)
        self._select(t, out, None)
        return self._decision(out)

    def moved(self, t: int) -> Decision:
        """§2.8: the watch session's transport closed or opened."""
        out: list[Switch] = []
        self._time_first(t, out)
        self.pending = True
        self._select(t, out, None)
        return self._decision(out)

    def stood(self, t: int) -> Decision:
        """§2.8: the round trip asked after the last move stands: what was
        kept applied in order, as one change, and one selection."""
        out: list[Switch] = []
        self._time_first(t, out)
        reread: list[str] = []
        last_before: dict[str, int | None] = {}
        if self.pending:
            kept, self.kept, self.pending = self.kept, [], False
            work: dict[str, set[str]] = {}
            gone: set[str] = set()
            for what, x in kept:
                address = x[0]
                if address not in work:
                    work[address] = set(self._p(address).known)
                if what == "declared":
                    work[address].add(x[1])
                elif what == "withdrawn":
                    work[address].discard(x[1])
                    gone.add(address)
                else:
                    _, complete, held = x
                    work[address] = set(held) if complete else work[address] | set(held)
            for address, known in work.items():
                p = self._p(address)
                p.known = known
                if not known and address in gone:
                    last_before[address] = p.seen
                    self._withdraw(address, p)
                    reread.append(address)
                elif p.withdrawn and known:
                    self._readmit(p)
        self._select(t, out, None, last_before)
        return self._decision(out, reread=self._rank_sorted(reread))

    def tick(self, t: int) -> Decision:
        out: list[Switch] = []
        self._time_first(t, out)
        return self._decision(out)

    def counts_line(self) -> dict[str, Any]:
        """The counts as the scenarios' line prints them (text 0.2, F-271):
        "Every provider known has its five counts, deleted among them … in
        rank order": every exact entry's address, which ranks whether or not
        heard, and every address a wildcard matches that was heard from, a
        sample counted, a token or a read (§2.4)."""
        known = set(self.counts["providers"]) | set(self.providers)
        known |= {e for e in self.bindings if not is_wild(e)}
        ordered = sorted(known, key=lambda a: rank_of(self.bindings, a))  # type: ignore[arg-type, return-value]
        zero = {k: 0 for k in PER_PROVIDER}
        return {"providers": {a: dict(self.counts["providers"].get(a, zero)) for a in ordered},
                WILD: self.counts[WILD], UNRESOLVED: self.counts[UNRESOLVED], UNBOUND: self.counts[UNBOUND]}

    def event(self, e: dict[str, Any]) -> Decision:
        """One of arbitrations.json's events."""
        t = tm.ms_to_ns(e["at"])
        what = e["event"]
        if what == "start":
            return self.start(t)
        if what == "sample":
            return self.sample(t, e["key"], bool(e.get("delete", False)), bool(e.get("expired", False)),
                               bool(e.get("incumbent", True)))
        if what == "declared":
            return self.declared(t, e["token"])
        if what == "withdrawn":
            return self.withdrawn(t, e["token"])
        if what == "read":
            return self.read(t, e["address"], bool(e["complete"]), e["held"])
        if what == "moved":
            return self.moved(t)
        if what == "stood":
            return self.stood(t)
        if what == "tick":
            return self.tick(t)
        raise ValueError(f"{what!r}: not an event")


def arbiter_of(role: dict[str, Any]) -> Arbiter:
    """arbitrations.json's ``role``."""
    r = role["resource"]
    return Arbiter(role["interface"], r["token"], r["template"], role.get("params") or {}, role["deadline_ms"],
                   role["bindings"])


def resolve_params(params: dict[str, str], address: str) -> dict[str, str]:
    """Core R2: ``self.system`` and ``self.service`` resolved to the
    consumer's own address (§2.2 check 8, §2.14)."""
    system, service = address.split("/", 1)
    return {k: system if v == "self.system" else service if v == "self.service" else v for k, v in params.items()}


def stream_key(address: str, interface: str, token: str, template: str, params: dict[str, str]) -> str:
    """The role's one key at a provider's address (§2.2 check 8)."""
    tpl = templates.parse_template(template)
    chunks = [s.text if s.kind == templates.LITERAL else slug(params[s.text]) for s in tpl.segments]
    return f"zk2/{address}/{interface}/{token}/{'/'.join(chunks)}"


# -- §5 what a tool may conclude ----------------------------------------------------------------

@dataclass(frozen=True)
class Answer:
    answer: str
    reason: str
    extra: tuple[tuple[str, Any], ...] = ()

    def expect(self) -> dict[str, Any]:
        return {"answer": self.answer, "reason": self.reason, **dict(self.extra)}


@dataclass(frozen=True)
class View:
    """A tool's own view (§2.14): how long it listened, whether it knows it
    lost samples within the last deadline, and what it selects."""

    listened_ns: int
    lost: bool
    active: str | None


def view_governed(revisions: Iterable[dict[str, Any]]) -> bool:
    """§2.14 (text 0.3): a tool's view "reads §2.5's outcome 6 with the
    required interface governed when every revision of it the tool holds
    lists ``redundancy.v1``". A descriptor names no revision of the required
    interface, so the tool cannot pick the consumer's; with none held, there
    is no view (the declaration is unread)."""
    revs = list(revisions)
    return bool(revs) and all("redundancy.v1" in (r.get("uses") or []) for r in revs)


def listened_ns(subscribed_ns: Iterable[int], now_ns: int) -> int:
    """§2.14 "Listening" (text 0.3): "It has listened from the instant its
    subscriptions to the role's stream are declared, the last of them":
    a watch's round trip before them is no listening. Before the last is
    declared, nothing."""
    subs = list(subscribed_ns)
    return 0 if not subs else max(0, now_ns - max(subs))


def conclude(declared: str | Declaration, bindings: list[str] | None, view: View | None) -> dict[str, Answer]:
    """§5's three questions over a tool's reading (conclusions.json):
    ``declared`` is a :class:`Declaration` or ``no_contract`` /
    ``revisions_disagree``; ``bindings`` is None when the tool read no
    consumer's descriptor (text 0.2, F-272): after the premises, the
    selection and the binding are not asked, ``no_descriptor``."""
    kind = declared if isinstance(declared, str) else declared.declaration
    deadline_ms = None if isinstance(declared, str) else declared.deadline_ms
    # the declaration
    premise = {NO_CONTRACT: Answer("unobservable", NO_CONTRACT),
               NOT_ARBITRATED: Answer("not_this_profiles", NOT_ARBITRATED),
               REVISIONS_DISAGREE: Answer("unobservable", REVISIONS_DISAGREE),
               UNREAD: Answer("unobservable", "no_provider_contract")}
    if kind in premise:
        decl = premise[kind]
    elif kind == INVALID:
        assert isinstance(declared, Declaration)
        decl = Answer("no", INVALID, (("reasons", list(declared.reasons)),))
    else:
        decl = Answer("yes", "valid")
    # the selection: the premises, then no descriptor, then the rest
    if kind in (NO_CONTRACT, NOT_ARBITRATED):
        sel = premise[kind]
    elif bindings is None:
        sel = Answer("not_asked", NO_DESCRIPTOR)
    elif kind in premise:
        sel = premise[kind]
    elif kind == INVALID:
        sel = Answer("unobservable", INVALID)
    elif not bindings:
        sel = Answer("no", "unbound")
    elif view is None:
        sel = Answer("not_asked", "not_listened")
    elif view.lost:
        sel = Answer("unobservable", "lost")
    elif view.active is not None:
        sel = Answer("yes", "selected", (("provider", view.active),))
    elif deadline_ms is not None and view.listened_ns < deadline_ms * tm.MS:
        sel = Answer("unobservable", "listening")
    else:
        sel = Answer("no", "dead_man")
    # the binding
    if kind in (NO_CONTRACT, NOT_ARBITRATED):
        binding = Answer(premise[kind].answer, premise[kind].reason, (("wildcards", []), ("shadowed", [])))
    elif bindings is None:
        binding = Answer("not_asked", NO_DESCRIPTOR, (("wildcards", []), ("shadowed", [])))
    else:
        w, s = wildcards(bindings), shadowed(bindings)
        reason, ans = ("wildcard", "yes") if w else ("shadowed", "yes") if s else ("exact", "no")
        binding = Answer(ans, reason, (("wildcards", w), ("shadowed", s)))
    return {"declaration": decl, "selection": sel, "binding": binding}


def reading_of(r: dict[str, Any]) -> tuple[str | Declaration, list[str] | None, View | None]:
    """conclusions.json's ``reading``."""
    d = r["declaration"]
    kind = d["declaration"]
    if kind in (NO_CONTRACT, REVISIONS_DISAGREE):
        declared: str | Declaration = kind
    elif kind == ARBITRATED:
        declared = Declaration(ARBITRATED, d.get("policy"), d.get("resource"), d.get("deadline_ms"),
                               d.get("period_ms"))
    elif kind == INVALID:
        declared = Declaration(INVALID, reasons=tuple(d.get("reasons", [])))
    else:
        declared = Declaration(kind)
    v = r.get("view")
    view = None if v is None else View(tm.ms_to_ns(v["listened_ms"]), bool(v["lost"]), v.get("active"))
    return declared, None if r["bindings"] is None else list(r["bindings"]), view


# -- the contract linter (§2.2: SHOULD warn of checks 1 to 7) ------------------------------------

def lint(canonical: dict[str, Any], held: dict[str, dict[str, Any]] | None = None) -> list[tuple[str, str]]:
    """(role, reason) for each of §2.2's checks 1 to 7 a role fails: 6 and 7
    only when ``held`` gives the required interface's canonical form, by its
    id. ``params_unbound`` needs the deployment's bindings, never read here."""
    out = []
    for role, req in canonical.get("requires", {}).items():
        r = role_of(canonical, role)
        prov = None if held is None or req["interface"] not in held else provider_of(held[req["interface"]])
        d = declaration(r, prov, {"*": "*"})
        if d.declaration == INVALID:
            out += [(role, x) for x in d.reasons if x != "params_unbound"]
    return out


__all__ = ["PROFILE", "POLICY", "Declaration", "declaration", "declaration_of", "tool_declaration", "role_of",
           "provider_of", "ranking", "rank_of", "matches", "entry_valid", "wildcards", "shadowed", "Arbiter", "arbiter_of",
           "Decision", "Switch", "conclude", "reading_of", "View", "Answer", "lint", "resolve_params", "stream_key"]

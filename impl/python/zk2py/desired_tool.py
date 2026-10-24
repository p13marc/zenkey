"""``desired.v1``'s tool on a bus (#609, round 57): §2.17's reading of a
target's roles, and §5's four questions over it (:func:`zk2py.desired.conclude`).

Written from ``spec/profiles/desired/v1.md`` (text 0.4), "What a tool reads
to answer", on core 0.39's presence rules: every presence read on a
session that has never held a liveliness subscriber, a read not made read
as one not complete, and an author absent only by a complete read.

Per role, in §2.17's order:
1. the target's presence, and its descriptor;
2. the role in the descriptor's ``requires``, and its binding (§2.3);
3. the slice: from the author's contract, the revision its descriptor
   names (held offline, else retrieved, core §8.4), else, with no
   descriptor naming one, a revision of its interface held offline (the
   reference's reading, F-213: one major's revisions declare the same
   keys); or, with no contract of the author's in hand, from the target
   alone, its requirement's template, its binding's parameters and the
   kind token its report's key carries (0.4) — ``slice_from`` says which;
4. its report, found by type identity among the target's report
   resources (§2.10), picked as 0.4 says;
5. the author's presence and its current version by S4's GET, the latest
   by stamp; an unstamped reply reads ``incomplete`` (0.4), a GET drawing
   nothing ``silent``; an absent author's archived last-known read beside,
   never current.

:func:`watch` reads again every 500 ms over a window, and finds a target
behind (0.4) only when every reading of it is not yet on one version of
its author's for the whole window, or, on a clock trusted to the HLC
delta, when that version is older than the window: never from one reading.
"""

from __future__ import annotations

import json
import time
from dataclasses import dataclass, field
from typing import Any, Iterable

import zenoh

from . import alarms, desired as ds, live, templates

#: freshness.v1 §2.6's HLC delta, which a trusted clock is within.
HLC_DELTA_S = 0.5


@dataclass
class Row:
    """One role of one target, read once."""

    target: str
    role: str
    listing: str  # bound, unbound, not_listed, unread
    reading: ds.Reading
    answers: dict[str, ds.Answer]
    slice: str | None = None
    slice_from: str | None = None
    problems: list[str] = field(default_factory=list)
    author: str | None = None
    #: the author's current version's stamp, text (None when there is none)
    author_stamp: str | None = None
    #: an absent author's slice from an archive: (archive address, stamp,
    #: document), never current
    archived: list[tuple[str, str | None, Any]] = field(default_factory=list)
    #: each presence read's own kind and reason, beside the answers (core
    #: §8.1, 0.38: a read not made keeps its reason)
    reads: list[tuple[str, str, str]] = field(default_factory=list)

    def pairs(self) -> dict[str, tuple[str, str]]:
        return {q: (a.answer, a.reason) for q, a in self.answers.items()}


def _first(x: Any) -> Any:
    """The first element of a list from the bus, or None for anything else:
    a malformed shape reads as nothing, never a crash."""
    return x[0] if isinstance(x, list) and x else None


def _id_of(instances: list[dict[str, Any]]) -> str | None:
    first = _first(instances)
    inst = first.get("instance") if isinstance(first, dict) else None
    return inst if isinstance(inst, str) else None


def _presence(session: zenoh.Session, address: str, timeout: float, reads: list) -> tuple[str, list[dict]]:
    p = live.list_presence(session, f"zk2/{address}/@zk/instance/*", timeout)
    reads.append((address, p.kind, p.not_made or (live.CUT_OFF_DURING if p.cut_off_during else "")))
    if p.instances:
        return ds.PRESENT, p.instances  # a read cannot invent a token
    if p.kind == live.COMPLETE:
        return ds.ABSENT, []
    # possibly incomplete, refused, or not made: the profile's premise
    return ds.INCOMPLETE, []


def _descriptor(session: zenoh.Session, address: str, instance: str, timeout: float) -> dict[str, Any] | None:
    got = [a for a in live.get_descriptor(session, f"zk2/{address}/@zk/instance/{instance}", timeout) if a.ok]
    if len(got) != 1:
        return None
    try:
        doc = json.loads(got[0].payload)
    except ValueError:
        return None
    return doc if isinstance(doc, dict) else None


def _contract(session: zenoh.Session, iface: str, fingerprint: str | None,
              offline: dict[str, dict[str, Any]], timeout: float) -> tuple[dict[str, Any] | None, str | None]:
    """A contract's canonical form, by its fingerprint: held offline, else
    retrieved (core §8.4). Returns (canonical, "offline" | "retrieved")."""
    if fingerprint is None:
        return None, None
    if fingerprint in offline:
        return offline[fingerprint], "offline"
    r = live.retrieve_bundle(session, iface, fingerprint, timeout)
    return (r.verified.contract, "retrieved") if r.verified else (None, None)


def _entry(doc: dict[str, Any] | None, iface: str) -> dict[str, Any] | None:
    for e in (doc or {}).get("interfaces") or []:
        if isinstance(e, dict) and e.get("iface") == iface:
            return e
    return None


def _report_resources(canonical: dict[str, Any]) -> list[dict[str, Any]]:
    """§2.10: "the state resources of its contracts typed desired.v1.Report
    from an artifact whose id §3 publishes"."""
    return [r for r in canonical.get("resources") or []
            if r.get("token") in ("state", "@state") and ds.is_profile_type(r.get("type"))]


def _template_selector(template: str) -> str:
    t = templates.parse_template(template)
    return "/".join(s.text if s.kind == templates.LITERAL else ("**" if s.kind == templates.REST else "*")
                    for s in t.segments)


def pick_report(replies: Iterable[tuple[str, bytes]], key: str) -> ds.ReportReading | None:
    """§2.10 (0.4, "Which reply is the report"), among the replies of every
    report resource of the target: the readable one whose ``key`` is the
    slice's; two readable ones that claim it and differ, unreadable; with
    none readable claiming it, one that does not read, unreadable too;
    otherwise none (None)."""
    claiming: list[ds.ReportReading] = []
    unreadable = False
    for _, payload in replies:
        r = ds.read_report_bytes(payload)
        if not r.readable:
            unreadable = True
            continue
        if (r.report or {}).get("key") == key:
            if all(x.report != r.report for x in claiming):
                claiming.append(r)
    if len(claiming) == 1:
        return claiming[0]
    if len(claiming) > 1:
        return ds.ReportReading(False, ("schema",))  # the target says two things of one role
    if unreadable:
        return ds.ReportReading(False, ("json",))
    return None


def _slice_from_requirement(target: str, requirement: dict[str, Any], template: str, author: str,
                            iface: str, explicit: bool) -> ds.Binding:
    """0.4: "the template its requirement names (core §3.1), each parameter
    filled as its binding says, and the kind token the report's key
    carries". Each template parameter is a target parameter bound to its
    ``self``; §2.3's checks are made over them."""
    t = templates.parse_template(template)
    params = dict(requirement.get("params") or {})
    names = [p for p in t.param_names]
    targets = tuple(ds.Target(p, params[p] if params.get(p) in (ds.SELF_SYSTEM, ds.SELF_SERVICE) else ds.EITHER)
                    for p in names)
    decl = ds.Declaration(ds.DECLARED, (), targets)
    return ds.bind(decl, template, explicit, iface, target, {**requirement, "bindings": [author]})


def read(session: zenoh.Session, target: str, role: str | None = None, *, contracts: Iterable[Any] = (),
         timeout: float = 1.0, clocks_synced: bool = False, archives: Iterable[str] = ()) -> list[Row]:
    """One reading of each role of ``target`` on a desired resource (or of
    ``role``): §2.17's premises and §5's answers."""
    contracts = list(contracts)
    offline = {c.fingerprint: c.canonical for c in contracts}
    # by interface, the latest minor held: with no descriptor naming the
    # author's revision, one of its interface held offline gives the slice,
    # every revision of one major declaring the same keys (F-213)
    by_iface: dict[str, dict[str, Any]] = {}
    for c in sorted(contracts, key=lambda c: c.minor or 0):
        by_iface[c.interface] = c.canonical
    reads: list[tuple[str, str, str]] = []
    t_presence, instances = _presence(session, target, timeout, reads)
    if t_presence != ds.PRESENT:
        r = ds.Reading(t_presence, None, None, "not_asked", ds.INCOMPLETE, None)
        return [Row(target, role or "-", "unread", r, ds.conclude(r), reads=reads)]
    inst = _id_of(instances)
    doc = _descriptor(session, target, inst, timeout) if inst is not None else None
    if doc is None:
        r = ds.Reading(t_presence, None, None, "not_asked", ds.INCOMPLETE, None)
        return [Row(target, role or "-", "unread", r, ds.conclude(r), reads=reads)]
    rows = []
    if "requires" in doc and not isinstance(doc["requires"], list):
        # a descriptor of a shape no rule reads (core §3.3 D000): unobservable
        r = ds.Reading(t_presence, None, None, "not_asked", ds.INCOMPLETE, None)
        return [Row(target, role or "-", "unread", r, ds.conclude(r), problems=["malformed: requires"], reads=reads)]
    requires = [x for x in doc.get("requires") or [] if isinstance(x, dict)]
    asked = [x for x in requires if role is None or x.get("role") == role]
    if role is not None and not asked:
        r = ds.Reading(t_presence, (False, False), None, "not_asked", ds.INCOMPLETE, None)
        return [Row(target, role, "not_listed", r, ds.conclude(r), reads=reads)]
    for req in asked:
        try:
            rows.append(_read_role(session, target, doc, req, offline, by_iface, timeout, clocks_synced, archives,
                                   list(reads)))
        except (KeyError, IndexError, TypeError, AttributeError, ValueError) as e:
            # a descriptor or a contract of a shape no rule reads: unobservable
            r = ds.Reading(t_presence, None, None, "not_asked", ds.INCOMPLETE, None)
            rows.append(Row(target, str(req.get("role")), "unread", r, ds.conclude(r),
                            problems=[f"malformed: {type(e).__name__}: {e}"], reads=list(reads)))
    return [r for r in rows if r is not None]


def _read_role(session: zenoh.Session, target: str, doc: dict[str, Any], req: dict[str, Any],
               offline: dict[str, dict[str, Any]], by_iface: dict[str, dict[str, Any]], timeout: float,
               clocks_synced: bool, archives: Iterable[str], reads: list) -> Row | None:
    name, iface = str(req.get("role")), str(req.get("interface"))
    bound = bool(req.get("bindings"))
    own = _entry(doc, str(req.get("declared_by")))
    t_canon, _ = _contract(session, str(req.get("declared_by")), None if own is None else own.get("contract"),
                           offline, timeout)
    problems: list[str] = []
    requires = t_canon.get("requires") if isinstance(t_canon, dict) else None
    rr = requires.get(name) if isinstance(requires, dict) else None
    template = _first(rr.get("resources")) if isinstance(rr, dict) else None
    template = template if isinstance(template, str) else None

    def row(r: ds.Reading, **kw: Any) -> Row:
        return Row(target, name, "bound" if bound else "unbound", r, ds.conclude(r), problems=problems,
                   reads=reads, **kw)

    if not bound:
        if not req.get("optional"):
            problems.append("required_unbound")
        r = ds.Reading(ds.PRESENT, (True, False), None, "not_asked", ds.INCOMPLETE, None)
        return row(r)
    author = _first(req.get("bindings"))
    if not isinstance(author, str) or len(author.split("/")) != 2:
        problems.append("not_one_author")
        r = ds.Reading(ds.PRESENT, (True, True), None, "not_asked", ds.INCOMPLETE, None)
        return row(r)
    # the author's contract, by its descriptor's revision
    a_presence, a_instances = _presence(session, author, timeout, reads)
    a_inst = _id_of(a_instances)
    a_doc = _descriptor(session, author, a_inst, timeout) if a_inst is not None else None
    a_entry = _entry(a_doc, iface)
    a_canon, how = _contract(session, iface, None if a_entry is None else a_entry.get("contract"), offline, timeout)
    if a_canon is None and a_entry is None:
        held = by_iface.get(iface)
        if held is not None:
            a_canon, how = held, "offline"
    binding, slice_from = None, None
    if a_canon is not None and template is not None:
        res = next((x for x in a_canon.get("resources") or [] if x.get("template") == template
                    and x.get("token") in ("state", "@state")), None)
        if res is not None:
            decl = ds.declare(res["token"], res["template"], res.get("annotations") or {},
                              dict(res.get("params") or {}))
            if decl.declaration == ds.DECLARED:
                binding = ds.bind(decl, template, res["token"] == "@state", iface, target, req)
                slice_from = "offline_contract" if how == "offline" else "author_contract"
            else:
                problems.append("declaration_invalid:" + ",".join(decl.reasons))
    # the target's reports, by type identity, among "every report resource
    # of the target" (§2.10, 0.4): of every contract its descriptor lists
    replies: list[tuple[str, bytes]] = []
    horizons: dict[str, tuple[float, str | None]] = {}
    for e in doc.get("interfaces") or []:
        if not isinstance(e, dict):
            continue
        canon = t_canon if e is own else _contract(session, str(e.get("iface")), e.get("contract"), offline,
                                                   timeout)[0]
        for res in _report_resources(canon or {}):
            sel = f"zk2/{target}/{e.get('iface')}/{res['token']}/{_template_selector(res['template'])}"
            st = live.get_state(session, sel, timeout)
            for x in st.replies:
                if not x.deleted:
                    replies.append((x.key, x.payload))
                    ttl = (res.get("annotations") or {}).get("freshness.ttl_s")
                    if isinstance(ttl, (int, float)) and ttl > 0:
                        horizons[x.key] = (float(ttl), x.stamp)
    if binding is None and template is not None and not problems:
        # 0.4: the slice from the target alone, the kind token its report's
        # key carries
        tokens = []
        for _, payload in replies:
            k = ((ds.read_report_bytes(payload).report or {}).get("key") or "")
            tokens += [tok for tok in ("state", "@state") if f"/{iface}/{tok}/" in k]
        explicit = "@state" in tokens and "state" not in tokens
        binding = _slice_from_requirement(target, req, template, author, iface, explicit)
        slice_from = "requirement"
    if binding is None or binding.binding != ds.BOUND:
        if binding is not None:
            problems += list(binding.reasons)
        r = ds.Reading(ds.PRESENT, (True, True), None, "not_asked", a_presence, None)
        return row(r, author=author, slice_from=slice_from)
    key = binding.key
    assert key is not None
    report = pick_report(replies, key)
    freshness = _freshness(replies, horizons, key, clocks_synced)
    current: Any = None
    stamp_text = None
    archived = []
    if a_presence == ds.PRESENT:
        st = live.get_state(session, key, timeout)
        mine = [x for x in st.replies if x.key == key]
        if st.errors:
            current = "incomplete"  # ended at its timeout, or an error reply
        elif any(x.stamp is None for x in mine):
            current = "incomplete"  # 0.4: an unstamped reply carries no version (core S1)
        elif not mine:
            current = "silent"
        else:
            latest = max(mine, key=lambda x: (ds.Stamp.parse(x.stamp).time, ds.Stamp.parse(x.stamp).id))
            current, stamp_text = ("delete" if latest.deleted else "put", ds.Stamp.parse(latest.stamp)), latest.stamp
    elif a_presence == ds.ABSENT:
        if not archives:
            # core §4.4: "An archive MUST hold its archive.v1 interface token"
            found = live.list_presence(session, "zk2/*/*/@zk/alive/archive.v1/**", timeout)
            archives = sorted({f"{x['system']}/{x['service']}" for x in found.alive})
        for arch in archives:
            st = live.get_state(session, alarms.archive_form(arch, key), timeout)
            for x in st.replies:
                archived.append((arch, x.stamp, None if x.deleted else x.payload))
    r = ds.Reading(ds.PRESENT, (True, True), report, freshness, a_presence, current)
    return row(r, slice=key, slice_from=slice_from, author=author, author_stamp=stamp_text, archived=archived)


def _freshness(replies: list[tuple[str, bytes]], horizons: dict[str, tuple[float, str | None]], key: str,
               clocks_synced: bool) -> str:
    """§2.17 step 4: the report's age, by ``freshness.v1`` where its
    resource declares a horizon (``not_asked`` otherwise). Its reply's stamp
    is aged only against a clock trusted to the HLC delta (freshness.v1
    §2.6): without one the age is unobservable, never fresh. ``key`` is the
    report key's own reply (the report resource's), found by the report it
    carries."""
    for rkey, payload in replies:
        if (ds.read_report_bytes(payload).report or {}).get("key") != key or rkey not in horizons:
            continue
        ttl, stamp = horizons[rkey]
        st = ds.Stamp.parse(stamp) if stamp else None
        if not clocks_synced or st is None:
            return "unobservable"
        age = time.time() - ((st.time >> 32) + (st.time & 0xFFFFFFFF) / 2 ** 32)
        return "stale" if age > ttl + HLC_DELTA_S else "fresh"
    return "not_asked"


@dataclass
class Window:
    """A reading made again over a window (0.4, "How long behind")."""

    rows: list[Row]
    readings: int
    #: per role, the author's version a target stayed not yet on for the
    #: whole window, and how ("window" or "clock")
    behind: dict[str, tuple[str, str]]


def watch(session: zenoh.Session, target: str, for_s: float, role: str | None = None, *,
          clocks_synced: bool = False, every_s: float = 0.5, reader: Any = None, **kw: Any) -> Window:
    """§2.17 (0.4): "never from one reading alone: the author's same version
    current, and the target not yet on it, in readings that long apart; or,
    with a clock trusted to the HLC delta, that version stamped more than
    that long and the delta before the reading". The answers stay not yet;
    ``behind`` is the tool's own finding beside them.

    - **By the window** (F-211, answered on #840): every reading over the
      window not yet on one version of the author's; any other answer, or
      another version, starts it again.
    - **By the clock**, with ``clocks_synced``: one reading may find it, the
      author's version stamped longer ago than the window and the delta.

    The finding follows the latest reading: a later reading that is not
    not yet on that version clears it, so ``behind`` holds only what the
    window's end still shows. ``reader`` stands in for :func:`read` (a
    reading's own test)."""
    reader = reader or read
    #: the window counts from the first reading's answer, so that a run of
    #: not-yet readings from it can last the whole window
    start: float | None = None
    #: per role, the version a run of not-yet readings is on, when it began,
    #: and how many readings it holds
    first_seen: dict[str, tuple[str, float, int]] = {}
    behind: dict[str, tuple[str, str]] = {}
    n = 0
    rows: list[Row] = []
    while True:
        rows = reader(session, target, role, clocks_synced=clocks_synced, **kw)
        n += 1
        now = time.monotonic()
        if start is None:
            start = now
        for r in rows:
            a = r.answers.get(ds.Q_CONVERGED)
            if a is None or a.answer != ds.NOT_YET or r.author_stamp is None:
                # any other answer starts the window again, and clears it
                first_seen.pop(r.role, None)
                behind.pop(r.role, None)
                continue
            seen = first_seen.get(r.role)
            if seen is None or seen[0] != r.author_stamp:
                first_seen[r.role] = (r.author_stamp, now, 1)
                behind.pop(r.role, None)  # another version: started again
            else:
                first_seen[r.role] = (seen[0], seen[1], seen[2] + 1)
            seen = first_seen[r.role]
            if clocks_synced and r.role not in behind:
                st = ds.Stamp.parse(r.author_stamp)
                age = time.time() - (st.time >> 32) - (st.time & 0xFFFFFFFF) / 2 ** 32 if st else 0
                if age > for_s + HLC_DELTA_S:
                    behind[r.role] = (r.author_stamp, "clock")
            # "never from one reading alone": two readings at least
            if seen[2] >= 2 and now - seen[1] >= for_s and r.role not in behind:
                behind[r.role] = (r.author_stamp, "window")
        if now - start >= for_s:
            break
        time.sleep(every_s)
    return Window(rows, n, behind)

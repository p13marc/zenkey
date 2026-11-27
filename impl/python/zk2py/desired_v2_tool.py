"""``desired.v2``'s tool on a bus (#609, round 121): §2.17's reading of a
target's roles, and §5's four questions over it
(:func:`zk2py.desired_v2.conclude`).

Written from ``spec/profiles/desired/v2.md`` (text 0.1) §2.17, §2.18 and §5,
on zk2py's ``desired.v1`` tool (:mod:`zk2py.desired_tool`) for every step v2
keeps. Per role:
1. the target's presence and its descriptor; the role in its ``requires``,
   and its binding (v1 §2.3);
2. the major (§2.18, "A tool reads both, per role: the report by its type
   identity, and the author's version by its revision's major"): the
   author's revision, named by its descriptor or held offline. With no
   revision of the author's in hand the text names no way to tell; zk2py
   reads the role as this major's when its target's contract has a report
   resource whose ``desired.report`` names it. A ``desired.v1`` role is read
   by :mod:`zk2py.desired_tool`;
3. its report, "found as §2.10 says. No slice is needed to find it"
   (:func:`zk2py.desired_v2.find_report`, on S4's GET of the report key);
4. the slice from the author's contract, the revision its descriptor names
   or one held offline. "The slice from the target alone goes": with no
   revision in hand, no slice, no GET of the author's;
5. the author's presence, and its version by a member read of the slice:
   a live entry's or a tombstone's in-band stamp, a snapshot with no entry
   (its run and asof), no reply (``silent``), or a read that ended in an
   error (``incomplete``); an absent author's archived last-known read
   beside, never current.

``from_requirement`` is v1's way, which §2.17 removes: with its author
absent, the slice taken from the target's requirement and binding, so that
``scenarios-v2.md`` §9's "As v1's" can be read either way (round 121).

:func:`watch` reads again every 500 ms over a window, and finds a target
behind only when every reading of it is not yet on one version of its
author's for the whole window: "How long behind is decided by v1's first way
alone ... A version has no time, so v1's second way goes."
"""

from __future__ import annotations

import time
from dataclasses import dataclass, field
from typing import Any, Iterable

import zenoh

from . import alarms, desired as ds, desired_tool as dt, desired_v2 as d2, live


@dataclass
class Row:
    """One role of one target, read once."""

    target: str
    role: str
    profile: str  # desired.v1 or desired.v2: the major it was read by
    listing: str  # bound, unbound, not_listed, unread
    answers: dict[str, ds.Answer]
    reading: Any = None
    slice: str | None = None
    slice_from: str | None = None
    report_key: str | None = None
    report: d2.ReportReading | None = None
    found: str | None = None
    author: str | None = None
    author_presence: str | None = None
    #: entry, tombstone, no_entry, silent, incomplete, not_read
    author_current: str | None = None
    #: the author's version, <run>/<seq>, or a snapshot with no entry's run
    #: and asof, <run>/<asof>
    author_version: str | None = None
    archived: list[tuple[str, str | None, bytes | None]] = field(default_factory=list)
    #: a refusal's place and line as the target's contract's words, where
    #: its tables name the indices (§2.10, §5: "read with the target's
    #: contract")
    place: str | None = None
    line: str | None = None
    problems: list[str] = field(default_factory=list)
    v1: Any = None

    def pairs(self) -> dict[str, tuple[str, str]]:
        return {q: (a.answer, a.reason) for q, a in self.answers.items()}


def _author_major(session: zenoh.Session, iface: str, a_entry: Any, offline: dict[str, dict[str, Any]],
                  by_iface: dict[str, list[dict[str, Any]]], timeout: float) -> tuple[dict[str, Any] | None, str]:
    """The author's revision: the one its descriptor names, held offline or
    retrieved; else the latest held offline of its interface. Returns
    (canonical, how): ``author_contract``, ``offline_contract`` or None."""
    canon, _ = dt._contract(session, iface, None if a_entry is None else a_entry.get("contract"), offline, timeout)
    if isinstance(canon, dict):
        return canon, ds.AUTHOR_CONTRACT
    held = by_iface.get(iface) or []
    return (held[0], ds.OFFLINE_CONTRACT) if held else (None, "")


def _report_resources(canonical: dict[str, Any]) -> list[d2.ReportResource]:
    return [d2.ReportResource(r["token"], r["template"], r.get("annotations") or {},
                              (r.get("type") or {}).get("kind") == "packed"
                              and (r.get("type") or {}).get("name") == d2.REPORT_TYPE
                              and d2.is_published((r.get("type") or {}).get("schema")))
            for r in canonical.get("resources") or [] if isinstance(r, dict)]


def _version_of(ib: Any) -> d2.Version | None:
    return None if ib is None else d2.Version.of(ib.width, ib.run, ib.seq)


def author_read(session: zenoh.Session, key: str, timeout: float) -> tuple[Any, str, str | None]:
    """§2.17 step 6, "the author's version, is its snapshot entry's: a live
    entry's, or a tombstone's", by a member read of ``key`` (core §4.2).
    Returns (``author_current`` for :class:`zk2py.desired_v2.Reading`, its
    word, its text)."""
    st = live.get_state(session, key, timeout)
    mine = [r for r in st.replies if r.key == key and r.refusal is None]
    if st.errors and not mine:
        return "incomplete", "incomplete", None
    for r in mine:
        ver = _version_of(r.inband)
        if ver is None:
            # a reply with no in-band stamp carries no version (core S1)
            return "incomplete", "unstamped", None
        return ver, "tombstone" if r.deleted else "entry", ver.text
    for h in st.snapshots:
        if h.key == key and h.done and not any(p["live"] or p["tombs"] for p in h.pages.values()):
            ne = d2.NoEntry(d2.Run(h.header["run_width"], h.header["run"]), h.header["asof"])
            return ne, "no_entry", d2.Version(ne.run, ne.asof).text
    return "silent", "silent", None


def read(session: zenoh.Session, target: str, role: str | None = None, *, contracts: Iterable[Any] = (),
         timeout: float = 1.0, archives: Iterable[str] = (), from_requirement: bool = False) -> list[Row]:
    """One reading of each role of ``target`` on a desired resource (or of
    ``role``): §2.17's premises and §5's answers, each role by its major."""
    contracts = list(contracts)
    offline = {c.fingerprint: c.canonical for c in contracts}
    by_iface: dict[str, list[dict[str, Any]]] = {}
    for c in sorted(contracts, key=lambda c: -(c.minor or 0)):
        by_iface.setdefault(c.interface, []).append(c.canonical)
    reads: list[Any] = []
    t_presence, instances = dt._presence(session, target, timeout, reads)

    def unread(why: str, presence: str = t_presence) -> list[Row]:
        r = ds.Reading(presence, None, None, "not_asked", ds.INCOMPLETE, None)
        return [Row(target, role or "-", "?", "unread", ds.conclude(r), reading=r, problems=[why])]

    if t_presence != ds.PRESENT:
        return unread("target not present")
    inst = dt._id_of(instances)
    doc = dt._descriptor(session, target, inst, timeout) if inst is not None else None
    if doc is None:
        return unread("no descriptor")
    requires = [x for x in doc.get("requires") or [] if isinstance(x, dict)]
    asked = [x for x in requires if role is None or x.get("role") == role]
    rows: list[Row] = []
    for req in asked:
        rows.append(_read_role(session, target, doc, req, offline, by_iface, timeout, archives, from_requirement,
                               contracts))
    return rows


def _read_role(session: zenoh.Session, target: str, doc: dict[str, Any], req: dict[str, Any],
               offline: dict[str, dict[str, Any]], by_iface: dict[str, list[dict[str, Any]]], timeout: float,
               archives: Iterable[str], from_requirement: bool, contracts: list[Any]) -> Row:
    name, iface = str(req.get("role")), str(req.get("interface"))
    bound = bool(req.get("bindings"))
    own = dt._entry(doc, str(req.get("declared_by")))
    t_iface = str(req.get("declared_by"))
    t_canon, _ = dt._contract(session, t_iface, None if own is None else own.get("contract"), offline, timeout)
    t_canon = t_canon if isinstance(t_canon, dict) else {}
    resources = _report_resources(t_canon)
    names_it = any(d2.names_role(r, name) for r in resources)
    author = dt._first(req.get("bindings")) if bound else None
    a_presence, a_entry = None, None
    if isinstance(author, str) and len(author.split("/")) == 2:
        reads: list[Any] = []
        a_presence, a_instances = dt._presence(session, author, timeout, reads)
        a_inst = dt._id_of(a_instances)
        a_doc = dt._descriptor(session, author, a_inst, timeout) if a_inst is not None else None
        a_entry = dt._entry(a_doc, iface)
    a_canon, how = _author_major(session, iface, a_entry, offline, by_iface, timeout)
    major_v2 = d2.PROFILE in (a_canon.get("uses") or ()) if a_canon is not None else names_it
    if not major_v2:
        # §2.18: "a desired.v1 author's versions by v1 §2.6": v1's tool
        got = dt.read(session, target, name, contracts=contracts, timeout=timeout)
        r = got[0] if got else None
        return Row(target, name, ds.PROFILE, r.listing if r else "unread", r.answers if r else {}, v1=r)

    def row(r: d2.Reading, **kw: Any) -> Row:
        return Row(target, name, d2.PROFILE, "bound" if bound else "unbound", d2.conclude(r), reading=r, **kw)

    if not bound:
        # §2.12: "A tool answers the kill switch from the descriptor, as in v1."
        return row(d2.Reading(ds.PRESENT, (True, False), None, "not_asked", ds.INCOMPLETE, None))
    # 3. the report: "found as §2.10 says. No slice is needed to find it."
    naming = [r for r in resources if d2.names_role(r, name)]
    replies: list[tuple[str, bytes]] = []
    for r in naming:
        key = d2.report_key(target, t_iface, r, name)
        st = live.get_state(session, key, timeout)
        replies += [(x.key, x.raw) for x in st.replies if x.key == key and not x.deleted and x.raw]
    found = d2.find_report(target, t_iface, resources, set(t_canon.get("requires") or {}), name, replies)
    report = found.as_reading()
    place = line = None
    fields = (report.report or {}) if report is not None else {}
    decl = found.declaration
    if decl is not None:
        p, ln = fields.get("pointer"), fields.get("detail")
        ptrs, dets = list(decl.pointers or ()), list(decl.details or ())
        place = ptrs[p] if isinstance(p, int) and 0 <= p < len(ptrs) else None
        line = dets[ln] if isinstance(ln, int) and 0 <= ln < len(dets) else None
    # 4. the slice, from a revision of the author's interface in hand
    rr = (t_canon.get("requires") or {}).get(name) or {}
    requirement = {"resources": rr.get("resources"), "bindings": req.get("bindings"),
                   "params": req.get("params") or {}, "optional": req.get("optional"), "unread": False}
    named = dt._revision(a_canon) if a_canon is not None and how == ds.AUTHOR_CONTRACT else None
    held = [dt._revision(a_canon)] if a_canon is not None and how == ds.OFFLINE_CONTRACT else []
    taken = ds.take_slice(target, iface, requirement, named, held)
    key, source = None, None
    if taken.candidates and (taken.source != ds.REQUIREMENT or from_requirement):
        # "The slice from the target alone goes. A report names no key, so it
        # cannot choose among candidates": v1's way takes the first, state
        key, source = taken.candidates[0], taken.source
    shown: dict[str, Any] = {"report_key": found.key, "report": report, "found": found.found, "author": author,
                             "place": place, "line": line,
                             "author_presence": a_presence, "slice": key, "slice_from": source,
                             "problems": list(taken.binding)}
    current: Any = None
    if key is None:
        r = d2.Reading(ds.PRESENT, (True, True), report, "not_asked", a_presence or ds.INCOMPLETE, None, slice=False)
        return row(r, author_current="not_read", **shown)
    archived: list[tuple[str, str | None, bytes | None]] = []
    word, text = "not_read", None
    if a_presence == ds.PRESENT:
        current, word, text = author_read(session, key, timeout)
    elif a_presence == ds.ABSENT:
        found_archives = list(archives)
        if not found_archives:
            got = live.list_presence(session, "zk2/*/*/@zk/alive/archive.v1/**", timeout)
            found_archives = sorted({f"{x['system']}/{x['service']}" for x in got.alive})
        for arch in found_archives:
            form = alarms.archive_form(arch, key)
            st = live.get_state(session, form, timeout)
            for x in st.replies:
                if x.key == form:
                    ver = _version_of(x.inband)
                    archived.append((arch, None if ver is None else ver.text, None if x.deleted else x.payload))
    r = d2.Reading(ds.PRESENT, (True, True), report, "not_asked", a_presence or ds.INCOMPLETE, current, slice=True)
    return row(r, author_current=word, author_version=text, archived=archived, **shown)


@dataclass
class Window:
    """A reading made again over a window (v1 §2.17's first way)."""

    rows: list[Row]
    readings: int
    #: per role, the author's version a target stayed not yet on for the
    #: whole window
    behind: dict[str, str]


def watch(session: zenoh.Session, target: str, for_s: float, role: str | None = None, *, every_s: float = 0.5,
          **kw: Any) -> Window:
    """"How long behind is decided by v1's first way alone: the same answer
    in every reading over the window": not yet on one version of the
    author's in every reading, the first and the last ``for_s`` apart, at
    least two; any other answer, or another version, starts it again."""
    start: float | None = None
    first_seen: dict[str, tuple[str, float, int]] = {}
    behind: dict[str, str] = {}
    n = 0
    rows: list[Row] = []
    while True:
        rows = read(session, target, role, **kw)
        n += 1
        now = time.monotonic()
        start = now if start is None else start
        for r in rows:
            a = r.answers.get(ds.Q_CONVERGED)
            if a is None or a.answer != ds.NOT_YET or r.author_version is None:
                first_seen.pop(r.role, None)
                behind.pop(r.role, None)
                continue
            seen = first_seen.get(r.role)
            if seen is None or seen[0] != r.author_version:
                first_seen[r.role] = (r.author_version, now, 1)
                behind.pop(r.role, None)
            else:
                first_seen[r.role] = (seen[0], seen[1], seen[2] + 1)
            seen = first_seen[r.role]
            if seen[2] >= 2 and now - seen[1] >= for_s:
                behind[r.role] = r.author_version
        if now - start >= for_s:
            break
        time.sleep(every_s)
    return Window(rows, n, behind)

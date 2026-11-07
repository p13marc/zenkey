"""A late joiner's seed over snapshots: ``conformance/seed/snapshot.json``
(#977 E2), read from that fixture's description alone.

**No text states it yet.** The fixture names "core §4.2, a late joiner's
seed over snapshots: … the omission rule, asof and pages", and core 0.99's
§4.2 says nothing of a snapshot, a snapshot key or an omission: its seed
merges "the GET's replies and the subscription's samples per key" (F-498).
So every rule below is quoted from the fixture's description, and none is
a core sentence.

**The merge** (:meth:`Seed.merge`): "The merge keeps the newer by in-band
stamp: within one run, one width and one value, a higher seq; another run
is unordered and kept, the later arrival; an equal stamp is one mutation,
kept once." Every key is an owner's state key and every sample carries an
in-band stamp, "so a consumer's merge and a tool's agree on every case".

**A page** (:meth:`Seed.arrive`): "A page's entries merge as it arrives,
each on its member's key: the page's key, then ``/`` and the id, the key
itself for the empty id. A live entry is a put, a tombstone a delete, each
stamped with its own run or the header's, and its seq." "A page heard on a
subscription is neither merged nor counted."

**A snapshot** is its pages that "share their key, tag, run width, run,
asof, time and coll. It is complete once it holds each index from 0 to a
page whose more bit is 0" (:func:`zk2py.snapshot.complete`).

**The omission rule** (:meth:`Seed._omit`), once a snapshot is complete:
"on a key K that is the snapshot key or a member's key of the collection
with that coll, the omission rule drops each member of the collection at K
or under it that the reader holds and the snapshot lists neither live nor
as a tombstone, unless held under the snapshot's run with a seq above asof,
or under another run and heard after the GET was asked: every arrival was,
no view entry was, and a held value heard again by an arrival with its
stamp, kept once, then was. A member matches its template under the
snapshot key, each parameter one chunk." A reader without the contract
holds no collection, so "no omission applies, since coll maps to no
template".
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any

from . import snapshot, templates


@dataclass(frozen=True)
class Collection:
    """What the reader's contract gives it of one collection: its index,
    its template, and its snapshot key."""

    coll: int
    template: str
    snapshot_key: str

    def _segments(self) -> list[Any]:
        return list(templates.parse_template(self.template).segments)

    def rest(self) -> list[Any]:
        """The template's segments after its snapshot key: from its first
        parameter on. The snapshot key ends with the literal chunks before
        it ("a template with no parameter is its own snapshot key")."""
        segs = self._segments()
        for i, seg in enumerate(segs):
            if seg.kind != templates.LITERAL:
                return segs[i:]
        return []

    def prefix_ok(self) -> bool:
        """Whether the snapshot key ends with the template's literal chunks
        before its first parameter."""
        segs = self._segments()
        lit = [seg.text for seg in segs[:len(segs) - len(self.rest())]]
        return not lit or self.snapshot_key.split("/")[-len(lit):] == lit

    def member(self, key: str) -> bool:
        """"A member matches its template under the snapshot key, each
        parameter one chunk": a rest parameter one chunk too, as the
        description says."""
        rest = self.rest()
        if key == self.snapshot_key:
            return not rest
        if not rest or not key.startswith(self.snapshot_key + "/"):
            return False
        tail = key[len(self.snapshot_key) + 1:].split("/")
        return len(tail) == len(rest) and all(
            t != "" and (seg.kind != templates.LITERAL or t == seg.text) for t, seg in zip(tail, rest))


@dataclass
class Held:
    """A key's value as the reader holds it: ``put`` or ``delete``, its
    in-band stamp ``(width, run, seq)``, a put's record in hex, and whether
    it was heard after the GET was asked."""

    sample: str
    stamp: tuple[int, int, int]
    record: str | None
    after_get: bool

    def as_json(self, key: str) -> dict[str, Any]:
        w, r, s = self.stamp
        out: dict[str, Any] = {"key": key, "sample": self.sample, "stamp": {"width": w, "run": r, "seq": s}}
        if self.sample == "put":
            out["record"] = self.record
        return out


@dataclass
class Heard:
    """One snapshot heard by the GET: its pages by index, in order of its
    first page, and the keys its omission dropped."""

    key: str
    header: dict[str, Any]
    pages: dict[int, dict[str, Any]] = field(default_factory=dict)
    done: bool = False
    omitted: list[str] = field(default_factory=list)

    def as_json(self) -> dict[str, Any]:
        h = self.header
        return {"key": self.key, "coll": h["coll"], "run": {"width": h["run_width"], "run": h["run"]},
                "asof": h["asof"], "pages": sorted(self.pages), "complete": self.done,
                "omitted": sorted(self.omitted, key=lambda k: k.encode("utf-8"))}


def _stamp(x: dict[str, Any]) -> tuple[int, int, int]:
    return int(x["width"]), int(x["run"]), int(x["seq"])


class Seed:
    """One reader's seed: what it holds (``view``) and the snapshots its GET
    brought (``heard``), fed one arrival at a time."""

    def __init__(self, collections: list[dict[str, Any]], view: list[dict[str, Any]]):
        self.collections = [Collection(int(c["coll"]), c["template"], c["snapshot_key"]) for c in collections]
        #: key -> what the reader holds there; every view entry was held
        #: before the GET was asked
        self.view: dict[str, Held] = {
            v["key"]: Held(v["sample"], _stamp(v["stamp"]), v.get("record"), False) for v in view}
        self.heard: list[Heard] = []

    def merge(self, key: str, sample: str, stamp: tuple[int, int, int], record: str | None) -> str:
        """One arrival on ``key``: ``kept`` (it replaces what is held, or
        nothing was), ``same`` (one mutation, kept once, now heard after the
        GET) or ``older`` (refused: within one run, a seq not above the
        held one's)."""
        held = self.view.get(key)
        if held is None or held.stamp[:2] != stamp[:2]:
            self.view[key] = Held(sample, stamp, record, True)
            return "kept"
        if stamp[2] > held.stamp[2]:
            self.view[key] = Held(sample, stamp, record, True)
            return "kept"
        if stamp[2] == held.stamp[2]:
            held.after_get = True
            return "same"
        return "older"

    def arrive(self, a: dict[str, Any]) -> None:
        """One of the fixture's ``arrivals``: a sample, or a snapshot's page
        read from its bytes (:func:`zk2py.snapshot.read_page`)."""
        if "page" not in a:
            self.merge(a["key"], a["sample"], _stamp(a["stamp"]), a.get("record"))
            return
        if a["via"] != "get":
            return
        page = snapshot.read_page(bytes.fromhex(a["hex"]), "state")
        header = {k: page[k] for k in snapshot.HEADER_FIELDS}
        snap = next((s for s in self.heard if s.key == a["key"] and s.header == header), None)
        if snap is None:
            snap = Heard(a["key"], header)
            self.heard.append(snap)
        snap.pages.setdefault(page["index"], page)
        for tomb, entries in ((False, page["live"]), (True, page["tombs"])):
            for e in entries:
                width = e["run_width"] if e["run_width"] is not None else page["run_width"]
                run = e["run"] if e["run"] is not None else page["run"]
                self.merge(snapshot.member_key(a["key"], e["id"]), "delete" if tomb else "put",
                           (width, run, e["seq"]), None if tomb else e["record"])
        if not snap.done:
            whole = snapshot.complete(list(snap.pages.values()))
            if whole is not None:
                snap.done = True
                self._omit(snap, whole)

    def _omit(self, snap: Heard, whole: dict[str, Any]) -> None:
        h = snap.header
        coll = [c for c in self.collections if c.coll == h["coll"]
                and (snap.key == c.snapshot_key or c.member(snap.key))]
        if not coll:
            return
        listed = {snapshot.member_key(snap.key, e["id"]) for e in whole["live"] + whole["tombs"]}
        run = (h["run_width"], h["run"])
        for key in list(self.view):
            if key in listed or not (key == snap.key or key.startswith(snap.key + "/")):
                continue
            if not any(c.member(key) for c in coll):
                continue
            held = self.view[key]
            spared = held.stamp[2] > h["asof"] if held.stamp[:2] == run else held.after_get
            if not spared:
                del self.view[key]
                snap.omitted.append(key)

    def result(self) -> dict[str, Any]:
        """As the fixture's ``expect`` writes it."""
        keys = sorted(self.view, key=lambda k: k.encode("utf-8"))
        return {"view": [self.view[k].as_json(k) for k in keys], "snapshots": [s.as_json() for s in self.heard]}


def run_case(case: dict[str, Any]) -> dict[str, Any]:
    """One of ``seed/snapshot.json``'s cases, read: every page read from its
    bytes, and checked against the fixture's ``page``."""
    seed = Seed(case["collections"], case["view"])
    for a in case["arrivals"]:
        if "page" in a:
            got = snapshot.read_page(bytes.fromhex(a["hex"]), "state")
            if got != a["page"]:
                raise ValueError(f"page {a['hex'][:32]}… reads {got!r}, the fixture's page is {a['page']!r}")
        seed.arrive(a)
    return seed.result()

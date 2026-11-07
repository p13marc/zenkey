"""A late joiner's seed over snapshots: ``conformance/seed/snapshot.json``,
read from core §4.2 as core 0.100 (#977 E3) states it.

Round 104 read the fixture from its description alone (F-498): core 0.99
stated none of it. Core 0.100's §4.2 states every rule below, and each is
quoted from it.

**The merge** (:meth:`Seed.merge`): "A late joiner MUST merge the GET's
replies and the subscription's samples per key, whichever path brought
each one. It MUST keep the newer by its in-band stamp: within one run, by
seq (§4.3)." "Two samples with one in-band stamp are the same mutation,
kept once." Across two runs "a tool keeps the later arrival", and a
consumer "keeps a value under a run other than its held value's": every
sample here carries an in-band stamp, so the two agree.

**A page** (:meth:`Seed.arrive`): "A snapshot's entries merge as replies
do, each as its page arrives, on its member's key: the snapshot's key,
then ``/`` and the id (§7.5). A live entry is a put, and a tombstone a
delete. Each has its stamp: its own run or the header's, and its seq." "A
consumer or a tool MUST NOT merge a snapshot heard on a subscription, nor
count it toward a complete snapshot."

**A snapshot** is complete, by §7.5, "when it holds one page of each index
from 0 to a page whose more bit is 0, all on one key with one CH, run,
asof, time and coll" (:func:`zk2py.snapshot.complete`).

**The omission rule** (:meth:`Seed._omit`): "A late joiner MUST apply the
omission rule when it holds a snapshot complete (§7.5) under run R as of
asof A, on a key K that is its collection's snapshot key or a member's
key. A member of the collection at K or under it, that the snapshot lists
neither live nor as a tombstone, has no value. The late joiner drops what
it holds for the member, unless that is newer than the snapshot: a value
under R with a seq above A; a value under another run that it heard after
it asked the GET, by either path. A mutation heard again then counts,
though it is kept once." "An incomplete snapshot drops nothing."

**The owner's revision maps the index** (F-497): "A late joiner MUST map a
snapshot's collection index with the revision of the interface that the
owner's descriptor names (§3.3), never with another revision it holds."
A case's ``revisions`` and ``descriptor`` give that: :func:`collections_of`
takes the named revision's collections alone. "Where that revision has no
state resource of the reply key's kind token at the index, or has one of
which the reply's key is neither the snapshot key nor a member's key, the
late joiner applies no omission rule over that snapshot. Its entries still
merge." "Without the contract, or without the owner's revision, a reader
cannot map a collection's index to its template, so it applies no omission
rule."

**A member** of a collection is a key its template matches under the
snapshot key, each parameter one chunk (the fixture's description). The
snapshot key is "its resource's key up to the template's first parameter
… A template with no parameter is its own snapshot key".
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
        """Whether ``key`` is a member's key of this collection: its
        template matches the key's resource chunks (core §2.2), the
        snapshot key holding the template's chunks before its first
        parameter. The fixture's description says "A member matches its
        template under the snapshot key, each parameter one chunk"; a rest
        parameter takes one or more, as core §2.2 says."""
        segs = self._segments()
        lit = len(segs) - len(self.rest())
        chunks = self.snapshot_key.split("/")
        base = "/".join(chunks[:len(chunks) - lit])
        if not key.startswith(base + "/"):
            return False
        return templates.match(templates.parse_template(self.template), key[len(base) + 1:].split("/")) is not None


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


def collections_of(case: dict[str, Any]) -> list[dict[str, Any]]:
    """The collections a case's reader maps a snapshot's index by: its
    contract's ``collections``, or, where it holds several revisions, those
    of the revision the owner's descriptor names (core §4.2, F-497), "never
    … another revision it holds". A descriptor naming a revision the reader
    does not hold maps nothing: "without the owner's revision", no omission
    rule."""
    if "revisions" in case:
        return list(case["revisions"].get(case.get("descriptor"), []))
    return list(case["collections"])


def run_case(case: dict[str, Any]) -> dict[str, Any]:
    """One of ``seed/snapshot.json``'s cases, read: every page read from its
    bytes, and checked against the fixture's ``page``."""
    seed = Seed(collections_of(case), case["view"])
    for a in case["arrivals"]:
        if "page" in a:
            got = snapshot.read_page(bytes.fromhex(a["hex"]), "state")
            if got != a["page"]:
                raise ValueError(f"page {a['hex'][:32]}… reads {got!r}, the fixture's page is {a['page']!r}")
        seed.arrive(a)
    return seed.result()


# -- the live reader: a GET's snapshots, read off the bus (core 0.100) ----------

@dataclass(frozen=True)
class Member:
    """One entry of a snapshot's page as a reply it stands for (core §4.2,
    "A snapshot's entries merge as replies do … A live entry is a put, and a
    tombstone a delete. Each has its stamp: its own run or the header's,
    and its seq"): its member's key, a put's record or a delete, its
    in-band stamp, and what it says of its confirmation (§7.5): ``none``,
    ``time`` (the snapshot's ``time``) or ``conf`` (``conf``), each time
    ``{"s", "ms"}`` or None for the unknown time."""

    key: str
    deleted: bool
    record: bytes
    inband: Any
    tag: int
    confirmation: str | None = None
    conf: dict[str, int] | None = None
    time: dict[str, int] | None = None

    def put_bytes(self) -> bytes:
        """The put the entry stands for, as core §4.4 has an archive keep
        it: "CH of form 0 with the snapshot's tag and the run's width, the
        entry's run and seq, then its record"; a tombstone's delete
        envelope."""
        from . import zk2c

        if self.deleted:
            return zk2c.deletion(self.inband)
        return zk2c.mutation(self.inband, self.record, self.tag)


def read_page_of(key: str, payload: bytes) -> dict[str, Any]:
    """A GET reply's payload read as a snapshot's page (§7.5), with the
    reply key's kind token; :class:`zk2py.snapshot.Refused` or
    :class:`zk2py.zk2c.Refused` for one the reading refuses."""
    from . import zk2c

    return snapshot.read_page(payload, zk2c.key_token(key))


def members_of(key: str, page: dict[str, Any]) -> list[Member]:
    """Each entry of ``page``, heard on ``key``, as the reply it stands for:
    "on its member's key: the snapshot's key, then ``/`` and the id"."""
    from . import zk2c

    out = []
    for tomb, entries in ((False, page["live"]), (True, page["tombs"])):
        for e in entries:
            width = e["run_width"] if e["run_width"] is not None else page["run_width"]
            run = e["run"] if e["run"] is not None else page["run"]
            ib = zk2c.InBand(width, run, e["seq"])
            mk = snapshot.member_key(key, e["id"])
            if tomb:
                out.append(Member(mk, True, b"", ib, page["tag"]))
            else:
                out.append(Member(mk, False, bytes.fromhex(e["record"]), ib, page["tag"], e["confirmation"],
                                  e.get("conf"), page["time"]))
    return out


class Tracker:
    """The snapshots one GET brought, by key and header (§7.5, "Complete":
    "one page of each index from 0 to a page whose more bit is 0, all on one
    key with one CH, run, asof, time and coll")."""

    def __init__(self) -> None:
        self.heard: list[Heard] = []

    def add(self, key: str, page: dict[str, Any]) -> tuple[Heard, dict[str, Any] | None]:
        """One page heard by the GET: its snapshot, and the whole snapshot
        the first time it is complete, else None."""
        header = {k: page[k] for k in snapshot.HEADER_FIELDS}
        snap = next((s for s in self.heard if s.key == key and s.header == header), None)
        if snap is None:
            snap = Heard(key, header)
            self.heard.append(snap)
        snap.pages.setdefault(page["index"], page)
        if snap.done:
            return snap, None
        whole = snapshot.complete(list(snap.pages.values()))
        if whole is None:
            return snap, None
        snap.done = True
        return snap, whole


def collections_from(contract: Any, prefix: str) -> list[Collection]:
    """A contract's collections as a reader maps a snapshot's index with
    them (core §4.2, §7.5): each ``state`` and ``@state`` resource, its
    index "among the interface's resources of the reply key's kind token, in
    the canonical form's order", its snapshot key under ``prefix``
    (``zk2/<system>/<service>/<iface>``), or none where its first chunk is a
    parameter."""
    out = []
    for token in ("state", "@state"):
        rows = [r for r in contract.canonical["resources"] if r["token"] == token]
        for i, r in enumerate(rows):
            lit = []
            for chunk in r["template"].split("/"):
                if chunk.startswith("{"):
                    break
                lit.append(chunk)
            if lit:
                out.append(Collection(i, r["template"], f"{prefix}/{token}/{'/'.join(lit)}"))
    return out


def omitted(snap: Heard, whole: dict[str, Any], collections: list[Collection],
            held: dict[str, tuple[Any, bool]]) -> list[str]:
    """Core §4.2's omission rule over a complete snapshot ``whole`` heard on
    ``snap.key``: the keys of ``held`` (each its in-band stamp, or None, and
    whether it was heard after the GET was asked) that "a member of the
    collection at K or under it, that the snapshot lists neither live nor
    as a tombstone" holds, and that are not "newer than the snapshot: a
    value under R with a seq above A; a value under another run that it
    heard after it asked the GET". An index that names no collection of the
    reply's key ("neither the snapshot key nor a member's key") draws no
    omission rule."""
    h = snap.header
    cols = [c for c in collections if c.coll == h["coll"] and (snap.key == c.snapshot_key or c.member(snap.key))
            and snap.key.split("/")[4:5] == c.snapshot_key.split("/")[4:5]]
    if not cols:
        return []
    listed = {snapshot.member_key(snap.key, e["id"]) for e in whole["live"] + whole["tombs"]}
    run = (h["run_width"], h["run"])
    out = []
    for key, (ib, after) in held.items():
        if key in listed or not (key == snap.key or key.startswith(snap.key + "/")):
            continue
        if not any(c.member(key) for c in cols):
            continue
        same_run = ib is not None and (ib.width, ib.run) == run
        spared = (same_run and ib.seq > h["asof"]) or (not same_run and after)
        if not spared:
            out.append(key)
    return out


class ByDescriptor:
    """"The owner's revision maps the index" (core §4.2, F-497's answer): a
    late joiner's collections for a snapshot, by "the revision of the
    interface that the owner's descriptor names (§3.3), never with another
    revision it holds". :meth:`prepare` reads the descriptors of the
    services a selector reaches, before the GET, so that the GET's
    callbacks make no query; :meth:`__call__` maps a reply key and a page's
    header to the collections of the revision named, or none: "Without the
    contract, or without the owner's revision, a reader … applies no
    omission rule".

    **Two instances.** Where two descriptors of one service answer, the one
    whose ``run`` is the snapshot's names its revision (§3.3: "``run`` is
    the run the instance writes under now"); where none states it, no
    revision is named."""

    def __init__(self, contracts: list[Any]):
        self.by_fp = {c.fingerprint: c for c in contracts}
        #: "<system>/<service>" -> each descriptor read, as a dict
        self.descriptors: dict[str, list[dict[str, Any]]] = {}

    def prepare(self, session: Any, selector: str) -> None:
        import json

        from . import live

        chunks = selector.split("/")
        if len(chunks) < 3 or chunks[0] != "zk2":
            return
        for a in live.get_descriptor(session, f"zk2/{chunks[1]}/{chunks[2]}/@zk/instance/*"):
            if not a.ok:
                continue
            try:
                doc = json.loads(a.payload)
            except ValueError:
                continue
            if isinstance(doc, dict) and isinstance(doc.get("service"), str):
                self.descriptors.setdefault(doc["service"], []).append(doc)

    def __call__(self, key: str, header: dict[str, Any]) -> list[Collection]:
        from . import live, zk2c

        chunks = key.split("/")
        if len(chunks) < 5:
            return []
        service, iface = f"{chunks[1]}/{chunks[2]}", chunks[3]
        docs = self.descriptors.get(service, [])
        run = zk2c.InBand(header["run_width"], header["run"], 0).run_hex()
        named = [d for d in docs if d.get("run") == run] or (docs if len(docs) == 1 else [])
        fps = {live.fingerprint_of(d, iface) for d in named} - {None}
        if len(fps) != 1:
            return []
        contract = self.by_fp.get(fps.pop())
        if contract is None:
            return []
        return collections_from(contract, "/".join(chunks[:4]))

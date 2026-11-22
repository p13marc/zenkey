"""``freshness.v2``: confirmed by a heartbeat (profile text 0.1, experimental,
written against core 0.82; ``spec/profiles/freshness/v2.md``; #978 F3).

It restates ``freshness.v1``'s rules by reference and replaces its §2.4: an
owner confirms its collections' members by one heartbeat per covered
interface, core §7.5's form 6 (:mod:`zk2py.heartbeat`), and puts nothing
again.

The session-free half:
- :func:`uses_v2`: "A contract lists ``freshness.v1`` or ``freshness.v2`` in
  ``uses``. A contract that lists both is read under this text";
- :func:`period_ns`: the owner's cadence, "at most half the shortest horizon
  among that interface's collections apart" (§2.4);
- :class:`View`, §2.5's rule over one owner's collections for one
  interface, as ``conformance/apply.json`` reads it: the puts and deletes it
  heard, each with its in-band stamp; each heartbeat read in §2.5's four
  steps (``revision``, a refused body, ``older``, then each section
  ``not_followed``, ``absent``, ``diverged`` or ``confirmed``); a gap in the
  heartbeat key's seqs; two divergences in a row due a re-read; and a read's
  answer replacing the collection, merged with what was heard after it was
  asked.

The runtime half: :class:`Subscriber` follows collections and their
heartbeat key on a session, re-reads by core S4's GET after two divergences,
and judges a member as ``freshness.v1`` §2.5 does from its last
confirmation. The owner's heartbeats are :class:`zk2py.owner.Owner`'s.
"""

from __future__ import annotations

import threading
import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Iterable

from . import freshness, heartbeat, templates, zk2c

PROFILE = heartbeat.PROFILE
REPO = Path(__file__).resolve().parents[3]
#: §3: the standard contract
CONTRACT = REPO / "spec" / "profiles" / "freshness" / "freshness.v2.toml"
SEQ_MOD, SEQ_HALF = 2**24, 2**23


def uses_v2(canonical: dict[str, Any]) -> bool:
    """§1 "One text per contract": a contract that lists ``freshness.v2``,
    with or without ``freshness.v1``, is read under this text."""
    return PROFILE in canonical.get("uses", [])


def period_ns(layout: heartbeat.Layout) -> int | None:
    """The owner's period: 0.95 of half the shortest horizon among the
    covered interface's collections, so that two heartbeats are never more
    than ttl/2 apart with a scheduler's delay (§2.4); None with no
    collection: no heartbeat (§2.4: "and has a collection")."""
    ttl = layout.shortest_ttl_s
    return None if ttl is None else ttl * 475_000_000


def standard_contract() -> Any:
    """§3's standard contract, loaded as any contract is."""
    from .contract import load_contract

    c = load_contract(CONTRACT)
    if not c.valid:
        raise ValueError(f"{CONTRACT}: {c.codes}")
    return c


def serial_newer(new: int, old: int) -> bool:
    """Core §4.3's serial order: ``new`` is newer than ``old`` when
    (new − old) mod 2^24 is from 1 to 2^23 − 1."""
    return 1 <= (new - old) % SEQ_MOD < SEQ_HALF


@dataclass
class Member:
    """One member the view holds: its last mutation's in-band stamp (None
    for one with none), whether it is live (its last mutation a put), when
    that mutation was taken, and its last heartbeat confirmation since."""

    stamp: zk2c.InBand | None
    live: bool
    at: int
    confirmed: int | None = None


def _stamp(d: dict[str, Any] | None) -> zk2c.InBand | None:
    return None if d is None else zk2c.InBand(d["run_width"], d["run"], d["seq"])


class View:
    """§2.5 for one owner's collections of one interface, "as a reader heard
    them, each with its in-band stamp". ``follows`` names the collections it
    follows, by resource (``state/peer/{peer}``): "its subscription names no
    parameter of the collection's template". ``address`` (``system/service``)
    restricts the members to one owner's; None takes any."""

    def __init__(self, layout: heartbeat.Layout, follows: Iterable[str], address: str | None = None):
        self.layout = layout
        self.follows = set(follows)
        unknown = self.follows - {c.resource for c in layout.collections}
        if unknown:
            raise ValueError(f"no such collections of {layout.interface}: {sorted(unknown)}")
        self.address = address
        self.members: dict[str, Member] = {}
        #: the last heartbeat counted (step 3 passed), its stamp
        self.last: zk2c.InBand | None = None
        #: whether the last heartbeat read followed a gap in its key's seqs
        self.gap = False
        #: divergences in a row, by collection
        self.streak: dict[str, int] = {c.resource: 0 for c in layout.collections}
        self._parsed = {c.resource: templates.parse_template(c.template) for c in layout.collections}

    # -- membership --------------------------------------------------------------------

    def collection_of(self, key: str) -> heartbeat.Collection | None:
        """The collection a base-relative key is a member of, or None."""
        chunks = key.split("/")
        if len(chunks) < 6 or chunks[0] != "zk2" or chunks[3] != self.layout.interface:
            return None
        if self.address is not None and "/".join(chunks[1:3]) != self.address:
            return None
        for c in self.layout.collections:
            if c.token == chunks[4] and templates.match(self._parsed[c.resource], chunks[5:]) is not None:
                return c
        return None

    def live_keys(self, c: heartbeat.Collection) -> list[str]:
        """The live members it holds of ``c``, in key order: the byte order of
        their base-relative keys (core §7.5)."""
        return sorted((k for k, m in self.members.items() if m.live and self.collection_of(k) == c),
                      key=lambda k: k.encode("utf-8"))

    def digest_of(self, c: heartbeat.Collection) -> int | None:
        """The view's digest of ``c`` (core §7.5), None where a live member
        it holds has no in-band stamp: "has no digest, so its collection
        diverges"."""
        ms = []
        for k in self.live_keys(c):
            st = self.members[k].stamp
            if st is None:
                return None
            ms.append((k, st.width, st.run, st.seq))
        return heartbeat.digest(ms)

    # -- what it heard -------------------------------------------------------------------

    def mutate(self, key: str, stamp: zk2c.InBand | None, at: int, live: bool) -> bool:
        """A put (``live``) or delete it heard. "A put older than the one held
        within one run is not taken": nor a delete, nor the same stamp again
        (one mutation, kept once). Returns whether it was taken; a mutation
        taken ends the member's heartbeat confirmation."""
        if self.collection_of(key) is None:
            return False
        held = self.members.get(key)
        if held is not None and held.stamp is not None and stamp is not None and held.stamp.same_run(stamp) \
                and stamp.seq <= held.stamp.seq:
            return False
        self.members[key] = Member(stamp, live, at)
        return True

    def read(self, resource: str, asked: int, at: int, live: Iterable[tuple[str, zk2c.InBand | None]],
             deleted: Iterable[tuple[str, zk2c.InBand | None]] = (), replace: bool = True) -> None:
        """A complete read's answer of ``resource``, asked at ``asked`` and
        arrived at ``at``: "The answer MUST then replace its view of the
        collection", "merged with what the subscriber heard of the
        collection after it asked … Within one run the higher seq is kept.
        Across two runs the answer's value stands." "A member the answer left
        out leaves the view". A ``reply_del`` is a deletion with its stamp.
        Every member of the collection loses its heartbeat confirmation, and
        the collection is no longer due a read.

        ``replace`` False (text 0.7): the answer's snapshot is incomplete, so
        "An incomplete one removes nothing": the members it lists merge, and
        none leaves the view."""
        c = self.layout.collection(resource)
        answer: dict[str, tuple[zk2c.InBand | None, bool]] = {}
        for k, st in live:
            if self.collection_of(k) == c:
                answer[k] = (st, True)
        for k, st in deleted:
            if self.collection_of(k) == c:
                answer[k] = (st, False)
        mine = [k for k in self.members if self.collection_of(k) == c]
        merged: dict[str, Member] = {}
        for k in set(mine) | set(answer):
            heard = self.members.get(k)
            if not replace and heard is not None and k not in answer:
                merged[k] = heard
                continue
            heard = heard if heard is not None and heard.at >= asked else None
            got = answer.get(k)
            if heard is None:
                if got is not None:
                    merged[k] = Member(got[0], got[1], at)
                continue
            if got is None:
                merged[k] = Member(heard.stamp, heard.live, heard.at)
                continue
            st = got[0]
            if heard.stamp is not None and st is not None and heard.stamp.same_run(st) and heard.stamp.seq >= st.seq:
                merged[k] = Member(heard.stamp, heard.live, heard.at)
            else:
                merged[k] = Member(st, got[1], at)
        for k in mine:
            self.members.pop(k, None)
        self.members.update(merged)
        self.streak[c.resource] = 0

    # -- a heartbeat ---------------------------------------------------------------------

    def heartbeat(self, tag: int, stamp: zk2c.InBand, body: bytes, at: int) -> dict[str, Any]:
        """§2.5's rule, one heartbeat at ``at``, in its steps, stopping at the
        first that answers. Returns ``apply.json``'s ``expect``."""
        if tag != self.layout.tag:
            return {"revision": True}
        try:
            value = heartbeat.read(self.layout, body)
        except heartbeat.Refused as e:
            return {"refused": e.token}
        last = self.last
        if last is not None and last.same_run(stamp) and not serial_newer(stamp.seq, last.seq):
            return {"older": True}
        gap = last is not None and last.same_run(stamp) and stamp.seq != (last.seq + 1) % SEQ_MOD
        self.last, self.gap = stamp, gap
        sections: dict[str, Any] = {}
        for c in self.layout.collections:
            if c.resource not in self.follows:
                sections[c.resource] = "not_followed"
                continue
            sections[c.resource] = self.section(c, value["sections"].get(c.resource), at)
        return {"read": {"gap": gap, "sections": sections}}

    def section(self, c: heartbeat.Collection, sec: dict[str, Any] | None, at: int) -> Any:
        """Step 4 for one collection it follows, ``sec`` its section's value
        or None for a map bit of 0: ``absent``, ``diverged``, or the members
        confirmed at ``at``. It moves the collection's count of divergences
        in a row: "A collection has one count" (text 0.9), whichever form
        its sections came in."""
        if sec is None:
            self.streak[c.resource] = 0
            return "absent"
        keys = self.live_keys(c)
        mine = self.digest_of(c)
        if mine is None or mine != int(sec["digest"], 16) or (c.templated and sec["live"] != len(keys)):
            self.streak[c.resource] += 1
            return "diverged"
        self.streak[c.resource] = 0
        if c.templated:
            confirmed = [keys[r] for r in sec["vouched"]]
        else:
            confirmed = keys if sec["vouched"] and keys else []
        for k in confirmed:
            self.members[k].confirmed = at
        return {"confirmed": confirmed}

    def reread_due(self) -> list[str]:
        """The collections two heartbeats in a row diverged on, in map
        order."""
        return [c.resource for c in self.layout.collections if self.streak[c.resource] >= 2]

    def confirmed(self, key: str) -> int | None:
        """The member's last heartbeat confirmation since its last mutation,
        or None (none, or not held)."""
        m = self.members.get(key)
        return None if m is None else m.confirmed

    def check(self, keys: Iterable[str]) -> dict[str, Any]:
        """``apply.json``'s ``check``."""
        return {"reread_due": self.reread_due(), "gap": self.gap,
                "confirmed": {k: self.confirmed(k) for k in keys}}


# -- text 0.9: the owner heartbeat, one put per owner and class (§2.4, §2.5) ---------

#: §2.4 "The class": ``link`` or ``host``, and §3: "Its parameter is `link`
#: or `host`, and an owner puts on no other."
CLASSES = ("link", "host")


def class_of(token: str, annotations: dict[str, Any]) -> str:
    """§2.4: "The class of a collection is `link` where its resolved
    `link.exposure` reads `link`, as link.v1 §2.2 reads it. It is `host`
    otherwise: absent, or any other value." """
    from . import link

    return "link" if link.exposure_of(token, annotations) == "link" else "host"


def classes_of(canonical: dict[str, Any]) -> dict[str, str]:
    """Each resource of a revision, ``<kind token>/<template>``, with its
    class, "as the revision it holds reads it" (§2.5)."""
    return {f"{r['token']}/{r['template']}": class_of(r["token"], r.get("annotations") or {})
            for r in canonical["resources"]}


def owner_heartbeat_key(system: str, service: str, cls: str) -> str:
    """§3: ``zk2/<system>/<service>/freshness.v2/stream/heartbeats/<class>``."""
    if cls not in CLASSES:
        raise ValueError(f"{cls!r}: an owner heartbeat key's class is link or host")
    return f"zk2/{system}/{service}/{PROFILE}/stream/heartbeats/{cls}"


@dataclass
class KeySequence:
    """One owner heartbeat key's sequence, in front of the views (§2.5 "A
    lost owner heartbeat": "The sequence is the owner heartbeat key's"):
    the last read owner heartbeat's stamp, whether it followed a lost one,
    and the interfaces a group on the key has named within the run."""

    last: zk2c.InBand | None = None
    gap: bool = False
    named: set[str] = field(default_factory=set)


class Reader:
    """§2.5 "An owner heartbeat" for one provider: a :class:`View` per
    interface the subscriber follows a collection of, and a
    :class:`KeySequence` per class key in front of them.

    - ``held``: the catalogs the reader holds (core §8.6);
    - ``named``: the id of the catalog its owner names (core §3.3), None for
      an owner that names none, whose every index is ``unknown_catalog``.

    :meth:`follow` adds an interface: the revision the reader holds, and the
    collections it follows. :meth:`owner_heartbeat` reads one put on a class
    key in §2.5's three steps. :meth:`heartbeat` reads an interface's own
    heartbeat through its view, as before. :meth:`lost` is the collections a
    key covers while its last read owner heartbeat followed a lost one."""

    def __init__(self, held: Iterable[Any] = (), named: str | None = None):
        self.held = list(held)
        self.named = named
        self.views: dict[str, View] = {}
        self.classes: dict[str, dict[str, str]] = {}
        self.keys: dict[str, KeySequence] = {c: KeySequence() for c in CLASSES}

    def follow(self, canonical: dict[str, Any], fingerprint: str, follows: Iterable[str] | None = None,
               address: str | None = None, view: View | None = None) -> View:
        """Follow collections of one interface, by the revision held."""
        layout = heartbeat.layout_of(canonical, fingerprint)
        if view is None:
            view = View(layout, [c.resource for c in layout.collections] if follows is None else follows, address)
        self.views[layout.interface] = view
        self.classes[layout.interface] = classes_of(canonical)
        return view

    def class_of(self, iface: str, resource: str) -> str:
        return self.classes[iface][resource]

    def followed_classes(self) -> list[str]:
        """§2.5 "Both forms": the owner heartbeat keys a subscriber
        subscribes to, "one for each class that the collections it follows
        have"."""
        have = {self.classes[i][r] for i, v in self.views.items() for r in v.follows}
        return [c for c in CLASSES if c in have]

    # -- one owner heartbeat --------------------------------------------------------------

    def owner_heartbeat_put(self, cls: str, payload: bytes, at: int, enveloped: bool = True) -> dict[str, Any]:
        """A put's payload on the class ``cls``'s key. Step 1 holds the
        envelope's parse too, and the put's form: "The envelope's parse
        comes first (above), then the put's form (``form``, below)" (core
        §7.5, 0.109). ``enveloped`` False is a put whose Encoding carries no
        envelope, refused ``form`` too. "A subscriber confirms nothing by a
        put refused so"."""
        from . import owner_heartbeat as oh

        try:
            put = oh.read(payload, enveloped=enveloped)
        except oh.Refused as e:
            return {"refused": e.token}
        return self._read(cls, put.stamp, put.body, at)

    def owner_heartbeat(self, cls: str, tag: int, stamp: zk2c.InBand, body: bytes, at: int) -> dict[str, Any]:
        """An owner heartbeat as its envelope gave it: CH's tag, the stamp
        and the body's bytes.
        1. **Refused**: "Core §7.5 refuses the owner heartbeat, before any
           group's sections are read: nothing is confirmed."
        2. **Older**: "Within one run, its seq is not after the last read
           owner heartbeat's on its key".
        3. **Read**: each group alone, in order."""
        from . import owner_heartbeat as oh

        if tag != 0:
            return {"refused": "tag"}
        try:
            value = oh.read_body(body)
        except oh.Refused as e:
            return {"refused": e.token}
        return self._read(cls, stamp, value, at)

    def _read(self, cls: str, stamp: zk2c.InBand, value: Any, at: int) -> dict[str, Any]:
        if cls not in CLASSES:
            raise ValueError(f"{cls!r}: no owner heartbeat key")
        k = self.keys[cls]
        last = k.last
        same = last is not None and last.same_run(stamp)
        if same and not serial_newer(stamp.seq, last.seq):
            return {"older": True}
        # "A read owner heartbeat follows a lost one when its seq is not the
        # last read one's plus 1 on its key, within one run."
        gap = same and stamp.seq != (last.seq + 1) % SEQ_MOD
        if not same:
            # "Another run on a key starts its sequence again, and its
            # covered collections with it."
            k.named = set()
        k.last, k.gap = stamp, gap
        groups = [self._group(cls, g, at) for g in value.groups]
        return {"read": {"gap": gap, "groups": groups, "lost": self.lost(cls)}}

    def _group(self, cls: str, g: Any, at: int) -> dict[str, Any]:
        """One group, read alone: "stops at the first of these that
        answers"."""
        from . import catalog as cat
        from . import owner_heartbeat as oh

        out: dict[str, Any] = {"index": g.index}
        # No index.
        try:
            entry = cat.resolve(self.held, self.named, g.index).entry
        except cat.Refused as e:
            return {**out, "refused": e.token}
        if entry is None:
            # resolve gives an entry for every index it does not refuse; one
            # that gave none is read as an index it refused, never guessed
            return {**out, "refused": "unknown_index"}
        iface, contract = entry
        # "A group names an interface once the subscriber has read its
        # index: any group of a read owner heartbeat that "No index" did not
        # stop."
        self.keys[cls].named.add(iface)
        out["iface"] = iface
        # Not followed.
        view = self.views.get(iface)
        if view is None or not view.follows:
            return {**out, "answer": "not_followed"}
        # Another revision.
        if contract != view.layout.fingerprint or g.tag != view.layout.tag:
            return {**out, "answer": "revision"}
        # Refused: "The sections are parsed whole, the other class's too."
        try:
            sections = oh.read_sections(view.layout, g.bits)
        except oh.Refused as e:
            return {**out, "refused": e.token}
        # Read.
        read: dict[str, Any] = {}
        for c in view.layout.collections:
            if c.resource not in view.follows:
                read[c.resource] = "not_followed"
            elif self.classes[iface][c.resource] != cls:
                # "A followed collection of the other class is not read,
                # whatever its map bit says."
                read[c.resource] = "other_class"
            else:
                read[c.resource] = view.section(c, sections.get(c.resource), at)
        return {**out, "sections": read}

    # -- what a gap says ------------------------------------------------------------------

    def lost(self, cls: str) -> list[tuple[str, str]]:
        """§2.5 "A lost owner heartbeat": while the key's last read owner
        heartbeat followed a lost one, "The collections a key covers, to a
        subscriber, are of the key's class, as the revision it holds reads
        it. They are those of each interface it follows that a group on
        that key has named within the run." As (interface, resource), the
        ones it follows; empty while the key is in sequence."""
        k = self.keys[cls]
        if not k.gap:
            return []
        return [(iface, c.resource) for iface in sorted(k.named) if iface in self.views
                for c in self.views[iface].layout.collections
                if c.resource in self.views[iface].follows and self.classes[iface][c.resource] == cls]

    def lost_delivery(self, iface: str, resource: str) -> bool:
        """Whether a member of this collection reads as v1 §2.5 rule 3 reads
        a lost delivery: its interface's own heartbeat followed a gap, or its
        class's key did and covers it. "Two keys are two sequences. A gap on
        one key says nothing of the other class's collections. It says
        nothing of an interface confirmed by its own heartbeat." """
        view = self.views.get(iface)
        if view is None:
            return False
        return view.gap or (iface, resource) in self.lost(self.classes[iface][resource])

    def heartbeat(self, iface: str, tag: int, stamp: zk2c.InBand, body: bytes, at: int) -> dict[str, Any]:
        """An interface's own heartbeat, on its own key: its view's, as
        before."""
        return self.views[iface].heartbeat(tag, stamp, body, at)

    def reread_due(self) -> list[tuple[str, str]]:
        """"The re-read follows two in a row, as above": (interface,
        resource)."""
        return [(i, r) for i, v in self.views.items() for r in v.reread_due()]


def apply_case(layout: heartbeat.Layout, case: dict[str, Any]) -> list[tuple[str, Any, Any]]:
    """One ``apply.json`` case through a :class:`View`: each heartbeat's
    answer and each check, as (what, got, want)."""
    v = View(layout, case["follows"])
    out = []
    for i, step in enumerate(case["steps"]):
        if "put" in step:
            v.mutate(step["put"]["key"], _stamp(step["put"]["stamp"]), step["at"], True)
        elif "delete" in step:
            v.mutate(step["delete"]["key"], _stamp(step["delete"]["stamp"]), step["at"], False)
        elif "read" in step:
            r = step["read"]
            v.read(r["collection"], r["asked"], step["at"], [(m["key"], _stamp(m["stamp"])) for m in r["live"]])
        elif "heartbeat" in step:
            h = step["heartbeat"]
            got = v.heartbeat(h["tag"], _stamp(h["stamp"]), bytes.fromhex(h["body"]), step["at"])
            out.append((f"step {i}: the heartbeat at {step['at']}", got, step["expect"]))
        elif "check" in step:
            want = step["check"]
            out.append((f"step {i}: the check", v.check(want["confirmed"]), want))
        else:
            out.append((f"step {i}", sorted(step), "a step apply.json defines"))
    return out


def _fixture_group(g: dict[str, Any]) -> dict[str, Any]:
    """A group's answer in ``apply.json``'s spelling."""
    out: dict[str, Any] = {"index": g["index"]}
    if "refused" in g:
        out["refused"] = g["refused"]
    elif "answer" in g:
        out[g["answer"]] = True
    else:
        out["sections"] = g["sections"]
    return out


def apply_owner_case(contracts: dict[str, Any], case: dict[str, Any],
                     reader: Any = None) -> list[tuple[str, Any, Any]]:
    """One ``apply.json`` case with a ``catalog`` (text 0.9) through a
    :class:`Reader`: each heartbeat's and owner heartbeat's answer and each
    check, as (what, got, want). ``contracts`` are the layouts' contracts,
    by name. "`catalog` is the catalog the owner names, which the subscriber
    holds: the layouts its entries name, in index order, an entry `{iface,
    contract}` naming a revision of `iface` that is no layout's; or `null`
    where the owner names none." """
    from . import catalog as cat

    make = Reader if reader is None else reader  # a changed reader, for what a case pins
    if case["catalog"] is None:
        r = make()
    else:
        entries = [(contracts[e].interface, contracts[e].fingerprint) if isinstance(e, str)
                   else (e["iface"], e["contract"]) for e in case["catalog"]]
        c = cat.Catalog(epoch=0, entries=entries, id="00000000")
        r = make([c], c.id)
    names = {}  # interface id -> layout name
    for name, follows in case["follows"].items():
        v = r.follow(contracts[name].canonical, contracts[name].fingerprint, follows)
        names[v.layout.interface] = name
    read_on: list[str] = []  # the keys one has been read on, a class or a layout
    out = []
    for i, step in enumerate(case["steps"]):
        at = step.get("at")
        if "put" in step or "delete" in step:
            m = step.get("put") or step["delete"]
            for v in r.views.values():
                v.mutate(m["key"], _stamp(m["stamp"]), at, "put" in step)
        elif "read" in step:
            rd = step["read"]
            r.views[contracts[rd["layout"]].interface].read(
                rd["collection"], rd["asked"], at, [(m["key"], _stamp(m["stamp"])) for m in rd["live"]])
        elif "heartbeat" in step:
            h = step["heartbeat"]
            got = r.heartbeat(contracts[h["layout"]].interface, h["tag"], _stamp(h["stamp"]),
                              bytes.fromhex(h["body"]), at)
            if "read" in got and h["layout"] not in read_on:
                read_on.append(h["layout"])
            out.append((f"step {i}: the heartbeat at {at}", got, step["expect"]))
        elif "owner_heartbeat" in step:
            h = step["owner_heartbeat"]
            got = r.owner_heartbeat(h["class"], h["tag"], _stamp(h["stamp"]), bytes.fromhex(h["body"]), at)
            if "read" in got:
                if h["class"] not in read_on:
                    read_on.append(h["class"])
                got = {"read": {"gap": got["read"]["gap"],
                                "groups": [_fixture_group(g) for g in got["read"]["groups"]]}}
            out.append((f"step {i}: the owner heartbeat at {at}", got, step["expect"]))
        elif "check" in step:
            want = step["check"]
            due: dict[str, list[str]] = {}
            for iface, resource in r.reread_due():
                due.setdefault(names[iface], []).append(resource)
            lost: dict[str, list[str]] = {}
            for iface, v in r.views.items():
                mine = [c.resource for c in v.layout.collections
                        if c.resource in v.follows and r.lost_delivery(iface, c.resource)]
                if mine:
                    lost[names[iface]] = mine
            gap = {k: (r.keys[k].gap if k in CLASSES else r.views[contracts[k].interface].gap) for k in read_on}
            confirmed = {k: next((v.confirmed(k) for v in r.views.values() if v.collection_of(k) is not None), None)
                         for k in want["confirmed"]}
            out.append((f"step {i}: the check", {"reread_due": due, "gap": gap, "lost": lost, "confirmed": confirmed},
                        want))
        else:
            out.append((f"step {i}", sorted(step), "a step apply.json defines"))
    return out


# -- the runtime half: a subscriber (§2.5) ---------------------------------------------

@dataclass
class Heard:
    """What a subscriber heard on one key, for its judgement."""

    kind: str                 # "put" or "delete": the last mutation heard
    put_at: int | None        # the last put's arrival
    last_confirmation: int    # its monotonic clock


@dataclass
class HeartbeatRecord:
    """One heartbeat as a subscriber heard it."""

    arrival_ns: int
    stamp: zk2c.InBand | None
    tag: int | None
    body: bytes | None
    answer: dict[str, Any]
    value: dict[str, Any] | None = None
    refusal: str | None = None


class Subscriber:
    """§2.5 on a session: one owner's collections of one interface, and its
    heartbeat key covering it. "It subscribes to each provider's heartbeat
    key covering the interface (§3)."

    - ``address``: the owner's ``system/service``;
    - ``canonical`` and ``fingerprint``: the covered revision it holds;
    - ``follows``: the collections it follows, each subscribed by its
      template's selector, every parameter a wildcard ("its subscription
      names no parameter of the collection's template");
    - ``seed``: whether it seeds each collection by its own S4 GET at the
      start, a late joiner's (core §4.2): the answer is a read, which
      replaces its view;
    - ``reread``: whether it re-reads a collection after two divergences in
      a row (the SHOULD), by S4's GET of the collection's template, each
      parameter a wildcard, or the member's key for one of one member.

    It ages each member from its last confirmation, a put it heard or a
    heartbeat that confirmed it, on its monotonic clock (§2.5), and reads
    every member unobservable past ttl while the last heartbeat followed a
    gap ("as v1 §2.5 rule 3 reads a lost delivery")."""

    def __init__(self, session: Any, address: str, canonical: dict[str, Any], fingerprint: str,
                 follows: Iterable[str] | None = None, seed: bool = False, reread: bool = True,
                 get_timeout: float = 1.0, wait_s: float = 0.2, reader: "Reader | None" = None):
        import zenoh

        from . import seed_snapshot as ss
        from types import SimpleNamespace

        self.session = session
        self.address = address
        self.layout = heartbeat.layout_of(canonical, fingerprint)
        #: the revision the reader holds, by which a stray reply_del is told
        #: (core 0.102 §4.2)
        self.revision = ss.ByDescriptor([SimpleNamespace(canonical=canonical, fingerprint=fingerprint)])
        #: core §7.5: each collection's index "among the interface's
        #: resources of the reply key's kind token, in the canonical form's
        #: order", by ``<token>/<template>``
        self.coll_index = {f"{t}/{r['template']}": i for t in ("state", "@state")
                           for i, r in enumerate(x for x in canonical["resources"] if x["token"] == t)}
        self.follows = list(follows) if follows is not None else [c.resource for c in self.layout.collections]
        self.view = View(self.layout, self.follows, address)
        #: text 0.9: the provider's reader over its owner heartbeat keys,
        #: which this interface's view sits behind (:class:`Provider`); None
        #: for a subscriber that reads heartbeats alone
        self.reader = reader
        if reader is not None:
            reader.follow(canonical, fingerprint, view=self.view)
        self.reread_enabled = reread
        self.get_timeout = get_timeout
        self.lock = threading.RLock()
        self.heard: dict[str, Heard] = {}
        self.heartbeats: list[HeartbeatRecord] = []
        #: (arrival, key, put or delete, in-band stamp, Zenoh timestamp text)
        self.samples: list[tuple[int, str, str, zk2c.InBand | None, str | None]] = []
        #: (collection, asked, arrived, members answered) per re-read, and
        #: per seed (a late joiner's own GET at the start)
        self.reads: list[tuple[str, int, int, int]] = []
        self.seeds: list[tuple[str, int, int, int]] = []
        #: each member's last confirmation: a put heard, or a heartbeat that
        #: confirmed it, on its monotonic clock
        self.last_conf: dict[str, int] = {}
        self._reading: set[str] = set()
        #: a read whose snapshot came incomplete, (collection, arrival): it
        #: removed nothing (§2.5, 0.7)
        self.incomplete_reads: list[tuple[str, int]] = []
        self.closed = False
        self.declared_ns = time.monotonic_ns()
        prefix = f"zk2/{address}/{self.layout.interface}"
        self.selectors = {c.resource: f"{prefix}/{c.token}/{_selector(c.template)}"
                          for c in self.layout.collections if c.resource in self.follows}
        #: §2.5 "The re-read" (text 0.7): "The GET is a collection read: the
        #: collection's snapshot key followed by ``/**`` (core §4.2)"; a
        #: template whose first chunk is a parameter has no snapshot key, and
        #: is read by its member selector
        self.read_selectors = {c.resource: _collection_read(prefix, c.token, c.template) or self.selectors[c.resource]
                               for c in self.layout.collections if c.resource in self.follows}
        self.hb_key = heartbeat.heartbeat_key(*address.split("/"), self.layout.interface)
        self.subs = [session.declare_subscriber(sel, zenoh.handlers.Callback(self._state))
                     for sel in self.selectors.values()]
        self.subs.append(session.declare_subscriber(self.hb_key, zenoh.handlers.Callback(self._beat)))
        if wait_s:
            time.sleep(wait_s)
        if seed:
            for resource in list(self.selectors):
                self._read(resource, seed=True)

    def _state(self, sample: Any) -> None:
        import zenoh

        at = time.monotonic_ns()
        key = str(sample.key_expr)
        if "*" in key:
            return  # core R6
        read = zk2c.read_sample(sample)
        live = sample.kind != zenoh.SampleKind.DELETE
        with self.lock:
            self.samples.append((at, key, "put" if live else "delete", read.stamp,
                                 None if sample.timestamp is None else str(sample.timestamp)))
            if read.refused:
                return
            self.view.mutate(key, read.stamp, at, live)
            if live:
                self.last_conf[key] = at
            prev = self.heard.get(key)
            self.heard[key] = Heard("put" if live else "delete", at if live else (prev.put_at if prev else None),
                                    at if live else (prev.last_confirmation if prev else at))

    def _beat(self, sample: Any) -> None:
        at = time.monotonic_ns()
        try:
            # core §7.5 (0.109), "Another form on a heartbeat's key": the
            # parse, then the put's form, "before it reads a tag or a body";
            # "A put with no envelope on either key is refused the same way".
            # An envelope's Encoding is `zenoh/bytes` whole, a schema suffix
            # being another Encoding (core §7.2, `envelope/carries.json`):
            # :func:`zk2py.zk2c.envelope_encoding` holds the sentences
            env = heartbeat.put_of(sample.payload.to_bytes(),
                                   enveloped=zk2c.envelope_encoding(sample.encoding))
        except heartbeat.Refused as e:
            with self.lock:
                self.heartbeats.append(HeartbeatRecord(at, None, None, None, {"refused": e.token}, refusal=e.token))
            return
        due: list[str] = []
        with self.lock:
            answer = self.view.heartbeat(env.tag, env.stamp, env.record or b"", at)
            value = None
            try:
                value = heartbeat.read(self.layout, env.record or b"")
            except heartbeat.Refused:
                pass
            self.heartbeats.append(HeartbeatRecord(at, env.stamp, env.tag, env.record, answer, value))
            for resource, sec in answer.get("read", {}).get("sections", {}).items():
                if isinstance(sec, dict):
                    for k in sec["confirmed"]:
                        self.last_conf[k] = at
                        h = self.heard.get(k)
                        if h is not None:
                            h.last_confirmation = max(h.last_confirmation, at)
            if self.reread_enabled:
                due = [r for r in self.view.reread_due() if r not in self._reading]
                self._reading.update(due)
        for resource in due:
            threading.Thread(target=self._read, args=(resource,), daemon=True).start()

    def _read(self, resource: str, seed: bool = False) -> None:
        """S4's GET of the collection, its answer a read (§2.5 "The
        re-read"). A read that fails, silent or ended at its timeout,
        replaces nothing."""
        from . import live

        c = self.layout.collection(resource)
        asked = time.monotonic_ns()
        try:
            if self.closed:
                return
            # core 0.102 §4.2 (review-bot on #1100, 2): the owner's revision,
            # the one this reader holds, read by its descriptors
            st = live.get_state(self.session, self.read_selectors[resource], timeout=self.get_timeout,
                                revision=self.revision)
            arrived = time.monotonic_ns()
            if st.silent:
                return
            ok = [(r.key, r.inband) for r in st.replies if not r.deleted and r.refusal is None]
            gone = [(r.key, r.inband) for r in st.replies if r.deleted]
            # §2.5 (0.7): "A member a complete snapshot left out leaves the
            # view, by core §4.2's omission rule. An incomplete one removes
            # nothing: the subscriber reads again."
            key = _collection_key(f"zk2/{self.address}/{self.layout.interface}", c.token, c.template)
            whole = key is not None and any(
                h.done and h.key == key and h.header["coll"] == self.coll_index.get(c.resource) for h in st.snapshots)
            with self.lock:
                self.view.read(c.resource, asked, arrived, ok, gone, replace=whole)
                (self.seeds if seed else self.reads).append((c.resource, asked, arrived, len(ok)))
            if not whole and key is not None and any(h.key == key and not h.done for h in st.snapshots):
                with self.lock:
                    self.incomplete_reads.append((c.resource, arrived))
        except Exception:  # noqa: BLE001 - a read that fails replaces nothing
            return
        finally:
            with self.lock:
                self._reading.discard(resource)

    # -- the judgement --------------------------------------------------------------------

    def horizon(self, key: str) -> freshness.Horizon:
        c = self.view.collection_of(key)
        if c is None:
            raise KeyError(f"{key}: no collection it follows")
        return freshness.Horizon("within", c.ttl_s)

    def observation(self, key: str, now_ns: int | None = None) -> freshness.Subscription:
        """freshness.v1 §2.5's observation of ``key`` at ``now_ns``: its last
        confirmation the later of its last put and the last heartbeat that
        confirmed it; incomplete while the last heartbeat followed a gap."""
        now = time.monotonic_ns() if now_ns is None else now_ns
        with self.lock:
            h = self.heard.get(key)
            m = self.view.members.get(key)
            gap = self.view.gap
            if self.reader is not None:
                # text 0.9 "A lost owner heartbeat": "read every member of
                # the collections that key covers as v1 §2.5 rule 3 reads a
                # lost delivery"
                c = self.view.collection_of(key)
                gap = gap or (c is not None and self.reader.lost_delivery(self.layout.interface, c.resource))
            last = self.last_conf.get(key)
        if (m is not None and not m.live) or (m is None and h is not None and h.kind == "delete"):
            return freshness.Subscription(now - self.declared_ns, not gap, freshness.Delivery("delete", 0))
        if last is None:
            return freshness.Subscription(now - self.declared_ns, not gap, None)
        return freshness.Subscription(now - self.declared_ns, not gap, freshness.Delivery("put", now - last))

    def judge(self, key: str, now_ns: int | None = None) -> freshness.Verdict:
        return freshness.judge_subscription(self.horizon(key), self.observation(key, now_ns))

    def answers(self, since_ns: int = 0) -> list[HeartbeatRecord]:
        with self.lock:
            return [h for h in self.heartbeats if h.arrival_ns >= since_ns]

    def close(self) -> None:
        self.closed = True
        for s in self.subs:
            try:
                s.undeclare()
            except Exception:  # noqa: BLE001 - closing anyway
                pass


@dataclass
class OwnerHeartbeatRecord:
    """One owner heartbeat as a provider's subscriber heard it."""

    arrival_ns: int
    cls: str
    stamp: zk2c.InBand | None
    payload: bytes
    answer: dict[str, Any]


#: How long a descriptor read that named no catalog stands for its run
#: before a subscriber asks again (§2.5: "The subscriber asks again later,
#: and not at every put"). The figure is zk2py's, and kept apart from the
#: reference's 30 s on purpose (review-bot on #1150): the text states none,
#: and freshness.v2 0.11's entry says "Those are its figures". A second
#: implementation at another figure shows that no rule rests on it: round
#: 117's pairings pass with zk2py at 10 s beside the reference, whose figure
#: the text gives as 30 s and zk2py did not measure.
ASK_AGAIN_S = 10.0


@dataclass
class Naming:
    """§2.5 "The catalog its owner names" (text 0.11), run by run: "An owner
    states it in its descriptor, as ``catalog`` (core §3.3). The descriptor
    is the one that states the put's run: a run is one instance's".

    zk2py's subscriber does the first of the two things the text allows
    "before it knows": "It refuses. It reads each group ``unknown_catalog``,
    as step 3 says. The owner heartbeat is still a read one." It holds no
    put.

    - ``word``: "Where no descriptor answers, as across a constrained face,
      the subscriber takes the deployment's word: the catalog it was
      provisioned". Given, no descriptor is read, and every run reads
      through it.
    - :meth:`of` is the id a put of ``run`` is read through, None where
      none is known: each group is then ``unknown_catalog``.
    - :meth:`due` says whether to read the descriptors for ``run`` now: no
      read is under way for it, and none failed for it within
      ``ask_again_ns``. "A read that fails names no catalog. Neither does
      one whose descriptors all state another run. The subscriber asks
      again later, and not at every put."
    - :meth:`answer` takes a read's descriptors, each a parsed document. A
      descriptor of the run that states no ``catalog`` is an answer: its
      owner names none (core §8.6), and the run is not asked again.
    - :meth:`begin` marks a read that asks for no run, the one a subscriber
      makes at its start. It reads every instance's descriptor, so while it
      is under way it covers every run: :meth:`due` asks for none, and a
      put heard meanwhile starts no second GET (review-bot on #1150).
      :meth:`answer` with no run ends it. A run it did not name is asked
      for at that run's next put."""

    word: str | None = None
    ask_again_ns: int = int(ASK_AGAIN_S * 1e9)
    known: dict[str, str | None] = field(default_factory=dict)
    failed: dict[str, int] = field(default_factory=dict)
    asking: set[str] = field(default_factory=set)
    #: reads asked at a put, for its run
    reads: int = 0
    #: run-less reads under way (:meth:`begin`), and how many were begun
    starting: int = 0
    started: int = 0

    def of(self, run: str | None) -> str | None:
        if self.word is not None:
            return self.word
        return None if run is None else self.known.get(run)

    def begin(self) -> bool:
        """A read for no run begins. False, and nothing begun, where the
        deployment's word is given: no descriptor is read then."""
        if self.word is not None:
            return False
        self.starting += 1
        self.started += 1
        return True

    def due(self, run: str | None, now_ns: int) -> bool:
        if self.word is not None or run is None or run in self.known or run in self.asking:
            return False
        if self.starting:
            return False  # the read under way reads this run's descriptor too
        f = self.failed.get(run)
        if f is not None and now_ns - f < self.ask_again_ns:
            return False
        self.asking.add(run)
        self.reads += 1
        return True

    def answer(self, run: str | None, descriptors: Iterable[Any], now_ns: int) -> None:
        for d in descriptors:
            r = d.get("run") if isinstance(d, dict) else None
            if isinstance(r, str):
                c = d.get("catalog")
                self.known[r] = c if isinstance(c, str) else None
        if run is None:
            self.starting = max(0, self.starting - 1)
        else:
            self.asking.discard(run)
            if run in self.known:
                self.failed.pop(run, None)
            else:
                self.failed[run] = now_ns


def descriptors_of(session: Any, address: str, timeout: float = 1.0) -> list[Any]:
    """Every descriptor ``address``'s instances answer with, each a parsed
    document: one GET of ``zk2/<address>/@zk/instance/*`` to every
    queryable (core §3.3 "The GET"), so a tokenless owner's is read too. A
    reply that is no JSON object is left out."""
    import json

    import zenoh

    from . import live

    out = []
    for a in live._answers(session, f"zk2/{address}/@zk/instance/*", zenoh.QueryTarget.ALL, timeout):
        if not a.ok or a.key is None or "*" in a.key:
            continue
        try:
            d = json.loads(a.payload)
        except ValueError:
            continue
        if isinstance(d, dict):
            out.append(d)
    return out


def descriptor_catalog(session: Any, address: str, timeout: float = 1.0) -> str | None:
    """The id of the catalog ``address``'s present instance names in its
    descriptor (core §3.3: "An owner given a catalog MUST state its id"), or
    None: no instance read, or it names none."""
    import json

    from . import live

    pres = live.list_presence(session, f"zk2/{address}/@zk/instance/*")
    if not pres.instances:
        return None
    d = live.get_descriptor(session, f"zk2/{address}/@zk/instance/{pres.instances[0]['instance']}", timeout=timeout)
    if len(d) != 1 or not d[0].ok:
        return None
    return json.loads(d[0].payload).get("catalog")


class Provider:
    """§2.5 on a session for one provider, both forms (text 0.9): a
    :class:`Subscriber` per interface followed, each on its collections and
    its interface's heartbeat key, and the provider's owner heartbeat keys,
    "one for each class that the collections it follows have", read through
    one :class:`Reader` in front of the subscribers' views.

    - ``revisions``: ``(canonical, fingerprint, follows)`` per interface,
      ``follows`` None for every collection;
    - ``held``: the catalogs the reader holds (core §8.6);
    - ``named``: the deployment's word for the catalog the owner names, as
      across a face, taken with no descriptor read. None reads it from the
      owner's descriptors, run by run (:class:`Naming`, text 0.11): once at
      the start, then at the first owner heartbeat of a run it knows no
      catalog for, and after a read that fails no sooner than
      ``ask_again_s`` later. Until then each group is ``unknown_catalog``;
    - ``seed``: the interfaces seeded by their own S4 GET at the start."""

    def __init__(self, session: Any, address: str, revisions: Iterable[tuple[dict[str, Any], str, Any]],
                 held: Iterable[Any] = (), named: str | None = None, seed: Iterable[str] = (),
                 reread: bool = True, get_timeout: float = 1.0, wait_s: float = 0.2,
                 ask_again_s: float = ASK_AGAIN_S, read_at_start: bool = True):
        import zenoh

        self.session, self.address = session, address
        self.lock = threading.RLock()
        self.reader = Reader(held, named)
        self.naming = Naming(named, int(ask_again_s * 1e9))
        self._get_timeout = get_timeout
        self.owner_heartbeats: list[OwnerHeartbeatRecord] = []
        self.closed = False
        seed = set(seed)
        self.subs: dict[str, Subscriber] = {}
        pending = []
        for canonical, fingerprint, follows in revisions:
            iface = canonical["interface"]
            self.subs[iface] = Subscriber(session, address, canonical, fingerprint, follows, seed=False,
                                          reread=reread, get_timeout=get_timeout, wait_s=0.0, reader=self.reader)
            if iface in seed:
                pending.append(iface)
        system, service = address.split("/")
        self.keys = {cls: owner_heartbeat_key(system, service, cls) for cls in self.reader.followed_classes()}
        # The start read is marked under way before a class key is subscribed:
        # an owner heartbeat heard before it returns asks for nothing.
        start = read_at_start and self.naming.begin()
        self.class_subs = [session.declare_subscriber(key, zenoh.handlers.Callback(
            lambda sample, cls=cls: self._owner_beat(cls, sample))) for cls, key in self.keys.items()]
        if start:
            self._name(None)
        if wait_s:
            time.sleep(wait_s)
        for iface in pending:
            for resource in list(self.subs[iface].selectors):
                self.subs[iface]._read(resource, seed=True)

    def _name(self, run: str | None) -> None:
        """One read of the owner's descriptors, for ``run`` (None at the
        start, which asks for no run and so fails for none; the caller has
        marked it under way, :meth:`Naming.begin`)."""
        try:
            got = descriptors_of(self.session, self.address, self._get_timeout)
        except Exception:  # noqa: BLE001 - a read that fails names no catalog
            got = []
        with self.lock:
            self.naming.answer(run, got, time.monotonic_ns())

    def _owner_beat(self, cls: str, sample: Any) -> None:
        at = time.monotonic_ns()
        if "*" in str(sample.key_expr):
            return  # core R6
        payload = sample.payload.to_bytes()
        due: list[tuple[str, str]] = []
        name = False
        stamp = None
        try:
            stamp = zk2c.parse(payload, "put").stamp
        except zk2c.Refused:
            pass
        run = None if stamp is None else stamp.run_hex()
        with self.lock:
            # "The descriptor is the one that states the put's run": the
            # catalog this put is read through is its run's, or none yet
            self.reader.named = self.naming.of(run)
            # each view is its subscriber's, mutated under that subscriber's
            # lock by the puts it hears: hold them all for the one read
            locks = [self.subs[i].lock for i in sorted(self.subs)]
            for lk in locks:
                lk.acquire()
            try:
                answer = self.reader.owner_heartbeat_put(cls, payload, at,
                                                         enveloped=zk2c.envelope_encoding(sample.encoding))
            finally:
                for lk in reversed(locks):
                    lk.release()
            self.owner_heartbeats.append(OwnerHeartbeatRecord(at, cls, stamp, payload, answer))
            for g in answer.get("read", {}).get("groups", []):
                sub = self.subs.get(g.get("iface", ""))
                if sub is None:
                    continue
                with sub.lock:
                    for sec in (g.get("sections") or {}).values():
                        if isinstance(sec, dict):
                            for k in sec["confirmed"]:
                                sub.last_conf[k] = at
                                h = sub.heard.get(k)
                                if h is not None:
                                    h.last_confirmation = max(h.last_confirmation, at)
                    if sub.reread_enabled:
                        mine = [r for r in sub.view.reread_due() if r not in sub._reading]
                        sub._reading.update(mine)
                        due += [(sub.layout.interface, r) for r in mine]
            # only a put the envelope's parse accepted states a run to ask for
            name = "refused" not in answer and self.naming.due(run, at)
        if name:
            threading.Thread(target=self._name, args=(run,), daemon=True).start()
        for iface, resource in due:
            threading.Thread(target=self.subs[iface]._read, args=(resource,), daemon=True).start()

    def answers(self, cls: str | None = None, since_ns: int = 0) -> list[OwnerHeartbeatRecord]:
        with self.lock:
            return [h for h in self.owner_heartbeats if h.arrival_ns >= since_ns and (cls is None or h.cls == cls)]

    def judge(self, key: str, now_ns: int | None = None) -> freshness.Verdict:
        iface = key.split("/")[3]
        return self.subs[iface].judge(key, now_ns)

    def close(self) -> None:
        self.closed = True
        for s in self.class_subs:
            try:
                s.undeclare()
            except Exception:  # noqa: BLE001 - closing anyway
                pass
        for sub in self.subs.values():
            sub.close()


def _collection_key(prefix: str, token: str, template: str) -> str | None:
    """Core §4.2: a collection's snapshot key, "its resource's key up to the
    template's first parameter", or None where its first chunk is one."""
    lit: list[str] = []
    for chunk in template.split("/"):
        if chunk.startswith("{"):
            break
        lit.append(chunk)
    return f"{prefix}/{token}/{'/'.join(lit)}" if lit else None


def _collection_read(prefix: str, token: str, template: str) -> str | None:
    """Core §4.2: a collection read, "the snapshot key followed by ``/**``"."""
    key = _collection_key(prefix, token, template)
    return None if key is None else f"{key}/**"


def _selector(template: str) -> str:
    """A template's selector: each parameter a wildcard, a rest ``**``."""
    return "/".join("**" if p.endswith("...}") else "*" if p.startswith("{") else p for p in template.split("/"))


@dataclass
class OwnerView:
    """What the owner holds of one member for its heartbeat: live, its
    in-band stamp, and whether it vouches for it."""

    key: str
    stamp: zk2c.InBand | None
    vouched: bool = True


def owner_body(layout: heartbeat.Layout, exposed: set[str], members: dict[str, OwnerView],
               time_value: dict[str, int] | None, vouch: bool = True) -> dict[str, Any]:
    """The value of the body an owner writes (§2.4): each exposed collection's
    map bit set, its digest over the members it holds live, and a bit for
    each member it vouches for; with ``vouch`` False (the clock guard,
    §2.10) none."""
    sections: dict[str, Any] = {}
    for c in layout.collections:
        if c.resource not in exposed:
            continue
        mine = sorted((m for m in members.values() if _member_of(layout, c, m.key)),
                      key=lambda m: m.key.encode("utf-8"))
        d = heartbeat.digest((m.key, m.stamp.width, m.stamp.run, m.stamp.seq) for m in mine if m.stamp is not None)
        if c.templated:
            sections[c.resource] = {"digest": f"{d:08x}", "live": len(mine),
                                    "vouched": [i for i, m in enumerate(mine) if vouch and m.vouched]}
        else:
            sections[c.resource] = {"digest": f"{d:08x}", "vouched": bool(mine and vouch and mine[0].vouched)}
    return {"time": time_value, "sections": sections}


def _member_of(layout: heartbeat.Layout, c: heartbeat.Collection, key: str) -> bool:
    chunks = key.split("/")
    return len(chunks) >= 6 and chunks[3] == layout.interface and chunks[4] == c.token \
        and templates.match(templates.parse_template(c.template), chunks[5:]) is not None


def revision_of(session: Any, iface: str, fingerprint: str, timeout: float = 1.0) -> dict[str, Any] | None:
    """The canonical form of the revision ``fingerprint`` of ``iface`` that
    a descriptor names (core §3.3), so that a reader knows whether its owner
    confirms by heartbeat (``alarms.v1`` §2.15, 0.11: "an owner whose
    revision of alarms.v1 uses freshness.v2, as its descriptor names it").
    A revision published under ``spec/profiles/.history`` or
    ``examples/zk2/.history`` is read there; any other is retrieved from its
    holders (core §8.4). None where neither gives it."""
    from pathlib import Path

    from . import bundle, live

    hexd = fingerprint.removeprefix("sha256:")
    repo = Path(__file__).resolve().parents[3]
    for root in (repo / "spec" / "profiles" / ".history", repo / "examples" / "zk2" / ".history"):
        f = root / iface / f"{hexd}.bundle.json"
        if f.is_file():
            try:
                return bundle.verify(f.read_bytes(), fingerprint).contract
            except Exception:  # noqa: BLE001 - a bundle that does not verify gives nothing
                return None
    if session is None:
        return None
    got = live.retrieve_bundle(session, iface, fingerprint, timeout=timeout)
    v = getattr(got, "verified", None)
    return None if v is None else v.contract


def descriptor_revision(session: Any, address: str, iface: str,
                        timeout: float = 1.0) -> tuple[dict[str, Any], str] | None:
    """The revision of ``iface`` ``address``'s present instance names in its
    descriptor, as (canonical, fingerprint), or None (no instance read, no
    such interface, the revision not in hand)."""
    import json

    from . import live

    pres = live.list_presence(session, f"zk2/{address}/@zk/instance/*")
    if not pres.instances:
        return None
    d = live.get_descriptor(session, f"zk2/{address}/@zk/instance/{pres.instances[0]['instance']}", timeout=timeout)
    if len(d) != 1 or not d[0].ok:
        return None
    entry = next((e for e in json.loads(d[0].payload).get("interfaces") or [] if e.get("iface") == iface), None)
    fp = None if entry is None else entry.get("contract")  # core §3.3: the revision's fingerprint
    if not fp:
        return None
    canonical = revision_of(session, iface, fp, timeout)
    return None if canonical is None else (canonical, fp)


def uses_v2(canonical: dict[str, Any] | None) -> bool:
    """Whether a revision uses ``freshness.v2`` (its owners confirm by
    heartbeat, never by a re-put)."""
    return canonical is not None and PROFILE in canonical.get("uses", [])

"""A conditional read's reader (core §4.2, 0.110, #986): the held stamp it
may send, and its merge of each answer. Written from §4.2 before
``conformance/seed/held.json`` was opened (round 116).

**What a reader sends** (:func:`ask`). "A consumer or a tool MUST NOT send a
held stamp other than the in-band stamp of the mutation it holds for that
member, as the owner wrote it."

- "That stamp is its put's envelope's, its delete's attachment's, or its
  snapshot entry's: the entry's own run or the header's, and the entry's
  seq."
- "Never a snapshot's asof, a Zenoh timestamp, nor another key's stamp. A
  reader that holds nothing for the member sends none, and neither does one
  that holds a value with no envelope."

zk2py's readers hold each value with the in-band stamp it arrived under,
a snapshot entry's resolved as it merged (its own run or the header's), so
the stamp to send is the held value's own and nothing is recomputed. A
conditional read "is a member read of one concrete key", so a reader sends
none on any other selector: an owner would ignore it there.

**A held stamp in answer** (:meth:`HeldSeed.arrive_held`). "A held stamp is
the owner's answer only as a reply to the GET that carried the same one, on
that GET's one concrete key. The same one has the same run, in width and
value, and the same seq. A consumer or a tool MUST NOT read any other held
stamp as unchanged." "The others are a held stamp that differs from the one
its GET carried, one in reply to a GET that carried none, one on another
key, and one heard on a subscription. Such a held stamp establishes
nothing. The reader merges nothing for it. It counts it undecodable,
refused as ``held``."

**What unchanged says.** "The reader keeps what it holds: a held stamp
carries no value, and merges none."

- "Where the reader still holds that mutation, the answer counts as the
  mutation heard again, by the GET. The omission rule's guard reads it so":
  the value is marked heard after the GET was asked, so another instance's
  complete snapshot of no entry, which arrives after it, spares it.
- "Where the reader holds another value by then, heard while its GET was
  out, it keeps that value. An answer never steps a member back."
- "Where the reader holds nothing for the member by then, its GET gave it
  no value. It reads again, with no held stamp."

**"By then".** zk2py reads it as the answer's arrival: §4.2 merges "each as
its page arrives", and "heard while its GET was out" is what came before the
answer did. So a snapshot of no entry that arrives *before* the held stamp
drops the value (it was not yet heard again), the held stamp then finds
nothing held, and the reader reads again; one that arrives *after* spares
it. The text names no instant, and the reading at the GET's end differs
only where something more arrives between the answer and the end.

**A snapshot in answer.** "A consumer or a tool that sent a held stamp MUST
accept a snapshot in answer, and merge it as any member read's":
:class:`zk2py.seed_snapshot.Seed`'s merge, unchanged.
"""

from __future__ import annotations

from typing import Any

from . import held, snapshot
from .seed_snapshot import Seed, _stamp
from .zk2c import InBand

UNCHANGED = "unchanged"
#: "It counts it undecodable, refused as `held`."
REFUSED = held.HELD


def ask(selector: str, holds: InBand | None) -> InBand | None:
    """The held stamp a reader may send on a GET of ``selector``, holding
    for that member a value under the in-band stamp ``holds`` (its put's
    envelope's, its delete's attachment's or its snapshot entry's), or None
    where it holds nothing for the member or "a value with no envelope".
    None on a selector that is not one concrete key."""
    if holds is None or not held.one_concrete_key(selector):
        return None
    return holds


def entry_stamp(header: dict[str, Any], entry: dict[str, Any]) -> InBand:
    """A snapshot entry's stamp, as a reader that holds the entry sends it:
    "the entry's own run or the header's, and the entry's seq", "never a
    snapshot's asof". ``header`` and ``entry`` are :mod:`zk2py.snapshot`'s
    page and entry."""
    if entry.get("run_width") is not None:
        return InBand(entry["run_width"], entry["run"], entry["seq"])
    return InBand(header["run_width"], header["run"], entry["seq"])


def reading(asked_key: str | None, asked: InBand | None, key: str, via: str, stamp: InBand) -> str:
    """One held stamp that arrived on ``key`` by ``via`` (``get``, a reply
    to the GET on ``asked_key`` that carried ``asked``, or anything else for
    a subscription): ``unchanged`` or ``held``."""
    if via == "get" and asked is not None and asked_key == key and held.same(asked, stamp):
        return UNCHANGED
    return REFUSED


class HeldSeed(Seed):
    """:class:`zk2py.seed_snapshot.Seed` with the GET it asked: its one key
    and the held stamp it carried, or None. ``readings`` holds each held
    stamp's reading as it arrived, and ``again`` whether the reader reads
    again, with no held stamp."""

    def __init__(self, collections: list[dict[str, Any]], view: list[dict[str, Any]], asked_key: str | None,
                 asked: InBand | None):
        super().__init__(collections, view)
        self.asked_key, self.asked = asked_key, asked
        self.readings: list[dict[str, Any]] = []
        self.again = False

    def arrive_held(self, key: str, via: str, stamp: InBand) -> str:
        got = reading(self.asked_key, self.asked, key, via, stamp)
        self.readings.append({"key": key, "via": via, "reads": got})
        if got != UNCHANGED:
            return got
        mine = self.view.get(key)
        if mine is None:
            # "Nothing held … its GET gave it no value. It reads again, with
            # no held stamp."
            self.again = True
        elif mine.stamp == (stamp.width, stamp.run, stamp.seq):
            # "the answer counts as the mutation heard again, by the GET"
            mine.after_get = True
        # else "A later value stands": nothing moves
        return got

    def arrive(self, a: dict[str, Any]) -> None:
        """One arrival: a sample or a page (the base class's), or a GET
        reply's or a put's payload that §7.5 reads as a held stamp."""
        if "hex" in a and "sample" not in a:
            r = held.read_reply(bytes.fromhex(a["hex"]), "state")
            if r.held:
                assert r.stamp is not None
                self.arrive_held(a["key"], a["via"], r.stamp)
                return
            if r.refusal is not None and len(bytes.fromhex(a["hex"])) <= held.size(32) and r.envelope is not None:
                # a held stamp with a tag: refused `tag`, merged for nothing
                self.readings.append({"key": a["key"], "via": a["via"], "reads": r.refusal})
                return
        super().arrive(a)

    def result(self) -> dict[str, Any]:
        out = super().result()
        out["held"] = list(self.readings)
        out["again"] = self.again
        return out


def stamp_of(x: dict[str, Any] | None) -> InBand | None:
    """A fixture's ``{width, run, seq}`` as an in-band stamp, or None."""
    if x is None:
        return None
    w, r, s = _stamp(x)
    return InBand(w, r, s)


__all__ = ["ask", "entry_stamp", "reading", "HeldSeed", "stamp_of", "UNCHANGED", "REFUSED", "snapshot"]

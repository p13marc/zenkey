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
from dataclasses import dataclass
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
             deleted: Iterable[tuple[str, zk2c.InBand | None]] = ()) -> None:
        """A complete read's answer of ``resource``, asked at ``asked`` and
        arrived at ``at``: "The answer MUST then replace its view of the
        collection", "merged with what the subscriber heard of the
        collection after it asked … Within one run the higher seq is kept.
        Across two runs the answer's value stands." "A member the answer left
        out leaves the view". A ``reply_del`` is a deletion with its stamp.
        Every member of the collection loses its heartbeat confirmation, and
        the collection is no longer due a read."""
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
            sec = value["sections"].get(c.resource)
            if sec is None:
                sections[c.resource] = "absent"
                self.streak[c.resource] = 0
                continue
            keys = self.live_keys(c)
            mine = self.digest_of(c)
            if mine is None or mine != int(sec["digest"], 16) or (c.templated and sec["live"] != len(keys)):
                sections[c.resource] = "diverged"
                self.streak[c.resource] += 1
                continue
            self.streak[c.resource] = 0
            if c.templated:
                confirmed = [keys[r] for r in sec["vouched"]]
            else:
                confirmed = keys if sec["vouched"] and keys else []
            for k in confirmed:
                self.members[k].confirmed = at
            sections[c.resource] = {"confirmed": confirmed}
        return {"read": {"gap": gap, "sections": sections}}

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
                 get_timeout: float = 1.0, wait_s: float = 0.2):
        import zenoh

        self.session = session
        self.address = address
        self.layout = heartbeat.layout_of(canonical, fingerprint)
        self.follows = list(follows) if follows is not None else [c.resource for c in self.layout.collections]
        self.view = View(self.layout, self.follows, address)
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
        self.closed = False
        self.declared_ns = time.monotonic_ns()
        prefix = f"zk2/{address}/{self.layout.interface}"
        self.selectors = {c.resource: f"{prefix}/{c.token}/{_selector(c.template)}"
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
            env = zk2c.parse(sample.payload.to_bytes(), "put")
        except zk2c.Refused as e:
            with self.lock:
                self.heartbeats.append(HeartbeatRecord(at, None, None, None, {"refused": e.token}, refusal=e.token))
            return
        if env.form != "heartbeat":
            with self.lock:
                self.heartbeats.append(HeartbeatRecord(at, env.stamp, env.tag, env.record,
                                                       {"refused": "form"}, refusal=f"a {env.form}"))
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
            st = live.get_state(self.session, self.selectors[resource], timeout=self.get_timeout)
            arrived = time.monotonic_ns()
            if not st.replies and not st.complete:
                return
            ok = [(r.key, r.inband) for r in st.replies if not r.deleted and r.refusal is None]
            gone = [(r.key, r.inband) for r in st.replies if r.deleted]
            with self.lock:
                self.view.read(c.resource, asked, arrived, ok, gone)
                (self.seeds if seed else self.reads).append((c.resource, asked, arrived, len(ok)))
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

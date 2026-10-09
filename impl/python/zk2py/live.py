"""The live half, first slice: presence, descriptors and contract retrieval
over Zenoh (core.md §3.3, §8.1–§8.4), on zenoh-python 1.10.1.

Everything here is a *tool*'s side of the rules: it reads the bus, it never
owns keys. The static half supplies the checks: :mod:`zk2py.keys` parses
token keys, :mod:`zk2py.descriptor` checks a descriptor, :mod:`zk2py.bundle`
verifies a bundle.

**Reading presence (§8.1, as amended by 0.5).** "A caller or tool's
liveliness GET on a session that holds a liveliness subscriber MUST use a
callback or an unbounded handler", and "every liveliness subscriber on that
session MUST be callback-driven, or drained as its samples arrive".
zenoh-python has no unbounded channel: its default handler and
``FifoChannel`` are bounded, and ``RingChannel`` drops. So:
- every liveliness GET here passes a ``zenoh.handlers.Callback`` (stable
  API, the default ``indirect`` mode), whose callback only appends to a
  Python list and whose drop function marks completion;
- any liveliness subscriber zk2py declares is a ``Callback`` too;
- "a liveliness GET that ended at its timeout, rather than at the routers'
  final reply" is reported as possibly incomplete (``complete=False``).
  0.8 says how to see it: "a GET that reached its timeout ends with an
  error reply, Timeout, and one the routers finished ends with none … A
  read that received any error reply is possibly incomplete". So
  completeness is read from the error replies, not from elapsed time;
- a complete read is what *this reader* could see: "A refused read is
  complete, and empty" (§8.1, 0.8), so an empty one never rules out a
  refusal by access control (:attr:`Presence.reading`);
- presence is polled with GETs, never inferred from silence (§3.2 R7, O5).

**Timeouts are the caller's (§8.1).** The defaults here are the 1 s that
"the scenarios, and so a conformance run, use".
"""

from __future__ import annotations

import json
import queue
import re
import threading
import time
from dataclasses import dataclass, field
from typing import Any

import zenoh

from . import bundle, keys

#: Liveliness GET timeout: the caller's choice, 1 s in a conformance run (§8.1).
PRESENCE_TIMEOUT_S = 1.0
#: GET timeout for a descriptor or a bundle attempt: likewise (§3.3, §8.4).
GET_TIMEOUT_S = 1.0
_DONE = object()


def open_client(endpoint: str, auth: tuple[str, str] | None = None) -> zenoh.Session:
    """A *client* session to one router endpoint, multicast scouting off: a
    tool that sees exactly the router it is pointed at. ``auth`` is a
    usrpwd (user, password), the principal's binding (§11.3)."""
    conf = zenoh.Config()
    conf.insert_json5("mode", json.dumps("client"))
    conf.insert_json5("connect/endpoints", json.dumps([endpoint]))
    conf.insert_json5("scouting/multicast/enabled", "false")
    if auth is not None:
        conf.insert_json5("transport/auth/usrpwd", json.dumps({"user": auth[0], "password": auth[1]}))
    return zenoh.open(conf)


# -- §8.1 presence -------------------------------------------------------------

@dataclass
class Presence:
    """The tokens a liveliness GET returned, parsed by form (§1.1)."""

    instances: list[dict[str, Any]] = field(default_factory=list)
    alive: list[dict[str, Any]] = field(default_factory=list)
    members: list[dict[str, Any]] = field(default_factory=list)
    #: keys under the selector that are not zk2 control keys
    other: list[str] = field(default_factory=list)
    #: the GET ended at the routers' final reply with no error reply
    complete: bool = False
    #: its error replies, "<encoding>: <payload>" (``zenoh/string: Timeout``)
    errors: list[str] = field(default_factory=list)
    elapsed_s: float = 0.0

    @property
    def count(self) -> int:
        return len(self.instances) + len(self.alive) + len(self.members) + len(self.other)

    @property
    def reading(self) -> str:
        """How a tool states this read (§8.1, 0.8): "A tool reports absence
        as what its reader could see. Where it cannot rule out a refusal, it
        SHOULD say so, as it says a read is possibly incomplete." A reader
        can never rule one out from the read itself (the CHANGELOG's "No
        probe that tells a refusal from absence")."""
        if not self.complete:
            why = "; ".join(self.errors) or "no final reply"
            return f"possibly incomplete ({why}): {self.count} tokens, silence is not a verdict"
        if self.count == 0:
            return "complete and empty for this reader: absent, or refused by access control"
        return f"complete for this reader: {self.count} tokens"


def list_presence(session: zenoh.Session, selector: str = "zk2/*/*/@zk/**",
                  timeout: float = PRESENCE_TIMEOUT_S) -> Presence:
    """Every liveliness token under ``selector``, by a liveliness GET through
    a callback handler (§8.1).

    ``**`` never crosses a verbatim chunk (§1.3), so the selector names
    ``@zk`` literally: ``zk2/*/*/@zk/**`` is every service's control
    tokens, and ``zk2/*/*/@zk/alive/nav.v2/**`` is "who implements nav.v2"
    (§8.1).
    """
    replies: list[Any] = []
    done = threading.Event()
    t0 = time.monotonic()
    session.liveliness().get(selector, zenoh.handlers.Callback(replies.append, done.set),
                             timeout=timeout)
    finished = done.wait(timeout + 5.0)
    p = Presence(elapsed_s=time.monotonic() - t0)
    for r in replies:
        if r.ok is None:
            p.errors.append(f"{r.err.encoding}: {r.err.payload.to_bytes().decode('utf-8', 'replace')}")
            continue
        key = str(r.ok.key_expr)
        parsed = keys.parse(key)
        form = parsed["form"] if parsed else None
        if form == "instance":
            p.instances.append(parsed)
        elif form == "alive":
            p.alive.append(parsed)
        elif form == "member":
            p.members.append(parsed)
        else:
            p.other.append(key)
    # §8.1 (0.8): ended by the routers' final reply, and no error reply. A
    # GET whose drop never came is incomplete too.
    p.complete = finished and not p.errors
    return p


# -- GET plumbing ----------------------------------------------------------------

@dataclass
class Answer:
    """One GET reply, as zk2py sees it."""

    ok: bool
    key: str | None
    encoding: str
    payload: bytes
    has_timestamp: bool = False
    has_attachment: bool = False
    #: the zid of the session that sent the reply, "whatever key the reply
    #: is on" (Appendix B, 0.12), or None when it cannot be read
    replier: str | None = None


def replier_of(reply: Any) -> str | None:
    """A reply's replier id, as a zid's text, or None.

    zenoh-python 1.10.1 exposes ``Reply.replier_id``, an ``EntityGlobalId``
    whose ``zid`` is the replying session's. Its type stub marks it
    ``@_unstable``, a marker only: the published wheel has it at run time.
    It is Rust's ``Reply::replier_id``, behind the ``unstable`` feature
    (Appendix B), so a binding built without it would have no attribute,
    which reads as None here. Appendix B (0.16): "a zenoh release that
    removed or changed it would leave every admin answer unverified, and
    the checks that read the admin space unobservable, never clean".
    :data:`READ_REPLIER` False simulates that release."""
    if not READ_REPLIER:
        return None
    try:
        rid = getattr(reply, "replier_id", None)
        return None if rid is None else str(rid.zid)
    except Exception:  # noqa: BLE001 - an unstable accessor that fails is no id
        return None


#: False simulates a binding without ``Reply.replier_id`` (Appendix B, 0.16).
READ_REPLIER = True


def _answers(session: zenoh.Session, selector: str, target: zenoh.QueryTarget,
             timeout: float):
    """Yield each reply *as it arrives* (an unbounded queue fed by a
    callback), until the GET completes.

    Consolidation is ``None``, which §8.4 (0.5) makes a MUST on both
    retrieval attempts, and §3.3 sets for the descriptor GET: zenoh's
    default holds every reply until the query finalizes, and a slow corrupt
    reply can displace the valid one.
    """
    q: queue.Queue = queue.Queue()
    session.get(selector, zenoh.handlers.Callback(q.put, lambda: q.put(_DONE)),
                target=target, consolidation=zenoh.ConsolidationMode.NONE, timeout=timeout)
    while True:
        try:
            item = q.get(timeout=timeout + 5.0)
        except queue.Empty:
            return
        if item is _DONE:
            return
        if item.ok is not None:
            s = item.ok
            yield Answer(True, str(s.key_expr), str(s.encoding), s.payload.to_bytes(),
                         s.timestamp is not None, s.attachment is not None, replier_of(item))
        else:
            e = item.err
            yield Answer(False, None, str(e.encoding), e.payload.to_bytes(), replier=replier_of(item))


# -- §3.3 the descriptor ---------------------------------------------------------

def get_descriptor(session: zenoh.Session, instance_key: str,
                   timeout: float = GET_TIMEOUT_S) -> list[Answer]:
    """GET an instance's descriptor (§3.3 "The GET", 0.5): one complete
    queryable answers "with one reply … Encoding application/json, no
    attachment and no timestamp"; a caller "GETs with consolidation None …
    takes the first reply". The target is the caller's (CHANGELOG 0.5);
    zk2py uses BestMatching. Every reply is returned; the caller judges them.

    §3.2 R6: a reply whose key is not concrete is discarded here.
    """
    out = []
    for a in _answers(session, instance_key, zenoh.QueryTarget.BEST_MATCHING, timeout):
        if a.ok and (a.key is None or "*" in a.key):
            continue
        out.append(a)
    return out


# -- §8.4 contract retrieval -----------------------------------------------------

@dataclass
class Attempt:
    target: str
    #: (accepted?, refusal tag or "accepted", the reply's encoding)
    replies: list[tuple[bool, str, str]] = field(default_factory=list)


@dataclass
class Retrieval:
    """The outcome of §8.4's procedure for one bundle."""

    key: str
    data: bytes | None = None  # the accepted, verified bundle bytes
    verified: bundle.Verified | None = None
    attempts: list[Attempt] = field(default_factory=list)

    @property
    def available(self) -> bool:
        return self.data is not None


def fingerprint_of(descriptor: dict[str, Any], iface: str, fp16: str | None = None) -> str | None:
    """§8.4 (0.8) "From a token to a fingerprint": "An interface token
    carries fp16 …, which is not enough to retrieve by. A tool reads the
    full fingerprint from the instance's descriptor (§3.3, the interface's
    contract), and retrieves by that. There is no retrieval by prefix."

    The interface's ``contract`` in a descriptor, or None when it lists no
    such interface, or, given the token's ``fp16``, when that is not the
    start of it (§1.2)."""
    for e in descriptor.get("interfaces", []):
        if e.get("iface") != iface or not isinstance(e.get("contract"), str):
            continue
        fp = e["contract"]
        if fp16 is not None and not fp.removeprefix("sha256:").startswith(fp16):
            return None
        return fp
    return None


def contract_key(iface: str, fingerprint: str) -> str:
    """§8.4: "the location-free key ``zk2/@zk/contract/<iface>/<sha256>``";
    §1.2: only the 64 hex digits are written."""
    return f"zk2/@zk/contract/{iface}/{fingerprint.removeprefix('sha256:')}"


def retrieve_bundle(session: zenoh.Session, iface: str, fingerprint: str,
                    timeout: float = GET_TIMEOUT_S) -> Retrieval:
    """Retrieve a contract bundle exactly as §8.4 says:

    1. GET with target ``BestMatching``;
    2. verify each reply as it arrives (§9.6, against the expected
       fingerprint), and accept the first valid one without waiting for the
       GET to complete;
    3. if none was valid, retry once with target ``All``;
    4. if still none, report the contract unavailable. Never accept an
       unverified bundle.
    """
    key = contract_key(iface, fingerprint)
    r = Retrieval(key)
    for target, name in ((zenoh.QueryTarget.BEST_MATCHING, "BestMatching"),
                         (zenoh.QueryTarget.ALL, "All")):
        attempt = Attempt(name)
        r.attempts.append(attempt)
        for a in _answers(session, key, target, timeout):
            if not a.ok:
                attempt.replies.append((False, "reply_err", a.encoding))
                continue
            if a.key != key:
                # §3.2 R6, and §8.4: a holder answers on the contract key.
                attempt.replies.append((False, "wrong_key", a.encoding))
                continue
            try:
                v = bundle.verify(a.payload, expect_fingerprint=fingerprint)
            except bundle.BundleError as e:
                attempt.replies.append((False, e.tag, a.encoding))
                continue
            attempt.replies.append((True, "accepted", a.encoding))
            r.data, r.verified = a.payload, v
            return r
    return r


# -- §8.1 presence shapes, read twice (0.10) --------------------------------------

#: The grace between the two reads of a presence shape: "longer than the
#: deployment's longest re-mint overlap" (§8.1, 0.10). The caller's choice;
#: 1 s here, as every other wait of a conformance run.
SHAPE_GRACE_S = 1.0


def presence_shapes(session: zenoh.Session, selector: str = "zk2/*/*/@zk/**",
                    timeout: float = PRESENCE_TIMEOUT_S) -> set[tuple[str, str, str]]:
    """The presence shapes §8.1 (0.10) names as faults, as read once:
    ``(instance key, what, interface)`` for
    - ``unlisted``: an interface token its instance's descriptor does not
      list;
    - ``no token``: an interface the descriptor lists with ``token`` true
      and no interface token.

    An instance whose descriptor GET gets no reply has no shape here: that
    silence is not a verdict (O5). "Exposed" is read from the descriptor
    alone, as listed with ``token`` not false: an instance that exposes
    none of a listed interface's resources holds no token for it (§5.1
    "The active instance"), which only the contract can show, and zk2py
    does not retrieve it here."""
    p = list_presence(session, selector, timeout)
    alive: dict[str, set[str]] = {}
    for a in p.alive:
        alive.setdefault(f"zk2/{a['system']}/{a['service']}/@zk/instance/{a['instance']}", set()).add(a["iface"])
    instances = {f"zk2/{i['system']}/{i['service']}/@zk/instance/{i['instance']}" for i in p.instances}
    shapes: set[tuple[str, str, str]] = set()
    for key in sorted(instances | set(alive)):
        answers = [a for a in get_descriptor(session, key) if a.ok]
        if len(answers) != 1:
            continue
        try:
            doc = json.loads(answers[0].payload)
        except ValueError:
            continue
        listed = {e.get("iface"): e for e in doc.get("interfaces", []) if isinstance(e, dict)}
        for iface in sorted(alive.get(key, set()) - set(listed)):
            shapes.add((key, "unlisted", iface))
        for iface, e in sorted(listed.items()):
            if e.get("token", True) is not False and iface not in alive.get(key, set()):
                shapes.add((key, "no token", str(iface)))
    return shapes


def presence_faults(session: zenoh.Session, selector: str = "zk2/*/*/@zk/**",
                    grace_s: float = SHAPE_GRACE_S) -> tuple[set, set]:
    """§8.1 (0.10): "A tool that decides a fault from the shape of presence
    … MUST see it in two reads a grace apart … Start-up, re-mint and
    teardown pass through such shapes briefly, by design."

    Returns (faults: the shapes both reads saw, passing: the shapes only
    the first saw)."""
    first = presence_shapes(session, selector)
    time.sleep(grace_s)
    second = presence_shapes(session, selector)
    return first & second, first - second


# -- §3.3 meta.zid: whose stamp (0.10) -------------------------------------------

def attribute_stamp(stamp_id: str | None, descriptor: dict[str, Any]) -> str:
    """S1's attribution by a tool (§3.3 ``meta``, 0.10): ``owner`` when a
    state stamp's id is the owner's ``meta.zid``, ``foreign`` when it is
    another, ``unattributable`` when the descriptor states no zid ("Without
    it, a stamp's clock is unattributable, never foreign"), and
    ``unstamped`` for a reply with no timestamp.

    §3.3 (0.11) "How a zid compares": "A tool MUST compare two zids by
    value, never by their text: case and leading zeros carry no meaning."
    A zid is hexadecimal (Appendix B), so its value is that number. A
    ``meta.zid`` that is not hexadecimal states no zid: unattributable."""
    if stamp_id is None:
        return "unstamped"
    meta = descriptor.get("meta")
    zid = meta.get("zid") if isinstance(meta, dict) else None
    value = _zid_value(zid)
    if value is None:
        return "unattributable"
    return "owner" if _zid_value(stamp_id) == value else "foreign"


def _zid_value(text: Any) -> int | None:
    """A zid's value, or None for text that is not hexadecimal."""
    if not isinstance(text, str) or re.fullmatch(r"[0-9a-fA-F]+", text) is None:
        return None
    return int(text, 16)


# -- §4.2 S4, through the routers' admin space (0.10 to 0.12) ---------------------

#: §4.2 (0.11) "What the check reads".
S4_ROUTERS = "@/*/router"
S4_STORAGES = "@/*/router/**/storage_manager/storages/**"
#: "an owner's state/** or @state/**", for every owner.
S4_STATE = ("zk2/*/*/*/state/**", "zk2/*/*/*/@state/**")


@dataclass
class S4Reading:
    """A tool's S4 check (§4.2): ``clean``, ``broken`` (a verified storage
    answers on owners' state) or ``unobservable`` ("Without it, a tool
    reports the check unobservable, never clean"; and an unverified answer
    "never contributes to a clean verdict", 0.12)."""

    verdict: str
    #: the routers whose answer is verified (0.12, "Who answered")
    routers: list[str] = field(default_factory=list)
    #: (router zid, storage key, its key_expr, intersects owners' state?)
    #: for each verified storage
    storages: list[tuple[str, str, Any, bool]] = field(default_factory=list)
    #: each verified router's `plugins`, read beside the storages
    plugins: dict[str, Any] = field(default_factory=dict)
    #: (key, replier id or None, why) for every answer not verified: unjudged
    unverified: list[tuple[str, str | None, str]] = field(default_factory=list)
    #: the operator told the tool to trust every answer (0.12)
    trusted: bool = False
    detail: str = ""


def _self_consistent(a: Answer) -> bool:
    """The reply's replier id is the zid its key names (§4.2, 0.12), by
    value (0.11)."""
    if a.key is None or a.replier is None:
        return False
    named = _zid_value(a.key.split("/")[1])
    return named is not None and _zid_value(a.replier) == named


def verified_routers(session: zenoh.Session, records: list[Answer]) -> set[int]:
    """§4.2 (0.13) "Verified routers, outward": "the routers the tool's
    session is connected to, and the session itself, are verified; so is
    every zid a verified router's own answer lists among its sessions with
    whatami router; and so on, until no new router is verified."

    ``records`` are the answers to ``@/*/router``. A router's "own answer"
    is a self-consistent one on its key. Appendix B (0.13): its document
    "lists its sessions, each with the peer's zid and whatami". Returns
    zid values."""
    verified = {_zid_value(str(z)) for z in session.info.routers_zid()} | {_zid_value(str(session.zid()))}
    verified.discard(None)
    while True:
        grown = set()
        for a in records:
            if not _self_consistent(a) or _zid_value(a.key.split("/")[1]) not in verified:
                continue
            try:
                doc = json.loads(a.payload)
            except ValueError:
                continue
            for s in doc.get("sessions", []) if isinstance(doc, dict) else []:
                if isinstance(s, dict) and s.get("whatami") == "router":
                    z = _zid_value(s.get("peer"))
                    if z is not None and z not in verified:
                        grown.add(z)
        if not grown:
            return verified
        verified |= grown


def unverified_why(a: Answer, verified: set[int], storage: bool = False) -> str | None:
    """§4.2 (0.12, 0.13) "Who answered": "A tool counts an answer as a
    router's only when the reply's replier id is the zid the key names, and
    that zid is a verified router." None when the answer counts, else why
    not, in the reference doctor's terms (0.13):
    - ``no replier id``: the binding gives none, so every answer is held;
    - ``a replier other than its key's router``: the spoof;
    - ``no verified router lists it``: a router record whose zid no
      verified router's own answer lists as a router, such as a client
      answering on its own key;
    - ``its router's own answer unverified``: a storage record under such
      a zid."""
    if a.key is None or a.replier is None:
        return "no replier id"
    if not _self_consistent(a):
        return "a replier other than its key's router"
    if _zid_value(a.key.split("/")[1]) not in verified:
        return "its router's own answer unverified" if storage else "no verified router lists it"
    return None


def check_s4(session: zenoh.Session, timeout: float = GET_TIMEOUT_S, trust: bool = False) -> S4Reading:
    """§4.2 (0.11): "The reference reads two selectors: @/*/router, the
    routers that answer; @/*/router/**/storage_manager/storages/**, one key
    per storage a router's storage manager runs, its value the storage's
    configuration with its key_expr. A storage whose key_expr intersects an
    owner's state/** or @state/** breaks S4. A router that answers the
    first selector and has nothing under the second runs no storage. When
    no router answers the first, the check is unobservable."

    0.12 "Who answered": each answer, router record or storage, counts only
    when :func:`unverified_why` finds nothing, against the routers verified
    outward from this session (0.13, :func:`verified_routers`). "Any other
    answer is unverified, and an unverified answer never contributes to a
    clean verdict." zk2py holds one as unjudged: it never breaks S4 and
    never lets it be clean.
    ``trust`` is the operator's alternative: "An operator MAY tell a tool to
    trust every answer when the deployment's grants deny @/** queryables to
    every principal".

    Beside the selectors, each verified router record's ``plugins`` is
    kept. A verified storage whose configuration has no readable
    ``key_expr`` makes the check unobservable rather than clean."""
    out = S4Reading("unobservable", trusted=trust)
    records = [a for a in _answers(session, S4_ROUTERS, zenoh.QueryTarget.ALL, timeout)
               if a.ok and a.key is not None]
    verified = verified_routers(session, records)
    answered = 0
    for a in records:
        answered += 1
        why = None if trust else unverified_why(a, verified)
        if why is not None:
            out.unverified.append((a.key, a.replier, why))
            continue
        zid = a.key.split("/")[1]
        if zid in out.routers:
            continue  # the same record by another path (a peer reaches a router twice)
        out.routers.append(zid)
        try:
            doc = json.loads(a.payload)
            out.plugins[zid] = doc.get("plugins") if isinstance(doc, dict) else "unreadable"
        except ValueError:
            out.plugins[zid] = "unreadable"
    if not answered:
        out.detail = f"no router answered {S4_ROUTERS}: the admin space is off"
        return out
    unreadable = []
    storages_sel = zenoh.KeyExpr(S4_STORAGES)
    for a in _answers(session, S4_STORAGES, zenoh.QueryTarget.ALL, timeout):
        if not a.ok or a.key is None:
            continue
        # §4.2 (0.15): "A storage is an answer to the second selector whose
        # key the selector includes, ending …/storage_manager/storages/<name>.
        # Other answers arrive too, and are not storages": a router's
        # `router/queryable/<key expr>` record of a `**` queryable (every
        # owner's `state/**`, S2) intersects the selector.
        chunks = a.key.split("/")
        if not storages_sel.includes(zenoh.KeyExpr(a.key)) or chunks[-3:-1] != ["storage_manager", "storages"]:
            continue
        why = None if trust else unverified_why(a, verified, storage=True)
        if why is not None:
            out.unverified.append((a.key, a.replier, why))
            continue
        if any(s[1] == a.key for s in out.storages):
            continue
        try:
            conf = json.loads(a.payload)
            kexpr = conf.get("key_expr") if isinstance(conf, dict) else None
            hits = isinstance(kexpr, str) and any(
                zenoh.KeyExpr(kexpr).intersects(zenoh.KeyExpr(s)) for s in S4_STATE)
        except Exception:  # noqa: BLE001 - not JSON, or not a key expression
            kexpr, hits = None, False
        if not isinstance(kexpr, str):
            unreadable.append(a.key)
        out.storages.append((a.key.split("/")[1], a.key, kexpr, bool(hits)))
    broken = [s for s in out.storages if s[3]]
    if broken:
        out.verdict = "broken"
        out.detail = f"a storage answers on owners' state: {[(z, k) for z, _, k, _ in broken]}"
    elif not out.routers:
        out.detail = f"no answer to {S4_ROUTERS} is verified: {out.unverified}"
    elif out.unverified:
        out.detail = f"verified {out.routers}, but unverified answers stand beside them: {out.unverified}"
    elif unreadable:
        out.detail = f"storages whose key_expr cannot be read: {unreadable}"
    else:
        out.verdict = "clean"
        out.detail = (f"{len(out.routers)} verified router(s), {len(out.storages)} storage(s), none on owners' "
                      f"state; plugins {out.plugins}" + ("; every answer trusted" if trust else ""))
    return out


# -- §4.2 a tool's S1 check (0.16) -----------------------------------------------

def s1_check(session: zenoh.Session, descriptor: dict[str, Any], stamp_id: str | None,
             timeout: float = GET_TIMEOUT_S, trust: bool = False) -> tuple[str, str]:
    """§4.2 "A tool's S1 check" (0.16, 0.17). "A tool attributes a state
    reply's stamp by comparing its id with the owner's meta.zid (§3.3), by
    value":
    - "A foreign stamp is a finding whatever else the tool read: an owner
      that is its own router stamps with that router's id, which is its own
      meta.zid." A reply with no stamp is a finding too (S1, S2).
    - "An owner's stamp is clean only when the tool verified at least one
      router ("Who answered", above) and meta.zid is none of the zids it
      knows to be routers: the routers its session is connected to, the
      routers it verified, and every zid a verified router lists as a
      router session."
    - "Otherwise S1 is unobservable for that owner, never clean."

    Without ``meta.zid`` the stamp is ``unattributable`` (§3.3). "Verified
    at least one router" is read as one router's admin answer verified,
    since 0.17 names the cases where none is: no admin read, the admin
    space off, no replier id. ``trust`` is 0.12's operator alternative.
    Returns (verdict, why)."""
    meta = descriptor.get("meta")
    zid = _zid_value(meta.get("zid") if isinstance(meta, dict) else None)
    if zid is None:
        return "unattributable", "the descriptor states no meta.zid (§3.3)"
    who = attribute_stamp(stamp_id, descriptor)
    if who != "owner":
        return "finding", f"the stamp is {who}: an owner that is its own router stamps with its own meta.zid"
    records = [a for a in _answers(session, S4_ROUTERS, zenoh.QueryTarget.ALL, timeout)
               if a.ok and a.key is not None]
    known = verified_routers(session, records)  # connected, the session, verified and listed
    if trust:
        known |= {_zid_value(a.key.split("/")[1]) for a in records}
    answered = [a for a in records if trust or unverified_why(a, known) is None]
    if zid in known:
        return "unobservable", "meta.zid is a router's zid: the owner is its own router"
    if not answered:
        return "unobservable", ("no router verified (no admin read, the admin space off, or no replier id): "
                                "the owner may be a router this tool cannot see")
    return "clean", f"the stamp is the owner's, and meta.zid is none of {len(known)} known routers"


# -- §5.1 O3 judged from outside (0.16, 0.17) --------------------------------------

def o3_verdict(result: CallResult, may_call: bool | None, present: bool | None = None) -> tuple[str, str]:
    """§5.1 O3 judged from outside (0.16, 0.17). "No tool can observe its
    grants (§11.3). It learns that they let it call from its operator, or
    from the deployment's §11.1 input when it holds one; a deployment that
    runs no access control lets every principal call. Told so, the tool
    holds a silence from an owner whose tokens it reads as the finding. Not
    told, the silence is unobservable, and the tool names the premise it
    lacked: a silence is never a verdict on its own (O5)."

    ``may_call`` is what the tool was told: True (its grants let it call),
    False (they do not), or None (not told). zk2py reads it from the
    deployment's input through ``acl.may_call``. ``present`` is whether the
    tool reads the owner's tokens. Returns (verdict, why): ``clean``,
    ``finding`` or ``unobservable``."""
    if not result.silent:
        return "clean", f"answered: {len(result.replies)} replies"
    if may_call is None:
        return "unobservable", "silent, and not told that its grants let it call (O5)"
    if may_call is False:
        return "unobservable", "silent, and told that its grants do not let it call: a refusal is silent too"
    if present is False:
        return "unobservable", "silent, and the owner's tokens are not read: absent, not unanswered (O5)"
    return "finding", "silent, from a present owner, under grants that let this caller call (O3)"


# -- §2.6 events replay, the consumer's bound (0.16) -----------------------------

_CROCKFORD = "0123456789abcdefghjkmnpqrstvwxyz"


def ulid_time_ms(ulid: str) -> int | None:
    """The 48-bit millisecond time a lowercase ULID chunk carries (§1.2):
    its first 10 Crockford base32 characters."""
    if len(ulid) != 26:
        return None
    n = 0
    for ch in ulid[:10]:
        i = _CROCKFORD.find(ch)
        if i < 0:
            return None
        n = n * 32 + i
    return n


def new_ulid(ms: int) -> str:
    """A lowercase ULID chunk for the millisecond time ``ms``, random below."""
    import secrets

    n = (ms << 80) | secrets.randbits(80)
    out = []
    for _ in range(26):
        out.append(_CROCKFORD[n & 31])
        n >>= 5
    return "".join(reversed(out))


def replay_events(session: zenoh.Session, selector: str, retention_s: int,
                  now_ms: int | None = None, timeout: float = GET_TIMEOUT_S) -> tuple[list[str], list[str]]:
    """§2.6: "A consumer replays with a wildcard GET bounded by the
    retention", and (0.16) "The retention is the bound a consumer applies
    on replay": a union storage prunes nothing by retention. The GET asks
    with ``_time``; since a backend may ignore it (the memory backend
    does, spike S5), the consumer filters by the ULID's time (state.md §8).
    Returns (kept keys, dropped keys)."""
    now_ms = int(time.time() * 1000) if now_ms is None else now_ms
    bound = now_ms - retention_s * 1000
    kept, dropped = [], []
    for a in _answers(session, f"{selector}?_time=[now(-{retention_s}s)..]", zenoh.QueryTarget.ALL, timeout):
        if not a.ok or a.key is None:
            continue
        t = ulid_time_ms(a.key.rsplit("/", 1)[-1])
        (kept if t is not None and t >= bound else dropped).append(a.key)
    return kept, dropped


# -- §4 state, a consumer's GET --------------------------------------------------

@dataclass
class StateReply:
    key: str
    deleted: bool                # a reply_del (S2, S3)
    payload: bytes
    encoding: str
    stamp: str | None            # Zenoh's "<ntp64>/<id hex>" text
    stamp_id: str | None         # the id half: the HLC that issued it (§4.1)


@dataclass
class StateReading:
    """The owner's answer to one state GET. ``silent`` is no reply at all:
    "That silence is not a verdict about the key" (S6, O5)."""

    replies: list[StateReply] = field(default_factory=list)
    errors: list[str] = field(default_factory=list)

    @property
    def silent(self) -> bool:
        return not self.replies and not self.errors


def get_state(session: zenoh.Session, selector: str, timeout: float = GET_TIMEOUT_S) -> StateReading:
    """S4: "a state GET MUST be addressed to the owner's keys, with target
    All and consolidation Latest set explicitly." Current state is the
    owner's answer (S6). §3.2 R6: a reply on a key that is not concrete is
    discarded."""
    q: queue.Queue = queue.Queue()
    session.get(selector, zenoh.handlers.Callback(q.put, lambda: q.put(_DONE)),
                target=zenoh.QueryTarget.ALL, consolidation=zenoh.ConsolidationMode.LATEST,
                timeout=timeout)
    out = StateReading()
    while True:
        try:
            item = q.get(timeout=timeout + 5.0)
        except queue.Empty:
            break
        if item is _DONE:
            break
        if item.ok is None:
            out.errors.append(f"{item.err.encoding}: {item.err.payload.to_bytes()!r}")
            continue
        s = item.ok
        key = str(s.key_expr)
        if "*" in key:
            continue
        ts = s.timestamp
        out.replies.append(StateReply(
            key, s.kind == zenoh.SampleKind.DELETE, s.payload.to_bytes(), str(s.encoding),
            None if ts is None else str(ts),
            None if ts is None else str(ts).split("/", 1)[1]))
    return out


# -- §5 operations, a caller's call ----------------------------------------------

@dataclass
class CallReply:
    kind: str            # "value", "envelope", "refused_envelope", "transport"
    key: str | None
    encoding: str
    payload: bytes
    envelope: dict[str, Any] | None = None   # the decoded §5.2 envelope
    refusal: str | None = None               # the tag a malformed envelope got


@dataclass
class CallResult:
    replies: list[CallReply] = field(default_factory=list)
    #: seconds from the call to its first reply, and to the query's
    #: completion (None when ``first`` returned before it)
    first_s: float | None = None
    done_s: float | None = None

    @property
    def silent(self) -> bool:
        """O5: "MUST NOT treat an empty reply set as a verdict". §5.1: "A
        call that ends there with no value and no envelope is silent", "the
        transport's own error reply included" (zenoh's `Timeout`, Appendix
        B), so a transport reply does not make a call answered."""
        return not any(r.kind in ("value", "envelope", "refused_envelope") for r in self.replies)


def call(session: zenoh.Session, key: str, payload: bytes = b"", *, fanout: bool = False,
         encoding: str | None = None, timeout: float = GET_TIMEOUT_S, first: bool = False) -> CallResult:
    """Call an operation (§5.1).

    - A concrete call uses target ``BestMatching`` (O1); a call to a fan-out
      operation, target ``All`` and consolidation ``None`` (O2).
    - Consolidation is ``None`` for a concrete call too: §5.1 O1 (0.7) says
      "a concrete call MUST set BestMatching and None", since ``Latest``
      would hold the reply until the query completes.
    - ``first``: a one-reply caller "takes the first value or envelope on
      the call's key, without waiting for the query to complete" (§5.1
      "A concrete call"); the call returns at the first value or envelope.
      Otherwise every reply is collected until the query completes.
    - A value reply is the result; a reply error is decoded by its encoding
      (§5.2). Only ``application/json``, ``application/cbor`` and
      ``application/protobuf;zk2.core.v1.Error`` carry an envelope; any
      other is the transport's (§5.2 "Transport errors").
    - No reply is silence, never a verdict (O5).
    """
    from . import envelope

    q: queue.Queue = queue.Queue()
    kwargs: dict[str, Any] = {
        "target": zenoh.QueryTarget.ALL if fanout else zenoh.QueryTarget.BEST_MATCHING,
        "consolidation": zenoh.ConsolidationMode.NONE,
        "timeout": timeout,
        "payload": payload,
    }
    if encoding is not None:
        kwargs["encoding"] = encoding
    t0 = time.monotonic()
    session.get(key, zenoh.handlers.Callback(q.put, lambda: q.put(_DONE)), **kwargs)
    out = CallResult()
    while True:
        try:
            item = q.get(timeout=timeout + 5.0)
        except queue.Empty:
            break
        if item is _DONE:
            out.done_s = time.monotonic() - t0
            break
        if out.first_s is None:
            out.first_s = time.monotonic() - t0
        if item.ok is not None:
            s = item.ok
            out.replies.append(CallReply("value", str(s.key_expr), str(s.encoding), s.payload.to_bytes()))
        else:
            enc, data = str(item.err.encoding), item.err.payload.to_bytes()
            if enc not in (envelope.JSON_ENCODING, envelope.CBOR_ENCODING, envelope.PROTOBUF_ENCODING):
                out.replies.append(CallReply("transport", None, enc, data))
                continue
            try:
                env = envelope.decode(enc, data)
                out.replies.append(CallReply("envelope", None, enc, data, envelope=env))
            except envelope.EnvelopeError as e:
                out.replies.append(CallReply("refused_envelope", None, enc, data, refusal=e.tag))
        if first and out.replies[-1].kind != "transport" and (out.replies[-1].kind != "value"
                                                              or out.replies[-1].key == key):
            break
    return out

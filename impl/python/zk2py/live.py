"""The live half, first slice: presence, descriptors and contract retrieval
over Zenoh (core.md §3.3, §8.1–§8.4; observer.md §3, §4), on zenoh-python
1.10.1. Core 0.58 moved the reader's rules quoted here to observer.md, word
for word: a bare § below is still the core's, a rule that moved is cited
``observer §n``, and a version in parentheses is the core amendment that
made the rule (observer §0).

Everything here is a *tool*'s side of the rules: it reads the bus, it never
owns keys. The static half supplies the checks: :mod:`zk2py.keys` parses
token keys, :mod:`zk2py.descriptor` checks a descriptor, :mod:`zk2py.bundle`
verifies a bundle.

**Reading presence (observer §3.1, 0.5).** "A caller or tool's
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
  complete, and empty" (observer §3.4, 0.8), so an empty one never rules out a
  refusal by access control (:attr:`Presence.reading`);
- presence is polled with GETs, never inferred from silence (R7, observer §7.1; O5).

**What a complete presence read is (observer §3.5 to §3.7, 0.37).**
- **A read through a session cut off is not complete**, "whatever its
  replies say", and "a reader SHOULD NOT make it": :func:`list_presence`
  checks the session's **links** (0.38, "What the check reads":
  ``Session.info.links()``, which zenoh-python 1.10.1 exposes, never
  ``transports()``, which can still list a closing transport) and makes no
  read through a session that holds none, reporting it **not made**, a
  third kind beside complete and possibly incomplete. It checks again when
  the read ends (0.38, F-185): a read in flight when the session's last
  link goes is answered at once, complete and empty, and it is **possibly
  incomplete**, cut off while it ran, its tokens kept as seen.
- **The read that establishes absence goes on a session with no
  liveliness subscriber**: every liveliness subscriber zk2py's readers
  declare goes through :func:`declare_liveliness_subscriber`, which marks
  its session for its life, and :func:`list_presence` makes no read on a
  marked session (not made, beside a subscriber). :func:`watch_session`
  opens a reader's **watch session** beside a session :func:`open_client`
  opened, and :func:`settle` is the one round trip on it before a seed.
- **Absence is established only by a complete read**:
  :mod:`zk2py.presence` follows tokens live and keeps a withdrawal
  unconfirmed until a read.

**A watch session's round trip at a population (observer §3.3, §3.7, 0.41).**
- **A read that runs to its session's interests timeout is not
  complete.** zenoh 1.10.1 ends a liveliness GET its routers have not
  finished at ``routing/interests/timeout`` with the final reply alone and
  no error reply. :func:`list_presence` reads a read whose time reached its
  session's interests timeout as possibly incomplete
  (:data:`AT_INTERESTS_TIMEOUT`), and :func:`settle` a round trip ended
  there as not standing (0.44, F-240: before, "as not made"). A session's configuration cannot be read back from
  zenoh-python, so :func:`open_client` records the interests timeout it
  opened a session with (:func:`interests_timeout_of`); a session zk2py did
  not open is taken at zenoh's default, 10 s.
- **Its sessions run with 120 s** (:data:`INTERESTS_TIMEOUT_S`), the
  reference's tools' value: "A reader whose reads may wait longer SHOULD
  run its session with an interests timeout above the longest it waits for
  one" (0.49, F-250). zk2py's readers wait 1 s by default and 30 s in
  presence §10 step 10; its follower asks each GET with 121 s and bounds its
  wait apart. Observer text 0.5 (F-298) states the reading zk2py took:
  "This rule measures that bound, never the GET's own timeout: such a
  reader meets it when the longest it waits is below the interests
  timeout", its example 120 s, 121 s and at most 30 s.
- **The round trip's key** is ``…/@zk/settle`` under the subscribers'
  presence subtree (:func:`settle_key`): it matches no token, carries
  nothing, and a grant that admits the subscribers admits it. Before 0.41
  zk2py asked ``zk2/@zk/settle``, which matches no token too, but lies
  outside every subscriber's subtree.
- **A round trip stands** (:class:`RoundTrip`) only when it ended at its
  routers' final reply, with no error reply, before the interests timeout,
  through a session that held a link before it and at its end: a watch
  session cut off answers it at once, as it answers every liveliness GET.

**A watch session on the wire keys (observer §3.7, 0.42).**
- **In a namespace, a watch session in no namespace.** "A reader that
  follows presence in a namespace SHOULD hold its watch session in **no**
  namespace, and declare its subscribers, without history, on the **wire
  keys** … It strips the namespace from every key it hears. A key outside
  the namespace is not the deployment's: it is counted, never folded."
  :func:`watch_session` opens a namespaced reading session's watch session
  in no namespace, and :func:`watch_or_beside` declares the subscriber on
  :func:`wire_selector` and makes the round trip on its wire key
  (``ns/zk2/*/*/@zk/settle``), every key heard handed on through
  :class:`Stripped`, which counts a key outside the namespace and drops it.
  Before 0.42 the watch session was opened in the reading session's
  namespace.
- **No GET of the deployment's keys on it**: "A watch session in no
  namespace makes none of the deployment's". zk2py's readers make every
  read on the reading session; the watch session holds subscribers and
  makes its round trip alone.
- **An interests timeout zk2py cannot read** (F-232): "a session it is
  handed and cannot read, it takes at zenoh's default, 10 s … such a reader
  says that its reads' completeness rests on the default". A read or a
  round trip on a session zk2py did not open, nor :func:`register`, says
  so (:attr:`Presence.interests_timeout_assumed`).

**One round trip, and one read, at a time (observer §3.7, 0.44).** "zenoh 1.10.1
tells a router nothing of a liveliness GET whose caller stops waiting for
it … and the router answers it in full. A reader SHOULD NOT ask a round
trip while one it asked since its watch session's transport last moved is
in flight, nor ask a read while one it asked of the same scope is: the
next that needs one takes up the one in flight". :class:`Flight` is one
GET asked on a thread of its own, its own timeout above the session's
interests timeout (:func:`flight_timeout`), so a caller's wait ending
leaves it running to be taken up; :class:`Barrier` is a watch session's
round trips through flights, one per transport state. :func:`settle` and
:func:`list_presence` stay the one-shot primitives a flight runs: a
profile reader that makes its round trip once, as written against 0.40,
still calls them.

0.45 (F-242, F-243) words it as zk2py already did it: the wait that ends
first is "its caller's or the reader's own"; "In flight is at the reader:
from the GET's ask until its final reply reaches the reader", so a GET
its own timeout ends cannot be taken up; and "A reader MAY ask each GET
with its own timeout at or above its session's interests timeout … and
bound its own wait apart", which :func:`flight_timeout` is. zk2py does so
for its follower's GETs and the runner's barrier; a profile reader's
blocking GET keeps its own timeout as its wait.

**Timeouts are the caller's (core §8.1).** The defaults here are the 1 s that
"the scenarios, and so a conformance run, use".
"""

from __future__ import annotations

import dataclasses
import json
import queue
import re
import threading
import time
from dataclasses import dataclass, field
from typing import Any

import zenoh

from . import admin_records, bundle, keys, zk2c

#: Liveliness GET timeout: the caller's choice, 1 s in a conformance run (§8.1).
PRESENCE_TIMEOUT_S = 1.0
#: GET timeout for a descriptor or a bundle attempt: likewise (§3.3, §8.4).
GET_TIMEOUT_S = 1.0
#: zenoh 1.10.1's ``routing/interests/timeout`` when a session's configuration
#: does not set it (observer §3.3, 0.41; core Appendix B).
ZENOH_INTERESTS_TIMEOUT_S = 10.0
#: The interests timeout zk2py opens its sessions with (observer §3.3, 0.41, worded
#: by 0.49, F-250: "A reader whose reads may wait longer SHOULD run its session
#: with an interests timeout above the longest it waits for one"): the
#: reference's tools run with 120 s.
INTERESTS_TIMEOUT_S = 120.0
_DONE = object()


#: How each session :func:`open_client` opened was opened, by ``id``, so that
#: :func:`watch_session` can open its watch session the same way. A zenoh-python
#: session takes no attribute and no weak reference, so the registry holds
#: the session itself, which keeps its id from being reused while it lives.
#: An entry goes with :func:`forget` (or :func:`close`), and every entry of
#: a session found closed is dropped as the registry is next touched
#: (:func:`_prune`), so a long-lived tool neither leaks nor hands back a
#: closed watch session.
_OPENED: dict[int, tuple[Any, str, tuple[str, str] | None]] = {}
#: Sessions that hold a liveliness subscriber zk2py declared (observer §3.7, 0.37):
#: "marked for its life", since the router keeps the declarations after the
#: subscriber is undeclared (Appendix B).
_MARKED: dict[int, Any] = {}
#: Each reading session's watch session: (the reading session, its watch).
#: Every lookup checks the stored session *is* the one asked about, since a
#: pruned entry's id may belong to another session later.
_WATCHES: dict[int, tuple[Any, Any]] = {}
#: What each session zk2py opened was opened with beyond its endpoint: (the
#: session, its namespace or None, its ``routing/interests/timeout`` in
#: seconds). zenoh-python reads no configuration back from a session, and
#: observer §3.3 (0.41) needs both: a read that reached the interests timeout is
#: possibly incomplete, and a watch session in a namespace keeps history.
_CONF: dict[int, tuple[Any, str | None, float]] = {}


def _is_closed(session: Any) -> bool:
    try:
        return bool(session.is_closed())
    except Exception:  # noqa: BLE001 - a binding without is_closed: taken as open
        return False


def _prune() -> None:
    """Drop every registry entry whose session is closed: a reading
    session closed takes its watch session with it (closed too), and a
    watch session closed is dropped, so that another is opened."""
    for k, (reader, w) in list(_WATCHES.items()):
        if _is_closed(reader):
            _WATCHES.pop(k, None)
            try:
                w.close()
            except Exception:  # noqa: BLE001 - closing anyway
                pass
        elif _is_closed(w):
            _WATCHES.pop(k, None)
    # Two threads may prune at once (a runner's sections in parallel): an
    # entry another one dropped is gone already.
    for k in [k for k, v in list(_OPENED.items()) if _is_closed(v[0])]:
        _OPENED.pop(k, None)
    for k in [k for k, v in list(_MARKED.items()) if _is_closed(v)]:
        _MARKED.pop(k, None)
    for k in [k for k, v in list(_CONF.items()) if _is_closed(v[0])]:
        _CONF.pop(k, None)


def forget(session: Any) -> None:
    """Drop the registry's entries for ``session`` (opened, marked, its watch
    session), closing the watch session zk2py opened for it."""
    if _OPENED.get(id(session), (None,))[0] is session:
        del _OPENED[id(session)]
    if _MARKED.get(id(session)) is session:
        del _MARKED[id(session)]
    if _CONF.get(id(session), (None,))[0] is session:
        del _CONF[id(session)]
    entry = _WATCHES.get(id(session))
    w = None
    if entry is not None and entry[0] is session:
        del _WATCHES[id(session)]
        w = entry[1]
    if w is not None:
        _OPENED.pop(id(w), None)
        _MARKED.pop(id(w), None)
        _CONF.pop(id(w), None)
        try:
            w.close()
        except Exception:  # noqa: BLE001 - closing anyway
            pass


def close(session: Any) -> None:
    """Close a session zk2py opened, with its watch session, and forget both."""
    forget(session)
    try:
        session.close()
    except Exception:  # noqa: BLE001 - closing anyway
        pass


def open_client(endpoint: str, auth: tuple[str, str] | None = None, *, namespace: str | None = None,
                interests_timeout_s: float = INTERESTS_TIMEOUT_S,
                transport: dict[str, Any] | None = None) -> zenoh.Session:
    """A *client* session to one router endpoint, multicast scouting off: a
    tool that sees exactly the router it is pointed at. ``auth`` is a
    usrpwd (user, password), the principal's binding (§11.3). ``namespace``
    is zenoh's session namespace (§1.6), and ``interests_timeout_s`` its
    ``routing/interests/timeout``, 120 s unless asked (observer §3.3, 0.41); both are
    recorded, since zenoh-python reads neither back. ``transport``, a far
    side's (:meth:`zk2py.far_side.FarSide.config`), is each zenoh key and its
    value: the session then runs the face's ``[transport]`` (link.v2 §2.20,
    observer §7.5, 0.23), its interests timeout among them."""
    _prune()
    if transport is not None and "routing/interests/timeout" in transport:
        interests_timeout_s = transport["routing/interests/timeout"] / 1000
    s = _open_client(endpoint, auth, namespace=namespace, interests_timeout_s=interests_timeout_s,
                     transport=transport)
    _OPENED[id(s)] = (s, endpoint, auth)
    _CONF[id(s)] = (s, namespace, interests_timeout_s)
    return s


def _open_client(endpoint: str, auth: tuple[str, str] | None = None, *, namespace: str | None = None,
                 interests_timeout_s: float = INTERESTS_TIMEOUT_S,
                 transport: dict[str, Any] | None = None) -> zenoh.Session:
    conf = zenoh.Config()
    conf.insert_json5("mode", json.dumps("client"))
    conf.insert_json5("connect/endpoints", json.dumps([endpoint]))
    conf.insert_json5("scouting/multicast/enabled", "false")
    # zenoh's value is in milliseconds (measured: 300 ends a stalled read at 0.30 s)
    conf.insert_json5("routing/interests/timeout", json.dumps(int(round(interests_timeout_s * 1000))))
    for path, value in (transport or {}).items():
        conf.insert_json5(path, json.dumps(value))
    if namespace is not None:
        conf.insert_json5("namespace", json.dumps(namespace))
    if auth is not None:
        conf.insert_json5("transport/auth/usrpwd", json.dumps({"user": auth[0], "password": auth[1]}))
    return zenoh.open(conf)


def register(session: Any, *, namespace: str | None = None,
             interests_timeout_s: float = ZENOH_INTERESTS_TIMEOUT_S) -> Any:
    """Record what a session zk2py opened by other means was opened with
    (:func:`open_client` records its own), for :func:`interests_timeout_of`
    and :func:`namespace_of`."""
    _CONF[id(session)] = (session, namespace, interests_timeout_s)
    return session


def interests_timeout_of(session: Any) -> float:
    """observer §3.3 (0.41): the session's ``routing/interests/timeout``, in seconds,
    as recorded when zk2py opened it; zenoh's default, 10 s, for a session
    zk2py did not record (zenoh-python reads no configuration back)."""
    entry = _CONF.get(id(session))
    return entry[2] if entry is not None and entry[0] is session else ZENOH_INTERESTS_TIMEOUT_S


def interests_timeout_known(session: Any) -> bool:
    """observer §3.3 (0.42, F-232): whether zk2py knows the session's interests
    timeout, having opened the session itself (:func:`open_client`) or been
    told it (:func:`register`). When not, it is taken at zenoh's default,
    and a read's completeness rests on that: "That holds only of a session
    whose configuration states none … such a reader says that its reads'
    completeness rests on the default"."""
    entry = _CONF.get(id(session))
    return entry is not None and entry[0] is session


#: What a read or a round trip on a session zk2py cannot read the interests
#: timeout of says (observer §3.3, 0.42, F-232).
INTERESTS_TIMEOUT_ASSUMED = ("its completeness rests on zenoh's default interests timeout, 10 s, taken for a "
                             "session zk2py did not open and cannot read (observer §3.3, 0.42)")


def _namespace_chunks(namespace: str | None) -> str:
    """A namespace as its chunks, without a leading or trailing ``/``, the
    one spelling :func:`wire_selector` and :func:`strip_namespace` both use
    (review-bot on PR #879); empty for none."""
    return (namespace or "").strip("/")


def wire_selector(namespace: str | None, selector: str) -> str:
    """observer §3.7 (0.42): a base-relative selector as a session in no namespace
    declares it to watch a deployment in ``namespace``, "the namespace in
    front, ``<ns>/zk2/…``"; unchanged for no namespace."""
    ns = _namespace_chunks(namespace)
    return f"{ns}/{selector}" if ns else selector


def strip_namespace(namespace: str | None, key: str) -> str | None:
    """observer §3.7 (0.42): "It strips the namespace from every key it hears. A key
    outside the namespace is not the deployment's": the key without its
    ``<ns>/`` chunks, or None for one outside the namespace. A key heard
    with no namespace to strip is the key."""
    ns = _namespace_chunks(namespace)
    if not ns:
        return key
    prefix = ns + "/"
    return key[len(prefix):] if key.startswith(prefix) and len(key) > len(prefix) else None


class _StrippedSample:
    """A liveliness sample as a reader in a namespace hears it from its watch
    session in no namespace: its key stripped, every other attribute the
    sample's."""

    def __init__(self, sample: Any, key: str) -> None:
        self._sample, self.key_expr = sample, key

    def __getattr__(self, name: str) -> Any:
        return getattr(self._sample, name)


class Stripped:
    """observer §3.7 (0.42): a subscriber's callback on a watch session in no
    namespace watching ``namespace``'s wire keys. Each sample is handed on
    with the namespace stripped from its key; a key outside the namespace is
    counted (:attr:`outside`) and never handed on: "it is counted, never
    folded"."""

    def __init__(self, namespace: str, callback: Any) -> None:
        self.namespace = namespace
        self.inner = callback.callback if isinstance(callback, zenoh.handlers.Callback) else callback
        self.outside = 0
        self.outside_keys: list[str] = []

    def __call__(self, sample: Any) -> None:
        key = strip_namespace(self.namespace, str(sample.key_expr))
        if key is None:
            self.outside += 1
            if len(self.outside_keys) < 16:
                self.outside_keys.append(str(sample.key_expr))
            return
        self.inner(_StrippedSample(sample, key))


def namespace_of(session: Any) -> str | None:
    """The session's namespace, as recorded when zk2py opened it; None for
    none, or for a session zk2py did not record."""
    entry = _CONF.get(id(session))
    return entry[1] if entry is not None and entry[0] is session else None


# -- observer §3, presence -------------------------------------------------------

#: Why a read was not made (observer §3.5, 0.37; 0.38 reads the links).
CUT_OFF = "cut off: the session holds no link to another zenoh node (observer §3.5, 0.37, 0.38)"
#: Why a read made is possibly incomplete when its session's links were
#: gone at its end (observer §3.5, 0.38, F-185).
CUT_OFF_DURING = ("cut off while it ran: the session's links were gone when it ended, so it came back at the cut "
                  "(observer §3.5, 0.38)")
#: Why a reader that follows presence makes no read while its watch session is
#: cut off, whatever its reading session's links (observer §3.7, 0.42, F-227, F-228):
#: "its subscribers hear nothing … it reports the read as not made, the
#: reason its watch session's".
WATCH_CUT_OFF = ("its watch session is cut off: it holds no link to another zenoh node, so the reader follows "
                 "nothing and makes no read it would follow (observer §3.7, 0.42)")
BESIDE_SUBSCRIBER = ("beside a liveliness subscriber: the session holds one, and a read there can complete "
                     "without the tokens it covers (observer §3.7, 0.37; core Appendix B)")
#: Why a read cannot be shown complete: this binding's links accessor
#: failed, so observer §3.5's check could not be made (never a pass).
LINKS_UNREADABLE = ("the session's links could not be read (Session.info.links() failed), so the "
                    "read cannot be shown complete (observer §3.5, 0.37, 0.38)")
#: Why a read made is possibly incomplete, and a round trip not standing,
#: when its time reached its session's interests timeout (observer §3.3, 0.41; 0.44,
#: F-240: "as not standing … it was asked, so it was made").
AT_INTERESTS_TIMEOUT = ("it ran to its session's interests timeout (routing/interests/timeout), where zenoh ends a "
                        "liveliness GET its routers have not finished with the final reply alone (observer §3.3, 0.41)")
#: The three kinds of a read (core 0.37's §8.1, now observer §3.5: "A read not made is a third kind,
#: never folded into either").
COMPLETE, POSSIBLY_INCOMPLETE, NOT_MADE = "complete", "possibly incomplete", "not made"


def _info(session: Any) -> Any:
    return session.info() if callable(getattr(session, "info", None)) else session.info


def links_held(session: Any) -> bool | None:
    """observer §3.5 (0.38, "What the check reads"): whether the session holds a link
    to another node, by ``Session.info.links()``. zenoh drops a closing
    transport's links first, then answers the reads in flight, then removes
    the transport, so ``transports()`` can still list it when a read is
    answered at the cut. None when this binding cannot say, which is then
    this reader's premise, never a pass."""
    try:
        return bool(_info(session).links())
    except Exception:  # noqa: BLE001 - an accessor this binding lacks says nothing
        return None


def transports_held(session: Any) -> bool | None:
    """Whether the session lists a transport (``Session.info.transports()``):
    what a scenario waits on to see a cut settle, never observer §3.5's check, which
    reads the links (0.38)."""
    try:
        return bool(_info(session).transports())
    except Exception:  # noqa: BLE001
        return None


def marked(session: Any) -> bool:
    """Whether the session holds a liveliness subscriber zk2py declared."""
    return _MARKED.get(id(session)) is session


def declare_liveliness_subscriber(session: Any, selector: str, callback: Any, *, history: bool = False) -> Any:
    """A liveliness subscriber, callback-driven (observer §3.1's handler rule), that
    marks its session (observer §3.7, 0.37): no presence read is made there after."""
    _MARKED[id(session)] = session
    handler = callback if isinstance(callback, zenoh.handlers.Callback) else zenoh.handlers.Callback(callback)
    return session.liveliness().declare_subscriber(selector, handler, history=history)


@dataclass(frozen=True)
class Placement:
    """Where :func:`watch_or_beside` put a reader's subscriber: on a watch
    session or beside the reads, and whether the round trip there came back
    (None beside the reads, where none is made). A reader whose round trip
    did not come back cannot show its seed complete (observer §3.7, 0.37)."""

    watch: bool
    settled: bool | None
    #: the subscriber's selector as declared: on a watch session in no
    #: namespace watching a namespaced deployment, its wire key (observer §3.7, 0.42)
    selector: str = ""
    #: the namespace stripped from every key heard, or None
    namespace: str | None = None
    #: the callback's stripping wrapper, counting keys outside the namespace
    stripped: Any = field(default=None, compare=False)

    def __bool__(self) -> bool:
        return self.watch

    @property
    def outside(self) -> int:
        """Keys heard outside the namespace, counted and never handed on
        (observer §3.7, 0.42)."""
        return self.stripped.outside if self.stripped is not None else 0


#: How many times :func:`watch_or_beside` tries the round trip.
SETTLE_TRIES = 3


def watch_or_beside(session: Any, selector: str, callback: Any, *, history: bool = False) -> tuple[Any, Placement]:
    """A reader's liveliness subscriber on its watch session (observer §3.7, 0.37),
    then the round trip there (tried :data:`SETTLE_TRIES` times), so the
    reading session holds none. For a session :func:`open_client` did not
    open, no watch session can be had: the subscriber goes beside the
    reads, unmarked, as before 0.37, and the placement says so.

    0.42: for a reading session in a namespace, the watch session is in
    none (:func:`watch_session`), so the subscriber is declared on the wire
    key (:func:`wire_selector`), the round trip asked on its wire key, and
    every key heard handed on stripped (:class:`Stripped`); ``selector``
    stays base-relative for the caller. ``history`` is the caller's: "its
    subscribers without history" is the rule for a reader that seeds by a
    read on its other session."""
    w = watch_session(session)
    if w is None:
        handler = callback if isinstance(callback, zenoh.handlers.Callback) else zenoh.handlers.Callback(callback)
        return session.liveliness().declare_subscriber(selector, handler, history=history), \
            Placement(False, None, selector)
    ns = namespace_of(session)
    if ns and namespace_of(w) is None:
        wire = wire_selector(ns, selector)
        stripped = Stripped(ns, callback)
        # the caller's Callback's drop hook and mode pass through
        # (review-bot on PR #879)
        is_cb = isinstance(callback, zenoh.handlers.Callback)
        indirect = callback.indirect if is_cb else True
        drop = callback.drop if is_cb else None
        handler = (zenoh.handlers.Callback(stripped, drop, indirect=indirect) if drop is not None
                   else zenoh.handlers.Callback(stripped, indirect=indirect))
        sub = declare_liveliness_subscriber(w, wire, handler, history=history)
        settled = any(settle(w, selector=wire) for _ in range(SETTLE_TRIES))
        return sub, Placement(True, settled, wire, ns, stripped)
    sub = declare_liveliness_subscriber(w, selector, callback, history=history)
    settled = any(settle(w, selector=selector) for _ in range(SETTLE_TRIES))
    return sub, Placement(True, settled, selector, namespace_of(w))


def watch_session(session: Any) -> Any | None:
    """observer §3.7 (0.37): the **watch session** of a reader whose reading session
    :func:`open_client` opened, opened the same way on first use (its
    interests timeout too), else None (a session zk2py cannot open again).

    0.42: in **no** namespace, whatever the reading session's: "A reader
    that follows presence in a namespace SHOULD hold its watch session in
    no namespace", watching the wire keys (:func:`watch_or_beside`). It
    makes no GET of the deployment's keys: "there its base-relative keys
    are the bus root's"."""
    _prune()
    entry = _WATCHES.get(id(session))
    if entry is not None and entry[0] is session:
        return entry[1]
    opened = _OPENED.get(id(session))
    if opened is None or opened[0] is not session:
        return None
    _, endpoint, auth = opened
    w = open_client(endpoint, auth, namespace=None, interests_timeout_s=interests_timeout_of(session))
    _WATCHES[id(session)] = (session, w)
    return w


#: The round trip's key when no subscriber's selector names ``@zk``.
SETTLE_KEY = "zk2/*/*/@zk/settle"


def settle_key(selector: str | None = None) -> str:
    """observer §3.7 (0.41): the round trip's key, "a key no token holds … under a key
    ``…/@zk/settle``, inside the presence subtree the subscribers read, so a
    grant that admits them admits it": the subscriber's selector up to its
    ``@zk`` chunk, then ``settle`` (``zk2/lab/*/@zk/instance/*`` →
    ``zk2/lab/*/@zk/settle``). A token's ``@zk`` chunk is followed by its
    kind and more (§1.1), so no token matches it. :data:`SETTLE_KEY` for a
    selector without ``@zk``."""
    chunks = (selector or "").split("/")
    if "@zk" in chunks:
        return "/".join(chunks[:chunks.index("@zk") + 1] + ["settle"])
    return SETTLE_KEY


@dataclass(frozen=True)
class RoundTrip:
    """One round trip on a watch session (observer §3.7, 0.37, 0.41), and whether it
    **stands**: it ended at its routers' final reply (``finished``), with no
    error reply, before the session's interests timeout, through a session
    holding a link when it was asked and when it ended. ``why`` says why not;
    ``made`` is False for one not made, no GET asked: through a session cut
    off or whose links cannot be read before it. One ended at the interests
    timeout was made and does not stand (0.44, F-240: "as not standing … it
    was asked, so it was made"; before, zk2py followed 0.41's "as not
    made"). Only ``stands`` is evidence of order. Truthy when it stands."""

    stands: bool
    made: bool
    key: str
    why: str = ""
    elapsed_s: float = 0.0
    finished: bool = False
    errors: int = 0
    tokens: int = 0
    asked_ns: int = 0
    #: the watch session's interests timeout was not known, so zenoh's
    #: default was taken: a round trip judged "before the interests
    #: timeout" rests on it (observer §3.3, 0.42, F-232)
    interests_timeout_assumed: bool = False
    #: monotonic ns when the GET was dropped (its drop callback ran),
    #: whatever ended it: its routers' final reply, an error reply (its own
    #: timeout), zenoh's interests timeout, or a cut. It says when, never
    #: how: ``finished``, ``errors``, ``why`` and ``stands`` say that, and a
    #: caller never reads a nonzero value as success. 0 when no GET was
    #: asked, or its drop had not run by the wait's end. ``asked_ns`` is
    #: taken before the links are checked, and ``elapsed_s`` runs from just
    #: before the GET to the asking thread's wake-up, so ``asked_ns +
    #: elapsed_s`` is off the drop by a few ms either way (round 72: at
    #: least 4.4 ms short beside a reopen's telling, a runner's false
    #: failure)
    ended_ns: int = 0

    def __bool__(self) -> bool:
        return self.stands


def settle(watch: Any, timeout: float = PRESENCE_TIMEOUT_S, *, selector: str | None = None) -> RoundTrip:
    """observer §3.7 (0.37): "one round trip on the watch session after declaring its
    subscribers, and before the seed: a liveliness GET there whose answer it
    discards serves, since its final reply follows the declarations".
    0.41: of :func:`settle_key` (``selector`` is the subscribers'), which
    matches no token and costs one round trip until the watch session's
    transport moves (0.44). It stands only when its final reply came, no
    error reply, before the session's interests timeout ("a round trip …
    that ended there as not standing", 0.44), and
    the session held a link before it and at its end: a watch session cut
    off answers it at once, as it answers every liveliness GET (observer §3.7, 0.37),
    so that answer orders nothing. Its answer is discarded, so a round trip
    that could not stand is not asked: none through a session cut off, nor
    through one whose links cannot be read (review of PR #876)."""
    key = settle_key(selector)
    asked = time.monotonic_ns()
    held = links_held(watch)
    if held is False:
        return RoundTrip(False, False, key, CUT_OFF, asked_ns=asked)
    if held is None:
        return RoundTrip(False, False, key, LINKS_UNREADABLE, asked_ns=asked)
    done = threading.Event()
    errors: list[Any] = []
    oks: list[Any] = []
    ended: list[int] = []

    def drop() -> None:
        ended.append(time.monotonic_ns())
        done.set()

    t0 = time.monotonic()
    try:
        watch.liveliness().get(key, zenoh.handlers.Callback(
            lambda r: (errors if r.ok is None else oks).append(r), drop), timeout=timeout)
    except Exception as e:  # noqa: BLE001 - a closed or broken session makes no round trip
        return RoundTrip(False, False, key, f"the GET could not be made: {e!r}", asked_ns=asked)
    finished = done.wait(timeout + 5.0)
    elapsed = time.monotonic() - t0
    after = links_held(watch)
    rt = dict(key=key, elapsed_s=elapsed, finished=finished, errors=len(errors), tokens=len(oks), asked_ns=asked,
              interests_timeout_assumed=not interests_timeout_known(watch), ended_ns=ended[0] if ended else 0)
    if not finished:
        return RoundTrip(False, True, why="no final reply", **rt)
    if errors:
        # ended at its timeout (an error reply): the declarations not shown ordered
        return RoundTrip(False, True, why="it ended with an error reply (its timeout)", **rt)
    if elapsed >= interests_timeout_of(watch):
        # 0.44 (F-240): asked, so made; it establishes nothing
        return RoundTrip(False, True, why=AT_INTERESTS_TIMEOUT, **rt)
    if after is False:
        return RoundTrip(False, True, why=CUT_OFF_DURING, **rt)
    if after is None:
        return RoundTrip(False, True, why=LINKS_UNREADABLE, **rt)
    return RoundTrip(True, True, **rt)


#: Why a round trip or a read came back to its caller without an answer:
#: the caller's wait ended first, and its GET runs on (observer §3.7, 0.44).
WAIT_ENDED = ("the wait ended first: the GET runs on, answered in full, and the next that needs one takes it up "
              "(observer §3.7, 0.44)")
#: Why a round trip that ended standing does not stand for its caller: the
#: watch session's transport moved while it ran (observer §3.7, 0.41).
MOVED_DURING = "the watch session's transport moved while it ran: it stands for no read after the move (observer §3.7, 0.41)"
#: Why a round trip that ended standing, taken up later, does not stand: the
#: watch session held no link when it was taken up, a cut its listener had
#: not yet told (observer §3.7, 0.41, 0.42; review-bot on PR #885).
CUT_OFF_SINCE = ("cut off since it ended: the watch session held no link when it was taken up, so it stands for "
                 "no read after (observer §3.7, 0.42)")


class Flight:
    """One liveliness GET in flight, asked once on a thread of its own and
    shared by every caller that waits for it (observer §3.7, 0.44: "One round
    trip, and one read, at a time … the next that needs one takes up the one
    in flight"; 0.45, F-243: "whichever wait ended first, its caller's or
    the reader's own", in flight "from the GET's ask until its final reply
    reaches the reader").

    zenoh 1.10.1 tells a router nothing of a GET whose caller stops waiting
    (Appendix B), and its own timeout ends it only for the caller: measured
    (round 69), after a read of 10,000 tokens whose own 0.2 s timeout ended,
    the next GET on that session took 258–266 ms where it takes 6–10 ms on
    a quiet one. So the GET is asked with an own timeout no
    shorter than its session's interests timeout (:func:`flight_timeout`),
    where zenoh's runtime ends it, and a caller's wait is its own: it ends,
    and the flight runs on to be taken up.

    ``gen`` is the caller's count of transport moves when it was asked;
    ``meta`` what the asker noted then; ``result`` (or ``error``) is set
    once, before ``done``."""

    def __init__(self, gen: int = 0, meta: dict[str, Any] | None = None) -> None:
        self.gen = gen
        self.meta: dict[str, Any] = dict(meta or {})
        self.asked_ns = time.monotonic_ns()
        self.done = threading.Event()
        self.result: Any = None
        self.error: BaseException | None = None
        #: how many callers took it up after the one that asked it
        self.taken_up = 0
        #: set by the caller that consumed its answer (a fold made once)
        self.consumed = False

    def run(self, fn: Any) -> "Flight":
        def go() -> None:
            try:
                self.result = fn()
            except BaseException as e:  # noqa: BLE001 - handed to every waiter, never lost on a thread
                self.error = e
            finally:
                self.done.set()

        threading.Thread(target=go, daemon=True, name="zk2py-flight").start()
        return self

    @property
    def in_flight(self) -> bool:
        return not self.done.is_set()

    def wait(self, wait_s: float | None) -> bool:
        """Wait ``wait_s`` (None: until it ends) for its answer: whether it came."""
        return self.done.wait(wait_s)


def flight_timeout(session: Any, at_least: float = 0.0) -> float:
    """A flight's own GET timeout (observer §3.7, 0.44; 0.45's MAY, F-243: "ask
    each GET with its own timeout at or above its session's interests
    timeout, where zenoh's runtime ends it"): above its session's interests
    timeout, where zenoh's runtime ends a liveliness GET its routers have
    not finished, so the GET's own timeout never ends it first while its
    router still answers it in full; never below ``at_least``."""
    return max(at_least, interests_timeout_of(session) + 1.0)


class Barrier:
    """A watch session's round trips, one at a time (observer §3.7, 0.44): "A
    reader SHOULD NOT ask a round trip while one it asked since its watch
    session's transport last moved is in flight … the next that needs one
    takes up the one in flight".

    :meth:`ask` takes up the round trip in flight, or one ended whose answer
    no caller has had, when it was asked at the current move count, else
    asks one (:func:`settle`, its GET's own timeout :func:`flight_timeout`,
    on a thread of its own), and waits ``wait`` for it (None: until it
    ends). A wait that ends first hands back a round trip not standing,
    :data:`WAIT_ENDED`, and the GET runs on. One asked before a move stands
    for nothing after it (:data:`MOVED_DURING`), and after a move the next
    caller asks anew. ``asked`` counts the round trips it asked,
    ``taken_up`` the times one was taken up.

    **The move count is one counter** (review-bot on PR #885). A caller
    that counts its watch session's moves itself hands the barrier that
    count, ``gen``, a callable read whenever the barrier needs it, and never
    calls :meth:`moved`; one that does not tells the barrier each move
    through :meth:`moved`, which counts them. So no two counters must be
    kept in step.

    **An answer taken up after it ended** (review-bot on PR #885). "A round
    trip stands until the watch session's transport moves", however long
    ago it ended: its final reply ordered the subscribers' declarations
    before every read asked after it, and its age changes nothing. But the
    move count holds only the moves the caller has heard, and a transport
    can close before its listener says so. So a round trip taken up after
    it ended stands only if the watch session holds a link when it is taken
    up, as :func:`settle` checks at a round trip's end: a cut not yet heard
    leaves it not standing (:data:`CUT_OFF_SINCE`), and the next caller
    asks anew."""

    def __init__(self, watch: Any, selector: str | None = None, *, get_timeout: float | None = None,
                 gen: Any = None) -> None:
        self.watch, self.selector = watch, selector
        self.key = settle_key(selector)
        self.get_timeout = get_timeout
        self.lock = threading.Lock()
        self._gen_of = gen
        self._moves = 0
        self.flight: Flight | None = None
        self.asked = 0
        self.taken_up = 0

    @property
    def gen(self) -> int:
        """The move count: the caller's (``gen``), else the barrier's own."""
        return self._gen_of() if self._gen_of is not None else self._moves

    def moved(self) -> None:
        """The watch session's transport closed or opened: a round trip asked
        before it is no longer one "asked since its watch session's
        transport last moved". Refused for a barrier that reads its
        caller's count, which then counts the move itself."""
        if self._gen_of is not None:
            raise RuntimeError("this barrier reads its caller's move count: the caller counts the move")
        with self.lock:
            self._moves += 1

    @property
    def in_flight(self) -> bool:
        with self.lock:
            return self.flight is not None and self.flight.in_flight

    def ask(self, wait: float | None = PRESENCE_TIMEOUT_S) -> tuple[RoundTrip, Flight]:
        """A round trip, taken up or asked, and its flight."""
        start = ended = False
        with self.lock:
            fl = self.flight
            gen = self.gen
            # in flight, or ended with its answer not yet handed back: taken up
            if fl is not None and fl.gen == gen and not fl.consumed:
                fl.taken_up += 1
                self.taken_up += 1
                ended = not fl.in_flight
            else:
                fl = self.flight = Flight(gen)
                self.asked += 1
                start = True
        if start:
            timeout = self.get_timeout if self.get_timeout is not None else flight_timeout(self.watch)
            fl.run(lambda: settle(self.watch, timeout, selector=self.selector))
        if not fl.wait(wait):
            return RoundTrip(False, True, self.key, WAIT_ENDED, asked_ns=fl.asked_ns,
                             interests_timeout_assumed=not interests_timeout_known(self.watch)), fl
        if fl.error is not None:
            with self.lock:
                fl.consumed = True
            return RoundTrip(False, False, self.key, f"the GET could not be made: {fl.error!r}",
                             asked_ns=fl.asked_ns), fl
        rt: RoundTrip = fl.result
        # an answer taken up after it ended: the watch session's links now,
        # since a cut its listener has not yet told is in no move count
        held = links_held(self.watch) if ended and rt.stands else True
        with self.lock:
            fl.consumed = True
            moved = fl.gen != self.gen
        why = MOVED_DURING if moved else CUT_OFF_SINCE if held is False else LINKS_UNREADABLE if held is None else ""
        if rt.stands and why:
            rt = dataclasses.replace(rt, stands=False, why=why)
        return rt, fl

    def round_trip(self, wait: float | None = PRESENCE_TIMEOUT_S) -> RoundTrip:
        """:meth:`ask`'s round trip alone."""
        return self.ask(wait)[0]


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
    #: observer §3.5 (0.37): why the read was not made (:data:`CUT_OFF`,
    #: :data:`BESIDE_SUBSCRIBER`), else None
    not_made: str | None = None
    #: the session's links were gone when the read ended: it was cut off
    #: while it ran, so it is possibly incomplete (observer §3.5, 0.38, F-185)
    cut_off_during: bool = False
    #: whether the session's links could be read, before the read and at its
    #: end; when not, the read cannot be shown complete
    links_checked: bool = True
    #: what ``Session.info.transports()`` said at the read's end, beside the
    #: links: informative only (observer §3.5, 0.38: the check reads the links)
    transports_at_end: bool | None = None
    #: its time reached its session's interests timeout, where zenoh ends a
    #: liveliness GET with the final reply alone: possibly incomplete (observer §3.3,
    #: 0.41)
    at_interests_timeout: bool = False
    #: the session's interests timeout the read was judged against, in s
    interests_timeout_s: float | None = None
    #: zk2py could not read the session's interests timeout and took
    #: zenoh's default: its completeness rests on that (observer §3.3, 0.42, F-232)
    interests_timeout_assumed: bool = False

    @property
    def kind(self) -> str:
        """observer §3.5 (0.37): complete, possibly incomplete, or not made."""
        if self.not_made is not None:
            return NOT_MADE
        return COMPLETE if self.complete else POSSIBLY_INCOMPLETE

    @property
    def count(self) -> int:
        return len(self.instances) + len(self.alive) + len(self.members) + len(self.other)

    @property
    def reading(self) -> str:
        """How a tool states this read (observer §3.4, 0.8): "A tool reports absence
        as what its reader could see. Where it cannot rule out a refusal, it
        SHOULD say so, as it says a read is possibly incomplete." A reader
        can never rule one out from the read itself (the CHANGELOG's "No
        probe that tells a refusal from absence")."""
        if self.not_made is not None:
            return f"not made ({self.not_made}): what it would establish is unobservable"
        if self.cut_off_during:
            return f"possibly incomplete ({CUT_OFF_DURING}): {self.count} tokens, silence is not a verdict"
        if not self.links_checked:
            return f"possibly incomplete ({LINKS_UNREADABLE}): {self.count} tokens, silence is not a verdict"
        if self.at_interests_timeout:
            return (f"possibly incomplete ({AT_INTERESTS_TIMEOUT}, {self.interests_timeout_s:g} s, after "
                    f"{self.elapsed_s:.3f} s): {self.count} tokens, silence is not a verdict")
        if not self.complete:
            why = "; ".join(self.errors) or "no final reply"
            return f"possibly incomplete ({why}): {self.count} tokens, silence is not a verdict"
        rests = f" ({INTERESTS_TIMEOUT_ASSUMED})" if self.interests_timeout_assumed else ""
        if self.count == 0:
            return f"complete and empty for this reader: absent, or refused by access control{rests}"
        return f"complete for this reader: {self.count} tokens{rests}"


def list_presence(session: zenoh.Session, selector: str = "zk2/*/*/@zk/**",
                  timeout: float = PRESENCE_TIMEOUT_S) -> Presence:
    """Every liveliness token under ``selector``, by a liveliness GET through
    a callback handler (observer §3.1).

    ``**`` never crosses a verbatim chunk (§1.3), so the selector names
    ``@zk`` literally: ``zk2/*/*/@zk/**`` is every service's control
    tokens, and ``zk2/*/*/@zk/alive/nav.v2/**`` is "who implements nav.v2"
    (§8.1).
    """
    if marked(session):
        return Presence(not_made=BESIDE_SUBSCRIBER)
    held = links_held(session)
    if held is False:
        return Presence(not_made=CUT_OFF)
    replies: list[Any] = []
    done = threading.Event()
    t0 = time.monotonic()
    session.liveliness().get(selector, zenoh.handlers.Callback(replies.append, done.set),
                             timeout=timeout)
    finished = done.wait(timeout + 5.0)
    p = Presence(elapsed_s=time.monotonic() - t0, interests_timeout_s=interests_timeout_of(session),
                 interests_timeout_assumed=not interests_timeout_known(session))
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
    # observer §3.3 (0.8): ended by the routers' final reply, and no error reply. A
    # GET whose drop never came is incomplete too.
    p.complete = finished and not p.errors
    # observer §3.3 (0.41): "A reader MUST read a read whose time reached its
    # session's interests timeout as possibly incomplete": zenoh ends such a
    # GET with the final reply alone, no error reply. Its time is counted
    # from before the GET was asked to its final reply, so it is never less
    # than the time zenoh's runtime counted.
    if finished and p.elapsed_s >= p.interests_timeout_s:
        p.at_interests_timeout, p.complete = True, False
    # observer §3.5 (0.37, 0.38): a session that reaches no node answers at once,
    # complete and empty; a read in flight when its last link goes is
    # answered so too. Checked again at the read's end, by the links (0.38,
    # F-185). A check that cannot be made, before or after, leaves the read
    # possibly incomplete: a broken accessor never weakens the rule.
    after = links_held(session)
    p.transports_at_end = transports_held(session)
    p.links_checked = held is not None and after is not None
    if after is False:
        p.cut_off_during, p.complete = True, False
    if not p.links_checked:
        p.complete = False
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
    #: a ``reply_del``: the reply's sample is a delete (round 108)
    deleted: bool = False
    #: the reply's attachment, or None for none (round 109: a ``reply_del``'s
    #: is its envelope, core §7.5)
    attachment: bytes | None = None


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
             timeout: float, *, user_asked: bool = False):
    """Yield each reply *as it arrives* (an unbounded queue fed by a
    callback), until the GET completes.

    Consolidation is ``None``, which §8.4 (0.5) makes a MUST on both
    retrieval attempts, and §3.3 sets for the descriptor GET: zenoh's
    default holds every reply until the query finalizes, and a slow corrupt
    reply can displace the valid one.

    A selector that reaches a node's declaration records is refused when
    this is called, before anything is asked (:func:`admin_guard`, observer §4.3,
    0.48), unless ``user_asked``. The GET itself is made at the first
    ``next()``, as before the guard, so a malformed selector still raises
    zenoh's ``ZError`` there.
    """
    admin_guard(selector, user_asked=user_asked)
    return _replies(session, selector, target, timeout)


def _replies(session: zenoh.Session, selector: str, target: zenoh.QueryTarget, timeout: float):
    """:func:`_answers`' GET, made at the first ``next()``, and its replies."""
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
                         s.timestamp is not None, s.attachment is not None, replier_of(item),
                         s.kind == zenoh.SampleKind.DELETE,
                         None if s.attachment is None else s.attachment.to_bytes())
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


def get_descriptors(session: zenoh.Session, address: str, instances: list[str],
                    timeout: float = GET_TIMEOUT_S) -> dict[str, list[Answer]]:
    """The descriptors of ``instances``, present instances of ``address``
    (``<system>/<service>``), in one GET (review-bot on #1100, 1): with one
    instance, :func:`get_descriptor` of its key; with more, one GET of
    ``zk2/<address>/@zk/instance/*`` with target All, each reply attributed
    by its key, since every instance answers on its own key (§3.3 "The
    GET"). Returns each instance's replies, an instance with none mapped to
    an empty list."""
    keys = {f"zk2/{address}/@zk/instance/{i}": i for i in instances}
    out: dict[str, list[Answer]] = {i: [] for i in instances}
    if len(instances) == 1:
        out[instances[0]] = get_descriptor(session, next(iter(keys)), timeout)
        return out
    for a in _answers(session, f"zk2/{address}/@zk/instance/*", zenoh.QueryTarget.ALL, timeout):
        if a.ok and a.key in keys:
            out[keys[a.key]].append(a)
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


# -- observer §3.2, presence shapes, read twice (0.10) ---------------------------------

#: The grace between the two reads of a presence shape: "longer than the
#: deployment's longest re-mint overlap" (observer §3.2, 0.10). The caller's choice;
#: 1 s here, as every other wait of a conformance run.
SHAPE_GRACE_S = 1.0


def presence_shapes(session: zenoh.Session, selector: str = "zk2/*/*/@zk/**",
                    timeout: float = PRESENCE_TIMEOUT_S) -> set[tuple[str, str, str]]:
    """The presence shapes observer §3.2 (0.10) names as faults, as read once:
    ``(instance key, what, interface)`` for
    - ``unlisted``: an interface token its instance's descriptor does not
      list;
    - ``no token``: an interface the descriptor lists with ``token`` true,
      unmarked, and no interface token, in a complete read (a read not
      complete can miss a token, never invent one: observer §3.6, 0.37);
    - ``standby holding`` (0.39): an interface token for an interface its
      descriptor marks ``"standby": true``. Observer §6.1: "its descriptor
      disagrees with its tokens besides: it holds a token for an interface
      its descriptor says it exposes nothing of".

    Observer §6.2 (0.39), "Reading the mark": "An instance listing the interface
    marked standby, and holding no token of it in a complete read … is a
    standby: no finding of this check, nor of §3.2's fault shapes, since
    its listing claims nothing exposed." An instance whose descriptor GET
    gets no reply has no shape here: that silence is not a verdict (O5).
    "Exposed" is read from the descriptor alone, as listed with ``token``
    not false and unmarked: an instance that exposes none of a listed
    interface's resources holds no token for it (§5.1 "The active
    instance"), which only the contract can show, and zk2py does not
    retrieve it here."""
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
            held = iface in alive.get(key, set())
            if e.get("standby") is True:
                if held:
                    shapes.add((key, "standby holding", str(iface)))
            elif e.get("token", True) is not False and not held and p.complete:
                shapes.add((key, "no token", str(iface)))
    return shapes


def presence_faults(session: zenoh.Session, selector: str = "zk2/*/*/@zk/**",
                    grace_s: float = SHAPE_GRACE_S, reads: int = 2,
                    log: list[tuple[float, set]] | None = None) -> tuple[set, set]:
    """observer §3.2 (0.10): "A tool that decides a fault from the shape of presence
    … MUST see it in two reads a grace apart … Start-up, re-mint and
    teardown pass through such shapes briefly, by design." Its transient
    half (text 0.2, F-282): "Seen in one of two reads a grace apart, that
    shape is no fault."

    ``reads`` reads, each ``grace_s`` after the last (operations.md §8 step
    6, core 0.59, reads three). A fault is a shape two consecutive reads,
    a grace apart, both saw. Returns (faults, passing: the shapes some read
    saw and no such pair holds); ``log``, when given, gets each read as
    (the monotonic instant it was asked, its shapes), in order."""
    if reads < 2:
        raise ValueError("observer §3.2: a fault needs two reads a grace apart")
    seen: list[set] = []
    for i in range(reads):
        if i:
            time.sleep(grace_s)
        at = time.monotonic()
        seen.append(presence_shapes(session, selector))
        if log is not None:
            log.append((at, seen[-1]))
    return shape_faults(seen)


def shape_faults(seen: list[set]) -> tuple[set, set]:
    """Observer §3.2 over reads made a grace apart, in order: (faults, the
    shapes two consecutive reads both saw; passing, the shapes some read saw
    and no such pair holds). A shape in one read only, as an owner's between
    core §8.2's steps shows one (text 0.2, F-282), passes. zk2py pairs
    each read with the next, as operations.md §8 step 6's "neither pair of
    reads a grace apart" does, so a shape seen in two reads with one
    between them that did not see it passes too: it did not hold between
    them. Fewer than two reads is no pair, and refused, as
    :func:`presence_faults` refuses ``reads < 2`` (review-bot on PR #952):
    one read is never the two the MUST asks for."""
    if len(seen) < 2:
        raise ValueError("observer §3.2: a fault needs two reads a grace apart")
    faults = set().union(*(a & b for a, b in zip(seen, seen[1:])))
    return faults, set().union(*seen) - faults


# -- observer §4.4, meta.zid: whose stamp (0.10) --------------------------------------

def attribute_stamp(stamp_id: str | None, descriptor: dict[str, Any]) -> str:
    """S1's attribution by a tool (observer §4.4, 0.10): ``owner`` when a
    state stamp's id is the owner's ``meta.zid``, ``foreign`` when it is
    another, ``unattributable`` when the descriptor states no zid ("Without
    it, a stamp's clock is unattributable, never foreign"), and
    ``unstamped`` for a reply with no timestamp.

    observer §4.4 (0.11) "Two zids compare by value": "A tool MUST compare two zids by
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


# -- observer §8.2 and §7.4, what a tool tells a person of its scope and its host -------

#: Observer §8.2 (text 0.1, the tooling guide's O5 promoted): "`*` and `**`
#: never match a verbatim chunk. So an ambient selector … never delivers"
#: these. Each kind is given with the key forms the core says it sits in,
#: written as selectors, so a selector reaches the kind when it intersects
#: one of them. Every pattern is the text's, never a guess at a layout:
#: - ``@stream``, ``@state``, ``@op``: core §1.1's data form,
#:   ``zk2/<system>/<service>/<iface>.v<major>/<kind>/<resource…>``, the
#:   kind token at position 5 (§1.1's table; §1.3's three verbatim kinds),
#:   and positions 6+ the resource chunks, one or more (``*/**``);
#: - ``@adv``: core §2.5, "advanced publication … MAY be used on plain
#:   ``stream`` and ``state`` resources only", and "An advanced publisher
#:   declares its own queryable and token under ``<key>/@adv/…``": a plain
#:   stream's or state's data key, then ``@adv``. (Core §11.1's Own grant
#:   spells the subtree ``…/*/stream/**/@adv/**``; ``*/**`` here keeps
#:   §1.1's one resource chunk at least.) An events key has none (§2.5);
#: - ``@zk``: core §1.1's four control forms: ``@zk/instance/<instance>``,
#:   ``@zk/alive/<iface>/<instance>/<fp16>`` and
#:   ``@zk/member/<iface>/<member>/<epoch>`` at position 4, and the contract
#:   bundle ``zk2/@zk/contract/<iface>/<sha256>`` at position 2.
VERBATIM_REACH = (
    ("@stream", ("zk2/*/*/*/@stream/*/**",)),
    ("@state", ("zk2/*/*/*/@state/*/**",)),
    ("@op", ("zk2/*/*/*/@op/*/**",)),
    ("@adv", ("zk2/*/*/*/stream/*/**/@adv/**", "zk2/*/*/*/state/*/**/@adv/**")),
    ("@zk", ("zk2/*/*/@zk/instance/*", "zk2/*/*/@zk/alive/*/*/*", "zk2/*/*/@zk/member/*/*/*",
             "zk2/@zk/contract/*/*")),
)


def blind_spots(selector: str) -> list[str]:
    """Observer §8.2: the verbatim kinds a base-relative selector never
    reaches, in :data:`VERBATIM_REACH`'s order. "A tool that offers a
    wildcard scope either names the verbatim kinds it watches beside it, or
    states that they are excluded": a tool reports this list as excluded.
    Read with zenoh's own intersection, as ``**`` crossing no verbatim chunk
    is zenoh's (core §1.3)."""
    sel = zenoh.KeyExpr(selector)
    return [kind for kind, reach in VERBATIM_REACH if not any(sel.intersects(zenoh.KeyExpr(r)) for r in reach)]


MIB = 1 << 20


#: Core §7.4's words for a limit below a pool's need.
BELOW_NEED = ("Below the pool's need, SHM silently falls back to TCP, and a metadata segment that cannot be "
              "locked panics")
#: What a tool says when it cannot read the limit at all: observer §7.4
#: (text 0.2, F-290), "a limit the tool cannot read leaves the check
#: unobservable".
UNREAD = "the memlock check is unobservable: the limit could not be read"


def memlock_note(limit: int | None = None) -> str:
    """Observer §7.4 (core §7.4's until 0.58): "A tool SHOULD check the limit
    at start: the ``RLIMIT_MEMLOCK`` that core §7.4 asks a deployment to set
    to at least the pool plus 2 MiB". Text 0.2 (F-290), "What the check
    reports": "the largest pool its soft limit covers, the limit less
    2 MiB, or, below 2 MiB, that it admits none. An unlimited limit admits
    any pool", which zk2py's note said already, with what core §7.4 says of
    a limit below a pool's need, in its words. ``limit`` stands in for the
    process's soft limit (a reading's input); ``None`` reads it. It raises
    when the limit cannot be read: :func:`memlock_said` is what a tool
    calls, and reads that check unobservable."""
    if limit is None:
        import resource

        soft = resource.getrlimit(resource.RLIMIT_MEMLOCK)[0]
        limit = -1 if soft == resource.RLIM_INFINITY else soft
    if limit < 0:
        return "RLIMIT_MEMLOCK is unlimited: it admits any SHM pool (core §7.4)"
    if limit < 2 * MIB:
        return (f"RLIMIT_MEMLOCK is {limit} bytes, below 2 MiB: it admits no SHM pool (core §7.4: \"at least the "
                f"pool plus 2 MiB\"; \"{BELOW_NEED}\")")
    return (f"RLIMIT_MEMLOCK is {limit} bytes: it admits an SHM pool of at most {limit - 2 * MIB} bytes (core §7.4: "
            f"\"at least the pool plus 2 MiB\"; \"{BELOW_NEED}\")")


def memlock_said(note: Any = memlock_note) -> str:
    """The note a tool prints at start (observer §7.4), whatever went wrong
    reading it: any failure reads "the memlock check is unobservable: the
    limit could not be read: …" (text 0.2, F-290: "a limit the tool cannot
    read leaves the check unobservable"), and the tool goes on (review-bot
    on PR #949). ``note`` stands in for :func:`memlock_note` in a
    reading."""
    try:
        return note()
    except Exception as e:  # noqa: BLE001 - a note, never a reason to stop
        return f"{UNREAD}: {type(e).__name__}: {e}"


# -- observer §4.2, S4 through the routers' admin space (0.10 to 0.12, 0.48) ----------

#: observer §4.2 (0.11; the second selector since 0.48) "What the check reads".
S4_ROUTERS = "@/*/router"
S4_STORAGES = "@/*/router/status/plugins/storage_manager/storages/**"
#: 0.11's second selector, which 0.48 retired: its ``**`` intersects a
#: router's ``queryable/**`` and ``subscriber/**`` records. Named only so
#: that zk2py's readings can show it reaches them; never asked.
S4_STORAGES_011 = "@/*/router/**/storage_manager/storages/**"

#: observer §4.3 (0.48; any node's, 0.50, F-253) "What a tool does not ask": a node's
#: declaration records, its ``linkstate/*`` and its ``route/successor/**``,
#: which zenoh 1.10.1 answers while it holds its routing tables' read lock
#: (Appendix B), from :mod:`zk2py.admin_records`, which the generator's
#: §11.1 check reads too (:func:`zk2py.acl.block_hazards`).
ADMIN_RECORDS = admin_records.ADMIN_RECORDS


class AdminRecordsRefused(Exception):
    """A selector that intersects a node's declaration records, its
    ``linkstate/*`` or its ``route/successor/**`` (observer §4.3, 0.48, 0.50): "A
    tool MUST NOT ask a selector that intersects one of these keys of any
    node unless its user asks for that read". Not a ``ValueError``: the ``except ValueError``
    around a payload's JSON in zk2py's readers never swallows it."""


def admin_records_reached(selector: str) -> list[str]:
    """The patterns of :data:`ADMIN_RECORDS` that ``selector``'s key
    expression intersects, by zenoh 1.10.1's own intersection; empty when it
    reaches none. The selector is parsed as ``Session.get`` parses it
    (``zenoh.Selector``), so its parameters (``?…``) play no part. A
    selector zenoh cannot parse reaches nothing here: it reaches no router
    either, since ``Session.get`` refuses it with zenoh's own error."""
    try:
        ke = zenoh.Selector(selector).key_expr
    except zenoh.ZError:
        return []
    return [p for p in ADMIN_RECORDS if ke.intersects(zenoh.KeyExpr(p))]


def admin_guard(selector: str, *, user_asked: bool = False) -> None:
    """Refuse, before anything is sent, a selector that reaches a node's
    declaration records (observer §4.3, 0.48), unless the user asked for that read.
    Every GET zk2py makes through :func:`_answers` passes here; no zk2py
    reader asks one, and none takes a user's selector. A malformed selector
    passes, and fails where it failed before the guard: at the GET, with
    zenoh's ``ZError``."""
    reached = admin_records_reached(selector)
    if reached and not user_asked:
        raise AdminRecordsRefused(f"{selector} intersects {reached}: a router answers those records under its "
                                  f"routing tables' read lock, and a declaration between two replies freezes it "
                                  f"for good (observer §4.3, 0.48; core Appendix B)")
#: "an owner's state/** or @state/**", for every owner.
S4_STATE = ("zk2/*/*/*/state/**", "zk2/*/*/*/@state/**")


@dataclass
class S4Reading:
    """A tool's S4 check (observer §4.2): ``clean``, ``broken`` (a verified storage
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
    """The reply's replier id is the zid its key names (observer §4.2, 0.12), by
    value (0.11)."""
    if a.key is None or a.replier is None:
        return False
    named = _zid_value(a.key.split("/")[1])
    return named is not None and _zid_value(a.replier) == named


def verified_routers(session: zenoh.Session, records: list[Answer]) -> set[int]:
    """observer §4.2 (0.13) "Verified routers, outward": "the routers the tool's
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
    """observer §4.2 (0.12, 0.13) "Who answered": "A tool counts an answer as a
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
    """observer §4.2 (0.11; the second selector since 0.48): "The reference reads two
    selectors: @/*/router, the routers that answer;
    @/*/router/status/plugins/storage_manager/storages/**, one key per
    storage a router's storage manager runs, its value the storage's
    configuration with its key_expr. … A storage whose key_expr intersects
    an owner's state/** or @state/** breaks S4. A router that answers the
    first selector and has no storage answer runs no storage. When no router
    answers the first, the check is unobservable."

    0.48 "What a tool does not ask": neither selector intersects a router's
    declaration records, its ``linkstate/*`` or its ``route/successor/**``
    (:data:`ADMIN_RECORDS`), and :func:`_answers` refuses one that would.
    0.11's ``@/*/router/**/storage_manager/storages/**`` did: a router's
    ``router/queryable/<…/state/**>`` record answered it, under the read
    lock (scenarios/state.md §12).

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
        # observer §4.2 (0.15): "A storage is an answer to the second selector whose
        # key the selector includes, ending …/storage_manager/storages/<name>.
        # Any other answer is not a storage." Under 0.11's selector a
        # router's `router/queryable/<key expr>` record of a `**` queryable
        # answered it; under 0.48's none does, and a key beyond a storage's
        # name, or a reply off the selector, is still no storage.
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


# -- observer §4.4, a tool's S1 check (0.16 to 0.18) --------------------------------

def s1_check(session: zenoh.Session, descriptor: dict[str, Any], stamp_id: str | None,
             timeout: float = GET_TIMEOUT_S, trust: bool = False, on_table: bool = True) -> tuple[str, str]:
    """Observer §4.4, "A tool's S1 check" (0.16 to 0.18). "A tool attributes a state
    reply's stamp by comparing its id with the owner's meta.zid (core §3.3), by
    value":
    - "A foreign stamp is a finding whatever else the tool read: an owner
      that is its own router stamps with that router's id, which is its own
      meta.zid." A reply with no stamp is a finding too (S1, S2).
    - "An owner's stamp is clean only when the tool counted at least one
      router's own answer ("Who answered", above) and meta.zid is none of
      the zids it knows to be routers: the routers its session is connected
      to, the routers whose answers it counted, and every zid such an
      answer lists as a router session."
    - "A counted answer, not a connection" (0.18): a connected router is a
      verified router, but "a connection alone shows nothing about the
      owner".
    - "Otherwise S1 is unobservable for that owner, never clean."

    Without ``meta.zid`` the stamp is ``unattributable`` (observer §4.4). The counted
    answers are the ``@/*/router`` answers that "Who answered" verifies.
    ``trust`` is 0.12's operator alternative. Returns (verdict, why).

    **Only on core §4.3's table** (observer §4.4, text 0.15): "What a tool
    checks is the Zenoh timestamps core §4.3's table keeps … Outside the
    table, a router's stamp, or the reader's own session's, is expected on
    an owner's sample, and is never a finding. So is no stamp at all."
    ``on_table`` False is such a resource: not this check's, whatever the
    stamp (an in-band stamp is the owner's, since only the owner writes its
    payload)."""
    if not on_table:
        return "not_asked", ("outside core §4.3's table: an in-band stamp is the owner's, and a router's stamp, the "
                             "reader's own or none is expected there, never a finding (observer §4.4)")
    meta = descriptor.get("meta")
    zid = _zid_value(meta.get("zid") if isinstance(meta, dict) else None)
    if zid is None:
        return "unattributable", "the descriptor states no meta.zid (observer §4.4)"
    who = attribute_stamp(stamp_id, descriptor)
    if who != "owner":
        return "finding", f"the stamp is {who}: an owner that is its own router stamps with its own meta.zid"
    records = [a for a in _answers(session, S4_ROUTERS, zenoh.QueryTarget.ALL, timeout)
               if a.ok and a.key is not None]
    known = verified_routers(session, records)  # connected, the session, verified and listed
    if trust:
        known |= {_zid_value(a.key.split("/")[1]) for a in records}
    counted = [a for a in records if trust or unverified_why(a, known) is None]  # routers' own answers
    if zid in known:
        return "unobservable", "meta.zid is a router's zid: the owner is its own router"
    if not counted:
        return "unobservable", ("no router's own answer counted (no admin read, the admin space off, or no "
                                "replier id): the owner may be a router this tool cannot see")
    return "clean", f"the stamp is the owner's, and meta.zid is none of {len(known)} known routers"


# -- hostid.v1 §2.12: two sessions on one address of a minted system ---------------

def hostid_collision(session: zenoh.Session, address: str, grace_s: float = 2.0) -> tuple[str, str, list]:
    """hostid.v1 §2.12 (0.2, 0.6), and §5's last question: "Do two sessions
    claim one address of a minted system?" A tool "reads the instance tokens
    of an address … twice, a grace apart, and the descriptor of each
    instance. Counting only instances whose system is known to be minted,
    by §5's first question (yes: the descriptor lists hostid.v1, and no
    contract the instance implements lists it in uses), and not a
    recognised standby (0.6): when each read shows counted instances
    stating at least two different meta.zid … that is a finding whose
    cause is undecided."

    0.6's **recognised standby** "lists only interfaces whose contracts use
    redundancy.v1, at least one, and that profile's question 'is this
    instance a standby, or a twin?' answers no, standby, of each, from the
    same two reads". So each read is of the address's whole ``@zk/**``,
    instance and interface tokens at once (``redundancy.v1`` §0), and the
    standby question is :func:`zk2py.redundancy.twin` over the two.

    The contracts an instance implements are read by the fingerprints its
    descriptor names (core §8.4). Returns (answer, why, reads), the answer
    one of:
    - ``finding``, its cause undecided, naming none;
    - ``no``: one read, ended by the routers' final reply, with every
      descriptor in it read, shows the counted instances stating at most
      one meta.zid, "and no instance in it lists hostid.v1 without being
      counted or recognised a standby";
    - ``not this profile's``: no instance of the address lists hostid.v1;
    - ``unobservable``: otherwise. A descriptor could not be read, a counted
      instance states no meta.zid, an instance lists hostid.v1 while the
      first question is unobservable for it, an instance lists only
      governed interfaces while the standby question is unobservable for
      one, or a read timed out.

    Each read is (complete, [(instance, descriptor or None, first answer,
    standby: True recognised, False counted, None unobservable)])."""
    from . import hostid, redundancy as rd

    contracts: dict[tuple[str, str], dict[str, Any] | None] = {}

    def canonical(e: dict[str, Any]) -> dict[str, Any] | None:
        key = (str(e.get("iface")), str(e.get("contract")))
        if key not in contracts:
            r = retrieve_bundle(session, *key)
            contracts[key] = r.verified.contract if r.verified else None
        return contracts[key]

    def uses_of(doc: dict[str, Any]) -> set[str] | None:
        out: set[str] = set()
        for e in doc.get("interfaces") or []:
            c = canonical(e)
            if c is None:
                return None
            out |= set(c.get("uses") or [])
        return out

    system, service = address.split("/")
    pres_reads = []
    for i in range(2):
        if i:
            time.sleep(grace_s)
        pres = list_presence(session, f"zk2/{address}/@zk/**")
        docs: dict[str, dict[str, Any] | None] = {}
        for inst in pres.instances:
            if inst["system"] != system or inst["service"] != service:
                continue
            got = [a for a in get_descriptor(session, f"zk2/{address}/@zk/instance/{inst['instance']}") if a.ok]
            try:
                doc = json.loads(got[0].payload) if len(got) == 1 else None
            except ValueError:
                doc = None
            docs[inst["instance"]] = doc if isinstance(doc, dict) else None
        pres_reads.append((pres, docs))

    def red_read(k: int, iface: str) -> rd.Read:
        pres, docs = pres_reads[k]
        holders = {a["instance"] for a in pres.alive if a["system"] == system and a["service"] == service
                   and a["iface"] == iface}
        seen = []
        for inst in sorted(set(docs) | holders, key=rd.id_key):
            doc = docs.get(inst)
            desc = rd.UNREAD if doc is None else rd.NOT_LISTED
            for e in (doc or {}).get("interfaces") or []:
                if isinstance(e, dict) and e.get("iface") == iface:
                    m, _ = rd.mark(e, "no_contract")
                    desc = rd.UNREAD if m is None else m
            seen.append(rd.Seen(inst, inst in holders, desc))
        return rd.Read(pres.complete, tuple(seen))

    def standby(inst: str, doc: dict[str, Any] | None) -> bool | None:
        """0.6: True a recognised standby, False counted, None unobservable
        (it lists only governed interfaces, and the question is
        unobservable for one)."""
        if doc is None:
            return None
        entries = [e for e in doc.get("interfaces") or [] if isinstance(e, dict)]
        if not entries:
            return False
        answers = []
        for e in entries:
            c = canonical(e)
            if c is None:
                answers.append(None)
                continue
            d = rd.declaration_of(c)
            if not d.governed:
                return False  # "an instance that lists an interface no contract of the profile governs"
            reading = rd.Reading(d.election or rd.CLAIM, red_read(0, str(e["iface"])), red_read(1, str(e["iface"])))
            a = rd.twin(reading, inst)
            answers.append(True if (a.answer, a.reason) == (rd.NO, "standby")
                           else None if a.answer == rd.UNOBSERVABLE else False)
        if False in answers:
            return False
        return None if None in answers else True

    reads = []
    for pres, docs in pres_reads:
        rows = []
        for inst, doc in docs.items():
            first = hostid.minted_by_listing(doc, uses_of(doc))[0] if doc is not None else "unread"
            rows.append((inst, doc, first, standby(inst, doc) if first == "yes" else False))
        reads.append((pres.complete, rows))

    def counted(rows):
        return [d for _, d, first, sb in rows if first == "yes" and sb is False]

    def zids(rows):
        return {_zid_value((d.get("meta") or {}).get("zid")) for d in counted(rows)} - {None}

    def lists(d):
        return d is not None and "hostid.v1" in (d.get("profiles") or [])

    if all(len(zids(rows)) >= 2 for _, rows in reads):
        return "finding", ("in both reads the counted instances state at least two meta.zid: its cause is "
                           "undecided (a collision, a cloned machine id, or a second process)"), reads
    if all(all(d is not None and not lists(d) for _, d, _, _ in rows) for _, rows in reads):
        return "not this profile's", "no instance of the address lists hostid.v1", reads
    for complete, rows in reads:
        every_read = all(d is not None for _, d, _, _ in rows)
        every_zid = all(_zid_value((d.get("meta") or {}).get("zid")) is not None for d in counted(rows))
        unaccounted = [d for _, d, first, sb in rows if lists(d) and not (first == "yes" and sb is not None)]
        if complete and every_read and every_zid and not unaccounted and counted(rows) and len(zids(rows)) <= 1:
            # redundancy.v1 §2.15 (0.3): a recognised standby "which a tool
            # names beside its answer"
            recognised = sorted((inst for inst, _, first, sb in rows if first == "yes" and sb is True), key=str)
            return "no", ("one complete read, every descriptor read, shows at most one meta.zid"
                          + (f", and {len(recognised)} recognised standby, not counted: "
                             f"{', '.join(recognised)} (redundancy.v1 §2.15, 0.3; hostid.v1 §2.12)"
                             if recognised else "")), reads
    return "unobservable", ("the finding is not established, and a descriptor could not be read, a counted "
                            "instance states no meta.zid, an instance lists hostid.v1 while whether it is "
                            "minted is unobservable, one lists only governed interfaces while whether it "
                            "stands by is unobservable, or a read timed out"), reads


# -- observer §7.2, O3 judged from outside (0.16, 0.17) ---------------------------------

#: Observer §7.2's finding, with what it says beside it (0.16: "It says so
#: beside the finding"; core 0.59's operations.md §5 step 2: "says beside it
#: that an access-control refusal is silent too").
TOLD_FINDING = ("silent, from a present owner, under grants that let this caller call: the O3 finding; "
                "an access-control refusal is silent too (O5, core §11.3), so it rests on what the tool was told")
#: Not told (operations.md §5 step 2): "it reports the silence
#: unobservable, never the finding, and names the premise it lacked: grants
#: that let it call, which no tool can observe (core §11.3)".
NOT_TOLD = ("silent, and not told that its grants let it call: unobservable, never the finding; the premise it "
            "lacked is grants that let it call, which no tool can observe (core §11.3; O5)")

def o3_verdict(result: CallResult, may_call: bool | None, present: bool | None = None) -> tuple[str, str]:
    """Observer §7.2, O3 judged from outside (0.16, 0.17). "No tool can observe its
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
        return "unobservable", NOT_TOLD
    if may_call is False:
        return "unobservable", "silent, and told that its grants do not let it call: a refusal is silent too"
    if present is False:
        return "unobservable", "silent, and the owner's tokens are not read: absent, not unanswered (O5)"
    return "finding", TOLD_FINDING


# -- §2.6 events replay, the consumer's bound (0.16), by form 4's time (0.103) --

@dataclass(frozen=True)
class Occurrence:
    """One replayed occurrence (core §2.6, 0.103), read through observer
    §7.3's first step: its key, its stamp, the key's occurrence chunk's
    (§1.2), its time, form 4's (§7.5), ``{"s", "ms"}`` from the deployment
    epoch or None for the unknown time, that time as Unix ms under the
    epoch the reader holds, and its record and CH's tag."""

    key: str
    stamp: zk2c.InBand
    time: dict[str, int] | None
    at_ms: int | None
    record: bytes
    tag: int = 0

    @property
    def member(self) -> str:
        """Its key without its occurrence chunk (core §2.7)."""
        return self.key.rsplit("/", 1)[0]


@dataclass
class Replay:
    """A replay as a consumer delivers it (core §2.6):
    - ``delivered``: every occurrence kept, in §2.6's order;
    - ``dropped``: those whose time is beyond the retention;
    - ``undecodable``: replies on an events key whose envelope was refused,
      ``(key, token)``, or that carry no event, ``(key, "form")`` (core 0.104
      §2.6, F-514's answer), a ``reply_del`` there among them (core 0.105,
      F-522's answer), never delivered;
    - ``not_zk2``: replies on keys the grammar refuses, a ULID's key among
      them (core 0.103 §1.2), which no reader resolves."""

    delivered: list[Occurrence] = field(default_factory=list)
    dropped: list[Occurrence] = field(default_factory=list)
    undecodable: list[tuple[str, str]] = field(default_factory=list)
    not_zk2: list[str] = field(default_factory=list)


#: Core §2.6 (0.104, 0.105): the token a consumer counts a sample on an events
#: key under when it carries no event and §7.5's parse refused nothing, "as
#: §4.2 counts a stray reply_del".
NO_EVENT = "form"


def no_event(key: str, read: zk2c.Read | None, deleted: bool = False) -> str | None:
    """Core §2.6 (0.105), "A sample that carries no event": "A consumer MUST
    NOT deliver a put on an events key whose payload is no event, such as a
    mutation or a payload with no envelope, or a delete on an events key,
    whether a subscription heard it or a replay returned it." Its count:
    "Where §7.5's parse refuses its envelope, the count names that refusal,
    such as short, sample_kind or key_stamp … Any other is refused as form
    …: its envelope reads as another form, or it carries none."

    ``read`` is the sample read through §7.5's parse (:func:`zk2c.read_put`,
    or for a delete :func:`delete_read`), or None for a sample that carries
    none. Returns the token the consumer counts the sample under, or None
    for an event, or a key that is no zk2 events key. A delete
    (``deleted``) is never an event: its attachment's refusal, else
    ``form``. A delete with no attachment carries none, ``form``, as core
    0.105's entry says ("one that reads as a delete, or none, as form";
    F-527, zk2py's reading)."""
    if zk2c.key_token(key) != "events":
        return None
    if read is None:
        return NO_EVENT
    if read.refused:
        return read.refusal or "refused"
    if deleted or read.form != "event" or read.stamp is None:
        return NO_EVENT
    return None


def delete_read(key: str, encoding: str, attachment: bytes | None) -> zk2c.Read | None:
    """A delete's envelope, its attachment, read through §7.5's parse; None
    where it has no attachment, which carries none (:func:`no_event`)."""
    return None if attachment is None else zk2c.read_delete(key, encoding, attachment)


def sample_no_event(sample: zenoh.Sample) -> str | None:
    """:func:`no_event` of a zenoh sample, a put's payload or a delete's
    attachment read through §7.5's parse."""
    key = str(sample.key_expr)
    if sample.kind == zenoh.SampleKind.DELETE:
        att = sample.attachment
        return no_event(key, delete_read(key, str(sample.encoding), None if att is None else att.to_bytes()),
                        deleted=True)
    return no_event(key, zk2c.read_put(key, str(sample.encoding), sample.payload.to_bytes()))


def occurrence_of(key: str, encoding: str, payload: bytes, epoch_s: int = 0) -> Occurrence | tuple[str, str] | None:
    """A put on ``key`` read as an occurrence: an :class:`Occurrence`, or
    ``(key, token)`` where its envelope is refused or it carries no event
    (:func:`no_event`), or None where the key is no zk2 events key."""
    parsed = zk2c.data_key(key)
    if parsed is None or parsed.get("form") != "data" or parsed.get("token") != "events":
        return None
    read = zk2c.read_put(key, encoding, payload)
    token = no_event(key, read)
    if token is not None:
        return (key, token)
    t = None if read.time == zk2c.UNKNOWN_TIME else read.time
    at = None if t is None else (epoch_s + t["s"]) * 1000 + t["ms"]
    return Occurrence(key, read.stamp, t, at, read.record or b"", read.tag or 0)


def replay_filter(occs: list[Occurrence], retention_s: int, now_ms: int) -> tuple[list[Occurrence], list[Occurrence]]:
    """Core §2.6's filter (0.103): "A consumer MUST keep a replayed occurrence
    whose time (§7.5's form 4) is within the retention, and drop one whose
    time is beyond it. A consumer MUST NOT judge an unknown time within the
    retention or beyond it: the storage's ``_time`` bound alone admitted
    that occurrence." An unknown time is kept. Returns (kept, dropped)."""
    bound = now_ms - retention_s * 1000
    kept = [o for o in occs if o.at_ms is None or o.at_ms >= bound]
    return kept, [o for o in occs if o.at_ms is not None and o.at_ms < bound]


def replay_order(occs: list[Occurrence]) -> list[Occurrence]:
    """Core §2.6's order (0.103): "A consumer MUST deliver a replay in the
    order of the occurrences' times, then their seqs within one run. The
    occurrences of an unknown time come after every one with a time, by seq
    within one run." "Between two runs at one time, and among the unknown
    times of two runs, the order is unspecified": zk2py breaks that tie as
    the reference does, by the run's value, a 16-bit run before a 32-bit
    one, so that two replays of one storage agree."""
    def rank(o: Occurrence) -> tuple[int, int, int, int, int]:
        known = 0 if o.at_ms is not None else 1
        return (known, o.at_ms or 0, o.stamp.width, o.stamp.run, o.stamp.seq)

    return sorted(occs, key=rank)


def replay_events(session: zenoh.Session, selector: str, retention_s: int, now_ms: int | None = None,
                  epoch_s: int = 0, timeout: float = GET_TIMEOUT_S) -> Replay:
    """Core §2.6: "A consumer replays with a wildcard GET bounded by the
    retention", and (0.16) "The retention is the bound a consumer applies
    on replay": a union storage prunes nothing by retention. The GET asks
    with ``_time``; a backend may ignore it (the memory backend does, spike
    S5), so the consumer filters by each event's time (0.103,
    :func:`replay_filter`) and delivers by §2.6's order
    (:func:`replay_order`). ``epoch_s`` is the deployment epoch the reader
    holds (§7.6: the catalog's in force, else 0)."""
    now_ms = int(time.time() * 1000) if now_ms is None else now_ms
    out = Replay()
    occs: list[Occurrence] = []
    for a in _answers(session, f"{selector}?_time=[now(-{retention_s}s)..]", zenoh.QueryTarget.ALL, timeout):
        if not a.ok or a.key is None:
            continue
        if a.deleted and zk2c.key_token(a.key) == "events":
            # Core 0.105 §2.6: a reply_del on an events key is never
            # delivered, counted by its attachment's refusal, else form.
            out.undecodable.append((a.key, no_event(a.key, delete_read(a.key, a.encoding or "", a.attachment),
                                                    deleted=True) or NO_EVENT))
            continue
        o = occurrence_of(a.key, a.encoding or "", a.payload, epoch_s)
        if o is None:
            out.not_zk2.append(a.key)
        elif isinstance(o, tuple):
            out.undecodable.append(o)
        else:
            occs.append(o)
    kept, out.dropped = replay_filter(occs, retention_s, now_ms)
    out.delivered = replay_order(kept)
    return out


def subscribe_events(session: zenoh.Session, selector: str, deliver: Any, undecodable: Any,
                     epoch_s: int = 0) -> Any:
    """A consumer's subscription to an event (core §2.6): each put heard on
    an events key read as :func:`occurrence_of` reads a replay's, so that
    "whether a subscription heard it or a replay returned it" one rule holds
    (0.104). ``deliver`` gets each :class:`Occurrence`; ``undecodable`` gets
    ``(key, token)`` for a put that carries no event (``form``) or whose
    envelope is refused, never delivered. A delete delivers nothing (core
    0.105 §2.6: "or a delete on an events key"): it is counted by its
    attachment's refusal, else ``form`` (:func:`sample_no_event`). Returns
    the subscriber."""
    def received(sample: zenoh.Sample) -> None:
        if sample.kind != zenoh.SampleKind.PUT:
            token = sample_no_event(sample)
            if token is not None:
                undecodable((str(sample.key_expr), token))
            return
        o = occurrence_of(str(sample.key_expr), str(sample.encoding), sample.payload.to_bytes(), epoch_s)
        if isinstance(o, tuple):
            undecodable(o)
        elif o is not None:
            deliver(o)

    return session.declare_subscriber(selector, zenoh.handlers.Callback(received))


# -- core S5, §4.4: last-known state from an archive, explicitly (round 107) --------

def archive_key(archive: str, origin: str) -> str:
    """Core §4.4: "Its key is the archived key without its leading ``zk2``
    chunk, slugged as a rest parameter, under the archive's own ``@state``
    token: ``zk2/<system>/<archive>/archive.v1/@state/{origin...}``". The
    archive's address is ``<system>/<service>``."""
    from . import slug

    chunks = origin.split("/")
    if chunks[0] != "zk2":
        raise ValueError(f"{origin}: not a zk2 key")
    return f"zk2/{archive}/archive.v1/@state/" + "/".join(slug.slug(c) for c in chunks[1:])


@dataclass
class LastKnown:
    """An archive's answer for one origin (core S5, §4.4): the bytes the
    owner put, its envelope included (``raw``), their record and in-band
    stamp, the reply's attachment (``{"iface", "contract", "type",
    "confirmed"}``), and whether it was a deletion. "The archive's answer is
    last-known, never current, and a consumer MUST NOT present it as
    current" (S6): :meth:`shown`."""

    key: str
    origin: str
    raw: bytes
    record: bytes | None
    stamp: Any
    attachment: dict[str, Any]
    deleted: bool = False

    def shown(self) -> str:
        conf = "confirmed" if self.attachment.get("confirmed") is True else "unconfirmed"
        what = "deleted" if self.deleted else f"{len(self.record or b'')} bytes"
        return f"last-known ({conf}), from an archive: {what} of {self.origin}, {self.stamp}"


def last_known(session: zenoh.Session, archive: str, origin: str,
               timeout: float = GET_TIMEOUT_S) -> LastKnown | None:
    """Core S5: "last-known state MUST be read from an archive (§4.4),
    explicitly": a GET of ``origin``'s key at ``archive`` (target ``All``,
    consolidation ``None``), its reply read envelope first (observer §7.3:
    the owner's put, as the archive keeps it). None for no reply."""
    key = archive_key(archive, origin)
    out = None
    for rep in session.get(key, target=zenoh.QueryTarget.ALL, consolidation=zenoh.ConsolidationMode.NONE,
                           timeout=timeout):
        if rep.ok is None:
            continue
        s = rep.ok
        att = s.attachment.to_bytes() if s.attachment is not None else b""
        try:
            doc = json.loads(att) if att else {}
        except ValueError:
            doc = {}
        deleted = s.kind == zenoh.SampleKind.DELETE
        raw = s.payload.to_bytes()
        rec, stamp = None, None
        if not deleted:
            try:
                env = zk2c.parse(raw, "put", zk2c.key_token(origin), zk2c.last_chunk(origin))
                rec, stamp = env.record, env.stamp
            except zk2c.Refused:
                pass
        out = LastKnown(str(s.key_expr), origin, raw, rec, stamp, doc if isinstance(doc, dict) else {}, deleted)
    return out


# -- §4 state, a consumer's GET --------------------------------------------------

@dataclass
class StateReply:
    key: str
    deleted: bool                # a reply_del (S2, S3)
    payload: bytes               # the envelope's record (observer §7.3, 0.14), or the bytes with none
    encoding: str
    stamp: str | None            # Zenoh's "<ntp64>/<id hex>" text
    stamp_id: str | None         # the id half: the HLC that issued it (§4.1)
    stamp_ns: int | None = None  # the stamp's time, ns since the UNIX epoch
    #: core §4.3 (0.73): the in-band stamp its envelope (a reply_del's
    #: attachment) carries, or None
    inband: Any = None
    #: core §7.5's token where its envelope was refused: undecodable
    #: (observer §7.3), its ``payload`` empty, never read as a payload
    refusal: str | None = None
    #: the bytes as they arrived, the envelope included (S2: "a reply as
    #: the bytes the mutation put")
    raw: bytes = b""
    #: a reply_del's attachment as it arrived
    attachment: bytes | None = None
    #: core 0.100 (#977 E3): where the value came as a snapshot's entry, the
    #: snapshot's key it rode (its member's key is ``key``), and what the
    #: entry says of its confirmation (§7.5): ``none``, ``time`` (vouched for
    #: at ``snapshot_time``) or ``conf`` (its last confirmation, ``conf``),
    #: each time ``{"s", "ms"}`` from the deployment epoch, None unknown
    snapshot_key: str | None = None
    confirmation: str | None = None
    conf: dict[str, int] | None = None
    snapshot_time: dict[str, int] | None = None
    #: the run of the snapshot that listed it, its header's, with the
    #: snapshot's asof as its seq: "the run its instance wrote under as it
    #: answered" (``alarms.v1`` 0.14 §2.14 step 5, F-503's answer)
    snapshot_run: Any = None


@dataclass
class StateReading:
    """The owner's answer to one state GET. ``silent`` is no reply at all:
    "That silence is not a verdict about the key" (S6, O5). ``replies``
    holds one value per key, merged per key (core 0.100 S4), a snapshot's
    entries among them; ``snapshots`` each snapshot the GET brought, with
    its pages and whether it is complete (§7.5); ``arrivals`` every reply as
    it came, ``(key, what, stamp)``, ``what`` ``put``, ``delete``, ``page``
    or ``refused``, ``stamp`` its Zenoh timestamp's text or None."""

    replies: list[StateReply] = field(default_factory=list)
    errors: list[str] = field(default_factory=list)
    snapshots: list[Any] = field(default_factory=list)
    arrivals: list[tuple[str, str, str | None]] = field(default_factory=list)

    @property
    def silent(self) -> bool:
        return not self.arrivals and not self.errors

    def complete_for(self, key: str) -> list[Any]:
        """The complete snapshots heard on ``key`` or above it (core §4.2:
        "on a key K that is its collection's snapshot key or a member's
        key")."""
        return [h for h in self.snapshots if h.done and (key == h.key or key.startswith(h.key + "/"))]


def get_state(session: zenoh.Session, selector: str, timeout: float = GET_TIMEOUT_S,
              across: Any = None, revision: Any = None) -> StateReading:
    """S4 (core 0.100): "a state GET MUST be addressed to the owner's keys,
    with target All and consolidation None set explicitly. The consumer MUST
    merge the GET's replies per key, each snapshot's entries among them, as a
    late joiner's seed merges them". Current state is the owner's answer
    (S6). §3.2 R6: a reply on a key that is not concrete is discarded.
    ``across``, a :class:`zk2py.far_side.FarSide`: the GET waits at least the
    face's reply timeout (core §8.1, 0.91).

    A reply whose payload is a snapshot's page (§7.5) is read as its
    entries, each "on its member's key" with its stamp, merged as replies
    are: within one run the higher seq, an equal stamp kept once, another
    run the later arrival (§4.2, observer §4.1). One GET holds no view
    before it, so its omission rule drops nothing (every value it holds was
    heard after it asked); :attr:`StateReading.snapshots` says which were
    complete, for a reader that holds one.

    **A stray ``reply_del``** (core 0.102 §4.2): with ``revision``, a
    :class:`zk2py.seed_snapshot.ByDescriptor` over the revisions held, a
    ``reply_del`` on a member its owner's revision answers by snapshot (for
    the run of its stamp) is "no owner's answer": it is counted undecodable,
    refused as ``form``, and never merged as a deletion. Without the owner's
    revision every ``reply_del`` is merged, as the text says a reader then
    does. The descriptors are read before the GET."""
    from . import seed as sd
    from . import seed_snapshot as ss
    from . import snapshot as snap_mod

    if across is not None:
        timeout = across.get_timeout_s(timeout)
    if revision is not None and hasattr(revision, "prepare"):
        revision.prepare(session, selector)
    q: queue.Queue = queue.Queue()
    session.get(selector, zenoh.handlers.Callback(q.put, lambda: q.put(_DONE)),
                target=zenoh.QueryTarget.ALL, consolidation=zenoh.ConsolidationMode.NONE,
                timeout=timeout)
    out = StateReading()
    tracker = ss.Tracker()
    held: dict[str, StateReply] = {}
    marks: dict[str, Any] = {}

    def merge(r: StateReply, mark: Any) -> None:
        if r.key in held:
            rel = sd.relation(mark, marks[r.key], sd.kind_of(r.key))
            if rel not in ("greater", "unordered"):
                return
        held[r.key] = r
        marks[r.key] = mark

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
        read = zk2c.read_sample(s)
        raw = s.payload.to_bytes()
        att = None if s.attachment is None else s.attachment.to_bytes()
        deleted = s.kind == zenoh.SampleKind.DELETE
        if not deleted and read.form == "snapshot":
            try:
                page = ss.read_page_of(key, raw)
            except (snap_mod.Refused, zk2c.Refused) as e:
                out.arrivals.append((key, "refused", None if ts is None else str(ts)))
                out.replies.append(StateReply(key, False, b"", str(s.encoding), None, None, None, None,
                                              getattr(e, "token", "snapshot"), raw, att))
                continue
            out.arrivals.append((key, "page", None if ts is None else str(ts)))
            tracker.add(key, page)
            head = zk2c.InBand(page["run_width"], page["run"], page["asof"])
            for m in ss.members_of(key, page):
                merge(StateReply(m.key, m.deleted, m.record, zk2c.ENCODING, None, None, None, m.inband, None,
                                 m.put_bytes() if not m.deleted else b"", m.put_bytes() if m.deleted else None,
                                 key, m.confirmation, m.conf, m.time, head), sd.Mark(m.inband, None))
            continue
        out.arrivals.append((key, "delete" if deleted else "put", None if ts is None else str(ts)))
        r = StateReply(
            key, deleted, b"" if deleted or read.refused else (read.record if read.record is not None else raw),
            str(s.encoding),
            None if ts is None else str(ts),
            None if ts is None else str(ts).split("/", 1)[1],
            None if ts is None else ts.get_time_as_ntp64().as_nanos(),
            read.stamp, read.refusal, raw, att)
        if read.refused:
            out.replies.append(r)
            continue
        if deleted and revision is not None and revision.per_key(key, read.stamp) is False:
            # core 0.102 §4.2: "A consumer or a tool MUST NOT merge a stray
            # reply_del as a deletion. It counts it undecodable, refused as
            # form"
            out.arrivals[-1] = (key, "refused", out.arrivals[-1][2])
            r.refusal = "form"
            out.replies.append(r)
            continue
        merge(r, sd.Mark(read.stamp, sd.Stamp.of(ts)))
    out.replies.extend(held.values())
    out.snapshots = list(tracker.heard)
    return out


# -- §5 operations, a caller's call ----------------------------------------------

#: The kinds of a call's reply (core 0.99 §5.1, §5.2): ``value``, a reply
#: whose value form was read; ``refused_value``, a reply the parse or its
#: type refuses, or one under another Encoding, "malformed, and an answer all
#: the same"; ``error``, an owner's error, an error form the parse accepts;
#: ``transport``, every other reply error, "one the parse refuses, and one
#: under another Encoding", silence (§5.1, "Retries").
ANSWERS = ("value", "refused_value", "error")


@dataclass
class CallReply:
    kind: str            # "value", "refused_value", "error", "transport"
    key: str | None
    encoding: str
    #: a value's record, the bytes every later step decodes (observer §7.3:
    #: "The value's record is the payload every later step decodes"); the
    #: reply error's bytes for an error and the transport's; b"" for a
    #: refused value, whose bytes are never handed on
    payload: bytes
    #: an owner's error: code, cause, message and detail (the detail its
    #: bytes, the error type's own)
    error: dict[str, Any] | None = None
    #: the token of the refusal: a refused value's, or §7.5's for a reply
    #: error read as the transport's, ``encoding`` for another Encoding
    refusal: str | None = None
    #: the replying instance's as-of mark (core §5.1), and CH's tag; None
    #: for a refused value and the transport's
    mark: zk2c.InBand | None = None
    tag: int | None = None
    #: the payload as it came
    raw: bytes = b""
    #: O6: a reply whose attachment is the ASCII bytes ``summary``
    summary: bool = False

    @property
    def envelope(self) -> dict[str, Any] | None:
        """The owner's error (the name it had while every error was an
        earlier core's envelope)."""
        return self.error

    @property
    def code(self) -> str | None:
        return None if self.error is None else self.error.get("code")


@dataclass
class CallResult:
    replies: list[CallReply] = field(default_factory=list)
    #: seconds from the call to its first reply, and to the query's
    #: completion (None when ``first`` returned before it)
    first_s: float | None = None
    done_s: float | None = None
    #: the attempts made (O4: more than one only for an idempotent
    #: operation, each after silence), the last one's replies above
    attempts: int = 1

    @property
    def silent(self) -> bool:
        """O5: "MUST NOT treat an empty reply set as a verdict". §5.1 (0.94):
        "A call that ends there with no value and no error is silent", and
        "A reply_err that is the transport's is silence too, a malformed
        error among them"; "A value is an answer, and so is a reply that does
        not parse as one"."""
        return not any(r.kind in ANSWERS for r in self.replies)


def read_reply(sample: Any) -> CallReply:
    """One reply of a call read as core 0.99 §5.1 reads it: under
    ``zenoh/bytes`` by §7.5's parse, its record "the payload every later
    step decodes"; a refused one is ``refused_value``, never decoded or
    sniffed as a bare record (observer §7.3). "Under another Encoding a
    reply is no value form, so it is malformed too": a ``refused_value``
    whose refusal is ``encoding``, an answer all the same, whatever its
    bytes ("a record sent with no form can parse as one")."""
    from . import calls

    key, enc, data = str(sample.key_expr), str(sample.encoding), sample.payload.to_bytes()
    att = None if sample.attachment is None else sample.attachment.to_bytes()
    summ = att == b"summary"
    if not calls.in_forms(enc):
        return CallReply("refused_value", key, enc, b"", refusal="encoding", raw=data, summary=summ)
    try:
        e = zk2c.parse(data, "reply")
    except zk2c.Refused as r:
        return CallReply("refused_value", key, enc, b"", refusal=r.token, raw=data, summary=summ)
    return CallReply("value", key, enc, e.record or b"", mark=e.stamp, tag=e.tag, raw=data, summary=summ)


def read_reply_error(encoding: str, data: bytes) -> CallReply:
    """One reply error of a call read as core 0.99 §5.2 reads it: "A caller
    or a tool MUST read a reply_err under the Encoding zenoh/bytes by §7.5's
    parse. It holds an error the parse accepts as the owner's. A caller or a
    tool MUST read every other reply_err as the transport's: one the parse
    refuses, and one under another Encoding." An earlier core's envelope,
    under ``application/json``, ``application/cbor`` or ``zk2.core.v1.Error``,
    is under another Encoding, so the transport's: "That is silence"
    (0.99's entry)."""
    if encoding == zk2c.ENCODING:
        try:
            e = zk2c.parse(data, "reply_err")
        except zk2c.Refused as r:
            return CallReply("transport", None, encoding, data, refusal=r.token, raw=data)
        assert e.error is not None
        err = {"code": e.error.code, "cause": e.error.cause, "message": e.error.message, "detail": e.error.detail}
        return CallReply("error", None, encoding, data, error=err, mark=e.stamp, tag=e.tag, raw=data)
    return CallReply("transport", None, encoding, data, refusal="encoding", raw=data)


def show_reply(r: CallReply, op: Any = None) -> dict[str, Any]:
    """A call's reply as a tool shows it, observer 0.24 §7.3, "A call's forms
    first":
    - a value by its record, "decoded … as the operation's response type, or
      its summary type on a summary reply" where ``op`` (a
      :class:`zk2py.calls.Operation`) is held, a packed one under CH's tag,
      else its bytes in hex;
    - "A refused value is undecodable, and the tool names the refusal", its
      bytes never decoded nor sniffed;
    - "An error is shown by its code, its cause and its message. Its detail
      is decoded as the operation's error type where the tool holds it, a
      packed one under CH's tag, and shown as its bytes otherwise";
    - "The transport's … is shown so, with its Encoding and its bytes, never
      as an owner's error";
    - "The as-of mark. A tool may show a reply's run and seq beside it": its
      run and seq, never an instance (the run "names an instance only
      through the descriptor that states it").
    A reply under another Encoding is a refused value, "undecodable as a
    refused one is" (observer 0.27 §7.3)."""
    from . import calls, packed

    out: dict[str, Any] = {"kind": r.kind, "encoding": r.encoding}
    if r.mark is not None:
        out["mark"] = {"run": r.mark.run_hex(), "seq": r.mark.seq}
    if r.kind == "transport":
        out["transport"] = {"encoding": r.encoding, "bytes": r.raw.hex(), "refusal": r.refusal}
        return out
    if r.kind == "refused_value":
        out["undecodable"] = r.refusal
        return out
    if r.kind == "error":
        e = r.error or {}
        out.update({"code": e.get("code"), "cause": e.get("cause"), "message": e.get("message")})
        d = e.get("detail")
        if isinstance(d, (bytes, bytearray)):
            shown: Any = bytes(d).hex()
            if op is not None and op.types.get("error") is not None:
                try:
                    v = op.decode("error", r.tag or 0, bytes(d))
                    shown = v.hex() if isinstance(v, bytes) else v
                except (packed.Refused, calls.Refused) as x:
                    shown = {"undecodable": x.token, "bytes": bytes(d).hex()}
            out["detail"] = shown
        else:
            out["detail"] = d
        return out
    role = calls.value_role(r.summary)
    value: Any = r.payload.hex()
    if op is not None and op.types.get(role) is not None:
        try:
            v = op.decode(role, r.tag or 0, r.payload)
            value = v.hex() if isinstance(v, bytes) else v
        except (packed.Refused, calls.Refused) as x:
            value = {"undecodable": x.token}
    out["value"] = value
    if r.summary:
        out["summary"] = True
    return out


def call(session: zenoh.Session, key: str, payload: bytes = b"", *, fanout: bool = False,
         encoding: str | None = None, timeout: float | None = GET_TIMEOUT_S, first: bool = False,
         attachment: bytes | None = None, recommended_ms: int | None = None, across: Any = None,
         tag: int = 0, op: Any = None, written: bool = False, attempts: int = 1,
         idempotent: bool = False) -> CallResult:
    """Call an operation (§5.1).

    - **The request (core 0.94 §5.1).** "A caller MUST write its request as
      §7.5's request, ``CH · record``, under the Encoding zenoh/bytes. CH's
      width bit is 0, and its tag is the request type's": ``payload`` is
      the record, ``tag`` the ``request`` type's, or ``op``'s (a
      :class:`zk2py.calls.Operation`). ``written``: ``payload`` is the
      request form as its caller wrote it, CH included, sent unchanged
      under zenoh/bytes (a profile's caller that writes its own form, or a
      test's malformed request). No caller sends under another Encoding:
      the bare wire is gone since core 0.99.
    - **The timeout** (§5.1, §8.1 at 0.91): ``timeout``, the caller's, at
      its word; ``None`` sets none, and the call waits the operation's
      ``timeout_ms`` (``recommended_ms``), else 10 s, and across a face
      (``across``, a :class:`zk2py.far_side.FarSide`) at least the face's
      reply timeout (:func:`zk2py.far_side.call_timeout_s`).
    - A concrete call uses target ``BestMatching`` (O1); a call to a fan-out
      operation, target ``All`` and consolidation ``None`` (O2).
    - Consolidation is ``None`` for a concrete call too: §5.1 O1 (0.7) says
      "a concrete call MUST set BestMatching and None", since ``Latest``
      would hold the reply until the query completes.
    - ``first``: a one-reply caller "takes the first value on the call's
      key, or the first error, without waiting for the query to complete"
      (§5.1 "A concrete call"). Otherwise every reply is collected until the
      query completes.
    - **The replies** (core 0.94): each read by :func:`read_reply` and
      :func:`read_reply_error`. No reply, and the transport's alone, is
      silence, never a verdict (O5).
    - **Retries (O4).** ``attempts`` above 1 calls again after silence
      alone, and only an ``idempotent`` operation: "calls an operation that
      is not idempotent once, whatever the configuration", and never a
      fan-out ("The reference retries no fan-out").
    - ``attachment``: the call metadata (O7), ``{"actor", "request_id"}``.
    """
    from . import far_side

    timeout = far_side.call_timeout_s(timeout, recommended_ms, across)
    if op is not None:
        tag = op.tag("request")
    kwargs: dict[str, Any] = {
        "target": zenoh.QueryTarget.ALL if fanout else zenoh.QueryTarget.BEST_MATCHING,
        "consolidation": zenoh.ConsolidationMode.NONE,
        "timeout": timeout,
    }
    if encoding is not None and encoding != zk2c.ENCODING:
        raise ValueError(f"a request travels under {zk2c.ENCODING}, never {encoding} (core 0.99 §5.1)")
    kwargs["payload"] = payload if written else zk2c.request(payload, tag)
    kwargs["encoding"] = zk2c.ENCODING
    if attachment is not None:
        kwargs["attachment"] = attachment
    tries = attempts if idempotent and not fanout else 1
    out = CallResult()
    for n in range(1, max(tries, 1) + 1):
        out = _call_once(session, key, kwargs, timeout, first)
        out.attempts = n
        if not out.silent:
            break
    return out


def _call_once(session: zenoh.Session, key: str, kwargs: dict[str, Any], timeout: float,
               first: bool) -> CallResult:
    q: queue.Queue = queue.Queue()
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
            out.replies.append(read_reply(item.ok))
        else:
            out.replies.append(read_reply_error(str(item.err.encoding), item.err.payload.to_bytes()))
        last = out.replies[-1]
        if first and last.kind != "transport" and (last.kind == "error" or last.key == key):
            break
    return out

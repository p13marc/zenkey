"""The live half, first slice: presence, descriptors and contract retrieval
over Zenoh (core.md §3.3, §8.1–§8.4), on zenoh-python 1.10.1.

Everything here is a *tool*'s side of the rules: it reads the bus, it never
owns keys. The static half supplies the checks: :mod:`zk2py.keys` parses
token keys, :mod:`zk2py.descriptor` checks a descriptor, :mod:`zk2py.bundle`
verifies a bundle.

**Reading presence (§8.1).** "A caller or tool's liveliness GET on a session
that holds a liveliness subscriber MUST use a callback or an unbounded
handler." zenoh-python has no unbounded channel: its default handler and
``FifoChannel`` are bounded FIFOs, and ``RingChannel`` drops. So every
liveliness GET here passes a ``zenoh.handlers.Callback`` (stable API, the
default ``indirect`` mode) whose callback only appends to a Python list,
and whose drop function marks completion.

Measured on 1.10.1 with 2,000 tokens, that is necessary but not sufficient
(SPEC-FINDINGS F-47). A session whose *liveliness subscriber* uses a bounded
handler that nobody drains starves its own GET: a callback GET then returns
257 of 2,002 tokens, and ends only at its timeout. So:
- :func:`list_presence` reports ``complete=False`` when the GET ended at its
  timeout rather than at the routers' final reply;
- any liveliness subscriber zk2py declares uses a ``Callback`` too;
- presence is polled with GETs, never inferred from silence (§3.2 R7, O5).
"""

from __future__ import annotations

import json
import queue
import threading
import time
from dataclasses import dataclass, field
from typing import Any

import zenoh

from . import bundle, keys

#: Liveliness GET timeout. The spec sets none (SPEC-FINDINGS F-49).
PRESENCE_TIMEOUT_S = 5.0
#: GET timeout for a descriptor or a bundle, per attempt. None set by the spec.
GET_TIMEOUT_S = 5.0
_DONE = object()


def open_client(endpoint: str) -> zenoh.Session:
    """A *client* session to one router endpoint, multicast scouting off: a
    tool that sees exactly the router it is pointed at."""
    conf = zenoh.Config()
    conf.insert_json5("mode", json.dumps("client"))
    conf.insert_json5("connect/endpoints", json.dumps([endpoint]))
    conf.insert_json5("scouting/multicast/enabled", "false")
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
    #: the GET ended at the routers' final reply, not at its timeout
    complete: bool = False
    elapsed_s: float = 0.0

    @property
    def count(self) -> int:
        return len(self.instances) + len(self.alive) + len(self.members) + len(self.other)


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
    # A GET whose drop fired only at the timeout may be partial (F-47).
    p.complete = finished and p.elapsed_s < timeout * 0.95
    for r in replies:
        if r.ok is None:
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
    return p


# -- GET plumbing ----------------------------------------------------------------

@dataclass
class Answer:
    """One GET reply, as zk2py sees it."""

    ok: bool
    key: str | None
    encoding: str
    payload: bytes


def _answers(session: zenoh.Session, selector: str, target: zenoh.QueryTarget,
             timeout: float):
    """Yield each reply *as it arrives* (an unbounded queue fed by a
    callback), until the GET completes.

    Consolidation is ``None``: with zenoh's default (``Auto``, which is
    ``Latest`` on a concrete key) replies are held until the query completes
    (§4.1), which §8.4 step 2 forbids ("Verify each reply as it arrives …
    without waiting for the GET to complete"). SPEC-FINDINGS F-50.
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
            yield Answer(True, str(s.key_expr), str(s.encoding), s.payload.to_bytes())
        else:
            e = item.err
            yield Answer(False, None, str(e.encoding), e.payload.to_bytes())


# -- §3.3 the descriptor ---------------------------------------------------------

def get_descriptor(session: zenoh.Session, instance_key: str,
                   timeout: float = GET_TIMEOUT_S) -> list[Answer]:
    """GET an instance's descriptor (§3.3: "a JSON document answered on GET at
    its instance key"). Every reply is returned; the caller judges them.

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

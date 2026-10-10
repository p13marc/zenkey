"""``health.v1``: how well a service serves (profile text 0.1, draft,
written against core 0.23; ``spec/profiles/health/v1.md``).

The first standard-contract profile (core §10 point 1): its contract is
``spec/profiles/health/health.v1.toml``, three resources, ``state/status``
(``freshness.ttl_s = 60``), ``state/checks/{check}`` and ``stream/faults``.

The session-free half, the reader's:
- the levels (§2.1, §2.6): :func:`level_of`, with unknown levels kept apart;
- :func:`judge`, §2.11's procedure over one :class:`Reading`, as
  ``conformance/judgements.json`` writes it, the status's freshness judged
  by :mod:`zk2py.freshness` at the contract's horizon;
- :func:`rollup` (§2.2, §5) and :func:`code_class` (§2.10);
- :func:`clock_ahead` (§5's fourth question);
- the payloads, ``health.v1.Status``, ``Check`` and ``Fault``, encoded and
  decoded as proto3 (``proto/health/v1/health.proto``).

The owner half, :class:`HealthOwner`, over :class:`zk2py.owner.Owner`:
- the status before step 4 (§2.3), re-put by freshness (§2.4), never
  deleted while the service runs;
- checks put on change and deleted when retired, with §2.2's rule and the
  order of the puts;
- faults as occurrences, with codes of §2.10's form;
- ``clock_ahead`` (§2.5): published when the guard first holds, again every
  30 s while it holds, and not after it releases.
"""

from __future__ import annotations

import re
import threading
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Any

from . import freshness as fr

REPO = Path(__file__).resolve().parents[3]
CONTRACT = REPO / "spec" / "profiles" / "health" / "health.v1.toml"
IFACE = "health.v1"
#: §3: Level.
UNSPECIFIED, OK, DEGRADED, FAILED = 0, 1, 2, 3
NAMES = {OK: "ok", DEGRADED: "degraded", FAILED: "failed"}
RANK = {"ok": 1, "degraded": 2, "failed": 3}
#: §2.4: the status's horizon, the contract's.
HORIZON = fr.horizon("state", {"freshness.ttl_s": 60})
#: §2.5: "once per status interval, 30 s".
FAULT_PERIOD_S = 30.0
#: §2.10.
CODE_FORM = re.compile(r"[a-z][a-z0-9_]*")
PROFILE_CODES = {"clock_ahead": FAILED}

# Verdicts (§5).
HEALTHY, UNHEALTHY, STALE, UNOBSERVABLE, NOT_ASKED = "healthy", "unhealthy", "stale", "unobservable", "not_asked"


# -- the levels (§2.1, §2.6) ---------------------------------------------------------

@dataclass(frozen=True)
class Unknown:
    """A level that is not one (§2.6): ``unknown_level`` for
    ``LEVEL_UNSPECIFIED`` or a number the enum does not list, ``undecodable``
    for a payload that does not decode."""

    reason: str
    value: Any = None


Level = str | Unknown  # "ok", "degraded", "failed", or Unknown


def level_of(value: Any) -> Level | None:
    """A level as judgements.json writes it (or a decoded enum number):
    ``"ok"``, ``"degraded"``, ``"failed"``; ``"unspecified"`` or a number
    the enum does not list is unknown; ``"undecodable"`` is undecodable;
    None is no value read."""
    if value is None:
        return None
    if isinstance(value, bool):
        return Unknown("unknown_level", value)
    if isinstance(value, int):
        return NAMES.get(value, Unknown("unknown_level", value))
    if value in RANK:
        return value
    if value == "undecodable":
        return Unknown("undecodable")
    return Unknown("unknown_level", value)


def worse(a: str, b: str) -> bool:
    return RANK[a] > RANK[b]


# -- §2.11 the reader's procedure ------------------------------------------------------

@dataclass(frozen=True)
class Reading:
    """One reading of one service (§0), as judgements.json gives it:
    - ``presence``: present, absent, incomplete, or across_face;
    - ``descriptor``: None (not read), ``{"lists": False}``, or
      ``{"lists": True, "token": bool}``; read only when present;
    - ``face``: ``{"status_crosses": bool}``, read only across a face;
    - ``observations`` of ``state/status`` (freshness.v1's);
    - ``level``: the latest status value's level, None for no value;
    - ``checks``: None when not read, else ``[(name, level)]``."""

    presence: str
    descriptor: dict[str, Any] | None = None
    face: dict[str, Any] | None = None
    observations: list[Any] | None = None
    level: Level | None = None
    checks: list[tuple[str, Level]] | None = None


@dataclass(frozen=True)
class Answer:
    verdict: str
    reason: str
    level: str | None = None
    detail: str = ""

    def expect(self) -> dict[str, Any]:
        return {"verdict": self.verdict, "reason": self.reason, "level": self.level}


def judge(r: Reading) -> Answer:
    """§2.11: "the first of these steps that answers"."""
    across = r.presence == "across_face"
    # 1. Presence.
    if r.presence == "absent":
        return Answer(NOT_ASKED, "absent", None, "a complete read without its instance token (§2.1)")
    if r.presence == "incomplete":
        return Answer(UNOBSERVABLE, "presence_incomplete", None, "its token not seen in a read possibly incomplete")
    if across:
        if not (r.face or {}).get("status_crosses"):
            return Answer(UNOBSERVABLE, "face_closed", None, "the face does not let state/status cross (§2.8)")
    elif r.presence == "present":
        # 2. The descriptor (§2.7); the interface token plays no part.
        if r.descriptor is None:
            return Answer(UNOBSERVABLE, "no_descriptor", None, "the descriptor was not read")
        if not r.descriptor.get("lists"):
            return Answer(NOT_ASKED, "not_listed", None, "its descriptor does not list health.v1")
    else:
        raise ValueError(f"presence {r.presence!r}")
    # 3. The status's freshness, freshness.v1 §2.7 at the contract's horizon.
    v = fr.judge("state", HORIZON, list(r.observations or []))
    if v.verdict == fr.STALE:
        if across and v.reason == "no_delivery":
            return Answer(UNOBSERVABLE, "nothing_crossed", None, "nothing of the status has crossed (§2.8)")
        return Answer(STALE, v.reason, None, v.detail)
    if v.verdict == fr.UNOBSERVABLE:
        return Answer(UNOBSERVABLE, v.reason, None, v.detail)
    if v.verdict == fr.NOT_ASKED:
        if v.reason == "last_known":
            return Answer(NOT_ASKED, "last_known", None, "only an archive's status was read (core S6)")
        if v.reason == "deleted":
            return Answer(UNOBSERVABLE, "deleted", None, "the owner deleted its status, which §2.3 forbids")
        return Answer(NOT_ASKED, v.reason, None, v.detail)
    # 4. The status's level, then 5. the checks read with it (§2.4).
    current = [lv for _, lv in (r.checks or []) if isinstance(lv, str)]
    worst = max(current, key=RANK.__getitem__) if current else None
    level = r.level if r.level is not None else Unknown("unknown_level")
    if isinstance(level, str):
        if worst is not None and worse(worst, level):
            return Answer(UNHEALTHY, "inconsistent", worst, f"a current check at {worst} under a status at {level}")
        return Answer(HEALTHY, "ok", "ok") if level == "ok" else Answer(UNHEALTHY, level, level)
    if worst is not None and worst != "ok":
        return Answer(UNHEALTHY, "check", worst, "a status at an unknown level, and a current check bad")
    return Answer(UNOBSERVABLE, level.reason, None, "a status at an unknown level, and no check bad")


def rollup(answers: list[Answer]) -> dict[str, Any]:
    """§2.2, §5: "the worst level among the verdicts it established",
    counting stale, unobservable and not-asked services apart."""
    counts = {k: 0 for k in (HEALTHY, NOT_ASKED, STALE, UNHEALTHY, UNOBSERVABLE)}
    worst = None
    for a in answers:
        counts[a.verdict] += 1
        if a.verdict in (HEALTHY, UNHEALTHY) and a.level is not None:
            if worst is None or worse(a.level, worst):
                worst = a.level
    return {"counts": counts, "worst": worst}


def code_class(code: str) -> str:
    """§2.10: ``profile``, ``application`` or ``malformed``. Exact: no
    trimming and no case folding."""
    if code in PROFILE_CODES:
        return "profile"
    return "application" if CODE_FORM.fullmatch(code) and code.isascii() else "malformed"


def clock_ahead(last_fault_ns: int | None, last_confirmation_ns: int | None) -> str:
    """§5, "Is this service's clock ahead?", on one subscriber's monotonic
    clock: yes, a ``clock_ahead`` fault with no confirmation of the status
    heard since; no, a confirmation after the last fault, or with none
    heard; unobservable, neither heard."""
    if last_fault_ns is not None and (last_confirmation_ns is None or last_confirmation_ns < last_fault_ns):
        return "yes"
    if last_confirmation_ns is not None:
        return "no"
    return UNOBSERVABLE


# -- the payloads (§3, proto3) --------------------------------------------------------

def _varint(v: int) -> bytes:
    if v < 0:
        v += 1 << 64
    out = bytearray()
    while True:
        b = v & 0x7F
        v >>= 7
        out.append(b | (0x80 if v else 0))
        if not v:
            return bytes(out)


def _fields(spec: list[tuple[int, str, Any]]) -> bytes:
    """proto3: a field at its default is not written."""
    out = bytearray()
    for number, kind, value in spec:
        if kind == "varint" and value:
            out += _varint(number << 3) + _varint(int(value))
        elif kind == "string" and value:
            data = value.encode("utf-8")
            out += _varint((number << 3) | 2) + _varint(len(data)) + data
    return bytes(out)


def status(level: int, reason: str = "", since_ns: int = 0) -> bytes:
    return _fields([(1, "varint", level), (2, "string", reason), (3, "varint", since_ns)])


def check(level: int, detail: str = "", checked_at_ns: int = 0) -> bytes:
    return _fields([(1, "varint", level), (2, "string", detail), (3, "varint", checked_at_ns)])


def fault(code: str, level: int, detail: str = "", at_ns: int = 0) -> bytes:
    return _fields([(1, "string", code), (2, "varint", level), (3, "string", detail), (4, "varint", at_ns)])


def decode(data: bytes, layout: dict[int, str]) -> dict[int, Any] | None:
    """A proto3 message whose fields are ``layout``'s (number -> "enum",
    "uint64" or "string"); None when it does not decode. Unknown fields are
    skipped, and the last occurrence of a field wins."""
    from .envelope import EnvelopeError, _skip
    from .envelope import _varint as read_varint

    out: dict[int, Any] = {}
    pos = 0
    try:
        while pos < len(data):
            key, pos = read_varint(data, pos)
            number, wire = key >> 3, key & 7
            if number == 0:
                return None
            kind = layout.get(number)
            if kind in ("enum", "uint64"):
                if wire != 0:
                    return None
                v, pos = read_varint(data, pos)
                if kind == "enum":
                    v &= (1 << 64) - 1
                    v = v - (1 << 64) if v >= 1 << 63 else v
                    v = ((v + (1 << 31)) % (1 << 32)) - (1 << 31)  # int32
                out[number] = v
            elif kind == "string":
                if wire != 2:
                    return None
                n, pos = read_varint(data, pos)
                if pos + n > len(data):
                    return None
                out[number] = data[pos:pos + n].decode("utf-8")
                pos += n
            else:
                pos = _skip(data, pos, wire, number)
    except (EnvelopeError, UnicodeDecodeError):
        return None
    return out


STATUS_LAYOUT = {1: "enum", 2: "string", 3: "uint64"}
CHECK_LAYOUT = {1: "enum", 2: "string", 3: "uint64"}
FAULT_LAYOUT = {1: "string", 2: "enum", 3: "string", 4: "uint64"}


def status_level(payload: bytes) -> Level:
    """A status payload's level (§2.6): unknown when UNSPECIFIED or not
    listed, undecodable when it does not decode as ``health.v1.Status``."""
    m = decode(payload, STATUS_LAYOUT)
    return Unknown("undecodable") if m is None else level_of(m.get(1, UNSPECIFIED))  # type: ignore[return-value]


def check_level(payload: bytes) -> Level:
    m = decode(payload, CHECK_LAYOUT)
    return Unknown("undecodable") if m is None else level_of(m.get(1, UNSPECIFIED))  # type: ignore[return-value]


def decode_fault(payload: bytes) -> dict[str, Any] | None:
    m = decode(payload, FAULT_LAYOUT)
    if m is None:
        return None
    return {"code": m.get(1, ""), "level": m.get(2, UNSPECIFIED), "detail": m.get(3, ""), "at_ns": m.get(4, 0)}


def decode_status(payload: bytes) -> dict[str, Any] | None:
    m = decode(payload, STATUS_LAYOUT)
    return None if m is None else {"level": m.get(1, UNSPECIFIED), "reason": m.get(2, ""), "since_ns": m.get(3, 0)}


# -- the owner half ---------------------------------------------------------------------

class HealthRuleError(ValueError):
    """A put that would break §2.2 or §2.10, refused before it is made."""


class HealthOwner:
    """A service's ``health.v1`` (§2.3), over :class:`zk2py.owner.Owner`:
    ``owner`` is the service, built with the health contract and any
    others, and the status it holds at start is put before step 4."""

    def __init__(self, system: str, service: str, *, contracts: list[Any] | None = None,
                 level: int = UNSPECIFIED, reason: str = "starting", fault_period_s: float = FAULT_PERIOD_S,
                 clock_ns: Any = None, **owner_kw: Any):
        from .contract import load_contract
        from .owner import Owner

        self.contract = load_contract(CONTRACT)
        if not self.contract.valid:
            raise ValueError(f"{CONTRACT}: {self.contract.codes}")
        self.level, self.reason = level, reason
        self.clock_ns = clock_ns or time.time_ns
        self.since_ns = self.clock_ns()
        self.fault_period_s = fault_period_s
        self.checks: dict[str, int] = {}
        self.faults: list[dict[str, Any]] = []
        self.owner = Owner(system, service, [self.contract, *(contracts or [])],
                           initial={f"{IFACE}/state/status": status(level, reason, self.since_ns)}, **owner_kw)
        self.owner.on_guard = self._guard
        self._repeat: threading.Thread | None = None
        self._stop = threading.Event()
        self._lock = threading.RLock()

    # keys
    def key(self, rel: str) -> str:
        return f"zk2/{self.owner.system}/{self.owner.service}/{IFACE}/{rel}"

    @property
    def status_key(self) -> str:
        return self.key("state/status")

    def check_key(self, name: str) -> str:
        from .slug import slug

        return self.key(f"state/checks/{slug(name)}")

    def start(self) -> None:
        self.owner.start()

    def close(self) -> None:
        """The status is left: "When it closes, it leaves the status" (§2.3)."""
        self._stop.set()
        self.owner.close()

    # §2.2 and §2.3
    def _worst_check(self, without: str | None = None) -> int:
        levels = [lv for n, lv in self.checks.items() if n != without and lv in NAMES]
        return max(levels, key=lambda lv: RANK[NAMES[lv]]) if levels else OK

    def set_status(self, level: int, reason: str = "") -> None:
        """Put the status. ``since_ns`` moves only when the level changes. A
        level better than the worst current check is refused (§2.2)."""
        with self._lock:
            if level in NAMES and self.checks:
                w = self._worst_check()
                if worse(NAMES[w], NAMES[level]):
                    raise HealthRuleError(f"status {NAMES[level]} would be better than a check at {NAMES[w]} (§2.2)")
            if level != self.level:
                self.since_ns = self.clock_ns()
            self.level, self.reason = level, reason
            self.owner.set_state(self.status_key, status(level, reason, self.since_ns))

    def set_check(self, name: str, level: int, detail: str = "") -> None:
        """Put a check on change. "A check that gets worse than the status:
        the status first, then the check" (§2.2): the status is put at the
        check's level, its reason naming the check, before the check."""
        with self._lock:
            if level in NAMES and self.level in NAMES and worse(NAMES[level], NAMES[self.level]):
                self.set_status(level, f"check {name}: {detail or NAMES[level]}")
            self.checks[name] = level
            self.owner.set_state(self.check_key(name), check(level, detail, self.clock_ns()))

    def retire_check(self, name: str) -> None:
        """Delete a retired check (core S3). The status, if it may improve,
        is the application's to put after (§2.2)."""
        with self._lock:
            self.checks.pop(name, None)
            self.owner.delete_state(self.check_key(name))

    def publish_fault(self, code: str, level: int, detail: str = "") -> None:
        """A fault: an occurrence (§2.3), its code of §2.10's form."""
        if code_class(code) == "malformed":
            raise HealthRuleError(f"fault code {code!r} is not [a-z][a-z0-9_]* (§2.10)")
        at = self.clock_ns()
        # §2.5 (0.2): "An owner that sets the stamp itself MAY, from the
        # clock it mints its state stamps with". zk2py does when that clock
        # is set apart from its session's (a test's offset clock), so the
        # offset stamps its faults as it stamps its state.
        stamp = self.owner.clock() if self.owner.clock is not None else None
        self.owner.publish(self.key("stream/faults"), fault(code, level, detail, at), timestamp=stamp)
        self.faults.append({"code": code, "level": level, "detail": detail, "at_ns": at})

    # §2.5
    def _guard(self, held: bool, offset_ns: int) -> None:
        if not held:
            self._stop.set()
            return
        self._stop = threading.Event()
        self.publish_fault("clock_ahead", FAILED, f"the clock is {offset_ns / 1e9:.3f} s ahead of its router")
        stop = self._stop

        def repeat() -> None:
            # "While the guard holds, the owner SHOULD publish it again once
            # per status interval, 30 s".
            while not stop.wait(self.fault_period_s):
                if not self.owner.ahead:
                    return
                self.publish_fault("clock_ahead", FAILED,
                                   f"the clock is still ahead of its router (last measured {offset_ns / 1e9:.3f} s)")

        self._repeat = threading.Thread(target=repeat, daemon=True)
        self._repeat.start()


# -- the reader half on a bus (§2.4, §2.7, §2.11) -----------------------------------

def status_agrees(r: Reading) -> str:
    """§5, "Does its status agree with its checks?": ``yes`` a fresh status
    at a level and no current check worse; ``no`` a current check worse
    than a fresh status at a level (a finding, from two readings a grace
    apart, §2.2); otherwise ``unobservable``, or ``not_asked`` as the first
    question's."""
    a = judge(r)
    if a.verdict == NOT_ASKED:
        return NOT_ASKED
    if a.reason == "inconsistent":
        return "no"
    v = fr.judge("state", HORIZON, list(r.observations or []))
    if v.verdict != fr.FRESH or not isinstance(r.level, str) or r.checks is None:
        return UNOBSERVABLE
    return "yes"


def _latest(*values: tuple[int | None, bytes] | None) -> bytes | None:
    """The payload with the latest stamp among (stamp_ns, payload) pairs."""
    got = [v for v in values if v is not None]
    if not got:
        return None
    return max(got, key=lambda v: -1 if v[0] is None else v[0])[1]


def read_near(tool: Any, address: str, *, subscriber: fr.Subscriber | None = None,
              trust: fr.ClockTrust | None = None, get: bool = True,
              timeout: float = 1.0) -> tuple[Reading, dict[str, Any]]:
    """One reading of ``address`` (``<system>/<service>``) on the near side,
    for §2.11: its presence (core §8.1), its descriptor (§2.7, the interface
    token playing no part), the status as ``subscriber`` saw it (a present,
    listed owner holds one, §2.3, so its silence ages it) and as a GET over
    ``health.v1/state/**`` answered it (core S2, aged against ``trust``),
    and the checks that GET answered (§2.4). Returns the reading and what
    was read."""
    import json

    from . import live
    from .slug import unslug

    info: dict[str, Any] = {}
    pres = live.list_presence(tool, f"zk2/{address}/@zk/**")
    info["presence"] = pres
    if pres.instances:
        presence = "present"
    elif pres.complete:
        presence = "absent"
    else:
        presence = "incomplete"
    descriptor = None
    if presence == "present":
        inst = pres.instances[0]["instance"]
        d = live.get_descriptor(tool, f"zk2/{address}/@zk/instance/{inst}", timeout=timeout)
        if len(d) == 1 and d[0].ok:
            doc = json.loads(d[0].payload)
            info["descriptor"] = doc
            entry = next((e for e in doc.get("interfaces") or [] if e.get("iface") == IFACE), None)
            descriptor = {"lists": False} if entry is None else {"lists": True, "token": entry.get("token", True)}
    status_key = f"zk2/{address}/{IFACE}/state/status"
    observations: list[Any] = []
    sub_value = None
    if subscriber is not None:
        observations.append(subscriber.observation(status_key))
        last = [x for x in subscriber.of(status_key) if x.kind == "put"]
        sub_value = (last[-1].stamp_ns, last[-1].payload) if last else None
    checks = None
    get_value = None
    if get:
        st = live.get_state(tool, f"zk2/{address}/{IFACE}/state/**", timeout=timeout)
        info["replies"] = st.replies
        mine = [x for x in st.replies if x.key == status_key]
        if mine:
            x = mine[-1]
            got = (fr.GetReading("delete", x.stamp_ns, x.stamp_id, x.stamp) if x.deleted
                   else fr.GetReading("put", x.stamp_ns, x.stamp_id, x.stamp, x.payload))
            get_value = None if x.deleted else (x.stamp_ns, x.payload)
        else:
            got = fr.GetReading(None)
        info["status_reading"] = got
        observations.append(got.observation(trust or fr.ClockTrust()))
        prefix = f"zk2/{address}/{IFACE}/state/checks/"
        checks = [(unslug(x.key[len(prefix):]) or x.key[len(prefix):], check_level(x.payload))
                  for x in st.replies if x.key.startswith(prefix) and not x.deleted]
    payload = _latest(sub_value, get_value)
    level = None if payload is None else status_level(payload)
    return Reading(presence, descriptor, None, observations, level, checks), info


def read_across(subscriber: fr.Subscriber, address: str, status_crosses: bool) -> Reading:
    """A reading across a constrained face (§2.8): presence and descriptors
    do not cross, and the deployment's word says whether the face lets
    ``state/status`` cross. Only the subscriber's observation is had."""
    status_key = f"zk2/{address}/{IFACE}/state/status"
    last = [x for x in subscriber.of(status_key) if x.kind == "put"]
    return Reading("across_face", None, {"status_crosses": status_crosses},
                   [subscriber.observation(status_key)], status_level(last[-1].payload) if last else None, None)

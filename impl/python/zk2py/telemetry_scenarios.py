"""The ``telemetry.v1`` scenarios (``spec/profiles/telemetry/scenarios.md``,
text 0.2) on in-process zenoh-python routers, judged by zk2py's reader:
``python -m zk2py.telemetry_scenarios [--only 1 …] [--owner <exe>]``.

Each section runs against one owner, through an adapter:
- **zk2py's owner** (the default): :class:`PyMeter`;
- **the Rust owner example** (``--owner <exe>``, or ``ZK2PY_OWNER``), a
  black box driven by its ``--telemetry`` stdin commands, read from its
  ``--help`` alone (:class:`RustMeter`): ``telemetry put|raw <resource>
  <json> [<attachment json>]``, ``telemetry new-epoch``, ``telemetry
  instance``. It is given no contract file: it implements ``meter.v1`` as it
  embeds it, and §1 checks that its descriptor's fingerprint is the one zk2py
  computes for the contract transcribed from the text.

Either way:
- **The bus** is the text's: R1, a router with timestamping on; O, R and R0
  its clients, each with a session of its own.
- **The contract** is ``meter.v1``, transcribed from the text into
  ``impl/python/interop/telemetry/``, listing the profile's file by its path
  from there.
- **O** is ``lab/meter``. The driver is its application: each Point's
  ``timestamp`` is the host's clock when the step takes the value, and both
  owners' checked writers fill the epoch where a put leaves it out, as the
  example's help says of its own ("a counter's or histogram's Point gains
  "epoch", a bare one whose attachment is telemetry.v1.Epoch gets it as its
  attachment"). A *raw* step is first offered to the checked writer, whose
  refusal is recorded, then sent from the owner's session unchecked (zk2py's
  ``raw``, the example's ``telemetry raw``), carrying the current epoch. A
  process's end is the owner's session closing (the example's stdin closed,
  a clean close); a new epoch in one process is make-before-break.
- **R** subscribes to the data keys and to the owner's instance tokens, with
  history, both before the owner starts, and records each arrival, its
  stamp, and each token's appearance and withdrawal on one monotonic clock.
  **R0** reads no presence (§2.14). "R reads" is
  :func:`zk2py.telemetry.conform`; "R pairs" is
  :func:`zk2py.telemetry.continuity` over each two consecutive conforming
  samples of a stream member in arrival order, and, for state, in the order
  a late joiner's merge keeps them (§2.10, 0.2).
- **§8** runs twice: R0 alone, and R0F, a client of a second router R2
  whose access control denies ``@zk`` on its ``unixsock-stream`` face
  (``link.v1`` §2.6), with a control off the face.

Exit 0 when every check passes, 1 when any fails, 2 when a section did not
run, unless ``--only`` asked for a subset.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
import tempfile
import threading
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Any

from . import seed
from . import telemetry as tm
from .alarms_scenarios import Report, _wait
from .freshness import stamp_ns_of

HERE = Path(__file__).resolve().parent
CONTRACT = HERE.parent / "interop" / "telemetry" / "meter.v1.toml"
ADDRESS = "lab/meter"
BASE = f"zk2/{ADDRESS}/meter.v1"
PREFIX = f"{BASE}/stream"
TOTAL = f"{BASE}/state/total"
#: "a section waits 500 ms after a token's withdrawal or appearance before
#: the next put"; "Samples of one member are published at least 200 ms
#: apart" (scenarios.md, "Spacing").
SETTLE_S = 0.5
APART_S = 1.0


def load() -> Any:
    from .contract import load_contract

    c = load_contract(CONTRACT, check_file_name=True)
    if not c.valid:
        raise RuntimeError(f"{CONTRACT}: {c.codes}")
    return c


class Refused(RuntimeError):
    """The checked writer refused a put, and sent nothing: ``reasons`` are
    §2.6's tokens, or ``decreased`` (§2.7, §2.8), ``undecodable``, or
    ``epoch_given`` (the example's own: an in-band epoch not the owner's)."""

    def __init__(self, reasons: list[str], answer: str = ""):
        super().__init__(answer or ", ".join(reasons))
        self.reasons = reasons
        #: the writer's answer as it came, for a check's failure detail
        self.answer = answer or f"zk2py's writer: {', '.join(reasons)}"


def now_ms() -> int:
    return time.time_ns() // 1_000_000


def point(value: Any, kind: str, *, unit: str | None = None, epoch: str | None = None) -> dict[str, Any]:
    """A Point timed by the host's clock now; its epoch only when given."""
    p: dict[str, Any] = {"timestamp": now_ms(), "value": {"type": kind, "value": value}}
    if unit is not None:
        p["unit"] = unit
    if epoch is not None:
        p["epoch"] = epoch
    return p


# -- the owners --------------------------------------------------------------------------

class PyMeter:
    """``lab/meter`` over zk2py's owner, a client of R1, with a checked
    writer: §2.6's refusals against the declaration, and a counter's or a
    histogram's decrease within this epoch (§2.7: "MUST NOT publish a
    counter value below one it published before for that member"; §2.8)."""

    rust = False

    def __init__(self, endpoint: str, contract: Any, extra: list[Any] | None = None) -> None:
        from .owner import Owner

        self.contract = contract
        self.decls = tm.contract_declarations(contract)
        self.rids = {k.split("/", 1)[1]: k for k in self.decls}
        system, service = ADDRESS.split("/")
        #: other contracts the same service implements (the probes' meterx.v1)
        self.owner = Owner(system, service, [contract, *(extra or [])], connect=endpoint)
        #: what the writer put in this epoch, by member
        self.published: dict[str, list[Any]] = {}
        self.owner.start()

    @property
    def instance(self) -> str:
        return self.owner.instance

    def _key(self, res: str) -> str:
        return f"{BASE}/{self.rids[res]}"

    def _filled(self, res: str, item: Any, attachment: Any) -> tuple[Any, Any]:
        d = self.decls[self.rids[res]]
        if d.carrier == "point" and isinstance(item, dict) and "epoch" not in item \
                and (d.kind or (item.get("value") or {}).get("type")) in ("counter", "histogram"):
            item = {**item, "epoch": self.instance}
        if d.epoch == "attachment" and attachment is None:
            attachment = self.instance
        return item, attachment

    def put(self, res: str, item: Any, attachment: Any = None) -> None:
        item, attachment = self._filled(res, item, attachment)
        d = self.decls[self.rids[res]]
        c = tm.conform(d, tm.Sample(item, tm.NO_ATTACHMENT if attachment is None else attachment))
        if c.conformance != tm.CONFORMS:
            raise Refused(list(c.reasons))
        if c.kind in ("counter", "histogram"):
            now = tm.read_of(c)
            if any(b.kind == now.kind and tm._decrease(b, now) for b in self.published.get(res, [])):
                raise Refused(["decreased"])
        self._send(res, item, attachment)
        if c.kind in ("counter", "histogram"):
            self.published.setdefault(res, []).append(tm.read_of(c))

    def raw(self, res: str, item: Any, attachment: Any = None) -> None:
        """scenarios.md "Raw puts": "from the owner's session, bypassing any
        check its runtime's writer makes"."""
        self._send(res, item, attachment)

    def raw_bytes(self, res: str, payload: bytes, attachment: bytes | None) -> None:
        self.owner.publish(self._key(res), payload, attachment=attachment)

    def _send(self, res: str, item: Any, attachment: Any) -> None:
        payload = json.dumps(item).encode()
        att = None if attachment is None else json.dumps(attachment).encode()
        if self.rids[res].startswith("state/"):
            self.owner.set_state(self._key(res), payload)
        else:
            self.owner.publish(self._key(res), payload, attachment=att)

    def new_epoch(self) -> str:
        """Core §8.1's re-mint: the writer's record is the old epoch's."""
        self.owner.new_epoch()
        self.published.clear()
        return self.instance

    def close(self) -> None:
        self.owner.close()


class RustMeter:
    """The owner example with ``--telemetry``, a black box: a child process
    driven by the stdin commands its ``--help`` documents. Its answers, as
    the help gives them: ``telemetry put <key> epoch=<id>``, ``telemetry raw
    <key>``, ``telemetry new-epoch <id>`` ("once the old tokens are
    withdrawn"), ``telemetry instance <id>``, and ``refused <line>:
    telemetry.v1 <why>: <words>``, ``<why>`` one of ``does_not
    <problem>[,<problem>...]``, ``undecodable``, ``epoch given=<json>
    current=<id>`` and ``decreased epoch=<id>``."""

    rust = True

    def __init__(self, exe: Path, endpoint: str) -> None:
        from .live_interop import Owner

        self.p = Owner(exe, ADDRESS, [], connect=endpoint, flags=["--telemetry"])
        k = self.p.wait_for("ready ", 120)
        if k is None:
            raise RuntimeError(f"the owner example did not start: {self.p.stderr[-3:]}")
        self.instance = k.rsplit("/", 1)[1]
        self.instance_key = k

    def _cmd(self, line: str) -> str:
        got = self.p.command(line, timeout=10.0)
        if got is None:
            raise RuntimeError(f"{line!r}: no answer")
        return got

    @staticmethod
    def _line(verb: str, res: str, item: Any, attachment: Any) -> str:
        line = f"telemetry {verb} {res} {json.dumps(item)}"
        return line if attachment is None else f"{line} {json.dumps(attachment)}"

    #: The help's ``<why>``: "does_not <problem>[,<problem>...] (v1.md
    #: §2.6's tokens), undecodable, epoch given=<json> current=<id>,
    #: decreased epoch=<id>", then ": <words>".
    WHY = re.compile(r"(?P<why>does_not (?P<problems>[a-z_]+(?:,[a-z_]+)*)|undecodable"
                     r"|epoch given=(?P<given>.+?) current=(?P<current>[0-9a-f]{16})"
                     r"|decreased epoch=(?P<epoch>[0-9a-f]{16})): (?P<words>.+)")

    def put(self, res: str, item: Any, attachment: Any = None) -> None:
        """A checked put. A refusal is read only in the form the help gives,
        ``refused <line>: telemetry.v1 <why>: <words>``, its problems §2.6's
        tokens; any other answer raises with the answer as it came, so a
        change in the example's wording fails its section loudly."""
        line = self._line("put", res, item, attachment)
        got = self._cmd(line)
        if got.startswith("telemetry put ") and re.fullmatch(r"telemetry put \S+ epoch=[0-9a-f]{16}", got):
            return
        prefix = f"refused {line}: telemetry.v1 "
        m = self.WHY.fullmatch(got[len(prefix):]) if got.startswith(prefix) else None
        if m is None:
            raise RuntimeError(f"{line!r}: an answer not in the form the help gives: {got!r}")
        if m["problems"] is not None:
            reasons = m["problems"].split(",")
            unknown = [r for r in reasons if r not in tm.SAMPLE_REASONS]
            if unknown:
                raise RuntimeError(f"{line!r}: problems that are not §2.6's tokens, {unknown}: {got!r}")
        elif m["why"] == "undecodable":
            reasons = ["undecodable"]
        elif m["given"] is not None:
            reasons = ["epoch_given"]
        else:
            reasons = ["decreased"]
        raise Refused(reasons, got)

    def raw(self, res: str, item: Any, attachment: Any = None) -> None:
        line = self._line("raw", res, item, attachment)
        got = self._cmd(line)
        if not got.startswith("telemetry raw "):
            raise RuntimeError(f"{line!r}: {got!r}")

    def new_epoch(self) -> str:
        got = self._cmd("telemetry new-epoch")
        if not got.startswith("telemetry new-epoch "):
            raise RuntimeError(f"telemetry new-epoch: {got!r}")
        self.instance = got.split()[-1]
        return self.instance

    def close(self) -> None:
        self.p.close()


@dataclass
class Offer:
    """What the checked writer said of a raw step's value: its reasons, and
    its answer as it came (None and "accepted" when it took the value)."""

    reasons: list[str] | None
    answer: str


def raw_step(o: Any, res: str, item: Any, attachment: Any = None) -> Offer:
    """A *raw* step: the checked writer is offered the same value first, and
    its refusal recorded; then the value goes out unchecked. When the
    checked writer takes it (and so publishes it), nothing more is sent."""
    try:
        o.put(res, item, attachment)
    except Refused as e:
        o.raw(res, item, attachment)
        return Offer(e.reasons, e.answer)
    return Offer(None, "accepted: the checked writer published it")


def _offers(offers: Any) -> str:
    return "; ".join(f"{x.reasons} from {x.answer!r}" for x in offers)


# -- the readers ---------------------------------------------------------------------------

def fold_seed(tokens: list[tuple[int, str, str]], read: list[str], t: int) -> list[tuple[int, str, str]]:
    """§2.9 (0.3): a complete read's instances folded into a subscription's
    token moves as of the read's end ``t``. The subscription's word wins:
    an instance it saw appear or withdrawn keeps that word, so a reply sent
    before a withdrawal never brings the token back; one it never mentioned
    is held from ``t``."""
    heard = {inst for _, _, inst in tokens}
    return list(tokens) + [(t, "put", inst) for inst in read if inst not in heard]


@dataclass
class Arrival:
    arrival_ns: int
    key: str
    payload: bytes
    attachment: bytes | None
    stamp: Any            # a zk2py.seed.Stamp, or None
    stamp_ns: int | None  # as zenoh reads it (§2.11, 0.2)
    via: str = "subscription"


class Reader:
    """R (``presence``) or R0: the data keys' samples with their stamps, and
    for R the owner's instance tokens, on one monotonic clock."""

    def __init__(self, session: Any, contract: Any, *, presence: bool, selector: str | None = None,
                 base: str = BASE) -> None:
        import zenoh

        #: the interface's key prefix, ``zk2/<address>/<iface>``
        self.base = base
        selector = selector if selector is not None else f"{base}/stream/*"
        self.session = session
        self.decls = tm.contract_declarations(contract)
        self.encodings = {f"{r['token']}/{r['template']}": r.get("encoding") for r in contract.canonical["resources"]}
        self.types = {f"{r['token']}/{r['template']}": r.get("type") for r in contract.canonical["resources"]}
        self.att_encodings = {f"{r['token']}/{r['template']}": r.get("attachment_encoding")
                              for r in contract.canonical["resources"]}
        self.lock = threading.Lock()
        #: §2.9 (0.3): the presence view "stands on a complete read of the
        #: tokens", never a subscription's history alone; None until one
        self.seeded_ns: int | None = None
        #: why the view stands on nothing, when no complete read came back
        self.unseeded: str | None = None
        self.arrivals: list[Arrival] = []
        self.tokens: list[tuple[int, str, str]] = []  # (t_ns, put|delete, instance)
        self.presence = presence
        self.entities: list[Any] = []
        self.subscribe(selector)
        if presence:
            def token(sample: Any) -> None:
                t = time.monotonic_ns()
                kind = "put" if sample.kind == zenoh.SampleKind.PUT else "delete"
                with self.lock:
                    self.tokens.append((t, kind, str(sample.key_expr).rsplit("/", 1)[1]))

            self.entities.append(session.liveliness().declare_subscriber(
                f"zk2/{ADDRESS}/@zk/instance/*", zenoh.handlers.Callback(token), history=True))
            self.seed_presence()

    #: §2.9 (0.3): a read that comes back possibly incomplete is made again
    SEED_TRIES = 3

    def seed_presence(self) -> bool:
        """§2.9 (0.3): a complete liveliness read after the subscription
        (core §8.1: ended at the routers' final reply, no error reply), made
        up to :data:`SEED_TRIES` times. Its tokens are folded in as of its
        end, and only for an instance the subscription has said nothing of:
        a token the subscription saw appear or withdrawn, before the read
        ended, keeps the subscription's word, so a reply sent before a
        withdrawal never brings the token back. With no complete read, the
        view stands on nothing, and :attr:`unseeded` says why."""
        from . import live

        for attempt in range(1, self.SEED_TRIES + 1):
            got = live.list_presence(self.session, f"zk2/{ADDRESS}/@zk/instance/*", timeout=1.0)
            if got.complete:
                break
        else:
            self.unseeded = (f"no complete liveliness read in {self.SEED_TRIES} tries ({got.reading}): presence is "
                             "unobservable for this reader, so no pair of its has presence evidence")
            return False
        t = time.monotonic_ns()
        with self.lock:
            self.tokens[:] = fold_seed(self.tokens, [x["instance"] for x in got.instances], t)
            self.seeded_ns = t
        return True

    def record(self, key: str, payload: bytes, attachment: Any, ts: Any, via: str) -> None:
        t = time.monotonic_ns()
        att = None if attachment is None else attachment.to_bytes()
        with self.lock:
            self.arrivals.append(Arrival(t, key, payload, att, seed.Stamp.of(ts),
                                         None if ts is None else stamp_ns_of(ts), via))

    def subscribe(self, selector: str) -> None:
        import zenoh

        def got(sample: Any) -> None:
            self.record(str(sample.key_expr), sample.payload.to_bytes(), sample.attachment, sample.timestamp,
                        "subscription")

        self.entities.append(self.session.declare_subscriber(selector, zenoh.handlers.Callback(got)))

    # -- reading

    def _member(self, key: str) -> str:
        """``<token>/<template>`` of a data key (the meter's templates are
        literal)."""
        return key.removeprefix(f"{self.base}/")

    def read(self, a: Arrival) -> tm.Conformance:
        """§2.6 against the resource's declaration: the payload decoded by
        its type and encoding (strict JSON, 0.2), the attachment by JSON
        when the declaration's epoch rides there."""
        rid = self._member(a.key)
        d = self.decls[rid]
        try:
            value = tm.decode(a.payload, self.types[rid], self.encodings[rid] or "json")
        except tm.Undecodable:
            return tm.conform(d, tm.Sample(undecodable=True))
        if d.epoch == "attachment" and a.attachment is not None:
            try:
                # §2.6 check 2 (0.3): "read as this section reads a sample of
                # its encoding: strict JSON, or CBOR, depth included".
                att = tm.decode_attachment(a.attachment, self.att_encodings[rid] or "json")
            except tm.Undecodable:
                # §2.6 check 2 (0.2): present and undecodable, epoch_invalid.
                return tm.conform(d, tm.Sample(value, attachment_undecodable=True))
            return tm.conform(d, tm.Sample(value, att))
        return tm.conform(d, tm.Sample(value))

    def of(self, res: str, since_ns: int = 0, key: str | None = None) -> list[tuple[Arrival, tm.Conformance]]:
        key = key or f"{self.base}/stream/{res}"
        with self.lock:
            got = [a for a in self.arrivals if a.key == key and a.arrival_ns >= since_ns]
        return [(a, self.read(a)) for a in got]

    def held_at(self, t: int) -> set[str]:
        held: set[str] = set()
        with self.lock:
            for ts, kind, inst in self.tokens:
                if ts > t:
                    break
                (held.add if kind == "put" else held.discard)(inst)
        return held

    def presence_of(self, t1: int, t2: int) -> tm.Presence | None:
        """scenarios.md "R pairs": "alone throughout when exactly one instance
        token was held at every instant from before the earlier arrival to
        after the later one's, with no liveliness event lost; earlier and
        later the only token held at each arrival". zenoh-python's callback
        handler loses nothing, and the subscription was declared before the
        owner started."""
        if not self.presence or self.seeded_ns is None or t1 < self.seeded_ns:
            return None
        h1, h2 = self.held_at(t1), self.held_at(t2)
        with self.lock:
            moved = any(t1 <= ts <= t2 for ts, _, _ in self.tokens)
        alone = len(h1) == 1 and not moved
        return tm.Presence(alone, next(iter(h1)) if len(h1) == 1 else None, next(iter(h2)) if len(h2) == 1 else None)

    def _pairs(self, ok: list[tuple[Arrival, tm.Conformance]], unit: str | None,
               bases: list[str] | None = None) -> list[tuple[Any, Any, tm.Pair]]:
        out = []
        for (a1, c1), (a2, c2) in zip(ok, ok[1:]):
            # §2.11's time base: "for state, the owner's stamp"
            r1 = tm.read_of(c1, stamp_ns=a1.stamp_ns if a1.key.startswith(f"{self.base}/state/") else None,
                            arrival_ns=a1.arrival_ns)
            r2 = tm.read_of(c2, stamp_ns=a2.stamp_ns if a2.key.startswith(f"{self.base}/state/") else None,
                            arrival_ns=a2.arrival_ns)
            out.append((_short(r1.value), _short(r2.value),
                        tm.continuity(r1, r2, self.presence_of(a1.arrival_ns, a2.arrival_ns), c2.unit or unit)))
            if bases is not None:
                # §2.11's time base, "the first that both samples have"
                bases.append("timestamp" if r1.timestamp_ms is not None and r2.timestamp_ms is not None
                             else "stamp" if r1.stamp_ns is not None and r2.stamp_ns is not None else "arrival")
        return out

    def pairs(self, res: str, since_ns: int = 0, bases: list[str] | None = None) -> list[tuple[Any, Any, tm.Pair]]:
        """A stream member's pairs: each two consecutive conforming samples,
        in arrival order (§2.10)."""
        ok = [(a, c) for a, c in self.of(res, since_ns) if c.conformance == tm.CONFORMS]
        return self._pairs(ok, self.decls[f"stream/{res}"].unit, bases)

    def state_pairs(self, key: str, rid: str, *, merged: bool = True, since_ns: int = 0,
                    bases: list[str] | None = None) -> list[tuple[Any, Any, tm.Pair]]:
        """A state member's pairs (§2.10, 0.2): over the samples a late
        joiner's merge keeps, by the owner's stamp, whichever path brought
        each (core §4.2), or with ``merged`` False in the order received,
        the reading 0.2 rules out (scenarios.md §9 expected 3). Timed by the
        owner's stamps (§2.11)."""
        got = self.of("", since_ns, key=key)
        if merged:
            flags = seed.kept([(a.stamp, "put") for a, _ in got])
            got = [x for x, k in zip(got, flags) if k]
        ok = [(a, c) for a, c in got if c.conformance == tm.CONFORMS]
        return self._pairs(ok, self.decls[rid].unit, bases)

    def current(self, res: str) -> tuple[Any, int] | None:
        """§2.12: the last conforming sample's value and its age by receipt,
        or None: unknown, never 0."""
        ok = [(a, c) for a, c in self.of(res) if c.conformance == tm.CONFORMS]
        if not ok:
            return None
        a, c = ok[-1]
        return c.value, time.monotonic_ns() - a.arrival_ns

    def stamp_ids(self, res: str, since_ns: int = 0, until_ns: int | None = None) -> set[str]:
        return {a.stamp.id for a, _ in self.of(res, since_ns) if a.stamp is not None
                and (until_ns is None or a.arrival_ns <= until_ns)}

    def close(self) -> None:
        for e in self.entities:
            try:
                e.undeclare()
            except Exception:  # noqa: BLE001 - closing anyway
                pass


def _short(v: Any) -> Any:
    return v["count"] if isinstance(v, dict) and "count" in v else v


class Bus:
    """R1, then R's and R0's sessions and subscriptions, before any owner."""

    def __init__(self, mode: Any, contract: Any, *, face: bool = False, tool: Path | None = None,
                 report: Report | None = None, sec: str = "") -> None:
        from . import live
        from .live_interop import _r1

        self.mode, self.contract = mode, contract
        #: zenctl, when the tool judges beside zk2py's reader
        self.tool_exe = tool
        self.router, self.endpoint, _ = _r1()
        self.r_session = live.open_client(self.endpoint)
        self.r0_session = live.open_client(self.endpoint)
        self.R = Reader(self.r_session, contract, presence=True)
        self.R0 = Reader(self.r0_session, contract, presence=False)
        self.face: Face | None = Face(self.endpoint, contract) if face else None
        self.owners: list[Any] = []
        readers = [("R", self.R)] + ([("R0F", self.face.R0F), ("N2", self.face.N2)] if self.face else [])
        for name, reader in readers:
            if reader.unseeded is not None and report is not None:
                report.note(sec, f"{name}: {reader.unseeded}")
        time.sleep(0.3)

    def tool_start(self, window_s: float, *, across_face: bool = False, endpoint: str | None = None,
                   selector: str | None = None) -> ToolRun | None:
        if self.tool_exe is None:
            return None
        return ToolRun(self.tool_exe, endpoint or self.endpoint, window_s, across_face=across_face,
                       selector=selector)

    def owner(self, extra: list[Any] | None = None) -> Any:
        o: Any = PyMeter(self.endpoint, self.contract, extra) if self.mode == "zk2py" \
            else RustMeter(self.mode, self.endpoint)
        self.owners.append(o)
        return o

    def appeared(self, instance: str, timeout: float = 5.0) -> bool:
        ok = _wait(lambda: instance in self.R.held_at(time.monotonic_ns()), timeout)
        time.sleep(SETTLE_S)
        return ok

    def gone(self, instance: str, timeout: float = 5.0) -> bool:
        ok = _wait(lambda: instance not in self.R.held_at(time.monotonic_ns()), timeout)
        time.sleep(SETTLE_S)
        return ok

    def close(self) -> None:
        for o in self.owners:
            try:
                o.close()
            except Exception:  # noqa: BLE001
                pass
        for r in (self.R, self.R0):
            r.close()
        if self.face is not None:
            self.face.close()
        for x in (self.r_session, self.r0_session, self.router):
            try:
                x.close()
            except Exception:  # noqa: BLE001
                pass


class Face:
    """§8's face (core §8.5, ``link.v1`` §2.6): R2, a router linked to R1,
    whose access control denies the ``@zk`` subtree to every session on its
    ``unixsock-stream`` link, in every message and both flows; R0F, a client
    of R2 over that link, and a liveliness watch of its own to show that
    nothing of presence crosses. The control is N2, a client of R2 over TCP,
    off the face: it sees the tokens R0F does not."""

    MESSAGES = ["put", "delete", "declare_subscriber", "query", "reply", "declare_queryable",
                "liveliness_token", "declare_liveliness_subscriber", "liveliness_query"]

    def __init__(self, r1_endpoint: str, contract: Any) -> None:
        import zenoh

        from . import live
        from .owner import free_loopback_port

        self.dir = tempfile.mkdtemp(prefix="zk2py-telemetry-face-")
        sock = os.path.join(self.dir, "face.sock")
        #: the face's endpoint, for a reader that stands across it
        self.endpoint = f"unixsock-stream/{sock}"
        near = f"tcp/127.0.0.1:{free_loopback_port()}"
        conf = zenoh.Config()
        conf.insert_json5("mode", json.dumps("router"))
        conf.insert_json5("listen/endpoints", json.dumps([f"unixsock-stream/{sock}", near]))
        conf.insert_json5("connect/endpoints", json.dumps([r1_endpoint]))
        conf.insert_json5("scouting/multicast/enabled", "false")
        conf.insert_json5("timestamping/enabled", "true")
        conf.insert_json5("access_control", json.dumps({
            "enabled": True, "default_permission": "allow",
            "rules": [{"id": "face-zk", "messages": self.MESSAGES, "flows": ["egress", "ingress"],
                       "permission": "deny", "key_exprs": ["zk2/*/*/@zk/**"]}],
            "subjects": [{"id": "far", "link_protocols": ["unixsock-stream"]}],
            "policies": [{"rules": ["face-zk"], "subjects": ["far"]}],
        }))
        self.router = zenoh.open(conf)
        c = zenoh.Config()
        c.insert_json5("mode", json.dumps("client"))
        c.insert_json5("connect/endpoints", json.dumps([f"unixsock-stream/{sock}"]))
        c.insert_json5("scouting/multicast/enabled", "false")
        self.session = zenoh.open(c)
        self.R0F = Reader(self.session, contract, presence=True)
        self.near_session = live.open_client(near)
        self.N2 = Reader(self.near_session, contract, presence=True)

    def close(self) -> None:
        self.R0F.close()
        self.N2.close()
        for x in (self.session, self.near_session, self.router):
            try:
                x.close()
            except Exception:  # noqa: BLE001
                pass


# -- the tool: zenctl telemetry, a second reader ------------------------------------------

#: The tool lists a member's latest 16 pairs, beside the count of all
#: (``continuity.pairs``) and, since F-155's answer, ``pairs_listed``.
#: Since F-154's answer the tool's "telemetry: listening …" line comes once
#: its subscriptions are declared and presence is seeded, and ``--for``
#: counts from then: the steps start on the line, with no margin (round 47
#: needed 1 s with zenctl-tc).
#: Across a face too, since F-159's answer (zenctl-tg): the line comes
#: after S4's GET of a state metric resource in scope was answered, or,
#: with none, after ``--settle``; :func:`tool_opening` asserts all three
#: openings. Round 49 waited 0.2 s across a face with zenctl-tf.


class ToolRun:
    """``zenctl telemetry`` over one section's window, a black box read from
    its ``--help``: ``--for <SECS>``, ``-c <endpoint>``, ``--across-face``
    with ``--contracts``, ``--format json``; exit 0, 1 (a finding) or 2 (no
    verdict). Its document: ``rows`` of ``resource`` (its declaration) and
    ``member`` (kind, unit, ``samples`` counts and ``problems``, ``pairs``),
    ``totals`` and ``judgement``."""

    def __init__(self, exe: Path, endpoint: str, window_s: float, *, across_face: bool = False,
                 selector: str | None = None) -> None:
        args = [str(exe), "telemetry", "--for", str(window_s), "-c", endpoint, "--format", "json", "--color", "never"]
        if across_face:
            args += ["--across-face", "--contracts", str(CONTRACT)]
        if selector is not None:
            args.append(selector)
        self.args, self.window_s = args, window_s
        self.out = tempfile.TemporaryFile(mode="w+")
        self.p = subprocess.Popen(args, stdout=self.out, stderr=subprocess.PIPE, text=True)
        self.err: list[str] = []
        threading.Thread(target=self._pump, daemon=True).start()
        if not _wait(lambda: any(x.startswith("telemetry: listening") for x in self.err), 15.0, 0.002):
            raise RuntimeError(f"zenctl telemetry did not start listening: {self.err[-3:]}")
        #: on this process's monotonic clock: the line's sighting, from when
        #: the steps start ("a sample put from then on is inside it")
        self.ready_ns = time.monotonic_ns()

    def _pump(self) -> None:
        assert self.p.stderr is not None
        for line in self.p.stderr:
            self.err.append(line.rstrip("\n"))

    def finish(self) -> tuple[int, dict[str, Any]]:
        code = self.p.wait(timeout=self.window_s + 60)
        self.out.seek(0)
        text = self.out.read()
        try:
            doc = json.loads(text)
        except ValueError as e:
            raise RuntimeError(f"zenctl telemetry's output is not one JSON document (exit {code}): {text[:300]!r}; "
                               f"stderr {self.err[-3:]}") from e
        return code, doc


#: zk2py's verdict (telemetry.verdict) as the tool's exit code
EXIT = {tm.FINDING: 1, tm.NO: 0, tm.COULD_NOT_TELL: 2}


def zk2py_view(reader: Reader, since_ns: int, keys: list[str] | None = None) -> dict[str, dict[str, Any]]:
    """What zk2py's reader read, per member, in the tool's terms: kind and
    unit (§2.5), the samples' counts and problems (§2.6), the pairs with
    their time base (§2.10, §2.11)."""
    out: dict[str, dict[str, Any]] = {}
    with reader.lock:
        seen = sorted({a.key for a in reader.arrivals if a.arrival_ns >= since_ns})
    for key in keys if keys is not None else seen:
        got = reader.of("", since_ns, key=key)
        if not got:
            continue
        rid = key.removeprefix(f"{reader.base}/")
        d = reader.decls[rid]
        cs = [c for _, c in got]
        conforming = [c for c in cs if c.conformance == tm.CONFORMS]
        kind = (d.kind, "declared") if d.kind else (conforming[-1].kind, "point") if conforming else (None, None)
        if d.unit:
            unit = (d.unit, "declared")
        elif conforming and conforming[-1].unit is not None:
            unit = (conforming[-1].unit, "point")
        else:
            unit = (None, None)
        bases: list[str] = []
        pairs = reader.state_pairs(key, rid, since_ns=since_ns, bases=bases) if rid.startswith("state/") \
            else reader.pairs(rid.split("/", 1)[1], since_ns, bases)
        problems: dict[str, int] = {}
        for c in cs:
            if c.conformance == tm.DOES_NOT:
                for r in c.reasons:
                    problems[r] = problems.get(r, 0) + 1
        out[key] = {
            "kind": kind, "unit": unit,
            "samples": {"received": len(cs), "conforms": len(conforming),
                        "does_not": sum(c.conformance == tm.DOES_NOT for c in cs),
                        "unobservable": sum(c.conformance == tm.UNOBSERVABLE for c in cs)},
            "problems": problems,
            "pairs": [(a, b, x.continuity, x.epoch, x.via, base, x.rate) for (a, b, x), base in zip(pairs, bases)],
            "conformances": cs, "pair_objects": [x for _, _, x in pairs],
            "answer": tm.member_answer(cs, [x for _, _, x in pairs]),
        }
    return out


def _tool_view(doc: dict[str, Any]) -> dict[str, dict[str, Any]]:
    out = {}
    for r in doc.get("rows", []):
        if r.get("row") != "member" or not r.get("samples", {}).get("received"):
            continue
        smp = r["samples"]
        out[r["key"]] = {
            "kind": (r.get("kind"), r.get("kind_from")), "unit": (r.get("unit"), r.get("unit_from")),
            "samples": {k: smp.get(k) for k in ("received", "conforms", "does_not", "unobservable")},
            "problems": dict(smp.get("problems", {})),
            #: the pair count, beside the list, which holds the latest 16
            "pair_count": r.get("continuity", {}).get("pairs"),
            "pairs_listed": r.get("continuity", {}).get("pairs_listed"),
            #: the member's own answer (telemetry.v1 0.4 §5)
            "answer": r.get("continuity", {}).get("answer"),
            # a histogram's value as zk2py shows it, its count (the whole
            # histogram is compared through its continuity)
            "pairs": [(_short(x.get("earlier")), _short(x.get("later")), x.get("continuity"), x.get("epoch"), x.get("via"),
                       x.get("base"), x.get("rate")) for x in r.get("pairs", [])],
        }
    return out


def _rates_agree(zk: Any, tool: Any, base: str) -> bool:
    """§2.11: a rate timed by the Points' timestamps or the owner's stamps
    is the same double for any two readers; one timed by receipt is each
    reader's own, compared within 5 %."""
    if zk is None or tool is None:
        return zk is tool
    return zk == tool if base in ("timestamp", "stamp") else abs(zk - tool) <= 0.05 * max(abs(zk), abs(tool))


def compare_tool(report: Report, sec: str, run: ToolRun, reader: Reader, *, keys: list[str] | None = None,
                 label: str = "the tool", since_ns: int | None = None, open_finding: str | None = None,
                 writes: bool = True) -> tuple[int, dict[str, Any]]:
    """The tool's reading of the window against zk2py's, member by member:
    kind and unit, the samples' counts and problems, each pair's continuity,
    epoch, evidence, time base and rate; then its exit code against
    :func:`zk2py.telemetry.verdict` over zk2py's own reading."""
    code, doc = run.finish()
    zk, tool = zk2py_view(reader, run.ready_ns if since_ns is None else since_ns, keys), _tool_view(doc)
    margin = f" (the steps started on the tool's listening line, F-154's and F-159's answers; exit {code})"
    if writes:
        # so that the tool mode cannot pass while testing nothing
        report.check(sec, f"{label}: zk2py's reader read samples in the tool's window", bool(zk),
                     f"{sum(v['samples']['received'] for v in zk.values())} samples" if zk
                     else "zk2py's view is empty" + margin)
    same = sorted(tool) == sorted(zk)
    report.check(sec, f"{label}: the members it read samples of are zk2py's", same,
                 f"tool {sorted(tool)}, zk2py {sorted(zk)}" + ("" if same else margin))
    zk_n = {k: v["samples"]["received"] for k, v in zk.items()}
    tool_n = {k: v["samples"]["received"] for k, v in tool.items()}
    report.check(sec, f"{label} read every sample zk2py's reader did ({sum(zk_n.values())} of them)",
                 tool_n == zk_n, f"tool {tool_n}, zk2py {zk_n}" + ("" if tool_n == zk_n else margin))
    for key in sorted(set(zk) & set(tool)):
        z, t = zk[key], tool[key]
        diffs = [f"{what}: zk2py {z[what]}, tool {t[what]}" for what in ("kind", "unit", "samples", "problems")
                 if z[what] != t[what]]
        if t["pair_count"] != len(z["pairs"]):
            diffs.append(f"pair count: zk2py {len(z['pairs'])}, tool {t['pair_count']}")
        if t["pairs_listed"] != len(t["pairs"]):
            diffs.append(f"pairs_listed {t['pairs_listed']}, but {len(t['pairs'])} listed")
        if t["answer"] != z["answer"]:
            diffs.append(f"the member's answer (§5, 0.4): zk2py {z['answer']}, tool {t['answer']}")
        # the tool lists the last pairs alone (F-155): compared on that tail
        tail = z["pairs"][len(z["pairs"]) - len(t["pairs"]):] if t["pairs"] else []
        zp = [x[:6] for x in tail]
        tp = [x[:6] for x in t["pairs"]]
        if zp != tp:
            diffs.append(f"pairs: zk2py {zp}, tool {tp}")
        else:
            bad = [(x, y) for x, y in zip(tail, t["pairs"]) if not _rates_agree(x[6], y[6], x[5])]
            if bad:
                diffs.append(f"rates: {[(x[6], y[6], x[5]) for x, y in bad]}")
        member = key.removeprefix(f"{reader.base}/")
        report.check(sec, f"{label} agrees on {member}: kind and unit, {z['samples']['received']} samples and their "
                          f"problems, {len(z['pairs'])} pairs (continuity, epoch, evidence, time base, rate), "
                          f"its answer {z['answer']}",
                     not diffs, "; ".join(diffs) + margin if diffs else f"pairs {[x[:6] for x in t['pairs']]}")
    want = tm.verdict([(v["conformances"], v["pair_objects"]) for v in zk.values()])
    if open_finding is not None and code != EXIT[want]:
        report.note(sec, f"{label} disagrees ({open_finding}): exit {code}, where zk2py reads {want} (exit "
                         f"{EXIT[want]}); judgement {doc.get('judgement')}")
    else:
        report.check(sec, f"{label}'s exit is zk2py's verdict over the window ({want}: exit {EXIT[want]})",
                     code == EXIT[want], f"exit {code}, judgement {doc.get('judgement')}"
                     + ("" if code == EXIT[want] else margin))
    return code, doc


# -- helpers -------------------------------------------------------------------------------

def _p(pair: tm.Pair) -> str:
    return f"{pair.continuity}/{pair.epoch}/{pair.via}" + (f" rate {pair.rate} {pair.rate_unit}" if pair.rate
                                                          is not None else f" no rate ({pair.no_rate})")


def _all_conform(reader: Reader, res: str, since: int = 0) -> tuple[bool, str]:
    got = reader.of(res, since)
    bad = [(c.conformance, c.reasons) for _, c in got if c.conformance != tm.CONFORMS]
    return bool(got) and not bad, f"{len(got)} samples, not conforming {bad}"


def _epochs(reader: Reader, res: str, since: int = 0, until: int | None = None) -> list[Any]:
    return [c.epoch for a, c in reader.of(res, since) if until is None or a.arrival_ns <= until]


def _pair_is(pair: tm.Pair, continuity: str, epoch: str, via: str | None) -> bool:
    return (pair.continuity, pair.epoch, pair.via) == (continuity, epoch, via)


def _no_finding(*pair_lists: list[tuple[Any, Any, tm.Pair]]) -> tuple[bool, str]:
    every = [p for lst in pair_lists for _, _, p in lst]
    bad = [p for p in every if p.continuity == "decreased" or (p.rate is not None and p.rate < 0)]
    return not bad, f"{len(every)} pairs, {len(bad)} decreased or negative"


def _frames(v: int) -> dict[str, Any]:
    return point(v, "counter")


def _descriptor(bus: Bus, o: Any) -> dict[str, Any] | None:
    from . import live

    got = live.get_descriptor(bus.r_session, f"zk2/{ADDRESS}/@zk/instance/{o.instance}")
    return json.loads(got[0].payload) if got and got[0].ok else None


# -- §1 A counter across a restart ---------------------------------------------------------

def section1(report: Report, mode: Any, contract: Any, tool: Path | None = None) -> None:
    sec = "§1"
    bus = Bus(mode, contract, tool=tool, report=report, sec=sec)
    try:
        o = bus.owner()
        i1 = o.instance
        bus.appeared(i1)
        d = _descriptor(bus, o)
        ifaces = [(x["iface"], x["contract"]) for x in (d or {}).get("interfaces", [])]
        report.check(sec, "setup: the owner implements meter.v1 as zk2py transcribed it from the text (the same "
                          "fingerprint), and lists telemetry.v1 in its descriptor's profiles",
                     ifaces == [("meter.v1", contract.fingerprint)] and "telemetry.v1" in (d or {}).get("profiles", []),
                     f"{ifaces}, profiles {(d or {}).get('profiles')}, zk2py {contract.fingerprint}")
        run = bus.tool_start(16)
        mark1 = time.monotonic_ns()
        for f, dv in [(0, 0), (10, 5), (20, 9)]:
            o.put("frames", _frames(f))
            o.put("drops", dv)
            time.sleep(APART_S)
        o.close()
        report.check(sec, "2. R sees I1's token withdrawn", bus.gone(i1))
        mark3 = time.monotonic_ns()
        o2 = bus.owner()
        i2 = o2.instance
        bus.appeared(i2)
        for f, dv in [(0, 0), (4, 2)]:
            o2.put("frames", _frames(f))
            o2.put("drops", dv)
            time.sleep(APART_S)
        time.sleep(0.3)
        R = bus.R
        for res in ("frames", "drops"):
            ok, why = _all_conform(R, res)
            report.check(sec, f"1. every {res} sample conforms", ok and len(R.of(res)) == 5, why)
        report.check(sec, "1. each frames Point carries I1 in step 1 and I2 in step 3, I1 ≠ I2",
                     _epochs(R, "frames", mark1, mark3) == [i1] * 3 and _epochs(R, "frames", mark3) == [i2] * 2
                     and i1 != i2, f"{_epochs(R, 'frames')}, I1 {i1}, I2 {i2}")
        report.check(sec, "1. the restart is a new session: the stamps' id changes with the instance (core §1.5)",
                     len(R.stamp_ids("frames", mark1, mark3)) == 1 and len(R.stamp_ids("frames", mark3)) == 1
                     and R.stamp_ids("frames", mark1, mark3) != R.stamp_ids("frames", mark3),
                     f"{R.stamp_ids('frames', mark1, mark3)} → {R.stamp_ids('frames', mark3)}")
        fr, dr = R.pairs("frames"), R.pairs("drops")
        got_f = [(a, b, _p(p)) for a, b, p in fr]
        got_d = [(a, b, _p(p)) for a, b, p in dr]
        ts = [c.timestamp_ms for _, c in R.of("frames")]
        arr = [a.arrival_ns for a, _ in R.of("drops")]
        within_f = [fr[0][2], fr[1][2]]
        report.check(sec, "2. frames within step 1: same_epoch, established in-band, 10 over the timestamps' "
                          "difference in 1/s, bit for bit (§2.11)",
                     all(_pair_is(p, "same_epoch", "established", "in_band") and p.rate_unit == "1/s" for p in within_f)
                     and [p.rate for p in within_f] == [tm.rate_of(10, (ts[1] - ts[0]) * 10**6),
                                                         tm.rate_of(10, (ts[2] - ts[1]) * 10**6)],
                     f"{got_f[:2]}")
        within_d = [dr[0][2], dr[1][2]]
        report.check(sec, "2. drops within step 1: same_epoch, established by presence (I1 alone), a rate by "
                          "the arrivals, no unit",
                     all(_pair_is(p, "same_epoch", "established", "presence") and p.rate_unit is None for p in within_d)
                     and [p.rate for p in within_d] == [tm.rate_of(5, arr[1] - arr[0]), tm.rate_of(4, arr[2] - arr[1])],
                     f"{got_d[:2]}")
        report.check(sec, "3. across the restart, frames 20 → 0: reset, established in-band, no rate",
                     fr[2][:2] == (20, 0) and _pair_is(fr[2][2], "reset", "established", "in_band")
                     and fr[2][2].rate is None, f"{got_f[2]}")
        pres = R.presence_of(arr[2], arr[3])
        report.check(sec, "3. drops 9 → 0: reset, established by presence (I1 alone at the earlier arrival, I2 at "
                          "the later), no rate",
                     dr[2][:2] == (9, 0) and _pair_is(dr[2][2], "reset", "established", "presence")
                     and dr[2][2].rate is None and pres is not None and (pres.earlier, pres.later) == (i1, i2),
                     f"{got_d[2]}, presence {pres}")
        report.check(sec, "4. within step 3, both same_epoch again, established, with rates",
                     _pair_is(fr[3][2], "same_epoch", "established", "in_band") and fr[3][2].rate is not None
                     and _pair_is(dr[3][2], "same_epoch", "established", "presence") and dr[3][2].rate is not None,
                     f"{got_f[3]}, {got_d[3]}")
        ok, why = _no_finding(fr, dr)
        report.check(sec, "5. nothing decreased, and no rate across the restart",
                     ok and fr[2][2].rate is None and dr[2][2].rate is None and len(fr) == 4 and len(dr) == 4, why)
        if run is not None:
            compare_tool(report, sec, run, R)
    finally:
        bus.close()


# -- §2 A counter across new_epoch in one process ------------------------------------------

def section2(report: Report, mode: Any, contract: Any, tool: Path | None = None) -> None:
    sec = "§2"
    bus = Bus(mode, contract, tool=tool, report=report, sec=sec)
    try:
        o = bus.owner()
        i1 = o.instance
        bus.appeared(i1)
        run = bus.tool_start(12)
        mark1 = time.monotonic_ns()
        for f, d in [(0, 0), (10, 5), (20, 9)]:
            o.put("frames", _frames(f))
            o.put("drops", d)
            time.sleep(APART_S)
        mark = time.monotonic_ns()
        i2 = o.new_epoch()
        gone = bus.gone(i1)
        with bus.R.lock:
            moves = [(k, inst) for t, k, inst in bus.R.tokens if t >= mark]
        report.check(sec, "2. R sees I2's token declared, then I1's withdrawn: an overlap, never a gap (core §8.1)",
                     gone and moves[:2] == [("put", i2), ("delete", i1)], f"{moves}")
        mark3 = time.monotonic_ns()
        for f, d in [(0, 0), (3, 3)]:
            o.put("frames", _frames(f))
            o.put("drops", d)
            time.sleep(APART_S)
        time.sleep(0.3)
        R, R0 = bus.R, bus.R0
        ids_before, ids_after = R.stamp_ids("frames", mark1, mark), R.stamp_ids("frames", mark3)
        report.check(sec, "1. every frames Point after step 2 carries I2, the process unchanged (one stamp id "
                          "throughout, its session's)",
                     _epochs(R, "frames", mark3) == [i2, i2] and len(ids_before) == 1 and ids_before == ids_after
                     and i1 != i2, f"{_epochs(R, 'frames', mark3)}, stamp ids {ids_before} → {ids_after}")
        fr, dr = R.pairs("frames"), R.pairs("drops")
        arr = [a.arrival_ns for a, _ in R.of("drops")]
        pres = R.presence_of(arr[2], arr[3])
        report.check(sec, "2. frames 20 → 0: reset, established in-band, no rate",
                     fr[2][:2] == (20, 0) and _pair_is(fr[2][2], "reset", "established", "in_band")
                     and fr[2][2].rate is None, _p(fr[2][2]))
        report.check(sec, "2. drops 9 → 0: reset, established by presence (I1 alone at the earlier arrival, I2 alone "
                          "at the later), no rate",
                     dr[2][:2] == (9, 0) and _pair_is(dr[2][2], "reset", "established", "presence")
                     and dr[2][2].rate is None and pres is not None and (pres.earlier, pres.later) == (i1, i2),
                     f"{_p(dr[2][2])}, presence {pres}")
        report.check(sec, "3. then each same_epoch, established, with a rate",
                     _pair_is(fr[3][2], "same_epoch", "established", "in_band") and fr[3][2].rate is not None
                     and _pair_is(dr[3][2], "same_epoch", "established", "presence") and dr[3][2].rate is not None,
                     f"{_p(fr[3][2])}; {_p(dr[3][2])}")
        d0 = R0.pairs("drops")
        report.check(sec, "4. R0 pairs drops 9 → 0 as reset, assumed: the same answer without the evidence",
                     len(d0) == 4 and d0[2][:2] == (9, 0) and _pair_is(d0[2][2], "reset", "assumed", None)
                     and d0[2][2].rate is None, f"{[(a, b, _p(p)) for a, b, p in d0]}")
        if run is not None:
            compare_tool(report, sec, run, R)
    finally:
        bus.close()


# -- §3 A decrease within one epoch --------------------------------------------------------

def section3(report: Report, mode: Any, contract: Any, tool: Path | None = None) -> None:
    sec = "§3"
    bus = Bus(mode, contract, tool=tool, report=report, sec=sec)
    try:
        o = bus.owner()
        i1 = o.instance
        bus.appeared(i1)
        run = bus.tool_start(6)
        o.put("frames", _frames(10))
        o.put("drops", 10)
        time.sleep(APART_S)
        offers = (raw_step(o, "frames", point(7, "counter", epoch=i1)), raw_step(o, "drops", 7))
        time.sleep(0.5)
        R, R0 = bus.R, bus.R0
        ok = all(_all_conform(r, res)[0] and len(r.of(res)) == 2 for r in (R, R0) for res in ("frames", "drops"))
        report.check(sec, "1. every sample conforms: conformance is per sample (§2.6)", ok,
                     f"R {_all_conform(R, 'frames')[1]}; {_all_conform(R, 'drops')[1]}")
        fr, dr = R.pairs("frames"), R.pairs("drops")
        report.check(sec, "2. R pairs frames 10 → 7 as decreased, established in-band, no rate",
                     len(fr) == 1 and fr[0][:2] == (10, 7) and _pair_is(fr[0][2], "decreased", "established", "in_band")
                     and fr[0][2].rate is None, f"{[_p(p) for _, _, p in fr]}")
        report.check(sec, "2. R pairs drops 10 → 7 as decreased, established by presence: the finding, no rate",
                     len(dr) == 1 and _pair_is(dr[0][2], "decreased", "established", "presence")
                     and dr[0][2].rate is None, f"{[_p(p) for _, _, p in dr]}")
        f0, d0 = R0.pairs("frames"), R0.pairs("drops")
        report.check(sec, "3. R0 pairs drops 10 → 7 as reset, assumed: never a finding, and no rate",
                     len(d0) == 1 and _pair_is(d0[0][2], "reset", "assumed", None) and d0[0][2].rate is None,
                     f"{[_p(p) for _, _, p in d0]}")
        report.check(sec, "3. R0 pairs frames 10 → 7 as decreased, established in-band: the epoch rides the Point",
                     len(f0) == 1 and _pair_is(f0[0][2], "decreased", "established", "in_band"),
                     f"{[_p(p) for _, _, p in f0]}")
        report.check(sec, "4. the checked writer, holding its owner to §2.7, refuses both puts of step 2, sent raw",
                     [x.reasons for x in offers] == [["decreased"], ["decreased"]], _offers(offers))
        if run is not None:
            compare_tool(report, sec, run, R)
    finally:
        bus.close()


# -- §4 A Point whose tag disagrees --------------------------------------------------------

def section4(report: Report, mode: Any, contract: Any, tool: Path | None = None) -> None:
    sec = "§4"
    bus = Bus(mode, contract, tool=tool, report=report, sec=sec)
    try:
        o = bus.owner()
        bus.appeared(o.instance)
        run = bus.tool_start(9)
        o.put("frames", _frames(1))
        time.sleep(APART_S)
        offers = [raw_step(o, "frames", point(3.5, "gauge", epoch=o.instance))]
        time.sleep(APART_S)
        offers.append(raw_step(o, "frames", point(2, "counter", unit="By", epoch=o.instance)))
        time.sleep(APART_S)
        offers.append(raw_step(o, "level", "x"))
        time.sleep(APART_S)
        o.put("frames", _frames(5))
        time.sleep(0.5)
        R = bus.R
        frames = R.of("frames")
        lv = R.of("level")
        s2 = frames[1][1] if len(frames) > 1 else None
        s3 = frames[2][1] if len(frames) > 2 else None
        report.check(sec, "1. step 2 reads does_not, tag_disagrees; the member's kind stays counter, declared",
                     s2 is not None and (s2.conformance, s2.reasons, s2.kind, s2.kind_from)
                     == (tm.DOES_NOT, ("tag_disagrees",), "counter", "declared"), f"{s2}")
        report.check(sec, "2. step 3 reads does_not, unit_disagrees; the member's unit stays 1",
                     s3 is not None and (s3.conformance, s3.reasons, s3.unit, s3.unit_from)
                     == (tm.DOES_NOT, ("unit_disagrees",), "1", "declared"), f"{s3}")
        s4 = lv[0][1] if lv else None
        report.check(sec, "3. step 4 (level, the string \"x\") reads does_not, wrong_type",
                     s4 is not None and (s4.conformance, s4.reasons) == (tm.DOES_NOT, ("wrong_type",)), f"{s4}")
        fr = R.pairs("frames")
        report.check(sec, "4. none of them pairs: frames 1 → 5, step 1's with step 5's, same_epoch",
                     len(fr) == 1 and fr[0][:2] == (1, 5) and fr[0][2].continuity == "same_epoch",
                     f"{[(a, b, _p(p)) for a, b, p in fr]}")
        report.check(sec, "5. the checked writer refuses steps 2 to 4 before sending, each sent raw",
                     [x.reasons for x in offers] == [["tag_disagrees"], ["unit_disagrees"], ["wrong_type"]],
                     _offers(offers))
        if run is not None:
            compare_tool(report, sec, run, R)
    finally:
        bus.close()


# -- §5 Histograms -------------------------------------------------------------------------

def section5(report: Report, mode: Any, contract: Any, tool: Path | None = None) -> None:
    sec = "§5"
    bus = Bus(mode, contract, tool=tool, report=report, sec=sec)
    try:
        o = bus.owner()
        bus.appeared(o.instance)
        declared = [0.005, 0.01, 0.025]

        def h(buckets: list[float], counts: list[int], s: float, epoch: str | None = None) -> dict[str, Any]:
            return point({"buckets": buckets, "counts": counts, "count": sum(counts), "sum": s}, "histogram",
                         epoch=epoch)

        run = bus.tool_start(8)
        o.put("latency", h(declared, [3, 10, 2, 1], 0.19))
        time.sleep(APART_S)
        offers = [raw_step(o, "latency", h([0.005, 0.01, 0.05], [3, 10, 2, 1], 0.19, o.instance))]
        time.sleep(APART_S)
        offers.append(raw_step(o, "latency", h(declared, [3, 9, 2, 1], 0.18, o.instance)))
        time.sleep(APART_S)
        o.put("latency", h(declared, [4, 12, 2, 1], -1.0))
        time.sleep(0.5)
        R = bus.R
        got = [(c.conformance, c.reasons) for _, c in R.of("latency")]
        report.check(sec, "1. steps 1, 3 and 4 conform; step 2 reads does_not, bounds_differ",
                     got == [(tm.CONFORMS, ()), (tm.DOES_NOT, ("bounds_differ",)), (tm.CONFORMS, ()),
                             (tm.CONFORMS, ())], f"{got}")
        p = R.pairs("latency")
        report.check(sec, "2. step 1 with step 3: decreased, established in-band (a bucket's count and count fell)",
                     len(p) == 2 and p[0][:2] == (16, 15) and _pair_is(p[0][2], "decreased", "established", "in_band"),
                     f"{[(a, b, _p(x)) for a, b, x in p]}")
        report.check(sec, "3. step 3 with step 4: same_epoch (every count grew, sum fell, which it may)",
                     len(p) == 2 and p[1][:2] == (15, 19) and _pair_is(p[1][2], "same_epoch", "established", "in_band"),
                     f"{[(a, b, _p(x)) for a, b, x in p]}")
        report.check(sec, "4. no pair has a rate: a histogram has none at this version (§2.11)",
                     bool(p) and all(x.rate is None and x.no_rate == "kind" for _, _, x in p),
                     f"{[x.no_rate for _, _, x in p]}")
        report.check(sec, "5. the checked writer refuses steps 2 and 3, each sent raw",
                     [x.reasons for x in offers] == [["bounds_differ"], ["decreased"]], _offers(offers))
        if run is not None:
            compare_tool(report, sec, run, R)
    finally:
        bus.close()


# -- §6 Absence is not zero ----------------------------------------------------------------

def section6(report: Report, mode: Any, contract: Any, tool: Path | None = None) -> None:
    sec = "§6"
    bus = Bus(mode, contract, tool=tool, report=report, sec=sec)
    try:
        o = bus.owner()
        bus.appeared(o.instance)
        run = bus.tool_start(10)
        o.put("level", -70.0)
        for d in (5, 6, 7):
            o.put("drops", d)
            time.sleep(APART_S)
        time.sleep(0.3)
        before = {res: len(bus.R.of(res)) for res in ("frames", "bytes", "drops", "latency", "level")}
        o.close()
        seen = []
        t0 = time.monotonic()
        while time.monotonic() - t0 < 3.0:
            seen.append(bus.R.current("level"))
            time.sleep(0.5)
        after = {res: len(bus.R.of(res)) for res in before}
        levels = [x[0] if x is not None else None for x in seen]
        ages = [x[1] for x in seen if x is not None]
        report.check(sec, "1. R's last sample of level is −70.0 throughout, shown with its age, never 0",
                     levels and all(v == -70.0 for v in levels) and ages == sorted(ages),
                     f"{levels}, ages {[a // 10**6 for a in ages]} ms")
        dr = bus.R.pairs("drops")
        report.check(sec, "2. R pairs drops 5 → 6 and 6 → 7, and nothing after 7: no rate of 0 from the silence",
                     [(a, b) for a, b, _ in dr] == [(5, 6), (6, 7)] and all(p.rate and p.rate > 0 for _, _, p in dr),
                     f"{[(a, b, _p(p)) for a, b, p in dr]}")
        unknown = {res: bus.R.current(res) for res in ("frames", "bytes", "latency")}
        report.check(sec, "3. after step 2, no new sample of any member, and a member with none reads unknown, "
                          "not 0",
                     after == before and all(v is None for v in unknown.values()), f"{before} → {after}, {unknown}")
        if run is not None:
            _, doc = compare_tool(report, sec, run, bus.R)
            latest = {r["key"]: r.get("latest") for r in doc["rows"] if r.get("row") == "member"}
            report.check(sec, "the tool shows level's last value, −70.0, and no value for a member with no sample",
                         latest.get(f"{PREFIX}/level") == -70.0 and latest.get(f"{PREFIX}/bytes") is None,
                         f"{latest}")
    finally:
        bus.close()


# -- §7 An in-band epoch on a bare counter -------------------------------------------------

def section7(report: Report, mode: Any, contract: Any, tool: Path | None = None) -> None:
    sec = "§7"
    bus = Bus(mode, contract, tool=tool, report=report, sec=sec)
    try:
        o = bus.owner()
        i1 = o.instance
        bus.appeared(i1)
        run = bus.tool_start(14)
        sent = []
        for v in (100, 200):
            o.put("bytes", v)
            sent.append(o.instance)
            time.sleep(APART_S)
        o.new_epoch()
        bus.gone(i1)
        for v in (0, 50):
            o.put("bytes", v)
            sent.append(o.instance)
            time.sleep(APART_S)
        mark = time.monotonic_ns()
        # Step 3 (0.2): "the JSON string "" (two bytes) as its attachment".
        offer = raw_step(o, "bytes", 60, "")
        time.sleep(0.5)
        R, R0 = bus.R, bus.R0
        atts = [a.attachment for a, _ in R.of("bytes")]
        report.check(sec, "1. each attachment of steps 1 and 2 is the owner's instance id when it published",
                     atts[:4] == [json.dumps(i).encode() for i in sent] and len(set(sent)) == 2 and sent[0] == i1,
                     f"{atts[:4]}")
        p0 = R0.pairs("bytes")
        report.check(sec, "2. R0, no presence: 100 → 200 same_epoch, established in-band, a rate in By/s",
                     len(p0) >= 1 and p0[0][:2] == (100, 200)
                     and _pair_is(p0[0][2], "same_epoch", "established", "in_band")
                     and p0[0][2].rate is not None and p0[0][2].rate_unit == "By/s", f"{[_p(p) for _, _, p in p0]}")
        report.check(sec, "2. R0: 200 → 0 reset, established in-band; 0 → 50 same_epoch, established in-band",
                     len(p0) == 3 and p0[1][:2] == (200, 0) and _pair_is(p0[1][2], "reset", "established", "in_band")
                     and p0[2][:2] == (0, 50) and _pair_is(p0[2][2], "same_epoch", "established", "in_band"),
                     f"{[(a, b, _p(p)) for a, b, p in p0]}")
        tail = R.of("bytes", mark)
        s3 = tail[0][1] if tail else None
        report.check(sec, "3. R reads step 3 (the JSON text \"\" as its attachment) as does_not, epoch_invalid, "
                          "and pairs it with nothing",
                     s3 is not None and tail[0][0].attachment == b'""'
                     and (s3.conformance, s3.reasons) == (tm.DOES_NOT, ("epoch_invalid",))
                     and [(a, b) for a, b, _ in R.pairs("bytes")] == [(100, 200), (200, 0), (0, 50)], f"{s3}")
        report.check(sec, "3. the checked writer refuses step 3's empty epoch (epoch_invalid, §2.6 check 2), so "
                          "it went raw", offer.reasons == ["epoch_invalid"], _offers([offer]))
        if o.rust:
            # The example's raw takes "<attachment json>": its help says the
            # attachment is JSON, and it refuses "the attachment is not one
            # JSON text", so zero bytes and x cannot be sent through it.
            report.note(sec, "§2.6 check 2's undecodable attachments (zero bytes, x): not sent, the example's raw "
                             "takes an attachment's JSON only")
            if run is not None:
                compare_tool(report, sec, run, R)
            return
        mark = time.monotonic_ns()
        o.raw_bytes("bytes", b"61", b"")
        time.sleep(APART_S)
        o.raw_bytes("bytes", b"62", b"x")
        time.sleep(0.5)
        extra = [(a.attachment, c.conformance, c.reasons) for a, c in R.of("bytes", mark)]
        report.check(sec, "§2.6 check 2 (0.2): an Epoch attachment that does not decode, zero bytes or x, is "
                          "epoch_invalid, paired with nothing",
                     len(extra) == 2 and all(e[1:] == (tm.DOES_NOT, ("epoch_invalid",)) for e in extra)
                     and len(R.pairs("bytes")) == 3, f"{extra}")
        if run is not None:
            compare_tool(report, sec, run, R)
    finally:
        bus.close()


# -- §8 A counter that crosses a face ------------------------------------------------------

def section8(report: Report, mode: Any, contract: Any, tool: Path | None = None) -> None:
    sec = "§8"
    bus = Bus(mode, contract, face=True, tool=tool, report=report, sec=sec)
    try:
        assert bus.face is not None
        time.sleep(0.5)
        o = bus.owner()
        i1 = o.instance
        bus.appeared(i1)
        run = bus.tool_start(15, across_face=True, endpoint=bus.face.endpoint, selector=f"zk2/{ADDRESS}/**")
        for v in (10, 20):
            o.put("drops", v)
            o.put("frames", _frames(v))
            time.sleep(APART_S)
        o.close()
        bus.gone(i1)
        o2 = bus.owner()
        bus.appeared(o2.instance)
        for v in (3, 8):
            o2.put("drops", v)
            o2.put("frames", _frames(v))
            time.sleep(APART_S)
        time.sleep(0.5)
        F = bus.face.R0F
        with F.lock:
            tokens_over_face = list(F.tokens)
        report.check(sec, "setup: R0F, across the face, receives the data keys and no instance token (link.v1 §2.6)",
                     len(F.of("drops")) == 4 and len(F.of("frames")) == 4 and not tokens_over_face,
                     f"{len(F.of('drops'))} drops, {len(F.of('frames'))} frames, tokens {tokens_over_face}")
        with bus.face.N2.lock:
            near = [(k, inst) for _, k, inst in bus.face.N2.tokens]
        report.check(sec, "setup, the control: N2, on R2 off the face, sees both instances' tokens come and go",
                     near == [("put", i1), ("delete", i1), ("put", o2.instance)], f"{near}")
        for name, reader in (("R0", bus.R0), ("R0F", F)):
            dr, fr = reader.pairs("drops"), reader.pairs("frames")
            report.check(sec, f"1. {name}: drops 10 → 20 same_epoch, assumed, with a rate; 20 → 3 reset, assumed, no "
                              "rate; 3 → 8 same_epoch, assumed",
                         [(a, b) for a, b, _ in dr] == [(10, 20), (20, 3), (3, 8)]
                         and _pair_is(dr[0][2], "same_epoch", "assumed", None) and dr[0][2].rate is not None
                         and _pair_is(dr[1][2], "reset", "assumed", None) and dr[1][2].rate is None
                         and _pair_is(dr[2][2], "same_epoch", "assumed", None),
                         f"{[(a, b, _p(p)) for a, b, p in dr]}")
            report.check(sec, f"2. {name}: frames 20 → 3 reset, established in-band: the Point's epoch crossed with it",
                         len(fr) == 3 and fr[1][:2] == (20, 3) and _pair_is(fr[1][2], "reset", "established", "in_band"),
                         f"{[(a, b, _p(p)) for a, b, p in fr]}")
            ok, why = _no_finding(dr, fr)
            report.check(sec, f"3. {name}: nothing decreased, and no negative rate", ok, why)
        if run is not None:
            compare_tool(report, sec, run, F, label="the tool across the face (--across-face)")
    finally:
        bus.close()


# -- §9 A state counter seeded -------------------------------------------------------------

def section9(report: Report, mode: Any, contract: Any, tool: Path | None = None) -> None:
    """R as a late joiner of ``total`` (core §4.2): its subscription first,
    then S4's GET (target ``All``, consolidation ``Latest``), whose reply it
    holds unread until 12 has arrived; a witness subscribed from the start
    shows the stamp of O's put of 10."""
    import zenoh

    from . import live

    sec = "§9"
    bus = Bus(mode, contract, tool=tool, report=report, sec=sec)
    witness_session = live.open_client(bus.endpoint)
    witness = Reader(witness_session, contract, presence=True, selector=TOTAL)
    if witness.unseeded is not None:
        report.note(sec, f"the witness: {witness.unseeded}")
    try:
        o = bus.owner()
        i1 = o.instance
        bus.appeared(i1)
        o.put("total", 10)
        _wait(lambda: len(witness.of("", key=TOTAL)) == 1, 3.0)
        time.sleep(0.3)
        # The tool starts after step 1, so its own S4 GET seeds 10.
        run = bus.tool_start(8)
        R = bus.R
        R.subscribe(TOTAL)
        time.sleep(0.2)
        replies = R.session.get(TOTAL, target=zenoh.QueryTarget.ALL, consolidation=zenoh.ConsolidationMode.LATEST,
                                timeout=2.0)
        time.sleep(0.3)
        o.put("total", 12)
        _wait(lambda: len(R.of("", key=TOTAL)) == 1, 3.0)
        held = []
        for r in replies:
            if r.ok is not None:
                s = r.ok
                R.record(str(s.key_expr), s.payload.to_bytes(), s.attachment, s.timestamp, "get")
                held.append(seed.Stamp.of(s.timestamp))
        time.sleep(0.2)
        o.put("total", 15)
        _wait(lambda: len(R.of("", key=TOTAL)) == 3, 3.0)
        time.sleep(0.3)
        got = R.of("", key=TOTAL)
        put10 = witness.of("", key=TOTAL)
        report.check(sec, "2. O answers the GET with 10, under its put's stamp (core S2)",
                     len(held) == 1 and bool(put10) and held[0] == put10[0][0].stamp and len(got) > 1
                     and json.loads(got[1][0].payload) == 10,
                     f"reply stamps {held}, the put's {put10[0][0].stamp if put10 else None}")
        report.check(sec, "1. R received 12, then 10, then 15, and every sample conforms",
                     [(json.loads(a.payload), a.via) for a, _ in got] == [(12, "subscription"), (10, "get"),
                                                                         (15, "subscription")]
                     and all(c.conformance == tm.CONFORMS for _, c in got),
                     f"{[(a.payload, a.via, c.conformance) for a, c in got]}")
        merged = R.state_pairs(TOTAL, "state/total")
        a12 = [a for a, _ in got if json.loads(a.payload) == 12]
        a15 = [a for a, _ in got if json.loads(a.payload) == 15]
        want_rate = tm.rate_of(3, a15[0].stamp_ns - a12[0].stamp_ns) if a12 and a15 else None
        report.check(sec, "2. through the merge, the reply is not kept: R pairs 12 → 15, same_epoch, established by "
                          "presence, 3 over the owner's stamps' nanoseconds",
                     [(a, b) for a, b, _ in merged] == [(12, 15)]
                     and _pair_is(merged[0][2], "same_epoch", "established", "presence")
                     and merged[0][2].rate == want_rate,
                     f"{[(a, b, _p(p)) for a, b, p in merged]}, want rate {want_rate}")
        by_receipt = R.state_pairs(TOTAL, "state/total", merged=False)
        report.check(sec, "3. paired in the order received, 12 → 10 would read decreased, established by presence; "
                          "through the merge, R pairs nothing decreased",
                     bool(by_receipt) and by_receipt[0][:2] == (12, 10)
                     and _pair_is(by_receipt[0][2], "decreased", "established", "presence")
                     and _no_finding(merged)[0], f"{[(a, b, _p(p)) for a, b, p in by_receipt]}")
        report.check(sec, "§2.11 (0.2): each stamp's nanoseconds, as zenoh-python reads them, are NTP64's as_nanos "
                          "as the text states it, the fraction rounded up",
                     bool(got) and all(a.stamp is not None and a.stamp_ns == tm.ntp64_ns(a.stamp.time) for a, _ in got),
                     f"{[(a.stamp_ns, a.stamp and tm.ntp64_ns(a.stamp.time)) for a, _ in got]}")
        if run is not None:
            # The tool is a late joiner of its own, its GET's reply 10 and
            # then 12 and 15: compared with the witness, which subscribed
            # before 10 and so read the same three in the owner's order.
            compare_tool(report, sec, run, witness, keys=[TOTAL], since_ns=0,
                         label="the tool (its own GET seeding 10), against the witness")
    finally:
        witness.close()
        witness_session.close()
        bus.close()


def tool_probes(report: Report, contract: Any, tool: Path) -> None:
    """§5 "Over many pairs" (0.3, 0.4) and "Over many samples" (0.5), the
    tool's verdict over windows built for it, each against zk2py's
    (:func:`zk2py.telemetry.verdict`): an undecodable sample beside a
    conforming one, a member heard under an invalid declaration
    (``meterx.v1``'s ``stream/odd``) and one in scope unheard; counters
    whose every pair was assumed, histograms likewise (both across a face,
    so no presence is read), a counter's one sample with no pair (F-152),
    one counter established beside another with no pair (0.4: each is
    asked), and no sample at all. Then the tool's answers to F-154 (a
    sample put on the "listening" line is inside the window) and F-155
    (``pairs_listed``)."""
    sec = "§5 over many pairs and samples"
    bus = Bus("zk2py", contract, tool=tool, report=report, sec=sec)
    xsession: Any = None
    X: Reader | None = None
    try:
        from . import live
        from .contract import load_contract

        # meterx.v1 is served throughout, its stream/odd's declaration
        # invalid: every window but the one that writes it checks that "only
        # what was read counts" (§5, 0.5).
        meterx = load_contract(CONTRACT.parent / "meterx.v1.toml", check_file_name=True)
        o = bus.owner(extra=[meterx])
        bus.appeared(o.instance)
        xbase = f"zk2/{ADDRESS}/meterx.v1"
        xsession = live.open_client(bus.endpoint)
        X = Reader(xsession, meterx, presence=True, base=xbase)
        face: dict[str, Any] = {"across_face": True, "selector": f"zk2/{ADDRESS}/**"}
        run = bus.tool_start(4, **face)
        assert run is not None
        for v in (1, 2, 3):
            o.put("drops", v)
            time.sleep(0.7)
        compare_tool(report, sec, run, bus.R0, label="the tool, counters whose every pair was assumed")
        run = bus.tool_start(4, **face)
        assert run is not None
        for counts in ([1, 0, 0, 0], [2, 0, 0, 0], [3, 1, 0, 0]):
            # raw: the checked writer would give the Point its epoch
            o.raw("latency", point({"buckets": [0.005, 0.01, 0.025], "counts": counts, "count": sum(counts),
                                    "sum": 0.1}, "histogram"))
            time.sleep(0.7)
        compare_tool(report, sec, run, bus.R0, label="the tool, histograms whose every pair was assumed")
        run = bus.tool_start(4)
        assert run is not None
        o.put("drops", 10)
        compare_tool(report, sec, run, bus.R, label="the tool, a counter's one sample and no pair (F-152, 0.4)")
        run = bus.tool_start(5)
        assert run is not None
        for v in (100, 200):
            o.put("frames", point(v, "counter"))
            time.sleep(0.7)
        o.put("drops", 11)
        compare_tool(report, sec, run, bus.R,
                     label="the tool, frames established in-band beside drops' one sample (0.4: each is asked)")
        run = bus.tool_start(4)
        assert run is not None
        compare_tool(report, sec, run, bus.R, label="the tool, no sample at all", writes=False)
        run = bus.tool_start(3)
        assert run is not None
        o.put("level", -70.0)
        time.sleep(0.4)
        o.raw_bytes("level", b"5 x", None)
        compare_tool(report, sec, run, bus.R,
                     label="the tool, a gauge's conforming sample beside an undecodable one (F-157, 0.5)")
        run = bus.tool_start(3)
        assert run is not None
        o.owner.publish(f"{xbase}/stream/odd", b"1.5")
        o.owner.publish(f"{xbase}/stream/good", b"2.5")
        compare_tool(report, sec, run, X,
                     label="the tool, a member under an invalid declaration heard, beside a valid gauge (0.5)")
        run = bus.tool_start(3)
        assert run is not None
        o.owner.publish(f"{xbase}/stream/good", b"3.5")
        compare_tool(report, sec, run, X,
                     label="the tool, the invalid resource in scope and unheard, a valid gauge heard (0.5)")
        # F-154's answer: a sample put the moment the line appears is inside
        # the window, three tries
        caught = []
        for v in (20, 21, 22):
            run = bus.tool_start(2, selector=f"{PREFIX}/drops")
            assert run is not None
            o.put("drops", v)
            code, doc = run.finish()
            rows = [r for r in doc["rows"] if r.get("row") == "member"]
            caught.append(rows[0]["samples"]["received"] if rows else 0)
        report.check(sec, "F-154's answer: a sample put the moment \"listening\" appears is inside the window, "
                          "in three tries of three", caught == [1, 1, 1], f"received {caught}")
        # F-155's answer: pairs_listed says when the listed pairs are a tail
        run = bus.tool_start(5, selector=f"{PREFIX}/drops")
        assert run is not None
        for v in range(30, 55):
            o.put("drops", v)
            time.sleep(0.12)
        code, doc = run.finish()
        m = [r for r in doc["rows"] if r.get("row") == "member"]
        c = m[0]["continuity"] if m else {}
        report.check(sec, "F-155's answer: 25 samples give 24 pairs, the row lists the latest 16, and pairs_listed "
                          "says 16",
                     c.get("pairs") == 24 and c.get("pairs_listed") == 16 and len(m[0].get("pairs", [])) == 16
                     and m[0]["pairs"][-1]["later"] == 54, f"{ {k: c.get(k) for k in ('pairs', 'pairs_listed')} }, "
                     f"{len(m[0].get('pairs', [])) if m else None} listed")
    finally:
        # the meterx reader and its session are closed however the probes end
        if X is not None:
            X.close()
        if xsession is not None:
            try:
                xsession.close()
            except Exception:  # noqa: BLE001 - closing anyway
                pass
        bus.close()


#: the "listening" line's words for each opening, as zenctl-th's --help and
#: line give them (F-160's answer), anchored on the line's start. The tool's
#: `telemetry --help` and its stderr line are the source: a later rewording
#: of either is updated here, in this one place.
LISTENING = "telemetry: listening"
OPENING_WORDS = {
    "get": re.compile(r"; across the face the owner answered (\d+) of (\d+) S4 GET\(s\) of a state metric resource "
                      r"in scope after the subscriptions, so it holds them"),
    "settle": re.compile(r"this line comes after a [0-9.]+s margin \(--settle\), never proof"),
    "descriptor": re.compile(r"; each of the (\d+) owner instance\(s\) in scope answered its descriptor GET after the "
                             r"subscriptions, so they hold them"),
}


def _worded(line: str, opening: str) -> bool:
    """Whether a "listening" line says it stood on ``opening``: the line
    starts with "telemetry: listening", holds that opening's words, with
    every GET it names answered (A of N, A = N >= 1), and none of another's."""
    if not line.startswith(LISTENING):
        return False
    m = OPENING_WORDS[opening].search(line)
    if m is None or any(OPENING_WORDS[o].search(line) for o in OPENING_WORDS if o != opening):
        return False
    if opening == "get":
        return int(m.group(1)) == int(m.group(2)) >= 1
    if opening == "descriptor":
        return int(m.group(1)) >= 1
    return True


def tool_opening(report: Report, contract: Any, tool: Path, tries: int = 20) -> None:
    """F-159's answer (zenctl-tg) and F-160's (zenctl-th): the "listening"
    line comes after a round trip that reaches the owner's own session, and
    says which one it stood on; a sample put the moment it appears is inside
    the window. Asserted three ways, ``tries`` times each, with no margin,
    each window's exit against zk2py's verdict over the same reading:
    - across the real face, a state metric resource in scope holding a value
      (``total``, checked held first by an S4 GET): on S4's GET of it, "the
      owner answered 1 of 1 S4 GET(s) … so it holds them";
    - across the real face, streams alone in scope: on ``--settle`` (0.25
      s), "a … margin (--settle), never proof";
    - with no face: on the owner's descriptor GET, "each of the 1 owner
      instance(s) in scope answered its descriptor GET … so they hold
      them".

    ``--only opening`` runs it alone."""
    import zenoh

    sec = "the tool's opening (F-159, F-160)"
    bus = Bus("zk2py", contract, face=True, tool=tool, report=report, sec=sec)
    try:
        assert bus.face is not None
        o = bus.owner()
        bus.appeared(o.instance)
        o.put("total", 1)
        time.sleep(0.5)
        held = [json.loads(r.ok.payload.to_bytes()) for r in bus.r_session.get(
            TOTAL, target=zenoh.QueryTarget.ALL, consolidation=zenoh.ConsolidationMode.LATEST, timeout=2.0)
            if r.ok is not None]
        report.check(sec, "before the first case: the owner holds total's value, which its S4 GET answers (core S2)",
                     held == [1], f"the GET's replies {held}")
        v = 1000
        cases = [
            ("across the real face, total in scope: on S4's GET", "get", bus.face.R0F,
             {"across_face": True, "endpoint": bus.face.endpoint, "selector": f"zk2/{ADDRESS}/**"}),
            ("across the real face, streams alone in scope: on --settle", "settle", bus.face.R0F,
             {"across_face": True, "endpoint": bus.face.endpoint, "selector": f"{PREFIX}/**"}),
            ("with no face: on the owner's descriptor GET", "descriptor", bus.R,
             {"selector": f"{PREFIX}/drops"}),
        ]
        drops = f"{PREFIX}/drops"
        for note, opening, reader, kw in cases:
            got, lines, exits, seen = [], [], [], []
            for _ in range(tries):
                run = bus.tool_start(0.6, **kw)
                assert run is not None
                v += 1
                o.put("drops", v)
                code, doc = run.finish()
                rows = [r for r in doc["rows"] if r.get("row") == "member" and r["key"] == drops]
                got.append(rows[0]["samples"]["received"] if rows else 0)
                lines += [x for x in run.err if x.startswith(LISTENING)]
                # zk2py's verdict over the same drops sample: a counter read
                # with no pair could not tell (§5, 0.4)
                zk = zk2py_view(reader, run.ready_ns, [drops])
                seen.append(sum(x["samples"]["received"] for x in zk.values()))
                want = EXIT[tm.verdict([(x["conformances"], x["pair_objects"]) for x in zk.values()])]
                exits.append((code, want))
            report.check(sec, f"{note}: a sample put the moment \"listening\" appears is inside the window, "
                              f"{tries} tries of {tries}", got == [1] * tries,
                         f"read {sum(got)} of {tries}: {got}")
            report.check(sec, f"{note}: each line, \"{LISTENING} …\", says it stood on this opening, in the words "
                              "--help gives", len(lines) == tries and all(_worded(x, opening) for x in lines),
                         f"{len(lines)} lines; {sorted(set(lines))[:2]}")
            # so that the exit check cannot pass over an empty view
            report.check(sec, f"{note}: zk2py's reader read the sample in each window, {tries} of {tries}",
                         seen == [1] * tries, f"read {sum(seen)} of {tries}: {seen}")
            report.check(sec, f"{note}: each exit is zk2py's verdict over the window (a counter's one sample: 2)",
                         all(c == w for c, w in exits) and seen == [1] * tries,
                         f"(exit, zk2py's) {sorted(set(exits))}")
    finally:
        bus.close()


def measure_stamps(report: Report) -> None:
    """§2.11 (0.2, F-142): over consecutive HLC stamps of one session,
    zenoh-python's ``NTP64.as_nanos()`` against :func:`tm.ntp64_ns`, the
    text's conversion; and how often the converted difference of two raw
    values differs from the difference of two conversions, which the text
    rules out."""
    import zenoh

    conf = zenoh.Config()
    conf.insert_json5("mode", json.dumps("peer"))
    conf.insert_json5("scouting/multicast/enabled", "false")
    conf.insert_json5("timestamping/enabled", "true")
    s = zenoh.open(conf)
    try:
        n, agree, e_differs, prev = 2000, 0, 0, None
        for _ in range(n):
            t = s.new_timestamp().get_time_as_ntp64()
            raw = int(repr(t))
            agree += t.as_nanos() == tm.ntp64_ns(raw)
            if prev is not None:
                pt, praw = prev
                e_differs += (t.as_nanos() - pt.as_nanos()) != (((raw - praw) * 10**9) >> 32)
            prev = (t, raw)
        report.check("§2.11", f"F-142 (0.2): the text's conversion equals zenoh-python's as_nanos() on {n} HLC stamps",
                     agree == n, f"{agree} of {n}")
        report.note("§2.11", f"the converted difference's floor differs from the difference of two conversions in "
                             f"{e_differs} of {n - 1} consecutive pairs")
    finally:
        s.close()


SECTIONS = {"1": section1, "2": section2, "3": section3, "4": section4, "5": section5, "6": section6,
            "7": section7, "8": section8, "9": section9}


def run(report: Report, mode: Any, only: list[str] | None, tool: Path | None = None) -> None:
    contract = load()
    for name in [n for n in SECTIONS if not only or n in only]:  # "opening" names no section
        try:
            SECTIONS[name](report, mode, contract, tool)
        except Exception as e:  # noqa: BLE001 - a section that cannot run is a failure
            import traceback

            traceback.print_exc()
            report.check(f"§{name}", "the section ran", False, f"{type(e).__name__}: {e}")
    if tool is not None and (not only or "opening" in only):
        try:
            tool_opening(report, contract, tool)
        except Exception as e:  # noqa: BLE001
            import traceback

            traceback.print_exc()
            report.check("the tool's opening (F-159, F-160)", "the tries ran", False, f"{type(e).__name__}: {e}")
    if tool is not None and (not only or "5" in only):
        try:
            tool_probes(report, contract, tool)
        except Exception as e:  # noqa: BLE001
            import traceback

            traceback.print_exc()
            report.check("§5 over many pairs", "the probes ran", False, f"{type(e).__name__}: {e}")
    measure_stamps(report)


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="python -m zk2py.telemetry_scenarios", description=__doc__.split("\n")[0])
    ap.add_argument("--only", action="append", choices=[*sorted(SECTIONS, key=int), "opening"],
                    help="run only these sections; \"opening\" is the tool's opening (with --tool)")
    ap.add_argument("--owner", type=Path, default=None,
                    help="run against the Rust owner example at this path (--telemetry); default $ZK2PY_OWNER")
    ap.add_argument("--tool", type=Path, default=None,
                    help="judge beside zk2py's reader with zenctl at this path (zenctl telemetry); default "
                         "$ZK2PY_ZENCTL")
    args = ap.parse_args(argv)
    mode: Any = "zk2py"
    env = os.environ.get("ZK2PY_OWNER") or None
    owner, source = (args.owner, "--owner") if args.owner is not None else (env, "ZK2PY_OWNER")
    if owner is not None:
        exe = Path(owner)
        if not exe.is_file():
            print(f"error: {exe} not found (the path came from {source})", file=sys.stderr)
            return 2
        mode = exe
    zenctl, zsource = (args.tool, "--tool") if args.tool is not None else (os.environ.get("ZK2PY_ZENCTL") or None,
                                                                            "ZK2PY_ZENCTL")
    tool = None
    if zenctl is not None:
        tool = Path(zenctl)
        if not tool.is_file():
            print(f"error: {tool} not found (the path came from {zsource})", file=sys.stderr)
            return 2
    if args.only and "opening" in args.only and tool is None:
        print("error: --only opening is the tool's window opening, and needs --tool <zenctl> (or ZK2PY_ZENCTL)",
              file=sys.stderr)
        return 2
    report = Report()
    if tool is not None:
        try:
            v = subprocess.run([str(tool), "--version"], capture_output=True, text=True, timeout=30)
            report.note("tool", f"{tool}: {(v.stdout or v.stderr).strip() or f'--version printed nothing (exit {v.returncode})'}")
        except (OSError, subprocess.TimeoutExpired) as e:
            report.note("tool", f"{tool}: --version did not run ({e})")
    run(report, mode, args.only, tool)
    passed = sum(1 for r in report.rows if r[2])
    failed = len(report.rows) - passed
    who = "zk2py's owner" if mode == "zk2py" else "the owner example"
    if tool is not None:
        who += ", zenctl telemetry judging beside zk2py's reader"
    print(f"telemetry scenarios ({who}): {passed} passed, {failed} failed, {len(report.not_run)} not run")
    for sec, name, ok, detail in report.rows:
        if not ok:
            print(f"FAIL [{sec}] {name}: {detail}")
    return 1 if failed else 2 if report.not_run and not args.only else 0


if __name__ == "__main__":
    rc = main()
    sys.stdout.flush()
    sys.stderr.flush()
    # zenoh-python can hang tearing down at interpreter exit; every session
    # here is already closed.
    os._exit(rc)

"""A generic watcher: ``python -m zk2py.echo_tool -c ENDPOINT [--for SECS]
[--count N] [--format json|ndjson] [--contracts FILE]… [--epoch SECS |
--catalog FILE] [--across-face SYSTEM/SERVICE]… [SELECTOR]`` (#609, rounds
83 and 84; packed records since the mend after round 94; across a face and
the catalog since round 95).

Written from ``spec/observer.md`` §8.1 and §8.2 (text 0.2; the medium, text
0.3, F-292) and ``spec/scenarios/grammar.md`` §1 step 5 (core 0.59, F-287),
as a tool's implementer reads them. It subscribes to ``SELECTOR``,
base-relative and in no namespace (``zk2/**`` by default), on a callback
(observer §3.1's handlers bind no data subscriber, but a callback keeps
nothing bounded), for ``--for`` seconds or until ``--count`` samples.

With ``--format json``, the default, it prints one JSON document:

- ``samples``: each sample as it arrived, its key, its kind token where the
  key parses as zk2 (core §1.1), its byte count and its encoding. A key that
  is not zk2 is a fact, not an error (observer §8.1): shown, with its kind
  ``null`` and ``zk2`` false. Each put's payload is **sniffed**
  (``sniffed``, :func:`zk2py.decoding.sniff`): a structural rendering, JSON,
  CBOR, text or hexadecimal, since the tool holds no declared type for it.
  ``declared_type`` says which case that is and where it stopped resolving
  the key (:func:`zk2py.decoding.unheld`): "A payload whose declared type
  the tool does not hold is sniffed (§8.1). That is a foreign key's, or a
  zk2 key's whose contract is not in hand or is unavailable (§2). The tool
  says which, naming where it stopped resolving the key" (observer §7.3,
  text 0.12, F-406). The tool reads no descriptor and holds no contract, so
  every zk2 data key stops at its contract, not in hand;
- ``scope``: the selector, the kind tokens it **watches** (the plain kinds
  ``stream``, ``state`` and ``events`` it reaches, and any verbatim kind it
  names), and the verbatim kinds it **excludes** (:func:`zk2py.live.blind_spots`),
  with the sentence observer §8.2 asks of it: "A tool that offers a
  wildcard scope either names the verbatim kinds it watches beside it, or
  states that they are excluded, in every medium it writes: a field of its
  report, or, for a stream of rows that has no place for one (NDJSON), its
  standard error." The same sentence goes to standard error before it
  listens, so a person reading either medium has it.

With ``--format ndjson`` it prints each sample as one JSON line as it
arrives, the rows of ``samples`` and nothing else: a stream of rows has no
place for the scope, so standard error states it (:func:`scope_line`),
before the first row, whatever the format (observer §8.2, text 0.3).

zenoh's callback only queues each row (:class:`Rows`); the main thread
writes them, so a reader slow to take standard output never stalls the
session's delivery. A write that fails (a closed pipe, any ``OSError``)
ends the run: the tool says on standard error that the stream was cut and
exits 2, never 0 on a truncated stream (review-bot on PR #955).

At its start it says on standard error what its ``RLIMIT_MEMLOCK`` admits
(observer §7.4, :func:`zk2py.live.memlock_said`; round 89).

Without ``--contracts`` it holds no contract, so every payload is one whose
declared type it does not hold, which it sniffs as observer §7.3 says.

**Packed records** (observer §7.3, text 0.16; core §7.6). Each
``--contracts`` file is one revision the tool holds of its interface, at
most one per interface, since the tool reads no descriptor to choose
between two. A zk2 data key whose interface it holds and whose resource is
of a ``packed`` type is read through that type's layout, under the tag its
envelope's CH carried: ``decoded`` is the record's JSON reading, and one
the layout refuses, or that came in no envelope, is ``undecodable``, the
refusal named. ``declared_type`` names the type ``packed:<name>``, decoded
or not. Its ``time`` fields are read under ``--epoch``, the deployment's,
0, the Unix epoch, where it states none. A held type of another kind is
still sniffed, and ``declared_type`` says so: decoding protobuf and JSON
Schema types field by field needs more than zk2py's echo holds (README,
"What it does not cover").

**Across a constrained face** (core §8.5, 0.78; observer §7.5, text 0.18).
``--across-face SYSTEM/SERVICE``, repeatable, needs ``--contracts`` and no
``SELECTOR``: the tool subscribes by the scopes the contracts in hand give
for those addresses, each concrete resource by its exact key and each
template by its selector, an event's ULID chunk a wildcard
(:func:`zk2py.face.scopes`), one subscriber each. No descriptor crosses, so
the addresses and their contracts are the deployment's word. The scope says
what it excludes: operations, control keys, ``@adv``, and every key no held
contract declares there (observer §8.2).

**A catalog** (core §8.6, 0.79). ``--catalog FILE`` is the deployment's
catalog, read as §8.6 reads one; a refused one is refused input, exit 2,
naming the refusal. Its epoch is the one packed ``time`` fields are read
under, in place of ``--epoch``: across a face, which catalog an owner writes
under is the deployment's word.

It judges nothing, so it exits 0 once it has watched, and 2 when it could
not (the session did not open, the selector is not a key expression, or
its standard output refused a write).
"""

from __future__ import annotations

import argparse
import json
import queue
import sys
import threading
import time
from typing import Any, Callable, TextIO

from . import decoding, live

#: The plain kinds by the data form's key (core §1.1: the kind token at
#: position 5, then one resource chunk or more; an event's last chunk is
#: its ULID, core §2.6). A selector watches a kind when it intersects its
#: form, by zenoh's intersection.
PLAIN_REACH = (
    ("stream", "zk2/*/*/*/stream/*/**"),
    ("state", "zk2/*/*/*/state/*/**"),
    ("events", "zk2/*/*/*/events/*/**"),
)


def scope_of(selector: str) -> dict[str, Any]:
    """Observer §8.2 for one base-relative selector: the kind tokens it
    watches, the verbatim kinds it excludes, and what a tool says of them."""
    import zenoh

    sel = zenoh.KeyExpr(selector)
    excluded = live.blind_spots(selector)
    watched = [k for k, form in PLAIN_REACH if sel.intersects(zenoh.KeyExpr(form))] + [
        k for k, _ in live.VERBATIM_REACH if k not in excluded]
    if excluded:
        says = (f"{selector} excludes {', '.join(excluded)}: `*` and `**` never match a verbatim chunk, so what it "
                f"does not show of them is out of its reach, not absent (observer §8.2; core §1.3)")
    else:
        says = f"{selector} excludes no verbatim kind (observer §8.2)"
    return {"selector": selector, "watched": watched, "excluded": excluded, "says": says}


def face_scope(selectors: list[str], addresses: list[str]) -> dict[str, Any]:
    """Observer §8.2 for observer §7.5's scopes: what they watch and what
    they exclude."""
    watched = sorted({sel.split("/")[4] for sel in selectors if len(sel.split("/")) > 4})
    says = (f"across a constrained face, {len(selectors)} scopes from the contracts in hand at "
            f"{', '.join(addresses)}: each concrete resource by its exact key and each template by its selector "
            "(observer §7.5); they exclude operations (@op), control keys (@zk), @adv sidecars, and every key no "
            "held contract declares at those addresses (observer §8.2)")
    return {"selector": None, "selectors": selectors, "across_face": addresses, "watched": watched,
            "excluded": ["@op", "@zk", "@adv"], "says": says}


def scope_line(scope: dict[str, Any]) -> str:
    """What standard error says of a scope, in every format: what it
    watches and what it excludes (observer §8.2). In NDJSON it is the only
    medium that says it."""
    return f"echo: watching {', '.join(scope['watched']) or 'no kind'}; {scope['says']}"


class Held:
    """The contracts a tool holds (``--contracts``), one revision per
    interface: what each zk2 data key's resource declares."""

    def __init__(self, contracts: list[Any]) -> None:
        from . import templates

        self.by_iface: dict[str, Any] = {}
        for c in contracts:
            if c.interface in self.by_iface:
                raise ValueError(f"{c.interface}: two revisions held, and this tool reads no descriptor to choose "
                                 "between them")
            self.by_iface[c.interface] = c
        self._templates = {
            name: [(templates.parse_template(r["template"]), r) for r in c.canonical["resources"]]
            for name, c in self.by_iface.items()}

    def resource(self, parsed: dict[str, Any]) -> tuple[Any, dict[str, Any]] | None:
        """The (contract, resource) a parsed data key is a sample of, or
        None where its interface is not held or no resource declares it."""
        from . import templates

        c = self.by_iface.get(parsed["iface"])
        if c is None:
            return None
        chunks = parsed["resource"][:-1] if parsed["token"] == "events" else parsed["resource"]
        rows = [(t, r) for t, r in self._templates[parsed["iface"]] if r["token"] == parsed["token"]]
        best = templates.resolve([t for t, _ in rows], chunks)
        if best is None:
            return None
        return c, next(r for t, r in rows if t is best[0])


def _held_type(c: Any, r: dict[str, Any]) -> dict[str, Any]:
    """``declared_type`` for a key whose contract the tool holds."""
    t = r["type"]
    name = {"packed": f"packed:{t.get('name')}", "jsonschema": f"json:{t.get('name')}",
            "raw": t.get("media_type")}.get(t["kind"], t.get("name") or t.get("message"))
    return {"held": True, "name": name, "kind": t["kind"],
            "says": f"{c.interface}'s {r['token']}/{r['template']} declares {name}"
                    + ("" if t["kind"] == "packed" else ": zk2py's echo decodes packed types alone, so it is sniffed")}


def row_of(key: str, delete: bool, size: int, encoding: str, payload: bytes | None = None,
           attachment: bytes | None = None, held: Held | None = None, epoch: int | None = None) -> dict[str, Any]:
    """One sample's row, the same in the JSON document's ``samples`` and as
    an NDJSON line: its key, whether it is zk2, its kind token (``None``
    for a key that is not zk2, a fact and not an error, observer §8.1);
    ``declared_type``, what the tool says of a payload whose declared type
    it does not hold, which case and where it stopped resolving the key;
    and ``sniffed``, a put's payload sniffed (observer §7.3, 0.12), ``None``
    for a delete, which has none, or when no payload was handed in.

    **The envelope first** (observer §7.3, text 0.14). A sample on a zk2
    stream, state or event key under ``zenoh/bytes`` is read as core §7.5's
    envelope: ``envelope`` is its in-band stamp, a run and a seq, never a
    time (observer §8.3, 0.15), and what is sniffed is its record. A
    refused envelope is ``undecodable``, its refusal named, and its bytes
    are never sniffed.

    **A packed type held** (``held``, observer §7.3 at 0.16): its put is
    read through the type's layout under its envelope's tag, ``decoded``
    its JSON reading, or ``undecodable``, ``undecodable as packed:<name>:
    <token> (<why>)``, where the token is the layout's refusal, or
    ``no_envelope``, or a refused envelope's own (core §7.5); never
    sniffed. ``time`` fields under ``epoch``, 0 where none is given."""
    from . import keys, packed, zk2c

    parsed = keys.parse(key)
    read = None
    if payload is not None or delete:
        read = zk2c.read_delete(key, encoding, attachment) if delete else zk2c.read_put(key, encoding, payload)
    stamp = None if read is None or read.stamp is None else {
        "run": read.stamp.run_hex(), "seq": read.stamp.seq, "shown": str(read.stamp)}
    found = held.resource(parsed) if held is not None and parsed and parsed.get("form") == "data" else None
    declared = decoding.unheld(key) if found is None else _held_type(*found)
    sniffed = decoded = None
    undecodable = None if read is None or not read.refused else read.undecodable()
    rec = None
    if found is not None and found[1]["type"]["kind"] == "packed":
        rec = found[0].schemas.packed_record(found[1]["type"])
    if rec is not None and not delete and payload is not None and read is not None:
        got = packed.read_sample(rec, read, epoch)
        if got.undecodable:
            # Every refusal under a packed type is named as that type's: its
            # layout's, no envelope, and a refused envelope's own token
            # (``read_sample`` hands that token on), so a row always says
            # ``undecodable as packed:<name>: <token> (<why>)``.
            undecodable = got.says(rec.name)
        else:
            decoded = got.value
    elif not delete and payload is not None and not (read is not None and read.refused):
        sniffed = decoding.sniff(read.record if read is not None and read.record is not None else payload, encoding)
    row = {"key": key, "zk2": parsed is not None,
           "kind": parsed.get("token") if parsed and parsed.get("form") == "data" else None,
           "delete": delete, "bytes": size, "encoding": encoding, "declared_type": declared,
           "envelope": stamp, "undecodable": undecodable, "sniffed": sniffed}
    if held is not None:
        row["decoded"] = decoded
    return row


#: What the report's notes say of every payload (observer §7.3, 0.12).
SNIFFED = ("this tool holds no contract, so every payload is sniffed, never decoded by its declared type: each row's "
           "declared_type says which case it is and where the tool stopped resolving the key (observer §7.3)")


#: What the report's notes say with ``--contracts`` (observer §7.3, 0.16).
HELD = ("this tool holds the contracts given: a packed type's record is decoded through its layout, under its "
        "envelope's tag, its time fields under the deployment epoch (0 where none is stated); every other payload "
        "is sniffed, each row's declared_type saying why (observer §7.3)")


class StreamCut(Exception):
    """Standard output refused a write: what the reader got is truncated."""


#: What standard error says when the stream is cut.
CUT = "echo: the stream was cut"


class Rows:
    """The rows a window hears, handed from zenoh's callback to the thread
    that writes them. :meth:`offer`, the callback's half, keeps the row
    under a lock and queues it, and never writes; :meth:`pump`, the main
    thread's, hands each queued row on in arrival order. ``count``, when
    set, keeps the first ``count`` rows and drops the rest."""

    def __init__(self, count: int = 0) -> None:
        self.count = count
        self.got: list[dict[str, Any]] = []
        self.queue: "queue.SimpleQueue[dict[str, Any]]" = queue.SimpleQueue()
        self.lock = threading.Lock()
        self.done = threading.Event()

    def offer(self, row: dict[str, Any]) -> None:
        with self.lock:
            if self.count and len(self.got) >= self.count:
                return
            self.got.append(row)
            self.queue.put(row)
            if self.count and len(self.got) >= self.count:
                self.done.set()

    def pump(self, each: Callable[[dict[str, Any]], None] | None, until: float) -> None:
        """Hand each row to ``each`` on the caller's thread until the
        monotonic instant ``until``, or until the count is reached and every
        row has been handed on. An exception ``each`` raises ends it."""
        while True:
            left = until - time.monotonic()
            if left <= 0 or (self.done.is_set() and self.queue.empty()):
                return
            try:
                row = self.queue.get(timeout=min(left, 0.05))
            except queue.Empty:
                continue
            if each is not None:
                each(row)

    def rest(self, each: Callable[[dict[str, Any]], None] | None) -> None:
        """Hand on every row still queued, the window over."""
        while True:
            try:
                row = self.queue.get_nowait()
            except queue.Empty:
                return
            if each is not None:
                each(row)

    def rows(self) -> list[dict[str, Any]]:
        with self.lock:
            return list(self.got)


def writer(out: TextIO) -> Callable[[dict[str, Any]], None]:
    """One NDJSON line per row on ``out``, flushed at once; a write that
    fails raises :class:`StreamCut`."""

    def line(row: dict[str, Any]) -> None:
        try:
            out.write(json.dumps(row) + "\n")
            out.flush()
        except OSError as e:
            raise StreamCut(str(e) or type(e).__name__) from e

    return line


def watch(session: Any, selector: str | list[str], window_s: float, count: int = 0,
          listening: Any = None, each: Any = None, held: Held | None = None,
          epoch: int | None = None) -> list[dict[str, Any]]:
    """Every sample the selector delivers within the window, or until
    ``count`` of them, in arrival order; ``listening`` is called once the
    subscriber is declared, the window's start, and ``each`` with every
    row in arrival order, on the caller's thread, never in zenoh's
    callback (:class:`Rows`). What ``each`` raises ends the window and
    reaches the caller. A list of selectors, observer §7.5's scopes, is a
    subscriber each."""
    import zenoh

    rows = Rows(count)

    def received(s: Any) -> None:
        payload = s.payload.to_bytes()
        att = None if s.attachment is None else s.attachment.to_bytes()
        rows.offer(row_of(str(s.key_expr), s.kind == zenoh.SampleKind.DELETE, len(payload), str(s.encoding),
                          payload, att, held, epoch))

    subs = [session.declare_subscriber(sel, zenoh.handlers.Callback(received))
            for sel in ([selector] if isinstance(selector, str) else selector)]
    if listening is not None:
        listening()
    try:
        rows.pump(each, time.monotonic() + window_s)
    finally:
        for sub in subs:
            sub.undeclare()
    rows.rest(each)
    return rows.rows()


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="python -m zk2py.echo_tool", description=__doc__.split("\n")[0])
    ap.add_argument("selector", nargs="?", default=None)
    ap.add_argument("-c", "--connect", required=True)
    ap.add_argument("--for", dest="window", type=float, default=5.0)
    ap.add_argument("--count", type=int, default=0)
    ap.add_argument("--format", choices=("json", "ndjson"), default="json")
    ap.add_argument("--contracts", action="append", default=[],
                    help="a contract file the tool holds, one revision per interface (repeatable)")
    ap.add_argument("--epoch", type=int, default=None,
                    help="the deployment epoch, Unix seconds, for packed time fields (default 0, core §7.6)")
    ap.add_argument("--catalog", default=None,
                    help="the deployment's catalog, whose epoch packed time fields count from (core §8.6)")
    ap.add_argument("--across-face", dest="across_face", action="append", default=[],
                    help="an address across a constrained face, subscribed by the scopes of the contracts held "
                         "(observer §7.5; repeatable, needs --contracts, not with a SELECTOR)")
    args = ap.parse_args(argv)
    if args.across_face and (args.selector is not None or not args.contracts):
        print("echo: --across-face needs --contracts and no SELECTOR: its scopes are the contracts' (observer §7.5)",
              file=sys.stderr)
        return 2
    if args.catalog is not None:
        from pathlib import Path

        from . import catalog

        if args.epoch is not None:
            print("echo: --catalog states the epoch: not with --epoch (core §8.6)", file=sys.stderr)
            return 2
        try:
            held_catalog = catalog.read(Path(args.catalog).read_bytes())
        except OSError as e:
            print(f"echo: --catalog: {e}", file=sys.stderr)
            return 2
        except catalog.Refused as e:
            print(f"echo: --catalog: refused, {e.token}: {e} (core §8.6)", file=sys.stderr)
            return 2
        args.epoch = catalog.resolve([held_catalog], held_catalog.id).epoch
    selector = args.selector if args.selector is not None else "zk2/**"
    held = None
    if args.contracts:
        from pathlib import Path

        from .contract import load_contract

        try:
            loaded = [load_contract(Path(p)) for p in args.contracts]
            bad = [f"{c.path}: {c.codes}" for c in loaded if not c.valid]
            if bad:
                raise ValueError("not a valid contract: " + "; ".join(bad))
            held = Held(loaded)
        except Exception as e:  # noqa: BLE001 - a contract it cannot hold is refused input
            print(f"echo: --contracts: {e}", file=sys.stderr)
            return 2
    watched: str | list[str] = selector
    if args.across_face:
        from . import face

        assert held is not None
        watched = face.scopes(held.by_iface.values(), args.across_face)
        scope = face_scope(watched, args.across_face)
    else:
        try:
            scope = scope_of(selector)
        except Exception as e:  # noqa: BLE001 - a selector zenoh refuses is no scope
            print(f"echo: {selector!r}: not a key expression: {e}", file=sys.stderr)
            return 2
    try:
        session = live.open_client(args.connect)
    except Exception as e:  # noqa: BLE001 - no session, no watch
        print(f"echo: the session did not open: {e}", file=sys.stderr)
        return 2
    print(scope_line(scope), file=sys.stderr, flush=True)
    # observer §7.4: "A tool SHOULD check the limit at start" (round 89).
    print(f"echo: {live.memlock_said()}", file=sys.stderr, flush=True)
    try:
        start = time.monotonic()
        samples = watch(session, watched, args.window, args.count,
                        lambda: print("echo: listening", file=sys.stderr, flush=True),
                        writer(sys.stdout) if args.format == "ndjson" else None, held, args.epoch)
        watched_s = round(time.monotonic() - start, 3)
        if args.format == "json":
            doc = {"report": "echo", "selector": scope["selector"], "scope": scope, "window_s": watched_s,
                   "samples": samples, "notes": [scope["says"], SNIFFED if held is None else HELD], "exit": 0}
            try:
                sys.stdout.write(json.dumps(doc, indent=2) + "\n")
                sys.stdout.flush()
            except OSError as e:
                raise StreamCut(str(e) or type(e).__name__) from e
    except StreamCut as e:
        print(f"{CUT}: {e}: what reached the reader is truncated", file=sys.stderr, flush=True)
        return 2
    finally:
        live.close(session)
    return 0


if __name__ == "__main__":
    code = main()
    try:
        sys.stdout.flush()
    except OSError as e:
        print(f"{CUT}: {e}: what reached the reader is truncated", file=sys.stderr, flush=True)
        code = code or 2
    import os

    os._exit(code)

"""A generic watcher: ``python -m zk2py.echo_tool -c ENDPOINT [--for SECS]
[--count N] [--format json|ndjson] [SELECTOR]`` (#609, rounds 83 and 84).

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
  ``null`` and ``zk2`` false;
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

from . import live

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


def scope_line(scope: dict[str, Any]) -> str:
    """What standard error says of a scope, in every format: what it
    watches and what it excludes (observer §8.2). In NDJSON it is the only
    medium that says it."""
    return f"echo: watching {', '.join(scope['watched']) or 'no kind'}; {scope['says']}"


def row_of(key: str, delete: bool, size: int, encoding: str) -> dict[str, Any]:
    """One sample's row, the same in the JSON document's ``samples`` and as
    an NDJSON line: its key, whether it is zk2, its kind token (``None``
    for a key that is not zk2, a fact and not an error, observer §8.1)."""
    from . import keys

    parsed = keys.parse(key)
    return {"key": key, "zk2": parsed is not None,
            "kind": parsed.get("token") if parsed and parsed.get("form") == "data" else None,
            "delete": delete, "bytes": size, "encoding": encoding}


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


def watch(session: Any, selector: str, window_s: float, count: int = 0,
          listening: Any = None, each: Any = None) -> list[dict[str, Any]]:
    """Every sample the selector delivers within the window, or until
    ``count`` of them, in arrival order; ``listening`` is called once the
    subscriber is declared, the window's start, and ``each`` with every
    row in arrival order, on the caller's thread, never in zenoh's
    callback (:class:`Rows`). What ``each`` raises ends the window and
    reaches the caller."""
    import zenoh

    rows = Rows(count)

    def received(s: Any) -> None:
        rows.offer(row_of(str(s.key_expr), s.kind == zenoh.SampleKind.DELETE, len(s.payload.to_bytes()),
                          str(s.encoding)))

    sub = session.declare_subscriber(selector, zenoh.handlers.Callback(received))
    if listening is not None:
        listening()
    try:
        rows.pump(each, time.monotonic() + window_s)
    finally:
        sub.undeclare()
    rows.rest(each)
    return rows.rows()


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="python -m zk2py.echo_tool", description=__doc__.split("\n")[0])
    ap.add_argument("selector", nargs="?", default="zk2/**")
    ap.add_argument("-c", "--connect", required=True)
    ap.add_argument("--for", dest="window", type=float, default=5.0)
    ap.add_argument("--count", type=int, default=0)
    ap.add_argument("--format", choices=("json", "ndjson"), default="json")
    args = ap.parse_args(argv)
    try:
        scope = scope_of(args.selector)
    except Exception as e:  # noqa: BLE001 - a selector zenoh refuses is no scope
        print(f"echo: {args.selector!r}: not a key expression: {e}", file=sys.stderr)
        return 2
    try:
        session = live.open_client(args.connect)
    except Exception as e:  # noqa: BLE001 - no session, no watch
        print(f"echo: the session did not open: {e}", file=sys.stderr)
        return 2
    print(scope_line(scope), file=sys.stderr, flush=True)
    try:
        start = time.monotonic()
        samples = watch(session, args.selector, args.window, args.count,
                        lambda: print("echo: listening", file=sys.stderr, flush=True),
                        writer(sys.stdout) if args.format == "ndjson" else None)
        watched_s = round(time.monotonic() - start, 3)
        if args.format == "json":
            doc = {"report": "echo", "selector": args.selector, "scope": scope, "window_s": watched_s,
                   "samples": samples, "notes": [scope["says"]], "exit": 0}
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

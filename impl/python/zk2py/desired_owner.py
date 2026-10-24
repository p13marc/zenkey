"""zk2py's desired.v1 owner as a process, driven as the Rust owner example
is (``owner --help``, its ``--desired-*`` options and ``desired …``
commands), so that ``zk2py.desired_scenarios`` runs either, or one of each,
as a black box::

    python -m zk2py.desired_owner --connect <endpoint> [--tokenless <iface>]...
        [--desired-author] [--desired-record <file>] [--desired-window <s>]
        [--desired-target <system>/<service>] [--desired-unbound]
        [--desired-archive <system>/<service>] [--desired-watch]
        [--desired-reread-ms <ms>] [--desired-hold]
        <system>/<service>

Standard output: ``connected <endpoint>``, then ``ready
zk2/<system>/<service>/@zk/instance/<id>``, then one line per command, in the
example's words. One more command, zk2py's own: ``desired queries``, the
state GETs the author's queryables received. A restart is the process's:
kill it, or close standard input (a clean close), and start it again.
"""

from __future__ import annotations

import argparse
import json
import os
import sys
from typing import Any

from . import desired
from .desired_live import Author, Target, canonical, read_document


def _list(xs: list[str]) -> str:
    return ",".join(xs) or "-"


def answer(line: str, author: Author | None, target: Target | None) -> str:
    words = line.split(" ", 2)
    if not words or words[0] != "desired":
        return f"refused {line}: unknown command"
    cmd = words[1] if len(words) > 1 else ""
    rest = words[2] if len(words) > 2 else ""
    a_cmds = {"put", "delete", "documents", "adoption", "queries", "gets"}
    t_cmds = {"report", "running", "fail-next", "refuse-next", "hold", "release", "reread", "acts", "seeds"}
    if cmd in a_cmds and author is None or cmd in t_cmds and target is None:
        return f"refused {line}: this owner is no {'author' if cmd in a_cmds else 'target'}"
    if cmd == "put":
        tgt, _, doc_text = rest.partition(" ")
        try:
            doc = read_document(doc_text)
        except desired.NotStrict as e:
            return f"refused {line}: not strict JSON: {e}"
        what, key, stamp = author.put(tgt, doc)  # type: ignore[union-attr]
        return f"desired put {key} {stamp}" if what == "put" else f"desired unchanged {key}"
    if cmd == "delete":
        what, key, stamp = author.delete(rest.strip())  # type: ignore[union-attr]
        return f"desired deleted {key} {stamp}" if what == "deleted" else f"desired none {key}"
    if cmd == "documents":
        return "desired documents " + canonical(  # type: ignore[union-attr]
            {t: e["document"] for t, e in author.documents.items()}).decode()
    if cmd == "adoption":
        ad = author.adoption  # type: ignore[union-attr]
        return (f"desired adoption found={ad['found']} reput={_list(ad['reput'])} redeleted={_list(ad['redeleted'])} "
                f"expired={_list(ad['expired'])} failed={_list(ad['failed'])}")
    if cmd == "new-epoch":
        # a re-mint in this session: the author's puts again what it keeps;
        # a target's keeps its acts and its report (§2.7, 0.3)
        if author is not None:
            return f"desired new-epoch {author.new_epoch()}"
        if target is not None:
            return f"desired new-epoch {target.new_epoch()}"
        return f"refused {line}: no author or target"
    if cmd == "queries":
        return f"desired queries {author.queries}"  # type: ignore[union-attr]
    if cmd == "gets":
        try:
            key, n = author.gets(rest.strip())  # type: ignore[union-attr]
        except ValueError as e:
            return f"refused {line}: {e}"
        return f"desired gets {key} {n}"
    if cmd == "seeds":
        # the owner example's answer (0.4): "desired seeds <n> answered=<n>
        # author=<present|absent|incomplete|not-read|not-made>
        # archive=<-|answered|nothing> [because <why>]", or "desired seeds 0
        # none" before the first
        ls = target.last_seed  # type: ignore[union-attr]
        if ls is None:
            return "desired seeds 0 none"
        why = f" because {ls['why']}" if ls["author"] == "not-made" and ls["why"] else ""
        return (f"desired seeds {target.seeds} answered={ls['answered']} author={ls['author']} "  # type: ignore[union-attr]
                f"archive={ls['archive']}{why}")
    assert target is not None
    if cmd == "report":
        return "desired report " + json.dumps(target.report(), sort_keys=True, separators=(",", ":"))
    if cmd == "running":
        return "desired running " + canonical(target.running_doc).decode()
    if cmd == "fail-next":
        target.fail_next()
        return "desired fail-next"
    if cmd == "refuse-next":
        code, _, pointer = rest.partition(" ")
        if not code:
            return f"refused {line}: usage: desired refuse-next <code> [<pointer>]"
        target.refuse_next(code, pointer or None)
        return f"desired refuse-next {code}"
    if cmd == "hold":
        target.hold()
        return "desired hold"
    if cmd == "release":
        target.release()
        return "desired release"
    if cmd == "reread":
        arg = rest.strip()
        if arg == "off":
            target.reread(None)
            return "desired reread off"
        try:
            target.reread(int(arg))
        except ValueError:
            return f"refused {line}: usage: desired reread <ms>|off"
        return f"desired reread {arg}"
    if cmd == "acts":
        return f"desired acts {target.acts} seeds={target.seeds} held={'true' if target.held else 'false'}"
    return f"refused {line}: unknown command"


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="python -m zk2py.desired_owner", description=__doc__.split("\n")[0])
    ap.add_argument("--connect", required=True)
    ap.add_argument("--tokenless", action="append", default=[])
    ap.add_argument("--desired-author", action="store_true")
    ap.add_argument("--desired-record")
    ap.add_argument("--desired-window", type=float, default=60.0)
    ap.add_argument("--desired-target")
    ap.add_argument("--desired-unbound", action="store_true")
    ap.add_argument("--desired-archive")
    ap.add_argument("--desired-watch", action="store_true")
    ap.add_argument("--desired-reread-ms", type=int)
    ap.add_argument("--desired-hold", action="store_true")
    ap.add_argument("address")
    args = ap.parse_args(argv)
    print(f"connected {args.connect}", flush=True)
    author: Author | None = None
    target: Target | None = None
    instance = None
    if args.desired_author:
        author = Author(args.address, args.connect, record=args.desired_record, window_s=args.desired_window,
                        tokenless=tuple(args.tokenless))
        instance = author.start()
    if args.desired_target or args.desired_unbound:
        target = Target(args.address, args.connect, author=args.desired_target or "lab/author",
                        unbound=args.desired_unbound, archive=args.desired_archive, watch=args.desired_watch,
                        reread_ms=args.desired_reread_ms, hold=args.desired_hold)
        instance = target.start()
    print(f"ready {instance}", flush=True)
    for raw in sys.stdin:
        line = raw.rstrip("\n")
        if not line:
            continue
        try:
            out: Any = answer(line, author, target)
        except Exception as e:  # noqa: BLE001 - a command that raises is refused, never fatal
            out = f"refused {line}: {e!r}"
        print(out, flush=True)
    for x in (target, author):
        if x is not None:
            x.close()
    sys.stdout.flush()
    os._exit(0)


if __name__ == "__main__":
    sys.exit(main())

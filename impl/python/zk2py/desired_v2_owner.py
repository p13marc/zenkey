"""zk2py's desired.v2 author or target as a process, driven as
``zk2py.desired_owner`` is, so that ``zk2py.desired_v2_scenarios`` runs two
zk2py processes against each other (#609, round 118)::

    python -m zk2py.desired_v2_owner --connect <endpoint> [--tokenless <iface>]...
        [--desired-author] [--desired-record <file>] [--desired-run-record <file>]
        [--desired-window <s>] [--desired-watched]
        [--desired-target <system>/<service>] [--desired-unbound]
        [--desired-presence-watch] [--desired-reread-ms <ms>] [--desired-hold]
        [--desired-across] [--desired-reply-timeout-ms <ms>] [--desired-watch] [--desired-no-move]
        [--desired-catalog <file>] [--desired-owner-heartbeat] [--desired-contract <file>]
        [--desired-archive <system>/<service>] [--desired-worker <file>] [--desired-word-alone]
        [--desired-after-descriptor] [--desired-descriptor-late-ms <ms>] [--desired-hold-s <s>]
        <system>/<service>

Standard output: ``connected <endpoint>``, then ``ready
zk2/<system>/<service>/@zk/instance/<id>``, then one line per command. A
refusal at the start is ``not started: <why>`` on standard error, exit 1.

The reference's runtime does not speak ``desired.v2`` at a6aab761, so these
words are zk2py's own, after ``desired.v1``'s: ``desired gets <target>``
answers each GET of the slice with "the held stamp it carried, or none, and
whether it answered with that stamp alone", and ``desired watch`` "what its
watch expects" (scenarios-v2, "What a section observes of a read").
"""

from __future__ import annotations

import argparse
import json
import os
import sys
from typing import Any

from . import desired
from .desired_v2_live import Author, Target, canonical, contract_file


def _dump(x: Any) -> str:
    return json.dumps(x, sort_keys=True, separators=(",", ":"))


def answer(line: str, author: Author | None, target: Target | None) -> str:
    words = line.split(" ", 2)
    if not words or words[0] != "desired":
        return f"refused {line}: unknown command"
    cmd = words[1] if len(words) > 1 else ""
    rest = words[2] if len(words) > 2 else ""
    a_cmds = {"put", "delete", "delete!", "documents", "adoption", "gets", "run"}
    t_cmds = {"report", "running", "fail-next", "hold", "release", "reread", "acts", "watch", "reads", "seeds",
              "holds"}
    if cmd not in a_cmds | t_cmds | {"new-epoch"}:
        # refused before any role is asked for: an author-only process holds
        # no target, and an unknown word is no target's command
        return f"refused {line}: unknown command"
    if cmd in a_cmds and author is None or cmd in t_cmds and target is None:
        return f"refused {line}: this owner is no {'author' if cmd in a_cmds else 'target'}"
    if cmd == "put":
        tgt, _, doc_text = rest.partition(" ")
        try:
            doc = desired.strict_json(doc_text.encode("utf-8", "surrogatepass"), exact=True)
        except desired.NotStrict as e:
            return f"refused {line}: not strict JSON: {e}"
        what, key, version = author.put(tgt, doc)  # type: ignore[union-attr]
        return f"desired put {key} {version}" if what == "put" else f"desired unchanged {key}"
    if cmd in ("delete", "delete!"):
        what, key, version = author.delete(rest.strip(), force=cmd == "delete!")  # type: ignore[union-attr]
        return f"desired deleted {key} {version}" if what == "deleted" else f"desired none {key}"
    if cmd == "documents":
        return "desired documents " + canonical(  # type: ignore[union-attr]
            {t: e["document"] for t, e in author.documents.items()}).decode()
    if cmd == "adoption":
        return "desired adoption " + _dump(author.adoption)  # type: ignore[union-attr]
    if cmd == "run":
        return f"desired run {author.owner.run_hex()}"  # type: ignore[union-attr]
    if cmd == "new-epoch":
        if author is not None:
            return f"desired new-epoch {author.new_epoch()}"
        return f"refused {line}: no author"
    if cmd == "gets":
        return "desired gets " + _dump(author.gets(rest.strip()))  # type: ignore[union-attr]
    assert target is not None
    if cmd == "report":
        return "desired report " + _dump(target.report())
    if cmd == "running":
        return "desired running " + canonical(target.running_doc).decode()
    if cmd == "fail-next":
        target.fail_next()
        return "desired fail-next"
    if cmd == "hold":
        target.hold()
        return "desired hold"
    if cmd == "release":
        target.release()
        return "desired release"
    if cmd == "reread":
        arg = rest.strip()
        try:
            target.reread(None if arg == "off" else int(arg))
        except ValueError as e:
            return f"refused {line}: {e}"
        return f"desired reread {arg}"
    if cmd == "acts":
        return f"desired acts {target.acts} reads={target.reads} held={'true' if target.held else 'false'}"
    if cmd == "seeds":
        # the owner example's words (its --help, desired.v1's): "desired seeds
        # <n> answered=<n> author=<...> archive=<-|answered|nothing> [because
        # <why>]", "desired seeds 0 none before the first"
        ls = target.last_seed
        if ls is None:
            return "desired seeds 0 none"
        # v2.md §2.18 (0.2): the author's revision its descriptors name, in
        # the reference's words, before ``because``
        why = f" because {ls['why']}" if ls.get("why") else ""
        return (f"desired seeds {target.seeds} answered={ls['answered']} author={ls['author']} "
                f"archive={ls['archive']} revision={ls.get('revision', 'not-read')}{why}")
    if cmd in ("watch", "reads"):
        return f"desired {cmd} " + _dump(target.watch_says())
    if cmd == "holds":
        # zk2py's own word (round 127's mend): what each read of a new run's
        # descriptors left under §2.18's "None states the run yet", in order:
        # [run, held|again|word|released, the appearances counted or the word];
        # "The hold is bounded" (review-bot on #1176): [run, bound, hold_s] where
        # the bound left the word, [run, late, what it drew] for a read it overtook
        with target.lock:
            return "desired holds " + _dump([list(x) for x in target.hold_log])
    return f"refused {line}: unknown command"


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="python -m zk2py.desired_v2_owner", description=__doc__.split("\n")[0])
    ap.add_argument("--connect", required=True)
    ap.add_argument("--tokenless", action="append", default=[])
    ap.add_argument("--desired-author", action="store_true")
    ap.add_argument("--desired-record")
    ap.add_argument("--desired-run-record")
    ap.add_argument("--desired-window", type=float, default=60.0)
    ap.add_argument("--desired-watched", action="store_true", help="tasking.v2, watched: freshness.ttl_s = 4")
    ap.add_argument("--desired-target")
    ap.add_argument("--desired-unbound", action="store_true")
    ap.add_argument("--desired-presence-watch", action="store_true")
    ap.add_argument("--desired-reread-ms", type=int)
    ap.add_argument("--desired-hold", action="store_true")
    ap.add_argument("--desired-across", action="store_true")
    ap.add_argument("--desired-reply-timeout-ms", type=int)
    ap.add_argument("--desired-watch", action="store_true", help="watch the author's heartbeat (§2.5)")
    ap.add_argument("--desired-no-move", action="store_true", help="do not move the digest it expects at its own change")
    ap.add_argument("--desired-contract", help="the author's interface in a revision of the caller's own, by its "
                                               "file: the author's contract, and what a target is told its author uses")
    ap.add_argument("--desired-worker", help="the target's contract, worker.v2 in a revision of the caller's own, by "
                                             "its file (scenarios-v2 §11's worker.v2, both)")
    ap.add_argument("--desired-archive", help="<system>/<service>: the target's last-known, read when a seed draws "
                                              "nothing and a complete presence read shows its author absent (v1 §2.9)")
    ap.add_argument("--desired-word-alone", action="store_true",
                    help="the target reads its author's revision by the deployment's word alone, never its "
                         "descriptor: a target built before desired.v2 (v2.md §2.18, scenarios-v2.md §16)")
    ap.add_argument("--desired-catalog", help="core §8.6's catalog: the author states its id; the target reads groups by it")
    ap.add_argument("--desired-owner-heartbeat", action="store_true",
                    help="the author confirms its interface by owner heartbeats (freshness.v2 §2.4); needs a catalog")
    ap.add_argument("--desired-after-descriptor", action="store_true",
                    help="the author's new instance puts its documents again after its descriptor (core §8.2 step "
                         "3) and before its tokens, where by default it puts them before step 3 (desired.v2 §2.18, "
                         "0.3, its why; round 127)")
    ap.add_argument("--desired-descriptor-late-ms", type=int, default=0,
                    help="the author's new instance waits this long after its puts again before its descriptor "
                         "(core §8.2 step 3), so a target's first read finds none stating its run (desired.v2 "
                         "§2.18, 0.3's mend, \"None states the run yet\"; round 127's mend)")
    ap.add_argument("--desired-hold-s", type=float, default=10.0,
                    help="the deployment's hold_s: a run that no read has stated within it of its first version "
                         "leaves the deployment's word, as across a face (desired.v2 §2.18, \"The hold is "
                         "bounded\"; review-bot on #1176); 10 s by default")
    ap.add_argument("address")
    args = ap.parse_args(argv)
    print(f"connected {args.connect}", flush=True)
    from .owner import OwnerRefused

    author: Author | None = None
    target: Target | None = None
    instance = None
    tasking = contract_file(args.desired_contract) if args.desired_contract else None
    try:
        if args.desired_author:
            author = Author(args.address, args.connect, record=args.desired_record,
                            run_record=args.desired_run_record, window_s=args.desired_window,
                            watched=args.desired_watched, tokenless=tuple(args.tokenless),
                            catalog=args.desired_catalog, owner_heartbeat=args.desired_owner_heartbeat,
                            tasking=tasking, after_descriptor=args.desired_after_descriptor,
                            descriptor_late_s=args.desired_descriptor_late_ms / 1000)
            instance = author.start()
        if args.desired_target or args.desired_unbound:
            target = Target(args.address, args.connect, author=args.desired_target or "lab/author",
                            unbound=args.desired_unbound, presence_watch=args.desired_presence_watch,
                            reread_ms=args.desired_reread_ms, hold=args.desired_hold, across=args.desired_across,
                            reply_timeout_ms=args.desired_reply_timeout_ms, watch=args.desired_watch,
                            moves=not args.desired_no_move, watched=args.desired_watched,
                            run_record=args.desired_run_record, catalog=args.desired_catalog, tasking=tasking,
                            archive=args.desired_archive, word_alone=args.desired_word_alone,
                            hold_s=args.desired_hold_s,
                            worker=contract_file(args.desired_worker) if args.desired_worker else None)
            instance = target.start()
    except OwnerRefused as e:
        print(f"not started: {e}", file=sys.stderr, flush=True)
        sys.stdout.flush()
        os._exit(1)
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

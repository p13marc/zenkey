"""``timing.v1``'s tool on a bus (§2.7, §5; #609, round 79): ``python -m
zk2py.timing_tool -c ENDPOINT [--for SECS] [--clock-bound MS]
[--contracts PATH]... [--across-face [--owner-zid ZID]...] SELECTOR``.

Written from ``spec/profiles/timing/v1.md`` (text 0.3, its roll-up text
0.4's, its held time and a roll-up's reason text 0.5's) §2.7 and §5, as a
tool's implementer reads them. ``SELECTOR`` names one owner's members,
``zk2/<system>/<service>/**`` or narrower; for each timed stream it reaches
(a ``stream`` or ``@stream`` resource with a ``timing.*`` annotation):

1. the owner's presence and each instance's descriptor (core §3.3), and the
   revision each names, held offline (``--contracts``) or retrieved (core
   §8.4);
2. a window (:class:`zk2py.timing_live.Window`) of ``--for`` seconds: its
   presence followed and every descriptor held first, then its
   subscriptions, the window's start (F-263), which standard error's
   ``timing: listening`` line marks; no binding, so a member is known only
   when its template has no parameter (text 0.3); a stall of its receive
   path is every member's loss (text 0.3); ε is ``--clock-bound``, never
   the HLC's delta;
3. §5's three questions of each member it held, and each read over its
   resource and over the run by its finding (F-264), a question whose
   every member is set aside not asked, or not this profile's (text 0.4,
   F-277), its reason and ``first`` the first member's giving its answer in
   byte order of the keys, none over no member (text 0.5, F-279). A
   member's ``held_ms`` is its longest unbroken stretch, never the sum: a
   known member never heard held only while waited for (F-278), a member
   that ended held, and its ``span_ms`` read, over its first life (F-280).

Across a constrained face (``--across-face``) no presence and no descriptor
cross: the contracts are ``--contracts``, the deployment's word, and each
``--owner-zid`` is a ``meta.zid`` on its word (§2.8, text 0.3); without
one every lifespan is ``unattributed``.

It prints one JSON document (``report``, ``rows``, ``run``, ``exit``), and
exits 1 on a finding over the run (a deadline missed with its owner
present and nothing lost, a period not kept likewise, a sample expired), 0
when every question asked over the run, at least one, is answered on its
other pole (a question not asked, or not this profile's, set aside: text
0.4), 2 otherwise (no member held, a question unobservable, none asked).
"""

from __future__ import annotations

import argparse
import json
import sys
import time
from typing import Any

from . import live
from . import timing as tm


def timed_streams(canonical: dict[str, Any], address: str) -> list[tuple[str, str]]:
    """Every timed stream a contract in hand declares at ``address``: a
    stream with a ``timing.*`` key among its resolved annotations (§0's
    timed resource, the invalid ones included, which §5 reads
    ``invalid``), plain or explicit, as ``(rid, key expression)``, the key
    expression its keys' at the address (core §1.1's data form, its
    template's parameters wildcards). Read from the contract alone: no
    selector decides what is listed."""
    from .timing_live import _template_expr

    out = []
    for r in canonical.get("resources") or []:
        if tm.kind_of(r["kind"]) != "stream" or not any(k.startswith("timing.") for k in r.get("annotations") or {}):
            continue
        out.append((f"{r['token']}/{r['template']}",
                    f"zk2/{address}/{canonical['interface']}/{r['token']}/{_template_expr(r['template'])}"))
    return out


def timed_rids(canonical: dict[str, Any], selector: str, address: str) -> list[str]:
    """The timed streams of a contract the selector reaches: of
    :func:`timed_streams`, those whose keys' expression the selector
    intersects, by zenoh's own intersection and nothing else."""
    import zenoh

    sel = zenoh.KeyExpr(selector)
    return [rid for rid, expr in timed_streams(canonical, address) if sel.intersects(zenoh.KeyExpr(expr))]


def unreached_rids(canonical: dict[str, Any], selector: str, address: str) -> list[str]:
    """The timed streams of a contract the selector does not reach, in byte
    order: every one :func:`timed_streams` lists at the address, less
    :func:`timed_rids`'s. An explicit ``@stream`` under a selector ending
    ``**`` is one (observer §8.2, "`*` and `**` never match a verbatim
    chunk"), and so is every stream when the selector names another
    address."""
    reached = set(timed_rids(canonical, selector, address))
    return sorted({rid for rid, _ in timed_streams(canonical, address) if rid not in reached}, key=str.encode)


#: Observer §8.3: "A tool says whose clock a time, an age or a latency is
#: on." The report's times, and a lifespan's age.
CLOCKS = {
    "window": "zk2py's monotonic clock: at_ms, due_ms, held_ms and span_ms count from the window's start, "
              "listening counted from its last subscription (timing.v1 §2.5, §2.7)",
    "lifespan": "a sample's age is this host's wall clock at its arrival less its stamp's time, the stamp the "
                "owner's when its id is the owner's meta.zid by value, else foreign (a router's, or a reader's "
                "own session's), or none (timing.v1 §2.6; observer §4.4, §8.3)",
}


def _rolled(question: str, members: list[tuple[str, tm.Answer]]) -> dict[str, Any]:
    """§5's roll-up of one question over a resource's or the run's members
    (text 0.4), its reason and ``first`` the first member's giving its
    answer in byte order of the keys, and over no member neither (text 0.5,
    F-279: "Over no member there is none")."""
    a = tm.rollup(question, members)
    out: dict[str, Any] = {"answer": a.answer}
    if a.reason is not None:
        out["reason"] = a.reason
        out["first"] = next(k for k, x in sorted(members, key=lambda kv: kv[0].encode())
                            if x.answer == a.answer)
    out["members"] = len(members)
    return out


def _address(selector: str) -> str:
    parts = selector.split("/")
    if len(parts) < 3 or parts[0] != "zk2" or "*" in parts[1] or "*" in parts[2]:
        raise SystemExit(f"timing: {selector!r}: zk2py's tool reads one owner, zk2/<system>/<service>/…")
    return f"{parts[1]}/{parts[2]}"


def run(session: Any, watch: Any, selector: str, offline: list[Any], *, window_s: float, clock_bound_ms: int | None,
        across_face: bool = False, owner_zids: tuple[str, ...] = (), timeout: float = 1.0,
        listening: Any = None) -> dict[str, Any]:
    from .arbitration_tool import Store
    from .timing_live import Window

    address = _address(selector)
    store = Store(None if across_face else session, offline, timeout)
    contracts: list[dict[str, Any]] = []
    if across_face:
        contracts = [c.canonical for c in offline]
    else:
        from .arbitration_tool import _descriptor

        p = live.list_presence(session, f"zk2/{address}/@zk/instance/*", timeout)
        for inst in sorted({x["instance"] for x in p.instances}):
            doc = _descriptor(session, address, inst, timeout)
            for e in (doc or {}).get("interfaces") or []:
                canon = store.get(e.get("iface"), e.get("contract")) if isinstance(e, dict) else None
                if canon is not None and canon not in contracts:
                    contracts.append(canon)
    plans = [(c, rids) for c in contracts if (rids := timed_rids(c, selector, address))]
    # observer §8.2: a wildcard scope names what it cannot reach, the
    # verbatim kinds and the timed streams of the revisions held it misses
    unreached = sorted({f"{c['interface']} {r}" for c in contracts for r in unreached_rids(c, selector, address)},
                       key=str.encode)
    scope = {"selector": selector, "excluded": live.blind_spots(selector), "unreached": unreached}
    windows = [Window(session, watch, address, c, rids, epsilon_ms=clock_bound_ms, across_face=across_face,
                      owner_zids=owner_zids) for c, rids in plans]
    if listening is not None:
        listening(windows)
    starts = [w.start_ns for w in windows if w.start_ns is not None]
    if starts:
        end = max(starts) + int(window_s * 1_000_000_000)
        while time.monotonic_ns() < end:
            time.sleep(min(0.05, max(0.0, (end - time.monotonic_ns()) / 1e9)))
    rows: list[dict[str, Any]] = []
    members: dict[str, list[tuple[str, tm.Answer]]] = {"deadline": [], "period": [], "lifespan": []}
    for (canon, rids), w in zip(plans, windows):
        got = w.end()
        rd = w.reader
        w.close()
        for rid in rids:
            res = rd.resources[rid]
            mine = {m: a for m, a in got.items() if m in set(res.first_held) | set(res.known)
                    | (set(res.tracker.held_in_window()) if res.tracker is not None else set())}
            rows.append({"row": "resource", "address": address, "iface": canon["interface"], "resource": rid,
                         "declaration": res.decl.expect(), "members": len(mine),
                         "questions": {q: _rolled(q, [(m, a[q]) for m, a in mine.items()])
                                       for q in ("deadline", "period", "lifespan")}})
            start, end = w.start_ns or 0, w.end_ns or 0
            for m, a in sorted(mine.items()):
                # ``mine`` is drawn from what ``end`` answered, whose keys are
                # its readings' and spans' (one member set, asserted there):
                # indexed, so a member missing from either fails, never a
                # silent None (review-bot on PR #941)
                r = w.readings[m]
                arrivals = sum(1 for x in rd.arrivals if x.key == m and x.handed and start <= x.mono_ns <= end)
                reps = [x for rid_, x in rd.reports if rid_ == rid and x.member == m and start <= x.at_ns <= end]
                misses = [{"at_ms": tm.ns_to_ms(x.at_ns - start), "due_ms": tm.ns_to_ms((x.due_ns or 0) - start),
                           "owner": x.owner} for x in reps if x.event == "missed"]
                # the member's first end (text 0.5, F-280: read over its first life)
                ended = next(({"at_ms": tm.ns_to_ms(x.at_ns - start), "reason": x.reason}
                              for x in reps if x.event == "ended"), None)
                rows.append({"row": "member", "key": m, "address": address, "iface": canon["interface"],
                             "resource": rid, "known": m in res.known, "arrivals": arrivals,
                             "held_ms": tm.ns_to_ms(r.deadline.held_ns),
                             # None only where the resource declares no period
                             "span_ms": None if (span := w.spans[m]) is None else tm.ns_to_ms(span),
                             "missed_present": r.deadline.missed_present,
                             "missed_not_present": r.deadline.missed_not_present,
                             "misses": misses, "ended": ended,
                             **{q: a[q].pair() for q in ("deadline", "period", "lifespan")}})
                for q in members:
                    members[q].append((m, a[q]))
    runq = {q: _rolled(q, members[q]) for q in members}
    # §5 over the run, read by each question's finding (F-264). A question
    # whose every member is set aside rolls up not asked, or not this
    # profile's (text 0.4, F-277): nobody put it, so it is set aside here
    # too, and the run is clean when every question asked is on its other
    # pole, at least one asked. With none asked there is no verdict.
    finding = any(runq[q]["answer"] == tm.FINDING[q] for q in runq)
    asked = [q for q in runq if runq[q]["answer"] not in tm.SET_ASIDE]
    clean = bool(asked) and all(runq[q]["answer"] == (tm.NO if tm.FINDING[q] == tm.YES else tm.YES) for q in asked)
    # No member held, or no timed resource in scope, is no verdict: the
    # empty roll-up is unobservable (conclusions.json #33, "no member at
    # all"), so a run with no member row can never exit 0, checked here by
    # itself (review-bot on PR #919)
    held = any(r["row"] == "member" for r in rows)
    code = 2 if not held else 1 if finding else 0 if clean else 2
    return {"report": "timing", "selector": selector, "across_face": across_face, "clock_bound_ms": clock_bound_ms,
            "window_s": window_s, "scope": scope, "clocks": CLOCKS, "rows": rows, "run": runq, "exit": code}


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="python -m zk2py.timing_tool", description=__doc__.split("\n")[0])
    ap.add_argument("selector")
    ap.add_argument("-c", "--connect", required=True)
    ap.add_argument("--for", dest="window", type=float, default=10.0)
    ap.add_argument("--clock-bound", type=int, default=None)
    ap.add_argument("--contracts", action="append", default=[])
    ap.add_argument("--across-face", action="store_true")
    ap.add_argument("--owner-zid", action="append", default=[])
    ap.add_argument("--timeout", type=float, default=1.0)
    args = ap.parse_args(argv)
    from .arbitration_tool import _load

    if args.owner_zid and not args.across_face:
        print("timing: --owner-zid is the deployment's word across a face (§2.8): give --across-face",
              file=sys.stderr)
        return 2
    # observer §7.4: a tool checks the limit at start, and says what it admits
    print(f"timing: {live.memlock_said()}", file=sys.stderr, flush=True)
    offline = _load(args.contracts)
    session, watch = live.open_client(args.connect), live.open_client(args.connect)

    def listening(windows: list[Any]) -> None:
        print(f"timing: listening {args.window}s to {len(windows)} window(s)", file=sys.stderr, flush=True)

    try:
        doc = run(session, watch, args.selector, offline, window_s=args.window, clock_bound_ms=args.clock_bound,
                  across_face=args.across_face, owner_zids=tuple(args.owner_zid), timeout=args.timeout,
                  listening=listening)
    finally:
        live.close(session)
        live.close(watch)
    if doc["scope"]["unreached"]:
        print(f"timing: {args.selector} reaches no {', '.join(doc['scope']['excluded']) or 'verbatim kind'}: "
              f"not watched {', '.join(doc['scope']['unreached'])} (observer §8.2)", file=sys.stderr, flush=True)
    print(json.dumps(doc, indent=2))
    return doc["exit"]


if __name__ == "__main__":
    code = main()
    sys.stdout.flush()
    import os

    os._exit(code)

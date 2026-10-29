"""``arbitration.v1``'s tool on a bus (§2.14, §5; #609, round 79):
``python -m zk2py.arbitration_tool -c ENDPOINT [--contracts PATH]...
[--role R] [--for SECS] [SYSTEM/SERVICE]``.

Written from ``spec/profiles/arbitration/v1.md`` (text 0.3) §2.14, as a
tool's implementer reads it, on core 0.56's presence rules: every read on a
session that never held a liveliness subscriber, the view's watch on a
session of its own.

For one consumer (``SYSTEM/SERVICE``), each instance presence shows:
1. its descriptor (core §3.3), and each role its ``requires`` lists that
   carries ``arbitration.policy``, or whose declaring contract is not in
   hand, or the one ``--role`` names;
2. the declaring contract, the one ``declared_by`` names at the revision the
   descriptor lists for it, held offline (``--contracts``) or retrieved
   (core §8.4); not in hand, ``no_contract``; ``declared_by`` null, a
   manifest's role, not this profile's;
3. every revision of the required interface the tool holds: offline, and
   each one the bound providers' descriptors name, retrieved (core §8.4),
   and :func:`zk2py.arbitration.tool_declaration` over them (two that
   differ, ``revisions_disagree``);
4. its own view, :class:`zk2py.arbitration_live.ToolView`, for ``--for``
   seconds: the binding and the parameter bindings from the descriptor
   (``self.system`` and ``self.service`` the consumer's own), no ε, the
   interface governed when every revision held lists ``redundancy.v1``,
   listening counted from its subscriptions (text 0.3);
5. :func:`zk2py.arbitration.conclude`.

With ``--role`` and no consumer's descriptor listing it, each contract in
``--contracts`` declaring the role is read with no descriptor: the selection
and the binding not asked, ``no_descriptor`` (text 0.2, F-272).

It prints one JSON document on its standard output (``report``, ``rows``,
``exit``), and exits 1 on any question's finding (a declaration invalid,
the view in the dead-man, a wildcard or shadowed entry), 0 when every
question asked answers its other pole (``unbound`` informative), 2
otherwise: an answer unobservable, or nothing arbitrated in scope.
"""

from __future__ import annotations

import argparse
import json
import sys
import time
from pathlib import Path
from typing import Any

from . import arbitration as ab
from . import live


def _load(paths: list[str]) -> list[Any]:
    from .contract import load_contract

    out = []
    for p in paths:
        path = Path(p)
        files = sorted(path.rglob("*.toml")) if path.is_dir() else [path]
        for f in files:
            c = load_contract(f)
            if c.valid:
                out.append(c)
            else:
                print(f"arbitration: {f}: not a valid contract: {', '.join(c.codes)}", file=sys.stderr)
    return out


class Store:
    """The contracts in hand: offline by fingerprint, retrieved on demand
    (core §8.4), never retrieved when held offline."""

    def __init__(self, session: Any, offline: list[Any], timeout: float) -> None:
        self.session, self.timeout = session, timeout
        self.by_fp: dict[str, dict[str, Any]] = {c.fingerprint: c.canonical for c in offline}
        self.offline = list(offline)
        self.retrieved: list[str] = []

    def get(self, iface: str, fingerprint: str | None) -> dict[str, Any] | None:
        if fingerprint is None:
            return None
        if fingerprint in self.by_fp:
            return self.by_fp[fingerprint]
        if self.session is None:
            return None
        r = live.retrieve_bundle(self.session, iface, fingerprint, self.timeout)
        if r.verified is None:
            return None
        self.by_fp[fingerprint] = r.verified.contract
        self.retrieved.append(fingerprint)
        return r.verified.contract

    def revisions_of(self, iface: str) -> dict[str, dict[str, Any]]:
        return {fp: c for fp, c in self.by_fp.items() if c.get("interface") == iface}


def _descriptor(session: Any, address: str, instance: str, timeout: float) -> dict[str, Any] | None:
    for a in live.get_descriptor(session, f"zk2/{address}/@zk/instance/{instance}", timeout):
        if a.ok:
            try:
                doc = json.loads(a.payload)
            except ValueError:
                return None
            return doc if isinstance(doc, dict) else None
    return None


def _provider_revisions(session: Any, store: Store, iface: str, bindings: list[str], timeout: float) -> None:
    """§2.14: the revisions of the required interface "retrieved from the
    bound providers' descriptors (core §8.4)": each instance holding the
    interface's token at an entry, its descriptor's fingerprint for it."""
    for e in bindings:
        if not ab.entry_valid(e):
            continue
        p = live.list_presence(session, f"zk2/{e}/@zk/alive/{iface}/**", timeout)
        for x in p.alive:
            if x["iface"] != iface:
                continue
            doc = _descriptor(session, f"{x['system']}/{x['service']}", x["instance"], timeout)
            if doc is not None:
                store.get(iface, live.fingerprint_of(doc, iface))


def _view_doc(tv: Any, view: ab.View) -> dict[str, Any]:
    with tv.lock:
        counts = tv.arb.counts_line()
        return {"whose": "the tool's own view", "listened_ms": view.listened_ns / 1e6, "lost": view.lost,
                "active": view.active, "governed": tv.governed, "switches": [
                    {**x["switch"], "at_ms": x["at_ms"], "last_ms": x["last_ms"]} for x in tv.lines],
                "providers": [{"address": a, **c} for a, c in counts["providers"].items()],
                "discards": {k: counts[k] for k in ("wild", "unresolved", "unbound")},
                "reads": len(tv.reads), "round_trips": len(tv.round_trips)}


def _row(declared: Any, bindings: list[str] | None, view: ab.View | None) -> dict[str, Any]:
    got = ab.conclude(declared, bindings, view)
    out = {q: a.expect() for q, a in got.items()}
    # §2.14: "the selection is labelled the tool's own view in every medium"
    # (text 0.3's stated reference), §10's "says each selection is its own
    # view": every selection, the one not asked or unobservable included
    out["selection"]["whose"] = "the tool's own view"
    if isinstance(declared, ab.Declaration) and declared.valid:
        out["declaration"].update({"policy": declared.policy, "resource": declared.resource,
                                   "deadline_ms": declared.deadline_ms, "period_ms": declared.period_ms})
    return out


def read(session: Any, watch: Any, consumer: str | None, offline: list[Any], *, role: str | None = None,
         listen_s: float = 2.0, timeout: float = 1.0) -> list[dict[str, Any]]:
    """§2.14's reading of one consumer's arbitrated roles (module doc), or,
    with ``role`` and no descriptor listing it, the offline contracts that
    declare it. ``session`` never holds a liveliness subscriber; ``watch``
    is the view's watch session."""
    from .arbitration_live import ToolView

    store = Store(session, offline, timeout)
    rows: list[dict[str, Any]] = []
    listed = False
    if consumer is not None and session is not None:
        p = live.list_presence(session, f"zk2/{consumer}/@zk/instance/*", timeout)
        for inst in sorted({x["instance"] for x in p.instances}):
            doc = _descriptor(session, consumer, inst, timeout)
            if doc is None:
                continue
            for req in doc.get("requires") or []:
                if not isinstance(req, dict) or (role is not None and req.get("role") != role):
                    continue
                name = req.get("role")
                iface = req.get("interface")
                declared_by = req.get("declared_by")
                bindings = [b for b in (req.get("bindings") or []) if isinstance(b, str)]
                params = ab.resolve_params({k: v for k, v in (req.get("params") or {}).items()
                                            if isinstance(v, str)}, consumer)
                if declared_by is None:
                    declared: Any = ab.NOT_ARBITRATED  # a manifest's role: no tool sees its policy
                else:
                    canon = store.get(declared_by, live.fingerprint_of(doc, declared_by))
                    if canon is None or name not in (canon.get("requires") or {}):
                        declared = ab.NO_CONTRACT
                    else:
                        r = ab.role_of(canon, name)
                        if ab.POLICY not in r["annotations"] and role is None:
                            continue  # not this profile's, and not asked by name
                        _provider_revisions(session, store, iface, bindings, timeout)
                        revs = store.revisions_of(iface)
                        declared = ab.tool_declaration(r, [ab.provider_of(c) for c in revs.values()], params)
                listed = True
                view, vdoc = None, None
                if listen_s > 0 and isinstance(declared, ab.Declaration) and declared.valid and bindings:
                    revs = store.revisions_of(iface)
                    tv = ToolView(session, watch, iface, declared, bindings, params,
                                  governed=ab.view_governed(revs.values()))
                    try:
                        time.sleep(listen_s)
                        view = tv.view()
                        vdoc = _view_doc(tv, view)
                    finally:
                        tv.close()
                row = {"row": "role", "consumer": consumer, "instance": inst, "role": name, "iface": iface,
                       "declared_by": declared_by, "bindings": bindings, "params": params,
                       "revisions": sorted(store.revisions_of(iface)) if iface else [],
                       **_row(declared, bindings, view)}
                if vdoc is not None:
                    row["view"] = vdoc
                rows.append(row)
    if role is not None and not listed:
        # F-272: no consumer's descriptor lists it; each offline contract
        # declaring the role, read with no descriptor
        for c in offline:
            req = (c.canonical.get("requires") or {}).get(role)
            if req is None:
                continue
            iface = req.get("interface")
            revs = store.revisions_of(iface)
            declared = ab.tool_declaration(ab.role_of(c.canonical, role), [ab.provider_of(x) for x in revs.values()])
            rows.append({"row": "role", "role": role, "iface": iface, "declared_by": c.canonical.get("interface"),
                         "bindings": None, "revisions": sorted(revs), **_row(declared, None, None)})
    return rows


#: §5's "Finding on" per question: the declaration's no (invalid), the
#: selection's no only as dead_man, and the binding's yes. The binding's yes
#: has exactly two reasons in §5's table, wildcard and shadowed, both
#: warnings and both findings, so its answer alone decides it; the reasons
#: are named here so a third would not pass unseen.
BINDING_FINDINGS = ("wildcard", "shadowed")


def is_finding(row: dict[str, Any]) -> bool:
    """§5: a declaration invalid; the view in the dead-man ("No, dead_man,
    where the deployment expects a commander"); a wildcard or a shadowed
    entry. ``unbound`` is the selection's own no and "is informative", never
    a finding: §5 puts it under the selection, not the binding."""
    decl, sel, binding = row["declaration"], row["selection"], row["binding"]
    if binding["answer"] == "yes":
        assert binding["reason"] in BINDING_FINDINGS, f"§5 gives the binding's yes no reason {binding['reason']!r}"
    return (decl["answer"] == "no"
            or (sel["answer"] == "no" and sel["reason"] == "dead_man")
            or (binding["answer"] == "yes" and binding["reason"] in BINDING_FINDINGS))


def exit_code(rows: list[dict[str, Any]]) -> int:
    """1 on any finding, 0 when every question asked answers its other pole
    (the selection's ``unbound`` informative), 2 otherwise: an answer
    unobservable, or nothing arbitrated in scope."""
    if any(is_finding(r) for r in rows):
        return 1
    if any(r[q]["answer"] == "unobservable" for r in rows for q in ("declaration", "selection", "binding")):
        return 2
    if not any(r["declaration"]["answer"] in ("yes", "no") for r in rows):
        return 2  # nothing arbitrated in scope
    return 0


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="python -m zk2py.arbitration_tool", description=__doc__.split("\n")[0])
    ap.add_argument("consumer", nargs="?", help="one consumer, <system>/<service>")
    ap.add_argument("-c", "--connect", help="a router's endpoint; none reads offline only")
    ap.add_argument("--contracts", action="append", default=[], help="contracts held offline, repeatable")
    ap.add_argument("--role", help="only this role")
    ap.add_argument("--for", dest="listen", type=float, default=2.0, help="the view's listen, seconds")
    ap.add_argument("--timeout", type=float, default=1.0)
    args = ap.parse_args(argv)
    offline = _load(args.contracts)
    session = watch = None
    if args.connect and args.consumer:
        session, watch = live.open_client(args.connect), live.open_client(args.connect)
    try:
        rows = read(session, watch, args.consumer, offline, role=args.role, listen_s=args.listen,
                    timeout=args.timeout)
    finally:
        for s in (session, watch):
            if s is not None:
                live.close(s)
    code = exit_code(rows)
    print(json.dumps({"report": "arbitration", "target": args.consumer, "role": args.role, "listen_s": args.listen,
                      "rows": rows, "exit": code}, indent=2))
    return code


if __name__ == "__main__":
    code = main()
    sys.stdout.flush()
    import os

    os._exit(code)

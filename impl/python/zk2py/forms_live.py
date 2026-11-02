"""Core 0.92's call forms beside the wire every call still uses (#985 C4a;
#609, round 100), on an in-process zenoh-python router:
``python -m zk2py.forms_live [--out report.json]``.

No rule at this version has a caller or an owner write a form (core §5.1,
"The forms"; §5.2, "The error form"). This run measures two facts the
texts lean on, and what an owner does with what 0.92 newly lints clean:

1. **A router's own ``Timeout``** (core §5.2 "Transport errors", Appendix
   B: "read from zenoh 1.10.1's source, not measured"). A queryable holds
   every query unanswered; a client calls it through the router, its
   timeout 1 s and then 2.5 s. Each reply error's Encoding and payload are
   kept, and read through :func:`zk2py.zk2c.read_reply_err`: a router's
   ``Timeout`` under ``zenoh/bytes`` is ``sample_kind`` ("T is a value's
   CH"), the querier's own under ``zenoh/string`` is ``encoding``. zk2py's
   caller (:func:`zk2py.live.call`) reads either as the transport's.
2. **A packed operation type** (core §7.1 and §9.2's E044 narrowed): the
   owner example (a black box) and zk2py's owner, each serving
   ``spec/conformance/contracts/ok-packed-operation.toml``, called on
   ``calibrate`` (packed request, response and error), ``scan`` (a JSON
   request, a packed response and summary, no error type) and ``reset`` (a
   packed request, a JSON response). Each answer's kind and Encoding. §5.2's
   "Encoding" names a JSON Schema, a protobuf and a raw type, never a
   packed one (F-471).

Exit 0 when every check passes, 1 when any fails, 2 when it cannot run (the
owner example missing).
"""

from __future__ import annotations

import argparse
import json
import os
import queue
import sys
import time
from pathlib import Path
from typing import Any

REPO = Path(__file__).resolve().parents[3]
BLACKBOX = Path(os.environ.get("ZK2PY_BLACKBOX", "/srv/dev/scratch/zenkey/bin/current"))
PACKED_OP = REPO / "spec" / "conformance" / "contracts" / "ok-packed-operation.toml"
SLOW = "zk2/lab/slow/zk2py_forms.v1/@op/slow"
#: each call of the packed operation contract, and a payload for it
CALLS = (("calibrate", b"\x00"), ("scan", b"{}"), ("reset", b"\x00"))


def timeouts(ep: str) -> dict[str, Any]:
    """(1): each reply error a held query draws, by the caller's timeout."""
    import zenoh

    from . import live, zk2c

    held: list[Any] = []
    srv = live.open_client(ep)
    qbl = srv.declare_queryable(SLOW, zenoh.handlers.Callback(held.append), complete=True)
    time.sleep(0.5)
    out: dict[str, Any] = {}
    try:
        for to in (1.0, 2.5):
            cli = live.open_client(ep)
            time.sleep(0.3)
            q: queue.Queue = queue.Queue()
            t0 = time.monotonic()
            cli.get(SLOW, zenoh.handlers.Callback(q.put, lambda: q.put(None)), timeout=to,
                    target=zenoh.QueryTarget.BEST_MATCHING, consolidation=zenoh.ConsolidationMode.NONE)
            rows: list[dict[str, Any]] = []
            while True:
                try:
                    item = q.get(timeout=to + 10)
                except queue.Empty:
                    rows.append({"never_final": True})
                    break
                at = round(time.monotonic() - t0, 3)
                if item is None:
                    rows.append({"final_s": at})
                    break
                if item.ok is not None:
                    rows.append({"value_s": at})
                    continue
                enc, data = str(item.err.encoding), item.err.payload.to_bytes()
                rows.append({"error_s": at, "encoding": enc, "payload": data.decode("utf-8", "replace"),
                             "form": zk2c.read_reply_err_token(enc, data)})
            # zk2py's caller on the same call: each reply error the transport's
            got = live.call(cli, SLOW, b"", timeout=to)
            out[f"{to:g}s"] = {"rows": rows, "caller": [r.kind for r in got.replies]}
            live.close(cli)
    finally:
        held.clear()
        qbl.undeclare()
        live.close(srv)
    return out


def packed_operation(ep: str) -> dict[str, Any]:
    """(2): both owners on a packed operation contract, each call's
    answers."""
    from . import live
    from .contract import load_contract
    from .live_interop import Owner as Exe
    from .owner import Owner

    def calls(cli: Any, service: str) -> dict[str, Any]:
        rows = {}
        for op, payload in CALLS:
            got = live.call(cli, f"zk2/lab/{service}/seed.v1/@op/{op}", payload, timeout=2.0)
            rows[op] = [{"kind": x.kind, "encoding": x.encoding,
                         "code": (x.envelope or {}).get("code") if x.kind == "envelope" else None,
                         "message": (x.envelope or {}).get("message") if x.kind == "envelope" else None}
                        for x in got.replies]
        return rows

    out: dict[str, Any] = {}
    cli = live.open_client(ep)
    try:
        p = Exe(BLACKBOX / "owner", "lab/rust", [PACKED_OP], connect=ep)
        try:
            out["rust_ready"] = p.wait_for("ready ", 60) is not None
            out["rust_stderr"] = [line[:300] for line in p.stderr[-4:]]
            time.sleep(1.0)
            if out["rust_ready"]:
                out["rust"] = calls(cli, "rust")
        finally:
            p.close()
        o = Owner("lab", "py", [load_contract(PACKED_OP)], connect=ep)
        try:
            o.start()
            time.sleep(1.0)
            out["py"] = calls(cli, "py")
        finally:
            o.close()
    finally:
        live.close(cli)
    return out


def run() -> dict[str, Any]:
    from .live_interop import _r1

    router, ep, _ = _r1()
    try:
        return {"timeouts": timeouts(ep), "packed_operation": packed_operation(ep)}
    finally:
        router.close()


def judge(report: dict[str, Any]) -> list[tuple[str, bool, Any]]:
    out: list[tuple[str, bool, Any]] = []
    t = report["timeouts"]
    errs = {k: [r for r in v["rows"] if "error_s" in r] for k, v in t.items()}
    seen = sorted({(r["encoding"], r["payload"], r["form"]) for v in errs.values() for r in v})
    out.append(("(1) every held query ends with one reply error, `Timeout`, then its final reply, near its timeout",
                all(len(v) == 1 and v[0]["payload"] == "Timeout" and "final_s" in t[k]["rows"][-1]
                    and abs(v[0]["error_s"] - float(k[:-1])) < 0.3 for k, v in errs.items()), errs))
    out.append(("(1) a router's `Timeout` comes under zenoh/bytes and reads sample_kind; the querier's under "
                "zenoh/string reads encoding: never an error form",
                all(enc in ("zenoh/bytes", "zenoh/string") and form == ("sample_kind" if enc == "zenoh/bytes"
                                                                        else "encoding")
                    for enc, _, form in seen), seen))
    out.append(("(1) Appendix B measured: the router's own `Timeout` under zenoh/bytes was seen",
                any(enc == "zenoh/bytes" for enc, _, _ in seen), seen))
    out.append(("(1) zk2py's caller reads every such reply error as the transport's (§5.2, O5)",
                all(set(v["caller"]) <= {"transport"} and v["caller"] for v in t.values()),
                {k: v["caller"] for k, v in t.items()}))
    p = report["packed_operation"]
    for who in ("rust", "py"):
        rows = p.get(who)
        out.append((f"(2) {who}: serves ok-packed-operation, and answers each call with one envelope, app, under "
                    "application/json (no form: core §5.1)",
                    rows is not None and all(len(v) == 1 and v[0]["kind"] == "envelope" and v[0]["code"] == "app"
                                             and v[0]["encoding"] == "application/json" for v in rows.values()),
                    rows if rows is not None else p.get("rust_stderr")))
    return out


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="python -m zk2py.forms_live", description=__doc__.split("\n")[0])
    ap.add_argument("--out", default=None, help="write the report as JSON here")
    args = ap.parse_args(argv)
    if not (BLACKBOX / "owner").is_file():
        print(f"forms_live: the owner example is not in {BLACKBOX}", file=sys.stderr)
        return 2
    try:
        report = run()
    except Exception as e:  # noqa: BLE001 - a step that could not run is exit 2
        print(f"forms_live: could not run: {type(e).__name__}: {e}", file=sys.stderr)
        return 2
    checks = judge(report)
    report["checks"] = [{"check": n, "ok": ok, "got": g} for n, ok, g in checks]
    if args.out:
        Path(args.out).write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    for n, ok, g in checks:
        print(f"{'PASS' if ok else 'FAIL'} {n}" + ("" if ok else f": {json.dumps(g, default=str)[:600]}"))
    passed = sum(1 for _, ok, _ in checks if ok)
    print(f"forms_live: {passed}/{len(checks)}")
    return 0 if passed == len(checks) else 1


if __name__ == "__main__":
    sys.exit(main())

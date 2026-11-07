"""``freshness.v2`` 0.6 §2.4 (#1080) and core 0.100 §4.3 "Minting", live:
a heartbeat's time, and a snapshot's, counted from the catalog's epoch
(#609, round 105). ``python -m zk2py.heartbeat_epoch_live [--owner O]…``.

**The deployment.** A catalog of the test's own, ``zk2-catalog/1``, epoch
1767225600 (zenoh-modem's, 2026-01-01), listing ``beacon.v2`` (freshness.v2's
scenario contract) and ``freshness.v2``'s standard contract, written to a
temporary file. The owner, the owner example run with ``--catalog`` or
zk2py's :class:`zk2py.owner.Owner` given the catalog, serves ``beacon.v2`` at
``lab/beacon`` with ``status`` put once.

**What is checked**, by zk2py's readers (a :class:`zk2py.freshness_v2.Subscriber`
on the heartbeat key, and S4's GET):
- the descriptor names the catalog's id (core §3.3, §8.6);
- each heartbeat's time, read as freshness.v2 §2.4 says, "counts from the
  deployment epoch … For an owner given a catalog, that is the catalog's
  epoch": its 32 bits of seconds are the reader's clock less 1767225600,
  within 5 s, never Unix seconds;
- the GET's snapshot's time and its entry's confirmation count from it too
  (core §4.3, "A snapshot carries each as a time … counted from the
  deployment epoch (§7.6)"), so a GET reader that ages it under the catalog's
  epoch judges ``status`` fresh, and one that reads it under 0 judges the
  clocks to disagree.

Exit 0 when every check passes, 1 when any fails.
"""

from __future__ import annotations

import argparse
import json
import sys
import tempfile
import time
from pathlib import Path
from typing import Any

from .blackbox import BLACKBOX

REPO = Path(__file__).resolve().parents[3]
BEACON = REPO / "impl" / "python" / "interop" / "freshness" / "beacon.v2.toml"
FRESHNESS_V2 = REPO / "spec" / "profiles" / "freshness" / "freshness.v2.toml"
EPOCH = 1767225600


def _catalog() -> tuple[dict[str, Any], str]:
    from . import catalog
    from .contract import load_contract

    b, f = load_contract(BEACON), load_contract(FRESHNESS_V2)
    doc = {"format": "zk2-catalog/1", "epoch": EPOCH,
           "entries": [{"index": 0, "iface": b.interface, "contract": b.fingerprint},
                       {"index": 1, "iface": f.interface, "contract": f.fingerprint}]}
    catalog.read(json.dumps(doc))  # the reader accepts it, or this raises
    return doc, catalog.catalog_id(doc)


def run(owner: str) -> list[tuple[str, bool, str]]:
    from . import catalog, live
    from . import freshness as fr
    from . import freshness_v2 as fv2
    from .contract import load_contract
    from .live_interop import _r1

    out: list[tuple[str, bool, str]] = []
    doc, cid = _catalog()
    path = Path(tempfile.mkdtemp(prefix="zk2py-cat-")) / "catalog.json"
    path.write_text(json.dumps(doc), encoding="utf-8")
    r, ep, _ = _r1(timestamping=True)
    tool = live.open_client(ep)
    o: Any = None
    sub: Any = None
    try:
        c = load_contract(BEACON)
        sub = fv2.Subscriber(tool, "lab/beacon", c.canonical, c.fingerprint, follows=["state/status"], wait_s=0.3)
        if owner == "rust":
            from .live_interop import Owner as Exe

            o = Exe(BLACKBOX / "owner", "lab/beacon", [BEACON], connect=ep, flags=["--catalog", str(path)])
            ready = o.wait_for("ready ", 120)
            if ready is None:
                return [(f"{owner}: the owner example starts with --catalog", False, str(o.stderr[-3:]))]
            instance_key = ready.strip()
            got = o.command("put beacon.v2 state/status")
        else:
            from .owner import Owner

            o = Owner("lab", "beacon", [c], connect=ep)
            o.catalog = catalog.read(json.dumps(doc))
            o.start()
            instance_key = o.instance_key
            o.set_state("zk2/lab/beacon/beacon.v2/state/status", b"up")
            got = "put"
        time.sleep(3.0)
        desc = [json.loads(a.payload) for a in live.get_descriptor(tool, instance_key) if a.ok]
        out.append((f"{owner}: the descriptor names the catalog's id {cid} (core §3.3, §8.6)",
                    bool(desc) and desc[0].get("catalog") == cid and got.startswith("put"),
                    f"{[d.get('catalog') for d in desc]}; {got}"))
        now_s = time.time()
        beats = [x for x in sub.heartbeats if x.value is not None and x.value["time"] is not None]
        secs = [x.value["time"]["s"] for x in beats]
        out.append((f"{owner}: each heartbeat's time counts from the catalog's epoch: its seconds the clock less "
                    f"{EPOCH}, within 5 s, never Unix seconds (freshness.v2 0.6 §2.4)",
                    len(beats) >= 2 and all(abs(now_s - EPOCH - s) <= 5.0 for s in secs[-3:]),
                    f"{len(beats)} heartbeats, last {secs[-3:]}, the clock less the epoch {int(now_s - EPOCH)}"))
        st = live.get_state(tool, "zk2/lab/beacon/beacon.v2/state/status", timeout=2.0)
        page = st.snapshots[0].pages[0] if len(st.snapshots) == 1 else None
        t = None if page is None else page["time"]
        out.append((f"{owner}: the GET's snapshot's time counts from the catalog's epoch (core §4.3, §7.6)",
                    t is not None and abs(time.time() - EPOCH - (t["s"] + t["ms"] / 1000)) <= 5.0,
                    f"{t}"))
        trust = fr.ClockTrust(fr.DEFAULT_DELTA_NS, word=True)
        right = fr.get_reading(tool, "zk2/lab/beacon/beacon.v2/state/status", epoch_s=EPOCH)
        wrong = fr.get_reading(tool, "zk2/lab/beacon/beacon.v2/state/status", epoch_s=0)
        now = time.time_ns()
        v_right = fr.judge_observation("state", {"freshness.ttl_s": 2}, right.observation(trust, now))
        v_wrong = fr.judge_observation("state", {"freshness.ttl_s": 2}, wrong.observation(trust, now))
        out.append((f"{owner}: a GET reader that ages the entry under the catalog's epoch judges status fresh; under "
                    "0 it reads a time 56 years back, stale (freshness.v2 0.7 §2.6)",
                    v_right.verdict == fr.FRESH and v_wrong.verdict == fr.STALE,
                    f"{v_right.pair()}, {v_wrong.pair()}"))
    except Exception as e:  # noqa: BLE001 - a run that could not finish is a failure
        out.append((f"{owner}: the run finished", False, f"{type(e).__name__}: {e}"))
    finally:
        if sub is not None:
            sub.close()
        if o is not None:
            o.close()
        tool.close()
        r.close()
        path.unlink(missing_ok=True)
        path.parent.rmdir()
    return out


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="python -m zk2py.heartbeat_epoch_live", description=__doc__.split("\n")[0])
    ap.add_argument("--owner", action="append", choices=("rust", "zk2py"))
    args = ap.parse_args(argv)
    failed = 0
    for owner in args.owner or ["rust", "zk2py"]:
        for name, ok, detail in run(owner):
            print(f"{'PASS' if ok else 'FAIL'} {name}" + (f": {detail}" if detail else ""), flush=True)
            failed += not ok
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())

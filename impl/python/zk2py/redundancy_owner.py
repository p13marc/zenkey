"""One candidate of a ``redundancy.v1`` governed interface as a process
(#609, round 56), driven as a black box by
:mod:`zk2py.redundancy_scenarios`::

    python -m zk2py.redundancy_owner --connect EP [--contract ledger.v1|PATH] [--instance ID]
        [--name A] [--retry S] [--grace S] [--lock FILE] [--refuse-without-lock]
        [--tokenless IFACE]... [--peer] [--hold-heal] [--wait-go] [--user U --password P] ADDRESS

It prints ``ready <instance>`` once started (``refused <why>`` and exit 2
when it refuses: no instance token appears), and answers one line per
command on standard input:

- ``status``: ``redundancy status role=<role> state=<state> executions=<n> zid=<zid>``;
- ``steps``: ``redundancy steps <json>``, each [monotonic s, event, step]
  the machine returned (the clock is the host's, ``CLOCK_MONOTONIC``);
- ``reads``: ``redundancy reads <json>``, each read's [monotonic s, kind,
  reason, complete, [[id, token, descriptor]…]];
- ``put <resource> <value>``: ``redundancy put <stamp>``, or ``redundancy
  refused <why>`` while it stands by (§2.4);
- ``release-heal``: lets a held heal's grace run (scenarios.md §9's hook);
- ``unlock``: releases the lock (§2.11, its loss);
- ``remint [ID]``: a re-mint in its own session (core §8.1),
  ``redundancy remint <new id>``;
- ``go``: with ``--wait-go``, starts it (scenarios.md §3's simultaneous
  start).

Standard input closed is a clean stop (§2.8).
"""

from __future__ import annotations

import argparse
import json
import sys

from .redundancy_live import Candidate, Refused


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="python -m zk2py.redundancy_owner")
    ap.add_argument("address")
    ap.add_argument("--connect", required=True)
    ap.add_argument("--contract", default="ledger.v1")
    ap.add_argument("--instance")
    ap.add_argument("--name", default="A")
    ap.add_argument("--retry", type=float, default=2.0)
    ap.add_argument("--grace", type=float, default=2.0)
    ap.add_argument("--lock")
    ap.add_argument("--refuse-without-lock", action="store_true")
    ap.add_argument("--tokenless", action="append", default=[])
    ap.add_argument("--peer", action="store_true")
    ap.add_argument("--hold-heal", action="store_true")
    ap.add_argument("--wait-go", action="store_true")
    ap.add_argument("--user")
    ap.add_argument("--password")
    a = ap.parse_args(argv)
    auth = (a.user, a.password) if a.user else None
    c = Candidate(a.address, a.connect, a.contract, instance=a.instance, name=a.name, retry_s=a.retry,
                  grace_s=a.grace, lock=a.lock, refuse_without_lock=a.refuse_without_lock,
                  tokenless=tuple(a.tokenless), peer=a.peer, hold_heal=a.hold_heal, auth=auth)

    def out(line: str) -> None:
        print(line, flush=True)

    def begin() -> bool:
        try:
            out(f"ready {c.start()}")
            return True
        except Refused as e:
            out(f"refused {e}")
            return False

    if not a.wait_go and not begin():
        return 2
    try:
        for line in sys.stdin:
            cmd, _, rest = line.strip().partition(" ")
            if cmd == "go":
                if not begin():
                    return 2
            elif cmd == "status":
                zid = str(c.owner.session.zid()) if c.owner and c.owner.session else "-"
                with c.lock:
                    out(f"redundancy status role={c.machine.role} state={c.machine.state} "
                        f"executions={c.executions} zid={zid}")
            elif cmd == "steps":
                with c.lock:
                    out("redundancy steps " + json.dumps([[t, e if isinstance(e, str) else next(iter(e)), s]
                                                          for t, e, s in c.log]))
            elif cmd == "reads":
                with c.lock:
                    out("redundancy reads " + json.dumps([[t, kind, why, r.complete,
                                                           [[i.id, i.token, i.descriptor] for i in r.instances]]
                                                          for t, r, kind, why in c.reads]))
            elif cmd == "put":
                rid, _, value = rest.partition(" ")
                try:
                    out(f"redundancy put {c.put(rid, value)}")
                except Exception as e:  # noqa: BLE001 - a standby's refusal, said
                    out(f"redundancy refused {e}")
            elif cmd == "release-heal":
                c.release_heal()
                out("redundancy released")
            elif cmd == "remint":
                out(f"redundancy remint {c.new_epoch(rest.strip() or None)}")
            elif cmd == "unlock":
                c.unlock()
                out("redundancy unlocked")
            else:
                out(f"redundancy unknown {cmd}")
    finally:
        c.close()
    return 0


if __name__ == "__main__":
    sys.exit(main())

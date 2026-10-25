"""One candidate of a ``redundancy.v1`` governed interface as a process
(#609, rounds 56 and 58), driven as a black box by
:mod:`zk2py.redundancy_scenarios`. Since round 58 it takes the owner
example's command line and answers its ``redundancy`` commands in its JSON,
so that one runner drives either::

    python -m zk2py.redundancy_owner --connect EP --standby [--election claim|external]
        [--instance-id ID] [--name WORD] [--retry-ms MS] [--grace-ms MS] [--poll-ms MS]
        [--lease-ms MS] [--lock-file PATH] [--refuse-without-lock] [--hold-heal] [--peer]
        [--tokenless IFACE]... [--wait-go] [--user U --password P] SYSTEM/SERVICE CONTRACT.toml

Standard output, first ``connected <endpoint>``, then ``ready
zk2/<system>/<service>/@zk/instance/<instance>`` (with ``--wait-go``,
``waiting go`` first, and ``ready`` once ``go`` is read); a refusal is ``not started: <why>`` on standard error,
exit 1, nothing declared. Then one line per command:

- ``redundancy role``: ``{"redundancy": "role", "instance", "interfaces":
  {<iface>: <standing>}}``, ``<standing>`` ``{"role", "why"?, "instance"?}``;
- ``redundancy reads``: the election's reads (``made``, ``complete``,
  ``not_made``, ``polls``, ``last``) and each session's GETs;
- ``redundancy lock hold|lose``, ``redundancy calls``, ``redundancy
  hold-heal on|off``, ``redundancy wait <iface> serving|standby <ms>``,
  ``redundancy new-epoch``, each answered as the owner example does;
- ``put <iface> <resource>``: one write, ``ok``, of a raw state: ``put
  <key>``, or ``refused …`` while it stands by (§2.4);
- ``redundancy zid``: the claim's session's, the reader's and the
  watch's zids; ``redundancy reads`` carries the poll's ``poll_log`` (0.3);
- zk2py's own: ``redundancy steps`` and ``redundancy log`` (every step,
  every read, timed on the host's monotonic clock), and ``put balance
  <value>``, a chosen value.

Standard input closed is a clean stop (§2.8).
"""

from __future__ import annotations

import argparse
import json
import sys
import time
from typing import Any

from .redundancy_live import Candidate, Refused


STAND_BY_WHYS = ("incumbent", "incomplete", "own_claim_missing", "descriptor_unread", "lower_candidate", "no_lock")


def standing(c: Candidate) -> dict[str, Any]:
    """The owner example's ``<standing>``: the role, and why it stands by
    and on whom, while it does."""
    from . import redundancy as rd

    with c.lock:
        m = c.machine
        if c.fault is not None:
            # a failed act, undone: it stands by and elects no more
            return {"role": "standby", "fault": c.fault}
        out: dict[str, Any] = {"role": m.role}
        # "<why> as elections.json writes it … or no_lock": a stand-by's,
        # never a step-down's
        if m.state == rd.STANDING_BY and m.why in STAND_BY_WHYS:
            out["why"] = m.why
            if m.named:
                out["instance"] = m.named
    return out


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="python -m zk2py.redundancy_owner")
    ap.add_argument("address")
    ap.add_argument("contracts", nargs="+")
    ap.add_argument("--connect", required=True)
    ap.add_argument("--standby", action="store_true")
    ap.add_argument("--election", choices=("claim", "external"))
    ap.add_argument("--instance-id")
    ap.add_argument("--name", default="A")
    ap.add_argument("--retry-ms", type=int, default=2000)
    ap.add_argument("--grace-ms", type=int, default=2000)
    ap.add_argument("--poll-ms", type=int)
    ap.add_argument("--lease-ms", type=int)
    ap.add_argument("--lock-file")
    ap.add_argument("--refuse-without-lock", action="store_true")
    ap.add_argument("--hold-heal", action="store_true")
    ap.add_argument("--peer", action="store_true")
    ap.add_argument("--tokenless", action="append", default=[])
    ap.add_argument("--wait-go", action="store_true")
    ap.add_argument("--reader-connect", help="zk2py's own: the reader through this endpoint (a test cuts it apart)")
    ap.add_argument("--hostid-root", help="hostid.v1: the directory standing in for / (the owner example's)")
    ap.add_argument("--user")
    ap.add_argument("--password")
    a = ap.parse_args(argv)
    auth = (a.user, a.password) if a.user else None
    print(f"connected {a.connect}", flush=True)

    def refuse(why: str) -> int:
        print(f"not started: {why}", file=sys.stderr, flush=True)
        return 1

    if len(a.contracts) != 1:
        return refuse("one governed contract, the scenarios' (zk2py's driver)")
    c = Candidate(a.address, a.connect, a.contracts[0], instance=a.instance_id, name=a.name,
                  retry_s=a.retry_ms / 1000, grace_s=a.grace_ms / 1000, lock=a.lock_file,
                  refuse_without_lock=a.refuse_without_lock, tokenless=tuple(a.tokenless), peer=a.peer,
                  hold_heal=a.hold_heal, auth=auth, poll_s=None if a.poll_ms is None else a.poll_ms / 1000,
                  lease_ms=a.lease_ms, reader_endpoint=a.reader_connect, hostid_root=a.hostid_root)
    if c.declaration.declaration == "governed" and not a.standby:
        return refuse(f"{c.iface} is governed by redundancy.v1, and this service takes part in no election "
                      "(--standby) (redundancy.v1 §2.5)")
    if a.election is not None and c.declaration.governed and a.election != c.election:
        return refuse(f"--election {a.election}: {c.iface}'s contract declares {c.election} (redundancy.v1 §2.2)")

    def out(line: Any) -> None:
        print(line if isinstance(line, str) else json.dumps(line, separators=(",", ":")), flush=True)

    def begin() -> bool:
        try:
            inst = c.start()
        except Refused as e:
            refuse(str(e))
            return False
        out(f"ready zk2/{c.address}/@zk/instance/{inst}")
        return True

    if not a.wait_go and not begin():
        return 1
    if a.wait_go:
        # the owner example's --wait-go: every session open, then waiting
        try:
            c.prepare()
        except Refused as e:
            return refuse(str(e))
        out("waiting go")
    try:
        for line in sys.stdin:
            words = line.split()
            if not words:
                continue
            try:
                answer = command(c, words, line.strip(), begin)
            except Exception as e:  # noqa: BLE001 - a command that raises is refused, never fatal
                answer = f"refused {line.strip()}: {type(e).__name__}: {e}"
            if answer is False:
                return 1
            out(answer)
    finally:
        c.close()
    return 0


def command(c: Candidate, words: list[str], line: str, begin: Any) -> Any:
    if words == ["go"]:
        return "go" if begin() else False
    if words[0] == "put":
        rest = words[1:]
        if len(rest) == 2 and rest[0] == "balance":
            return f"put zk2/{c.address}/{c.iface}/state/balance {c.put('balance', rest[1])}"
        if len(rest) != 2:
            return f"refused {line}: usage: put <iface> <resource>"
        iface, res = rest
        if iface != c.iface or not res.startswith("state/"):
            return f"refused {line}: {iface} has no resource {res!r} zk2py's driver writes"
        try:
            c.put(res.removeprefix("state/"), "ok")
        except Exception as e:  # noqa: BLE001 - a standby's refusal, said (§2.4)
            return f"refused {line}: {e}"
        return f"put zk2/{c.address}/{iface}/{res}"
    if words[0] != "redundancy" or len(words) < 2:
        return f"refused {line}: unknown command"
    cmd, rest = words[1], words[2:]
    if cmd == "role":
        return {"redundancy": "role", "instance": c.instance, "interfaces": {c.iface: standing(c)}}
    if cmd == "reads":
        with c.lock:
            k = dict(c.counts)
        iface = {"made": k["made"], "complete": k["complete"], "not_made": k["not_made"], "polls": k["polls"],
                 "last": k["last"]}
        if c.poll_s is not None:
            iface["polls_complete"] = k["polls_complete"]  # zk2py's own
        sessions: dict[str, Any] = {"reader": {"reads": k["reader_reads"], "settles": k["reader_settles"]}}
        if c.poll_s is None:
            sessions["watch"] = {"reads": k["watch_reads"], "settles": k["watch_settles"]}
        else:
            with c.lock:
                iface["poll_log"] = [dict(x) for x in c.poll_log]
        return {"redundancy": "reads", "interfaces": {c.iface: iface}, "sessions": sessions}
    if cmd == "lock" and rest in (["hold"], ["lose"]):
        if rest == ["hold"]:
            held = c.relock()
        else:
            c.unlock()
            held = False
        return {"redundancy": "lock", "held": bool(held)}
    if cmd == "calls":
        with c.lock:
            return {"redundancy": "calls", "calls": c.executions}
    if cmd == "hold-heal" and rest in (["on"], ["off"]):
        if rest == ["on"]:
            c._hold_heal.clear()
        else:
            c.release_heal()
        return {"redundancy": "hold-heal", "held": rest == ["on"]}
    if cmd == "wait" and len(rest) == 3 and rest[1] in ("serving", "standby"):
        iface, want, ms = rest[0], rest[1], int(rest[2])
        deadline = time.monotonic() + ms / 1000
        while True:
            st = standing(c)
            done = (st["role"] == "serving") if want == "serving" else (st["role"] == "standby" and "why" in st)
            if done or time.monotonic() >= deadline:
                return {"redundancy": "wait", "iface": iface, "reached": done, "standing": st}
            time.sleep(0.02)
    if cmd == "new-epoch" and not rest:
        return {"redundancy": "new-epoch", "instance": c.new_epoch()}
    if cmd == "steps":
        with c.lock:
            return {"redundancy": "steps", "steps": [[t, e if isinstance(e, str) else next(iter(e)), s]
                                                     for t, e, s in c.log]}
    if cmd == "log":
        with c.lock:
            return {"redundancy": "log", "reads": [[t, kind, why, r.complete,
                                                    [[i.id, i.token, i.descriptor] for i in r.instances]]
                                                   for t, r, kind, why in c.reads]}
    if cmd == "zid" and not rest:
        # the owner example's: the claim's session, the reader and the watch
        def zid(x: Any) -> str | None:
            return None if x is None else str(x.zid())

        out = {"redundancy": "zid", "zid": zid(c.owner.session if c.owner else None), "reader": zid(c.reader)}
        if c.watch is not None:
            out["watch"] = zid(c.watch)
        return out
    return f"refused {line}: unknown command"


if __name__ == "__main__":
    rc = main()
    sys.stdout.flush()
    sys.stderr.flush()
    import os

    # zenoh-python can hang tearing down at interpreter exit, its runtime's
    # threads outliving the sessions; every session here is closed (main's
    # finally), and a runner waits on this process's exit code.
    os._exit(rc)

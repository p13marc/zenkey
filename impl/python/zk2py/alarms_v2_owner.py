"""zk2py's ``alarms.v2`` owner as a process (#609, round 124), driven by the
same words as the reference's owner example, so one runner drives either::

    python -m zk2py.alarms_v2_owner --connect <endpoint> [--alarms-record <file>]
        [--alarms-rules <rule>,...] [--cardinality alarms.v2:state/alarms/{slot}=<n>]
        [--tokenless <iface>]... [--run-file <file>] <system>/<service> <contract.toml>...

A contract file whose interface is ``alarms.v2`` is implemented in place of
the standard contract (a copy with another ``freshness.ttl_s``); the others
are the service's own, one of which may hold its table.

Standard output: ``listening``/``connected`` is not printed; first
``ready zk2/<system>/<service>/@zk/instance/<id> run <hex>``, then one line
per command read on standard input, in the owner example's words:

- ``alarm raise <json>`` -> ``alarm raised|escalated|refreshed|deferred|unchanged
  <alert_key> <slot> {"record", "words"}``, or ``refused <line>: <why>``;
- ``alarm resolve <alert_key>`` -> ``alarm resolved <alert_key> <slot> <json>``
  or ``alarm none <alert_key>``;
- ``alarm confirm <alert_key>|all on|off``;
- ``alarm list`` -> ``alarm held <json>``;
- ``alarm adoption`` -> ``alarm adoption source=… adopted=… resolved=… free=… left=…``;
- zk2py's own, a test owner's (scenarios-v2 §9, ``lab/liar``):
  ``raw <slot> <record hex> [<words json>]``, bytes put as they are;
- ``close``.
"""

from __future__ import annotations

import argparse
import json
import sys
from typing import Any

from . import alarms_v2 as a2
from .alarms_v2_live import AlarmsV2Owner, Refused


def _json(a: Any) -> str:
    return json.dumps({"record": a.record, "words": a.words if a.record.get("rule") == a2.NOT_AVAILABLE else None},
                      sort_keys=True)


def answer(line: str, o: AlarmsV2Owner) -> str:
    w = line.split(None, 2)
    try:
        if w[0] == "alarm" and w[1] == "raise":
            j = json.loads(w[2])
            act, key, slot, a = o.raise_(j["rule"], j["severity"], j.get("summary", ""), j.get("labels"),
                                         j.get("category"), j.get("reading"))
            return f"alarm {act} {key} {slot} {_json(a)}"
        if w[0] == "alarm" and w[1] == "resolve":
            got = o.resolve(w[2].strip())
            if got is None:
                return f"alarm none {w[2].strip()}"
            return f"alarm resolved {w[2].strip()} {got[0]} {_json(got[1])}"
        if w[0] == "alarm" and w[1] == "confirm":
            who, on = w[2].split()
            o.confirm(None if who == "all" else who, on == "on")
            return f"alarm confirm {who} {on}"
        if w[0] == "alarm" and w[1] == "list":
            return f"alarm held {json.dumps(o.held(), sort_keys=True)}"
        if w[0] == "alarm" and w[1] == "adoption":
            ad = o.adoption
            return (f"alarm adoption source={ad['source']} adopted={','.join(map(str, ad['adopted']))} "
                    f"resolved={','.join(map(str, ad['resolved']))} free={','.join(map(str, ad['free']))} "
                    f"left={','.join(map(str, ad['left']))}")
        if w[0] == "raw":
            rest = line.split(None, 3)
            o.put_raw(int(rest[1]), bytes.fromhex(rest[2]), rest[3].encode() if len(rest) > 3 else None)
            return f"raw {rest[1]}"
    except Refused as e:
        return f"refused {line}: {e}"
    except (ValueError, KeyError, IndexError) as e:
        return f"refused {line}: {e}"
    return f"refused {line}: unknown command"


def main(argv: list[str] | None = None) -> int:
    from .contract import load_contract

    ap = argparse.ArgumentParser(prog="python -m zk2py.alarms_v2_owner")
    ap.add_argument("--connect", required=True)
    ap.add_argument("--alarms-record")
    ap.add_argument("--alarms-rules")
    ap.add_argument("--cardinality", action="append", default=[])
    ap.add_argument("--tokenless", action="append", default=[])
    ap.add_argument("--run-file")
    # core §8.1, §8.4 "On a first hop": every interface tokenless, the
    # instance token alone held, and no bundle held (round 127: a revision a
    # reader cannot retrieve, alarms.v2 §2.7)
    ap.add_argument("--first-hop", action="store_true")
    ap.add_argument("address")
    ap.add_argument("contracts", nargs="*")
    a = ap.parse_args(argv)
    system, service = a.address.split("/", 1)
    own, ttl = [], None
    for f in a.contracts:
        c = load_contract(f)
        if not c.valid:
            print(f"error: {f}: {c.codes}", file=sys.stderr)
            return 2
        if c.interface == a2.IFACE:
            ttl = next((r.get("annotations", {}).get("freshness.ttl_s") for r in c.canonical["resources"]
                        if r["template"] == "alarms/{slot}"), None)
            if ttl is None:
                print(f"error: {f}: no `alarms/{{slot}}` with a `freshness.ttl_s` (alarms.v2 §2.7)", file=sys.stderr)
                return 2
            continue
        own.append(c)
    bound = None
    for x in a.cardinality:
        target, _, n = x.partition("=")
        if target == f"{a2.IFACE}:state/alarms/{{slot}}":
            bound = int(n)
    rules = None if a.alarms_rules is None else set(a.alarms_rules.split(","))
    o = AlarmsV2Owner(system, service, contracts=own, record=a.alarms_record, rules=rules, bound=bound,
                      ttl_s=ttl, connect=a.connect, tokenless=set(a.tokenless) or None, run_record=a.run_file,
                      **({"first_hop": True} if a.first_hop else {}))
    o.start()
    print(f"ready zk2/{system}/{service}/@zk/instance/{o.owner.instance} run {o.owner.run_hex()}", flush=True)
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        if line == "close":
            break
        print(answer(line, o), flush=True)
    o.close()
    return 0


if __name__ == "__main__":
    sys.exit(main())

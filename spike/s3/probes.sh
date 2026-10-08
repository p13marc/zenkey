#!/usr/bin/env bash
# S3's link probes (#599): what crosses a constrained face, and why.
#   s3/probes.sh <results dir>
# 1. RF presence (100 tokens over 2,400 bit/s): zenoh defaults, 1 KB batches,
#    a 60 s lease, both.
# 2. ACL placement: 200 `@zk` tokens or 200 data queryables across a
#    router-to-router link, with the R7 deny on both routers, on one, on
#    egress only; and the ground as a client of the vehicle router.
# Unix-socket paths must stay under 108 bytes: the socket dir is short.
set -u
out=${1:?results dir}
mkdir -p "$out"
spike=./target/release/spike
sock=$(mktemp -d /tmp/s3p.XXXX)
python3 - "$out" <<'PY'
import json, sys
d = sys.argv[1]
deny = {"access_control": {
    "enabled": True, "default_permission": "allow",
    "rules": [{"id": "zk-off-the-link",
               "messages": ["put", "delete", "declare_subscriber", "query", "reply", "declare_queryable",
                            "liveliness_token", "liveliness_query", "declare_liveliness_subscriber"],
               "flows": ["egress", "ingress"], "permission": "deny", "key_exprs": ["zk2/**/@zk/**"]}],
    "subjects": [{"id": "link", "link_protocols": ["tcp"]}],
    "policies": [{"id": "r7", "rules": ["zk-off-the-link"], "subjects": ["link"]}]}}
json.dump(deny, open(f"{d}/acl-deny-zk.json", "w"), indent=1)
eg = json.loads(json.dumps(deny)); eg["access_control"]["rules"][0]["flows"] = ["egress"]
json.dump(eg, open(f"{d}/acl-deny-zk-egress.json", "w"), indent=1)
da = json.loads(json.dumps(deny)); da["access_control"]["rules"][0]["key_exprs"] = ["zk2/v/*/camera.v1/**"]
json.dump(da, open(f"{d}/acl-deny-data.json", "w"), indent=1)
for name, cfg in {"batch-1k": {"transport/link/tx/batch_size": 1024},
                  "lease-60s": {"transport/link/tx/lease": 60000},
                  "batch-1k-lease-60s": {"transport/link/tx/batch_size": 1024, "transport/link/tx/lease": 60000}}.items():
    json.dump(cfg, open(f"{d}/link-{name}.json", "w"))
PY
log="$out/probes.log"
: > "$log"
run() { echo "== $*" | tee -a "$log"; "$spike" "$@" 2>&1 | grep -vE '^\s*$' | grep -E "first view|replay|settled|bytes:|state GET|tokens zk2|subscriber|in the vehicle" | tee -a "$log"; }
run s3-rf-probe --tokens 100 --bps 300
for v in batch-1k lease-60s batch-1k-lease-60s; do run s3-rf-probe --tokens 100 --bps 300 --extra "$out/link-$v.json"; done
p() { run s3-acl-probe --dir "$sock" --bps 1000000 "$@"; }
p --zk 200
p --zk 200 --acl "$out/acl-deny-zk.json"
p --zk 200 --acl "$out/acl-deny-zk.json" --side rv
p --zk 200 --acl "$out/acl-deny-zk-egress.json" --side rv
p --zk 200 --acl "$out/acl-deny-zk.json" --side rg
p --data 200
p --data 200 --acl "$out/acl-deny-data.json"
p --client-link --zk 200
p --client-link --zk 200 --live-sub
p --client-link --zk 200 --acl "$out/acl-deny-zk.json" --side rv
rm -rf "$sock"

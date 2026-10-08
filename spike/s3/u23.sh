#!/usr/bin/env bash
# U23 (#605, spec §13): the far side of a constrained face as a router of its
# own, placed in a south region of the near router (zenoh 1.10.1
# `gateway.south`, matched by `region_name`), with and without the `@zk`
# deny. Compare with the router-to-router and client-link rows of probes.sh.
#   s3/u23.sh <results dir>
set -u
out=${1:?results dir}
mkdir -p "$out"
spike=./target/release/spike
sock=$(mktemp -d /tmp/u23.XXXX)
log="$out/u23.log"
: > "$log"
run() { echo "== $*" | tee -a "$log"; "$spike" s3-acl-probe --dir "$sock" --bps 1000000 "$@" 2>&1 | grep -E "state GET|tokens zk2|subscriber|bytes:|in the vehicle" | tee -a "$log"; }
acl="$out/acl-deny-zk.json"
run --zk 200
run --zk 200 --acl "$acl"
run --zk 200 --south
run --zk 200 --south --acl "$acl"
run --zk 200 --south --acl "$acl" --side rv
run --zk 200 --south --acl "$acl" --live-sub
run --data 200 --south
run --data 200 --south --acl "$out/acl-deny-data.json"
rm -rf "$sock"

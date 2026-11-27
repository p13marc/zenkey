#!/usr/bin/env bash
# #989's wire baseline: one W2 run (constrained-overhead-review.md §2.2).
#
# Usage: measure-w2.sh <medium> <stack> <run-name>
#
# 1. Bring the path up (wire.sh), each driver's management plane recorded
#    from its own unix socket.
# 2. The far side, on router B, as a host deployment reads: one
#    `zenctl echo --no-decode` subscriber on zk2/** (every element the device
#    puts: telemetry, health, alarms, config, the desired report, events),
#    and the desired author (w2-drive.py starts it).
# 3. w2-drive.py runs the device on router A and the timeline: settle, the
#    steady window, the acts, the reconnect. Its marks.json bounds each.
# 4. Bring everything down.
#
# Environment: wire.sh's (WORK, INST, BASE, DICT, LOSS_START, CORES), CONTRACTS
# (the directory of gendev.v1.toml, default this one's contracts/),
# SETTLE_S (120), STEADY_S (2100), CUT_S (the lease + 5 s), AFTER_S (90), and
# LOSS_AT_STEADY: with LOSS_START=0, the loss the relay takes 60 s before the
# steady window, so the setup's declarations all cross.
#
# ZK2C=1 (#989's zk2c run, with medium rf-zk2c): w2-drive.py's --zk2c device,
# and the far side reads as core §8.5 asks across a constrained link, never
# `zk2/**`: one `zenctl echo --across-face` per interface, whose scope is each
# resource by its exact key, so router A declares each key's scope and a put
# names its member by a short suffix. gendev.v1's is narrowed to frames/sample
# and its events. No face policy: W2's standard contracts expose nothing
# `link`, so a face would carry none of health, alarms or config; the routers
# run link.v2's transport (the medium's) as peers, as phase 2's did. With
# RUN_FILE=1 the device and gen take 16-bit runs from run files (core §4.3).
#
# OWNER_HEARTBEAT=1 (#1110's H4, with ZK2C=1; off by default, so the recorded
# runs reproduce): the device sends owner heartbeats (freshness.v2 §2.4), one
# put per class of exposure in place of each listed interface's heartbeat.
# - The catalog, LOGS/catalog.json, is `zk2 catalog new` over the device's
#   three revisions that have a collection: health.v1, alarms.v1, rfcfg.v1
#   (BIN/zk2, zenkey-model's).
# - Every far reader gets `--catalog`. A reader's scope then also holds the
#   class key of each collection in hand, here
#   `…/freshness.v2/stream/heartbeats/host`. Without it the reader declares
#   no class key, nothing asks router A for one, and no owner heartbeat
#   crosses.
# V2=1 (#986's J5 and #984's S4, with ZK2C=1; off by default, so the recorded runs
# reproduce): w2-drive.py's --v2. The device's desired target and its alarms run on
# desired.v2 and alarms.v2, nothing else changed: its alarms by contracts/w2rules.v1.toml's
# table, each re-read a conditional read waiting rf0's 15-s reply timeout, the author on a
# 16-bit run with RUN_FILE=1. The far readers read alarms.v2's standard contract and
# worker.v2. DESIRED_TTL=<s> gives author and target "tasking.v2, watched" at that horizon
# (desired.v2 §2.5): the author's heartbeat, B->A, in place of the target's 300-s period.
# Its revision of tasking.v2 is LOGS/tasking.v2.toml, which the host's probe reads.
# WATCH=1 (the honesty run of the same): w2-drive.py's --watch, the host's
# `zenctl watch` on health.v1's status, with the catalog and with none, and
# `zenctl health --across-face crosses` before and inside the cut.
set -euo pipefail

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
export WORK=${WORK:-/srv/dev/targets/zenkey-989}
export INST=${INST:-rf}
export BASE=${BASE:-17400}
MEDIUM=${1:?medium: rf, rf-zk2c, sf7, sf9 or sf12}
STACK=${2:?stack: bare or lean}
NAME=${3:?run name}
export LOGS=$WORK/logs/$NAME
mkdir -p "$LOGS"
[[ -e $LOGS/marks.json ]] && { echo "$LOGS already holds a run" >&2; exit 2; }
W=$HERE/wire.sh
BIN=${BIN:-$WORK/bin}

trap '$W down || true' EXIT

$W up "$MEDIUM" "$STACK"
LEASE_S=$(python3 -I -c 'import json,sys; print(json.load(open(sys.argv[1]))["lease"] // 1000)' "$WORK/cfg/$INST/params.json")
for d in a b; do
  $W start "counters-$d" bash -c "$BIN/zenctl echo 'v1/*/telemetry/modem/device/*/**' --no-decode --format ndjson \
    -c unixsock-stream/$WORK/run/$INST/$d/mgmt.sock | python3 -I -u $HERE/stamp.py $LOGS/counters-$d.jsonl"
done
sleep 10
REPO=${REPO:-$(cd "$HERE/../../../.." && pwd)}
# gendev.v1.toml names its schemas by a path relative to the checkout: a
# frozen copy elsewhere reads the contract from the checkout's directory.
CONTRACTS=${CONTRACTS:-$HERE/contracts}
DEV=h-3fa9c2d41b7e/rf0
ZK2C_ARGS=()
CATALOG=()
if [[ ${ZK2C:-0} == 1 && ${OWNER_HEARTBEAT:-0} == 1 ]]; then
  "$BIN/zk2" catalog new "$REPO/spec/profiles/health/health.v1.toml" \
    "$REPO/spec/profiles/alarms/alarms.v1.toml" \
    "$REPO/spec/profiles/config/conformance/contracts/rfcfg.v1.toml" >"$LOGS/catalog.json"
  CATALOG=(--catalog "$LOGS/catalog.json")
fi
ALARMS_TOML=$REPO/spec/profiles/alarms/alarms.v1.toml
WORKER_TOML=$REPO/zenkey/tests/contracts/worker.v1.toml
if [[ ${ZK2C:-0} == 1 && ${V2:-0} == 1 ]]; then
  ALARMS_TOML=$REPO/spec/profiles/alarms/alarms.v2.toml
  WORKER_TOML=$REPO/zenkey/tests/contracts/worker.v2.toml
  TASKING_TOML=$REPO/zenkey/tests/contracts/tasking.v2.toml
  if [[ ${DESIRED_TTL:-0} != 0 ]]; then
    # "tasking.v2, watched": the file with that horizon, as the owner example makes it.
    cp "$REPO/zenkey/tests/contracts/tasking.json" "$LOGS/"
    sed "s/\"freshness.ttl_s\" = 0 }/\"freshness.ttl_s\" = $DESIRED_TTL }/" \
      "$REPO/zenkey/tests/contracts/tasking.v2.toml" >"$LOGS/tasking.v2.toml"
    grep -q "\"freshness.ttl_s\" = $DESIRED_TTL }" "$LOGS/tasking.v2.toml"
    TASKING_TOML=$LOGS/tasking.v2.toml
  fi
fi
if [[ ${ZK2C:-0} == 1 ]]; then
  # name, contract, selector (empty: the interface's whole scope)
  for spec in "frames|$CONTRACTS/gendev.v1.toml|zk2/$DEV/gendev.v1/stream/frames/sample" \
              "events|$CONTRACTS/gendev.v1.toml|zk2/$DEV/gendev.v1/events/**" \
              "health|$REPO/spec/profiles/health/health.v1.toml|" \
              "alarms|$ALARMS_TOML|" \
              "rfcfg|$REPO/spec/profiles/config/conformance/contracts/rfcfg.v1.toml|" \
              "worker|$WORKER_TOML|"; do
    IFS='|' read -r name contract sel <<<"$spec"
    # The reader's command line is a string for `bash -c`: each word that is a path or a
    # selector goes in quoted (%q), so a directory with a space in it stays one word, and a
    # selector's `**` is never a glob. An empty selector and an empty CATALOG add no word.
    catalog_q=""
    ((${#CATALOG[@]})) && catalog_q=$(printf ' %q' "${CATALOG[@]}")
    $W start "far-$name" bash -c "$(printf %q "$BIN/zenctl") echo ${sel:+$(printf %q "$sel")} \
      --namespace '' --across-face $DEV --contracts $(printf %q "$contract")$catalog_q \
      --no-decode --format ndjson -c tcp/127.0.0.1:$((BASE + 2)) \
      | python3 -I -u $(printf %q "$HERE/stamp.py") $(printf %q "$LOGS/far.jsonl")"
  done
  ZK2C_ARGS=(--zk2c --rfcfg "$REPO/spec/profiles/config/conformance/contracts/rfcfg.v1.toml")
  [[ ${RUN_FILE:-0} == 1 ]] && ZK2C_ARGS+=(--run-files)
  [[ ${OWNER_HEARTBEAT:-0} == 1 ]] && ZK2C_ARGS+=("${CATALOG[@]}" --owner-heartbeat)
  [[ ${WATCH:-0} == 1 ]] && ZK2C_ARGS+=(--watch --health-contract "$REPO/spec/profiles/health/health.v1.toml")
  [[ ${V2:-0} == 1 ]] && ZK2C_ARGS+=(--v2 --rules "$HERE/contracts/w2rules.v1.toml" \
    --desired-ttl "${DESIRED_TTL:-0}" --tasking "$TASKING_TOML" --worker "$WORKER_TOML" \
    --alarms-contract "$ALARMS_TOML")
else
  $W start far bash -c "$BIN/zenctl echo 'zk2/**' --no-decode --format ndjson -c tcp/127.0.0.1:$((BASE + 2)) \
    | python3 -I -u $HERE/stamp.py $LOGS/far.jsonl"
fi
sleep 3
out=$($W start drive python3 -I -u "$HERE/w2-drive.py" --owner "$BIN/owner" --zenctl "$BIN/zenctl" \
  --contract "$CONTRACTS/gendev.v1.toml" \
  --device-connect "tcp/127.0.0.1:$((BASE + 1))" --host-connect "tcp/127.0.0.1:$((BASE + 2))" \
  --control "127.0.0.1:$((BASE + 14))" --logs "$LOGS" \
  --settle-s "${SETTLE_S:-120}" --steady-s "${STEADY_S:-2100}" \
  --cut-s "${CUT_S:-$((LEASE_S + 5))}" --after-s "${AFTER_S:-90}" \
  ${LOSS_AT_STEADY:+--loss-at-steady "$LOSS_AT_STEADY"} "${ZK2C_ARGS[@]}")
echo "$out"
pid=${out##* }
while kill -0 "$pid" 2>/dev/null; do sleep 5; done
cat "$LOGS/drive.log"
[[ -e $LOGS/marks.json ]] || { echo "the drive ended without its marks" >&2; exit 1; }
echo "done: $LOGS"

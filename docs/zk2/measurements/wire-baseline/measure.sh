#!/usr/bin/env bash
# #989's wire baseline: one W1 run on a simulated radio.
#
# Usage: measure.sh <medium> <stack> <run-name>     (wire.sh's media and stacks)
#
# 1. Bring the path up (wire.sh), each driver's management plane recorded:
#    every reading, stamped, from its own unix socket, so none of it is on the air.
# 2. The idle window: the link alone, no zk2 client on either router.
# 3. W1: modem.v3's 4 link readings at one a minute. `zenctl gen` brings up a
#    modem.v3 owner on router A (every resource at 1/60 Hz, on the host side);
#    four `zenctl echo --no-decode` subscribers on router B, one per link
#    reading, are the far side, so zenoh carries those four and no other data.
# 4. Bring everything down. WORK/logs/<run-name>/marks.json holds the windows'
#    bounds; analyze.py reads the run.
#
# W1=73 (#989's zk2c run, with medium rf-zk2c and wire.sh's FACE_POLICY): all
# 73 readings instead. `zenctl gen` brings up modem.v3 under zenoh-modem's
# catalog on router A, as the principal rf-node, every resource at 1/60 Hz:
# frames/fast carries the 73 readings in one packed record a minute, a full
# record every 15. Its face's plan on router A lets through only what crosses
# rf0 (frames/fast and the `face/` events). The far side is one `zenctl echo
# --across-face` on router B, whose scope is that plan: it declares nothing
# the plan keeps off the face. The owner is zenkey's generator, not
# zenoh-modem's modem-mgmt, which is not ported yet (zenoh-modem#214).
#
# Environment: wire.sh's (WORK, INST, BASE, LOSS_START, CORES, FACE_POLICY),
# W1 (4 or 73), RUN_FILE=1 (W1=73: gen takes 16-bit runs from a run file, as a
# constrained owner does, core §4.3; else each instance draws a 32-bit run),
# DEVICE, FACE and PRINCIPAL (W1=73: rf0's, below), and the windows:
# SETTLE_S (30) after the link opens, IDLE_S (600), W1_LEAD_S (90: gen's
# first tick is at once, so 90 starts the window half a period after its
# second) and W1_S (600; 0 ends the run after the idle window). With
# LOSSLESS_SETUP=1 and LOSS_START=0, the link opens and W1 sets up with no
# loss, and each window runs at the medium's: so every declaration crosses.
set -euo pipefail

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
REPO=${REPO:-$(cd "$HERE/../../../.." && pwd)}
export WORK=${WORK:-/srv/dev/targets/zenkey-989}
export INST=${INST:-rf}
export BASE=${BASE:-17400}
MEDIUM=${1:?medium: rf, rf-zk2c, sf7, sf7-zk2c, sf9 or sf12}
STACK=${2:?stack: bare or lean}
NAME=${3:?run name}
SETTLE_S=${SETTLE_S:-30}
IDLE_S=${IDLE_S:-600}
W1_LEAD_S=${W1_LEAD_S:-90}
W1_S=${W1_S:-600}
# W1=73's device, the face it sits behind and the principal it runs as: rf0's by
# default; lora0's is h-c2a95e41d7f0/lora0, lora and lora-node.
DEVICE=${DEVICE:-h-3fa9c2d41b7e/rf0}
FACE=${FACE:-rf}
PRINCIPAL=${PRINCIPAL:-rf-node}
export LOGS=$WORK/logs/$NAME
mkdir -p "$LOGS"
[[ -e $LOGS/marks.json ]] && { echo "$LOGS already holds a run" >&2; exit 2; }
W=$HERE/wire.sh
ZENCTL=${BIN:-$WORK/bin}/zenctl
CONTRACT=$REPO/examples/zk2/zenoh-modem/modem.v3.toml
KEY=zk2/h-3fa9c2d41b7e/rf0/modem.v3/stream
LINK=(sdu/dropped_total sched/queue_depth air/duty_cycle_available air/duty_cycle_wait_ms)

now() { python3 -I -c 'import time; print(f"{time.time():.6f}")'; }
mark() { echo "\"$1\": $(now)," >>"$LOGS/marks.part"; echo "mark $1 $(date -Is)"; }

trap '$W down || true' EXIT

$W up "$MEDIUM" "$STACK"
for d in a b; do
  $W start "counters-$d" bash -c "$ZENCTL echo 'v1/*/telemetry/modem/device/*/**' --no-decode --format ndjson \
    -c unixsock-stream/$WORK/run/$INST/$d/mgmt.sock | python3 -I -u $HERE/stamp.py $LOGS/counters-$d.jsonl"
done
sleep "$SETTLE_S"
# LOSSLESS_SETUP=1 (with LOSS_START=0): each setup is made lossless, so every
# declaration crosses, and the medium's loss holds in each window.
LOSS=$(python3 -I -c 'import json,sys; print(json.load(open(sys.argv[1]))["loss"])' "$WORK/cfg/$INST/params.json")
lossy() { [[ ${LOSSLESS_SETUP:-0} == 1 ]] && $W loss "$1" || true; }
lossy "$LOSS"

mark idle_start
sleep "$IDLE_S"
mark idle_end
[[ $W1_S == 0 ]] && { { echo "{"; cat "$LOGS/marks.part"; echo "\"medium\": \"$MEDIUM\", \"stack\": \"$STACK\"}"; } >"$LOGS/marks.json"; rm "$LOGS/marks.part"; echo "done: $LOGS"; exit 0; }
lossy 0

if [[ ${W1:-4} == 73 ]]; then
  MODEM=$REPO/examples/zk2/zenoh-modem
  $W start far bash -c "$ZENCTL echo --namespace '' --across-face $DEVICE \
    --face-policy $FACE_POLICY --face $FACE --contracts $MODEM --catalog $MODEM/catalog.json \
    --zenoh-config $WORK/cfg/auth/local.json5 --format ndjson -c tcp/127.0.0.1:$((BASE + 2)) \
    | python3 -I -u $HERE/stamp.py $LOGS/far.jsonl --all"
  sleep 5
  mark gen_start
  $W start gen "$ZENCTL" gen --namespace '' "$DEVICE" modem.v3 --contracts "$MODEM" \
    --catalog "$MODEM/catalog.json" --zenoh-config "$WORK/cfg/auth/$PRINCIPAL.json5" \
    ${RUN_FILE:+--run-file "$LOGS/gen.run"} \
    --rate 0.016666666666666666 --duration $((W1_LEAD_S + W1_S + 90)) \
    -c tcp/127.0.0.1:$((BASE + 1)) --format ndjson
else
  for r in "${LINK[@]}"; do
    n=${r//\//_}
    $W start "far-$n" bash -c "$ZENCTL echo $KEY/$r --no-decode --format ndjson -c tcp/127.0.0.1:$((BASE + 2)) \
      | python3 -I -u $HERE/stamp.py $LOGS/far.jsonl"
  done
  sleep 5
  mark gen_start
  $W start gen "$ZENCTL" gen h-3fa9c2d41b7e/rf0 modem.v3 --contracts "$CONTRACT" \
    --rate 0.016666666666666666 --duration $((W1_LEAD_S + W1_S + 90)) \
    -c tcp/127.0.0.1:$((BASE + 1)) --format ndjson
fi
sleep $((W1_LEAD_S - 30))
lossy "$LOSS"
sleep 30
mark w1_start
sleep "$W1_S"
mark w1_end

{ echo "{"; cat "$LOGS/marks.part"; echo "\"medium\": \"$MEDIUM\", \"stack\": \"$STACK\", \"w1\": ${W1:-4}}"; } >"$LOGS/marks.json"
rm "$LOGS/marks.part"
echo "done: $LOGS"

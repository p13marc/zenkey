#!/usr/bin/env bash
# #989's honesty checks across rf0's face (zk2c, W1-73's path), one each.
#
# Usage: FACE_POLICY=<zenohd-rf0.face.toml> honesty.sh <run-name>
#
# The path is W1-73's: medium rf-zk2c, no layers, rf0's plan on router A,
# `zenctl gen` as modem.v3's owner (zenkey's generator, not zenoh-modem's
# modem-mgmt, zenoh-modem#214) and, on router B, the far side reading across
# the face (--across-face, --face-policy, --face rf), as a ground operator's
# tools would. Every tool's output goes to WORK/logs/<run-name>/, and
# marks.json says when each step began. The medium's loss (5 %) holds
# throughout, but where a step sets another.
#
# 1. Loss is detected: the relay takes 75 % for 240 s (four frames: one is
#    lost with p = 1 - 0.25^4); the far frame reader's next frame `follows`
#    a `gap` with the records `lost`, and every field it omits reads unknown,
#    `gap` ("not included since records were lost"), never its last value.
# 2. No false "nothing fires": `zenctl telemetry` across the face over a
#    window holding the loss reads no rate across it and says the reader
#    lost deliveries, never a rate of 0.
# 3. Stale is never fresh: the relay cuts the link for longer than
#    frames/fast's 300-s horizon. While it is cut, `zenctl watch`, `zenctl
#    telemetry` and `zenctl doctor` run across the face; none may read the
#    readings as current.
# 4. A reset is detected: gen stops, and a new gen starts (a new instance and
#    a new run); its first frame `follows` a `run` the reader did not hold,
#    and `zenctl telemetry`, over a window holding both runs, reads a counter
#    pair across the two as `reset`.
#
# Environment: wire.sh's (WORK, INST, BASE, CORES, FACE_POLICY), and STEPS,
# the steps run (default "loss stale reset"; 2 rides on loss's window).
set -euo pipefail

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
REPO=${REPO:-$(cd "$HERE/../../../.." && pwd)}
export WORK=${WORK:-/srv/dev/targets/zenkey-989}
export INST=${INST:-honesty}
export BASE=${BASE:-17600}
export LOSS_START=${LOSS_START:-0.05}
NAME=${1:?run name}
export LOGS=$WORK/logs/$NAME
mkdir -p "$LOGS"
[[ -e $LOGS/marks.json ]] && { echo "$LOGS already holds a run" >&2; exit 2; }
: "${FACE_POLICY:?a link.v2 face policy: examples/zk2/zenoh-modem/faces/zenohd-rf0.face.toml}"
W=$HERE/wire.sh
ZENCTL=${BIN:-$WORK/bin}/zenctl
MODEM=$REPO/examples/zk2/zenoh-modem
DEV=h-3fa9c2d41b7e/rf0
B=tcp/127.0.0.1:$((BASE + 2))
ACROSS=(--namespace '' --face-policy "$FACE_POLICY" --face rf --contracts "$MODEM"
        --catalog "$MODEM/catalog.json" --zenoh-config "$WORK/cfg/auth/local.json5" -c "$B")
SERIES=zk2/$DEV/modem.v3/stream/frames/fast

now() { python3 -I -c 'import time; print(f"{time.time():.6f}")'; }
mark() { echo "\"$1\": $(now)," >>"$LOGS/marks.part"; echo "mark $1 $(date -Is)"; }
gen() { # name, seconds
  $W start "$1" "$ZENCTL" gen --namespace '' "$DEV" modem.v3 --contracts "$MODEM" \
    --catalog "$MODEM/catalog.json" --zenoh-config "$WORK/cfg/auth/rf-node.json5" \
    --rate 0.016666666666666666 --duration "$2" -c tcp/127.0.0.1:$((BASE + 1)) --format ndjson
}
probe() { # name, command...: one tool's reading across the face, at most 120 s
  local name=$1; shift
  mark "$name"
  local rc=0
  timeout 120 taskset -c "${CORES:-0-1}" "$@" </dev/null >"$LOGS/$name.out" 2>"$LOGS/$name.err" || rc=$?
  echo "\"${name}_exit\": $rc," >>"$LOGS/marks.part"
  echo "$name exit $rc"
}

trap '$W down || true' EXIT

$W up rf-zk2c bare
# The far side: the frame reader, records and field states, for the whole run.
$W start far bash -c "$ZENCTL echo --across-face $DEV ${ACROSS[*]@Q} --format ndjson \
  | python3 -I -u $HERE/stamp.py $LOGS/far.jsonl --all"
$W start watch "$ZENCTL" watch "$DEV" modem.v3 frames/fast --across-face "${ACROSS[@]}" --format ndjson
sleep 5
mark gen1
gen gen1 3000
sleep 200

step() { [[ " ${STEPS:-loss stale reset} " == *" $1 "* ]]; }

if step loss; then
  # 2's window opens before the loss and closes after it.
  $W start telemetry "$ZENCTL" telemetry "$SERIES" --for 600 --across-face "${ACROSS[@]}" \
    --lease 240 --wait-before-close 240 --format json
  sleep 30
  # 1. Loss.
  $W loss 0.75
  mark loss_on
  sleep 240
  $W loss 0.05
  mark loss_off
  sleep 420
fi

if step stale; then
  # 3. Stale: a cut past the 300-s horizon (and the 240-s lease).
  $W cut
  mark cut
  sleep 330
  probe stale-watch "$ZENCTL" watch "$DEV" modem.v3 frames/fast --across-face "${ACROSS[@]}" \
    --for 60 --format ndjson
  probe stale-telemetry "$ZENCTL" telemetry "$SERIES" --for 60 --across-face "${ACROSS[@]}" \
    --lease 240 --wait-before-close 240 --format json
  probe stale-doctor "$ZENCTL" doctor --namespace '' --zenoh-config "$WORK/cfg/auth/local.json5" \
    -c "$B" --format json
  $W restore
  mark restore
  sleep 300
fi

if step reset; then
  # 4. Reset: a telemetry window holding both runs; gen1 stops (its timeout's
  # PID, which wire.sh started), and gen2 starts.
  $W start telemetry2 "$ZENCTL" telemetry "$SERIES" --for 480 --across-face "${ACROSS[@]}" \
    --lease 240 --wait-before-close 240 --format json
  sleep 150
  pid=$(awk '$2 == "gen1" {print $1}' "$WORK/run/$INST/pids")
  kill -TERM "$pid"
  mark gen1_stop
  sleep 20
  mark gen2
  gen gen2 600
  sleep 330
fi

{ echo "{"; cat "$LOGS/marks.part"; echo "\"medium\": \"rf-zk2c\", \"stack\": \"bare\"}"; } >"$LOGS/marks.json"
rm "$LOGS/marks.part"
echo "done: $LOGS"

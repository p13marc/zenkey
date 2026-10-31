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
# Environment: wire.sh's (WORK, INST, BASE, DICT), and SETTLE_S (120),
# STEADY_S (2100), CUT_S (the lease + 5 s), AFTER_S (90).
set -euo pipefail

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
export WORK=${WORK:-/srv/dev/targets/zenkey-989}
export INST=${INST:-rf}
export BASE=${BASE:-17400}
MEDIUM=${1:?medium: rf, sf7, sf9 or sf12}
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
$W start far bash -c "$BIN/zenctl echo 'zk2/**' --no-decode --format ndjson -c tcp/127.0.0.1:$((BASE + 2)) \
  | python3 -I -u $HERE/stamp.py $LOGS/far.jsonl"
sleep 3
out=$($W start drive python3 -I -u "$HERE/w2-drive.py" --owner "$BIN/owner" --zenctl "$BIN/zenctl" \
  --contract "$HERE/contracts/gendev.v1.toml" \
  --device-connect "tcp/127.0.0.1:$((BASE + 1))" --host-connect "tcp/127.0.0.1:$((BASE + 2))" \
  --control "127.0.0.1:$((BASE + 14))" --logs "$LOGS" \
  --settle-s "${SETTLE_S:-120}" --steady-s "${STEADY_S:-2100}" \
  --cut-s "${CUT_S:-$((LEASE_S + 5))}" --after-s "${AFTER_S:-90}")
echo "$out"
pid=${out##* }
while kill -0 "$pid" 2>/dev/null; do sleep 5; done
cat "$LOGS/drive.log"
[[ -e $LOGS/marks.json ]] || { echo "the drive ended without its marks" >&2; exit 1; }
echo "done: $LOGS"

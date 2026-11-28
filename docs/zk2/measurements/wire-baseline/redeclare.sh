#!/usr/bin/env bash
# rc-023: does a far reader's re-declaration (core §8.5, zenkey::redeclare) repair a lost
# DeclareSubscriber across rf0's face?
#
# Usage: FACE_POLICY=<zenohd-rf0.face.toml> CASE=alone|shared redeclare.sh <run-name>
#
# W1-73's path (wire.sh rf-zk2c bare, rf0's plan on router A), no loss but one cut, `zenctl gen`
# as modem.v3's owner:
# 1. R0, a far frame reader (`zenctl echo --across-face`), declares every key once, so router B's
#    key expressions toward router A cross; then it leaves, and its undeclarations cross.
# 2. The relay cuts the link for 8.5 s and R1, the same reader, starts inside the cut: router B's
#    DeclareSubscribers toward A, which name the key expressions A holds, are lost, and the
#    transport stays up.
# 3. CASE=shared: R2, the same reader again, starts 15 s after the restore.
# 4. WATCH_S (720 s), more than two of frames/fast's 300-s horizons: R1 declares again at 600 s
#    of silence. Since core 0.121 (#1185) it declares a repair subscription at 1,200 s, so the
#    re-soak watches 1,380 s (WATCH_S=1380).
# R1.jsonl and R2.jsonl hold what each heard, and R1.log and R2.log each re-declaration and
# repair (READER_LOG); the capture is bare, so zdecode.py reads every declaration that crossed.
#
# Environment: wire.sh's (WORK, INST, BASE, CORES, FACE_POLICY), CASE, WATCH_S, READER_LOG.
set -euo pipefail

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
REPO=${REPO:-$(cd "$HERE/../../../.." && pwd)}
export WORK=${WORK:-/srv/dev/targets/zenkey-989}
export INST=${INST:-redeclare}
export BASE=${BASE:-17800}
export LOSS_START=0 LIFE=${LIFE:-40m}
NAME=${1:?run name}
WATCH_S=${WATCH_S:-720}
READER_LOG=${READER_LOG:-warn,zenkey::redeclare=info}
: "${FACE_POLICY:?a link.v2 face policy: examples/zk2/zenoh-modem/faces/zenohd-rf0.face.toml}"
[[ ${CASE:-} == alone || ${CASE:-} == shared ]] || { echo "CASE=alone or CASE=shared" >&2; exit 2; }
export LOGS=$WORK/logs/$NAME
mkdir -p "$LOGS"
[[ -e $LOGS/acts.jsonl ]] && { echo "$LOGS already holds a run" >&2; exit 2; }
W=$HERE/wire.sh
Z=${BIN:-$WORK/bin}/zenctl
M=$REPO/examples/zk2/zenoh-modem
DEV=h-3fa9c2d41b7e/rf0
B=tcp/127.0.0.1:$((BASE + 2))

now() { python3 -I -c 'import time; print(f"{time.time():.6f}")'; }
mark() { echo "{\"t\": $(now), \"mark\": \"$1\"}" >>"$LOGS/acts.jsonl"; echo "mark $1 $(date -u +%T)"; }
reader() { # name
  $W start "$1" bash -c "RUST_LOG='$READER_LOG' $Z echo --namespace '' --across-face $DEV --face-policy $FACE_POLICY --face rf \
    --contracts $M --catalog $M/catalog.json --zenoh-config $WORK/cfg/auth/local.json5 --format ndjson \
    -c $B | python3 -I -u $HERE/stamp.py $LOGS/$1.jsonl --all"
}

trap '$W down || true' EXIT
$W up rf-zk2c bare
sleep 5
mark gen
$W start gen "$Z" gen --namespace '' "$DEV" modem.v3 --contracts "$M" --catalog "$M/catalog.json" \
  --zenoh-config "$WORK/cfg/auth/rf-node.json5" --rate 0.016666666666666666 --duration $((WATCH_S + 480)) \
  -c tcp/127.0.0.1:$((BASE + 1)) --format ndjson
sleep 35
mark r0
reader r0
sleep 70
r0=$(awk '$2 == "r0" {print $1}' "$WORK/run/$INST/pids" | head -n1)   # its timeout's PID
[[ -n $r0 ]] || { echo "no pid recorded for r0" >&2; exit 2; }
kill -TERM "$r0"
mark r0_stop
sleep 10
$W cut
mark cut
sleep 0.5
mark r1
reader r1
sleep 8
$W restore
mark restore
if [[ $CASE == shared ]]; then
  sleep 15
  mark r2
  reader r2
fi
sleep "$WATCH_S"
mark end

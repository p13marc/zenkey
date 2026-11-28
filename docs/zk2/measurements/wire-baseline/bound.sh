#!/usr/bin/env bash
# #1136: where zenoh starts to fragment a put across rf0's face, bare.
#
# Usage: bound.sh <medium> <run-name>     (rf-zk2c: 16-bit sequence numbers; rf-zk2c-32: 32-bit)
#
# 1. Bring rf0's face up with no layers and no loss (wire.sh, FACE_POLICY rf0's policy; at
#    32 bits a copy of it with `sequence = "32bit"`), the routers' ids ZIDS, so that A->B's
#    initial sequence number is known before the run (bound.py computes it, and checks it against the OpenAck).
# 2. The far side: one `zenctl echo` on router B of frames/fast's key alone, which it
#    declares, so that router A sends each put by that key's id, as a device's frame crosses.
# 3. The near side: `zenctl replay` as rf-node of captures written here from PUTS, one put
#    every GAP_S seconds on frames/fast's key (a capture's `t` is in microseconds), each
#    with its QoS axes and a payload of its length, so that each put is alone in its batch.
#    Payload i is the byte i, repeated. Replay declares one publisher per key, with its first
#    row's QoS, so each run of puts with one QoS is a capture and a replay of its own.
# 4. Bring everything down. WORK/logs/<run>/marks.json holds the puts; bound.py reads the run.
#
# Environment: wire.sh's (WORK, INST, BASE, CORES, BIN, ZENOHD, ZIDS); PUTS, the puts in order,
# "<priority>/<congestion>/<reliability>:<payload bytes>" separated by spaces; GAP_S (8);
# SETTLE_S (40) after the link opens.
set -euo pipefail

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
REPO=${REPO:-$(cd "$HERE/../../../.." && pwd)}
export WORK=${WORK:-/srv/dev/targets/zenkey-989}
export INST=${INST:-bound}
export BASE=${BASE:-17800}
export LOSS_START=0 REPO
MEDIUM=${1:?medium: rf-zk2c or rf-zk2c-32}
NAME=${2:?run name}
GAP_S=${GAP_S:-8}
SETTLE_S=${SETTLE_S:-40}
PUTS=${PUTS:?the puts, "<qos axes>:<bytes> ..."}
export LOGS=$WORK/logs/$NAME
mkdir -p "$LOGS"
[[ -e $LOGS/marks.json ]] && { echo "$LOGS already holds a run" >&2; exit 2; }
W=$HERE/wire.sh
ZENCTL=${BIN:-$WORK/bin}/zenctl
KEY=zk2/h-3fa9c2d41b7e/rf0/modem.v3/stream/frames/fast
POLICY=$REPO/examples/zk2/zenoh-modem/faces/zenohd-rf0.face.toml
if [[ $MEDIUM == rf-zk2c-32 ]]; then
  mkdir -p "$WORK/cfg/$INST"
  sed 's/^sequence *= *"16bit"/sequence             = "32bit"/' "$POLICY" >"$WORK/cfg/$INST/rf0-32bit.face.toml"
  grep -q '^sequence *= *"32bit"' "$WORK/cfg/$INST/rf0-32bit.face.toml"
  POLICY=$WORK/cfg/$INST/rf0-32bit.face.toml
fi
export FACE_POLICY=$POLICY

python3 -I - "$KEY" "$GAP_S" "$LOGS" $PUTS <<'EOF'
import base64, json, sys
key, gap, logs, puts = sys.argv[1], float(sys.argv[2]), sys.argv[3], sys.argv[4:]
head = {"zrec": 3, "selectors": [key], "base": "", "captured_at": "2026-11-28T00:00:00Z",
        "excluded": ["@stream", "@state", "@op", "@zk", "@adv"]}
part, last, f, t = 0, None, None, 0
for i, p in enumerate(puts):
    qos, n = p.rsplit(":", 1)
    if qos != last:
        part, last, t = part + 1, qos, 0
        f = open(f"{logs}/puts-{part}.zrec", "w")
        f.write(json.dumps(head) + "\n")
        open(f"{logs}/puts.list", "a").write(f"{logs}/puts-{part}.zrec\n")
    row = {"key": key, "t": round(gap * t * 1e6), "delete": False, "qos_axes": qos,
           "bytes": base64.b64encode(bytes([i]) * int(n)).decode()}
    f.write(json.dumps(row) + "\n")
    f.flush()
    t += 1
EOF

now() { python3 -I -c 'import time; print(f"{time.time():.6f}")'; }
trap '$W down || true' EXIT
$W up "$MEDIUM" bare
sleep "$SETTLE_S"
$W start far bash -c "$ZENCTL echo '$KEY' --no-decode --format ndjson \
  --zenoh-config $WORK/cfg/auth/local.json5 -c tcp/127.0.0.1:$((BASE + 2)) \
  | python3 -I -u $HERE/stamp.py $LOGS/far.jsonl"
sleep 15
start=$(now)
while read -r z; do
  timeout 20m taskset -c "${CORES:-0-1}" "$ZENCTL" replay --i-know --force-base \
    --zenoh-config "$WORK/cfg/auth/rf-node.json5" -c tcp/127.0.0.1:$((BASE + 1)) \
    "$z" </dev/null >>"$LOGS/replay.log" 2>&1
  sleep "$GAP_S"
done <"$LOGS/puts.list"
sleep 15
end=$(now)
python3 -I - "$LOGS/marks.json" "$MEDIUM" "$start" "$end" "${ZIDS:-}" $PUTS <<'EOF'
import json, sys
out, medium, start, end, zids, puts = sys.argv[1], sys.argv[2], float(sys.argv[3]), float(sys.argv[4]), sys.argv[5], sys.argv[6:]
rows = [{"index": i, "qos": p.rsplit(":", 1)[0], "payload": int(p.rsplit(":", 1)[1])} for i, p in enumerate(puts)]
json.dump({"medium": medium, "stack": "bare", "zids": zids, "puts_start": start, "puts_end": end,
           "puts": rows}, open(out, "w"), indent=1)
EOF
echo "done: $LOGS"

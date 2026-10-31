#!/usr/bin/env bash
# Train the compress layer's 1 KB dictionary on a bare run's own SDUs (#989),
# as layer-compress/README.md says: every SDU the relay carried, both
# directions, the 1-byte keep-alives left out. The same file goes to both ends.
#
# Usage: train-dictionary.sh <bare-run-dir> [out.zdict]
set -euo pipefail
HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
RUNDIR=${1:?the log directory of a bare run}
OUT=${2:-$HERE/rf.zdict}
SAMPLES=$(mktemp -d)
trap 'rm -rf "$SAMPLES"' EXIT
python3 -I - "$RUNDIR/air.jsonl" "$SAMPLES" <<'EOF'
import json, sys
n = 0
for line in open(sys.argv[1]):
    r = json.loads(line)
    if r["n"] >= 2:
        open(f"{sys.argv[2]}/{n:06d}.sdu", "wb").write(bytes.fromhex(r["hex"]))
        n += 1
print(f"{n} samples", file=sys.stderr)
EOF
taskset -c 0-1 timeout 120 zstd -q --train "$SAMPLES"/* --maxdict=1024 -o "$OUT"
ls -l "$OUT"

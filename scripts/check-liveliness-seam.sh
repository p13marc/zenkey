#!/usr/bin/env bash
# One seam for liveliness subscribers (#830, core §8.1, 0.37): a liveliness
# GET from a session that holds a liveliness subscriber can complete without
# a token that is live, so every one the runtime, the fleet and the tools
# declare goes through `zenkey::presence::declare_subscriber`, which marks
# its session and makes the runtime refuse a liveliness read on it. A
# `.liveliness()` followed within two lines by `.declare_subscriber(` in a
# crate's sources declares one around the seam, and fails here. Tests may:
# they show the hazard on purpose.
set -euo pipefail
cd "$(dirname "$0")/.."
seam=zenkey/src/presence.rs
bad=""
while IFS= read -r f; do
    [ "$f" = "$seam" ] && continue
    hits=$(awk -v f="$f" '/liveliness\(\)/ { l = NR }
        /declare_subscriber/ { if (l > 0 && NR - l <= 2) print f ":" NR ": " $0 }' "$f")
    [ -n "$hits" ] && bad="$bad$hits"$'\n'
done < <(git ls-files '*/src/*.rs' '*/src/**/*.rs' '*/examples/*.rs' | grep -v '^docs/')
if [ -n "$bad" ]; then
    echo "a liveliness subscriber declared around zenkey::presence::declare_subscriber (#830):" >&2
    printf '%s' "$bad" >&2
    exit 1
fi
echo "liveliness subscribers: every one through zenkey::presence::declare_subscriber"

#!/usr/bin/env bash
# The dispatch dispatches (#209).
#
# `zenctl`'s `run()` matches on the parsed command and calls a `cmd::` function.
# It does not open sessions, build reports, or reach for the fleet engine — it
# had grown five verbs that did all three, which is how `storage list`'s
# coverage join came to exist twice, in two states of honesty.
#
# The rule is mechanical, so this is the gate: `lib.rs` names `zenkey_fleet`
# nowhere. A verb that needs the engine needs a module.
set -euo pipefail

hits=$(grep -n 'zenkey_fleet' zenctl/src/lib.rs || true)
if [ -n "$hits" ]; then
    echo "dispatch seam: zenctl/src/lib.rs reaches for the fleet engine:"
    echo "$hits"
    echo
    echo "A verb that needs zenkey_fleet needs a cmd:: module. The dispatch"
    echo "resolves a Bus, matches, and calls one function (#209)."
    exit 1
fi

# The other half of the same rule: `run()` stays a dispatch, and 600 lines is
# where #209 drew the line. Not a style preference — it is the size at which
# the arms stop being readable as a table of verbs.
lines=$(awk '/^pub async fn run/{f=1} f{n++} f&&/^}$/{print n; exit}' zenctl/src/lib.rs)
if [ "$lines" -gt 600 ]; then
    echo "dispatch seam: run() is $lines lines, over the 600 #209 set."
    echo "The next verb to grow a body belongs in cmd/, not here."
    exit 1
fi

echo "dispatch seam: run() is $lines lines and names no engine (#209)."

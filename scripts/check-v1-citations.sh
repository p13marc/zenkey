#!/usr/bin/env bash
# No zk2 source or report fixture cites v1's RFC set (#925).
#
# v1's RFCs (`RFC 01` to `RFC 13`) left `main` with #925: they are frozen
# on the `v1` branch, about a keyspace zk2 does not use. The zk2 tools cite
# zk2's texts instead: `observer §n` (spec/observer.md), `core §n`
# (spec/core.md), a profile's `§n`, or the tooling guide
# (docs/zk2/tooling-guide.md). A report's `cite` and a note's text reach
# the user, so a v1 citation there sends a zk2 user to the wrong text.
#
# What this reads: every tracked `.rs` file, every `.trycmd` case, and
# every file under a crate's `tests/cmd/` or `tests/fixtures/`, which pin
# what the tools print. `impl/python/` is the independent implementation's
# and is not read.
#
# What it flags: `RFC` followed by a number from 1 to 13, with or without
# a leading zero (`RFC 09`, `RFC 13 §4.4`, `RFC 3`), and the same split
# across a line break (`(RFC` / `/// 09 §5.1`). An IETF RFC is never
# flagged: its number has four digits (`RFC 3339`, `RFC 8259`, `RFC 6901`),
# and a one- or two-digit number followed by another digit is not a match.
#
# Same shape as the other grep gates: exit 1 and every finding on a hit.
set -euo pipefail
cd "$(dirname "$0")/.."

files=$(git ls-files -- '*.rs' '*.trycmd' '*/tests/cmd/*' '*/tests/fixtures/*' \
    | grep -v '^impl/' | sort -u)

findings=$(printf '%s\n' "$files" | xargs -d '\n' awk '
    # `RFC` and a v1 number (1 to 13, a leading zero allowed), not followed
    # by a digit: four-digit IETF numbers never match.
    function v1(s) { return s ~ /RFC (0?[1-9]|1[0-3])([^0-9]|$)/ }
    FNR == 1 { prev = "" }
    {
        if (v1($0)) {
            printf "%s:%d: %s\n", FILENAME, FNR, $0
        } else if (prev ~ /RFC[[:space:]]*$/) {
            # A citation split across lines: the number opens this one,
            # after any comment leader.
            rest = $0
            sub(/^[[:space:]]*((\/\/[\/!]?)|#|\*)?[[:space:]]*/, "", rest)
            if (rest ~ /^(0?[1-9]|1[0-3])([^0-9]|$)/) {
                printf "%s:%d: %s\n", FILENAME, FNR, $0
            }
        }
        prev = $0
    }
' || true)

if [ -n "$findings" ]; then
    echo "v1-citations: these cite v1's RFC set, which is not on main (#925):" >&2
    printf '%s\n' "$findings" >&2
    echo "Cite zk2's home instead: observer §n, core §n, a profile's §n, or the" >&2
    echo "tooling guide (docs/zk2/tooling-guide.md). An IETF RFC (four digits) is fine." >&2
    exit 1
fi
count=$(printf '%s\n' "$files" | grep -c .)
echo "v1-citations: none of the $count source and fixture files cites v1's RFC set."

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

# Prints every v1 citation in the files listed, one per line, in `$1` (a
# file of paths), and returns awk's own status: a failure of awk (a bad
# regex, a file it cannot read) is an error, never "no finding".
scan() {
    local list=$1
    [ -s "$list" ] || return 0
    xargs -d '\n' awk '
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
    ' < "$list"
}

# The scan's own test, run before every real one (review-bot on PR #948):
# planted samples it must flag, and IETF numbers and look-alikes it must
# not, so a regression in its patterns fails the gate instead of passing
# it with nothing checked.
self_test() {
    local dir
    dir=$(mktemp -d)
    trap 'rm -rf "$dir"' RETURN
    cat > "$dir/flag.rs" <<'SAMPLES'
// RFC 09 §5.1 O6
/// see RFC 13 §4.4
let cite = "RFC 3 §2";
// a citation split across lines (RFC
// 05 §2.1)
/// split in a doc block, RFC
/// 13 §1.2
 * a C-style block, RFC
 * 1 §2
SAMPLES
    cat > "$dir/pass.rs" <<'SAMPLES'
// RFC 3339 timestamps, RFC 8259 JSON, RFC 6901 pointers, RFC 4648 base64
// RFC 140 is no v1 chapter; RFC 0 is none either
// an RFC
// 3339 split across lines is still an IETF number
SAMPLES
    printf '%s\n' "$dir/flag.rs" > "$dir/list.flag"
    printf '%s\n' "$dir/pass.rs" > "$dir/list.pass"
    local flagged passed
    flagged=$(scan "$dir/list.flag" | wc -l)
    passed=$(scan "$dir/list.pass" | wc -l)
    if [ "$flagged" -ne 6 ] || [ "$passed" -ne 0 ]; then
        echo "v1-citations: the scan's self-test failed: $flagged of 6 planted citations flagged, $passed false hits." >&2
        exit 2
    fi
}

self_test

list=$(mktemp)
trap 'rm -f "$list"' EXIT
git ls-files -- '*.rs' '*.trycmd' '*/tests/cmd/*' '*/tests/fixtures/*' \
    | { grep -v '^impl/' || true; } | sort -u > "$list"
if [ ! -s "$list" ]; then
    echo "v1-citations: no source or fixture file found to read; the check reads git's tracked files." >&2
    exit 2
fi

rc=0
findings=$(scan "$list") || rc=$?
if [ "$rc" -ne 0 ]; then
    echo "v1-citations: the scan failed (awk exit $rc); nothing was checked." >&2
    exit 2
fi

if [ -n "$findings" ]; then
    echo "v1-citations: these cite v1's RFC set, which is not on main (#925):" >&2
    printf '%s\n' "$findings" >&2
    echo "Cite zk2's home instead: observer §n, core §n, a profile's §n, or the" >&2
    echo "tooling guide (docs/zk2/tooling-guide.md). An IETF RFC (four digits) is fine." >&2
    exit 1
fi
count=$(grep -c . "$list")
echo "v1-citations: none of the $count source and fixture files cites v1's RFC set."

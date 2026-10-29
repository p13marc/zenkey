#!/usr/bin/env bash
# The zk2 spec's version and inventory, stated once (#922).
#
# The core's version and the list of profiles were written out in four
# places, and nothing checked them: by 0.56 the README called the core
# 0.27 and named three of eleven profiles, and spec/README.md called it
# 0.40 and named eight. Now each has one source, and this gate keeps it so:
#
#   * the version is `core.md`'s header (`**Version 0.N**`), and it is the
#     newest entry of spec/CHANGELOG.md (`## 0.N — …`), which records every
#     amendment;
#   * README.md and spec/README.md state no core version: they link the
#     header. "States a core version" is a `0.N` (two parts, an optional
#     `v`: never a crate's `0.20.0` or `0.11.x`) within 30 characters
#     holding no digit after the word `core` or `spec` (`core 0.27`,
#     `[core.md](spec/core.md), 0.27`, `spec 0.56`), or within 6
#     characters holding no letter or digit after `version`
#     (`version 0.40`, `Version **0.56**`). Line breaks count as
#     characters, so a version wrapped onto the next line is still found;
#   * the observer text keeps the same rule (#925): `observer.md`'s header
#     (`| **Text version** | 0.N …`) is the newest entry of
#     spec/observer-CHANGELOG.md (`## 0.N — …`), and README.md and
#     spec/README.md state no observer version: a `0.N` within 30
#     characters holding no digit after the word `observer`
#     (`observer 0.1`, `[observer.md](spec/observer.md) text 0.1`);
#   * profiles are listed once, in spec/profiles/README.md's index: every
#     `spec/profiles/<name>/v<N>.md` has one row whose first cell is
#     [`<name>.v<N>`](<name>/v<N>.md), and every row names a text that
#     exists. The index holds two tables since #938, the profiles in use
#     and "Experimental: no adopter yet", and the rows of both count.
#     Who uses each, and the status that follows, are
#     zenkey-model/tests/profiles.rs's to check, since they read the
#     contracts.
#
# Same shape as check-rfc-status.sh, which keeps v1's RFC headers.
set -euo pipefail
cd "$(dirname "$0")/.."

core=spec/core.md
chlog=spec/CHANGELOG.md
observer=spec/observer.md
ochlog=spec/observer-CHANGELOG.md
index=spec/profiles/README.md
fail=0

# Every input must exist: a renamed or missing file would otherwise read as
# "nothing stated" and pass (review-bot on PR #943).
for f in "$core" "$chlog" "$observer" "$ochlog" "$index" README.md spec/README.md; do
    if [ ! -f "$f" ]; then
        echo "spec-status: $f is missing; the check reads it." >&2
        exit 2
    fi
done

# --- 1. core.md's header names the newest amendment ------------------------
# The header: everything above the first `## ` heading.
header_version=$(awk '/^## / { exit } { print }' "$core" \
    | grep -oE '^\*\*Version 0\.[0-9]+\*\*' | head -1 | grep -oE '0\.[0-9]+' || true)
newest=$(grep -oE '^## 0\.[0-9]+ ' "$chlog" | grep -oE '0\.[0-9]+' \
    | sort -t. -k2,2n | tail -1 || true)
if [ -z "$newest" ]; then
    echo "spec-status: $chlog has no '## 0.N — …' entry." >&2
    fail=1
elif [ -z "$header_version" ]; then
    echo "spec-status: $core's header states no '**Version 0.N**' line above its first section." >&2
    fail=1
elif [ "$header_version" != "$newest" ]; then
    echo "spec-status: $core's header says version $header_version; $chlog's newest entry is $newest." >&2
    echo "  An amendment moves both: its entry in the changelog, and the header's version." >&2
    fail=1
fi

# --- 1b. observer.md's header names its newest version (#925) ---------------
# The header: everything above the first `## ` heading, as for the core; its
# version is the `| **Text version** | 0.N` row of its table, as a profile's.
obs_version=$(awk '/^## / { exit } { print }' "$observer" \
    | grep -oE '^\| \*\*Text version\*\* \| 0\.[0-9]+' | head -1 | grep -oE '0\.[0-9]+' || true)
obs_newest=$(grep -oE '^## 0\.[0-9]+ ' "$ochlog" | grep -oE '0\.[0-9]+' \
    | sort -t. -k2,2n | tail -1 || true)
if [ -z "$obs_newest" ]; then
    echo "spec-status: $ochlog has no '## 0.N — …' entry." >&2
    fail=1
elif [ -z "$obs_version" ]; then
    echo "spec-status: $observer's header states no '| **Text version** | 0.N' row above its first section." >&2
    fail=1
elif [ "$obs_version" != "$obs_newest" ]; then
    echo "spec-status: $observer's header says text version $obs_version; $ochlog's newest entry is $obs_newest." >&2
    echo "  A change moves both: its entry in the changelog, and the header's version." >&2
    fail=1
fi

# --- 2. the front doors state no core version ------------------------------
# grep -z reads each file as one record, so `[^0-9]` spans line breaks; -b
# gives each match's byte offset, turned into a line number below.
two_part='v?0\.[0-9]+([^.0-9x]|\.[^0-9x])'
pattern="(\<([Cc]ore|[Ss]pec)[^0-9]{0,30}|\<[Vv]ersion[^0-9A-Za-z]{0,6})${two_part}"
stated=0
for doc in README.md spec/README.md; do
    # grep's own status, apart from tr's: 1 is no match, above 1 an error
    # (an unreadable file or a bad pattern), never a silent pass.
    rc=0
    hits=$(grep -zobE "$pattern" "$doc" | tr '\0\n' '\n '; exit "${PIPESTATUS[0]}") || rc=$?
    if [ "$rc" -gt 1 ]; then
        echo "spec-status: grep could not read $doc (exit $rc)." >&2
        exit 2
    fi
    [ -z "$hits" ] && continue
    while IFS= read -r hit; do
        [ -z "$hit" ] && continue
        offset=${hit%%:*}
        text=${hit#*:}
        # `spec/observer.md) text 0.1` names the observer's version, which
        # 2b reports: the word `observer` between `spec` and the number.
        case "$text" in *[Oo]bserver*) continue ;; esac
        line=$(( $(head -c "$offset" "$doc" | wc -l) + 1 ))
        echo "spec-status: $doc:$line states a core version: '$text'" >&2
        stated=1
    done <<< "$hits"
done
if [ "$stated" -ne 0 ]; then
    echo "  The version lives in $core's header alone: link the header and state no number." >&2
    fail=1
fi

# --- 2b. the front doors state no observer version (#925) ------------------
obs_pattern="\<[Oo]bserver[^0-9]{0,30}${two_part}"
obs_stated=0
for doc in README.md spec/README.md; do
    rc=0
    hits=$(grep -zobE "$obs_pattern" "$doc" | tr '\0\n' '\n '; exit "${PIPESTATUS[0]}") || rc=$?
    if [ "$rc" -gt 1 ]; then
        echo "spec-status: grep could not read $doc (exit $rc)." >&2
        exit 2
    fi
    [ -z "$hits" ] && continue
    while IFS= read -r hit; do
        [ -z "$hit" ] && continue
        offset=${hit%%:*}
        text=${hit#*:}
        line=$(( $(head -c "$offset" "$doc" | wc -l) + 1 ))
        echo "spec-status: $doc:$line states an observer version: '$text'" >&2
        obs_stated=1
    done <<< "$hits"
done
if [ "$obs_stated" -ne 0 ]; then
    echo "  The version lives in $observer's header alone: link the header and state no number." >&2
    fail=1
fi

# --- 3. the profiles index lists every profile text, and only those ---------
# The index: the rows of every table under `## Index`, down to the next
# `## ` heading, a `### ` subsection's included. Each table's header row
# (the row just above a separator) and its separator are skipped.
rows=$(awk '
    function flush() { if (held != "") print held; held = "" }
    /^## / { flush(); inside = ($0 == "## Index"); next }
    !inside { next }
    /^\|/ {
        if ($0 ~ /^\|[-:| ]+\|[[:space:]]*$/) { held = ""; next }
        flush(); held = $0; next
    }
    { flush() }
    END { flush() }
' "$index")
if [ -z "$rows" ]; then
    echo "spec-status: $index has no table under '## Index'." >&2
    fail=1
fi
# A first cell: [`<name>.v<N>`](<link>).
cell='^\[`([a-z0-9_]+)\.v([0-9]+)`\]\(([^)]*)\)$'
listed=""
while IFS= read -r row; do
    [ -z "$row" ] && continue
    first=$(printf '%s\n' "$row" | awk -F'|' '{ print $2 }' | sed 's/^ *//; s/ *$//')
    if ! [[ "$first" =~ $cell ]]; then
        echo "spec-status: $index's index has a row whose first cell is not [\`<name>.v<N>\`](<name>/v<N>.md): '$first'" >&2
        fail=1
        continue
    fi
    name=${BASH_REMATCH[1]}
    major=${BASH_REMATCH[2]}
    link=${BASH_REMATCH[3]}
    if [ "$link" != "$name/v$major.md" ]; then
        echo "spec-status: $index lists $name.v$major with the link '$link'; its text is $name/v$major.md." >&2
        fail=1
    fi
    if [ ! -f "spec/profiles/$name/v$major.md" ]; then
        echo "spec-status: $index lists $name.v$major, and spec/profiles/$name/v$major.md does not exist." >&2
        fail=1
    fi
    case " $listed " in
        *" $name/v$major.md "*)
            echo "spec-status: $index lists $name.v$major twice; a profile has one row." >&2
            fail=1
            ;;
    esac
    listed="$listed $name/v$major.md"
done <<< "$rows"

for text in spec/profiles/*/v[0-9]*.md; do
    [ -f "$text" ] || continue
    rel=${text#spec/profiles/}
    case " $listed " in
        *" $rel "*) ;;
        *)
            echo "spec-status: $text exists, and $index's index has no row for it." >&2
            fail=1
            ;;
    esac
done

if [ "$fail" -ne 0 ]; then
    exit 1
fi
count=$(printf '%s\n' "$listed" | wc -w)
echo "spec-status: core.md's header names the newest amendment, $newest, and observer.md's its newest text, $obs_newest; README.md and spec/README.md state neither version; the profiles index lists all $count profile texts."

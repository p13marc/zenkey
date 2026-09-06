#!/usr/bin/env bash
# Status lines, mechanized (RFC CHANGELOG discipline, v1.25 S6).
#
# A chapter header's "amended in" list and the changelog are the same record
# written twice, and v1.22's errata sweep showed what happens while nothing
# checks them against each other: 08's header omitted v1.20 — MUSTs a reader
# could not discover from the header. Two conventions make the record
# parseable (stated in rfcs/CHANGELOG.md's preamble):
#
#   * every changelog entry ends with `> *Amends: NN, NN.*` (or
#     `> *Amends: none — reason.*`; an explanation may follow the list after
#     an em dash) — the chapters that entry substantively amended;
#   * every amended chapter's header paragraph carries exactly one
#     `*amended in v1.X, … — see [CHANGELOG.md](CHANGELOG.md)*` clause
#     (or `*created in …*` for a chapter born as an amendment), naming
#     exactly the versions whose Amends line names it.
#
# Same shape as the other grep gates: the rule is parseable, so it is
# enforced.
set -euo pipefail
cd "$(dirname "$0")/.."

chlog=rfcs/CHANGELOG.md
fail=0

# Every entry carries its Amends line — v1.0 included.
entries=$(grep -c '^> \*\*v1\.[0-9]* ' "$chlog")
amends=$(grep -c '^> \*Amends: ' "$chlog")
if [ "$entries" -ne "$amends" ]; then
    echo "rfc-status: $chlog has $entries entries but $amends '*Amends: …*' lines." >&2
    echo "Every entry ends with one — 'none' if it amended no chapter's text." >&2
    exit 1
fi

# The ledger, flattened to "chapter version" pairs.
pairs=$(awk '
    /^> \*\*v1\.[0-9]+ / { match($0, /v1\.[0-9]+/); ver = substr($0, RSTART, RLENGTH) }
    /^> \*Amends: / {
        line = $0
        sub(/^> \*Amends: /, "", line)
        sub(/\.\*[[:space:]]*$/, "", line)
        sub(/ —.*/, "", line)
        if (line == "none") next
        n = split(line, ch, /,[[:space:]]*/)
        for (i = 1; i <= n; i++) {
            if (ch[i] !~ /^(0[1-9]|1[0-9])$/) { printf "BAD %s %s\n", ver, ch[i] }
            else printf "%s %s\n", ch[i], ver
        }
    }
' "$chlog")

if printf '%s\n' "$pairs" | grep -q '^BAD'; then
    echo "rfc-status: unparseable chapter number in an Amends line:" >&2
    printf '%s\n' "$pairs" | grep '^BAD' >&2
    exit 1
fi

# Every chapter the ledger names exists.
for nn in $(printf '%s\n' "$pairs" | awk '{ print $1 }' | sort -u); do
    if ! ls rfcs/"$nn"-*.md >/dev/null 2>&1; then
        echo "rfc-status: an Amends line names chapter $nn, and rfcs/$nn-*.md does not exist." >&2
        fail=1
    fi
done

for f in rfcs/[0-9][0-9]-*.md; do
    nn=$(basename "$f" | cut -c1-2)
    [ "$nn" = "00" ] && continue

    expected=$(printf '%s\n' "$pairs" | awk -v nn="$nn" '$1 == nn { print $2 }' | sort -u -t. -k2,2n)

    # The header paragraph: the **Status: line through the first blank line.
    header=$(awk '/^\*\*Status:/ { found = 1 } found { if (!NF) exit; print }' "$f" | tr '\n' ' ')
    if [ -z "$header" ]; then
        echo "rfc-status: $f has no '**Status:' header paragraph." >&2
        fail=1
        continue
    fi

    clause=$(printf '%s\n' "$header" | grep -oE '\*(amended|created) in [^*]*\*' || true)
    nclauses=$(printf '%s' "$clause" | grep -c 'in' || true)
    if [ "$nclauses" -gt 1 ]; then
        echo "rfc-status: $f has $nclauses amended/created clauses; the record lives in exactly one." >&2
        fail=1
        continue
    fi
    if [ -n "$clause" ] && ! printf '%s' "$clause" | grep -qF 'see [CHANGELOG.md](CHANGELOG.md)'; then
        echo "rfc-status: $f's clause does not point at CHANGELOG.md:" >&2
        echo "  $clause" >&2
        fail=1
    fi
    actual=$(printf '%s\n' "$clause" | grep -oE 'v1\.[0-9]+' | sort -u -t. -k2,2n || true)

    if [ "$expected" != "$actual" ]; then
        echo "rfc-status: $f disagrees with $chlog's Amends record." >&2
        echo "  header says:    ${actual:-'(no amendments)'}" | tr '\n' ' ' >&2; echo >&2
        echo "  changelog says: ${expected:-'(no amendments)'}" | tr '\n' ' ' >&2; echo >&2
        echo "  Fix whichever is lying; the Amends lines are the ledger." >&2
        fail=1
    fi
done

# The set's version is stated in three places outside the ledger, and each
# has drifted at least once (CLAUDE.md and README.md sat at v1.28 through two
# amendments, #420). The newest entry is the truth; the three must name it.
newest=$(grep -oE '^> \*\*v1\.[0-9]+ ' "$chlog" | head -1 | grep -oE 'v1\.[0-9]+')
for spec in 'rfcs/00-index.md|\*\*Status: v1\.[0-9]+\*\*' 'CLAUDE.md|normative RFC set\*\* \(v1\.[0-9]+;' 'README.md|convention is at \*\*v1\.[0-9]+ '; do
    file=${spec%%|*}
    pattern=${spec#*|}
    stated=$(grep -oE "$pattern" "$file" | head -1 | grep -oE 'v1\.[0-9]+' || true)
    if [ -z "$stated" ]; then
        echo "rfc-status: $file no longer states the set's version where this gate looks ($pattern)." >&2
        fail=1
    elif [ "$stated" != "$newest" ]; then
        echo "rfc-status: $file says the set is at $stated; $chlog's newest entry is $newest." >&2
        fail=1
    fi
done

if [ "$fail" -ne 0 ]; then
    exit 1
fi
echo "rfc-status: every chapter header agrees with the CHANGELOG's Amends record, and the set's version is stated as $newest throughout."

#!/usr/bin/env bash
# One seam for liveliness subscribers and GETs (#830, core §8.1, 0.37): a
# liveliness GET from a session that holds a liveliness subscriber can
# complete without a token that is live, so every one the runtime, the fleet
# and the tools declare goes through `zenkey::presence::declare_subscriber`,
# which marks its session and makes the runtime refuse a liveliness read on
# it. Every liveliness GET goes through `zenkey::presence` too (#849, core
# 0.38): its reads check the session before and after, and each GET is
# counted per session (`zenkey::presence::liveliness_gets`), which is what
# `spec/scenarios/presence.md` §9 observes of a reader.
#
# Outside the seam, every `.liveliness()` in a crate's sources and examples
# (comments aside) must be followed by `.declare_token(`, across lines if
# need be; anything else — `.get(`, `.declare_subscriber(`, an advanced
# subscriber, or the handle bound to a name to be used later — fails here.
# Tests may: they show the hazard on purpose.
set -euo pipefail
cd "$(dirname "$0")/.."
seam=zenkey/src/presence.rs
bad=""
while IFS= read -r f; do
    [ "$f" = "$seam" ] && continue
    hits=$(sed -E 's#^[[:space:]]*//.*$##' "$f" \
        | grep -Pzo '\.liveliness\(\)\s*(\.\s*\w+)?' \
        | tr -d ' \t\n' | tr '\0' '\n' \
        | grep -v -x -E '\.liveliness\(\)\.declare_token' || true)
    [ -n "$hits" ] && bad="$bad$(printf '%s\n' "$hits" | sed "s|^|$f: |")"$'\n'
done < <(git ls-files '*.rs' | grep -v -E '^(docs|impl)/|/tests/')
if [ -n "$bad" ]; then
    echo "a liveliness() use outside zenkey::presence other than declare_token (#830, #849):" >&2
    printf '%s' "$bad" >&2
    exit 1
fi
echo "liveliness subscribers and GETs: every one through zenkey::presence"

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

# A tool's watch session (#875, core §8.1, 0.42) runs in no namespace and
# watches the deployment's wire keys, its subscribers without history. Its
# type, `zenkey_fleet::WatchSession`, is the one place that knows the
# namespace: every selector it declares carries it, every key it hears has
# it stripped (a key outside it refused), and its barrier asks it on the
# wire. So outside the runtime (`zenkey/src`, whose owner-side subscribers
# run on a session the application hands in) and that type
# (`zenkey-fleet/src/bus/watch.rs`):
# - no liveliness subscriber is declared through
#   `zenkey::presence::declare_subscriber`, and no barrier is made through
#   `zenkey::presence`'s `settle`, `settle_at`, `Barrier::new` or
#   `Barrier::at`: a watch subscriber goes through `WatchSession::subscribe`,
#   its barrier through `WatchSession::barrier`, so none is declared without
#   the namespace on its wire key;
# - a `WatchSession` is made only by `zenkey_fleet::open_watch`
#   (`zenkey-fleet/src/bus/session.rs`), whose namespace is a required
#   argument.
# Tests may: they measure zenoh's own behaviour, and build sessions by hand.
in_type=zenkey-fleet/src/bus/watch.rs
opener=zenkey-fleet/src/bus/session.rs
wbad=""
while IFS= read -r f; do
    case "$f" in zenkey/src/*|"$in_type") continue ;; esac
    code=$(sed -E 's#^[[:space:]]*//.*$##' "$f")
    hits=$(printf '%s\n' "$code" \
        | grep -n -E 'presence::(declare_subscriber|settle|settle_at)[[:space:]]*\(|Barrier::(new|at)[[:space:]]*\(' || true)
    [ -n "$hits" ] && wbad="$wbad$(printf '%s\n' "$hits" | sed "s|^|$f:|")"$'\n'
    if [ "$f" != "$opener" ]; then
        hits=$(printf '%s\n' "$code" | grep -n -E 'WatchSession::new[[:space:]]*\(' || true)
        [ -n "$hits" ] && wbad="$wbad$(printf '%s\n' "$hits" | sed "s|^|$f:|")"$'\n'
    fi
done < <(git ls-files '*.rs' | grep -v -E '^(docs|impl)/|/tests/|/app_tests/')
if [ -n "$wbad" ]; then
    echo "a watch subscriber, barrier or watch session outside zenkey_fleet::WatchSession (#875):" >&2
    printf '%s' "$wbad" >&2
    exit 1
fi
echo "liveliness subscribers and GETs: every one through zenkey::presence"
echo "the tools' watch subscribers: every one through zenkey_fleet::WatchSession, on the wire"

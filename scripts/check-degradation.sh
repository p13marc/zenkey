#!/usr/bin/env bash
# Degrading past a missing registry goes through one door (#210).
#
# A verb that slices only *enrich* may continue without them; a verb they
# *determine* must not. The first spells it `slices_optional()`, which announces
# the degradation once per invocation; the second spells it `slice_set().await?`
# and fails.
#
# What this refuses is the third spelling: `.ok()` or `.unwrap_or_default()`
# straight off `slice_set()`, which degrades in silence. There were ten of
# those, each with its own comment explaining itself, and the comments were the
# only record of the rule.
#
# **Multiline-aware on purpose.** The acceptance criterion #210 was written with
# — `grep -c 'slice_set().await'` — passes straight over `service info`, whose
# call is split across lines by rustfmt. A gate that cannot see a live site is
# worse than no gate, because it reports a clean sweep.
set -euo pipefail

# Join the whole tree into one stream with newlines collapsed, so a call broken
# across lines reads the same as one that is not. `-z` makes grep treat the
# input as a single record; `tr` puts the line numbers out of reach, so the
# report below re-finds offenders per file.
bad=0
for f in $(find zenctl/src -name '*.rs' ! -name 'bus.rs'); do
    flat=$(tr '\n' ' ' < "$f" | sed 's/  */ /g')
    if echo "$flat" | grep -q '\.slice_set() \?\. \?await \?\. \?\(ok\|unwrap_or_default\)()'; then
        echo "degradation seam: $f degrades off slice_set() directly:"
        grep -n 'slice_set()' "$f"
        bad=1
    fi
done

if [ "$bad" -ne 0 ]; then
    echo
    echo "Use Bus::slices_optional() -> Result<Option<SliceSet>>: it announces the"
    echo "degradation once (RFC 09 §5.1 O4) and keeps a source the *user* named"
    echo "(--registry /typo) fatal, which a bare .ok() cannot (#210)."
    exit 1
fi

echo "degradation seam: every degradation goes through slices_optional (#210)."

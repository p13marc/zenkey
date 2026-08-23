#!/usr/bin/env bash
# The spacing grid, gated (#192).
#
# zengui's 8pt scale was defined, tested — and optional: ~80 sites said
# `.padding(4)` or `.spacing(2)` by taste, and density (Ctrl+Shift+D) can only
# multiply what goes through the tokens. One rule, four numbers, no
# exceptions: SM inside a card, MD between cards / dock padding, LG between
# sections, XL page-level — spelled `sp.*` (the density-resolved `Spacing`)
# inside a workspace dock and `space::*` in the one-row window chrome.
#
# Same shape as the type-scale gate (#191): the rule is greppable, so it is
# enforced. A `.padding(` / `.spacing(` line must reference the scale —
# `space::` / `sp.` — or `Padding::ZERO`, which is the *absence* of spacing,
# not a fifth number.
set -euo pipefail

# tokens.rs is the file that defines the scale; its doc comments spell the
# bare forms this gate forbids, on purpose.
bad=$(grep -rnE '\.(padding|spacing)\(' zengui/src/view zengui/src/app.rs --include='*.rs' \
    | grep -v '^zengui/src/view/tokens\.rs:' \
    | grep -vE '\.(padding|spacing)\(([^)]*\b(space::|sp\.|Padding::ZERO))' || true)
if [ -n "$bad" ]; then
    echo "spacing: raw numeric .padding()/.spacing() — use the grid (#192):"
    echo "$bad"
    echo
    echo "The grid is assigned by role, not by taste: SM inside a card, MD"
    echo "between cards / dock padding, LG between sections, XL page-level,"
    echo "XS for tight icon/label gaps. Inside a dock, take the resolved"
    echo "Spacing (sp.sm, ...) so density can act; in the window chrome, the"
    echo "space:: constants. Pick the role; the number follows."
    exit 1
fi
echo "spacing: every padding and spacing is on the grid (#192)."

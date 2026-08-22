#!/usr/bin/env bash
# The type scale, gated (#191).
#
# zengui's five-step scale was defined, tested for monotonicity — and unused:
# 137 sites said CAPTION and one said TITLE, so the app read as a wall of 12px.
# The fix was to assign sizes by *role* through the constructors in `view/kit.rs`
# (`title`/`section`/`emphasis`/`body`/`caption`), and this gate is what keeps
# the next site honest, the same way "no raw Color outside theme/kit" already
# is a rule a grep can enforce:
#
#   1. `.size(` outside `kit.rs`/`tokens.rs` must take a `font::` constant —
#      a text_input or pick_list may size itself, but only off the scale.
#   2. A bare `text(` must not appear outside `kit.rs` — every piece of text
#      names its role, and the size follows from that.
set -euo pipefail

fail=0

# 1. `.size(<arg>)` with anything but a `font::` constant. The empty-argument
# form (`bounds.size()`) is geometry, not typography, and stays out of scope.
bad_size=$(grep -rn '\.size([^)]' zengui/src --include='*.rs' \
    | grep -vE '^zengui/src/view/(kit|tokens)\.rs:' \
    | grep -vE '\.size\((tokens::)?font::' || true)
if [ -n "$bad_size" ]; then
    echo "type scale: .size() outside kit.rs/tokens.rs must take a font:: constant:"
    echo "$bad_size"
    fail=1
fi

# 2. A bare `text(` constructor. `.text(` (the theme's color accessor) and
# `fn text(` (its definition) are not the widget; everything else is.
bad_text=$(grep -rnE '(^|[^.[:alnum:]_])text\(' zengui/src --include='*.rs' \
    | grep -v '^zengui/src/view/kit\.rs:' \
    | grep -vE 'fn text\(' || true)
if [ -n "$bad_text" ]; then
    echo "type scale: bare text( outside kit.rs — use kit::{title, section, emphasis, body, caption}:"
    echo "$bad_text"
    fail=1
fi

if [ "$fail" -ne 0 ]; then
    echo
    echo "The scale is assigned by role, not by taste (#191): TITLE is the"
    echo "subject, SECTION a dock or pane header, EMPHASIS a card title or"
    echo "stat, BODY prose, CAPTION metadata. Pick the role; the size follows."
    exit 1
fi
echo "type scale: every text names its role (#191)."

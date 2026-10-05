#!/usr/bin/env bash
# The type scale, gated (#191).
#
# zengui's five-step scale was defined, tested for monotonicity — and unused:
# 137 sites said CAPTION and one said TITLE, so the app read as a wall of 12px.
# The fix was to assign sizes by *role* through the constructors in `view/kit/`
# (`title`/`section`/`emphasis`/`body`/`caption`), and this gate is what keeps
# the next site honest, the same way "no raw Color outside theme/kit" already
# is a rule a grep can enforce:
#
#   1. `.size(` outside `kit/`/`tokens.rs` must take a `font::` constant —
#      a text_input or pick_list may size itself, but only off the scale.
#   2. A bare `text(` must not appear outside `kit/` — every piece of text
#      names its role, and the size follows from that.
#   3. A `.font(` outside kit/tokens/fonts names a `face::` constant (#533).
#   4. No literal `Pixels(<number>)` — a canvas's text is on the scale too.
#   5. `rich_text(`/`span(` only in kit.
set -euo pipefail

fail=0

# 1. `.size(<arg>)` with anything but a `font::` constant. The empty-argument
# form (`bounds.size()`) is geometry, not typography, and stays out of scope.
bad_size=$(grep -rn '\.size([^)]' zengui/src --include='*.rs' \
    | grep -vE '^zengui/src/view/(kit/|(kit|tokens)\.rs:)' \
    | grep -vE '\.size\((tokens::)?font::' || true)
if [ -n "$bad_size" ]; then
    echo "type scale: .size() outside kit/ or tokens.rs must take a font:: constant:"
    echo "$bad_size"
    fail=1
fi

# 2. A bare `text(` constructor. `.text(` (the theme's color accessor) and
# `fn text(` (its definition) are not the widget; everything else is.
bad_text=$(grep -rnE '(^|[^.[:alnum:]_])text\(' zengui/src --include='*.rs' \
    | grep -vE '^zengui/src/view/kit(\.rs:|/)' \
    | grep -vE 'fn text\(' || true)
if [ -n "$bad_text" ]; then
    echo "type scale: bare text( outside kit/ — use kit::{title, section, emphasis, body, caption}:"
    echo "$bad_text"
    fail=1
fi

# 3. A face (#533): `.font(` outside kit/tokens/fonts must name a `face::`
# constant — the bundled families, by role. `Font::MONOSPACE` and
# `Font::with_name` would reach past the bundle to whatever the host has.
bad_font=$(grep -rn '\.font(' zengui/src --include='*.rs' \
    | grep -vE '^zengui/src/view/(kit/|(kit|tokens|fonts)\.rs:)' \
    | grep -v 'face::' || true)
bad_face=$(grep -rnE 'Font::(MONOSPACE|with_name|DEFAULT)' zengui/src --include='*.rs' \
    | grep -vE '^zengui/src/view/(kit/|(kit|tokens|fonts)\.rs:)' || true)
if [ -n "$bad_font$bad_face" ]; then
    echo "type scale: a font outside kit/tokens/fonts must be a face:: constant (#533):"
    echo "$bad_font"
    echo "$bad_face"
    fail=1
fi

# 4. A literal pixel size (#533). A canvas draws its own text with
# `size: Pixels(..)`, which rule 1 cannot see; it takes a `font::` constant
# like everything else.
bad_pixels=$(grep -rnE 'Pixels\([0-9]' zengui/src --include='*.rs' \
    | grep -vE '^zengui/src/view/(kit/|(kit|tokens|fonts)\.rs:)' || true)
if [ -n "$bad_pixels" ]; then
    echo "type scale: a literal Pixels(..) outside kit/tokens/fonts — use Pixels(font::X):"
    echo "$bad_pixels"
    fail=1
fi

# 5. Rich text (#533). `rich_text(`/`span(` build text the role constructors
# never see — and that `iced_test` cannot find, since a rich text reports no
# text to `operate`. They live in kit, behind a constructor that does.
bad_rich=$(grep -rnE '(^|[^.[:alnum:]_])(rich_text|span)\(' zengui/src --include='*.rs' \
    | grep -vE '^zengui/src/view/kit(\.rs:|/)' || true)
if [ -n "$bad_rich" ]; then
    echo "type scale: rich_text/span outside kit/:"
    echo "$bad_rich"
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

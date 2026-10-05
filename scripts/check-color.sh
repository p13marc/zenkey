#!/usr/bin/env bash
# Colour comes from the theme (#534).
#
# zengui's theme.rs said a CI grep forbade raw colours outside theme/kit from
# day one — and none existed; the rule held by convention until #534 made it
# a gate, the same shape as the type scale (#191), the interactive seam (#193)
# and the spacing grid (#192):
#
#   1. No colour is constructed outside `view/theme.rs`. `kit/` may say
#      `Color::TRANSPARENT` (an absent fill is not a colour), nothing else.
#   1b. No `Border {` or `Shadow {` outside kit/theme (#537): frames are
#       kit's.
#   2. Nothing outside theme.rs reaches into iced's palette or builds an iced
#      theme: every colour is a `colors(theme).<role>()`, so a view names a
#      role and the theme decides the colour.
set -euo pipefail

fail=0

# 1. A constructed colour.
bad_color=$(grep -rnE 'Color::(from_rgb|from_rgba|from_rgb8|from_rgba8|from_linear_rgba|new|WHITE|BLACK)\b|color!\(|Color \{' \
    zengui/src --include='*.rs' \
    | grep -v '^zengui/src/view/theme\.rs:' || true)
if [ -n "$bad_color" ]; then
    echo "colour: a colour constructed outside view/theme.rs:"
    echo "$bad_color"
    fail=1
fi
bad_transparent=$(grep -rn 'Color::TRANSPARENT' zengui/src --include='*.rs' \
    | grep -vE '^zengui/src/view/(kit/|(theme|kit)\.rs:)' || true)
if [ -n "$bad_transparent" ]; then
    echo "colour: Color::TRANSPARENT outside theme/kit — a view asks kit for the widget:"
    echo "$bad_transparent"
    fail=1
fi

# 1b. A frame or a shadow (#537): a border carries a colour and a radius, a
# shadow a colour and a lift — both are kit's (the dock frame, the modal, the
# chip), so a view asks kit for the widget rather than drawing its own edge.
bad_frame=$(grep -rnE '(^|[^[:alnum:]_])(iced::)?(Border|Shadow) \{' zengui/src --include='*.rs' \
    | grep -vE '^zengui/src/view/(kit/|(theme|kit)\.rs:)' || true)
if [ -n "$bad_frame" ]; then
    echo "colour: a Border/Shadow drawn outside kit/theme — ask kit for the frame:"
    echo "$bad_frame"
    fail=1
fi

# 2. A reach past the roles.
bad_palette=$(grep -rnE 'extended_palette\(|\.palette\(\)|Theme::custom|iced::Theme::(Light|Dark)\b|Theme::(Light|Dark)\b' \
    zengui/src --include='*.rs' \
    | grep -v '^zengui/src/view/theme\.rs:' || true)
if [ -n "$bad_palette" ]; then
    echo "colour: iced's palette or theme named outside view/theme.rs — use colors(theme).<role>():"
    echo "$bad_palette"
    fail=1
fi

if [ "$fail" -ne 0 ]; then
    echo
    echo "A view names a role — text_muted, tone(Tone::Caution), selected —"
    echo "and the theme decides the colour (#534). That is what lets both"
    echo "themes be tested for contrast in one place."
    exit 1
fi
echo "colour: every colour is the theme's (#534)."

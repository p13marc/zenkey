#!/usr/bin/env bash
# Interactive elements route through kit:: (#193).
#
# Every button, input, picker, checkbox and slider in zengui is built by a
# constructor in `view/kit.rs` — that is the one place the `button::Status`
# set (Active, Hovered, Pressed, Disabled) is handled, and the one place
# keyboard focus is styled (`kit::focus_ring`: a ring, never a fill change).
# A raw widget constructor at a call site would re-open the per-site styling
# this closed: hover feedback that exists on one list and not the next, a
# focus state that changes a fill, a disabled state nobody drew.
#
# Same shape as the type-scale gate (#191): the rule is greppable, so it is
# enforced.
set -euo pipefail

fail=0

# A bare or `iced::widget::`-qualified interactive constructor outside kit.rs.
# `button::Status` / `text_input::Style` are the *modules* (kit's own
# vocabulary) and carry no `(`; only the constructor calls are flagged.
bad=$(grep -rnE '(^|[^.[:alnum:]_])((iced::)?widget::)?(button|text_input|pick_list|checkbox|slider|mouse_area)\(' \
    zengui/src --include='*.rs' \
    | grep -v '^zengui/src/view/kit\.rs:' || true)
if [ -n "$bad" ]; then
    echo "interactive: raw widget constructor outside kit.rs — use"
    echo "kit::{action, link, row_button, input, picker, check, scrub}:"
    echo "$bad"
    fail=1
fi

if [ "$fail" -ne 0 ]; then
    echo
    echo "The four button::Status variants and the focus ring are handled"
    echo "once, in view/kit.rs (#193). Pick the constructor; the states"
    echo "follow."
    exit 1
fi
echo "interactive: every interactive element routes through kit:: (#193)."

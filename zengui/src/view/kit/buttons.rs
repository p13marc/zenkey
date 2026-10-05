//! Every interactive constructor (#193): buttons by rank (#535), links,
//! rows, inputs, pickers, checkboxes, the scrubber, icon buttons, tooltips.

use iced::widget::{
    Button, Checkbox, PickList, Slider, TextInput, button, container, row, tooltip,
};
use iced::{Border, Color, Element, Length};

use super::caption;
use super::icon::{Icon, icon};
use crate::view::theme::colors;
use crate::view::tokens::{face, font, radius, space, stroke};

// ---------------------------------------------------------------------------
// Interactive constructors (#193)
// ---------------------------------------------------------------------------
//
// Every interactive element in zengui is built by one of the constructors
// below, so the full `button::Status` set — Active, Hovered, Pressed,
// Disabled — is handled once, here, instead of per call site. Keyboard focus
// is always [`focus_ring`]: a ring, never a fill change, because a fill
// change is colour-only by definition and fails the invariant it is meant to
// signal. The `check-interactive.sh` gate keeps the next call site honest.

/// The keyboard-focus ring (#193): 2px of `primary()`.
///
/// Iced draws borders just inside the widget's bounds, so the ring's 1px
/// offset comes from the padding every constructor here keeps between the
/// ring and its content — the ring never touches the glyphs it outlines.
pub fn focus_ring(theme: &iced::Theme) -> Border {
    Border {
        color: colors(theme).primary(),
        width: stroke::FOCUS,
        radius: radius::CONTROL.into(),
    }
}

/// The four button ranks (#535). Until then every action was primary-filled
/// — "run doctor" and "clear" looked the same, so nothing told the eye which
/// one a surface was *for*.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Rank {
    /// The surface's one verb: filled primary.
    Primary,
    /// An ordinary action: the panel with a hairline.
    Secondary,
    /// A quiet one — a toggle, a clear, an export: no fill until hovered.
    Ghost,
    /// The irreversible wire act: an outline in the danger hue.
    Danger,
}

fn ranked<'a, M: 'a>(content: impl Into<Element<'a, M>>, rank: Rank) -> Button<'a, M> {
    button(content.into()).style(move |theme: &iced::Theme, status| {
        let c = colors(theme);
        let (fill, text, edge) = match rank {
            Rank::Primary => (Some(c.primary()), c.on_primary(), Color::TRANSPARENT),
            Rank::Secondary => (Some(c.panel()), c.text(), c.line()),
            Rank::Ghost => (None, c.text_muted(), Color::TRANSPARENT),
            Rank::Danger => (
                None,
                c.danger(),
                c.tone_border(crate::view::theme::Tone::Negative),
            ),
        };
        let base = button::Style {
            background: fill.map(Into::into),
            text_color: text,
            border: Border {
                color: edge,
                width: stroke::HAIRLINE,
                radius: radius::CONTROL.into(),
            },
            ..button::Style::default()
        };
        match (status, rank) {
            (button::Status::Active, _) => base,
            (button::Status::Hovered | button::Status::Pressed, Rank::Primary) => button::Style {
                background: Some(c.primary_strong().into()),
                ..base
            },
            (button::Status::Hovered | button::Status::Pressed, Rank::Danger) => button::Style {
                background: c
                    .tone_fill(crate::view::theme::Tone::Negative)
                    .map(Into::into),
                ..base
            },
            (button::Status::Hovered, _) => button::Style {
                background: Some(c.hover().into()),
                text_color: c.text(),
                ..base
            },
            (button::Status::Pressed, _) => button::Style {
                background: Some(c.raised().into()),
                text_color: c.text(),
                ..base
            },
            // Disabled reads in words too: the label stays, dimmed, and
            // the fill drops to the panel — never just a fainter colour.
            (button::Status::Disabled, _) => button::Style {
                background: fill.map(|_| c.raised().into()),
                text_color: c.text_dim(),
                border: Border {
                    color: c.line(),
                    ..base.border
                },
                ..base
            },
        }
    })
}

/// The surface's one verb — "run doctor", "send", "sweep admin space".
/// One per surface: two primaries are a surface that has not decided
/// what it is for.
pub fn primary<'a, M: 'a>(content: impl Into<Element<'a, M>>) -> Button<'a, M> {
    ranked(content, Rank::Primary)
}

/// An ordinary action — "save", "go to subject", "copy graphviz (dot)".
pub fn secondary<'a, M: 'a>(content: impl Into<Element<'a, M>>) -> Button<'a, M> {
    ranked(content, Rank::Secondary)
}

/// A quiet action — a toggle, "clear", "ndjson": no fill until hovered.
pub fn ghost<'a, M: 'a>(content: impl Into<Element<'a, M>>) -> Button<'a, M> {
    ranked(content, Rank::Ghost)
}

/// The irreversible wire act — send's "retire" (a tombstone on the bus).
/// An outline in the danger hue, so it is never the obvious button.
pub fn danger<'a, M: 'a>(content: impl Into<Element<'a, M>>) -> Button<'a, M> {
    ranked(content, Rank::Danger)
}

/// An inline, text-like button — breadcrumb chunks, the ▸/▾ expand markers,
/// the tree's expand chevrons and watch eyes, a dock's close icon.
///
/// Hover paints the wash rather than fading the label: a fading label reads
/// as the *text* changing, and on a dense surface it is easy to lose.
pub fn link<'a, M: 'a>(content: impl Into<Element<'a, M>>) -> Button<'a, M> {
    button(content.into()).style(|theme: &iced::Theme, status| {
        let c = colors(theme);
        let base = button::Style {
            background: None,
            text_color: c.text(),
            border: Border {
                color: Color::TRANSPARENT,
                width: 0.0,
                radius: radius::CONTROL.into(),
            },
            ..button::Style::default()
        };
        match status {
            button::Status::Active => base,
            button::Status::Hovered => button::Style {
                background: Some(c.hover().into()),
                ..base
            },
            button::Status::Pressed => button::Style {
                background: Some(c.border().into()),
                ..base
            },
            button::Status::Disabled => button::Style {
                text_color: c.text_dim(),
                ..base
            },
        }
    })
}

/// A full-width row click target — tree rows, echo lines, history entries,
/// palette results.
///
/// Transparent at rest; hover paints the wash so a 40k-row list is trackable
/// with the mouse (#193); `selected` paints the stronger shade persistently.
/// The row's own content still carries the words — the background is an aid,
/// never the message.
pub fn row_button<'a, M: 'a>(content: impl Into<Element<'a, M>>, selected: bool) -> Button<'a, M> {
    button(content.into())
        .width(Length::Fill)
        .style(move |theme: &iced::Theme, status| {
            let c = colors(theme);
            let background = if selected {
                // A primary tint, not the border shade a hover's press also
                // paints: selection is a state, hover is the mouse (#534).
                Some(c.selected().into())
            } else {
                match status {
                    button::Status::Hovered => Some(c.hover().into()),
                    button::Status::Pressed => Some(c.border().into()),
                    button::Status::Active | button::Status::Disabled => None,
                }
            };
            button::Style {
                background,
                text_color: if matches!(status, button::Status::Disabled) {
                    c.text_dim()
                } else {
                    c.text()
                },
                border: Border {
                    color: Color::TRANSPARENT,
                    width: 0.0,
                    radius: radius::CONTROL.into(),
                },
                ..button::Style::default()
            }
        })
}

/// A single-line input. Focus is [`focus_ring`], handled once here — the
/// input's own padding is the ring's offset from the value text.
pub fn input<'a, M: Clone + 'a>(placeholder: &str, value: &str) -> TextInput<'a, M> {
    iced::widget::text_input(placeholder, value).style(|theme: &iced::Theme, status| {
        use iced::widget::text_input::{Status, Style};
        let c = colors(theme);
        let base = Style {
            background: c.background().into(),
            border: Border {
                color: c.border(),
                width: stroke::HAIRLINE,
                radius: radius::CONTROL.into(),
            },
            icon: c.text_muted(),
            placeholder: c.text_dim(),
            value: c.text(),
            selection: c.primary(),
        };
        match status {
            Status::Active => base,
            Status::Hovered => Style {
                border: Border {
                    color: c.text_muted(),
                    ..base.border
                },
                ..base
            },
            Status::Focused { .. } => Style {
                border: focus_ring(theme),
                ..base
            },
            Status::Disabled => Style {
                background: c.surface().into(),
                value: c.text_dim(),
                ..base
            },
        }
    })
}

/// A search field (#558): [`input`] with a search glyph inside it, on the
/// left — the palette, the tree's find box, echo's filter. The glyph keeps
/// saying what the field is for once its placeholder has been typed over.
pub fn search<'a, M: Clone + 'a>(placeholder: &str, value: &str) -> TextInput<'a, M> {
    input(placeholder, value).icon(iced::widget::text_input::Icon {
        font: face::ICONS,
        code_point: Icon::Search.codepoint(),
        size: Some(font::CAPTION.into()),
        spacing: space::SM,
        side: iced::widget::text_input::Side::Left,
    })
}

/// A dropdown. An open picker wears the [`focus_ring`] — it holds the
/// keyboard.
pub fn picker<'a, T, L, V, M>(
    options: L,
    selected: Option<V>,
    on_select: impl Fn(T) -> M + 'a,
) -> PickList<'a, T, L, V, M>
where
    T: ToString + PartialEq + Clone + 'a,
    L: std::borrow::Borrow<[T]> + 'a,
    V: std::borrow::Borrow<T> + 'a,
    M: Clone,
{
    iced::widget::pick_list(options, selected, on_select).style(|theme: &iced::Theme, status| {
        use iced::widget::pick_list::{Status, Style};
        let c = colors(theme);
        let base = Style {
            text_color: c.text(),
            placeholder_color: c.text_dim(),
            handle_color: c.text_muted(),
            background: c.background().into(),
            border: Border {
                color: c.border(),
                width: stroke::HAIRLINE,
                radius: radius::CONTROL.into(),
            },
        };
        match status {
            Status::Active => base,
            Status::Hovered => Style {
                border: Border {
                    color: c.text_muted(),
                    ..base.border
                },
                ..base
            },
            Status::Opened { .. } => Style {
                border: focus_ring(theme),
                ..base
            },
        }
    })
}

/// A labelled checkbox. The box fills `primary()` when checked — and the
/// label says what is checked, so the fill is never the only carrier.
pub fn check<'a, M: 'a>(is_checked: bool) -> Checkbox<'a, M> {
    iced::widget::checkbox(is_checked).style(|theme: &iced::Theme, status| {
        use iced::widget::checkbox::{Status, Style};
        let c = colors(theme);
        let checked = match status {
            Status::Active { is_checked }
            | Status::Hovered { is_checked }
            | Status::Disabled { is_checked } => is_checked,
        };
        let base = Style {
            background: if checked {
                c.primary().into()
            } else {
                c.background().into()
            },
            icon_color: c.on_primary(),
            border: Border {
                color: c.border(),
                width: stroke::HAIRLINE,
                radius: radius::CHECK.into(),
            },
            text_color: Some(c.text()),
        };
        match status {
            Status::Active { .. } => base,
            Status::Hovered { .. } => Style {
                border: Border {
                    color: c.primary(),
                    ..base.border
                },
                ..base
            },
            Status::Disabled { .. } => Style {
                background: c.surface().into(),
                text_color: Some(c.text_dim()),
                ..base
            },
        }
    })
}

/// The replay scrubber. Construction routes through `kit::` like every other
/// interactive element; the theme's slider style already draws its hover and
/// drag states off the same palette.
pub fn scrub<'a, T, M: Clone>(
    range: std::ops::RangeInclusive<T>,
    value: T,
    on_change: impl Fn(T) -> M + 'a,
) -> Slider<'a, T, M>
where
    T: Copy + From<u8> + PartialOrd,
{
    iced::widget::slider(range, value, on_change)
}

/// A button that is an icon, with or without its word.
///
/// Icon-only is for **stateless verbs** that are also in the palette and
/// carry a [`tip`] — settings, close, tear off. A control with state (a
/// density, a zoom level, a recording) passes its word: a tooltip is not
/// read until hovered, and a state no one can see is not stated (the
/// scope editor's rule, #187).
pub fn icon_button<'a, M: 'a>(i: Icon, word: Option<&'a str>) -> Button<'a, M> {
    let content: Element<'a, M> = match word {
        None => icon(i).into(),
        Some(w) => row![icon(i), caption(w)]
            .spacing(space::XS)
            .align_y(iced::Alignment::Center)
            .into(),
    };
    link(content).padding([space::XS, space::SM])
}

/// A hover hint. Never the only place something is said: what a tooltip
/// holds is a *name* or a *shortcut* for a control whose meaning is
/// already on screen.
pub fn tip<'a, M: 'a>(
    content: impl Into<Element<'a, M>>,
    label: impl iced::widget::text::IntoFragment<'a>,
) -> Element<'a, M> {
    tooltip(
        content,
        container(caption(label))
            .padding([space::XS, space::SM])
            .style(|theme: &iced::Theme| {
                let c = colors(theme);
                container::Style {
                    background: Some(c.surface().into()),
                    text_color: Some(c.text()),
                    border: Border {
                        color: c.border(),
                        width: stroke::HAIRLINE,
                        radius: radius::CONTROL.into(),
                    },
                    ..container::Style::default()
                }
            }),
        tooltip::Position::Bottom,
    )
    .gap(space::XS)
    .into()
}

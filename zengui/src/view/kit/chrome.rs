//! Window chrome (#536): the segmented control, toggle chips, the status
//! dot and the pill they sit in.

use iced::widget::{Row, button, container, row, text};
use iced::{Border, Color, Element, Length};

use super::icon::{Icon, icon};
use super::{caption, tip};
use crate::view::theme::{Tone, colors};
use crate::view::tokens::{face, radius, space, stroke};

/// One segment of a [`segmented`] control.
pub struct Segment<'a, T> {
    pub value: T,
    pub label: String,
    pub icon: Option<Icon>,
    /// A count beside the label — a separate text, so the label stays
    /// findable on its own. `None` is no pill, never a zero: a count that
    /// was not taken is not a count of nothing (O4).
    pub count: Option<String>,
    /// A hover hint: the segment's shortcut. Never the only place the
    /// segment says what it is.
    pub tip: Option<&'a str>,
}

/// A segmented control (#536): one track, one segment lit — the active one
/// takes the panel and SemiBold, so which is lit reads in weight and fill,
/// not in colour alone. `active: None` lights nothing, which is a state the
/// caller then names in words (the layout presets say "custom layout").
pub fn segmented<'a, T, M>(
    segments: Vec<Segment<'a, T>>,
    active: Option<T>,
    on_select: impl Fn(T) -> M + 'a,
) -> Element<'a, M>
where
    T: Clone + PartialEq + 'a,
    M: Clone + 'a,
{
    let mut track = Row::new().spacing(space::XS / 2.0);
    for seg in segments {
        let lit = active.as_ref() == Some(&seg.value);
        let mut content = Row::new()
            .spacing(space::XS)
            .align_y(iced::Alignment::Center);
        if let Some(i) = seg.icon {
            content = content.push(icon(i));
        }
        content = content.push(if lit {
            caption(seg.label).font(face::SEMIBOLD)
        } else {
            caption(seg.label)
        });
        if let Some(n) = seg.count {
            content = content.push(count_pill(n));
        }
        let b = button(content)
            .padding([space::XS / 2.0, space::SM])
            .on_press(on_select(seg.value.clone()))
            .style(move |theme: &iced::Theme, status| {
                let c = colors(theme);
                let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
                button::Style {
                    background: if lit {
                        Some(c.panel().into())
                    } else if hovered {
                        Some(c.hover().into())
                    } else {
                        None
                    },
                    text_color: if lit || hovered {
                        c.text()
                    } else {
                        c.text_muted()
                    },
                    border: Border {
                        color: if lit { c.line() } else { Color::TRANSPARENT },
                        width: stroke::HAIRLINE,
                        radius: radius::CONTROL.into(),
                    },
                    ..button::Style::default()
                }
            });
        track = track.push(match seg.tip {
            Some(t) => tip(b, t),
            None => b.into(),
        });
    }
    container(track)
        .padding(space::XS / 2.0)
        .style(|theme: &iced::Theme| container::Style {
            background: Some(colors(theme).raised().into()),
            border: Border {
                radius: radius::CONTROL.into(),
                ..Border::default()
            },
            ..container::Style::default()
        })
        .into()
}

/// A count in a pill (#536): mono, fully round, beside a label.
pub fn count_pill<'a, M: 'a>(n: impl Into<String>) -> Element<'a, M> {
    container(
        caption(n.into())
            .font(face::MONO)
            .style(|theme: &iced::Theme| text::Style {
                color: Some(colors(theme).text_muted()),
            }),
    )
    .padding([0.0, space::XS + space::XS / 2.0])
    .style(|theme: &iced::Theme| container::Style {
        background: Some(colors(theme).well().into()),
        border: Border {
            radius: radius::PILL.into(),
            ..Border::default()
        },
        ..container::Style::default()
    })
    .into()
}

/// An on/off toggle that says which by its shape (#536): on is a filled
/// chip with a hairline, off an outline with dimmed words — never the same
/// box in two colours. The dock strip's four toggles.
pub fn toggle_chip<'a, M: Clone + 'a>(
    i: Option<Icon>,
    label: &'a str,
    on: bool,
    on_press: M,
) -> Element<'a, M> {
    let mut content = Row::new()
        .spacing(space::XS)
        .align_y(iced::Alignment::Center);
    if let Some(i) = i {
        content = content.push(icon(i));
    }
    content = content.push(caption(label));
    button(content)
        .padding([space::XS / 2.0, space::SM])
        .on_press(on_press)
        .style(move |theme: &iced::Theme, status| {
            let c = colors(theme);
            let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
            button::Style {
                background: match (on, hovered) {
                    (_, true) => Some(c.hover().into()),
                    (true, false) => Some(c.raised().into()),
                    (false, false) => None,
                },
                text_color: if on { c.text() } else { c.text_dim() },
                border: Border {
                    color: c.line(),
                    width: stroke::HAIRLINE,
                    radius: radius::CONTROL.into(),
                },
                ..button::Style::default()
            }
        })
        .into()
}

/// A status: a dot in a tone **and** its word (#536). The dot is the glance;
/// the word is the claim — there is no constructor for the dot alone.
pub fn status<'a, M: 'a>(tone: Tone, word: impl Into<String>) -> Element<'a, M> {
    let dot = container(iced::widget::Space::new())
        .width(Length::Fixed(DOT))
        .height(Length::Fixed(DOT))
        .style(move |theme: &iced::Theme| container::Style {
            background: Some(colors(theme).tone(tone).into()),
            border: Border {
                radius: radius::PILL.into(),
                ..Border::default()
            },
            ..container::Style::default()
        });
    row![
        dot,
        caption(word.into()).style(move |theme: &iced::Theme| text::Style {
            color: Some(colors(theme).tone(tone)),
        })
    ]
    .spacing(space::XS + space::XS / 2.0)
    .align_y(iced::Alignment::Center)
    .into()
}

/// A status dot's diameter.
const DOT: f32 = space::SM;

/// A framed group in the chrome (#536): the panel, a hairline, the control
/// radius — the connection pill's box.
pub fn pill<'a, M: 'a>(content: impl Into<Element<'a, M>>) -> Element<'a, M> {
    container(content)
        .padding([space::XS, space::SM])
        .style(|theme: &iced::Theme| {
            let c = colors(theme);
            container::Style {
                background: Some(c.panel().into()),
                border: Border {
                    color: c.line(),
                    width: stroke::HAIRLINE,
                    radius: radius::CONTROL.into(),
                },
                ..container::Style::default()
            }
        })
        .into()
}

/// An icon and its word, as a button's content (#536).
pub fn labelled<'a, M: 'a>(i: Icon, word: impl Into<String>) -> Element<'a, M> {
    row![icon(i), caption(word.into())]
        .spacing(space::XS)
        .align_y(iced::Alignment::Center)
        .into()
}

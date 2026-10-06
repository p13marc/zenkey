//! Frames (#537): the dock panel and its header, the modal and its scrim.

use iced::widget::{container, opaque, row, text};
use iced::{Border, Element, Length, Shadow, Vector};

use super::caption;
use super::icon::{Icon, icon_caption};
use crate::view::theme::colors;
use crate::view::tokens::{face, radius, space, stroke};

/// A dock's panel (#537): the dock floats on the well as a card. A focused
/// dock's hairline takes the primary — and its title goes SemiBold, so
/// focus is never the colour alone.
pub fn dock_frame(focused: bool) -> impl Fn(&iced::Theme) -> container::Style {
    move |theme: &iced::Theme| {
        let c = colors(theme);
        container::Style {
            background: Some(c.panel().into()),
            border: Border {
                color: if focused {
                    crate::view::theme::alpha(c.primary(), 0.55)
                } else {
                    c.line()
                },
                width: stroke::HAIRLINE,
                radius: radius::CARD.into(),
            },
            ..container::Style::default()
        }
    }
}

/// A dock's title (#537): its icon and its name. The name is the role's
/// own lowercase label — it is what the tests and the dock strip call it.
pub fn dock_title<'a, M: 'a>(i: Icon, label: &'a str, focused: bool) -> Element<'a, M> {
    let ink = move |theme: &iced::Theme| text::Style {
        color: Some(if focused {
            colors(theme).text()
        } else {
            colors(theme).text_muted()
        }),
    };
    let name = if focused {
        caption(label).font(face::SEMIBOLD)
    } else {
        caption(label)
    };
    row![icon_caption(i).style(ink), name.style(ink)]
        .spacing(space::XS + space::XS / 2.0)
        .align_y(iced::Alignment::Center)
        .into()
}

/// An overlay's card (#537): the panel at the modal radius, lifted off the
/// scrim by a shadow. `height: None` sizes to the content.
pub fn modal<'a, M: 'a>(
    content: impl Into<Element<'a, M>>,
    width: f32,
    height: Option<f32>,
) -> Element<'a, M> {
    let mut frame = container(content)
        .padding(space::MD)
        .width(Length::Fixed(width));
    if let Some(h) = height {
        frame = frame.height(Length::Fixed(h));
    }
    frame
        .style(|theme: &iced::Theme| {
            let c = colors(theme);
            container::Style {
                background: Some(c.panel().into()),
                border: Border {
                    color: c.line(),
                    width: stroke::HAIRLINE,
                    radius: radius::MODAL.into(),
                },
                shadow: Shadow {
                    color: c.shadow(),
                    offset: Vector::new(0.0, space::SM),
                    blur_radius: space::LG,
                },
                ..container::Style::default()
            }
        })
        .into()
}

/// The layer an overlay floats on (#537): the window dimmed behind it, and
/// **opaque** — a click on the dimmed panes no longer reaches them, as a
/// modal's never should. Esc still peels one layer at a time; that rule is
/// the app's, not this widget's.
pub fn scrim<'a, M: 'a>(layer: impl Into<Element<'a, M>>) -> Element<'a, M> {
    opaque(
        container(layer)
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(iced::alignment::Horizontal::Center)
            .padding(space::XL)
            .style(|theme: &iced::Theme| container::Style {
                background: Some(colors(theme).scrim().into()),
                ..container::Style::default()
            }),
    )
}

/// A callout (#537): a card tinted by a tone — the replay banner's frame.
/// The tint groups; the words inside still make every claim.
pub fn callout<'a, M: 'a>(
    tone: crate::view::theme::Tone,
    content: impl Into<Element<'a, M>>,
) -> Element<'a, M> {
    container(content)
        .padding([space::SM, space::MD])
        .width(Length::Fill)
        .style(move |theme: &iced::Theme| {
            let c = colors(theme);
            container::Style {
                background: c.tone_fill(tone).map(Into::into),
                border: Border {
                    color: c.tone_border(tone),
                    width: stroke::HAIRLINE,
                    radius: radius::CARD.into(),
                },
                ..container::Style::default()
            }
        })
        .into()
}

/// A refusal or a failure, said one way everywhere (#559): the error mark
/// and the words, in danger's ink, on a Negative callout. The words are the
/// claim — usually the producer's or the validator's own, carried verbatim —
/// and the mark is what survives when the colour does not.
pub fn error<'a, M: 'a>(message: impl Into<String>) -> Element<'a, M> {
    use crate::view::theme::{SeverityTone, Tone};
    let ink = |theme: &iced::Theme| iced::widget::text::Style {
        color: Some(colors(theme).tone(Tone::Negative)),
    };
    callout(
        Tone::Negative,
        iced::widget::row![
            super::caption(SeverityTone::Error.glyph()).style(ink),
            super::body(message.into()).style(ink),
        ]
        .spacing(space::SM)
        .align_y(iced::Alignment::Center),
    )
}

/// An inset well (#539): a payload's hex or its decoded document, set into
/// the panel on the darker well with a hairline — the code block of the
/// window. Returns the container so the caller can size it.
pub fn inset<'a, M: 'a>(content: impl Into<Element<'a, M>>) -> container::Container<'a, M> {
    container(content)
        .padding(space::SM)
        .style(|theme: &iced::Theme| {
            let c = colors(theme);
            container::Style {
                background: Some(c.well().into()),
                border: Border {
                    color: c.line(),
                    width: stroke::HAIRLINE,
                    radius: radius::CARD.into(),
                },
                ..container::Style::default()
            }
        })
}

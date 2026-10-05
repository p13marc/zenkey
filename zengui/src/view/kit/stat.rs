//! Numbers at a glance (#542): the stat tile and the share bar.

use iced::widget::{Space, column, container, row, text};
use iced::{Border, Element, Length};

use super::icon::{Icon, icon_caption};
use super::{caption, emphasis, eyebrow};
use crate::view::theme::colors;
use crate::view::tokens::{face, radius, space, stroke};

/// A stat tile (#542): an icon on the raised step, an eyebrow, the number at
/// EMPHASIS in the data face, and a footer that says what the number covers
/// — or why there is none. The footer is not decoration: "—" with "not
/// asked" under it is the whole claim.
pub fn stat<'a, M: 'a>(
    i: Icon,
    label: &'static str,
    value: String,
    footer: String,
) -> Element<'a, M> {
    let tile = container(icon_caption(i).style(|theme: &iced::Theme| text::Style {
        color: Some(colors(theme).primary()),
    }))
    .padding(space::XS)
    .style(|theme: &iced::Theme| container::Style {
        background: Some(colors(theme).raised().into()),
        border: Border {
            radius: radius::CONTROL.into(),
            ..Border::default()
        },
        ..container::Style::default()
    });
    container(
        column![
            row![tile, eyebrow(label)]
                .spacing(space::SM)
                .align_y(iced::Alignment::Center),
            emphasis(value).font(face::MONO),
            caption(footer).style(|theme: &iced::Theme| text::Style {
                color: Some(colors(theme).text_muted()),
            }),
        ]
        .spacing(space::XS),
    )
    .padding([space::SM, space::MD])
    .width(Length::Fill)
    .style(|theme: &iced::Theme| {
        let c = colors(theme);
        container::Style {
            background: Some(c.panel().into()),
            border: Border {
                color: c.line(),
                width: stroke::HAIRLINE,
                radius: radius::CARD.into(),
            },
            ..container::Style::default()
        }
    })
    .into()
}

/// A share as a bar **and** its percentage (#542) — the bar is the glance,
/// the number is the claim. `None` (a share of nothing) draws the empty
/// track and "—", never 0%.
pub fn share_bar<'a, M: 'a>(share: Option<f32>) -> Element<'a, M> {
    let fill = share.unwrap_or(0.0).clamp(0.0, 1.0);
    let track = container(
        container(Space::new())
            .width(Length::Fixed(SHARE_BAR * fill))
            .height(Length::Fixed(space::XS + space::XS / 2.0))
            .style(|theme: &iced::Theme| container::Style {
                background: Some(colors(theme).primary().into()),
                border: Border {
                    radius: radius::PILL.into(),
                    ..Border::default()
                },
                ..container::Style::default()
            }),
    )
    .width(Length::Fixed(SHARE_BAR))
    .style(|theme: &iced::Theme| container::Style {
        background: Some(colors(theme).raised().into()),
        border: Border {
            radius: radius::PILL.into(),
            ..Border::default()
        },
        ..container::Style::default()
    });
    let words = match share {
        Some(f) => format!("{:.1}%", f * 100.0),
        None => "—".to_string(),
    };
    row![track, caption(words).font(face::MONO)]
        .spacing(space::SM)
        .align_y(iced::Alignment::Center)
        .into()
}

/// The share bar's track length.
const SHARE_BAR: f32 = 80.0;

//! Empty states and field grids (#539).

use iced::widget::{Column, Row, column, container, text};
use iced::{Border, Element, Length};

use super::icon::{Icon, glyph_text};
use super::{body, caption, emphasis};
use crate::view::theme::colors;
use crate::view::tokens::{face, font, radius, space};

/// Why a surface is empty (#539) — and so which icon it wears. The icon is
/// fixed by the kind, never chosen at a call site: "not asked" can never be
/// dressed as an alert, and a quiet bus can never look like a filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmptyKind {
    /// Nothing was asked (RFC 09 §5.1 O4): no sweep, no probe, no registry.
    NotAsked,
    /// Asked, and nothing answered or arrived — a non-verdict (RFC 05 §3.1).
    Silent,
    /// Asked, and the answer was an empty list: a fact.
    Empty,
    /// There is something, and the filter hides it.
    Filtered,
    /// Nothing is selected.
    Unselected,
    /// The question does not apply here.
    Inapplicable,
}

impl EmptyKind {
    pub const ALL: [EmptyKind; 6] = [
        EmptyKind::NotAsked,
        EmptyKind::Silent,
        EmptyKind::Empty,
        EmptyKind::Filtered,
        EmptyKind::Unselected,
        EmptyKind::Inapplicable,
    ];

    pub const fn icon(self) -> Icon {
        match self {
            EmptyKind::NotAsked => Icon::NotAsked,
            EmptyKind::Silent => Icon::Silent,
            EmptyKind::Empty => Icon::Empty,
            EmptyKind::Filtered => Icon::FilterOff,
            EmptyKind::Unselected => Icon::Unselected,
            EmptyKind::Inapplicable => Icon::Info,
        }
    }
}

/// A placeholder for a surface with nothing to show (#539).
///
/// Takes an explanation, not just a noun: "no keys" invites the user to
/// conclude the bus is quiet, which may be false (RFC 05 §3.1). Every call
/// site says *why* it is empty and what would change it — and since #539 its
/// kind, which fixes the icon in the well above the words.
pub fn empty<'a, M: 'a>(
    kind: EmptyKind,
    headline: impl Into<String>,
    why: impl Into<String>,
) -> Element<'a, M> {
    empty_column(kind, headline.into(), why.into()).into()
}

/// [`empty`], with the one action that would change it — "watch this key"
/// under "not watched".
pub fn empty_with<'a, M: 'a>(
    kind: EmptyKind,
    headline: impl Into<String>,
    why: impl Into<String>,
    action: impl Into<Element<'a, M>>,
) -> Element<'a, M> {
    empty_column(kind, headline.into(), why.into())
        .push(action)
        .into()
}

fn empty_column<'a, M: 'a>(kind: EmptyKind, headline: String, why: String) -> Column<'a, M> {
    let well = container(glyph_text(kind.icon()).size(font::SECTION).style(
        |theme: &iced::Theme| text::Style {
            color: Some(colors(theme).text_muted()),
        },
    ))
    .padding(space::SM + space::XS)
    .style(|theme: &iced::Theme| container::Style {
        background: Some(colors(theme).raised().into()),
        border: Border {
            radius: radius::PILL.into(),
            ..Border::default()
        },
        ..container::Style::default()
    });
    column![
        well,
        emphasis(headline).font(face::SEMIBOLD),
        container(
            body(why)
                .align_x(iced::alignment::Horizontal::Center)
                .style(|theme: &iced::Theme| text::Style {
                    color: Some(colors(theme).text_muted()),
                })
        )
        .max_width(EMPTY_MEASURE),
    ]
    .spacing(space::SM)
    .padding(space::LG)
    .width(Length::Fill)
    .align_x(iced::Alignment::Center)
}

/// How wide an empty state's explanation runs before it wraps — a measure,
/// so a sentence reads as one rather than as a banner.
const EMPTY_MEASURE: f32 = 440.0;

/// A field's label (#539): small, SemiBold, muted, in capitals. A `&'static
/// str` so it is always a constant written for the purpose — never a
/// pinned word upper-cased at runtime.
pub fn eyebrow<'a>(s: &'static str) -> iced::widget::Text<'a> {
    caption(s)
        .font(face::SEMIBOLD)
        .style(|theme: &iced::Theme| text::Style {
            color: Some(colors(theme).text_muted()),
        })
}

/// One field (#539): an eyebrow over its value on the raised step.
pub fn field<'a, M: 'a>(label: &'static str, value: impl Into<Element<'a, M>>) -> Element<'a, M> {
    column![
        eyebrow(label),
        container(value)
            .padding([space::XS, space::SM])
            .style(|theme: &iced::Theme| container::Style {
                background: Some(colors(theme).raised().into()),
                border: Border {
                    radius: radius::CONTROL.into(),
                    ..Border::default()
                },
                ..container::Style::default()
            }),
    ]
    .spacing(space::XS / 2.0)
    .into()
}

/// A labelled control (#559): an eyebrow over the input, and a muted helper
/// line under it when there is one. The label stays when the field is typed
/// into — a placeholder does not — so a placeholder keeps only an example,
/// and whatever explains the field, its cost or its blast radius, is the
/// helper.
pub fn form_field<'a, M: 'a>(
    label: &'static str,
    control: impl Into<Element<'a, M>>,
    help: Option<String>,
) -> Element<'a, M> {
    let mut col = column![eyebrow(label), control.into()]
        .spacing(space::XS)
        .width(Length::Fill);
    if let Some(help) = help {
        col = col.push(caption(help).style(|theme: &iced::Theme| text::Style {
            color: Some(colors(theme).text_muted()),
        }));
    }
    col.into()
}

/// A grid of fields that wraps to the width it is given (#539).
pub fn fields<'a, M: 'a>(items: Vec<Element<'a, M>>) -> Element<'a, M> {
    Row::from_vec(items)
        .spacing(space::SM)
        .width(Length::Fill)
        .wrap()
        .vertical_spacing(space::SM)
        .into()
}

//! List-row pieces (#538): the leading edge, the coloured preview, the
//! retired chip, the age word.

use iced::widget::{Space, container, rich_text, row, span, stack, text};
use iced::{Border, Element, Length};

use super::caption;
use crate::prefs::ThemeChoice;
use crate::view::syntax::SyntaxSpan;
use crate::view::theme::{alpha, colors, syntax_ink};
use crate::view::tokens::{face, font, radius, space, stroke};

/// What a row's leading edge says (#538).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    /// An ordinary sample: a quiet edge in the line colour.
    Put,
    /// A tombstone (RFC 04 §1.2): the retired colour — never danger's,
    /// because retirement is a fact, not a negative verdict. The row says
    /// "DELETE" in words too.
    Retired,
    /// A write that failed (#562's send log): danger's edge, because the
    /// act did not happen — and the row says why in words.
    Failed,
}

/// A finding (#564): its check as a severity badge — glyph, word and colour
/// from the tone — the subject it is about, and its evidence, on a card.
/// The doctor's findings and the Fields section's are one shape now.
pub fn finding<'a, M: 'a>(
    severity: crate::view::theme::SeverityTone,
    check: impl Into<String>,
    subject: impl Into<String>,
    evidence: impl Into<String>,
) -> Element<'a, M> {
    super::card(
        iced::widget::column![
            iced::widget::row![
                super::badge_severity(severity, check),
                super::caption(subject.into()).font(face::MONO),
            ]
            .spacing(space::SM)
            .align_y(iced::Alignment::Center),
            super::muted(evidence.into()),
        ]
        .spacing(space::XS),
    )
}

/// A list row with a 3px leading edge (#538). The edge groups; the row's
/// own words carry the claim.
pub fn edge_row<'a, M: 'a>(edge: Edge, content: impl Into<Element<'a, M>>) -> Element<'a, M> {
    let mark = container(Space::new())
        .width(Length::Fixed(stroke::EDGE))
        .height(Length::Fill)
        .style(move |theme: &iced::Theme| {
            let c = colors(theme);
            container::Style {
                background: Some(
                    match edge {
                        Edge::Put => c.line(),
                        Edge::Retired => c.retired(),
                        Edge::Failed => c.danger(),
                    }
                    .into(),
                ),
                border: Border {
                    radius: radius::CHECK.into(),
                    ..Border::default()
                },
                ..container::Style::default()
            }
        });
    row![mark, content.into()]
        .spacing(space::SM)
        .height(Length::Fill)
        .into()
}

/// A payload preview in its syntax colours (#538): mono caption — keys and
/// strings tinted, literals in the text colour, punctuation dim.
///
/// The spans were computed once, at ingest. A rich text's spans take
/// resolved colours rather than a theme closure, which is why the caller
/// hands over the [`ThemeChoice`] — the colours are still the theme's
/// ([`syntax_ink`]), only resolved one step earlier.
///
/// Drawn over a plain, transparent twin of the same characters: a rich text
/// reports no text to `operate`, so without the twin a test could no longer
/// find a preview on screen — and a preview nothing can read is one nothing
/// pins. With no spans (prose, a tombstone, an oversize note) it is the
/// plain muted text alone.
pub fn code<'a, M: 'a>(
    preview: &'a str,
    spans: &'a [SyntaxSpan],
    choice: ThemeChoice,
) -> Element<'a, M> {
    let plain = caption(preview).font(face::MONO);
    if spans.is_empty() {
        return plain
            .style(|theme: &iced::Theme| text::Style {
                color: Some(colors(theme).text_muted()),
            })
            .into();
    }
    let runs: Vec<_> = spans
        .iter()
        .map(|s| {
            span(&preview[s.start as usize..s.end as usize])
                .font(face::MONO)
                .size(font::CAPTION)
                .color(syntax_ink(choice, s.role))
        })
        .collect();
    stack![
        plain.style(|_: &iced::Theme| text::Style {
            color: Some(iced::Color::TRANSPARENT),
        }),
        rich_text::<(), _, _, _>(runs)
            .font(face::MONO)
            .size(font::CAPTION),
    ]
    .into()
}

/// A tombstone's word in a chip (#538) — the retired colour, tinted like a
/// verdict chip but on a hue no verdict uses.
pub fn retired_chip<'a, M: 'a>(label: &'a str) -> Element<'a, M> {
    container(
        caption(label)
            .font(face::SEMIBOLD)
            .style(|theme: &iced::Theme| text::Style {
                color: Some(colors(theme).retired()),
            }),
    )
    .padding([super::CHIP_AIR, space::XS])
    .style(|theme: &iced::Theme| {
        let c = colors(theme);
        container::Style {
            background: Some(alpha(c.retired(), 0.12).into()),
            border: Border {
                color: alpha(c.retired(), 0.35),
                width: stroke::HAIRLINE,
                radius: radius::CHIP.into(),
            },
            ..container::Style::default()
        }
    })
    .into()
}

/// How long since a key was last seen, as a word (#538): "now" under three
/// seconds, then seconds, minutes, hours. It replaced a dot whose colour was
/// the only thing saying how fresh the key was — and whose `●` was the
/// Registered badge's glyph in the same row.
pub fn freshness<'a, M: 'a>(age_s: f32) -> Element<'a, M> {
    caption(super::age_word(age_s))
        .font(face::MONO)
        .style(move |theme: &iced::Theme| text::Style {
            color: Some(colors(theme).freshness(age_s)),
        })
        .into()
}

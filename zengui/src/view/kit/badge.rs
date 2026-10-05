//! Chips (#193, #535): the six badge scales, data chips, status chips.

use iced::widget::{container, row, text};
use iced::{Border, Color, Element};

use super::caption;
use crate::view::theme::colors;
use crate::view::tokens::{face, radius, space, stroke};

/// The one badge renderer (#193): glyph **and** word, colour on both — and,
/// since #535, a chip behind them.
///
/// Private, because the glyph is not a parameter any call site gets to pass —
/// each public badge constructor derives it from its tone enum, so a badge
/// with colour alone, or two states of a scale sharing a glyph, cannot be
/// written. Glyph and word stay **two texts**: the glyph is the carrier that
/// survives when colour does not, and the tests find each on its own.
///
/// The chip's anatomy is the tone's, decided in `theme`: an answer is
/// tinted (fill at 10%, a hairline at 28%), commentary sits on the raised
/// step, and a question not asked ([`Tone::Neutral`]) is an **outline with
/// no fill** — absence of information drawn as absence, so "—" can never be
/// mistaken for a quieter "no".
///
/// [`Tone::Neutral`]: crate::view::theme::Tone::Neutral
fn glyph_badge<'a, M: 'a>(
    glyph: &'static str,
    label: impl Into<String>,
    tone: crate::view::theme::Tone,
) -> Element<'a, M> {
    let ink = move |theme: &iced::Theme| text::Style {
        color: Some(colors(theme).tone(tone)),
    };
    chip_frame(
        row![caption(glyph).style(ink), caption(label.into()).style(ink)]
            .spacing(space::XS)
            .align_y(iced::Alignment::Center),
        move |theme| {
            let c = colors(theme);
            (c.tone_fill(tone), c.tone_border(tone))
        },
    )
}

/// The chip's box, shared by every chip kind: a hairline at
/// [`radius::CHIP`], a single pixel of vertical air — so a chip is one
/// caption line plus two pixels, and fits a compact tree row (the
/// `chips_fit_every_row` test holds it there).
fn chip_frame<'a, M: 'a>(
    content: impl Into<Element<'a, M>>,
    paint: impl Fn(&iced::Theme) -> (Option<Color>, Color) + 'a,
) -> Element<'a, M> {
    container(content)
        .padding([CHIP_AIR, space::XS])
        .style(move |theme: &iced::Theme| {
            let (fill, edge) = paint(theme);
            container::Style {
                background: fill.map(Into::into),
                border: Border {
                    color: edge,
                    width: stroke::HAIRLINE,
                    radius: radius::CHIP.into(),
                },
                ..container::Style::default()
            }
        })
        .into()
}

/// A chip's vertical air (#535): a quarter of XS — one pixel.
pub(crate) const CHIP_AIR: f32 = space::XS / 4.0;

/// How tall a chip stands (#535): one caption line and its air — iced draws
/// the hairline inside the bounds, so it costs no height.
/// Every virtualized row is measured against this.
pub const CHIP_HEIGHT: f32 = crate::view::tokens::CAPTION_LINE + 2.0 * CHIP_AIR;

/// A data chip (#535): a size, a rate, an id, an encoding — mono on the
/// raised step, muted. Data, not a verdict: no tone, no glyph.
pub fn data_chip<'a, M: 'a>(s: impl iced::widget::text::IntoFragment<'a>) -> Element<'a, M> {
    chip_frame(
        caption(s)
            .font(face::MONO)
            .style(|theme: &iced::Theme| text::Style {
                color: Some(colors(theme).text_muted()),
            }),
        |theme| (Some(colors(theme).raised()), Color::TRANSPARENT),
    )
}

/// A status chip (#535): one sentence in a tone's chip — the status strip's
/// cells (#537). The words carry the state; the tint only groups it.
pub fn status_chip<'a, M: 'a>(
    tone: crate::view::theme::Tone,
    s: impl Into<String>,
) -> Element<'a, M> {
    chip_frame(
        caption(s.into()).style(move |theme: &iced::Theme| text::Style {
            color: Some(colors(theme).tone(tone)),
        }),
        move |theme| {
            let c = colors(theme);
            (c.tone_fill(tone), c.tone_border(tone))
        },
    )
}

/// A registration badge, theme-resolved. The glyph comes from the tone
/// (#193): passing a pre-resolved `Color` would bake in one theme's palette,
/// and passing a glyph would let two states share one.
pub fn tone_badge<'a, M: 'a>(
    tone: crate::view::theme::RegistrationTone,
    label: impl Into<String>,
) -> Element<'a, M> {
    glyph_badge(tone.glyph(), label, tone.tone())
}

/// A presence badge (#61) — theme-resolved like [`tone_badge`], glyph from
/// the tone.
pub fn badge_presence<'a, M: 'a>(
    tone: crate::view::theme::PresenceTone,
    label: impl Into<String>,
) -> Element<'a, M> {
    glyph_badge(tone.glyph(), label, tone.tone())
}

/// A doctor-severity badge (#71) — theme-resolved like [`tone_badge`],
/// carrying the CLI's severity marks via the tone.
pub fn badge_severity<'a, M: 'a>(
    tone: crate::view::theme::SeverityTone,
    label: impl Into<String>,
) -> Element<'a, M> {
    glyph_badge(tone.glyph(), label, tone.tone())
}

/// A storage-coverage badge (#70) — theme-resolved like [`tone_badge`], and
/// carrying the CLI's own mark via the tone so the two explorers read alike.
/// Mark **and** text, never colour alone.
pub fn badge_coverage<'a, M: 'a>(
    tone: crate::view::theme::CoverageTone,
    label: impl Into<String>,
) -> Element<'a, M> {
    glyph_badge(tone.glyph(), label, tone.tone())
}

/// A payload-conformance verdict badge (#164) — theme-resolved like
/// [`tone_badge`], glyph from the tone. Three states, never a boolean, and
/// the call sites derive the tone from a [`zenkey::schema::validate::Verdict`]
/// through [`crate::verdict::tone`] so "not validated" can never be collapsed
/// into either answer.
pub fn badge_verdict<'a, M: 'a>(
    tone: crate::view::theme::VerdictTone,
    label: impl Into<String>,
) -> Element<'a, M> {
    glyph_badge(tone.glyph(), label, tone.tone())
}

/// A why-ladder rung badge (#214) — theme-resolved like [`tone_badge`],
/// glyph from the tone. `NotAsked` gets its own mark and swatch, because a
/// rung that prints `No` where it means "not asked" is the failure the
/// ladder was built to replace (RFC 09 §5.1 O4).
pub fn badge_rung<'a, M: 'a>(
    tone: crate::view::theme::RungTone,
    label: impl Into<String>,
) -> Element<'a, M> {
    glyph_badge(tone.glyph(), label, tone.tone())
}

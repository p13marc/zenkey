//! Small shared widgets.
//!
//! The only place besides [`super::theme`] allowed to name a colour — and here
//! only `Color::TRANSPARENT`, the absence of one (`check-color.sh`, #534) — and
//! the only place allowed to call `text(` at all (#191): every
//! view builds its text through the five role constructors below, so the type
//! scale is assigned by role, not by taste, and a grep can enforce it.
//!
//! The same discipline covers the two other encodings a call site used to be
//! able to get wrong (#193): a badge's glyph comes from its tone enum, never
//! from the call site, so colour cannot be the only carrier and two states of
//! a scale cannot share a mark; and every interactive element is built by a
//! `kit::` constructor, so the `button::Status` set and the [`focus_ring`]
//! are each handled exactly once (`check-interactive.sh` enforces the seam).
//!
//! One module per kind since #535, re-exported flat — `kit::primary`,
//! `kit::Icon`, `kit::window` — so the split can be re-cut without a call
//! site moving: `badge` (chips: the six badge scales, data and status
//! chips), `buttons` (every interactive constructor), `chrome` (the
//! segmented control, toggle chips, status dots, pills), `icon`, `list`
//! (the shared virtualized window), `rows` (a row's edge, its coloured
//! preview, the retired chip, the age word), `format` (bytes, rates,
//! plurals, ages).

mod badge;
mod buttons;
mod chrome;
mod format;
mod frame;
mod icon;
mod list;
mod rows;

pub use badge::*;
pub use buttons::*;
pub use chrome::*;
pub use format::*;
pub use frame::*;
pub use icon::*;
pub use list::*;
pub use rows::*;

use iced::widget::text::IntoFragment;
use iced::widget::{Text, column, container, row, text};
use iced::{Border, Element, Length};

use super::theme::colors;
use super::tokens::{face, font, radius, space, stroke};

/// The subject (`font::TITLE`). One per window: the selected key or producer
/// in the location bar. If two of these are visible, one of them is lying
/// about being the subject.
pub fn title<'a>(s: impl IntoFragment<'a>) -> Text<'a> {
    text(s).size(font::TITLE)
}

/// A dock or pane header (`font::SECTION`). See also [`section_header`],
/// which pairs one of these with trailing controls.
pub fn section<'a>(s: impl IntoFragment<'a>) -> Text<'a> {
    text(s).size(font::SECTION).font(face::SEMIBOLD)
}

/// Emphasis (`font::EMPHASIS`): card titles, key values, the number in a
/// stat tile, the roster origin.
pub fn emphasis<'a>(s: impl IntoFragment<'a>) -> Text<'a> {
    text(s).size(font::EMPHASIS).font(face::MEDIUM)
}

/// Prose (`font::BODY`): empty-state explanations, echo lines, log lines,
/// help text — anything meant to be *read* rather than scanned.
pub fn body<'a>(s: impl IntoFragment<'a>) -> Text<'a> {
    text(s).size(font::BODY)
}

/// Metadata (`font::CAPTION`): table cells, badges, timestamps, byte counts.
/// The dense stuff — which is exactly why it must not be the whole app.
pub fn caption<'a>(s: impl IntoFragment<'a>) -> Text<'a> {
    text(s).size(font::CAPTION)
}

/// A bordered panel.
pub fn card<'a, M: 'a>(content: impl Into<Element<'a, M>>) -> Element<'a, M> {
    container(content)
        .padding(space::MD)
        .width(Length::Fill)
        .style(|theme: &iced::Theme| {
            let c = colors(theme);
            container::Style {
                background: Some(c.surface().into()),
                border: Border {
                    color: c.border(),
                    width: stroke::HAIRLINE,
                    radius: radius::CARD.into(),
                },
                ..container::Style::default()
            }
        })
        .into()
}

/// A section title with optional trailing controls.
pub fn section_header<'a, M: 'a>(
    title: impl Into<String>,
    trailing: Option<Element<'a, M>>,
) -> Element<'a, M> {
    let mut r = row![
        section(title.into()).style(|theme: &iced::Theme| text::Style {
            color: Some(colors(theme).text()),
        })
    ]
    .spacing(space::SM)
    .align_y(iced::Alignment::Center);
    r = r.push(iced::widget::space::horizontal());
    if let Some(t) = trailing {
        r = r.push(t);
    }
    r.into()
}

/// Dimmed caption text.
pub fn muted<'a, M: 'a>(s: impl Into<String>) -> Element<'a, M> {
    caption(s.into())
        .style(|theme: &iced::Theme| text::Style {
            color: Some(colors(theme).text_muted()),
        })
        .into()
}

/// Monospaced caption text — keys, payload previews, dense table cells.
pub fn mono<'a, M: 'a>(s: impl Into<String>) -> Element<'a, M> {
    caption(s.into()).font(face::MONO).into()
}

/// A placeholder for a pane with nothing to show.
///
/// Takes an explanation, not just a noun: "no keys" invites the user to
/// conclude the bus is quiet, which may be false (RFC 05 §3.1). Every call site
/// says *why* it is empty and what would change it.
pub fn empty_state<'a, M: 'a>(
    headline: impl Into<String>,
    why: impl Into<String>,
) -> Element<'a, M> {
    container(
        column![
            emphasis(headline.into()).style(|theme: &iced::Theme| text::Style {
                color: Some(colors(theme).text_muted()),
            }),
            body(why.into()).style(|theme: &iced::Theme| text::Style {
                color: Some(colors(theme).text_muted()),
            }),
        ]
        .spacing(space::SM)
        .align_x(iced::Alignment::Center),
    )
    .center_x(Length::Fill)
    .padding(space::LG)
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The window is O(visible) and covers the viewport, at any offset.
    ///
    /// This lived in `tree.rs` and was the tree's alone; History drew all 200
    /// of its rows and Echo capped drawing at 300 with no window at all
    /// (#183). Three lists, one window, one test.
    #[test]
    fn the_window_is_bounded_and_covers_the_viewport_at_every_height() {
        for row_height in [24.0_f32, 40.0, 44.0] {
            let rows = 50_000;
            let viewport = 600.0;
            let expected = (viewport / row_height).ceil() as usize + 1 + 2 * OVERSCAN;

            let (first, last) = window(rows, Viewport::scrolled(0.0, viewport), row_height);
            assert_eq!(first, 0);
            assert!(last - first <= expected, "top: {} rows", last - first);

            let mid = 25_000.0 * row_height;
            let (first, last) = window(rows, Viewport::scrolled(mid, viewport), row_height);
            assert!((25_000 - OVERSCAN..=25_000).contains(&first));
            assert!(last - first <= expected, "middle: {} rows", last - first);
            assert!(
                (last - first) as f32 * row_height >= viewport,
                "the window must cover the viewport it was asked about"
            );

            // Past the end clamps rather than panicking on the slice.
            let (first, last) = window(10, Viewport::scrolled(1_000_000.0, viewport), row_height);
            assert!(first <= 10 && last <= 10 && first <= last);
        }
    }

    /// No row paints into its neighbour (#538). Echo drew two body lines
    /// in a 20px row for months — 36px of text, unclipped, every preview
    /// under the next key. Each list now states what it holds (`ROW_TEXT`),
    /// and this holds the rows to it at both densities, chips included:
    /// a badge became a chip (#535), and a chip has height.
    #[test]
    fn rows_never_clip() {
        use super::super::tokens::{CAPTION_LINE, Spacing};
        use super::super::{echo, history, tree};
        use crate::prefs::Density;
        // Echo's head line is a body line with chips beside it — the line,
        // not the chip, sets its height, which `echo.rs` asserts at compile
        // time.
        for density in Density::ALL {
            let sp = Spacing::of(density);
            for (list, base, holds) in [
                ("tree", tree::ROW_HEIGHT, CHIP_HEIGHT.max(CAPTION_LINE)),
                ("echo", echo::ROW_HEIGHT, echo::ROW_TEXT),
                ("history", history::ROW_HEIGHT, history::ROW_TEXT),
            ] {
                let text = if list == "tree" { CAPTION_LINE } else { holds };
                let row = sp.row(base, text);
                assert!(
                    holds <= row,
                    "a {density:?} {list} row is {row}px and holds {holds}px"
                );
            }
        }
    }

    #[test]
    fn ages_are_one_short_word() {
        assert_eq!(age_word(0.4), "now");
        assert_eq!(age_word(2.9), "now");
        assert_eq!(age_word(12.0), "12s");
        assert_eq!(age_word(240.0), "4m");
        assert_eq!(age_word(7_300.0), "2h");
        assert_eq!(
            age_word(-1.0),
            "now",
            "a clock that stepped back is not the future"
        );
    }

    #[test]
    fn bytes_render_in_the_right_unit() {
        assert_eq!(human_bytes(0), "0 B");
        assert_eq!(human_bytes(512), "512 B");
        assert_eq!(human_bytes(1024), "1.0 KiB");
        assert_eq!(human_bytes(1536), "1.5 KiB");
        assert_eq!(human_bytes(1024 * 1024), "1.0 MiB");
        assert_eq!(human_bytes(1024 * 1024 * 1024), "1.0 GiB");
        // Saturates at the largest unit rather than running off the table.
        assert!(human_bytes(u64::MAX).ends_with("TiB"));
    }

    #[test]
    fn counts_are_pluralised() {
        assert_eq!(plural(0, "key"), "0 keys");
        assert_eq!(plural(1, "key"), "1 key");
        assert_eq!(plural(2, "key"), "2 keys");
    }

    /// Zero rate is "—", not "0.0/s": a key with no recent traffic has no
    /// measured rate, which is not the same as a measured rate of zero.
    #[test]
    fn a_missing_rate_is_not_reported_as_zero() {
        assert_eq!(human_rate(0.0), "—");
        assert_eq!(human_rate(-1.0), "—");
        assert_eq!(human_rate(0.25), "0.25/s");
        assert_eq!(human_rate(12.0), "12.0/s");
        assert_eq!(human_rate(5000.0), "5.0k/s");
    }
}

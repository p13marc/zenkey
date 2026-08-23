//! Small shared widgets.
//!
//! Together with [`super::theme`] this is the only other place allowed to name
//! a color — and the only place allowed to call `text(` at all (#191): every
//! view builds its text through the five role constructors below, so the type
//! scale is assigned by role, not by taste, and a grep can enforce it.
//!
//! The same discipline covers the two other encodings a call site used to be
//! able to get wrong (#193): a badge's glyph comes from its tone enum, never
//! from the call site, so colour cannot be the only carrier and two states of
//! a scale cannot share a mark; and every interactive element is built by a
//! `kit::` constructor, so the `button::Status` set and the [`focus_ring`]
//! are each handled exactly once (`check-interactive.sh` enforces the seam).

use iced::widget::text::IntoFragment;
use iced::widget::{
    Button, Checkbox, PickList, Slider, Text, TextInput, button, column, container, row, text,
};
use iced::{Border, Color, Element, Length};

use super::theme::colors;
use super::tokens::{font, space};

/// The subject (`font::TITLE`). One per window: the selected key or producer
/// in the location bar. If two of these are visible, one of them is lying
/// about being the subject.
pub fn title<'a>(s: impl IntoFragment<'a>) -> Text<'a> {
    text(s).size(font::TITLE)
}

/// A dock or pane header (`font::SECTION`). See also [`section_header`],
/// which pairs one of these with trailing controls.
pub fn section<'a>(s: impl IntoFragment<'a>) -> Text<'a> {
    text(s).size(font::SECTION)
}

/// Emphasis (`font::EMPHASIS`): card titles, key values, the number in a
/// stat tile, the roster origin.
pub fn emphasis<'a>(s: impl IntoFragment<'a>) -> Text<'a> {
    text(s).size(font::EMPHASIS)
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
                    width: 1.0,
                    radius: 4.0.into(),
                },
                ..container::Style::default()
            }
        })
        .into()
}

/// One tab of the right-pane strip: the active tab reads `primary` on the
/// pane surface, inactive tabs read muted with no background — and hovering
/// an inactive one paints the hover wash, like every interactive element
/// (#193).
pub fn tab<'a, M: Clone + 'a>(
    label: impl Into<String>,
    active: bool,
    on_press: M,
) -> Element<'a, M> {
    button(caption(label.into()))
        .padding([2, 8])
        .style(move |theme: &iced::Theme, status| {
            let c = colors(theme);
            let mut style = button::Style {
                background: active.then(|| c.surface().into()),
                text_color: if active { c.primary() } else { c.text_muted() },
                border: Border {
                    color: if active {
                        c.border()
                    } else {
                        Color::TRANSPARENT
                    },
                    width: 1.0,
                    radius: 4.0.into(),
                },
                ..Default::default()
            };
            match status {
                button::Status::Active => {}
                button::Status::Hovered | button::Status::Pressed if !active => {
                    style.background = Some(c.hover().into());
                    style.text_color = c.text();
                }
                button::Status::Hovered | button::Status::Pressed => {}
                button::Status::Disabled => style.text_color = c.text_dim(),
            }
            style
        })
        .on_press(on_press)
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

/// The one badge renderer (#193): glyph **and** word, colour on both.
///
/// Private, because the glyph is not a parameter any call site gets to pass —
/// each public badge constructor derives it from its tone enum, so a badge
/// with colour alone, or two states of a scale sharing a glyph, cannot be
/// written.
fn glyph_badge<'a, M: 'a>(
    glyph: &'static str,
    label: impl Into<String>,
    style: impl Fn(&iced::Theme) -> text::Style + Clone + 'a,
) -> Element<'a, M> {
    row![
        caption(glyph).style(style.clone()),
        caption(label.into()).style(style),
    ]
    .spacing(space::XS)
    .align_y(iced::Alignment::Center)
    .into()
}

/// A registration badge, theme-resolved. The glyph comes from the tone
/// (#193): passing a pre-resolved `Color` would bake in one theme's palette,
/// and passing a glyph would let two states share one.
pub fn tone_badge<'a, M: 'a>(
    tone: super::theme::RegistrationTone,
    label: impl Into<String>,
) -> Element<'a, M> {
    glyph_badge(tone.glyph(), label, move |theme: &iced::Theme| {
        text::Style {
            color: Some(colors(theme).registration(tone)),
        }
    })
}

/// A presence badge (#61) — theme-resolved like [`tone_badge`], glyph from
/// the tone.
pub fn badge_presence<'a, M: 'a>(
    tone: super::theme::PresenceTone,
    label: impl Into<String>,
) -> Element<'a, M> {
    glyph_badge(tone.glyph(), label, move |theme: &iced::Theme| {
        text::Style {
            color: Some(colors(theme).presence(tone)),
        }
    })
}

/// A doctor-severity badge (#71) — theme-resolved like [`tone_badge`],
/// carrying the CLI's severity marks via the tone.
pub fn badge_severity<'a, M: 'a>(
    tone: super::theme::SeverityTone,
    label: impl Into<String>,
) -> Element<'a, M> {
    glyph_badge(tone.glyph(), label, move |theme: &iced::Theme| {
        text::Style {
            color: Some(colors(theme).severity(tone)),
        }
    })
}

/// A storage-coverage badge (#70) — theme-resolved like [`tone_badge`], and
/// carrying the CLI's own mark via the tone so the two explorers read alike.
/// Mark **and** text, never colour alone.
pub fn badge_coverage<'a, M: 'a>(
    tone: super::theme::CoverageTone,
    label: impl Into<String>,
) -> Element<'a, M> {
    glyph_badge(tone.glyph(), label, move |theme: &iced::Theme| {
        text::Style {
            color: Some(colors(theme).coverage(tone)),
        }
    })
}

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
        width: 2.0,
        radius: 4.0.into(),
    }
}

/// A chrome action button — "run", "save", "send".
///
/// Call sites chain `.padding` / `.on_press` / `.width`; the four statuses
/// are decided here. No `on_press` renders as `Disabled`, dimmed — still
/// words, not just a colour change.
pub fn action<'a, M: 'a>(content: impl Into<Element<'a, M>>) -> Button<'a, M> {
    button(content.into()).style(|theme: &iced::Theme, status| {
        let c = colors(theme);
        let base = button::Style {
            background: Some(c.primary().into()),
            text_color: c.on_primary(),
            border: Border {
                color: Color::TRANSPARENT,
                width: 0.0,
                radius: 4.0.into(),
            },
            ..button::Style::default()
        };
        match status {
            button::Status::Active => base,
            button::Status::Hovered | button::Status::Pressed => button::Style {
                background: Some(c.primary_strong().into()),
                ..base
            },
            button::Status::Disabled => button::Style {
                background: Some(c.surface().into()),
                text_color: c.text_dim(),
                ..base
            },
        }
    })
}

/// An inline, text-like button — breadcrumb chunks, the ▸/▾ expand markers,
/// the ◉/○ watch toggles, a pane's "×".
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
                radius: 4.0.into(),
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
                Some(c.border().into())
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
                    radius: 4.0.into(),
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
                width: 1.0,
                radius: 4.0.into(),
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
                width: 1.0,
                radius: 4.0.into(),
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
                width: 1.0,
                radius: 2.0.into(),
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
    caption(s.into()).font(iced::Font::MONOSPACE).into()
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

/// Human-readable byte count.
pub fn human_bytes(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut v = n as f64;
    let mut unit = 0;
    while v >= 1024.0 && unit < UNITS.len() - 1 {
        v /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{n} B")
    } else {
        format!("{v:.1} {}", UNITS[unit])
    }
}

/// `n` of something, pluralised. "1 keys" reads as a bug in the tool.
pub fn plural(n: usize, singular: &str) -> String {
    if n == 1 {
        format!("{n} {singular}")
    } else {
        format!("{n} {singular}s")
    }
}

/// Human-readable rate.
pub fn human_rate(hz: f64) -> String {
    if hz <= 0.0 {
        "—".to_string()
    } else if hz < 1.0 {
        format!("{hz:.2}/s")
    } else if hz < 1000.0 {
        format!("{hz:.1}/s")
    } else {
        format!("{:.1}k/s", hz / 1000.0)
    }
}

/// Rows rendered beyond each window edge, so scrolling never shows a gap.
pub(crate) const OVERSCAN: usize = 8;

/// The visible row window for a scroll position: `(first, last)` indices into
/// a fixed-height row list, with overscan.
///
/// Pure, so it is testable without a renderer — and shared, so the three
/// virtualized lists cannot drift into three different ideas of "visible".
/// The tree had this and nothing else did: Echo capped *drawing* at 300 rows
/// with no window, and History drew every retained entry (#183).
///
/// A list of `rows` rows is `rows * row_height` tall whether it is drawn or
/// not, which is what the spacers above and below the window are for. Callers
/// that forget them get a window that scrolls against the wrong extent.
pub fn window(rows: usize, scroll_y: f32, viewport_h: f32, row_height: f32) -> (usize, usize) {
    let first = (scroll_y / row_height).floor() as usize;
    let visible = (viewport_h / row_height).ceil() as usize + 1;
    let first = first.saturating_sub(OVERSCAN);
    let last = (first + visible + 2 * OVERSCAN).min(rows);
    (first.min(rows), last)
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
        for row_height in [20.0_f32, 24.0, 34.0] {
            let rows = 50_000;
            let viewport = 600.0;
            let expected = (viewport / row_height).ceil() as usize + 1 + 2 * OVERSCAN;

            let (first, last) = window(rows, 0.0, viewport, row_height);
            assert_eq!(first, 0);
            assert!(last - first <= expected, "top: {} rows", last - first);

            let mid = 25_000.0 * row_height;
            let (first, last) = window(rows, mid, viewport, row_height);
            assert!((25_000 - OVERSCAN..=25_000).contains(&first));
            assert!(last - first <= expected, "middle: {} rows", last - first);
            assert!(
                (last - first) as f32 * row_height >= viewport,
                "the window must cover the viewport it was asked about"
            );

            // Past the end clamps rather than panicking on the slice.
            let (first, last) = window(10, 1_000_000.0, viewport, row_height);
            assert!(first <= 10 && last <= 10 && first <= last);
        }
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

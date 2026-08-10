//! Small shared widgets.
//!
//! Together with [`super::theme`] this is the only other place allowed to name
//! a color.

use iced::widget::{column, container, row, text};
use iced::{Border, Color, Element, Length};

use super::theme::colors;
use super::tokens::{font, space};

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
/// pane surface, inactive tabs read muted with no background.
pub fn tab<'a, M: Clone + 'a>(
    label: impl Into<String>,
    active: bool,
    on_press: M,
) -> Element<'a, M> {
    iced::widget::button(text(label.into()).size(font::CAPTION))
        .padding([2, 8])
        .style(move |theme: &iced::Theme, _status| {
            let c = colors(theme);
            iced::widget::button::Style {
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
            }
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
        text(title.into())
            .size(font::SECTION)
            .style(|theme: &iced::Theme| text::Style {
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

/// A small colored label.
///
/// Carries a dot **and** text: meaning is never conveyed by color alone, which
/// matters here because the registration states are exactly the kind of thing a
/// colorblind user would otherwise have to guess at.
pub fn badge<'a, M: 'a>(color: Color, label: impl Into<String>) -> Element<'a, M> {
    row![
        text("●")
            .size(font::CAPTION)
            .style(move |_: &iced::Theme| text::Style { color: Some(color) }),
        text(label.into())
            .size(font::CAPTION)
            .style(move |_: &iced::Theme| text::Style { color: Some(color) }),
    ]
    .spacing(space::XS)
    .align_y(iced::Alignment::Center)
    .into()
}

/// A badge whose color is resolved against the live theme.
///
/// Prefer this over [`badge`] wherever the color is semantic: passing a
/// pre-resolved `Color` would bake in one theme's palette.
pub fn tone_badge<'a, M: 'a>(
    tone: super::theme::RegistrationTone,
    label: impl Into<String>,
) -> Element<'a, M> {
    let style = move |theme: &iced::Theme| text::Style {
        color: Some(colors(theme).registration(tone)),
    };
    row![
        text("●").size(font::CAPTION).style(style),
        text(label.into()).size(font::CAPTION).style(style),
    ]
    .spacing(space::XS)
    .align_y(iced::Alignment::Center)
    .into()
}

/// A presence badge (#61) — theme-resolved like [`tone_badge`].
pub fn badge_presence<'a, M: 'a>(
    tone: super::theme::PresenceTone,
    label: impl Into<String>,
) -> Element<'a, M> {
    let style = move |theme: &iced::Theme| text::Style {
        color: Some(colors(theme).presence(tone)),
    };
    row![
        text("●").size(font::CAPTION).style(style),
        text(label.into()).size(font::CAPTION).style(style),
    ]
    .spacing(space::XS)
    .align_y(iced::Alignment::Center)
    .into()
}

/// A doctor-severity badge (#71) — theme-resolved like [`tone_badge`],
/// mirroring the CLI's severity marks.
pub fn badge_severity<'a, M: 'a>(
    tone: super::theme::SeverityTone,
    label: impl Into<String>,
) -> Element<'a, M> {
    let style = move |theme: &iced::Theme| text::Style {
        color: Some(colors(theme).severity(tone)),
    };
    let mark = match tone {
        super::theme::SeverityTone::Error => "✗",
        super::theme::SeverityTone::Warning => "⚠",
        super::theme::SeverityTone::Info => "·",
    };
    row![
        text(mark).size(font::CAPTION).style(style),
        text(label.into()).size(font::CAPTION).style(style),
    ]
    .spacing(space::XS)
    .align_y(iced::Alignment::Center)
    .into()
}

/// A storage-coverage badge (#70) — theme-resolved like [`tone_badge`], and
/// carrying the CLI's own mark so the two explorers read alike. Mark **and**
/// text, never colour alone.
pub fn badge_coverage<'a, M: 'a>(
    tone: super::theme::CoverageTone,
    label: impl Into<String>,
) -> Element<'a, M> {
    let style = move |theme: &iced::Theme| text::Style {
        color: Some(colors(theme).coverage(tone)),
    };
    let mark = match tone {
        super::theme::CoverageTone::Covered => "✓",
        super::theme::CoverageTone::Partial => "~",
        super::theme::CoverageTone::Uncovered => "·",
    };
    row![
        text(mark).size(font::CAPTION).style(style),
        text(label.into()).size(font::CAPTION).style(style),
    ]
    .spacing(space::XS)
    .align_y(iced::Alignment::Center)
    .into()
}

/// Dimmed caption text.
pub fn muted<'a, M: 'a>(s: impl Into<String>) -> Element<'a, M> {
    text(s.into())
        .size(font::CAPTION)
        .style(|theme: &iced::Theme| text::Style {
            color: Some(colors(theme).text_muted()),
        })
        .into()
}

/// Monospaced body text — keys, payload previews.
pub fn mono<'a, M: 'a>(s: impl Into<String>) -> Element<'a, M> {
    text(s.into())
        .size(font::CAPTION)
        .font(iced::Font::MONOSPACE)
        .into()
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
            text(headline.into())
                .size(font::BODY)
                .style(|theme: &iced::Theme| text::Style {
                    color: Some(colors(theme).text_muted()),
                }),
            muted(why.into()),
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

#[cfg(test)]
mod tests {
    use super::*;

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

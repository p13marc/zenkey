//! The live echo pane.

use iced::widget::{Column, column, row, text, text_input};
use iced::{Element, Length};

use crate::echo::{EchoLine, EchoRing};
use crate::message::Message;
use crate::view::kit::{self, human_bytes};
use crate::view::theme::colors;
use crate::view::tokens::{font, space};

/// How many lines to draw. The ring may hold more; drawing thousands of rows
/// per frame is what makes a GUI feel broken under load.
const VISIBLE: usize = 300;

/// Does this line pass the current filter?
///
/// Substring over key and preview, case-insensitive. Deliberately not a key
/// expression: filtering here is a *display* concern over what was already
/// received, and conflating it with the subscription scope would let a user
/// think they had narrowed the bus when they had only narrowed the view.
pub fn matches(line: &EchoLine, filter: &str) -> bool {
    if filter.is_empty() {
        return true;
    }
    let needle = filter.to_lowercase();
    line.key.to_lowercase().contains(&needle) || line.preview.to_lowercase().contains(&needle)
}

/// Render the echo pane.
pub fn pane<'a>(
    ring: &'a EchoRing,
    filter: &'a str,
    key_filter: Option<&'a str>,
) -> Element<'a, Message> {
    let header = kit::section_header(
        "Echo",
        Some(
            row![
                text_input("filter…", filter)
                    .on_input(Message::EchoFilterChanged)
                    .size(font::CAPTION)
                    .width(Length::Fixed(220.0)),
                iced::widget::button(text("clear").size(font::CAPTION))
                    .on_press(Message::ClearEcho)
                    .padding(4),
            ]
            .spacing(space::SM)
            .align_y(iced::Alignment::Center)
            .into(),
        ),
    );

    let mut shown = 0usize;
    let mut body = Column::new().spacing(1);
    for line in ring.iter() {
        if shown >= VISIBLE {
            break;
        }
        if let Some(k) = key_filter
            && !line.key.starts_with(k)
        {
            continue;
        }
        if !matches(line, filter) {
            continue;
        }
        shown += 1;
        body = body.push(line_view(line));
    }

    let content: Element<'_, Message> = if shown == 0 {
        kit::empty_state(
            "No matching samples",
            if ring.is_empty() {
                "Nothing has arrived on the current scope yet. That is not a \
                 statement about the bus (RFC 05 §3.1)."
            } else {
                "The ring holds samples, but none match the current filter."
            },
        )
    } else {
        iced::widget::scrollable(body).height(Length::Fill).into()
    };

    column![header, loss_strip(ring), content]
        .spacing(space::SM)
        .into()
}

/// What was lost, and how. Always rendered — a zero here is information.
///
/// Three counters rather than one because they are three different failures:
/// `lagged` means the bus outran us, `coalesced` means one tick carried more
/// than the batch cap, `evicted` means the ring chose to forget old lines.
/// Collapsing them would hide which one happened.
fn loss_strip<'a>(ring: &EchoRing) -> Element<'a, Message> {
    if ring.lagged() == 0 && ring.evicted() == 0 {
        return kit::muted(format!("{} lines retained", ring.len()));
    }
    let msg = format!(
        "{} lines retained · {} dropped by the bus (we fell behind) · {} evicted (ring full)",
        ring.len(),
        ring.lagged(),
        ring.evicted(),
    );
    text(msg)
        .size(font::CAPTION)
        .style(|theme: &iced::Theme| text::Style {
            color: Some(colors(theme).warning()),
        })
        .into()
}

fn line_view(line: &EchoLine) -> Element<'_, Message> {
    let key = text(line.key.clone())
        .size(font::CAPTION)
        .font(iced::Font::MONOSPACE)
        .style(|theme: &iced::Theme| text::Style {
            color: Some(colors(theme).text()),
        });

    // A tombstone is authoritative retirement, not an empty value
    // (RFC 04 §1.2) — so it must not look like a put with no payload.
    let preview = text(line.preview.clone())
        .size(font::CAPTION)
        .font(iced::Font::MONOSPACE)
        .style(move |theme: &iced::Theme| text::Style {
            color: Some(if line_is_delete(line) {
                colors(theme).danger()
            } else {
                colors(theme).text_muted()
            }),
        });

    column![
        row![
            key,
            iced::widget::space::horizontal(),
            kit::muted(human_bytes(line.len as u64)),
        ]
        .spacing(space::SM),
        preview,
    ]
    .spacing(1)
    .padding(iced::Padding::from([2.0, 0.0]))
    .into()
}

fn line_is_delete(line: &EchoLine) -> bool {
    line.is_delete
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(key: &str, preview: &str) -> EchoLine {
        EchoLine {
            seq: 0,
            key: key.to_string(),
            preview: preview.to_string(),
            len: preview.len(),
            encoding: String::new(),
            is_delete: false,
            timestamp: None,
        }
    }

    #[test]
    fn an_empty_filter_matches_everything() {
        assert!(matches(&line("a/b", "x"), ""));
    }

    #[test]
    fn the_filter_covers_key_and_payload_case_insensitively() {
        let l = line("v1/h-abc/telemetry/sysinfo/CPU", "{\"Value\":3}");
        assert!(matches(&l, "cpu"));
        assert!(matches(&l, "SYSINFO"));
        assert!(matches(&l, "value"));
        assert!(!matches(&l, "nowhere"));
    }
}

//! The publish pane (§6.4 item 3's other half, issue #60) — the GUI face of
//! `zenctl topic pub`.
//!
//! Sharpened by the owner directive of 2026-08-09: this is a **data
//! publishing** surface, not a supervision convenience. That changes two
//! things about what the pane owes its user.
//!
//! **The body is encoded by the engine, never here.** The editor holds JSON;
//! `zenkey_fleet::prepare_publish` (#97) turns it into the bytes that ship,
//! against the producer's served schema, labelled with the declared encoding.
//! No GUI-side codec logic exists or may be added — a missing codec is a
//! zenkey-fleet issue (`docs/redesign-2026-07.md` §15). What the pane *does*
//! own is saying which of the three outcomes happened: encoded, sent as
//! typed, or sent raw. A payload that shipped unencoded and looked like one
//! that did not would be the write-side version of the O4 mistake this whole
//! GUI is built to avoid.
//!
//! **Publishing can be sustained.** A repeat interval drives a real stream
//! from the pane, and the publication's #38 matching badge says whether
//! anyone is consuming it — a routing fact about *this* publisher, never a
//! fleet verdict (RFC 05 §3.1). Every send is logged, the log is bounded, and
//! when the bound bites the pane reports what it dropped (RFC 09 §5.1 O6).

use std::collections::VecDeque;

use iced::widget::{Column, button, checkbox, column, pick_list, row, text, text_input};
use iced::{Element, Length};
use zenkey::qos::QosProfile;
use zenkey_fleet::{BodySource, KeyFacts};

use crate::message::Message;
use crate::view::kit;
use crate::view::theme::colors;
use crate::view::tokens::{font, space};

/// How many send-log lines the pane keeps. Bounded on purpose: a 5 Hz stream
/// left running overnight is 150k lines, and an explorer that grows without
/// bound is the O6 bug this project already fixed once for the key tree.
pub const LOG_LINES: usize = 200;

/// One line of the in-pane send log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogLine {
    pub text: String,
    /// Failures render differently — a send that errored must not read like
    /// one that landed.
    pub ok: bool,
}

/// A QoS profile wrapped for the picker (`pick_list` needs `Display`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QosChoice(pub QosProfile);

impl std::fmt::Display for QosChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0.name())
    }
}

/// Every profile, as picker options. Derived from the closed enum
/// (`QosProfile::ALL`), so the picker cannot drift from RFC 04 §3.
pub fn qos_choices() -> Vec<QosChoice> {
    QosProfile::ALL.into_iter().map(QosChoice).collect()
}

/// The pane's editable state (owned by the app).
#[derive(Debug, Clone)]
pub struct PublishForm {
    pub key: String,
    pub body: String,
    pub qos: QosChoice,
    /// Optional `Encoding` override; empty means "let the registry and the
    /// schema kind decide" (the engine's ladder).
    pub encoding: String,
    /// Send the bytes verbatim — the explicit escape hatch, mirroring
    /// `zenctl topic pub --raw`.
    pub raw: bool,
    /// Keep publishing on an interval once armed.
    pub repeat: bool,
    pub interval: String,
    /// Live classification of the composed key (the same ladder the tree
    /// uses). `None` before anything is typed.
    pub facts: Option<KeyFacts>,
    /// How the last prepared body was produced — the honesty line.
    pub source: Option<BodySource>,
    /// The engine's note about that preparation, rendered verbatim.
    pub note: Option<String>,
    /// The encoding actually set on the wire, once resolved.
    pub encoding_used: Option<String>,
    /// A publication is armed and repeating.
    pub armed: bool,
    /// Matching status of the armed publication (#38). `None` = **not asked**,
    /// which is not "nobody listens" (O4).
    pub matching: Option<bool>,
    /// Newest first, bounded by [`LOG_LINES`].
    pub log: VecDeque<LogLine>,
    /// How many lines the bound has discarded (O6).
    pub dropped: u64,
    pub in_flight: bool,
    /// The last refusal, when a send did not happen at all.
    pub error: Option<String>,
}

impl Default for PublishForm {
    fn default() -> Self {
        PublishForm {
            key: String::new(),
            body: String::new(),
            qos: QosChoice(QosProfile::Sampled),
            encoding: String::new(),
            raw: false,
            repeat: false,
            interval: "1.0".into(),
            facts: None,
            source: None,
            note: None,
            encoding_used: None,
            armed: false,
            matching: None,
            log: VecDeque::new(),
            dropped: 0,
            in_flight: false,
            error: None,
        }
    }
}

impl PublishForm {
    /// Append a line, dropping the oldest when the bound bites — and counting
    /// what it dropped, because a silently-trimmed log claims a completeness
    /// it does not have (O6).
    pub fn log(&mut self, ok: bool, text: impl Into<String>) {
        self.log.push_front(LogLine {
            text: text.into(),
            ok,
        });
        while self.log.len() > LOG_LINES {
            self.log.pop_back();
            self.dropped += 1;
        }
    }

    /// The repeat interval in seconds, floored at 100 ms so a typo cannot
    /// turn the pane into a load generator.
    pub fn interval_secs(&self) -> f64 {
        self.interval.trim().parse::<f64>().unwrap_or(1.0).max(0.1)
    }
}

/// Messages the pane emits (wrapped into the app's `Message::Publish`).
#[derive(Debug, Clone)]
pub enum PublishMsg {
    KeyChanged(String),
    BodyChanged(String),
    QosPicked(QosChoice),
    EncodingChanged(String),
    RawToggled(bool),
    RepeatToggled(bool),
    IntervalChanged(String),
    /// Prepare, declare, and send once (arming the repeat if it is on).
    Send,
    /// Undeclare the armed publication.
    Stop,
}

fn msg(m: PublishMsg) -> Message {
    Message::Publish(m)
}

pub fn pane<'a>(form: &'a PublishForm, slices_loaded: bool) -> Element<'a, Message> {
    let key = text_input("key: full wire key to publish on", &form.key)
        .on_input(|t| msg(PublishMsg::KeyChanged(t)))
        .size(font::CAPTION);

    // KeyFacts feedback as you type — the same classification ladder the tree
    // renders, so a key that will not refine says so *before* the send.
    let mut facts_col = Column::new().spacing(2);
    match (&form.facts, form.key.trim().is_empty()) {
        (_, true) => {
            facts_col = facts_col.push(kit::muted(
                "a full wire key, base included — this session is un-namespaced (RFC 09 §5)",
            ));
        }
        (Some(f), false) => facts_col = facts_col.push(crate::view::detail::facts_section(f)),
        (None, false) => {}
    }
    if !slices_loaded && !form.key.trim().is_empty() {
        facts_col = facts_col.push(kit::muted(
            "no registry loaded — the body cannot be schema-checked, and that is \
             \"not asked\", not \"unregistered\" (O4)",
        ));
    }

    let body = text_input(
        "body: JSON (encoded for the wire by the engine)",
        &form.body,
    )
    .on_input(|t| msg(PublishMsg::BodyChanged(t)))
    .size(font::CAPTION);

    let qos = pick_list(qos_choices(), Some(form.qos), |q| {
        msg(PublishMsg::QosPicked(q))
    })
    .placeholder("qos")
    .text_size(font::CAPTION);

    let encoding = text_input("encoding override (optional)", &form.encoding)
        .on_input(|t| msg(PublishMsg::EncodingChanged(t)))
        .size(font::CAPTION);

    let raw = checkbox(form.raw)
        .label("send raw")
        .on_toggle(|b| msg(PublishMsg::RawToggled(b)))
        .text_size(font::CAPTION);
    let repeat = checkbox(form.repeat)
        .label("repeat")
        .on_toggle(|b| msg(PublishMsg::RepeatToggled(b)))
        .text_size(font::CAPTION);
    let interval = text_input("interval (s)", &form.interval)
        .on_input(|t| msg(PublishMsg::IntervalChanged(t)))
        .size(font::CAPTION)
        .width(Length::Fixed(90.0));

    let ready = !form.key.trim().is_empty() && !form.in_flight;
    let mut send = button(
        text(if form.in_flight {
            "sending…"
        } else if form.armed {
            "re-send"
        } else {
            "send"
        })
        .size(font::CAPTION),
    )
    .padding(4);
    if ready {
        send = send.on_press(msg(PublishMsg::Send));
    }
    let mut controls = row![send].spacing(space::SM);
    if form.armed {
        controls = controls.push(
            button(text("stop").size(font::CAPTION))
                .padding(4)
                .on_press(msg(PublishMsg::Stop)),
        );
    }

    let mut col = column![
        kit::section_header("Publish", None),
        key,
        facts_col,
        body,
        row![qos, encoding].spacing(space::SM),
        row![raw, repeat, interval]
            .spacing(space::SM)
            .align_y(iced::Alignment::Center),
        controls,
    ]
    .spacing(space::SM);

    if let Some(e) = &form.error {
        col = col.push(text(format!("refused: {e}")).size(font::CAPTION).style(
            |theme: &iced::Theme| text::Style {
                color: Some(colors(theme).danger()),
            },
        ));
    }
    if let Some(line) = provenance(form) {
        col = col.push(line);
    }
    if let Some(note) = &form.note {
        col = col.push(kit::muted(note.clone()));
    }
    if form.armed {
        col = col.push(kit::muted(matching_sentence(form.matching)));
    }
    col = col.push(log_view(form));

    iced::widget::scrollable(col).height(Length::Fill).into()
}

/// How the last body reached the wire. Encoded and as-typed must never look
/// alike — that is the whole reason `BodySource` leaves the engine.
fn provenance(form: &PublishForm) -> Option<Element<'_, Message>> {
    let source = form.source.as_ref()?;
    let encoding = form.encoding_used.as_deref().unwrap_or("(no encoding set)");
    Some(match source {
        BodySource::Encoded { type_name } => {
            kit::muted(format!("encoded as {type_name} → {encoding}"))
        }
        BodySource::AsTyped => kit::muted(format!("sent as typed → {encoding}")),
        BodySource::Raw => kit::muted("sent raw — bytes verbatim, not encoded".to_string()),
    })
}

/// The #38 badge, worded exactly as the CLI words it: a routing fact about
/// *this* publisher. `false` is never a claim about the fleet, and `None` is
/// never rendered as `false`.
fn matching_sentence(matching: Option<bool>) -> String {
    match matching {
        Some(true) => "matching: a subscriber currently matches this publication".into(),
        Some(false) => "matching: no subscriber currently matches this publication — a \
                        routing fact about this publisher, not a fleet verdict (RFC 05 §3.1)"
            .into(),
        None => "matching: not asked".into(),
    }
}

fn log_view(form: &PublishForm) -> Element<'_, Message> {
    let mut col = Column::new().spacing(2);
    if form.log.is_empty() {
        return col.push(kit::muted("no sends yet")).into();
    }
    col = col.push(kit::muted(if form.dropped > 0 {
        format!(
            "send log — {} shown, {} dropped (bounded at {LOG_LINES})",
            form.log.len(),
            form.dropped
        )
    } else {
        format!("send log — {}", kit::plural(form.log.len(), "entry"))
    }));
    for line in &form.log {
        col = col.push(if line.ok {
            kit::mono(line.text.clone())
        } else {
            text(line.text.clone())
                .size(font::CAPTION)
                .font(iced::Font::MONOSPACE)
                .style(|theme: &iced::Theme| text::Style {
                    color: Some(colors(theme).danger()),
                })
                .into()
        });
    }
    col.into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_log_is_bounded_and_counts_what_it_dropped() {
        let mut form = PublishForm::default();
        for i in 0..(LOG_LINES + 5) {
            form.log(true, format!("send {i}"));
        }
        assert_eq!(form.log.len(), LOG_LINES);
        assert_eq!(form.dropped, 5);
        // Newest first: the last send is the head.
        assert_eq!(form.log[0].text, format!("send {}", LOG_LINES + 4));
    }

    #[test]
    fn the_interval_floors_a_typo_instead_of_spinning() {
        let mut form = PublishForm::default();
        assert_eq!(form.interval_secs(), 1.0);
        form.interval = "0".into();
        assert_eq!(form.interval_secs(), 0.1);
        form.interval = "not a number".into();
        assert_eq!(form.interval_secs(), 1.0);
        form.interval = "2.5".into();
        assert_eq!(form.interval_secs(), 2.5);
    }

    /// The picker is the enum, not a copy of it.
    #[test]
    fn the_qos_picker_offers_exactly_the_closed_vocabulary() {
        let names: Vec<&str> = qos_choices().iter().map(|c| c.0.name()).collect();
        assert_eq!(
            names,
            ["sampled", "refreshed", "transition", "alert", "frame"]
        );
    }
}

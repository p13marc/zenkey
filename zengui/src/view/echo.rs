//! The live echo pane (issue #72, echo v2).
//!
//! The bootstrap shipped a scrollback with a substring filter. What made it
//! not a daily driver was the three things a busy bus does to you: it scrolls
//! the line you found off the top, it gives you no way to look *into* a line,
//! and it gives you no way to get the lines out. This pane closes those, and
//! sharpens the filter from "substring" to "a key expression **and** a
//! substring", which are different questions and were being asked as one.
//!
//! The honesty rule that shapes all of it: **pausing does not stop the bus.**
//! A frozen view over a ring that keeps filling is a lie unless the gap is
//! reported, so [`EchoView::paused_gap`] exists and the strip renders it —
//! RFC 09 §5.1 O6, applied to a scrollback the user deliberately stopped.

use iced::widget::{Column, button, column, row, text, text_input};
use iced::{Element, Length};

use crate::echo::{EchoLine, EchoRing};
use crate::message::Message;
use crate::view::kit::{self, human_bytes};
use crate::view::theme::colors;
use crate::view::tokens::{font, space};

/// How many lines to draw. The ring may hold more; drawing thousands of rows
/// per frame is what makes a GUI feel broken under load.
const VISIBLE: usize = 300;

/// The pane's view state (owned by the app).
#[derive(Debug, Clone, Default)]
pub struct EchoView {
    /// Substring over key and payload.
    pub filter: String,
    /// A key expression narrowing the view — *display* only. Empty = no
    /// key filter.
    pub key_filter: String,
    /// Why the key filter is not in force, when it does not parse. Shown
    /// rather than silently ignored: a filter that looks applied and is not
    /// would make the pane claim a narrowing it never did.
    pub key_filter_error: Option<String>,
    /// Follow the tail. `false` freezes the view at [`Self::paused_at`].
    pub following: bool,
    /// The ring sequence the view is frozen at, while paused.
    pub paused_at: Option<u64>,
    /// How many samples arrived during the *last* pause, reported once on
    /// resume so a gap in the scrollback is never silent.
    pub resumed_gap: Option<u64>,
}

impl EchoView {
    pub fn new() -> EchoView {
        EchoView {
            following: true,
            ..EchoView::default()
        }
    }

    /// Freeze at the ring's current head.
    pub fn pause(&mut self, next_seq: u64) {
        self.following = false;
        self.paused_at = Some(next_seq);
        self.resumed_gap = None;
    }

    /// Resume, recording what arrived while frozen.
    pub fn resume(&mut self, next_seq: u64) {
        self.resumed_gap = self.paused_at.map(|at| next_seq.saturating_sub(at));
        self.following = true;
        self.paused_at = None;
    }

    /// How many samples have arrived since the freeze, right now.
    pub fn paused_gap(&self, next_seq: u64) -> Option<u64> {
        self.paused_at.map(|at| next_seq.saturating_sub(at))
    }

    /// Set the key filter, validating it. An invalid expression is *recorded*
    /// and not applied — the same posture the scope validator takes, because
    /// a filter silently doing nothing is worse than one that says why.
    pub fn set_key_filter(&mut self, value: String) {
        self.key_filter_error = if value.trim().is_empty() {
            None
        } else {
            crate::scope::validate_selector(&value)
                .err()
                .map(|e| e.to_string())
        };
        self.key_filter = value;
    }

    /// The compiled key filter, when one is in force.
    fn key_expr(&self) -> Option<zenoh::key_expr::KeyExpr<'_>> {
        if self.key_filter.trim().is_empty() || self.key_filter_error.is_some() {
            return None;
        }
        zenoh::key_expr::KeyExpr::new(self.key_filter.as_str()).ok()
    }

    /// Does this line pass every filter in force?
    ///
    /// The key filter is a **key-expression intersection**, not a prefix
    /// match: `v1/*/state/**` is the question an operator means, and
    /// `starts_with` would answer a different one.
    pub fn admits(&self, line: &EchoLine, selection: Option<&str>) -> bool {
        if let Some(sel) = selection
            && line.key != sel
        {
            // A tree selection narrows the pane to that key exactly.
            return false;
        }
        if let Some(filter) = self.key_expr()
            && !zenoh::key_expr::KeyExpr::new(line.key.as_str())
                .map(|k| filter.intersects(&k))
                .unwrap_or(false)
        {
            return false;
        }
        matches(line, &self.filter)
    }
}

/// Does this line pass the substring filter?
///
/// Case-insensitive over key and preview. Deliberately *not* the key filter's
/// job: substring-over-payload and key-expression-over-key are different
/// questions, and one box asking both was how the bootstrap let a user think
/// they had narrowed the bus when they had only narrowed the view.
pub fn matches(line: &EchoLine, filter: &str) -> bool {
    if filter.is_empty() {
        return true;
    }
    let needle = filter.to_lowercase();
    line.key.to_lowercase().contains(&needle) || line.preview.to_lowercase().contains(&needle)
}

/// The lines a given view shows, newest first, bounded by the draw cap.
///
/// Returns `(lines, matched)` — `matched` counts everything that passed the
/// filters, so the strip can say "showing N of M" rather than implying the
/// bound is the answer.
pub fn visible<'a>(
    ring: &'a EchoRing,
    view: &EchoView,
    selection: Option<&str>,
) -> (Vec<&'a EchoLine>, usize) {
    let mut lines = Vec::new();
    let mut matched = 0usize;
    for line in ring.iter() {
        // While paused, anything newer than the freeze point is invisible —
        // but it is *counted*, which is what the gap report is made of.
        if let Some(at) = view.paused_at
            && line.seq >= at
        {
            continue;
        }
        if !view.admits(line, selection) {
            continue;
        }
        matched += 1;
        if lines.len() < VISIBLE {
            lines.push(line);
        }
    }
    (lines, matched)
}

/// One line in `zenctl topic echo --format ndjson`'s row shape (#72).
///
/// The **shared** fields are byte-for-byte the CLI's — `key`, `origin`,
/// `subject`, `encoding`, `timestamp`, `value` — so one `jq` script reads a
/// GUI export and a CLI pipe alike. The two differ in exactly the places the
/// two tools differ, and neither difference is faked:
///
/// - the CLI carries `type`/`typed` from its schema decode; that decode is
///   async and must never run on a render path, so it is **absent** here
///   rather than defaulted to `null`/`false`, which would claim the lookup
///   happened and found nothing;
/// - this row carries `bytes` and `delete`, which the ring knows and the CLI's
///   row does not spell — a tombstone in particular is authoritative
///   retirement (RFC 04 §1.2) and worth exporting as such.
pub fn ndjson_line(line: &EchoLine, base: &str) -> String {
    let parsed = zenkey::grammar::parse_full(base, &line.key);
    let obj = serde_json::json!({
        "key": line.key,
        "origin": parsed.as_ref().map(|p| p.origin.chunk().to_string()),
        "subject": parsed.as_ref().map(|p| p.subject.join("/")),
        "encoding": line.encoding,
        "timestamp": line.timestamp,
        "bytes": line.len,
        "delete": line.is_delete,
        "value": serde_json::from_str::<serde_json::Value>(&line.preview)
            .unwrap_or(serde_json::Value::String(line.preview.clone())),
    });
    obj.to_string()
}

/// Every visible line as ndjson, newest first.
pub fn export(ring: &EchoRing, view: &EchoView, selection: Option<&str>, base: &str) -> String {
    let (lines, _) = visible(ring, view, selection);
    let mut out = String::new();
    for line in lines {
        out.push_str(&ndjson_line(line, base));
        out.push('\n');
    }
    out
}

/// Messages the pane emits.
#[derive(Debug, Clone)]
pub enum EchoMsg {
    FilterChanged(String),
    KeyFilterChanged(String),
    FollowToggled,
    Clear,
    /// A line was clicked — drills through to the payload inspector.
    LineClicked(String),
    /// Copy the visible lines as ndjson to the clipboard.
    Export,
}

fn msg(m: EchoMsg) -> Message {
    Message::Echo(m)
}

/// Render the echo pane.
pub fn pane<'a>(
    ring: &'a EchoRing,
    view: &'a EchoView,
    selection: Option<&'a str>,
    next_seq: u64,
) -> Element<'a, Message> {
    let controls = row![
        text_input("filter payload/key…", &view.filter)
            .on_input(|t| msg(EchoMsg::FilterChanged(t)))
            .size(font::CAPTION)
            .width(Length::Fixed(170.0)),
        text_input("key expr, e.g. v1/*/state/**", &view.key_filter)
            .on_input(|t| msg(EchoMsg::KeyFilterChanged(t)))
            .size(font::CAPTION)
            .width(Length::Fixed(190.0)),
        button(text(if view.following { "pause" } else { "follow" }).size(font::CAPTION))
            .on_press(msg(EchoMsg::FollowToggled))
            .padding(4),
        button(text("ndjson").size(font::CAPTION))
            .on_press(msg(EchoMsg::Export))
            .padding(4),
        button(text("clear").size(font::CAPTION))
            .on_press(msg(EchoMsg::Clear))
            .padding(4),
    ]
    .spacing(space::SM)
    .align_y(iced::Alignment::Center);

    let header = kit::section_header("Echo", Some(controls.into()));

    let (lines, matched) = visible(ring, view, selection);
    let mut body = Column::new().spacing(1);
    for line in &lines {
        body = body.push(line_view(line));
    }

    let content: Element<'_, Message> = if lines.is_empty() {
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

    let mut col = column![header];
    if let Some(err) = &view.key_filter_error {
        col = col.push(
            text(format!("key filter not applied: {err}"))
                .size(font::CAPTION)
                .style(|theme: &iced::Theme| text::Style {
                    color: Some(colors(theme).danger()),
                }),
        );
    }
    col = col.push(state_strip(ring, view, matched, lines.len(), next_seq));
    col = col.push(loss_strip(ring));
    col.push(content).spacing(space::SM).into()
}

/// What the view is doing to the ring: how much of it is on screen, and — the
/// load-bearing half — what a pause is currently hiding.
fn state_strip<'a>(
    ring: &EchoRing,
    view: &EchoView,
    matched: usize,
    shown: usize,
    next_seq: u64,
) -> Element<'a, Message> {
    let mut parts = vec![format!(
        "showing {shown} of {matched} matched · {} retained",
        ring.len()
    )];
    if let Some(gap) = view.paused_gap(next_seq) {
        // The whole reason pause is safe to offer: the number it is hiding is
        // on screen while it hides it.
        parts.push(format!("paused — {gap} arrived since"));
    } else if let Some(gap) = view.resumed_gap {
        parts.push(format!("resumed — {gap} arrived while paused"));
    }
    let paused = view.paused_at.is_some();
    text(parts.join(" · "))
        .size(font::CAPTION)
        .style(move |theme: &iced::Theme| text::Style {
            color: Some(if paused {
                colors(theme).warning()
            } else {
                colors(theme).text_muted()
            }),
        })
        .into()
}

/// What was lost, and how. Always rendered — a zero here is information.
///
/// Three counters rather than one because they are three different failures:
/// `lagged` means the bus outran us, `coalesced` means one tick carried more
/// than the batch cap, `evicted` means the ring chose to forget old lines.
/// Collapsing them would hide which one happened.
fn loss_strip<'a>(ring: &EchoRing) -> Element<'a, Message> {
    if ring.lagged() == 0 && ring.evicted() == 0 && ring.coalesced() == 0 {
        return kit::muted("nothing dropped");
    }
    let msg = format!(
        "{} dropped by the bus (we fell behind) · {} coalesced (tick batch cap) · \
         {} evicted (ring full)",
        ring.lagged(),
        ring.coalesced(),
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
    let is_delete = line.is_delete;
    let preview = text(line.preview.clone())
        .size(font::CAPTION)
        .font(iced::Font::MONOSPACE)
        .style(move |theme: &iced::Theme| text::Style {
            color: Some(if is_delete {
                colors(theme).danger()
            } else {
                colors(theme).text_muted()
            }),
        });

    // The whole row is the click target: drilling in is the common action,
    // and a hairline button next to a monospace key is not.
    button(
        column![
            row![
                key,
                iced::widget::space::horizontal(),
                kit::muted(human_bytes(line.len as u64)),
            ]
            .spacing(space::SM),
            preview,
        ]
        .spacing(1),
    )
    .on_press(msg(EchoMsg::LineClicked(line.key.clone())))
    .style(iced::widget::button::text)
    .padding(iced::Padding::from([2.0, 0.0]))
    .width(Length::Fill)
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(seq: u64, key: &str, preview: &str) -> EchoLine {
        EchoLine {
            seq,
            key: key.to_string(),
            preview: preview.to_string(),
            len: preview.len(),
            encoding: "application/json".into(),
            is_delete: false,
            timestamp: None,
        }
    }

    #[test]
    fn an_empty_filter_matches_everything() {
        assert!(matches(&line(0, "a/b", "x"), ""));
    }

    #[test]
    fn the_filter_covers_key_and_payload_case_insensitively() {
        let l = line(0, "v1/h-abc/telemetry/sysinfo/CPU", "{\"Value\":3}");
        assert!(matches(&l, "cpu"));
        assert!(matches(&l, "SYSINFO"));
        assert!(matches(&l, "value"));
        assert!(!matches(&l, "nowhere"));
    }

    /// #72's acceptance: a keyexpr filter selects by **intersection**, so
    /// `v1/*/state/**` shows exactly state traffic — including from origins
    /// the user never typed, which a prefix match would have missed.
    #[test]
    fn a_keyexpr_filter_selects_by_intersection_not_prefix() {
        let mut view = EchoView::new();
        view.set_key_filter("v1/*/state/**".into());
        assert!(view.key_filter_error.is_none());

        assert!(view.admits(&line(0, "v1/h-aaa/state/sysinfo/health", "{}"), None));
        assert!(view.admits(&line(1, "v1/h-bbb/state/netring/errors", "{}"), None));
        assert!(!view.admits(&line(2, "v1/h-aaa/telemetry/sysinfo/cpu", "{}"), None));
        assert!(!view.admits(&line(3, "demo/foreign/key", "{}"), None));
    }

    /// A filter that does not parse is *reported*, not silently ignored — a
    /// pane that looks narrowed and is not would claim a coverage it lacks.
    #[test]
    fn an_invalid_key_filter_is_reported_and_not_applied() {
        let mut view = EchoView::new();
        view.set_key_filter("v1/$*/state".into());
        let err = view.key_filter_error.clone().expect("must report");
        assert!(err.contains("RFC 03 §2"), "{err}");
        // …and nothing is filtered out while it is invalid.
        assert!(view.admits(&line(0, "demo/anything", "{}"), None));

        // Clearing it clears the error too.
        view.set_key_filter(String::new());
        assert!(view.key_filter_error.is_none());
    }

    /// Pausing freezes the view and *counts* what it hides; resuming reports
    /// the gap (O6 applied to a scrollback the user stopped on purpose).
    #[test]
    fn a_pause_hides_lines_but_never_hides_the_gap() {
        let mut view = EchoView::new();
        assert!(view.paused_gap(10).is_none(), "not paused, no gap");
        view.pause(10);
        assert_eq!(view.paused_gap(10), Some(0));
        assert_eq!(view.paused_gap(37), Some(27));
        view.resume(37);
        assert_eq!(view.resumed_gap, Some(27));
        assert!(view.following);
        assert!(view.paused_at.is_none());
    }

    /// The exported row is the CLI's row: one `jq` script for both.
    #[test]
    fn the_export_matches_the_cli_row_shape() {
        let l = line(
            0,
            "v1/h-3fa9c2d41b7e/state/sysinfo/health",
            "{\"status\":\"ok\"}",
        );
        let json: serde_json::Value = serde_json::from_str(&ndjson_line(&l, "")).unwrap();
        assert_eq!(json["key"], "v1/h-3fa9c2d41b7e/state/sysinfo/health");
        assert_eq!(json["origin"], "h-3fa9c2d41b7e");
        // The subject is the tail *after* the producer chunk (RFC 03 §1),
        // which is exactly what the CLI row carries.
        assert_eq!(json["subject"], "health");
        assert_eq!(json["encoding"], "application/json");
        // A JSON payload rides as JSON, not as a string containing JSON.
        assert_eq!(json["value"]["status"], "ok");

        // A non-JSON payload rides as a string rather than being dropped.
        let raw = line(1, "demo/text", "just words");
        let json: serde_json::Value = serde_json::from_str(&ndjson_line(&raw, "")).unwrap();
        assert_eq!(json["value"], "just words");
        assert_eq!(
            json["origin"],
            serde_json::Value::Null,
            "foreign key: no origin"
        );

        // The shared field set is exactly the CLI's; the schema-decode fields
        // are absent rather than defaulted, so a consumer can tell "not
        // decoded here" from "decoded as nothing".
        let json: serde_json::Value = serde_json::from_str(&ndjson_line(&l, "")).unwrap();
        let obj = json.as_object().unwrap();
        for shared in ["key", "origin", "subject", "encoding", "timestamp", "value"] {
            assert!(obj.contains_key(shared), "missing shared field {shared}");
        }
        for cli_only in ["type", "typed"] {
            assert!(
                !obj.contains_key(cli_only),
                "{cli_only} must be absent, not null — the decode never ran"
            );
        }
    }
}

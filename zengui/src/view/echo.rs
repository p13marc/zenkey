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
    /// [`Self::key_filter`], compiled — `None` when the box is empty or the
    /// expression did not validate. Written only by
    /// [`Self::set_key_filter`], so it cannot drift from the text beside it.
    compiled: Option<zenoh::key_expr::OwnedKeyExpr>,
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
    ///
    /// This is the **only** write point, which is what lets the compiled form
    /// live beside the text (#178): it was being parsed inside `admits`, once
    /// per line per frame, so a 2,000-line ring re-parsed the same filter
    /// 120,000 times a second to answer a question that changes on a
    /// keystroke.
    pub fn set_key_filter(&mut self, value: String) {
        self.key_filter_error = if value.trim().is_empty() {
            None
        } else {
            crate::scope::validate_selector(&value)
                .err()
                .map(|e| e.to_string())
        };
        self.compiled = (self.key_filter_error.is_none() && !value.trim().is_empty())
            .then(|| zenoh::key_expr::OwnedKeyExpr::new(value.clone()).ok())
            .flatten();
        self.key_filter = value;
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
        if let Some(filter) = &self.compiled
            && !line
                .key_expr
                .as_ref()
                .map(|k| filter.intersects(k))
                .unwrap_or(false)
        {
            // A line whose key does not parse is one the filter cannot judge,
            // and an unjudgeable line is not an admitted one — which is what
            // the `unwrap_or(false)` here has always meant.
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

/// One line in the explorers' row dialect (#72), written through the engine's
/// [`zenkey_fleet::SampleRow`] so it cannot drift from the reader (#235).
///
/// This doc used to claim the shared fields were "byte-for-byte the CLI's".
/// They were not: the row spelled a payload byte *count* as `bytes`, which
/// [`zenkey_fleet::parse_row`] reads as base64 of the wire payload
/// (RFC 09 §5.2) and rejects as malformed. Sharing the writer is what makes
/// the claim true rather than restating it.
///
/// The two tools still differ, in exactly the places they differ, and
/// neither difference is faked:
///
/// - the CLI carries `type`/`typed` from its schema decode; that decode is
///   async and must never run on a render path, so it is **absent** here
///   rather than defaulted to `null`/`false`, which would claim the lookup
///   happened and found nothing;
/// - this row carries `payload_bytes`, which the ring knows and the CLI's
///   row spells from the same field.
pub fn ndjson_line(line: &EchoLine, base: &str) -> String {
    let mut row = zenkey_fleet::SampleRow::of_key(&line.key, base);
    row.encoding = Some(line.encoding.clone()).filter(|e| !e.is_empty());
    row.timestamp = line.timestamp.clone();
    row.delete = line.is_delete;
    // The payload *size*, under the key that means a size. `bytes` is the
    // base64 wire payload in this dialect and has been since RFC 09 §5.2 —
    // writing a count there made the export unreadable by the only reader
    // it has, rather than merely lossy (#235).
    row.payload_bytes = Some(line.len);
    row.value = Some(
        serde_json::from_str::<serde_json::Value>(&line.preview)
            .unwrap_or(serde_json::Value::String(line.preview.clone())),
    );
    // Present only when the wire carried one — absent, never null (#117),
    // byte-for-byte the CLI row's convention.
    if let Some(att) = &line.attachment {
        row.attachment = Some(
            serde_json::from_str::<serde_json::Value>(att)
                .unwrap_or(serde_json::Value::String(att.clone())),
        );
        row.attachment_bytes = line.attachment_len;
    }
    row.to_line()
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
    // Both texts borrow, and the click message is built on the click (#178).
    // Up to 300 rows are drawn per frame and each was cloning two `String`s
    // for a rendering identical to the last one's; `on_press_with` moves the
    // third clone from every frame to the one frame somebody actually clicks.
    let key = text(line.key.as_str())
        .size(font::CAPTION)
        .font(iced::Font::MONOSPACE)
        .style(|theme: &iced::Theme| text::Style {
            color: Some(colors(theme).text()),
        });

    // A tombstone is authoritative retirement, not an empty value
    // (RFC 04 §1.2) — so it must not look like a put with no payload.
    let is_delete = line.is_delete;
    let preview = text(line.preview.as_str())
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
    .on_press_with(|| msg(EchoMsg::LineClicked(line.key.clone())))
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
            key_expr: zenoh::key_expr::OwnedKeyExpr::new(key.to_string()).ok(),
            key: key.to_string(),
            preview: preview.to_string(),
            len: preview.len(),
            encoding: "application/json".into(),
            is_delete: false,
            timestamp: None,
            attachment: None,
            attachment_len: None,
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

    /// The compiled filter cannot drift from the text beside it, because
    /// `set_key_filter` is the only write point — which is exactly what makes
    /// caching it safe (#178). Every state transition, in one test.
    #[test]
    fn the_compiled_filter_tracks_the_text_through_every_state() {
        let mut view = EchoView::new();
        assert!(view.compiled.is_none(), "an empty box filters nothing");

        view.set_key_filter("v1/*/state/**".into());
        assert!(view.compiled.is_some());

        // Invalid: reported, and *not* applied — so nothing is compiled.
        view.set_key_filter("v1/$*/state".into());
        assert!(view.key_filter_error.is_some());
        assert!(
            view.compiled.is_none(),
            "the previous filter must not keep filtering under a new, bad one"
        );

        // Back to valid, then empty.
        view.set_key_filter("demo/**".into());
        assert!(view.compiled.is_some());
        view.set_key_filter("   ".into());
        assert!(view.compiled.is_none(), "whitespace is an empty box");
    }

    /// A line whose key does not parse cannot be judged by a key filter, and
    /// an unjudgeable line is not an admitted one. Not a wire case — the
    /// sample carried the key — but a replayed row can be anything.
    #[test]
    fn a_line_whose_key_does_not_parse_is_not_admitted_by_a_key_filter() {
        let mut bad = line(0, "v1/h-aaa/state/sysinfo/health", "{}");
        bad.key_expr = None;
        let mut view = EchoView::new();
        assert!(view.admits(&bad, None), "no filter, no judgement to fail");
        view.set_key_filter("v1/**".into());
        assert!(!view.admits(&bad, None));
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

    /// The export is not merely *shaped* like the pipe's rows — it is
    /// readable by the only reader they have (#235). This is the assertion
    /// the old test's name claimed and did not make: it compared fields by
    /// hand, and `"bytes": <a byte count>` passed every one of those
    /// comparisons while making the row unparseable.
    #[test]
    fn an_exported_row_reads_back_through_the_engines_reader() {
        let mut l = line(
            0,
            "v1/h-3fa9c2d41b7e/state/sysinfo/health",
            "{\"status\":\"ok\"}",
        );
        l.attachment = Some(r#"{"who":"me"}"#.to_string());
        l.attachment_len = Some(12);
        let row = zenkey_fleet::parse_row(&ndjson_line(&l, "")).expect("the export reads back");
        assert_eq!(row.key, "v1/h-3fa9c2d41b7e/state/sysinfo/health");
        assert_eq!(row.payload, br#"{"status":"ok"}"#);
        assert_eq!(row.encoding.as_deref(), Some("application/json"));
        assert_eq!(row.attachment.as_deref(), Some(&br#"{"who":"me"}"#[..]));
        assert!(!row.delete);

        // A tombstone reads back as a tombstone, not as an empty put
        // (RFC 04 §1.2).
        let mut dead = line(1, "v1/h-3fa9c2d41b7e/state/sysinfo/health", "");
        dead.is_delete = true;
        let row = zenkey_fleet::parse_row(&ndjson_line(&dead, "")).expect("a tombstone reads back");
        assert!(row.delete);
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
        // Absent, not null: `json["origin"]` yields `Null` for a missing key
        // too, so asserting on the value cannot tell the two apart — which
        // is the whole reason the engine's corpus compares whole documents
        // (RFC 09 §5.1 O4).
        assert!(
            !json.as_object().unwrap().contains_key("origin"),
            "a foreign key carries no origin — absent, never null"
        );

        // The shared field set is exactly the CLI's; the schema-decode fields
        // are absent rather than defaulted, so a consumer can tell "not
        // decoded here" from "decoded as nothing".
        let json: serde_json::Value = serde_json::from_str(&ndjson_line(&l, "")).unwrap();
        let obj = json.as_object().unwrap();
        for shared in ["key", "origin", "subject", "encoding", "value"] {
            assert!(obj.contains_key(shared), "missing shared field {shared}");
        }
        assert!(
            !obj.contains_key("timestamp"),
            "an unstamped sample omits the HLC rather than nulling it: nothing \
             stamped it, which is not the same as it having been stamped null"
        );
        for cli_only in ["type", "typed"] {
            assert!(
                !obj.contains_key(cli_only),
                "{cli_only} must be absent, not null — the decode never ran"
            );
        }
        assert!(
            !obj.contains_key("attachment"),
            "attachment must be absent when the wire carried none, never null (#117)"
        );

        // And when one arrived, it rides — with its true size beside it.
        let mut with_att = l.clone();
        with_att.attachment = Some(r#"{"who":"me"}"#.to_string());
        with_att.attachment_len = Some(12);
        let json: serde_json::Value = serde_json::from_str(&ndjson_line(&with_att, "")).unwrap();
        assert_eq!(json["attachment"]["who"], "me");
        assert_eq!(json["attachment_bytes"], 12);
    }
}

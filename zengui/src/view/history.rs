//! The history pane (issue #63): what the selected key did, and what changed
//! between one sample and the next.
//!
//! Three statements, in the order a reader needs them:
//!
//! 1. **Is anything being recorded at all** — history exists only for a key
//!    under an active watch, and a pane that renders "nothing yet" for a key
//!    nobody is watching would be reporting a verdict it never obtained
//!    (RFC 09 §5.1 O4). The empty state says which of the two it is, and
//!    offers the watch.
//! 2. **The timeline** — newest first, each row labelled with *which clock*
//!    stamped it: the publisher's HLC where one exists, this observer's
//!    arrival otherwise. They are never silently interchanged.
//! 3. **The diff** — the selected row against the one before it. A tombstone
//!    is retirement, and the put after one is a new value, not a change
//!    (RFC 04 §1.2); a payload with no structural form degrades to a byte
//!    comparison that says so rather than inventing field names.

use iced::widget::{Column, button, column, row, text};
use iced::{Element, Length};
use zenkey_fleet::diff::{Change, ValueDiff};

use crate::history::{HistoryEntry, HistoryRecorder};
use crate::message::{Message, PaneMsg, SubjectMsg};
use crate::view::kit::{self, human_bytes};
use crate::view::theme::colors;
use crate::view::tokens::{font, space};

/// How many field changes one diff lists before the rest are counted.
const MAX_CHANGES: usize = 50;

/// How much of an HLC timestamp a row shows before truncating.
const STAMP_CHARS: usize = 34;

/// Messages the pane emits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryMsg {
    /// A timeline row was clicked — its diff opens.
    Select(u64),
    /// Forget the retained entries (the eviction count survives).
    Clear,
}

fn msg(m: HistoryMsg) -> Message {
    Message::Pane(PaneMsg::History(m))
}

/// What the app hands the pane.
pub struct HistoryData<'a> {
    /// The selected wire key, if any.
    pub key: Option<&'a str>,
    /// The recording, when one is running for that key.
    pub recorder: Option<&'a HistoryRecorder>,
    /// Whether an active watch covers the key. `false` means no sample can
    /// reach the recorder — the load-bearing distinction of this pane.
    pub watched: bool,
}

pub fn pane<'a>(data: HistoryData<'a>) -> Element<'a, Message> {
    let Some(key) = data.key else {
        return column![
            kit::section_header("History", None),
            kit::empty_state(
                "Nothing selected",
                "Select a key to record what it does. An explorer records nothing \
                 it was not asked to (#85), so history starts at selection and \
                 earlier samples are never backfilled.",
            ),
        ]
        .spacing(space::SM)
        .into();
    };

    let mut col = Column::new().spacing(space::SM);
    col = col.push(kit::section_header(
        "History",
        Some(
            button(text("clear").size(font::CAPTION))
                .on_press(msg(HistoryMsg::Clear))
                .padding(4)
                .into(),
        ),
    ));
    col = col.push(kit::mono(key.to_string()));

    if !data.watched {
        col = col.push(kit::empty_state(
            "Not watched — nothing is being recorded",
            "History only exists for a key under an active watch, and no watch \
             covers this one. Watching it records from now on; the samples that \
             already went past are gone, not hidden.",
        ));
        col = col.push(
            button(text("watch this key").size(font::CAPTION))
                .on_press(Message::Subject(SubjectMsg::WatchToggled(key.to_string())))
                .padding(4),
        );
        return col.into();
    }

    let Some(rec) = data.recorder else {
        return col
            .push(kit::empty_state(
                "No recording for this key",
                "The recorder follows the selection; this one has not started yet.",
            ))
            .into();
    };

    col = col.push(kit::muted(format!(
        "recording since selection · {} retained · {} evicted (ring full)",
        rec.ring.len(),
        rec.ring.evicted(),
    )));

    if rec.ring.is_empty() {
        return col
            .push(kit::empty_state(
                "No samples yet",
                "The watch is active and nothing has arrived on this key since it \
                 was selected. That is not a statement about the bus (RFC 05 §3.1).",
            ))
            .into();
    }

    col = col.push(kit::muted(
        "times are the publisher's HLC where one was stamped and this observer's \
         arrival otherwise — every row says which",
    ));

    let focus = rec.focus();
    let newest = rec.ring.newest().map(|e| e.seq).unwrap_or(0);
    let mut rows = Column::new().spacing(1);
    for entry in rec.ring.iter() {
        rows = rows.push(row_view(entry, newest, Some(entry.seq) == focus, rec));
    }
    col = col.push(
        iced::widget::scrollable(rows)
            .height(Length::FillPortion(3))
            .width(Length::Fill),
    );

    col = col.push(diff_section(rec, focus));
    col.into()
}

/// One timeline row.
fn row_view<'a>(
    entry: &HistoryEntry,
    newest: u64,
    focused: bool,
    rec: &HistoryRecorder,
) -> Element<'a, Message> {
    // t-0 is the newest; the label counts back from it, which is how a reader
    // asks the question ("what did it look like two samples ago").
    let age = newest.saturating_sub(entry.seq);
    let marker = if focused { "▸" } else { " " };
    let kind = if entry.is_delete {
        "DELETE (tombstone)".to_string()
    } else {
        format!("put {}", human_bytes(entry.len as u64))
    };
    let is_delete = entry.is_delete;

    let head = row![
        kit::mono(format!("{marker} t-{age}")),
        kit::muted(stamp(entry, rec)),
        text(kind)
            .size(font::CAPTION)
            .style(move |theme: &iced::Theme| text::Style {
                color: Some(if is_delete {
                    colors(theme).danger()
                } else {
                    colors(theme).text_muted()
                }),
            }),
    ]
    .spacing(space::SM);

    button(column![head, kit::mono(entry.preview.clone())].spacing(1))
        .on_press(msg(HistoryMsg::Select(entry.seq)))
        .style(iced::widget::button::text)
        .padding(iced::Padding::from([2.0, 0.0]))
        .width(Length::Fill)
        .into()
}

/// The row's time, labelled with the clock that produced it.
fn stamp(entry: &HistoryEntry, rec: &HistoryRecorder) -> String {
    match &entry.timestamp {
        Some(t) => {
            let shown: String = if t.chars().count() > STAMP_CHARS {
                t.chars().take(STAMP_CHARS).chain(['…']).collect()
            } else {
                t.clone()
            };
            format!("hlc {shown}")
        }
        None => format!(
            "arrival +{:.3}s",
            entry
                .received
                .saturating_duration_since(rec.since)
                .as_secs_f64()
        ),
    }
}

/// The diff panel: the focused row against the one before it.
fn diff_section<'a>(rec: &HistoryRecorder, focus: Option<u64>) -> Element<'a, Message> {
    let mut col = Column::new().spacing(2);
    let Some((prev, entry)) = focus.and_then(|s| rec.ring.pair(s)) else {
        return col.push(kit::muted("no row selected")).into();
    };
    let newest = rec.ring.newest().map(|e| e.seq).unwrap_or(0);
    col = col.push(kit::muted(format!(
        "changes at t-{}",
        newest.saturating_sub(entry.seq)
    )));

    // A tombstone is not a value, so nothing about it is a field change.
    if entry.is_delete {
        return col
            .push(note(
                "retired — an authoritative delete (RFC 04 §1.2), not an empty value",
            ))
            .into();
    }
    let Some(prev) = prev else {
        return col
            .push(kit::muted(if rec.ring.evicted() > 0 {
                "the sample before this one was evicted — nothing to compare against"
            } else {
                "the first sample recorded — nothing to compare against"
            }))
            .into();
    };
    if prev.is_delete {
        return col
            .push(note(
                "new value after retirement — not a change to the previous value",
            ))
            .into();
    }

    match (&prev.value, &entry.value) {
        (Some(old), Some(new)) => {
            let d = zenkey_fleet::diff::diff(old, new, MAX_CHANGES);
            if d.is_empty() {
                return col
                    .push(kit::muted(
                        "no change — every field is identical to the previous sample",
                    ))
                    .into();
            }
            col.push(changes_view(&d)).into()
        }
        _ => col.push(bytes_view(prev, entry)).into(),
    }
}

fn changes_view<'a>(d: &ValueDiff) -> Element<'a, Message> {
    let mut col = Column::new().spacing(1);
    for change in &d.changes {
        let (line, tone) = match change {
            Change::Changed { path, old, new } => (
                format!("~ {path}  {} → {}", brief(old), brief(new)),
                Tone::Changed,
            ),
            Change::Added { path, new } => (format!("+ {path}  {}", brief(new)), Tone::Added),
            Change::Removed { path, old } => (format!("- {path}  {}", brief(old)), Tone::Removed),
        };
        col = col.push(
            text(line)
                .size(font::CAPTION)
                .font(iced::Font::MONOSPACE)
                .style(move |theme: &iced::Theme| text::Style {
                    color: Some(match tone {
                        Tone::Changed => colors(theme).warning(),
                        Tone::Added => colors(theme).success(),
                        Tone::Removed => colors(theme).danger(),
                    }),
                }),
        );
    }
    if d.truncated > 0 {
        col = col.push(kit::muted(format!(
            "… {} not listed (the diff is bounded)",
            kit::plural(d.truncated, "further change"),
        )));
    }
    col.into()
}

#[derive(Clone, Copy)]
enum Tone {
    Changed,
    Added,
    Removed,
}

/// The honest fallback when at least one side is not a document.
fn bytes_view<'a>(prev: &HistoryEntry, entry: &HistoryEntry) -> Element<'a, Message> {
    let old = prev.payload.to_bytes();
    let new = entry.payload.to_bytes();
    let d = zenkey_fleet::diff::byte_diff(&old, &new);
    let which = match (prev.value.is_some(), entry.value.is_some()) {
        (false, false) => "neither sample has a structural form",
        (true, false) => "this sample has no structural form",
        (false, true) => "the previous sample has no structural form",
        (true, true) => unreachable!("two structural values are diffed as fields above"),
    };
    let mut col = Column::new().spacing(1);
    col = col.push(kit::muted(format!(
        "{which} — compared as bytes, not as fields"
    )));
    if d.is_empty() {
        col = col.push(kit::muted("no change — the bytes are identical"));
        return col.into();
    }
    let (old_range, new_range) = d.ranges();
    col = col.push(kit::mono(format!(
        "{} B → {} B · first {} B and last {} B in common · differs over {}..{} → {}..{}",
        d.old_len,
        d.new_len,
        d.common_prefix,
        d.common_suffix,
        old_range.start,
        old_range.end,
        new_range.start,
        new_range.end,
    )));
    col.into()
}

/// A short rendering of one value, for a diff line.
fn brief(v: &serde_json::Value) -> String {
    const MAX: usize = 60;
    let s = v.to_string();
    if s.chars().count() <= MAX {
        return s;
    }
    s.chars().take(MAX).chain(['…']).collect()
}

/// A statement about the timeline that is not a field change.
fn note<'a>(s: &'static str) -> Element<'a, Message> {
    text(s)
        .size(font::CAPTION)
        .style(|theme: &iced::Theme| text::Style {
            color: Some(colors(theme).danger()),
        })
        .into()
}

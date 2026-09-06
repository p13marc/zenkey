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

use iced::widget::{Column, column, row, text};
use iced::{Element, Length};
use zenkey_fleet::model::diff::{Change, ValueDiff};

use crate::history::{HistoryEntry, HistoryRecorder};
use crate::message::{Message, PaneMsg, SlotId, SubjectMsg};
use crate::view::kit::{self, human_bytes};
use crate::view::theme::colors;
use crate::view::tokens::{CAPTION_LINE, Spacing};

/// How many field changes one diff lists before the rest are counted.
const MAX_CHANGES: usize = 50;

/// How much of an HLC timestamp a row shows before truncating.
const STAMP_CHARS: usize = 34;

/// Messages the pane emits.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum HistoryMsg {
    /// A timeline row was clicked — its diff opens.
    Select(u64),
    /// Forget the retained entries (the eviction count survives).
    Clear,
    /// The timeline scrolled: (absolute y offset, viewport height) — what the
    /// virtualized window renders against (#183).
    Scrolled(crate::view::kit::Viewport),
    /// Compare the newest sample against the loaded snapshot's row for this
    /// key (#219), or stop.
    CompareSnapshotToggled,
}

fn msg(slot: SlotId, m: HistoryMsg) -> Message {
    Message::Pane(PaneMsg::History(slot, m))
}

/// What the app hands the pane.
pub struct HistoryData<'a> {
    /// The subject slot this section renders (#257) — every message it
    /// emits carries it home.
    pub slot: SlotId,
    /// The selected wire key, if any.
    pub key: Option<&'a str>,
    /// The recording, when one is running for that key.
    pub recorder: Option<&'a HistoryRecorder>,
    /// Whether an active watch covers the key. `false` means no sample can
    /// reach the recorder — the load-bearing distinction of this pane.
    pub watched: bool,
    /// Scroll position + viewport height, driving the virtual window (#183).
    pub scroll: crate::view::kit::Viewport,
    /// The loaded `.zsnap`'s row for this key and the file's header (#219),
    /// when a snapshot is loaded and carries the key.
    pub snapshot: Option<(&'a zenkey_fleet::SnapshotRow, &'a zenkey_fleet::ZsnapHeader)>,
    /// Whether the panel compares against that row rather than the
    /// previous sample.
    pub compare_snapshot: bool,
    /// The dock's resolved spacing grid (#192).
    pub sp: Spacing,
}

/// One timeline row's height. Two lines — the head and the payload preview —
/// so a row is taller than a tree row and the window must say so.
///
/// The rows are fixed-height by *construction*, not by hope: `row_view` builds
/// a two-line button and the container below pins it. A preview that wrapped
/// would silently break the window's arithmetic, which is why the preview is
/// pre-truncated by the recorder rather than by the layout.
///
/// The comfortable baseline: the section renders at
/// `sp.row(ROW_HEIGHT, 2.0 * CAPTION_LINE)` (#192) — two lines of text and
/// almost no air, so this is the row density compacts the least.
pub const ROW_HEIGHT: f32 = 34.0;

/// The Inspector's history sections (#182). See [`super::detail::section`]
/// for why this is a `Column`.
pub fn section<'a>(data: HistoryData<'a>) -> Column<'a, Message> {
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
        .spacing(data.sp.sm);
    };

    let sp = data.sp;
    let slot = data.slot;
    let mut col = Column::new().spacing(sp.sm);
    col = col.push(kit::section_header(
        "History",
        Some(
            kit::action(kit::caption("clear"))
                .on_press(msg(data.slot, HistoryMsg::Clear))
                .padding(sp.xs)
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
            kit::action(kit::caption("watch this key"))
                .on_press(Message::Subject(SubjectMsg::WatchToggled(key.to_string())))
                .padding(sp.xs),
        );
        return col;
    }

    let Some(rec) = data.recorder else {
        return col.push(kit::empty_state(
            "No recording for this key",
            "The recorder follows the selection; this one has not started yet.",
        ));
    };

    col = col.push(kit::muted(format!(
        "recording since selection · {} retained · {} evicted (ring full)",
        rec.ring.len(),
        rec.ring.evicted(),
    )));

    if rec.ring.is_empty() {
        return col.push(kit::empty_state(
            "No samples yet",
            "The watch is active and nothing has arrived on this key since it \
             was selected. That is not a statement about the bus (RFC 05 §3.1).",
        ));
    }

    col = col.push(kit::muted(
        "times are the publisher's HLC where one was stamped and this observer's \
         arrival otherwise — every row says which",
    ));

    let focus = rec.focus();
    let newest = rec.ring.newest().map(|e| e.seq).unwrap_or(0);
    // O(visible), like the tree (#183). This drew every retained entry — up to
    // 200 buttons, each with a preview — on every frame, for a list of which
    // the viewport shows perhaps a dozen. The tree has had the fix since #65;
    // History never got it, and the asymmetry was invisible because both
    // *bounds* were honest and neither said what drawing cost.
    let total = rec.ring.len();
    // Two lines of text per row: density shaves the row's air, not its type
    // (#192) — the window arithmetic and the containers share the result.
    let row_h = sp.row(ROW_HEIGHT, 2.0 * CAPTION_LINE);
    let (first, last) = kit::window(total, data.scroll, row_h);
    let mut rows = Column::new();
    if first > 0 {
        rows = rows.push(iced::widget::Space::new().height(Length::Fixed(first as f32 * row_h)));
    }
    for entry in rec.ring.iter().skip(first).take(last - first) {
        rows = rows.push(
            iced::widget::container(row_view(
                data.slot,
                entry,
                newest,
                Some(entry.seq) == focus,
                rec,
                sp,
            ))
            .height(Length::Fixed(row_h)),
        );
    }
    if last < total {
        rows = rows
            .push(iced::widget::Space::new().height(Length::Fixed((total - last) as f32 * row_h)));
    }
    col = col.push(
        iced::widget::scrollable(rows)
            .height(Length::FillPortion(3))
            .width(Length::Fill)
            .on_scroll(move |viewport| msg(slot, HistoryMsg::Scrolled(viewport.into()))),
    );

    // The snapshot comparison (#219): offered only when the loaded file
    // carries this key; the caption names the snapshot's moment *and span*,
    // because a snapshot is collected over one (RFC 13 §4.4).
    if let Some((row, header)) = data.snapshot {
        col = col.push(
            kit::action(kit::caption(if data.compare_snapshot {
                "compare with previous sample"
            } else {
                "compare with snapshot"
            }))
            .on_press(msg(slot, HistoryMsg::CompareSnapshotToggled))
            .padding(sp.xs),
        );
        if data.compare_snapshot {
            col = col.push(snapshot_section(rec, row, header, sp));
            return col;
        }
    }
    col = col.push(diff_section(rec, focus, sp));
    col
}

/// The newest sample against the snapshot's row for the key (#219): the
/// same pair renderer as the timeline diff, with the left column captioned
/// by the snapshot's moment and span.
fn snapshot_section<'a>(
    rec: &HistoryRecorder,
    row: &zenkey_fleet::SnapshotRow,
    header: &zenkey_fleet::ZsnapHeader,
    sp: Spacing,
) -> Element<'a, Message> {
    let mut col = Column::new().spacing(sp.xs);
    col = col.push(kit::muted(format!(
        "snapshot {} (over {:.2}s) → newest sample",
        header.collected_at, header.collection_span_s
    )));
    let Some(newest) = rec.ring.newest() else {
        return col
            .push(kit::muted(
                "no live sample yet — nothing to compare the snapshot against",
            ))
            .into();
    };
    let snapshot_entry = HistoryEntry::from_snapshot_row(row);
    col.push(pair_view(
        Some(&snapshot_entry),
        newest,
        rec.ring.evicted(),
        sp,
    ))
    .into()
}

/// One timeline row.
fn row_view<'a>(
    slot: SlotId,
    entry: &HistoryEntry,
    newest: u64,
    focused: bool,
    rec: &HistoryRecorder,
    sp: Spacing,
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
        kit::caption(kind).style(move |theme: &iced::Theme| text::Style {
            color: Some(if is_delete {
                colors(theme).danger()
            } else {
                colors(theme).text_muted()
            }),
        }),
    ]
    .spacing(sp.sm);

    // No spacing and no padding on the two-line body: the row is pinned to
    // `sp.row(..)`, which already spends all the air the density allows —
    // spacing here would only push the preview past the clip.
    kit::row_button(column![head, kit::mono(entry.preview.clone())], false)
        .on_press(msg(slot, HistoryMsg::Select(entry.seq)))
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
fn diff_section<'a>(
    rec: &HistoryRecorder,
    focus: Option<u64>,
    sp: Spacing,
) -> Element<'a, Message> {
    let mut col = Column::new().spacing(sp.xs);
    let Some((prev, entry)) = focus.and_then(|s| rec.ring.pair(s)) else {
        return col.push(kit::muted("no row selected")).into();
    };
    let newest = rec.ring.newest().map(|e| e.seq).unwrap_or(0);
    col = col.push(kit::muted(format!(
        "changes at t-{}",
        newest.saturating_sub(entry.seq)
    )));
    col.push(pair_view(prev, entry, rec.ring.evicted(), sp))
        .into()
}

/// One pair, rendered: `entry` against `prev` — the row before it in the
/// ring, or a snapshot's row (#219). A tombstone is retirement, the put
/// after one is a new value, and a pair with no structural form on one
/// side degrades to bytes and says so.
fn pair_view<'a>(
    prev: Option<&HistoryEntry>,
    entry: &HistoryEntry,
    evicted: u64,
    sp: Spacing,
) -> Element<'a, Message> {
    let col = Column::new().spacing(sp.xs);
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
            .push(kit::muted(if evicted > 0 {
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
            let d = zenkey_fleet::value_diff(old, new, MAX_CHANGES);
            if d.is_empty() {
                return col
                    .push(kit::muted(
                        "no change — every field is identical to the previous sample",
                    ))
                    .into();
            }
            col.push(changes_view(&d, sp)).into()
        }
        _ => col.push(bytes_view(prev, entry, sp)).into(),
    }
}

fn changes_view<'a>(d: &ValueDiff, sp: Spacing) -> Element<'a, Message> {
    let mut col = Column::new().spacing(sp.xs);
    for change in &d.changes {
        let (line, tone) = match change {
            Change::Changed { path, old, new } => (
                format!("~ {path}  {} → {}", brief(old), brief(new)),
                Tone::Changed,
            ),
            Change::Added { path, new } => (format!("+ {path}  {}", brief(new)), Tone::Added),
            Change::Removed { path, old } => (format!("- {path}  {}", brief(old)), Tone::Removed),
        };
        col = col.push(kit::caption(line).font(iced::Font::MONOSPACE).style(
            move |theme: &iced::Theme| text::Style {
                color: Some(match tone {
                    Tone::Changed => colors(theme).warning(),
                    Tone::Added => colors(theme).success(),
                    Tone::Removed => colors(theme).danger(),
                }),
            },
        ));
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
fn bytes_view<'a>(prev: &HistoryEntry, entry: &HistoryEntry, sp: Spacing) -> Element<'a, Message> {
    let old = prev.payload.to_bytes();
    let new = entry.payload.to_bytes();
    let d = zenkey_fleet::byte_diff(&old, &new);
    let which = match (prev.value.is_some(), entry.value.is_some()) {
        (false, false) => "neither sample has a structural form",
        (true, false) => "this sample has no structural form",
        (false, true) => "the previous sample has no structural form",
        (true, true) => unreachable!("two structural values are diffed as fields above"),
    };
    let mut col = Column::new().spacing(sp.xs);
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
    kit::body(s)
        .style(|theme: &iced::Theme| text::Style {
            color: Some(colors(theme).danger()),
        })
        .into()
}

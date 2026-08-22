//! One surface that follows the subject (#182).
//!
//! Detail, History, Blobs and Media were four mutually-exclusive tabs that
//! each answered part of *one* question about *one* thing. "What changed in
//! this payload?" needs the value, the decode, the diff against the previous
//! sample and the chart — and that used to be `RightPane::Detail` and
//! `RightPane::History` alternating.
//!
//! ## Composition, not a rewrite
//!
//! Every section here is the pane that used to be a tab, called unchanged.
//! What each one *says* is identical, which is why the honesty assertions in
//! `tests/panes.rs` survived as section tests with a one-line call-site edit:
//! every claim they pin is about text on screen, and the text did not move.
//!
//! The one real change is that a section returns a `Column` rather than an
//! `Element` wrapping its own `scrollable`. A scrollable nested inside a
//! scrollable is a layout bug, so exactly one caller owns the scroll — this
//! one.
//!
//! ## Plane conditionality comes from the classification seam
//!
//! Whether a key is on the `@blob` or `@media` plane is
//! [`ClassKind`], read off the projected
//! [`KeyFacts`] — never a `key.contains("@blob")`. `keyfacts` is the single
//! seam where a key becomes convention-aware (this crate's whole
//! key-agnostic-core claim), and a second, string-shaped classifier here
//! would be the thing that claim forbids.
//!
//! ## Three of the issue's six rows are not here
//!
//! #182's table has `Router`, `Storage` and `Bus` rows. #181 shipped a
//! [`Subject`] with only the variants an interaction can construct, and
//! nothing in the app selects a router, a storage or "the bus" — the admin
//! pane has no selection at all. Those rows arrive with the pane that can
//! construct their subject; until then they would be arms no test could
//! reach.

use iced::widget::{Column, column, scrollable};
use iced::{Element, Length};
use zenkey_fleet::facts::{ClassKind, KeyFacts, KeyShape};
use zenkey_fleet::{LatencyReport, SliceSet};

use super::detail::{DetailData, Fetched, SeriesData};
use super::history::HistoryData;
use super::{blob, detail, history, kit, media, nodes};
use crate::blob::BlobState;
use crate::history::HistoryRecorder;
use crate::message::{Message, Subject};
use crate::nodes::NodeRoster;
use crate::view::media::MediaState;
use crate::view::nodes::DetailState;
use crate::view::tokens::space;

/// Everything the Inspector may show, for whatever the subject turns out to
/// be.
///
/// One struct rather than thirteen arguments, in the crate's usual shape (see
/// [`super`]'s contract). It is deliberately *not* a nest of the four section
/// data structs: those are built here, from these fields, so that the mapping
/// from "what the app holds" to "what a section needs" is written once and in
/// one place.
pub struct InspectorData<'a> {
    pub subject: &'a Subject,
    /// The projected facts for a key subject — the plane classifier, and the
    /// facts ladder the Detail section renders.
    pub facts: Option<&'a KeyFacts>,
    pub fetched: Fetched<'a>,
    pub decoded: Option<&'a (Option<String>, zenkey_fleet::decode::Rendering)>,
    pub series: Option<&'a SeriesData>,
    pub history: Option<&'a HistoryRecorder>,
    /// The timeline's scroll offset and viewport height (#183).
    pub history_scroll: (f32, f32),
    /// Whether an active watch covers the subject key — the distinction the
    /// History section rests on.
    pub watched: bool,
    pub latency: Option<(LatencyReport, u64)>,
    pub blob: &'a BlobState,
    pub media: &'a MediaState,
    pub slices: Option<&'a SliceSet>,
    pub roster: &'a NodeRoster,
    pub node_detail: &'a DetailState,
}

/// Which plane a key sits on, when it sits on one at all.
///
/// `None` for anything the grammar did not place: an unparsed key, a key
/// outside the base, or no projection yet. "We have not classified it" and
/// "it is on the ordinary planes" render the same way here, which is correct —
/// neither adds a blob section.
fn plane(facts: Option<&KeyFacts>) -> Option<ClassKind> {
    match &facts?.shape {
        KeyShape::V1(f) => Some(f.class_kind),
        _ => None,
    }
}

pub fn pane<'a>(d: InspectorData<'a>) -> Element<'a, Message> {
    let body = match d.subject {
        Subject::None => nothing_selected(),
        Subject::Key(key) => key_sections(key, &d),
        Subject::Prefix(prefix) => prefix_sections(prefix),
        Subject::Origin(origin) => origin_sections(origin, &d),
    };
    scrollable(body.spacing(space::MD).padding(space::SM))
        .height(Length::Fill)
        .into()
}

fn nothing_selected<'a>() -> Column<'a, Message> {
    column![
        kit::section_header("Inspector", None),
        kit::empty_state(
            "Nothing selected",
            "Pick a key in the tree, or a node in the dashboard. The Inspector \
             follows whatever the window is looking at — it holds nothing of \
             its own.",
        ),
    ]
}

/// A prefix names a subtree, and a subtree has no value, no history and no
/// plane.
///
/// Saying so is the point: this is the state that used to render as a Detail
/// pane full of "no value fetched" for a key that was never a key (#85).
fn prefix_sections<'a>(prefix: &'a str) -> Column<'a, Message> {
    column![
        kit::section_header("Inspector", None),
        // The subject — the one TITLE in the window (#191).
        kit::title(prefix).font(iced::Font::MONOSPACE),
        kit::empty_state(
            "A subtree, not a key",
            "Nothing was fetched, because a prefix names no value any producer \
             publishes. Expand it and select a leaf.",
        ),
    ]
}

fn key_sections<'a>(key: &'a str, d: &InspectorData<'a>) -> Column<'a, Message> {
    let mut col = detail::section(DetailData {
        key,
        facts: d.facts,
        fetched: d.fetched,
        decoded: d.decoded,
        series: d.series,
        history_entries: d.history.map(|r| r.ring.len()),
        observed: d.history.and_then(|r| r.ring.newest()),
        latency: d.latency.clone(),
    });

    // The plane sections, when the key is on one. Order matters: the plane is
    // the more specific fact, so it reads after the general one.
    match plane(d.facts) {
        Some(ClassKind::Blob) => {
            col = col.push(blob::section(d.blob, d.slices.is_some()));
        }
        Some(ClassKind::Media) => {
            col = col.push(media::section(d.media, d.slices));
        }
        _ => {}
    }

    col.push(history::section(HistoryData {
        key: Some(key),
        recorder: d.history,
        watched: d.watched,
        scroll: d.history_scroll,
    }))
}

fn origin_sections<'a>(origin: &'a str, d: &InspectorData<'a>) -> Column<'a, Message> {
    column![
        kit::section_header("Inspector", None),
        // The subject — the one TITLE in the window (#191).
        kit::title(origin).font(iced::Font::MONOSPACE),
        nodes::presence_section(d.roster, origin),
        nodes::detail_section(d.node_detail),
    ]
}

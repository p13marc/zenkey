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
use zenkey_fleet::facts::{ClassKind, KeyFacts, KeyShape, Registration};
use zenkey_fleet::report::TopicRow;
use zenkey_fleet::{KeyTreeSnapshot, LatencyReport, SliceSet};

use super::detail::{DetailData, Fetched, SeriesData};
use super::history::HistoryData;
use super::{blob, detail, history, kit, media, nodes};
use crate::blob::BlobState;
use crate::history::HistoryRecorder;
use crate::message::{Message, SlotId, Subject};
use crate::nodes::NodeRoster;
use crate::view::media::MediaState;
use crate::view::nodes::DetailState;
use crate::view::tokens::Spacing;

/// Everything the Inspector may show, for whatever the subject turns out to
/// be.
///
/// One struct rather than thirteen arguments, in the crate's usual shape (see
/// [`super`]'s contract). It is deliberately *not* a nest of the four section
/// data structs: those are built here, from these fields, so that the mapping
/// from "what the app holds" to "what a section needs" is written once and in
/// one place.
pub struct InspectorData<'a> {
    /// Which subject slot this surface is bound to (#257): the docked
    /// Inspector says [`SlotId::FOLLOW`]; a pinned window says its own.
    /// Every section message carries it home, so two Inspectors over two
    /// slots never speak into each other's state.
    pub slot: SlotId,
    pub subject: &'a Subject,
    /// The projected facts for a key subject — the plane classifier, and the
    /// facts ladder the Detail section renders.
    pub facts: Option<&'a KeyFacts>,
    pub fetched: Fetched<'a>,
    pub decoded: Option<&'a zenkey_fleet::decode::DecodedSample>,
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
    /// The bounded field observation on the subject key (#223).
    pub fields: &'a super::fields::FieldsState,
    /// The why ladder's state for the subject key (#214).
    pub why: &'a super::why::WhyState,
    /// The deployment base, for building the wire chunks the observed-tree
    /// lookups below need.
    pub base: &'a str,
    /// The observed key tree — what has actually arrived (#234): the
    /// declared-subjects section joins the registry's claims against it, so
    /// a subject declared and never seen is loud rather than invisible.
    pub observed: &'a KeyTreeSnapshot,
    /// The dock's resolved spacing grid (#192) — Comfortable by default: a
    /// form wants air.
    pub sp: Spacing,
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
    scrollable(body.spacing(d.sp.md).padding(d.sp.sm))
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
        // The subject, restated in the pane. The one TITLE lives in the
        // location bar since #185.
        kit::emphasis(prefix).font(iced::Font::MONOSPACE),
        kit::empty_state(
            "A subtree, not a key",
            "Nothing was fetched, because a prefix names no value any producer \
             publishes. Expand it and select a leaf.",
        ),
    ]
}

fn key_sections<'a>(key: &'a str, d: &InspectorData<'a>) -> Column<'a, Message> {
    let mut col = detail::section(DetailData {
        slot: d.slot,
        key,
        facts: d.facts,
        fetched: d.fetched,
        decoded: d.decoded,
        series: d.series,
        history_entries: d.history.map(|r| r.ring.len()),
        observed: d.history.and_then(|r| r.ring.newest()),
        latency: d.latency.clone(),
        sp: d.sp,
    });

    // The declared type's place in the registry vocabulary (#234) — the
    // type-subject arm the engine's interface projections answer.
    if let Some(section) = type_section(d) {
        col = col.push(section);
    }

    // The plane sections, when the key is on one. Order matters: the plane is
    // the more specific fact, so it reads after the general one.
    match plane(d.facts) {
        Some(ClassKind::Blob) => {
            col = col.push(blob::section(d.blob, d.slices.is_some(), d.sp));
        }
        Some(ClassKind::Media) => {
            col = col.push(media::section(d.media, d.slices, d.sp));
        }
        _ => {}
    }

    // The bounded field observation (#223) and the why ladder (#214): both
    // follow the slot's subject, both cost only what their buttons say — and
    // both spend the dock's resolved grid (#192).
    col = col.push(super::fields::section(d.fields, d.slot, d.sp));
    col = col.push(super::why::section(d.why, d.slot, d.sp));

    col.push(history::section(HistoryData {
        slot: d.slot,
        key: Some(key),
        recorder: d.history,
        watched: d.watched,
        scroll: d.history_scroll,
        sp: d.sp,
    }))
}

fn origin_sections<'a>(origin: &'a str, d: &InspectorData<'a>) -> Column<'a, Message> {
    // The roster→slice join is the engine's `node_rows` (#234) — the same
    // rows `zenctl node list --verbose` prints, not a scan of this pane's
    // own. `live_map()` is `None` while unseeded, so an unasked join stays
    // "not asked" (O4).
    let joined = d
        .roster
        .live_map()
        .map(|m| zenkey_fleet::node_rows(&m, d.slices));
    column![
        kit::section_header("Inspector", None),
        // The subject, restated in the pane. The one TITLE lives in the
        // location bar since #185.
        kit::emphasis(origin).font(iced::Font::MONOSPACE),
        nodes::presence_section(d.roster, origin, joined.as_ref(), d.sp),
        nodes::detail_section(d.node_detail, d.sp),
    ]
    .push(declared_subjects(origin, d))
}

/// How many declared-subject rows render before the list stops and says so —
/// sysinfo alone declares 121, and an unbounded list is the O6 bug this
/// project already fixed once for the key tree (RFC 09 §5.1 O6).
const DECLARED_ROWS: usize = 40;

/// What this origin's producers **declare**, joined against what has been
/// **observed** — #234's headline. The tree answers "what is publishing?";
/// this section answers "what did the registry promise?", and the difference
/// between the two questions is the registry itself: a subject declared and
/// never published used to be invisible in the GUI, which is precisely the
/// case an explorer should make loud.
///
/// The rows are [`SliceSet::topic_list`] — the engine's projection, declared
/// not observed — with the ledger rows included, because "which hosts still
/// serve a deprecated subject" is RFC 08 §6's headline buy.
fn declared_subjects<'a>(origin: &'a str, d: &InspectorData<'a>) -> Column<'a, Message> {
    let mut col = column![kit::section_header("declared subjects", None)].spacing(d.sp.xs);
    let Some(slices) = d.slices else {
        return col.push(kit::muted(
            "no registry loaded — what this origin's producers declare is \
             unknown (not asked, O4)",
        ));
    };
    let Some(producers) = d.roster.get(origin) else {
        return col.push(kit::muted(
            "no liveliness token observed — the producers to project are \
             unknown (RFC 05 §3.1)",
        ));
    };
    let mut shown = 0usize;
    let mut cut = 0usize;
    for producer in producers.keys() {
        // Instance suffixes share the base slice (RFC 03 §1.5) — the same
        // resolution the engine's `node_rows` applies.
        let name = zenkey::grammar::Producer::parse_chunk(producer)
            .map(|p| p.name().to_string())
            .unwrap_or_else(|_| producer.clone());
        if slices.get(&name).is_none() {
            col = col.push(kit::muted(format!(
                "{producer}: no slice declares it — its subjects are unknown, \
                 not absent (O4)"
            )));
            continue;
        }
        let list = match slices.topic_list(Some(&name), None, None, true) {
            Ok(list) => list,
            Err(e) => {
                col = col.push(kit::muted(format!("{producer}: {e}")));
                continue;
            }
        };
        let declared = list.subjects.iter().filter(|r| !r.deprecated).count();
        let observed = list
            .subjects
            .iter()
            .filter(|r| !r.deprecated)
            .filter(|r| subject_observed(d.observed, d.base, origin, producer, r))
            .count();
        col = col.push(kit::muted(format!(
            "{producer}: {declared} declared subject(s) · {observed} observed \
             on this origin"
        )));
        for row in &list.subjects {
            if shown >= DECLARED_ROWS {
                cut += 1;
                continue;
            }
            shown += 1;
            col = col.push(declared_row(
                row,
                subject_observed(d.observed, d.base, origin, producer, row),
                d.sp,
            ));
        }
    }
    if cut > 0 {
        col = col.push(kit::muted(format!("+{cut} more not shown (display bound)")));
    }
    col
}

/// One declared subject, with the declared-vs-observed verdict beside it.
///
/// "Not observed by this session" is worded to what the evidence supports:
/// the tree holds only what watches let through, so absence there is a fact
/// about this window's coverage, never proof nothing publishes (O4/O5).
fn declared_row<'a>(row: &TopicRow, observed: bool, sp: Spacing) -> Element<'a, Message> {
    if row.deprecated {
        return kit::muted(format!(
            "{} DEPRECATED{}{}",
            row.path,
            row.deprecated_since
                .as_deref()
                .map(|s| format!(" since {s}"))
                .unwrap_or_default(),
            row.replaced_by
                .as_deref()
                .map(|r| format!(" — replaced by {r}"))
                .unwrap_or_default(),
        ));
    }
    let status = match (row.open_ended, observed) {
        (false, true) => "observed on this origin".to_string(),
        (false, false) => {
            "declared — not observed by this session (only watched keys are seen)".to_string()
        }
        // The registry fixes an open-ended family's shape, not its members
        // (RFC 08 §2), so members are only checkable at the literal prefix.
        (true, true) => "family — observed under its prefix on this origin".to_string(),
        (true, false) => "family — nothing observed under its prefix by this session".to_string(),
    };
    iced::widget::row![
        kit::mono(format!("{} {} ({})", row.class, row.path, row.type_name)),
        kit::muted(status),
    ]
    .spacing(sp.sm)
    .align_y(iced::Alignment::Center)
    .into()
}

/// Whether anything has been observed under one declared subject on one
/// origin, judged against the tick's tree snapshot — zero bus cost, the same
/// in-memory join the roster's freshness column makes.
///
/// The chunks are assembled from parsed identities: the origin comes from a
/// liveliness token, the class and path from a registry slice — never from
/// user input. A `{var}` position truncates to the family's literal prefix,
/// and a service origin carries no producer chunk (RFC 06 §5).
fn subject_observed(
    tree: &KeyTreeSnapshot,
    base: &str,
    origin: &str,
    producer: &str,
    row: &TopicRow,
) -> bool {
    let mut chunks: Vec<&str> = Vec::new();
    if !base.is_empty() {
        chunks.extend(base.split('/'));
    }
    chunks.push("v1");
    chunks.push(origin);
    chunks.push(&row.class);
    if !origin.starts_with('@') {
        chunks.push(producer);
    }
    for c in row.path.split('/') {
        if c.contains('{') {
            break;
        }
        chunks.push(c);
    }
    tree.node(&chunks).is_some()
}

/// The subject's payload type against the registry's type vocabulary (#234):
/// [`SliceSet::interface_list`] for where it sits in the vocabulary,
/// [`SliceSet::interface_show`] for every declaration that carries it — the
/// engine's projections, consumed rather than re-derived from the slices.
fn type_section<'a>(d: &InspectorData<'a>) -> Option<Column<'a, Message>> {
    let slices = d.slices?;
    let Registration::Registered(subject) = &d.facts?.registration else {
        return None;
    };
    let type_name = subject.type_name.as_str();
    let mut col = column![kit::section_header("Type", None)].spacing(d.sp.xs);
    let vocabulary = slices.interface_list();
    let carriers = vocabulary
        .types
        .iter()
        .find(|t| t.name == type_name)
        .map(|t| t.carriers)
        .unwrap_or(0);
    col = col.push(kit::muted(format!(
        "{type_name} — one of {} declared payload type(s), carried by \
         {carriers} declaration(s)",
        vocabulary.types.len(),
    )));
    match slices.interface_show(type_name) {
        Ok(show) => {
            // Bounded: TelemetryPoint alone has a three-digit carrier list,
            // and the bound is disclosed rather than silent (O6).
            const CARRIER_ROWS: usize = 12;
            for c in show.carriers.iter().take(CARRIER_ROWS) {
                col = col.push(kit::mono(format!(
                    "{} · {} {}",
                    c.producer, c.class, c.path
                )));
            }
            if show.carriers.len() > CARRIER_ROWS {
                col = col.push(kit::muted(format!(
                    "+{} more carrier(s) not shown (display bound)",
                    show.carriers.len() - CARRIER_ROWS
                )));
            }
        }
        // Unreachable while the type came off a Registered rung of the same
        // slices, but a projection that errors is rendered, not swallowed.
        Err(e) => col = col.push(kit::muted(e.to_string())),
    }
    Some(col)
}

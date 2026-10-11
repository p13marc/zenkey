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
//! Which plane a key sits on is read off its classification — a zk2 key's
//! kind token, from its identity (#614, FL7), or a v1 key's [`ClassKind`],
//! from its v1 facts projected on demand — never a `key.contains("@blob")`.
//! `keyfacts` is the single seam where a key becomes convention-aware (this
//! crate's whole key-agnostic-core claim), and a second, string-shaped
//! classifier here would be the thing that claim forbids. The `@blob` and
//! `@media` sections are v1's planes, foreign keys' since the port, until
//! FL11 removes them: no zk2 kind token opens one.
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
use zenkey_fleet::LatencyReport;
use zenkey_fleet_v1::SliceSet;
use zenkey_fleet_v1::model::facts::{ClassKind, KeyFacts, KeyShape, Registration};

use crate::keyfacts::{KeyGroup, Resolution};

use super::detail::{DetailData, Fetched, SeriesData};
use super::history::HistoryData;
use super::{blob, detail, history, kit, media, nodes};
use crate::blob::BlobState;
use crate::history::HistoryRecorder;
use crate::message::{Message, SlotId, Subject};
use crate::nodes::NodesState;
use crate::presence_link::PresenceUpdate;
use crate::view::media::MediaState;
use crate::view::tokens::{Spacing, face};

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
    /// The key subject's zk2 identity (#614, FL7): the identity line the
    /// Detail section renders, and a zk2 key's plane.
    pub identity: Option<&'a Resolution>,
    /// The key subject's v1 facts, projected on demand
    /// ([`crate::keyfacts::v1_facts`]): a v1 key's blob and media planes,
    /// the Config tool's target and the registry's type vocabulary, until
    /// FL11 removes them.
    pub v1: Option<KeyFacts>,
    pub fetched: Fetched<'a>,
    pub decoded: Option<&'a crate::value::DecodedValue>,
    pub series: Option<&'a SeriesData>,
    pub history: Option<&'a HistoryRecorder>,
    /// The timeline's scroll offset and viewport height (#183).
    pub history_scroll: crate::view::kit::Viewport,
    /// Whether an active watch covers the subject key — the distinction the
    /// History section rests on.
    pub watched: bool,
    /// The loaded `.zsnap` (#219), when one is: the History section offers
    /// a comparison against its row for the subject key.
    pub snapshot: Option<&'a zenkey_fleet_v1::Snapshot>,
    /// Whether that comparison is showing, on this slot.
    pub compare_snapshot: bool,
    pub latency: Option<(LatencyReport, u64)>,
    pub blob: &'a BlobState,
    pub media: &'a MediaState,
    pub slices: Option<&'a SliceSet>,
    /// The presence link's last landing (#614, FL9): an address subject's
    /// status, since when, and its instances. `None` = not read (O4).
    pub presence: Option<&'a PresenceUpdate>,
    /// The nodes pane's model: an address's descriptors, its freshness
    /// join, and the health reading of the address selected.
    pub nodes: &'a NodesState,
    /// The bounded field observation on the subject key (#223).
    pub fields: &'a super::fields::FieldsState,
    /// The why ladder's state for the subject key (#214).
    pub why: &'a super::why::WhyState,
    /// The declared readers of the subject (#224).
    pub consumers: &'a super::consumers::ConsumersState,
    /// The dock's resolved spacing grid (#192) — Comfortable by default: a
    /// form wants air.
    pub sp: Spacing,
}

/// Which plane a key sits on, when it sits on one at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Plane {
    /// A zk2 data key, placed by its kind token (#614, FL7). No kind token
    /// opens a plane section: `@stream` and `@state` are read like any
    /// other stream and state.
    Zk2,
    /// A v1 key's class — foreign since the port, and the only keys the
    /// `@blob` and `@media` sections still read, until FL11.
    V1(ClassKind),
}

/// Which plane a key sits on: a zk2 key's kind token first, else a v1
/// key's class.
///
/// `None` for anything neither grammar placed: an unparsed key, a key
/// outside the namespace, or no identity yet. "We have not classified it"
/// and "it is on the ordinary planes" render the same way here, which is
/// correct — neither adds a plane section.
fn plane(identity: Option<&Resolution>, v1: Option<&KeyFacts>) -> Option<Plane> {
    if let Some(KeyGroup::Resource { token, .. }) = identity.map(|r| &r.identity.group)
        && token.parse::<zenkey_model::grammar::KindToken>().is_ok()
    {
        return Some(Plane::Zk2);
    }
    match &v1?.shape {
        KeyShape::V1(f) => Some(Plane::V1(f.class_kind)),
        _ => None,
    }
}

pub fn pane<'a>(d: InspectorData<'a>) -> Element<'a, Message> {
    let body = match d.subject {
        Subject::None => nothing_selected(),
        Subject::Key(key) => key_sections(key, &d),
        Subject::Prefix(prefix) => prefix_sections(prefix, &d),
        Subject::Address(addr) => address_sections(addr, &d),
    };
    scrollable(body.spacing(d.sp.md).padding(d.sp.sm))
        .height(Length::Fill)
        .into()
}

fn nothing_selected<'a>() -> Column<'a, Message> {
    column![
        kit::section_header("Inspector", None),
        kit::empty(
            kit::EmptyKind::Unselected,
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
fn prefix_sections<'a>(prefix: &'a str, d: &InspectorData<'a>) -> Column<'a, Message> {
    column![
        kit::section_header("Inspector", None),
        // The subject, restated in the pane. The one TITLE lives in the
        // location bar since #185.
        kit::emphasis(prefix).font(face::MONO),
        kit::empty(
            kit::EmptyKind::Inapplicable,
            "A subtree, not a key",
            "Nothing was fetched, because a prefix names no value any producer \
             publishes. Expand it and select a leaf.",
        ),
        // A subtree has readers even though it has no value (#224): the
        // sweep relates declarations to `<prefix>/**`.
        super::consumers::section(d.consumers, d.slot, d.sp),
    ]
}

fn key_sections<'a>(key: &'a str, d: &InspectorData<'a>) -> Column<'a, Message> {
    let mut col = detail::section(DetailData {
        slot: d.slot,
        key,
        identity: d.identity,
        config: d.v1.as_ref().and_then(crate::configure::config_target),
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
    match plane(d.identity, d.v1.as_ref()) {
        Some(Plane::V1(ClassKind::Blob)) => {
            col = col.push(blob::section(d.blob, d.slices.is_some(), d.sp));
        }
        Some(Plane::V1(ClassKind::Media)) => {
            col = col.push(media::section(d.media, d.slices, d.sp));
        }
        Some(Plane::Zk2 | Plane::V1(_)) | None => {}
    }

    // The bounded field observation (#223) and the why ladder (#214): both
    // follow the slot's subject, both cost only what their buttons say — and
    // both spend the dock's resolved grid (#192).
    col = col.push(super::fields::section(d.fields, d.slot, d.sp));
    col = col.push(super::why::section(d.why, d.slot, d.sp));
    // The declared readers (#224): one admin sweep per click, never
    // ambient — and never worded as matching status (RFC 12 §9).
    col = col.push(super::consumers::section(d.consumers, d.slot, d.sp));

    col.push(history::section(HistoryData {
        slot: d.slot,
        key: Some(key),
        recorder: d.history,
        watched: d.watched,
        scroll: d.history_scroll,
        snapshot: d
            .snapshot
            .and_then(|s| s.rows.iter().find(|r| r.key == key).map(|r| (r, &s.header))),
        compare_snapshot: d.compare_snapshot,
        sp: d.sp,
    }))
}

/// An address names a service, and a service has presence, instances and a
/// health — no value, no decode, no chart (#614, FL9; D10). The sections are
/// the nodes pane's own, called unchanged: one claim, one wording.
fn address_sections<'a>(
    addr: &'a zenkey_model::grammar::Addr,
    d: &InspectorData<'a>,
) -> Column<'a, Message> {
    column![
        kit::section_header("Inspector", None),
        // The subject, restated in the pane. The one TITLE lives in the
        // location bar since #185.
        kit::emphasis(addr.to_string()).font(face::MONO),
        nodes::address_sections(addr, d.presence, d.nodes, d.sp),
    ]
}

/// The subject's payload type against the registry's type vocabulary (#234):
/// [`SliceSet::interface_list`] for where it sits in the vocabulary,
/// [`SliceSet::interface_show`] for every declaration that carries it — the
/// engine's projections, consumed rather than re-derived from the slices.
fn type_section<'a>(d: &InspectorData<'a>) -> Option<Column<'a, Message>> {
    let slices = d.slices?;
    let Registration::Registered(subject) = &d.v1.as_ref()?.registration else {
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

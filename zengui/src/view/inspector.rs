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
//! ## No plane sections
//!
//! A key's plane once added sections: v1's `@blob` and `@media` planes, read
//! off the key's classification and never off a `key.contains("@blob")`.
//! They were deleted in #614 (FL11), because no zk2 profile serves either
//! plane yet (#613), and no zk2 kind token opens a section: `@stream` and
//! `@state` are read like any other stream and state. A plane section that
//! comes back must be read from the classification seam, `keyfacts`, as
//! these were.
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

use crate::keyfacts::Resolution;

use super::detail::{DetailData, Fetched, LastKnown, SeriesData};
use super::history::HistoryData;
use super::{detail, history, kit, nodes};
use crate::history::HistoryRecorder;
use crate::message::{Message, SlotId, Subject};
use crate::nodes::NodesState;
use crate::presence_link::PresenceUpdate;
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
    /// Detail section renders, and the type section's one input (FL8a).
    pub identity: Option<&'a Resolution>,
    pub fetched: Fetched<'a>,
    /// Whether the panes show a replay (#614, FL8a): the value is the
    /// capture's row, and nothing is asked of the bus.
    pub replaying: bool,
    /// The `archive.v1` providers presence shows (#614, FL8a); `None` =
    /// presence not read.
    pub archives: Option<Vec<zenkey_model::grammar::Addr>>,
    /// An archive's last-known answer for the slot, when one was asked.
    pub last_known: Option<&'a LastKnown>,
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
    pub snapshot: Option<&'a zenkey_fleet::Snapshot>,
    /// Whether that comparison is showing, on this slot.
    pub compare_snapshot: bool,
    pub latency: Option<(LatencyReport, u64)>,
    /// The presence link's last landing (#614, FL9): an address subject's
    /// status, since when, and its instances. `None` = not read (O4).
    pub presence: Option<&'a PresenceUpdate>,
    /// The nodes pane's model: an address's descriptors, its freshness
    /// join, and the health reading of the address selected.
    pub nodes: &'a NodesState,
    /// The bounded field observation on the subject key (#223).
    pub fields: &'a super::fields::FieldsState,
    /// The why ladder's state for the subject key or service (#214).
    pub why: &'a super::why::WhyState,
    /// The roles bound to the subject (#224; R3 since #614, FL8b).
    pub consumers: &'a super::consumers::ConsumersState,
    /// The dock's resolved spacing grid (#192) — Comfortable by default: a
    /// form wants air.
    pub sp: Spacing,
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
        // A subtree has readers even though it has no value (#224): a
        // service's or an interface's subtree names whom the R3 edges bind.
        super::consumers::section(d.consumers, d.slot, d.sp),
    ]
}

fn key_sections<'a>(key: &'a str, d: &InspectorData<'a>) -> Column<'a, Message> {
    let mut col = detail::section(DetailData {
        slot: d.slot,
        key,
        identity: d.identity,
        fetched: d.fetched,
        decoded: d.decoded,
        series: d.series,
        history_entries: d.history.map(|r| r.ring.len()),
        observed: d.history.and_then(|r| r.ring.newest()),
        latency: d.latency.clone(),
        replaying: d.replaying,
        archives: d.archives.clone(),
        last_known: d.last_known,
        sp: d.sp,
    });

    // The declared type and its place in the revision (#234; zk2's since
    // #614, FL8a) — the type-subject arm the contract answers.
    if let Some(section) = type_section(d) {
        col = col.push(section);
    }

    // The bounded field observation (#223) and the why ladder (#214): both
    // follow the slot's subject, both cost only what their buttons say — and
    // both spend the dock's resolved grid (#192).
    col = col.push(super::fields::section(d.fields, d.slot, d.sp));
    col = col.push(super::why::section(d.why, d.slot, d.sp));
    // The roles bound to it (#224; R3 since #614, FL8b): one press, read
    // from the presence read in hand — never worded as matching status.
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
/// the nodes pane's own, called unchanged: one claim, one wording. Since
/// FL8b it also has a silence to explain (why, for a service) and roles
/// bound to it (its R3 edges).
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
        super::why::section(d.why, d.slot, d.sp),
        super::consumers::section(d.consumers, d.slot, d.sp),
    ]
}

/// The key's declared type (#234; zk2's since #614, FL8a): the type its
/// resource names, as the contract defines it — kind, name and the artifact
/// it lives in ([`zenkey_fleet::type_view`]) — its attachment's when it
/// declares one, and the members of the revision that carry the same type,
/// from the revision's own schema view rather than a scan of the pane's.
fn type_section<'a>(d: &InspectorData<'a>) -> Option<Column<'a, Message>> {
    use zenkey_model::contract::Body;

    let resolved = d.identity?.resolved.as_ref()?;
    let (ty, attachment) = match &resolved.resource().body {
        Body::Data(data) => (
            zenkey_fleet::type_view(&data.type_),
            data.attachment.as_ref().map(zenkey_fleet::type_view),
        ),
        Body::Operation(op) => (zenkey_fleet::type_view(&op.response), None),
    };
    let mut col = column![kit::section_header("Type", None)].spacing(d.sp.xs);
    col = col.push(kit::fields(vec![
        kit::field("TYPE", kit::mono(ty.name.clone())),
        kit::field("KIND", kit::mono(ty.kind.clone())),
        kit::field(
            "SCHEMA",
            kit::mono(match &ty.schema {
                Some(id) => short_id(id),
                // A raw type names no artifact: its bytes are not a tool's
                // to read (§7.1).
                None => "— (a raw type names no artifact)".to_string(),
            }),
        ),
    ]));
    if let Some(att) = attachment {
        col = col.push(kit::muted(format!("attachment: {} {}", att.kind, att.name)));
    }
    match resolved.revision.schema_view(None, false) {
        Ok(view) => {
            // Bounded, and the bound disclosed rather than silent (O6).
            const CARRIER_ROWS: usize = 12;
            let carriers: Vec<_> = view
                .members
                .iter()
                .filter(|m| m.type_.name == ty.name)
                .collect();
            col = col.push(kit::muted(format!(
                "{} — carried by {} of {}'s {}",
                ty.name,
                kit::plural(carriers.len(), "member"),
                resolved.iface,
                kit::plural(view.members.len(), "member"),
            )));
            for c in carriers.iter().take(CARRIER_ROWS) {
                col = col.push(kit::mono(format!("{} · {}", c.resource, c.member)));
            }
            if carriers.len() > CARRIER_ROWS {
                col = col.push(kit::muted(format!(
                    "+{} more not shown (display bound)",
                    carriers.len() - CARRIER_ROWS
                )));
            }
        }
        // Unreachable for the revision the key resolved through, but a
        // projection that errors is rendered, not swallowed.
        Err(e) => col = col.push(kit::muted(e)),
    }
    Some(col)
}

/// `sha256:` and sixteen hex digits — the engine's own short form.
fn short_id(id: &str) -> String {
    let hex = id.strip_prefix("sha256:").unwrap_or(id);
    match hex.get(..16) {
        Some(head) if hex.len() > 16 => format!("sha256:{head}…"),
        _ => id.to_owned(),
    }
}

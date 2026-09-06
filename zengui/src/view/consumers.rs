//! The Inspector's Consumers section (#224): who *declares* a reader of
//! the slot's subject — the same join `zenctl registry consumers` prints,
//! same relations, same attribution.
//!
//! Its one law is RFC 12 §9's: this is not matching status. A row is a
//! declaration the admin space served, not proof anything reads the key,
//! and an admin space that does not answer renders **"not asked"**, never
//! an empty consumer set (RFC 13 §3 O4). The vocabulary here never says
//! "listening", "matching" or "unmatched".
//!
//! Cost posture: one button, one admin sweep (topology plus the five
//! declared-entity selectors), never ambient — a pane that quietly re-swept
//! the admin space to keep itself fresh would be the frugality failure the
//! RFC v1.18 note records.

use std::sync::Arc;

use iced::widget::{Column, row};
use zenkey_fleet::report::{AdminAnswer, Attribution, ConsumerRow, ConsumersReport, Relation};

use crate::message::{Message, PaneMsg, SlotId};
use crate::view::kit;
use crate::view::tokens::Spacing;

/// The section's interactions.
#[derive(Debug, Clone)]
pub enum ConsumersMsg {
    /// The "consumers?" button. Runs on the subject key, or on `<prefix>/**`
    /// for a subtree.
    Run,
    /// A sweep finished.
    Done(Result<Arc<ConsumersReport>, crate::services::ServiceError>),
}

/// The section's state — follows the subject.
#[derive(Default)]
pub struct ConsumersState {
    pub in_flight: bool,
    /// The target the running (or landed) sweep was asked about — the
    /// staleness guard.
    pub asked: Option<String>,
    pub report: Option<Result<Arc<ConsumersReport>, crate::services::ServiceError>>,
}

impl ConsumersState {
    /// A new subject: the old sweep answered for the old target.
    pub fn forget_subject(&mut self) {
        self.in_flight = false;
        self.asked = None;
        self.report = None;
    }
}

fn msg(slot: SlotId, m: ConsumersMsg) -> Message {
    Message::Pane(PaneMsg::Consumers(slot, m))
}

/// The relation, in the words `zenctl` uses — set relations between two
/// key expressions, which is all a declaration supports.
pub fn relation_label(relation: Relation) -> &'static str {
    match relation {
        Relation::Exact => "exact",
        Relation::Narrower => "narrower — a subset of the target",
        Relation::Wider => "wider — a superset of the target",
        Relation::Intersects => "intersects in part",
        Relation::Total => "total — a whole-base `**`, intersects everything",
    }
}

fn origin_label(r: &ConsumerRow) -> String {
    if !r.origins.is_empty() {
        return r.origins.join(" ");
    }
    match r.attribution {
        Attribution::Session => "session only, unattributed".to_string(),
        Attribution::ReportedOnly => "reported only — no session named".to_string(),
    }
}

/// How many rows render before the list stops and says so (O6).
const ROWS: usize = 24;

/// The Consumers section for a key or prefix subject. `slot` is the
/// subject slot the surface is bound to (#257) — `Run` carries it home.
pub fn section(state: &ConsumersState, slot: SlotId, sp: Spacing) -> Column<'_, Message> {
    let mut col = Column::new().spacing(sp.sm);

    let run_label = if state.in_flight {
        "asking…"
    } else {
        "consumers?"
    };
    let mut run = kit::action(kit::caption(run_label)).padding(sp.xs);
    if !state.in_flight {
        run = run.on_press(msg(slot, ConsumersMsg::Run));
    }
    col = col.push(kit::section_header("Consumers", Some(run.into())));

    let Some(report) = &state.report else {
        // Never-run is not an empty consumer set (O4).
        return col.push(kit::muted(
            "not asked yet — \"consumers?\" sweeps the admin space once for every \
             declared subscriber and querier related to this subject; a \
             declaration is not proof of use (RFC 12 §9)",
        ));
    };
    let report = match report {
        Ok(r) => r,
        Err(e) => return col.push(kit::muted(format!("consumers sweep failed: {e}"))),
    };

    match report.admin {
        AdminAnswer::NotAvailable => {
            return col.push(kit::muted(
                "no admin space answered — zenoh's adminspace.enabled is off by \
                 default; the declared readers are not asked, never none (RFC 13 §3 O4)",
            ));
        }
        AdminAnswer::Answered { answered, nodes } => {
            col = col.push(kit::muted(format!(
                "{answered} admin space(s) answered ({nodes} node(s) heard of) — a \
                 declaration is not proof of use; sessions behind an admin space that \
                 did not answer are not shown (O5)"
            )));
        }
    }
    if report.rows.is_empty() {
        return col.push(kit::muted(
            "nothing declared in the answering admin space(s) relates to this subject \
             — a reading of what was declared there, not a verdict about who reads it",
        ));
    }
    for r in report.rows.iter().take(ROWS) {
        let who = if r.is_self {
            format!("{}  (this zengui session)", r.zid)
        } else {
            r.zid.clone()
        };
        col = col.push(
            row![
                kit::mono(format!("{who} · {}", r.whatami.as_deref().unwrap_or("—"))),
                kit::muted(origin_label(r)),
            ]
            .spacing(sp.sm)
            .align_y(iced::Alignment::Center),
        );
        col = col.push(kit::muted(format!(
            "  {} {} — {}",
            match r.kind {
                zenkey_fleet::EntityKind::Querier => "querier",
                _ => "subscriber",
            },
            r.keyexpr,
            relation_label(r.relation)
        )));
    }
    if report.rows.len() > ROWS {
        col = col.push(kit::muted(format!(
            "+{} more not shown (display bound)",
            report.rows.len() - ROWS
        )));
    }
    if report.reply_elided > 0 {
        col = col.push(kit::muted(format!(
            "{} admin reply(ies) past the bound not kept — a reader among them is not \
             shown (O6)",
            report.reply_elided
        )));
    }
    col
}

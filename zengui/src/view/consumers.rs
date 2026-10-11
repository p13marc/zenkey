//! The Inspector's Consumers section (#224; zk2's since #614, FL8b): who
//! *binds* the slot's subject — the R3 edges the descriptors declare
//! ([`zenkey_fleet::Catalog::graph`]), kept to the roles whose provider is
//! the subject's address, and its interface when the subject names one.
//!
//! Its one law is D13's (and RFC 12 §9's before it): this is not matching
//! status. A row is a role a descriptor binds to a provider present now,
//! never proof that anything reads, and a subject no presence read stands
//! behind renders **"not asked"**, never an empty set (RFC 13 §3 O4). The
//! vocabulary here never says "listening", "matching" or "unmatched". v1's
//! admin-space join (declared subscribers and queriers by key expression)
//! went with v1: zk2's graph is read from descriptors and tokens only, never
//! inferred from traffic (R3).
//!
//! Cost posture: one button, and no bus at all — the graph is computed from
//! the presence read the lens already holds, on the update thread, because
//! it is session-free and bounded by the deployment's own descriptors.

use std::sync::Arc;

use iced::widget::{Column, row};
use zenkey_fleet::report::{BindingGraph, GraphEdge};

use crate::message::{Message, PaneMsg, SlotId};
use crate::view::kit;
use crate::view::tokens::Spacing;

/// The section's interactions.
#[derive(Debug, Clone)]
pub enum ConsumersMsg {
    /// The "consumers?" button: the R3 edges bound to the subject.
    Run,
}

/// What a subject's bindings are asked about: an address, and the interface
/// when the subject names one (a key, an interface's subtree).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsumersTarget {
    /// `<system>/<service>`.
    pub address: String,
    /// `<name>.v<major>`, when the subject names one.
    pub iface: Option<String>,
}

/// The R3 edges bound to a target, from one presence read's descriptors.
#[derive(Debug, Clone)]
pub struct Consumers {
    pub target: ConsumersTarget,
    /// Whether the presence read completed before its timeout (§8.1): an
    /// incomplete read may have missed a consumer.
    pub complete: bool,
    /// Each role bound to the target, sorted by consumer, role, provider.
    pub edges: Vec<GraphEdge>,
    /// Instances whose descriptor did not read: their roles are missing.
    pub undescribed: usize,
    /// Whether the subject is a key: a role binds an interface, and which of
    /// its resources it reads is its contract's to say, not a descriptor's.
    pub key: bool,
}

impl Consumers {
    /// `graph`'s edges bound to `target`: those whose provider is the
    /// target's address, and whose role requires the target's interface
    /// when it names one.
    pub fn of(graph: &BindingGraph, target: ConsumersTarget, key: bool) -> Consumers {
        let edges = graph
            .edges
            .iter()
            .filter(|e| e.provider == target.address)
            .filter(|e| target.iface.as_ref().is_none_or(|i| *i == e.interface))
            .cloned()
            .collect();
        Consumers {
            target,
            complete: graph.complete,
            edges,
            undescribed: graph.undescribed.len(),
            key,
        }
    }
}

/// What one press of the button found (#614, FL8b) — never an error: the
/// graph is in hand or it is not, and a subject names a service or not.
#[derive(Debug, Clone)]
pub enum ConsumersAnswer {
    /// The R3 edges the presence read's descriptors declare for the target.
    Read(Consumers),
    /// No presence read stands behind the lens: not asked, never none (O4).
    PresenceNotRead,
    /// The panes show a replay, whose lens holds no presence read — the live
    /// one describes another world (O4).
    Replaying,
    /// The subject names no service, and why.
    Inapplicable(&'static str),
}

/// The section's state — follows the subject.
#[derive(Default)]
pub struct ConsumersState {
    pub report: Option<Arc<ConsumersAnswer>>,
}

impl ConsumersState {
    /// A new subject: the old answer was about the old target.
    pub fn forget_subject(&mut self) {
        self.report = None;
    }
}

fn msg(slot: SlotId, m: ConsumersMsg) -> Message {
    Message::Pane(PaneMsg::Consumers(slot, m))
}

/// How many rows render before the list stops and says so (O6).
const ROWS: usize = 24;

/// The Consumers section for a key, prefix or address subject. `slot` is
/// the subject slot the surface is bound to (#257) — `Run` carries it home.
pub fn section(state: &ConsumersState, slot: SlotId, sp: Spacing) -> Column<'_, Message> {
    let mut col = Column::new().spacing(sp.sm);
    let run = kit::primary(kit::caption("consumers?"))
        .padding(sp.xs)
        .on_press(msg(slot, ConsumersMsg::Run));
    col = col.push(kit::section_header("Consumers", Some(run.into())));

    let Some(answer) = &state.report else {
        // Never-run is not an empty consumer set (O4).
        return col.push(kit::muted(
            "not asked yet — \"consumers?\" reads the roles the descriptors bind to this \
             subject (R3, from the presence read in hand); a binding is never proof that \
             anything reads it (D13)",
        ));
    };
    let c = match answer.as_ref() {
        ConsumersAnswer::Read(c) => c,
        ConsumersAnswer::PresenceNotRead => {
            return col.push(kit::empty(
                kit::EmptyKind::NotAsked,
                "Presence not read",
                "R3 edges come from descriptors, and no presence read stands behind the \
                 lens — the bindings are not asked, never none (RFC 13 §3 O4)",
            ));
        }
        ConsumersAnswer::Replaying => {
            return col.push(kit::muted(
                "not asked during a replay — the replay's lens holds no presence read, and \
                 the live one describes another world than the capture's (O4)",
            ));
        }
        ConsumersAnswer::Inapplicable(why) => return col.push(kit::muted(*why)),
    };
    let target = match &c.target.iface {
        Some(iface) => format!("{} · {iface}", c.target.address),
        None => c.target.address.clone(),
    };
    col = col.push(kit::muted(format!(
        "R3: from descriptors — {} bound to {target}; a binding names a provider of the \
         interface, never a claim that anything reads, nor of what flows (D13)",
        kit::plural(c.edges.len(), "role")
    )));
    if !c.complete {
        col = col.push(kit::muted(
            "the presence read was possibly incomplete (§8.1) — a role it missed is not shown",
        ));
    }
    if c.undescribed > 0 {
        col = col.push(kit::muted(format!(
            "{} whose descriptor did not read — their roles are missing (O5)",
            kit::plural(c.undescribed, "instance"),
        )));
    }
    if c.key {
        col = col.push(kit::muted(
            "a role binds the interface; which of its resources it reads is its contract's \
             to say, not the descriptor's",
        ));
    }
    if c.edges.is_empty() {
        return col.push(kit::empty(
            kit::EmptyKind::Empty,
            "No role binds this subject",
            "no descriptor in the presence read binds a role to it — a reading of what was \
             declared, not a verdict about who reads it",
        ));
    }
    // A card per binding (#564): the consumer and its role, then what it
    // binds — a declaration, never a status.
    for e in c.edges.iter().take(ROWS) {
        col = col.push(kit::card(
            Column::new()
                .spacing(sp.xs)
                .push(
                    row![
                        kit::mono(e.consumer.clone()),
                        kit::muted(format!("role {}", e.role)),
                    ]
                    .spacing(sp.sm)
                    .align_y(iced::Alignment::Center),
                )
                .push(kit::muted(format!(
                    "binds {} for {}",
                    e.provider, e.interface
                ))),
        ));
    }
    if c.edges.len() > ROWS {
        col = col.push(kit::muted(format!(
            "+{} more not shown (display bound)",
            c.edges.len() - ROWS
        )));
    }
    col
}

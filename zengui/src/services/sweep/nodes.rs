//! One address's health, asked on selection; who serves it, and what its
//! arbitrated roles follow, once the selection settles.

use std::sync::Arc;
use std::time::Duration;

use iced::Task;
use zenkey_model::grammar::Addr;

use crate::message::{Message, PaneMsg};
use crate::view::nodes::NodesMsg;

/// How far apart the two readings are (`health.v1` §2.2): a break of the
/// aggregation rule is a finding only when both show it.
pub const HEALTH_GRACE: Duration = Duration::from_secs(1);

/// One address's `health.v1` — the nodes pane's one data-plane cost, paid
/// on selection and never polled (the laziness ground rule, #84/#85; D11).
///
/// Read as `zenctl health <address>` reads it, through the engine's
/// `run_health` on the session **in** the namespace: presence and the
/// descriptor, then two GETs of the owner's `health.v1/state/**` a grace
/// apart. No window: a subscription long enough to measure a clock (31 s)
/// is no reading for a click, so the clock's ground is the operator's word
/// or none — and with none, a reply's age is unobservable, which the pane
/// says (freshness.v1 §2.6).
pub fn health(
    session: zenoh::Session,
    namespace: String,
    addr: Addr,
    clocks_synced: bool,
    timeout: Duration,
) -> Task<Message> {
    Task::perform(
        async move {
            let spec = zenkey_fleet::HealthSpec {
                timeout,
                window: None,
                grace: HEALTH_GRACE,
                clocks_synced,
                face: None,
            };
            let report = zenkey_fleet::run_health(
                &session,
                &namespace,
                zenkey_fleet::HealthTarget::One(addr.clone()),
                spec,
                |_| {},
            )
            .await;
            (addr, namespace, Arc::new(report))
        },
        |(addr, ran, report)| {
            Message::Pane(PaneMsg::Nodes(NodesMsg::HealthRead(addr, ran, report)))
        },
    )
}

/// One address's `redundancy.v1` (#841, RC), on selection beside its
/// health, never polled: read as `zenctl redundancy <address>` reads it,
/// through the engine's `run_redundancy` on the session **in** the
/// namespace — which holds no liveliness subscriber, the presence link's
/// monitor being on a watch session of its own — two presence reads a
/// grace apart, each with its descriptors, and the revisions they name
/// from the window's own store. Every role is the fleet's judge's.
pub fn redundancy(
    session: zenoh::Session,
    namespace: String,
    addr: Addr,
    store: zenkey_fleet::BundleStore,
    timeout: Duration,
) -> Task<Message> {
    Task::perform(
        async move {
            let report = zenkey_fleet::run_redundancy(
                &session,
                &namespace,
                Some(addr.clone()),
                zenkey_fleet::RedundancySpec::new(timeout),
                // The window's, seeded with the contracts held offline.
                &store,
            )
            .await;
            (addr, namespace, Arc::new(report))
        },
        |(addr, ran, report)| {
            Message::Pane(PaneMsg::Nodes(NodesMsg::RedundancyRead(addr, ran, report)))
        },
    )
}

/// How many of each role's own deadlines the Workbench's view listens
/// (#909, BC): one is `listening`'s threshold (§2.14), and a second lets a
/// commander arrive within it.
pub const ARBITRATION_DEADLINES: u32 = 2;

/// One address's `arbitration.v1` (#909, BC), at the selection's settle
/// beside `redundancy.v1`, never polled: read as
/// `zenctl arbitration <address>` reads it, through the engine's `run_arbitration` on the
/// session **in** the namespace — presence, the descriptor, the declaring
/// contract and every revision of the required interface in the window's
/// store — and the tool's own view over [`ARBITRATION_DEADLINES`] of each
/// role's deadline, its watch on the presence monitor's watch session (in
/// no namespace on the wire keys). Without one, the deadline alone, as the
/// report says. Every answer is the fleet's judge's.
pub fn arbitration(
    session: zenoh::Session,
    watch: Option<zenkey_fleet::WatchSession>,
    namespace: String,
    addr: Addr,
    store: zenkey_fleet::BundleStore,
    timeout: Duration,
) -> Task<Message> {
    Task::perform(
        async move {
            let mut spec = zenkey_fleet::ArbitrationSpec::new(timeout);
            spec.listen = Duration::ZERO;
            spec.deadlines = Some(ARBITRATION_DEADLINES);
            let offline = zenkey_fleet::ContractSet::new();
            let report = zenkey_fleet::run_arbitration(
                &session,
                watch.as_ref(),
                &namespace,
                Some(addr.clone()),
                // The window's, seeded with the contracts held offline.
                &store,
                &offline,
                spec,
            )
            .await;
            (addr, namespace, Arc::new(report))
        },
        |(addr, ran, report)| {
            Message::Pane(PaneMsg::Nodes(NodesMsg::ArbitrationRead(addr, ran, report)))
        },
    )
}

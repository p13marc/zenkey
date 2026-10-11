//! One address's health, asked on selection.

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
            )
            .await;
            (addr, namespace, Arc::new(report))
        },
        |(addr, ran, report)| {
            Message::Pane(PaneMsg::Nodes(NodesMsg::HealthRead(addr, ran, report)))
        },
    )
}

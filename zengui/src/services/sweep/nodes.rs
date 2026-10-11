//! One node's detail, asked on selection.

use std::sync::Arc;
use std::time::Duration;

use iced::Task;

use crate::message::{Message, PaneMsg};
use crate::services::ServiceError;
use crate::view::nodes::NodesMsg;

/// One node's detail — the nodes pane's one data-plane cost, paid on selection
/// (the laziness ground rule, #84/#85).
pub fn node_info(
    session: zenoh::Session,
    base: String,
    origin: String,
    timeout: Duration,
) -> Task<Message> {
    Task::perform(
        async move {
            let fleet = zenkey_fleet_v1::Fleet::new(&session, &base);
            let out = zenkey_fleet_v1::node_info(&fleet, &origin, timeout, true)
                .await
                .map(Arc::new)
                .map_err(ServiceError::of);
            (origin, base, out)
        },
        |(origin, ran, out)| Message::Pane(PaneMsg::Nodes(NodesMsg::InfoLoaded(origin, ran, out))),
    )
}

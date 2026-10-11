//! The consumers join for one target.

use std::sync::Arc;
use std::time::Duration;

use iced::Task;

use crate::message::{Message, PaneMsg};
use crate::services::ServiceError;

/// The consumers join for one target (#224): one pass over the admin
/// space — topology, the five declared-entity sweeps, the attachments the
/// token entities yield — and the engine's relation ranking. Never
/// ambient: this runs when the section's button says so, and the pane
/// states *not asked* until then (O4).
pub fn consumers_of(
    slot: crate::message::SlotId,
    session: zenoh::Session,
    base: String,
    target: String,
    timeout: Duration,
) -> Task<Message> {
    Task::perform(
        async move {
            let fleet = zenkey_fleet_v1::Fleet::new(&session, &base);
            zenkey_fleet_v1::consumers(&fleet, &target, timeout)
                .await
                .map(Arc::new)
                .map_err(ServiceError::of)
        },
        // The asking slot's id rides the landing (#257).
        move |out| {
            Message::Pane(PaneMsg::Consumers(
                slot,
                crate::view::consumers::ConsumersMsg::Done(out),
            ))
        },
    )
}

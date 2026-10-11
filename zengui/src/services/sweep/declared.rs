//! The declared keyspace's one bus input.
//!
//! Since #614 the tree starts from zk2's contract skeleton
//! (`zenkey_fleet::Skeleton`), built from the lens's own inputs — the
//! namespace, the presence read behind the catalog, the contract store and
//! the contracts held offline — on the update thread, because none of that
//! costs a query (`Deployment::refresh_skeleton`). What does is the admin
//! space's account of what sessions actually declared, asked here.

use std::sync::Arc;
use std::time::Duration;

use iced::Task;

use crate::message::{BusMsg, Message};
use crate::services::ServiceError;

/// Read the admin space's declared entities on the session in no namespace
/// — the admin space has none. `adminspace.enabled` defaults to false, so a
/// silent admin space is the common answer, and it lands as `Ok(None)`.
pub fn declared(raw: zenoh::Session, timeout: Duration) -> Task<Message> {
    Task::perform(
        async move {
            zenkey_fleet::declared_entities(&raw, timeout)
                .await
                .map(|d| d.map(Arc::new))
                .map_err(ServiceError::of)
        },
        |r| Message::Bus(BusMsg::DeclaredRead(r)),
    )
}

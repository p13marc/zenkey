//! The admin sweep.

use std::sync::Arc;
use std::time::Duration;

use anyhow::Context as _;
use iced::Task;
use zenkey_fleet_v1::SliceSet;

use crate::message::{Message, PaneMsg};
use crate::services::ServiceError;
use crate::view::admin::AdminMsg;

/// The admin sweep (#131): routers, storages, declared entities, the coverage
/// join, the topology and the origin attachments, as one act the user asked
/// for.
///
/// `slices` here is the app's *resolved* set — bus, dirs or the union — and
/// deliberately not the doctor's dirs-only one: the coverage join asks "what
/// does the registry say exists", and a dirs-only answer would empty the table
/// on every bus-registry-only deployment. An invisible, plausible-looking
/// wrong answer is worse than no table.
pub fn admin(
    session: zenoh::Session,
    base: String,
    slices: Option<Arc<SliceSet>>,
    timeout: Duration,
) -> Task<Message> {
    Task::perform(
        async move {
            let routers = zenkey_fleet_v1::routers(&session, timeout)
                .await
                .context("failed to list the routers")
                .map_err(ServiceError::of)?;
            let storages = zenkey_fleet_v1::storages(&session, timeout)
                .await
                .context("failed to list the storages")
                .map_err(ServiceError::of)?;
            let declared = zenkey_fleet_v1::declared_entities(&session, timeout)
                .await
                .context("failed to list the declared entities")
                .map_err(ServiceError::of)?;
            let (coverage, coverage_note) = match slices.as_deref() {
                Some(set) => (zenkey_fleet_v1::state_coverage(set, &base, &storages), None),
                None => (
                    Vec::new(),
                    Some(
                        "no registry slices resolved, so the declared state \
                         families are unknown — this is \"not asked\", not \
                         \"uncovered\" (RFC 09 §5.1 O4). Pass --registry, \
                         or wait for the bus registry (RFC 08 §6)."
                            .to_string(),
                    ),
                ),
            };
            let topology = zenkey_fleet_v1::topology(&session, timeout)
                .await
                .context("failed to read the topology")
                .map_err(ServiceError::of)?;
            let origins = zenkey_fleet_v1::origin_attachments(
                &zenkey_fleet_v1::Fleet::new(&session, &base),
                timeout,
            )
            .await
            .context("failed to read the origin attachments")
            .map_err(ServiceError::of)?;
            Ok(Arc::new(crate::admin::AdminSweep {
                routers,
                storage: zenkey_fleet_v1::report::StorageList { storages, coverage },
                declared,
                coverage_note,
                topology,
                origins,
                base,
            }))
        },
        |out| Message::Pane(PaneMsg::Admin(AdminMsg::Done(out))),
    )
}

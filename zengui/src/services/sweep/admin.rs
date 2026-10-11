//! The admin sweep.

use std::sync::Arc;
use std::time::Duration;

use anyhow::Context as _;
use iced::Task;

use crate::message::{Message, PaneMsg};
use crate::services::ServiceError;
use crate::view::admin::AdminMsg;

/// The admin sweep (#131; zk2's since #614, FL9): routers, storages, each
/// storage judged against S4, and the topology with zk2's instances joined
/// onto it, as one act the user asked for.
///
/// The admin space is read through `raw`, the session in no namespace (a
/// namespaced session's `@` selector is rewritten and matches nothing).
/// The instance join reads presence and descriptors through `deployment`,
/// the session in the namespace, as `zenctl admin graph` does
/// (`topology_with_instances`); without one the topology is the admin
/// space's alone and the join is "not asked". Only a verified router's
/// session list attaches an instance (§4.2): an answer any session could
/// have given under `@/<zid>/router` attaches nothing.
///
/// The declared entities are not read here: the window already holds them
/// (`Deployment::admin_declared`, the skeleton's input), and the sweep's
/// handler asks for them again beside this.
pub fn admin(
    raw: zenoh::Session,
    deployment: Option<zenoh::Session>,
    namespace: String,
    timeout: Duration,
) -> Task<Message> {
    Task::perform(
        async move {
            let routers = zenkey_fleet::routers(&raw, timeout)
                .await
                .context("failed to list the routers")
                .map_err(ServiceError::of)?;
            let storages = zenkey_fleet::storages(&raw, timeout)
                .await
                .context("failed to list the storages")
                .map_err(ServiceError::of)?;
            let topology = match &deployment {
                Some(session) => {
                    zenkey_fleet::topology_with_instances(&raw, session, &namespace, timeout, false)
                        .await
                }
                None => zenkey_fleet::topology(&raw, timeout).await,
            }
            .context("failed to read the topology")
            .map_err(ServiceError::of)?;
            let s4 = crate::admin::s4_rows(&namespace, &storages);
            Ok(Arc::new(crate::admin::AdminSweep {
                routers,
                storages,
                s4,
                topology,
                namespace,
            }))
        },
        |out| Message::Pane(PaneMsg::Admin(AdminMsg::Done(out))),
    )
}

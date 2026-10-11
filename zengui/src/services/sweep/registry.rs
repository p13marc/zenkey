//! The declared keyspace the tree starts from.
//!
//! Since #614 (FL6a) there is no v1 registry to build it from: `--registry`
//! is refused, the bus serves no v1 slices, and the window's declared world
//! is zk2's contracts (`services::contracts`). The skeleton is zk2's
//! (`zenkey_fleet::Skeleton`), built here from the admin space's declared
//! entities alone — so the tree shows what is observed, and nothing it
//! cannot back.

use std::sync::Arc;
use std::time::Duration;

use iced::Task;
use zenkey_fleet::{ContractSet, Lens, Skeleton};

use crate::message::{BusMsg, Message};

/// (Re)build the skeleton from the admin space's declared entities, asked
/// on the session in no namespace — the admin space has none.
pub fn skeleton(raw: zenoh::Session, namespace: String, timeout: Duration) -> Task<Message> {
    Task::perform(
        async move {
            let admin = zenkey_fleet::declared_entities(&raw, timeout)
                .await
                .unwrap_or(None);
            let none = ContractSet::new();
            let lens = Lens::new(&namespace, None, &none);
            Ok(Arc::new(Skeleton::build(&lens, admin.as_ref())))
        },
        |r| Message::Bus(BusMsg::SkeletonBuilt(r)),
    )
}

//! The declared keyspace the tree starts from.
//!
//! Since #614 (FL6a) there is no v1 registry to build it from: `--registry`
//! is refused, the bus serves no v1 slices, and the window's declared world
//! is zk2's contracts (`services::contracts`). Until FL6b swaps in the zk2
//! skeleton (`zenkey_fleet::Skeleton::build` over the lens), the v1 one is
//! built from **no** slices and no roster — the admin space's declared
//! entities alone — so the tree shows what is observed, and nothing it
//! cannot back.

use std::sync::Arc;
use std::time::Duration;

use iced::Task;
use zenkey_fleet_v1::{Skeleton, SliceSet};

use crate::message::{BusMsg, Message};

/// (Re)build the skeleton: no slices, no roster (#614, FL6a), and the admin
/// space's declared entities, asked on the session in no namespace — the
/// admin space has none.
pub fn skeleton(raw: zenoh::Session, namespace: String, timeout: Duration) -> Task<Message> {
    Task::perform(
        async move {
            let admin = zenkey_fleet_v1::declared_entities(&raw, timeout)
                .await
                .unwrap_or(None);
            let roster = crate::message::LiveRoster::default();
            let skeleton = Arc::new(Skeleton::build(
                &namespace,
                &SliceSet::default(),
                &roster,
                admin.as_ref(),
            ));
            Ok((skeleton, Arc::new(roster)))
        },
        |r| Message::Bus(BusMsg::SkeletonBuilt(r)),
    )
}

//! The registry slices — from the bus, from dirs, or their union — and the
//! skeleton built over them.

use std::sync::Arc;
use std::time::Duration;

use iced::Task;
use zenkey_fleet_v1::{Skeleton, SliceSet};

use crate::message::{BusMsg, Message};
use crate::services::ServiceError;

/// The registry slices, from the bus alone.
pub fn slices(session: zenoh::Session, base: String, timeout: Duration) -> Task<Message> {
    Task::perform(
        async move {
            SliceSet::from_bus(&zenkey_fleet_v1::Fleet::new(&session, &base), timeout)
                .await
                .map(Arc::new)
                .map_err(ServiceError::of)
        },
        |r| Message::Bus(BusMsg::SlicesLoaded(r)),
    )
}

/// The registry slices from the `--registry` dirs alone, for a session that
/// never opened (#196's doctrine, which zenctl already keeps): a transport
/// that will not come up leaves what is on disk answerable. Off the update
/// thread (#255) — it reads every file in every dir. Lands on
/// [`BusMsg::SlicesLoaded`], whose handler labels a set loaded with dirs
/// configured as dirs-only, which is what this is.
pub fn slices_from_dirs(dirs: Vec<std::path::PathBuf>) -> Task<Message> {
    Task::perform(
        async move {
            SliceSet::from_dirs(&dirs)
                .map(Arc::new)
                .map_err(ServiceError::of)
        },
        |r| Message::Bus(BusMsg::SlicesLoaded(r)),
    )
}

/// The §6.1 union (issue #43): served wins, dirs fill, and the disagreement
/// count reaches the status strip as data.
pub fn slices_union(
    session: zenoh::Session,
    base: String,
    dirs: Vec<std::path::PathBuf>,
    timeout: Duration,
) -> Task<Message> {
    Task::perform(
        async move {
            SliceSet::from_union(
                &zenkey_fleet_v1::Fleet::new(&session, &base),
                &dirs,
                timeout,
            )
            .await
            .map(|out| {
                let counts = crate::view::status::UnionCounts {
                    from_bus: out.from_bus.len(),
                    dirs_only: out.dirs_only.len(),
                    disagreements: out.disagreements.len(),
                    // Read before the set moves into the `Arc` (#399).
                    self_disagreements: self_disagreements(&out.set),
                };
                (Arc::new(out.set), counts)
            })
            .map_err(ServiceError::of)
        },
        |r| Message::Bus(BusMsg::SlicesUnionLoaded(r)),
    )
}

/// (Re)build the skeleton. Slices are already loaded; roster and admin are
/// gathered inside the task (both metadata-only).
pub fn skeleton(
    session: zenoh::Session,
    base: String,
    slices: Arc<SliceSet>,
    timeout: Duration,
) -> Task<Message> {
    Task::perform(
        async move {
            let fleet = zenkey_fleet_v1::Fleet::new(&session, &base);
            let roster = zenkey_fleet_v1::roster(&fleet, timeout)
                .await
                .unwrap_or_default();
            let admin = zenkey_fleet_v1::declared_entities(&session, timeout)
                .await
                .unwrap_or(None);
            let skeleton = Arc::new(Skeleton::build(&base, &slices, &roster, admin.as_ref()));
            Ok((skeleton, Arc::new(roster)))
        },
        |r| Message::Bus(BusMsg::SkeletonBuilt(r)),
    )
}

/// Producers the fleet does not agree with itself about (#399).
///
/// Zero for a set that never asked — files carry no origin — which is why the
/// status strip's `Dirs` variant has no such field at all rather than a zero
/// this would hand it (RFC 13 §3 O4).
pub fn self_disagreements(set: &SliceSet) -> usize {
    set.collapsed()
        .as_option()
        .copied()
        .unwrap_or(&[])
        .iter()
        .filter(|c| !c.agreed)
        .count()
}

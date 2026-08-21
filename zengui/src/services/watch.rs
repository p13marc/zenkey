//! Coverage: what this window has asked the bus to keep telling it (#175).
//!
//! Every function here takes the monitor rather than a session, because a watch
//! is a declaration against the *pump*, not a query against the bus. That the
//! monitor lives in the app's observation half and the timeout in its
//! deployment half is the honest statement that a watch is coverage over a
//! deployment.

use std::sync::Arc;

use iced::Task;
use zenkey_fleet::{Monitor, MonitorSpec, WatchId};

use crate::message::{BusMsg, DeploymentMsg, Message, SubjectMsg};

/// Start the pump.
///
/// No data-plane selectors: watches are lazy and opt-in (issue #85). Only
/// liveliness rides from the start, because it is zero payload by construction
/// (RFC 04 §5).
pub fn start_monitor(
    session: zenoh::Session,
    liveliness: Vec<String>,
    max_keys: usize,
) -> Task<Message> {
    Task::perform(
        async move {
            Monitor::start(
                &session,
                MonitorSpec {
                    selectors: vec![],
                    liveliness,
                    max_keys,
                    ..Default::default()
                },
            )
            .await
            .map(Arc::new)
            .map_err(|e| e.to_string())
        },
        |r| Message::Bus(BusMsg::MonitorStarted(r)),
    )
}

/// Watch one subtree, seeding it first (issue #92).
///
/// The seed is what makes an observation honest on arrival: current state comes
/// before live traffic, through the same merge discipline as everything else,
/// so an empty pane means "nothing is there" rather than "nothing has happened
/// since you looked".
pub fn subtree(
    monitor: Arc<Monitor>,
    path: String,
    selector: String,
    policy: zenkey_fleet::SeedPolicy,
) -> Task<Message> {
    Task::perform(
        async move {
            monitor
                .watch_seeded(&selector, policy)
                .await
                .map_err(|e| e.to_string())
        },
        move |r| Message::Subject(SubjectMsg::WatchStarted(path.clone(), r)),
    )
}

/// Release one subtree's watch.
pub fn release(monitor: Arc<Monitor>, path: String, id: WatchId) -> Task<Message> {
    Task::perform(
        async move { monitor.unwatch(id).await.map_err(|e| e.to_string()) },
        move |r| Message::Subject(SubjectMsg::WatchReleased(path.clone(), r)),
    )
}

/// Watch the whole active scope preset.
///
/// A failure per selector is warned and skipped rather than aborting the rest:
/// a scope is several selectors, and one that will not declare does not make
/// the others untrue.
pub fn scope(
    monitor: Arc<Monitor>,
    selectors: Vec<String>,
    policy: zenkey_fleet::SeedPolicy,
) -> Task<Message> {
    Task::perform(
        async move {
            let mut ids = Vec::new();
            for sel in &selectors {
                match monitor.watch_seeded(sel, policy).await {
                    Ok(id) => ids.push(id),
                    Err(e) => tracing::warn!("scope watch {sel}: {e}"),
                }
            }
            ids
        },
        |r| Message::Deployment(DeploymentMsg::ScopeWatchesStarted(r)),
    )
}

/// Release every scope watch.
pub fn release_scope(monitor: Arc<Monitor>, ids: Vec<WatchId>) -> Task<Message> {
    Task::perform(
        async move {
            for id in ids {
                if let Err(e) = monitor.unwatch(id).await {
                    tracing::warn!("scope unwatch: {e}");
                }
            }
            Ok(())
        },
        |r| Message::Deployment(DeploymentMsg::ScopeWatchesReleased(r)),
    )
}

/// Watch one media key — a plain watch, not a seeded one: a media stream's
/// past frames are not what the viewer asked for.
pub fn media(monitor: Arc<Monitor>, key: String) -> Task<Message> {
    Task::perform(
        async move { monitor.watch(&key).await.map_err(|e| e.to_string()) },
        |r| {
            Message::Pane(crate::message::PaneMsg::Media(
                crate::view::media::MediaMsg::Watched(r),
            ))
        },
    )
}

/// Release the media watch — the "unviewed streams cost zero subscriptions"
/// half of #69's contract.
pub fn release_media(monitor: Arc<Monitor>, id: WatchId) -> Task<Message> {
    Task::perform(
        async move {
            let _ = monitor.unwatch(id).await;
        },
        |()| {
            Message::Pane(crate::message::PaneMsg::Media(
                crate::view::media::MediaMsg::Stopped,
            ))
        },
    )
}

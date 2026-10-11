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
use crate::services::ServiceError;

/// Start the pump: zk2's monitor on the session in no namespace (#614,
/// FL6b), so a wire selector reaches any key on the bus.
///
/// No data-plane selectors: watches are lazy and opt-in (issue #85). No
/// liveliness either since #614 (FL6a): presence is zk2's instance and
/// interface tokens, read by its own monitor on the session in the
/// namespace ([`start_presence`]).
pub fn start_monitor(session: zenoh::Session, max_keys: usize) -> Task<Message> {
    Task::perform(
        async move {
            Monitor::start(
                &session,
                MonitorSpec {
                    selectors: vec![],
                    max_keys,
                    ..Default::default()
                },
            )
            .await
            .map(Arc::new)
            .map_err(ServiceError::of)
        },
        |r| Message::Bus(BusMsg::MonitorStarted(r)),
    )
}

/// End the pump with its teardown **acknowledged** — every watch undeclared
/// and waited for, rather than left to a dropped handle.
///
/// Reopening the session builds a new monitor over the same keys, and the old
/// one's subscribers were undeclaring in the background while the new one's
/// were declaring. [`Monitor::shutdown`] consumes the monitor and zengui holds
/// it through an `Arc` (every watch service takes one), so this can only make
/// the acknowledged teardown when this handle is the last — which it normally
/// is, the other clones living inside watch tasks that have already finished.
/// When it is not, dropping the clone falls back to `Drop`'s abort, which is
/// exactly what happened before and no worse.
pub fn shutdown(monitor: Arc<Monitor>) -> Task<Message> {
    Task::future(async move {
        match Arc::try_unwrap(monitor) {
            Ok(monitor) => {
                if let Err(e) = monitor.shutdown().await {
                    tracing::warn!("monitor shutdown: {e}");
                }
            }
            Err(shared) => {
                tracing::debug!("monitor still shared on teardown; falling back to drop");
                drop(shared);
            }
        }
    })
    .discard()
}

/// Start the presence monitor on the session in `namespace` (#614, FL6a):
/// zk2's three token selectors and nothing else
/// (`MonitorSpec::with_presence`), its own broadcast, and no stats ticks
/// worth the name — zenwatch's shape. Lands on `PresenceStarted` with the
/// namespace it was started in, so one the window has since left is
/// dropped.
pub fn start_presence(session: zenoh::Session, namespace: String) -> Task<Message> {
    Task::perform(
        async move {
            let spec = MonitorSpec {
                // Presence moves arrive as events; a stats tick on a monitor
                // that watches no samples would be noise.
                stats_tick: std::time::Duration::from_secs(3600),
                ..MonitorSpec::default()
            }
            .with_presence("", &zenkey_fleet::PresenceScope::all());
            let r = Monitor::start(&session, spec)
                .await
                .map(Arc::new)
                .map_err(ServiceError::of);
            (namespace, r)
        },
        |(namespace, r)| Message::Bus(BusMsg::PresenceStarted(namespace, r)),
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
                .map_err(ServiceError::of)
        },
        move |r| Message::Subject(SubjectMsg::WatchStarted(path.clone(), r)),
    )
}

/// Release one subtree's watch.
pub fn release(monitor: Arc<Monitor>, path: String, id: WatchId) -> Task<Message> {
    Task::perform(
        async move { monitor.unwatch(id).await.map_err(ServiceError::of) },
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
        async move { monitor.watch(&key).await.map_err(ServiceError::of) },
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

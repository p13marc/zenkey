//! Opening the sessions, and what the app asks the moment they exist (#175).
//!
//! **Two sessions** since #614 (FL6a), as zenctl and zenwatch run (decided
//! 2026-10-08): one **in** the deployment's namespace, for what reads the
//! deployment the way its own consumers do — presence, descriptors,
//! contracts, the lens, state — and one in **none**, for the data pump's
//! wire selectors and the admin space, which is what lets the window see a
//! key outside the deployment and say so (the tooling guide's O1).

use std::time::Duration;

use iced::Task;

use crate::message::{BusMsg, Message};
use crate::services::{ServiceError, ServiceResult};

/// The connection settings, climbed once: what both sessions are opened
/// from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transport {
    pub zenoh_config: Option<std::path::PathBuf>,
    pub connect: Vec<String>,
    pub listen: Vec<String>,
    pub scouting: Option<bool>,
}

impl Transport {
    /// The transport half of the settings in force.
    pub fn of(s: &crate::config::Settings) -> Transport {
        Transport {
            zenoh_config: s.zenoh_config.clone(),
            connect: s.connect.clone(),
            listen: s.listen.clone(),
            scouting: s.scouting,
        }
    }
}

/// The two sessions a zk2 window runs on (#614, FL6a).
#[derive(Debug, Clone)]
pub struct Sessions {
    /// In no namespace: the data pump, wire selectors, the admin space.
    pub raw: zenoh::Session,
    /// In the deployment's namespace: presence, descriptors, contracts.
    pub session: zenoh::Session,
    /// The namespace `session` was opened in; `""` is the bus root.
    pub namespace: String,
}

/// An open's outcome, with the half that failed kept recognisable: a
/// transport that would not come up is [`NeverOpened`], a config the user
/// named is theirs (#196).
fn spend<T>(opened: Result<T, zenkey_fleet::OpenFailure>) -> ServiceResult<T> {
    opened.map_err(|f| match f {
        zenkey_fleet::OpenFailure::Config(e) => ServiceError::of(e),
        zenkey_fleet::OpenFailure::Transport(e) => {
            ServiceError::of(anyhow::Error::new(NeverOpened(e)))
        }
    })
}

/// The session in no namespace.
async fn raw(t: &Transport) -> ServiceResult<zenoh::Session> {
    spend(
        zenkey_fleet::open_reporting(t.zenoh_config.as_deref(), &t.connect, &t.listen, t.scouting)
            .await,
    )
}

/// The session in `namespace`.
async fn namespaced(namespace: &str, t: &Transport) -> ServiceResult<zenoh::Session> {
    spend(
        zenkey_fleet::open_in_namespace(
            namespace,
            t.zenoh_config.as_deref(),
            &t.connect,
            &t.listen,
            t.scouting,
        )
        .await,
    )
}

/// A **watch session** for `namespace` (#830, core §8.1, 0.37; #875): a
/// session of its own over the same connection, in **no** namespace, for
/// the presence monitor's liveliness subscribers on the deployment's wire
/// keys, without history, so its round trip is one round trip at any
/// population until its transport moves (core §8.1, 0.44, F-239). It makes
/// no read, so the session in the namespace holds no
/// liveliness subscriber and makes every read: a liveliness GET beside one
/// can complete without a token that is live. One more session per window,
/// reopened with the others on a namespace switch.
pub async fn watch_session(
    namespace: &str,
    t: &Transport,
) -> ServiceResult<zenkey_fleet::WatchSession> {
    spend(
        zenkey_fleet::open_watch(
            namespace,
            t.zenoh_config.as_deref(),
            &t.connect,
            &t.listen,
            t.scouting,
        )
        .await,
    )
}

/// Both sessions, the raw one first: a dead transport is said once, by the
/// first open, and the second is never tried against it.
///
/// The launch open and the reconnect build this same call from the same
/// settings. They land on *different* messages — the launch on
/// `BusMsg::SessionsOpened`, the reconnect on the Connect pane's own
/// `Switched`, because that is where a failed context switch is displayed —
/// so the two wrappers stay, and only the duplication goes.
async fn sessions(namespace: String, t: Transport) -> ServiceResult<Sessions> {
    let raw = raw(&t).await?;
    let session = namespaced(&namespace, &t).await?;
    Ok(Sessions {
        raw,
        session,
        namespace,
    })
}

/// A session that never opened because the transport would not come up —
/// `zenkey_fleet::OpenFailure::Transport`, kept recognisable through the
/// `ServiceError` it travels in.
///
/// The fork is #196's, which zenctl spends the same way: a transport failure
/// leaves what is on disk answerable, so `--contracts` still load
/// ([`never_opened`] is what the `SessionsOpened(Err)` handler asks). A
/// `Config` failure — a file the user named that does not parse, an endpoint
/// that is not one, a namespace the session refuses — is the user's to fix,
/// and answering past it would hide it. Since the session became a client
/// (#501) this is what a dead router *is*.
///
/// Transparent: `Display` and `source()` are the engine error's own, so the
/// link line reads exactly as it did.
#[derive(Debug)]
pub struct NeverOpened(pub zenkey_fleet::Error);

impl std::fmt::Display for NeverOpened {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl std::error::Error for NeverOpened {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        std::error::Error::source(&self.0)
    }
}

/// Did this open fail on the transport — the half that leaves the disk
/// answerable — rather than on the user's own config?
pub fn never_opened(e: &ServiceError) -> bool {
    e.inner().chain().any(|c| c.is::<NeverOpened>())
}

/// Open both sessions at launch, or after a reconnect.
pub fn open(namespace: String, t: Transport) -> Task<Message> {
    Task::perform(sessions(namespace, t), |r| {
        Message::Bus(BusMsg::SessionsOpened(r))
    })
}

/// Open both sessions for a context the user switched to.
///
/// Lands on the Connect pane rather than the bus, because that is where its
/// failure is shown — the rule #176 settled every placement with.
pub fn reopen(namespace: String, t: Transport) -> Task<Message> {
    Task::perform(sessions(namespace, t), |r| {
        Message::Pane(crate::message::PaneMsg::Context(
            crate::view::contexts::ContextMsg::Switched(r),
        ))
    })
}

/// Open the session in a namespace the picker moved to (#614, FL6a): the
/// session in no namespace is the same bus and is kept. The landing names
/// the namespace, so one the window has since moved past is dropped.
pub fn open_namespaced(namespace: String, t: Transport) -> Task<Message> {
    Task::perform(
        async move {
            let r = namespaced(&namespace, &t).await;
            (namespace, r)
        },
        |(namespace, r)| Message::Bus(BusMsg::NamespacedOpened(namespace, r)),
    )
}

/// Which deployment namespaces hold zk2 instance tokens on this bus
/// (`zenkey_fleet::namespace_listing`), read on the session in **no**
/// namespace — through a namespaced one every namespace but its own would
/// be invisible.
///
/// Deliberately never filtered by the namespace already selected: it exists
/// to answer "what could I point at?", and filtering by the answer would make
/// it useless (the same rule `zenctl namespace list` states).
pub fn list_namespaces(raw: &zenoh::Session, timeout: Duration) -> Task<Message> {
    let raw = raw.clone();
    Task::perform(
        async move {
            zenkey_fleet::namespace_listing(&raw, timeout)
                .await
                .map(std::sync::Arc::new)
                .map_err(ServiceError::of)
        },
        |r| Message::Bus(BusMsg::NamespacesListed(r)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn transport(connect: &str) -> Transport {
        Transport {
            zenoh_config: None,
            connect: vec![connect.into()],
            listen: vec![],
            scouting: None,
        }
    }

    /// #501/#503: the session is a client, so a router that does not answer
    /// fails the open — promptly, as a link error the status strip renders —
    /// where the old peer session opened onto nothing and drew an empty tree.
    /// Port 1 on loopback: privileged, so nothing a test run holds.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_dead_router_is_a_link_error_not_a_hang() {
        let opened = tokio::time::timeout(
            Duration::from_secs(5),
            sessions(String::new(), transport("tcp/127.0.0.1:1")),
        )
        .await
        .expect("a dead router answers at once — never a hang");
        let Err(e) = opened else {
            panic!("a session opened against an endpoint nothing answers")
        };
        assert!(e.to_string().contains("tcp/127.0.0.1:1"), "{e}");
        assert!(
            never_opened(&e),
            "the transport half: the contracts still load"
        );
    }

    /// #503: an endpoint that does not parse is refused by name, not dropped
    /// into a session that reaches nothing.
    #[tokio::test]
    async fn a_malformed_endpoint_is_a_link_error_by_name() {
        let Err(e) = sessions(String::new(), transport("127.0.0.1:7449")).await else {
            panic!("a session opened on an endpoint that does not parse")
        };
        assert!(e.to_string().contains("tcp/127.0.0.1:7449"), "{e}");
        assert!(
            !never_opened(&e),
            "the user's half: nothing answers past it"
        );
    }
}

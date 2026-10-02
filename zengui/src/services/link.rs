//! Opening a session, and what the app asks the moment one exists (#175).

use std::time::Duration;

use iced::Task;

use crate::message::{BusMsg, Message};
use crate::services::{ServiceError, ServiceResult};

/// What the four connection settings name.
///
/// The launch open and the reconnect built this same call from the same four
/// fields, ten lines apart in different functions. They land on *different*
/// messages — the launch on `BusMsg::SessionOpened`, the reconnect on the
/// Connect pane's own `Switched`, because that is where a failed context switch
/// is displayed — so the two wrappers stay, and only the duplication goes.
async fn session(
    zenoh_config: Option<std::path::PathBuf>,
    connect: Vec<String>,
    listen: Vec<String>,
    scouting: Option<bool>,
) -> ServiceResult<zenoh::Session> {
    zenkey_fleet::open_reporting(zenoh_config.as_deref(), &connect, &listen, scouting)
        .await
        .map_err(|f| match f {
            zenkey_fleet::OpenFailure::Config(e) => ServiceError::of(e),
            zenkey_fleet::OpenFailure::Transport(e) => {
                ServiceError::of(anyhow::Error::new(NeverOpened(e)))
            }
        })
}

/// A session that never opened because the transport would not come up —
/// `zenkey_fleet::OpenFailure::Transport`, kept recognisable through the
/// `ServiceError` it travels in.
///
/// The fork is #196's, which zenctl spends the same way: a transport failure
/// leaves what is on disk answerable, so `--registry` dirs still load
/// ([`never_opened`] is what the `SessionOpened(Err)` handler asks). A
/// `Config` failure — a file the user named that does not parse, an endpoint
/// that is not one — is the user's to fix, and answering past it would hide
/// it. Since the session became a client (#501) this is what a dead router
/// *is*; the old peer session opened onto nothing and the dirs loaded by
/// accident.
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

/// Open a session at launch, or after a reconnect.
pub fn open(
    zenoh_config: Option<std::path::PathBuf>,
    connect: Vec<String>,
    listen: Vec<String>,
    scouting: Option<bool>,
) -> Task<Message> {
    Task::perform(session(zenoh_config, connect, listen, scouting), |r| {
        Message::Bus(BusMsg::SessionOpened(r))
    })
}

/// Open a session for a context the user switched to.
///
/// Lands on the Connect pane rather than the bus, because that is where its
/// failure is shown — the rule #176 settled every placement with.
pub fn reopen(
    zenoh_config: Option<std::path::PathBuf>,
    connect: Vec<String>,
    listen: Vec<String>,
    scouting: Option<bool>,
) -> Task<Message> {
    Task::perform(session(zenoh_config, connect, listen, scouting), |r| {
        Message::Pane(crate::message::PaneMsg::Context(
            crate::view::contexts::ContextMsg::Switched(r),
        ))
    })
}

/// Which deployment bases are actually in use on this bus.
///
/// Deliberately never filtered by the base already selected: it exists to
/// answer "what could I point at?", and filtering by the answer would make it
/// useless (the same rule `zenctl base list` states).
pub fn discover_bases(session: &zenoh::Session, timeout: Duration) -> Task<Message> {
    let session = session.clone();
    Task::perform(
        async move {
            zenkey_fleet::discover_bases(&session, timeout)
                .await
                .map_err(ServiceError::of)
        },
        |r| Message::Bus(BusMsg::BasesDiscovered(r)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// #501/#503: the session is a client, so a router that does not answer
    /// fails the open — promptly, as a link error the status strip renders —
    /// where the old peer session opened onto nothing and drew an empty tree.
    /// Port 1 on loopback: privileged, so nothing a test run holds.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_dead_router_is_a_link_error_not_a_hang() {
        let opened = tokio::time::timeout(
            Duration::from_secs(5),
            session(None, vec!["tcp/127.0.0.1:1".into()], vec![], None),
        )
        .await
        .expect("a dead router answers at once — never a hang");
        let Err(e) = opened else {
            panic!("a session opened against an endpoint nothing answers")
        };
        assert!(e.to_string().contains("tcp/127.0.0.1:1"), "{e}");
        assert!(never_opened(&e), "the transport half: the dirs still load");
    }

    /// #503: an endpoint that does not parse is refused by name, not dropped
    /// into a session that reaches nothing.
    #[tokio::test]
    async fn a_malformed_endpoint_is_a_link_error_by_name() {
        let Err(e) = session(None, vec!["127.0.0.1:7449".into()], vec![], None).await else {
            panic!("a session opened on an endpoint that does not parse")
        };
        assert!(e.to_string().contains("tcp/127.0.0.1:7449"), "{e}");
        assert!(
            !never_opened(&e),
            "the user's half: nothing answers past it"
        );
    }
}

//! Opening a session, and what the app asks the moment one exists (#175).

use std::time::Duration;

use iced::Task;

use crate::message::{BusMsg, Message};

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
) -> Result<zenoh::Session, String> {
    zenkey_fleet::open_with_config(zenoh_config.as_deref(), &connect, &listen, scouting)
        .await
        .map_err(|e| e.to_string())
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
                .map_err(|e| e.to_string())
        },
        |r| Message::Bus(BusMsg::BasesDiscovered(r)),
    )
}

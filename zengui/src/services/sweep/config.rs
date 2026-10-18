//! One service's `config.v1`, read (#768, CD): the engine's reading,
//! `zenkey_fleet::run_config`, as `zenctl config get` makes it — presence
//! and the descriptor (the premise of every §5 row), an optional window of
//! listening to the view, the status and the change events, then one S4
//! GET of the owner's `config.v1/state/**`, and §5's answers judged from
//! what was read (`spec/profiles/config/v1.md` §5).

use std::sync::Arc;
use std::time::SystemTime;

use iced::Task;
use zenkey_fleet::ConfigSpec;
use zenkey_model::grammar::Addr;

use crate::configure::{ReadLanding, ReadWhy};
use crate::message::{Message, PaneMsg};
use crate::view::configure::ConfigMsg;

/// One reading of `addr`, on the session in the namespace, landing with
/// the address and the namespace it ran in (the staleness guard, #109).
/// Never an error: a read that could not be made is the report's
/// `unobservable`, in the engine's words.
pub fn config(
    session: zenoh::Session,
    namespace: String,
    addr: Addr,
    spec: ConfigSpec,
    why: ReadWhy,
) -> Task<Message> {
    Task::perform(
        async move {
            let report = zenkey_fleet::run_config(&session, &namespace, addr.clone(), spec).await;
            ReadLanding {
                addr,
                namespace,
                why,
                report: Arc::new(report),
                at: SystemTime::now(),
            }
        },
        |l| Message::Pane(PaneMsg::Config(ConfigMsg::Read(l))),
    )
}

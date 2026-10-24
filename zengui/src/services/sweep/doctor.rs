//! The doctor run.

use std::sync::Arc;

use iced::Task;

use crate::message::{Message, PaneMsg};
use crate::view::doctor::DoctorMsg;

/// The doctor run (#109; zk2's since #614, FL9): the engine's `run_doctor`
/// over the window's two sessions, as `zenctl doctor` runs it — the
/// deployment read through the session **in** the namespace, the routers'
/// admin space and the presence domain through the one in **none**
/// (decided 2026-10-08).
///
/// `store` is the window's own contract store: a verified revision is
/// content-addressed and kept across runs, and the engine forgets its
/// absences at the start of each, so a revision no holder served is asked
/// about again — the same store the lens resolves through, whose answers
/// then move with it.
///
/// It cannot fail: an input it could not read is that check's
/// `Unobservable`, and a scope with no zk2 token in it is the report's own
/// unobservable — never a green run.
pub fn doctor(
    session: zenoh::Session,
    raw: zenoh::Session,
    namespace: String,
    store: zenkey_fleet::BundleStore,
    spec: zenkey_fleet::DoctorSpec,
) -> Task<Message> {
    Task::perform(
        async move {
            let bus = zenkey_fleet::DoctorBus {
                session,
                raw,
                namespace: namespace.clone(),
                // The doctor follows no presence live (#830).
                watch: None,
            };
            let report = zenkey_fleet::run_doctor(&bus, &store, &spec).await;
            crate::doctor::DoctorRun {
                report: Arc::new(report),
                namespace,
            }
        },
        |run| Message::Pane(PaneMsg::Doctor(DoctorMsg::Done(run))),
    )
}

//! The doctor run.

use std::sync::Arc;

use iced::Task;
use zenkey_fleet_v1::SliceSet;

use crate::message::{Message, PaneMsg};
use crate::services::ServiceError;
use crate::view::doctor::DoctorMsg;

/// The doctor run (#109).
///
/// `dirs` rather than the app's resolved slice set, deliberately: the doctor
/// diffs local declarations against served ones, and handing it the union
/// would diff the bus against itself.
pub fn doctor(
    session: zenoh::Session,
    base: String,
    dirs: Vec<std::path::PathBuf>,
    spec: zenkey_fleet_v1::DoctorSpec,
) -> Task<Message> {
    Task::perform(
        async move {
            // `None` when no dirs were given: "no registry loaded" is a
            // different claim from "a registry that declares nothing", and
            // only the engine's `Option` can say which happened.
            let locals = match dirs.is_empty() {
                true => None,
                false => match SliceSet::from_dirs(&dirs) {
                    Ok(set) => Some(set),
                    Err(e) => {
                        return Err(ServiceError::of(
                            anyhow::Error::new(e).context("failed to read the registry dirs"),
                        ));
                    }
                },
            };
            zenkey_fleet_v1::run_doctor(
                &zenkey_fleet_v1::Fleet::new(&session, &base),
                locals.as_ref(),
                &spec,
            )
            .await
            .map(|r| crate::doctor::DoctorRun {
                report: Arc::new(r),
                base,
            })
            .map_err(ServiceError::of)
        },
        |out| Message::Pane(PaneMsg::Doctor(DoctorMsg::Done(out))),
    )
}

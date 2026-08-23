//! The Inspector's Fields section (#223): one bounded observation window on
//! a slot's key, run on demand.
//!
//! Takes `&mut SubjectSlot` because the section follows its slot's subject
//! (#257) — its report is evidence about one key's window, dropped on
//! re-selection (the Detail/History precedent: a window onto one subject,
//! not a pane with a life of its own). The landing routes back by the slot's
//! id, so a pinned Inspector's report can never land in the docked one.

use iced::Task;

use crate::message::Message;
use crate::services;
use crate::state::Deployment;
use crate::state::subject::SubjectSlot;
use crate::view::fields::FieldsMsg;

pub(crate) fn update(slot: &mut SubjectSlot, dep: &Deployment, msg: FieldsMsg) -> Task<Message> {
    match msg {
        FieldsMsg::WindowChanged(t) => {
            slot.fields.window = t;
            Task::none()
        }
        FieldsMsg::Run => {
            if slot.fields.in_flight {
                return Task::none();
            }
            let Some(key) = slot.current.key().filter(|k| !k.contains('{')) else {
                slot.fields.report = Some(Err(
                    "the subject is not a concrete key — a field window watches \
                     exactly one key"
                        .into(),
                ));
                return Task::none();
            };
            let Some(window) = slot.fields.window_secs() else {
                return Task::none();
            };
            let Some(session) = dep.session.clone() else {
                slot.fields.report = Some(Err("no session — connect first".into()));
                return Task::none();
            };
            let Some(store) = dep.schema_store.clone() else {
                slot.fields.report = Some(Err("no schema store yet — connecting".into()));
                return Task::none();
            };
            slot.fields.in_flight = true;
            slot.fields.asked = Some(key.to_string());
            services::value::field(
                slot.id,
                session,
                dep.base().to_string(),
                dep.slices.clone(),
                store,
                key.to_string(),
                std::time::Duration::from_secs(window),
            )
        }
        FieldsMsg::Done(outcome) => {
            slot.fields.in_flight = false;
            // Staleness guard: the window was about the key it was asked on.
            // A report for a superseded subject is dropped — the section was
            // already reset by the re-selection, and landing it would title
            // one key's table with another's numbers.
            if slot.fields.asked.as_deref() != slot.current.key() {
                return Task::none();
            }
            if let Ok(report) = &outcome {
                slot.fields.sparks = crate::view::fields::sparks_for(report, slot.history.as_ref());
            }
            slot.fields.report = Some(outcome);
            Task::none()
        }
    }
}

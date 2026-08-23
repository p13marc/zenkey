//! The Inspector's Fields section (#223): one bounded observation window on
//! the subject key, run on demand.
//!
//! Takes `&mut SubjectState` because the section follows the subject — its
//! report is evidence about one key's window, dropped on re-selection (the
//! Detail/History precedent: a window onto the selected key, not a pane
//! with a life of its own).

use iced::Task;

use crate::message::Message;
use crate::services;
use crate::state::{Deployment, SubjectState};
use crate::view::fields::FieldsMsg;

pub(crate) fn update(sub: &mut SubjectState, dep: &Deployment, msg: FieldsMsg) -> Task<Message> {
    match msg {
        FieldsMsg::WindowChanged(t) => {
            sub.fields.window = t;
            Task::none()
        }
        FieldsMsg::Run => {
            if sub.fields.in_flight {
                return Task::none();
            }
            let Some(key) = sub.current.key().filter(|k| !k.contains('{')) else {
                sub.fields.report = Some(Err(
                    "the subject is not a concrete key — a field window watches \
                     exactly one key"
                        .into(),
                ));
                return Task::none();
            };
            let Some(window) = sub.fields.window_secs() else {
                return Task::none();
            };
            let Some(session) = dep.session.clone() else {
                sub.fields.report = Some(Err("no session — connect first".into()));
                return Task::none();
            };
            let Some(store) = dep.schema_store.clone() else {
                sub.fields.report = Some(Err("no schema store yet — connecting".into()));
                return Task::none();
            };
            sub.fields.in_flight = true;
            sub.fields.asked = Some(key.to_string());
            services::value::field(
                session,
                dep.base().to_string(),
                dep.slices.clone(),
                store,
                key.to_string(),
                std::time::Duration::from_secs(window),
            )
        }
        FieldsMsg::Done(outcome) => {
            sub.fields.in_flight = false;
            // Staleness guard: the window was about the key it was asked on.
            // A report for a superseded subject is dropped — the section was
            // already reset by the re-selection, and landing it would title
            // one key's table with another's numbers.
            if sub.fields.asked.as_deref() != sub.current.key() {
                return Task::none();
            }
            if let Ok(report) = &outcome {
                sub.fields.sparks = crate::view::fields::sparks_for(report, sub.history.as_ref());
            }
            sub.fields.report = Some(outcome);
            Task::none()
        }
    }
}

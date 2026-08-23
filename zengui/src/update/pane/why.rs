//! The Inspector's Why section (#214): the silence ladder, run on demand at
//! its frugal default.
//!
//! Takes `&mut SubjectState` because the ladder follows the subject. The run
//! also reveals the Inspector dock — the palette speaks the same `Run`, and
//! a ladder rendered into a closed dock is a control that did nothing.

use iced::Task;

use crate::message::{Message, RightPane, WorkspaceMsg};
use crate::services;
use crate::state::{Deployment, SubjectState};
use crate::view::why::WhyMsg;

pub(crate) fn update(sub: &mut SubjectState, dep: &Deployment, msg: WhyMsg) -> Task<Message> {
    match msg {
        WhyMsg::Run => {
            if sub.why.in_flight {
                return Task::none();
            }
            let Some(key) = sub.current.key() else {
                sub.why.report = Some(Err(
                    "the subject is not a concrete key — the ladder asks about \
                     exactly one key"
                        .into(),
                ));
                return Task::none();
            };
            let Some(session) = dep.session.clone() else {
                sub.why.report = Some(Err("no session — connect first".into()));
                return Task::none();
            };
            sub.why.in_flight = true;
            sub.why.asked = Some(key.to_string());
            // The frugal default, always (RFC v1.18): control-plane sweeps
            // and one bounded GET — no subscriber, so `wire-heard` reads
            // "not asked" and the section says so.
            let run = services::value::why(
                session,
                dep.base().to_string(),
                dep.slices.clone(),
                key.to_string(),
                dep.timeout(),
            );
            // Reveal the Inspector: the palette raises this same message,
            // and the ladder lands in the Inspector's Why section.
            Task::batch([
                Task::done(Message::Workspace(WorkspaceMsg::PaneSelected(
                    RightPane::Inspector,
                ))),
                run,
            ])
        }
        WhyMsg::Done(outcome) => {
            sub.why.in_flight = false;
            // Staleness guard: a ladder for a superseded subject is dropped.
            if sub.why.asked.as_deref() != sub.current.key() {
                return Task::none();
            }
            sub.why.report = Some(outcome);
            Task::none()
        }
    }
}

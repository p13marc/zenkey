//! The Inspector's Why section (#214): the silence ladder, run on demand at
//! its frugal default.
//!
//! Takes `&mut SubjectSlot` because the ladder follows its slot's subject
//! (#257). A *follow-slot* run also reveals the Inspector dock — the palette
//! speaks the same `Run`, and a ladder rendered into a closed dock is a
//! control that did nothing. A pinned slot's run reveals nothing: its ladder
//! lands in the window whose button was just pressed.

use iced::Task;

use crate::message::{Message, RightPane, WorkspaceMsg};
use crate::services;
use crate::state::Deployment;
use crate::state::subject::SubjectSlot;
use crate::view::why::WhyMsg;

pub(crate) fn update(slot: &mut SubjectSlot, dep: &Deployment, msg: WhyMsg) -> Task<Message> {
    match msg {
        WhyMsg::Run => {
            if slot.why.in_flight {
                return Task::none();
            }
            let Some(key) = slot.current.key() else {
                slot.why.report = Some(Err(
                    "the subject is not a concrete key — the ladder asks about \
                     exactly one key"
                        .into(),
                ));
                return Task::none();
            };
            let Some(session) = dep.session.clone() else {
                slot.why.report = Some(Err("no session — connect first".into()));
                return Task::none();
            };
            slot.why.in_flight = true;
            slot.why.asked = Some(key.to_string());
            // The frugal default, always (RFC v1.18): control-plane sweeps
            // and one bounded GET — no subscriber, so `wire-heard` reads
            // "not asked" and the section says so.
            let run = services::value::why(
                slot.id,
                session,
                dep.base().to_string(),
                dep.slices.clone(),
                key.to_string(),
                dep.timeout(),
            );
            if slot.id.is_pin() {
                return run;
            }
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
            slot.why.in_flight = false;
            // Staleness guard: a ladder for a superseded subject is dropped.
            if slot.why.asked.as_deref() != slot.current.key() {
                return Task::none();
            }
            slot.why.report = Some(outcome);
            Task::none()
        }
    }
}

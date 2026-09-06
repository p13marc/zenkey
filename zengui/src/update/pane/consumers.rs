//! The Inspector's Consumers section (#224): one admin sweep per click,
//! never ambient.
//!
//! Takes `&mut SubjectSlot` because the sweep follows its slot's subject
//! (#257), exactly as the Why ladder does. A *follow-slot* run reveals the
//! Inspector dock; a pinned slot's run lands in the window whose button
//! was just pressed.

use iced::Task;

use crate::message::{Message, RightPane, Subject, WorkspaceMsg};
use crate::services;
use crate::state::Deployment;
use crate::state::subject::SubjectSlot;
use crate::view::consumers::ConsumersMsg;

/// The wire target one subject resolves to: a key as itself, a subtree
/// as `<prefix>/**`. An origin has no keyexpr to relate declarations to.
fn target_of(subject: &Subject) -> Option<String> {
    match subject {
        Subject::Key(k) => Some(k.clone()),
        Subject::Prefix(p) => Some(format!("{p}/**")),
        Subject::Origin(_) | Subject::None => None,
    }
}

pub(crate) fn update(slot: &mut SubjectSlot, dep: &Deployment, msg: ConsumersMsg) -> Task<Message> {
    match msg {
        ConsumersMsg::Run => {
            if slot.consumers.in_flight {
                return Task::none();
            }
            let Some(target) = target_of(&slot.current) else {
                slot.consumers.report = Some(Err(
                    "the subject is not a key or a subtree — declarations relate to a \
                     key expression"
                        .into(),
                ));
                return Task::none();
            };
            let Some(session) = dep.session.clone() else {
                slot.consumers.report = Some(Err("no session — connect first".into()));
                return Task::none();
            };
            slot.consumers.in_flight = true;
            slot.consumers.asked = Some(target.clone());
            let run = services::sweep::consumers_of(
                slot.id,
                session,
                dep.base().to_string(),
                target,
                dep.timeout(),
            );
            if slot.id.is_pin() {
                return run;
            }
            Task::batch([
                Task::done(Message::Workspace(WorkspaceMsg::PaneSelected(
                    RightPane::Inspector,
                ))),
                run,
            ])
        }
        ConsumersMsg::Done(outcome) => {
            slot.consumers.in_flight = false;
            // Staleness guard: a sweep for a superseded subject is dropped.
            if slot.consumers.asked != target_of(&slot.current) {
                return Task::none();
            }
            slot.consumers.report = Some(outcome);
            Task::none()
        }
    }
}

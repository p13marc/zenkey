//! The Inspector's Why section (#214): zk2's silence ladder (#614, FL8b),
//! run on demand for a key or a service.
//!
//! Takes `&mut SubjectSlot` because the ladder follows its slot's subject
//! (#257). A *follow-slot* run also reveals the Inspector dock — the palette
//! speaks the same `Run`, and a ladder rendered into a closed dock is a
//! control that did nothing. A pinned slot's run reveals nothing: its ladder
//! lands in the window whose button was just pressed.

use std::sync::Arc;

use iced::Task;

use crate::message::{Message, RightPane, Subject, WorkspaceMsg};
use crate::services;
use crate::state::Deployment;
use crate::state::subject::SubjectSlot;
use crate::view::why::WhyMsg;

/// What the ladder explains for `subject`, and how its staleness guard
/// names it: a concrete key as the wire spells it, an address as a
/// service. A prefix and nothing name no one thing to explain.
fn target_of(subject: &Subject) -> Option<(zenkey_fleet::WhyTarget, String)> {
    match subject {
        Subject::Key(k) if !k.contains('{') => {
            Some((zenkey_fleet::WhyTarget::Key(k.clone()), k.clone()))
        }
        Subject::Address(a) => Some((zenkey_fleet::WhyTarget::Service(a.clone()), a.to_string())),
        _ => None,
    }
}

pub(crate) fn update(
    slot: &mut SubjectSlot,
    dep: &Deployment,
    replaying: bool,
    msg: WhyMsg,
) -> Task<Message> {
    match msg {
        WhyMsg::Run => {
            if slot.why.in_flight {
                return Task::none();
            }
            let Some((target, asked)) = target_of(&slot.current) else {
                slot.why.report = Some(Err(
                    "the subject is not a concrete key or a service — the ladder explains \
                     exactly one"
                        .into(),
                ));
                return Task::none();
            };
            // Never under `@op` (#614, FL8b): a GET there is a call, and the
            // ladder acts on nothing.
            if crate::scope::names_operation(&asked) {
                slot.why.report = Some(Err(
                    "an operation's key is not explained here — the ladder calls nothing, \
                     and the Workbench's call asks an operation"
                        .into(),
                ));
                return Task::none();
            }
            // A replay asks the bus nothing (#614, FL8a): the ladder reads
            // the live deployment, which is not the capture on screen.
            if replaying {
                slot.why.report = Some(Err(
                    "a replay asks the bus nothing — leave it to ask the live deployment".into(),
                ));
                return Task::none();
            }
            let spec = zenkey_fleet::WhySpec {
                timeout: dep.timeout(),
                window: services::value::WHY_WINDOW,
            };
            // The rungs a key alone decides need no session (`zenctl why`'s
            // order): another namespace's key, or one that is not zk2.
            let offline = zenkey_fleet::judge_why(&zenkey_fleet::WhyObservation::new(
                dep.namespace(),
                target.clone(),
                spec,
            ));
            if matches!(
                offline.stopped_at,
                Some(zenkey_fleet::report::RungId::Namespace | zenkey_fleet::report::RungId::Key)
            ) {
                slot.why.asked = Some(asked);
                slot.why.report = Some(Ok(Arc::new(offline)));
                return Task::none();
            }
            let Some(session) = dep.session.clone() else {
                slot.why.report = Some(Err("no session in the namespace — connect first".into()));
                return Task::none();
            };
            slot.why.in_flight = true;
            slot.why.asked = Some(asked);
            let run = services::value::why(
                slot.id,
                session,
                dep.store.clone(),
                dep.namespace().to_string(),
                target,
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
            if slot.why.asked != target_of(&slot.current).map(|(_, a)| a) {
                return Task::none();
            }
            slot.why.report = Some(outcome);
            Task::none()
        }
    }
}

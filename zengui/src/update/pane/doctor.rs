//! The doctor panel (#71, #109; zk2's since #614, FL9): run on demand, and
//! never invent a verdict.
//!
//! It used to write `right_pane` directly — clicking a finding lands the user
//! where the finding is about — and stopped when #180 made pane-landing a
//! `PaneSelected` message: the jump now travels the same path the palette
//! and the workbench strip use, which also restores the workbench dock if
//! the user closed it.

use iced::Task;

use crate::message::{DeploymentMsg, Message, RightPane, Subject, SubjectMsg, WorkspaceMsg};
use crate::services;
use crate::state::workspace::Verdicts;
use crate::update::Ctx;
use crate::view::doctor::DoctorMsg;

pub(crate) fn update(v: &mut Verdicts, msg: DoctorMsg, cx: Ctx) -> Task<Message> {
    match msg {
        DoctorMsg::DeepToggled(deep) => {
            v.doctor.deep = deep;
            Task::none()
        }
        DoctorMsg::ClocksSynced(synced) => {
            v.doctor.clocks_synced = synced;
            Task::none()
        }
        DoctorMsg::Reread => {
            // v1's "re-ask schemas", on zk2's inputs: presence and the
            // contracts are read again — the lens's catalog, and every
            // revision no holder served. A permission, not a sweep of the
            // data plane (#85): what it costs is one presence read and the
            // retrievals it owes. The deployment owns both, so the work is
            // its message's.
            v.doctor.rereads += 1;
            Task::done(Message::Deployment(DeploymentMsg::RereadPresence))
        }
        DoctorMsg::Run => {
            if v.doctor.in_flight {
                return Task::none();
            }
            // Two sessions, as `zenctl doctor` reads (decided 2026-10-08):
            // the deployment in its namespace, the admin space in none.
            let (Some(session), Some(raw)) = (cx.dep.session.clone(), cx.dep.raw.clone()) else {
                v.doctor.error = Some("no session — connect first".into());
                return Task::none();
            };
            v.doctor.in_flight = true;
            v.doctor.error = None;
            let mut spec = zenkey_fleet::DoctorSpec::new(cx.dep.timeout());
            spec.deep = v.doctor.deep;
            spec.clocks_synced = v.doctor.clocks_synced;
            services::sweep::doctor(
                session,
                raw,
                cx.dep.namespace().to_string(),
                cx.dep.store.clone(),
                spec,
            )
        }
        DoctorMsg::Done(run) => {
            let namespace = cx.dep.namespace().to_string();
            v.doctor.finish(run, &namespace);
            Task::none()
        }
        DoctorMsg::FindingClicked(index) => {
            let Some(finding) = v
                .doctor
                .current
                .as_deref()
                .and_then(|r| r.findings().nth(index))
            else {
                return Task::none();
            };
            match crate::doctor::finding_target(finding, cx.dep.namespace()) {
                // A concrete key: select in the tree (with the usual
                // on-demand fetch); the dock stays on Doctor so the finding
                // list is not lost.
                Some(crate::doctor::Target::Key(key)) => Task::batch([
                    Task::done(Message::Workspace(WorkspaceMsg::Reveal(key.clone()))),
                    Task::done(Message::Subject(SubjectMsg::Select(Subject::Key(key)))),
                ]),
                // An address: point the workspace at it and land on the
                // nodes tool, which is where an address is legible.
                Some(crate::doctor::Target::Address(addr)) => Task::done(Message::Workspace(
                    WorkspaceMsg::PaneSelected(RightPane::Nodes),
                ))
                .chain(Task::done(Message::Subject(SubjectMsg::Select(
                    Subject::Address(addr),
                )))),
                None => Task::none(),
            }
        }
    }
}

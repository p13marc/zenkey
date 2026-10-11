//! The doctor panel (#71, #109): run on demand, and never invent a verdict.
//!
//! It used to write `right_pane` directly — clicking a finding lands the user
//! where the finding is about — and stopped when #180 made pane-landing a
//! `PaneSelected` message: the jump now travels the same path the palette
//! and the workbench strip use, which also restores the workbench dock if
//! the user closed it.

use iced::Task;

use crate::message::{Message, RightPane, Subject, SubjectMsg, WorkspaceMsg};
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
        DoctorMsg::ListenChanged(t) => {
            v.doctor.listen = t;
            Task::none()
        }
        DoctorMsg::ReaskSchemas => {
            // Local and immediate: the store simply forgets, and the next
            // decode that needs a schema asks the bus. Nothing is fetched
            // here — an explorer retrieves nothing it was not asked for
            // (#85), and "re-ask" is a permission, not a sweep.
            if let Some(store) = cx.dep.schema_store.as_ref() {
                store.forget_all();
                v.doctor.schemas_forgotten += 1;
                // The cached payload verdicts were judged under the schemas
                // just forgotten (#164) — the third door a schema hash can
                // change through, and like the other two it clears.
                v.payloads.clear();
            }
            Task::none()
        }
        DoctorMsg::Run => {
            if v.doctor.in_flight {
                return Task::none();
            }
            let Some(session) = cx.dep.raw.clone() else {
                v.doctor.error = Some("no session — connect first".into());
                return Task::none();
            };
            v.doctor.in_flight = true;
            v.doctor.error = None;
            let base = cx.dep.namespace().to_string();
            let timeout = cx.dep.timeout();
            let deep = v.doctor.deep;
            let listen = v.doctor.listen_window();
            // No registry dirs since #614 (FL6a): the v1 doctor runs with no
            // locals ("no registry loaded", never "a registry that declares
            // nothing") until FL9 runs zk2's against the contracts.
            let dirs = Vec::new();
            services::sweep::doctor(
                session,
                base,
                dirs,
                zenkey_fleet_v1::DoctorSpec {
                    deep,
                    sample: None,
                    timeout,
                    listen,
                },
            )
        }
        DoctorMsg::Done(outcome) => {
            let base = cx.dep.namespace().to_string();
            v.doctor.finish(outcome, &base);
            Task::none()
        }
        DoctorMsg::FindingClicked(index) => {
            let Some(finding) = v
                .doctor
                .current
                .as_deref()
                .and_then(|r| r.findings.get(index))
            else {
                return Task::none();
            };
            match crate::doctor::finding_target(finding, cx.dep.namespace()) {
                // A concrete key: select in the tree (with the usual
                // on-demand fetch); the right pane stays on Doctor so
                // the finding list is not lost.
                Some(crate::doctor::Target::Key(key)) => Task::batch([
                    Task::done(Message::Workspace(WorkspaceMsg::Reveal(key.clone()))),
                    Task::done(Message::Subject(SubjectMsg::Select(Subject::Key(key)))),
                ]),
                // An origin subject: point the workspace at it and land on
                // the nodes tool, which is where an origin is legible.
                Some(crate::doctor::Target::Node(origin)) => Task::done(Message::Workspace(
                    WorkspaceMsg::PaneSelected(RightPane::Nodes),
                ))
                .chain(Task::done(Message::Subject(SubjectMsg::Select(
                    Subject::Origin(origin),
                )))),
                None => Task::none(),
            }
        }
    }
}

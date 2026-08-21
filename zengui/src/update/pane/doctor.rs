//! The doctor panel (#71, #109): run on demand, and never invent a verdict.
//!
//! It writes `right_pane` — clicking a finding lands the user where the
//! finding is about — so the tab strip is in its signature. That is the whole
//! reason the parameter exists, and the reason it is a `&mut RightPane` rather
//! than the whole workspace.

use iced::Task;

use crate::message::{Message, PaneMsg, RightPane, SubjectMsg, WorkspaceMsg};
use crate::services;
use crate::state::workspace::Verdicts;
use crate::update::Ctx;
use crate::view;
use crate::view::doctor::DoctorMsg;

pub(crate) fn update(
    v: &mut Verdicts,
    pane: &mut RightPane,
    msg: DoctorMsg,
    cx: Ctx,
) -> Task<Message> {
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
            }
            Task::none()
        }
        DoctorMsg::Run => {
            if v.doctor.in_flight {
                return Task::none();
            }
            let Some(session) = cx.dep.session.clone() else {
                v.doctor.error = Some("no session — connect first".into());
                return Task::none();
            };
            v.doctor.in_flight = true;
            v.doctor.error = None;
            let base = cx.dep.base().to_string();
            let timeout = cx.dep.timeout();
            let deep = v.doctor.deep;
            let listen = v.doctor.listen_window();
            // Locals come from the registry DIRS only — never the union
            // set, which would diff the bus against itself.
            let dirs = cx.dep.settings.registry.clone();
            services::sweep::doctor(
                session,
                base,
                dirs,
                zenkey_fleet::DoctorSpec {
                    deep,
                    sample: None,
                    timeout,
                    listen,
                },
            )
        }
        DoctorMsg::Done(outcome) => {
            let base = cx.dep.base().to_string();
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
            match crate::doctor::finding_target(finding, cx.dep.base()) {
                // A concrete key: select in the tree (with the usual
                // on-demand fetch); the right pane stays on Doctor so
                // the finding list is not lost.
                Some(crate::doctor::Target::Key(key)) => Task::batch([
                    Task::done(Message::Workspace(WorkspaceMsg::Reveal(key.clone()))),
                    Task::done(Message::Subject(SubjectMsg::SelectKey(Some(key)))),
                ]),
                // An origin/producer subject: land on the nodes pane.
                Some(crate::doctor::Target::Node(origin)) => {
                    *pane = RightPane::Nodes;
                    Task::done(Message::Pane(PaneMsg::Nodes(
                        view::nodes::NodesMsg::Selected(origin),
                    )))
                }
                None => Task::none(),
            }
        }
    }
}

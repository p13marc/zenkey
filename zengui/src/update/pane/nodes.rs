//! The node dashboard (#61; zk2's since #614, FL9): who is up, and one
//! address's health on demand.
//!
//! Since #181 the pane does not own a selection. Clicking a card raises
//! `SubjectMsg::Select(Subject::Address(..))`, and the health reading is
//! issued by the subject handler with every other consequence of pointing
//! the window somewhere — so what is left here is the async landing, the
//! clock toggle, the explicit re-read and the click-through. `&mut Verdicts`
//! is for the health *verdict*, a claim about this deployment, dropped with
//! it.

use iced::Task;

use crate::message::{Message, Subject, SubjectMsg, WorkspaceMsg};
use crate::scope;
use crate::services;
use crate::state::workspace::Verdicts;
use crate::update::Ctx;
use crate::view::nodes::NodesMsg;

pub(crate) fn update(v: &mut Verdicts, msg: NodesMsg, cx: Ctx) -> Task<Message> {
    match msg {
        NodesMsg::HealthRead(addr, ran_against, report) => {
            // Stale guard: only the selected address's reading lands — and
            // only when it ran in the namespace this window shows (#109). An
            // address is namespace-relative, so the same `robot1/nav` in the
            // deployment the window left would pass the selection-only check.
            if cx.sub.follow.current.address() == Some(&addr) && ran_against == cx.dep.namespace() {
                v.nodes.health_read(addr, report);
            }
            Task::none()
        }
        NodesMsg::AskHealth => ask(v, cx),
        NodesMsg::ClocksSynced(synced) => {
            v.nodes.clocks_synced = synced;
            // The word changes what a reading can judge: the address in
            // view is read again under it, never left showing an answer
            // judged on the other ground.
            ask(v, cx)
        }
        NodesMsg::ShowInTree(addr) => {
            let path = scope::address_display_path(cx.dep.namespace(), &addr);
            // Reveal, then select WITHOUT fetching: a subtree prefix is
            // not a concrete key, and asking for its value would be a GET
            // no owner answers (#85).
            Task::batch([
                Task::done(Message::Workspace(WorkspaceMsg::Reveal(path.clone()))),
                Task::done(Message::Subject(SubjectMsg::Select(Subject::Prefix(path)))),
            ])
        }
    }
}

/// Read the selected address's health, when the window points at one.
fn ask(v: &mut Verdicts, cx: Ctx) -> Task<Message> {
    let Some(addr) = cx.sub.follow.current.address().cloned() else {
        return Task::none();
    };
    health(v, cx.dep, addr)
}

/// One `health.v1` reading of `addr` on the session in the namespace,
/// marked as asked — the subject handler's call too, on selection.
pub(crate) fn health(
    v: &mut Verdicts,
    dep: &crate::state::Deployment,
    addr: zenkey_model::grammar::Addr,
) -> Task<Message> {
    let Some(session) = dep.session.clone() else {
        return Task::none();
    };
    v.nodes.asking(addr.clone());
    services::sweep::health(
        session,
        dep.namespace().to_string(),
        addr,
        v.nodes.clocks_synced,
        dep.timeout(),
    )
}

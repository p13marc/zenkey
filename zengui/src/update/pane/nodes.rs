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
        NodesMsg::RedundancyRead(addr, ran_against, report) => {
            // The health reading's guard (#109).
            if cx.sub.follow.current.address() == Some(&addr) && ran_against == cx.dep.namespace() {
                v.nodes.redundancy_read(addr, report);
            }
            Task::none()
        }
        NodesMsg::AskHealth => ask(v, cx),
        NodesMsg::AskRedundancy => match cx.sub.follow.current.address().cloned() {
            Some(addr) => redundancy(v, cx.dep, addr),
            None => Task::none(),
        },
        NodesMsg::RedundancySettled(addr, due) => {
            // Still the selection, and the latest one's settle (#862's
            // review); the landing keeps its own guard.
            if cx.sub.follow.current.address() == Some(&addr) && v.nodes.settled(&addr, due) {
                redundancy(v, cx.dep, addr)
            } else {
                Task::none()
            }
        }
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

/// How long a selection settles before its `redundancy.v1` reading is
/// asked: a burst of selections asks one, the last's (#862's review).
pub(crate) const REDUNDANCY_SETTLE: std::time::Duration = std::time::Duration::from_millis(300);

/// `addr` selected (the subject handler's call): its `redundancy.v1`
/// reading owed, asked [`REDUNDANCY_SETTLE`] later if the selection is
/// still it. Nothing is owed without a session in the namespace to ask
/// through.
pub(crate) fn redundancy_soon(
    v: &mut Verdicts,
    dep: &crate::state::Deployment,
    addr: zenkey_model::grammar::Addr,
) -> Task<Message> {
    if dep.session.is_none() {
        return Task::none();
    }
    let due = v.nodes.due_redundancy(addr.clone());
    // The timer is made inside the task, on iced's tokio executor, never
    // here: `update` runs outside any runtime, and `tokio::time::sleep`
    // made outside one panics (review on PR #867). A test that drives
    // `update` and never runs the task makes none.
    Task::perform(
        async { tokio::time::sleep(REDUNDANCY_SETTLE).await },
        move |()| {
            Message::Pane(crate::message::PaneMsg::Nodes(NodesMsg::RedundancySettled(
                addr, due,
            )))
        },
    )
}

/// One `redundancy.v1` reading of `addr` on the session in the namespace,
/// marked as asked — the settle's call, and "read again"'s (#841, RC).
pub(crate) fn redundancy(
    v: &mut Verdicts,
    dep: &crate::state::Deployment,
    addr: zenkey_model::grammar::Addr,
) -> Task<Message> {
    let Some(session) = dep.session.clone() else {
        return Task::none();
    };
    v.nodes.asking_redundancy(addr.clone());
    services::sweep::redundancy(
        session,
        dep.namespace().to_string(),
        addr,
        dep.store.clone(),
        dep.timeout(),
    )
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

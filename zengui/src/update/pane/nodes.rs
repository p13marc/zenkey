//! The node dashboard (#61): who is alive, and one detail on demand.
//!
//! `&mut Verdicts` rather than a narrower pair, because the roster it reads
//! and the selection it writes live in the same group for the same reason —
//! they are all claims about *this* fleet, and they are dropped together.

use iced::Task;

use crate::message::{Message, SubjectMsg, WorkspaceMsg};
use crate::scope;
use crate::services;
use crate::state::workspace::Verdicts;
use crate::update::Ctx;
use crate::view::nodes::{DetailState, NodesMsg};

pub(crate) fn update(v: &mut Verdicts, msg: NodesMsg, cx: Ctx) -> Task<Message> {
    match msg {
        NodesMsg::Selected(origin) => {
            if v.node_selected.as_deref() == Some(origin.as_str()) {
                // Re-click deselects (and drops the detail state).
                v.node_selected = None;
                v.node_detail = DetailState::NotAsked;
                return Task::none();
            }
            v.node_selected = Some(origin.clone());
            v.node_detail = DetailState::Loading(origin.clone());
            let Some(session) = cx.dep.session.clone() else {
                return Task::none();
            };
            let base = cx.dep.base().to_string();
            let timeout = cx.dep.timeout();
            // The pane's one data-plane cost: a one-shot node_info on
            // selection (laziness ground rule, #84/#85).
            services::sweep::node_info(session, base, origin, timeout)
        }
        NodesMsg::InfoLoaded(origin, ran_against, outcome) => {
            // Stale guard: only the currently selected origin's info lands
            // — and only when it ran against the base this window shows.
            // Origin ids are base-independent (#109): re-selecting the
            // same host after a base switch would otherwise admit the old
            // deployment's reply through the selection-only check.
            if v.node_selected.as_deref() == Some(origin.as_str()) && ran_against == cx.dep.base() {
                v.node_detail = DetailState::Loaded(origin, outcome);
            }
            Task::none()
        }
        NodesMsg::ShowInTree(origin) => {
            let path = scope::origin_display_path(cx.dep.base(), &origin);
            // Reveal, then select WITHOUT fetching: a subtree prefix is
            // not a concrete key, and asking for its value would be a GET
            // no producer answers (#85).
            Task::batch([
                Task::done(Message::Workspace(WorkspaceMsg::Reveal(path.clone()))),
                Task::done(Message::Subject(SubjectMsg::SelectPath(path))),
            ])
        }
    }
}

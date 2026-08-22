//! The node dashboard (#61): who is alive, and one detail on demand.
//!
//! Since #181 the pane does not own a selection. Clicking a card raises
//! `SubjectMsg::Select(Subject::Origin(..))` and the `node_info` fetch is
//! issued by the subject handler with every other consequence of pointing the
//! window somewhere — so what is left here is the async landing and the
//! click-through, and `&mut Verdicts` is for the detail *verdict*, which is a
//! claim about this fleet and is dropped with it.

use iced::Task;

use crate::message::{Message, Subject, SubjectMsg, WorkspaceMsg};
use crate::scope;
use crate::state::workspace::Verdicts;
use crate::update::Ctx;
use crate::view::nodes::{DetailState, NodesMsg};

pub(crate) fn update(v: &mut Verdicts, msg: NodesMsg, cx: Ctx) -> Task<Message> {
    match msg {
        NodesMsg::InfoLoaded(origin, ran_against, outcome) => {
            // Stale guard: only the currently selected origin's info lands
            // — and only when it ran against the base this window shows.
            // Origin ids are base-independent (#109): re-selecting the
            // same host after a base switch would otherwise admit the old
            // deployment's reply through the selection-only check.
            if cx.sub.current.origin() == Some(origin.as_str()) && ran_against == cx.dep.base() {
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
                Task::done(Message::Subject(SubjectMsg::Select(Subject::Prefix(path)))),
            ])
        }
    }
}

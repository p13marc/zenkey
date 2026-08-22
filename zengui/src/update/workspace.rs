//! The shell around the panes: which one shows, and the tree's own chrome
//! (#175).
//!
//! It names five of six only because of one arm: `Replay` hands straight to
//! [`pane::replay`](super::pane::replay), which is a bus in disguise. Every
//! other arm here moves the tree and nothing else — the honest statement that
//! the shell owns the tree's chrome while the tree owns its rows.

use iced::Task;

use crate::message::{Message, WorkspaceMsg};
use crate::state::{Deployment, Observation, SubjectState, TreeState, Workspace};

/// The shell around the panes, and the replay mode.
pub(crate) fn update(
    dep: &mut Deployment,
    obs: &mut Observation,
    sub: &mut SubjectState,
    tree: &mut TreeState,
    work: &mut Workspace,
    msg: WorkspaceMsg,
) -> Task<Message> {
    match msg {
        WorkspaceMsg::PivotSelected(pivot) => {
            tree.pivot = pivot;
            tree.tree_scroll.0 = 0.0;
            tree.reflatten(dep, obs);
            Task::none()
        }
        WorkspaceMsg::TreeSearchChanged(q) => {
            tree.tree_search = q;
            tree.tree_scroll.0 = 0.0;
            tree.reflatten(dep, obs);
            Task::none()
        }
        WorkspaceMsg::TreeScrolled(y, h) => {
            // View-only state: the next frame renders the new window.
            tree.tree_scroll = (y, h.max(100.0));
            Task::none()
        }
        WorkspaceMsg::ToggleNode(path) => {
            // Collapsing takes the subtree with it (#179) — see
            // `expansion.rs` for why that trade is the fix rather than a
            // side effect of it.
            tree.expanded.toggle(&path);
            tree.reflatten(dep, obs);
            Task::none()
        }
        WorkspaceMsg::Replay(msg) => super::pane::replay::update(dep, obs, sub, tree, work, msg),
        WorkspaceMsg::Reveal(path) => {
            let mut prefix = String::new();
            for chunk in path.split('/') {
                if !prefix.is_empty() {
                    prefix.push('/');
                }
                prefix.push_str(chunk);
                tree.expanded.open(prefix.clone());
            }
            tree.reflatten(dep, obs);
            Task::none()
        }
        WorkspaceMsg::ActivityTab(tab) => {
            // Choosing a stream brings the dock back if it was put away: a
            // tab that selects an invisible thing is a tab that does nothing.
            work.activity.shown = work.activity.tab != tab || !work.activity.shown;
            work.activity.tab = tab;
            Task::none()
        }
        WorkspaceMsg::ActivityToggled => {
            work.activity.shown = !work.activity.shown;
            Task::none()
        }
        WorkspaceMsg::PaneSelected(pane) => {
            work.right_pane = pane;
            Task::none()
        }
    }
}

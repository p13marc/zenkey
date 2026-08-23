//! The shell around the panes: the dock grid, and the tree's own chrome
//! (#175, #180).
//!
//! It names all six sub-states, and each for a traceable reason: the tree
//! arms move the tree; `Replay` hands straight to
//! [`pane::replay`](super::pane::replay), which is a bus in disguise; and the
//! layout arms write `chrome.prefs.layout` — the grid's persisted form —
//! because a layout that does not survive a restart is the bug #180 exists to
//! fix (`prefs.split` was written, clamped, round-trip tested and read by
//! nothing).
//!
//! ## One seam persists the layout
//!
//! Every arm that can change the grid's shape funnels through [`persist`]:
//! capture the tree, stamp it custom (or the preset that made it), and mark
//! the prefs dirty for the settle timer. A splitter drag emits per pixel, so
//! nothing here writes the file directly — the same lesson the window
//! geometry taught in #189.

use iced::Task;
use iced::widget::pane_grid;

use crate::message::{Message, RightPane, WorkspaceMsg};
use crate::prefs::{DockRole, WorkspaceLayout};
use crate::state::{Chrome, Deployment, Observation, SubjectState, TreeState, Workspace};

/// Record the grid's current shape as the given layout name and let the
/// settle timer write it once (#189's lesson: never one file write per
/// pixel of a drag).
fn persist(chrome: &mut Chrome, layout: WorkspaceLayout) {
    chrome.prefs.layout = layout;
    chrome.prefs_dirty = true;
}

/// A layout change by hand: whatever preset it started as, it is custom now.
fn persist_custom(chrome: &mut Chrome, work: &Workspace) {
    persist(chrome, WorkspaceLayout::custom(work.docks.capture()));
}

/// The shell around the panes, and the replay mode.
pub(crate) fn update(
    chrome: &mut Chrome,
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
            // Choosing a stream brings the dock back if it was closed: a
            // control that selects an invisible thing is a control that does
            // nothing.
            work.activity.tab = tab;
            if work.docks.restore(DockRole::Activity) {
                persist_custom(chrome, work);
            }
            Task::none()
        }
        WorkspaceMsg::PaneSelected(pane) => {
            // `Inspector` is a dock, not a workbench tool (#180): revealing
            // it must not overwrite which tool the workbench was on.
            let changed = if pane == RightPane::Inspector {
                work.docks.restore(DockRole::Inspector)
            } else {
                work.right_pane = pane;
                work.docks.restore(DockRole::Workbench)
            };
            if changed {
                persist_custom(chrome, work);
            }
            Task::none()
        }
        WorkspaceMsg::PaneResized(e) => {
            // The widget already bounds the drag to the splitter's legal
            // range; the clamp is for symmetry with the prefs load path, so
            // no ratio the app ever *stores* can pin a dock at zero.
            work.docks.grid.resize(e.split, e.ratio.clamp(0.05, 0.95));
            persist_custom(chrome, work);
            Task::none()
        }
        WorkspaceMsg::PaneDragged(pane_grid::DragEvent::Dropped { pane, target }) => {
            work.docks.grid.drop(pane, target);
            work.docks.focus = Some(pane);
            persist_custom(chrome, work);
            Task::none()
        }
        WorkspaceMsg::PaneDragged(
            pane_grid::DragEvent::Picked { .. } | pane_grid::DragEvent::Canceled { .. },
        ) => Task::none(),
        WorkspaceMsg::DockFocused(pane) => {
            work.docks.focus = Some(pane);
            Task::none()
        }
        WorkspaceMsg::FocusDock(role) => {
            // Alt+L/I/A (#190): focus follows the key the way it follows a
            // click (`DockFocused`), and a closed dock comes back first —
            // `restore` anchors it at its home edge and only that layout
            // change is persisted; moving the focus alone owes no write.
            if work.docks.restore(role) {
                persist_custom(chrome, work);
            }
            work.docks.focus = work.docks.pane_of(role);
            Task::none()
        }
        WorkspaceMsg::DockToggled(role) => {
            if work.docks.toggle(role) {
                persist_custom(chrome, work);
            }
            Task::none()
        }
        WorkspaceMsg::LayoutPreset(preset) => {
            use crate::prefs::LayoutPreset;
            work.docks = crate::state::workspace::DockGrid::from_layout(&preset.root());
            // A preset is a stance, not just a shape (epic #172): Watch
            // opens on the echo stream, Diagnose pivots the locator by
            // origin and opens on the doctor.
            match preset {
                LayoutPreset::Explore => {}
                LayoutPreset::Watch => work.activity.tab = crate::message::ActivityTab::Echo,
                LayoutPreset::Diagnose => {
                    work.activity.tab = crate::message::ActivityTab::Doctor;
                    if tree.pivot != crate::view::tree::Pivot::Origin {
                        tree.pivot = crate::view::tree::Pivot::Origin;
                        tree.tree_scroll.0 = 0.0;
                        tree.reflatten(dep, obs);
                    }
                }
            }
            persist(chrome, preset.layout());
            Task::none()
        }
    }
}

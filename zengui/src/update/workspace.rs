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
//!
//! ## A role has one home (#186)
//!
//! `TearOff` moves a dock from the grid to a window of its own;
//! `WindowClosed` is the way back — and the main window's close is the
//! application's exit, because a daemon never stops on its own. While a role
//! is torn, every reveal path that would `restore` it into the grid focuses
//! its window instead: a dock rendered in the grid *and* in a window would
//! be one region making two claims.

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
/// The capture is both halves of the layout — the grid's tree *and* the
/// torn-off windows (#186), because a tear-off that vanished from the file
/// would re-dock on restart.
pub(crate) fn persist_custom(chrome: &mut Chrome, work: &Workspace) {
    persist(
        chrome,
        WorkspaceLayout {
            preset: None,
            root: work.docks.capture(),
            torn: work.windows.capture(),
        },
    );
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
            // nothing. A *torn* dock is not invisible — it is elsewhere, so
            // the reveal is its window, focused (#186).
            work.activity.tab = tab;
            if let Some(id) = work.windows.window_of(DockRole::Activity) {
                return iced::window::gain_focus(id);
            }
            if work.docks.restore(DockRole::Activity) {
                persist_custom(chrome, work);
            }
            Task::none()
        }
        WorkspaceMsg::PaneSelected(pane) => {
            // `Inspector` is a dock, not a workbench tool (#180): revealing
            // it must not overwrite which tool the workbench was on.
            let role = if pane == RightPane::Inspector {
                DockRole::Inspector
            } else {
                work.right_pane = pane;
                DockRole::Workbench
            };
            // A role has one home (#186): while its window is torn off,
            // restoring it into the grid would show the dock twice, so the
            // reveal is that window.
            if let Some(id) = work.windows.window_of(role) {
                return iced::window::gain_focus(id);
            }
            if work.docks.restore(role) {
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
            // A torn dock's focus is its window's (#186).
            if let Some(id) = work.windows.window_of(role) {
                return iced::window::gain_focus(id);
            }
            if work.docks.restore(role) {
                persist_custom(chrome, work);
            }
            work.docks.focus = work.docks.pane_of(role);
            Task::none()
        }
        WorkspaceMsg::DockToggled(role) => {
            // While a dock is torn off, the strip's toggle points at its
            // window (#186): restoring the role into the grid beside its
            // open window would render one region twice.
            if let Some(id) = work.windows.window_of(role) {
                return iced::window::gain_focus(id);
            }
            if work.docks.toggle(role) {
                persist_custom(chrome, work);
            }
            Task::none()
        }
        WorkspaceMsg::TearOff(role) => {
            // The Locator never tears off (#186): there is one bus and one
            // tree of it, and the window that navigates is the main one.
            if role == DockRole::Locator {
                return Task::none();
            }
            // Already torn: a second tear-off is the first window, focused —
            // two windows showing one dock would be two claims about one
            // region.
            if let Some(id) = work.windows.window_of(role) {
                return iced::window::gain_focus(id);
            }
            // The dock leaves the grid first, by the #180 machinery — and
            // its refusals hold: the last dock stays, because a main window
            // with zero regions renders nothing and can never be clicked
            // back. (A dock *closed* in the grid tears off without it: the
            // window is where it reopens.)
            if work.docks.is_open(role) && !work.docks.close(role) {
                return Task::none();
            }
            let (id, open) =
                iced::window::open(crate::state::workspace::torn_settings(role, None, None));
            work.windows.torn.push(crate::state::workspace::TornWindow {
                id,
                role,
                size: None,
                position: None,
            });
            persist_custom(chrome, work);
            open.discard()
        }
        WorkspaceMsg::WindowClosed(id) => {
            use crate::state::workspace::ClosedWindow;
            match work.windows.classify(id) {
                // The main window is the application (#186): a daemon does
                // not stop with its last window, so this is where closing
                // it becomes a clean shutdown rather than a zombie process.
                // A dirty preference — a drag the settle timer never
                // reached — is flushed on the way out, through the same
                // services seam as every other write (#255).
                ClosedWindow::Main => {
                    work.windows.exiting = true;
                    let goodbye = iced::exit();
                    if chrome.prefs_dirty {
                        chrome.prefs_dirty = false;
                        crate::services::prefs::save(chrome.prefs.clone()).chain(goodbye)
                    } else {
                        goodbye
                    }
                }
                // A torn-off dock's window: the dock comes home. Restore,
                // not toggle — the role cannot be in the grid while torn,
                // and its home edge is where #180 put it.
                ClosedWindow::Torn(role) => {
                    work.windows.remove(id);
                    work.docks.restore(role);
                    persist_custom(chrome, work);
                    Task::none()
                }
                // A close racing a layout change (a preset already
                // re-docked this window's role): everything it would do is
                // done.
                ClosedWindow::Unknown => Task::none(),
            }
        }
        WorkspaceMsg::LayoutPreset(preset) => {
            use crate::prefs::LayoutPreset;
            work.docks = crate::state::workspace::DockGrid::from_layout(&preset.root());
            // All three presets are fully docked, and the new grid already
            // holds every role — a torn window left open would show a dock
            // the grid also shows. Forgotten first, so each window's close
            // event classifies as `Unknown` and cannot double-restore.
            let redocked: Vec<Task<Message>> = work
                .windows
                .torn
                .drain(..)
                .map(|t| iced::window::close(t.id))
                .collect();
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
            Task::batch(redocked)
        }
    }
}

//! What the user has open, typed, run and captured.
//!
//! Four sub-groups, split by what a base change owes each:
//!
//! - [`Verdicts`] — the nodes, doctor, blob and admin panes, whose content is
//!   *a verdict about a fleet*. They are dropped with it. (`roster` lives here
//!   rather than in [`super::observation`], which is the one placement worth
//!   arguing about: it is fed by the tick, like the key tree. But being fed by
//!   the tick is not a group — the tick also feeds `media.viewing` and the
//!   subject\'s history. It is `NodesData`\'s first field, and it is cleared
//!   with `node_detail`, so it is filed with it. The origin *selection* is no
//!   longer here at all — since #181 there is one subject, and it is the
//!   user's, not this pane's.)
//! - [`Workbench`] — what the user typed. Kept: a half-written publish body is
//!   not a claim about any fleet.
//! - [`EchoPane`] — the live line ring and its filters. Kept, and the ring is
//!   bounded at launch.
//! - [`ReplayMode`] — a mode, and a base change inside it is the user moving
//!   around the file they opened.

use std::sync::Arc;

use iced::widget::pane_grid;

use crate::echo::EchoRing;
use crate::message::{ActivityTab, RightPane};
use crate::prefs::{DockRole, LayoutAxis, LayoutNode};
use crate::view;

/// The workspace grid (#180): `pane_grid::State` plus the focus.
///
/// `pane_grid::State` is not serializable — its pane and split ids are
/// widget-internal counters — so this is the runtime half of a pair: the grid
/// is **rebuilt** from the persisted [`LayoutNode`] on launch
/// ([`DockGrid::from_layout`]) and **captured** back into one on every layout
/// change ([`DockGrid::capture`]). The ids differ across a restart; the tree
/// of splits, ratios and roles is what survives, and it is all that matters.
pub(crate) struct DockGrid {
    pub(crate) grid: pane_grid::State<DockRole>,
    /// The dock last clicked — where a restored dock anchors, and what #190's
    /// dock-focus keys will move.
    pub(crate) focus: Option<pane_grid::Pane>,
}

fn configuration(node: &LayoutNode) -> pane_grid::Configuration<DockRole> {
    match node {
        LayoutNode::Split { axis, ratio, a, b } => pane_grid::Configuration::Split {
            axis: match axis {
                LayoutAxis::Horizontal => pane_grid::Axis::Horizontal,
                LayoutAxis::Vertical => pane_grid::Axis::Vertical,
            },
            ratio: *ratio,
            a: Box::new(configuration(a)),
            b: Box::new(configuration(b)),
        },
        LayoutNode::Dock(role) => pane_grid::Configuration::Pane(*role),
    }
}

fn capture(
    node: &pane_grid::Node,
    panes: &std::collections::BTreeMap<pane_grid::Pane, DockRole>,
) -> Option<LayoutNode> {
    match node {
        pane_grid::Node::Split {
            axis, ratio, a, b, ..
        } => {
            match (capture(a, panes), capture(b, panes)) {
                (Some(a), Some(b)) => Some(LayoutNode::Split {
                    axis: match axis {
                        pane_grid::Axis::Horizontal => LayoutAxis::Horizontal,
                        pane_grid::Axis::Vertical => LayoutAxis::Vertical,
                    },
                    ratio: *ratio,
                    a: Box::new(a),
                    b: Box::new(b),
                }),
                // A split with one missing side collapses to the other — the
                // shape `State::close` leaves cannot actually produce this,
                // but a capture must not invent a dock to fill a hole.
                (Some(one), None) | (None, Some(one)) => Some(one),
                (None, None) => None,
            }
        }
        pane_grid::Node::Pane(pane) => panes.get(pane).map(|role| LayoutNode::Dock(*role)),
    }
}

impl DockGrid {
    pub(crate) fn from_layout(root: &LayoutNode) -> DockGrid {
        DockGrid {
            grid: pane_grid::State::with_configuration(configuration(root)),
            focus: None,
        }
    }

    /// The serializable tree this grid is, for prefs. `None` is structurally
    /// unreachable (a grid holds at least one pane); the fallback keeps the
    /// persisted layout legal rather than panicking over a widget invariant.
    pub(crate) fn capture(&self) -> LayoutNode {
        capture(self.grid.layout(), &self.grid.panes)
            .unwrap_or(LayoutNode::Dock(DockRole::Inspector))
    }

    pub(crate) fn pane_of(&self, role: DockRole) -> Option<pane_grid::Pane> {
        self.grid
            .iter()
            .find(|(_, r)| **r == role)
            .map(|(pane, _)| *pane)
    }

    pub(crate) fn is_open(&self, role: DockRole) -> bool {
        self.pane_of(role).is_some()
    }

    /// Bring a closed dock back, each role at its home edge — the locator on
    /// the left, the activity dock on the bottom, the inspector and the
    /// workbench on the right. Returns whether the layout changed, so the
    /// caller knows to persist it.
    pub(crate) fn restore(&mut self, role: DockRole) -> bool {
        if self.is_open(role) {
            return false;
        }
        let anchor = self
            .focus
            .filter(|p| self.grid.get(*p).is_some())
            .or_else(|| self.grid.iter().next().map(|(pane, _)| *pane));
        let Some(anchor) = anchor else {
            return false;
        };
        let edge = match role {
            DockRole::Locator => pane_grid::Edge::Left,
            DockRole::Activity => pane_grid::Edge::Bottom,
            DockRole::Inspector | DockRole::Workbench => pane_grid::Edge::Right,
        };
        let Some((pane, _)) = self.grid.split(pane_grid::Axis::Vertical, anchor, role) else {
            return false;
        };
        self.grid.drop(pane, pane_grid::Target::Edge(edge));
        self.focus = Some(pane);
        true
    }

    /// Close an open dock. The last dock stays: a workspace with zero regions
    /// is a window that renders nothing and can never be clicked back.
    /// Returns whether the layout changed.
    pub(crate) fn close(&mut self, role: DockRole) -> bool {
        if self.grid.len() <= 1 {
            return false;
        }
        let Some(pane) = self.pane_of(role) else {
            return false;
        };
        let sibling = self.grid.close(pane).map(|(_, sibling)| sibling);
        if self.focus == Some(pane) {
            self.focus = sibling;
        }
        true
    }

    pub(crate) fn toggle(&mut self, role: DockRole) -> bool {
        if self.is_open(role) {
            self.close(role)
        } else {
            self.restore(role)
        }
    }
}

/// What an armed repeating publication resends each tick: the declaration,
/// the prepared bytes, and the attachment that rode the first send (#117).
pub(crate) struct RepeatLoad {
    pub(crate) publication: Arc<zenkey_fleet::Publication>,
    pub(crate) bytes: Arc<Vec<u8>>,
    pub(crate) attachment: Option<Arc<Vec<u8>>>,
}

/// The bottom dock: the session's time-ordered streams (#183).
///
/// Echo, the publish log, doctor results and the replay scrubber are all
/// about the *session* and none of them about the subject — which is why they
/// competed badly for tab slots with panes that follow the subject, and why
/// verifying a publish used to mean leaving the form to look at Echo.
/// Putting the dock away is the grid's job since #180 — closing its pane —
/// so the old `shown` flag is gone: two ways to hide one region is one way
/// too many, and the grid's way survives a restart.
#[derive(Default)]
pub(crate) struct ActivityDock {
    pub(crate) tab: ActivityTab,
}

/// A running capture (#74, started from the location bar): dropping the
/// notify without firing it would leak the task, so `stop` is fired on
/// toggle-off and on exit.
pub(crate) struct RecordingHandle {
    pub(crate) stop: Arc<tokio::sync::Notify>,
    pub(crate) path: String,
}

sub_state! {
    /// Panes whose content is a verdict about a fleet.
    #[derive(Default)]
    pub(crate) struct Verdicts {
        /// The node dashboard's presence model (#61), fed by liveliness only.
        pub(crate) roster: crate::nodes::NodeRoster,
        /// Its one-shot `node_info` detail — the pane's only data-plane cost.
        pub(crate) node_detail: view::nodes::DetailState,
        /// The doctor panel's run state (#71) — run-on-demand only.
        pub(crate) doctor: crate::doctor::DoctorState,
        /// The blob browser's state (#68) — probe and fetch, both on demand.
        pub(crate) blob: crate::blob::BlobState,
        /// The admin & storage panel's state (#70) — swept on demand.
        pub(crate) admin: crate::admin::AdminState,
    }
}

impl Verdicts {
    /// Drop every verdict — deliberately **not** `Default::default()`.
    ///
    /// `DoctorState::clear` keeps `deep` and `listen` (user input, not a
    /// verdict) and `BlobState::clear` cancels an in-flight transfer first. A
    /// blanket default would abandon a running fetch and silently retype the
    /// doctor\'s form.
    pub(crate) fn forget(&mut self) {
        self.roster.clear();
        self.node_detail = view::nodes::DetailState::NotAsked;
        self.doctor.clear();
        self.blob.clear();
        self.admin.clear();
    }
}

sub_state! {
    /// What the user typed, and the one live declaration it can arm.
    #[derive(Default)]
    pub(crate) struct Workbench {
        /// The connection pane's state (issue #67): contexts and endpoints.
        pub(crate) context_form: view::contexts::ContextForm,

        pub(crate) call_form: view::call::CallForm,
        pub(crate) publish_form: view::publish::PublishForm,
        /// The armed publication and what it repeats (#60). Held here rather
        /// than in the form because a `Publication` is a live bus declaration, not
        /// view state — dropping it undeclares.
        pub(crate) publication: Option<RepeatLoad>,
        /// The media viewer's state (#69) — subscribes only on an explicit
        /// view, never for opening the pane (the RFC 07 §1 plane deserves the
        /// laziest posture in the app).
        pub(crate) media: view::media::MediaState,
    }
}

sub_state! {
    /// The live line ring and how it is filtered.
    pub(crate) struct EchoPane {
        pub(crate) echo: EchoRing,
        /// The echo pane's view state (issue #72): filters, follow-tail, gaps.
        pub(crate) echo_view: view::echo::EchoView,
        /// Scroll position + viewport height, driving the virtual window
        /// (#183). Session-lived, unlike the timeline's: the stream is about
        /// the session, not about the subject.
        pub(crate) echo_scroll: (f32, f32),
    }
}

sub_state! {
    /// Replay (#74) and capture, which are the same pane\'s two directions.
    #[derive(Default)]
    pub(crate) struct ReplayMode {
        /// Replay mode (issue #74): while `Some`, the panes are fed from the
        /// file and the live link subscription is not built at all — nothing in
        /// replay can publish or subscribe, structurally.
        pub(crate) replay: Option<crate::replay::ReplayState>,
        /// The open row's path input; `None` = row hidden.
        pub(crate) replay_open: Option<String>,
        /// Why the last open failed, shown beside the path box.
        pub(crate) replay_note: Option<String>,
        /// A capture in flight (the location bar's record toggle): the stop signal
        /// and where it is writing.
        pub(crate) recording: Option<RecordingHandle>,
        /// The last finished capture, for the status strip: (samples, dropped,
        /// path) or the failure.
        pub(crate) recorded: Option<Result<(u64, u64, String), String>>,
    }
}

sub_state! {
    pub(crate) struct Workspace {
        /// The dock grid (#180): which regions are open, where, and how big.
        pub(crate) docks: DockGrid,
        /// Which tool the Workbench dock is showing. `Inspector` is not a
        /// tool — the Inspector is a dock of its own since #180, and
        /// [`WorkspaceMsg::PaneSelected`](crate::message::WorkspaceMsg) maps
        /// it to that dock rather than storing it here.
        pub(crate) right_pane: RightPane,
        pub(crate) verdicts: Verdicts,
        pub(crate) activity: ActivityDock,
        pub(crate) bench: Workbench,
        pub(crate) echo: EchoPane,
        pub(crate) replay: ReplayMode,
    }
}

impl Workspace {
    pub(crate) fn new(echo_lines: usize, layout: &LayoutNode) -> Workspace {
        Workspace {
            docks: DockGrid::from_layout(layout),
            right_pane: RightPane::Call,
            verdicts: Verdicts::default(),
            activity: ActivityDock::default(),
            bench: Workbench::default(),
            echo: EchoPane {
                echo: EchoRing::new(echo_lines),
                echo_view: view::echo::EchoView::new(),
                echo_scroll: (0.0, 600.0),
            },
            replay: ReplayMode::default(),
        }
    }
}

#[cfg(test)]
mod grid_tests {
    use super::*;
    use crate::prefs::LayoutPreset;

    /// The round trip the persistence rests on: rebuild-from-layout then
    /// capture is the identity, for every preset and for a bent tree.
    #[test]
    fn a_layout_rebuilds_and_captures_to_itself() {
        for p in LayoutPreset::ALL {
            let root = p.root();
            assert_eq!(
                DockGrid::from_layout(&root).capture(),
                root,
                "{}",
                p.label()
            );
        }
        let bent = LayoutNode::Split {
            axis: LayoutAxis::Horizontal,
            ratio: 0.42,
            a: Box::new(LayoutNode::Dock(DockRole::Workbench)),
            b: Box::new(LayoutNode::Dock(DockRole::Activity)),
        };
        assert_eq!(DockGrid::from_layout(&bent).capture(), bent);
    }

    /// Close removes exactly one dock; restore brings it back; the last dock
    /// cannot be closed at all.
    #[test]
    fn docks_close_and_restore_and_the_last_one_stays() {
        let mut docks = DockGrid::from_layout(&LayoutPreset::Watch.root());
        assert!(docks.is_open(DockRole::Activity));
        assert!(docks.close(DockRole::Activity));
        assert!(!docks.is_open(DockRole::Activity));
        assert!(!docks.close(DockRole::Activity), "already closed");

        assert!(docks.restore(DockRole::Activity));
        assert!(docks.is_open(DockRole::Activity));
        assert!(!docks.restore(DockRole::Activity), "already open");

        assert!(docks.close(DockRole::Locator));
        assert!(docks.close(DockRole::Activity));
        assert!(
            !docks.close(DockRole::Inspector),
            "the last dock must survive"
        );
        assert!(docks.is_open(DockRole::Inspector));
    }
}

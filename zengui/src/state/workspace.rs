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

use iced::window;

use crate::echo::EchoRing;
use crate::message::{ActivityTab, RightPane, SlotId};
use crate::prefs::{DockRole, LayoutAxis, LayoutNode, TornDock};
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
    /// The dock last clicked (`DockFocused`) or summoned by its Alt-letter
    /// (`FocusDock`, #190) — where a restored dock anchors.
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

/// One dock in a window of its own (#186): the runtime half of a
/// [`TornDock`], carrying the widget-internal window id the way [`DockGrid`]
/// carries pane ids — minted at open, different across a restart, never
/// serialized.
pub(crate) struct TornWindow {
    pub(crate) id: window::Id,
    pub(crate) role: DockRole,
    /// Which subject slot this window renders (#257). [`SlotId::FOLLOW`] for
    /// every role but a pinned Inspector: tearing the Inspector off pins the
    /// current subject into a slot of its own, and this is the binding. A
    /// pinned window is session-only — see [`WindowSet::capture`].
    pub(crate) slot: SlotId,
    /// The window's last reported size, for the persisted layout.
    pub(crate) size: Option<(f32, f32)>,
    /// The window's last reported position (`None` where the platform never
    /// says — Wayland).
    pub(crate) position: Option<(f32, f32)>,
}

/// What a closed window was (#186) — the whole close policy, as data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ClosedWindow {
    /// The main workspace window: closing it is closing the application.
    /// A daemon does not stop when its windows are gone (iced 0.14
    /// `daemon.rs`), so without this verdict the process would outlive its
    /// last window as a zombie.
    Main,
    /// A torn-off dock's window: the dock returns to the grid.
    Torn(DockRole),
    /// A window this state never knew or already forgot — a close event
    /// racing a layout change. Answering `Main` here would exit the app on a
    /// stale event, so "I don't know" must be sayable.
    Unknown,
}

/// The session's windows (#186): the main workspace and every torn-off dock.
///
/// Like [`DockGrid`], this is the runtime half of a persisted pair: the ids
/// are minted by `window::open` at boot ([`crate::app::Zengui`]) or on a
/// tear-off, and what survives a restart is the [`TornDock`] list in the
/// named layout ([`crate::prefs::WorkspaceLayout::torn`]).
#[derive(Default)]
pub(crate) struct WindowSet {
    /// The main window. `None` only in a test that never booted — the view
    /// treats every id it cannot name as the main workspace, so this is read
    /// solely by [`WindowSet::classify`].
    pub(crate) main: Option<window::Id>,
    pub(crate) torn: Vec<TornWindow>,
    /// The main window closed and `iced::exit` is in flight — pinned here so
    /// the close semantics are a fact a test can read, not only a task it
    /// cannot.
    pub(crate) exiting: bool,
}

impl WindowSet {
    pub(crate) fn role_of(&self, id: window::Id) -> Option<DockRole> {
        self.torn.iter().find(|t| t.id == id).map(|t| t.role)
    }

    pub(crate) fn window_of(&self, role: DockRole) -> Option<window::Id> {
        self.torn.iter().find(|t| t.role == role).map(|t| t.id)
    }

    /// The window that is this role's *home* — bound to the follow slot —
    /// if it has one (#186, #257). A pinned Inspector window is not the
    /// Inspector's home: it holds its own subject, so the reveal paths must
    /// not point the selection at it. Every role but the Inspector only ever
    /// has follow-bound windows, so for them this is [`WindowSet::window_of`].
    pub(crate) fn follow_window_of(&self, role: DockRole) -> Option<window::Id> {
        self.torn
            .iter()
            .find(|t| t.role == role && t.slot == SlotId::FOLLOW)
            .map(|t| t.id)
    }

    /// The slot the given window renders, when it is a torn dock's (#257).
    pub(crate) fn slot_of(&self, id: window::Id) -> Option<SlotId> {
        self.torn.iter().find(|t| t.id == id).map(|t| t.slot)
    }

    pub(crate) fn classify(&self, id: window::Id) -> ClosedWindow {
        if self.main == Some(id) {
            ClosedWindow::Main
        } else if let Some(role) = self.role_of(id) {
            ClosedWindow::Torn(role)
        } else {
            ClosedWindow::Unknown
        }
    }

    pub(crate) fn remove(&mut self, id: window::Id) {
        self.torn.retain(|t| t.id != id);
    }

    /// The serializable half, for the named layout — the same capture shape
    /// as [`DockGrid::capture`].
    ///
    /// Pinned windows are deliberately left out (#257): a pin's worth is its
    /// evidence — the recorder, the fetch, the decode — and evidence is
    /// session-lived. Persisting the identity alone would restore a window
    /// titled with a key and empty of everything about it, which is the
    /// identity-only freeze the issue rejects, one restart over. What
    /// persists is the follow-bound torn state, exactly as #186 left it.
    pub(crate) fn capture(&self) -> Vec<TornDock> {
        self.torn
            .iter()
            .filter(|t| t.slot == SlotId::FOLLOW)
            .map(|t| TornDock {
                role: t.role,
                size: t.size,
                position: t.position,
            })
            .collect()
    }
}

/// What a torn-off dock's window opens as (#186): the persisted geometry
/// when there is one, else a default sized for what the dock holds — wide
/// and short for the Activity streams, tall for the subject surfaces.
pub(crate) fn torn_settings(
    role: DockRole,
    size: Option<(f32, f32)>,
    position: Option<(f32, f32)>,
) -> window::Settings {
    let (w, h) = size.unwrap_or(match role {
        DockRole::Activity => (960.0, 380.0),
        DockRole::Inspector | DockRole::Workbench => (520.0, 700.0),
        // Never torn; the arm exists because a match must say what it
        // would mean.
        DockRole::Locator => (400.0, 700.0),
    });
    window::Settings {
        size: iced::Size::new(w, h),
        position: position
            .map(|(x, y)| window::Position::Specific(iced::Point::new(x, y)))
            .unwrap_or_default(),
        ..window::Settings::default()
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
        /// The payload-conformance verdict cache (#164): per-key verdicts of
        /// the tick's bounded validation batches. Filed here because a
        /// verdict about a payload is a verdict about a fleet — judged under
        /// its schemas — and dropped with the rest of them.
        pub(crate) payloads: crate::verdict::VerdictCache,
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
        // The verdicts were judged under the old fleet's schemas (#164).
        self.payloads.clear();
    }
}

sub_state! {
    /// What the user typed, and the one live declaration it can arm.
    #[derive(Default)]
    pub(crate) struct Workbench {
        /// The connection pane's state (issue #67): contexts and endpoints.
        pub(crate) context_form: view::contexts::ContextForm,
        /// The key-expression editor's draft (#187) — the Selectors overlay.
        pub(crate) scope_form: view::scope_editor::ScopeForm,
        /// The Settings overlay's draft (#188).
        pub(crate) settings_form: view::settings::SettingsForm,

        /// The Send pane's one form (#184): publish and call, behind a mode
        /// toggle.
        pub(crate) send_form: view::send::SendForm,
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
        /// A `.zrec` parse in flight (#255): the path being loaded. The
        /// replay tab renders it as an explicit loading state — a load is
        /// not an empty capture and not a hung window (RFC 09 §5.1 O4).
        pub(crate) replay_loading: Option<String>,
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
        /// The session's windows (#186): the main one, and a window per
        /// torn-off dock. Empty until boot opens them — the pure
        /// constructor mints no window ids, so a headless test never does.
        pub(crate) windows: WindowSet,
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
            windows: WindowSet::default(),
            right_pane: RightPane::Send,
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

    /// The close policy as data (#186): the main window is `Main`, a torn
    /// dock's window names its role, and anything else — a close event
    /// racing a layout change — is `Unknown`, because answering `Main` for
    /// a stale id would exit the application on a window that no longer
    /// matters.
    #[test]
    fn a_window_set_classifies_and_forgets() {
        let mut set = WindowSet::default();
        let main = window::Id::unique();
        let torn = window::Id::unique();
        let stranger = window::Id::unique();
        assert_eq!(
            set.classify(main),
            ClosedWindow::Unknown,
            "before boot, no id is the main window"
        );
        set.main = Some(main);
        set.torn.push(TornWindow {
            id: torn,
            role: DockRole::Activity,
            slot: SlotId::FOLLOW,
            size: None,
            position: None,
        });
        assert_eq!(set.classify(main), ClosedWindow::Main);
        assert_eq!(set.classify(torn), ClosedWindow::Torn(DockRole::Activity));
        assert_eq!(set.classify(stranger), ClosedWindow::Unknown);
        assert_eq!(set.window_of(DockRole::Activity), Some(torn));
        assert_eq!(set.role_of(torn), Some(DockRole::Activity));

        set.remove(torn);
        assert_eq!(
            set.classify(torn),
            ClosedWindow::Unknown,
            "a forgotten window is a stranger — its late events do nothing"
        );
        assert_eq!(set.window_of(DockRole::Activity), None);
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

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

use crate::echo::EchoRing;
use crate::message::{ActivityTab, RightPane};
use crate::view;

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
pub(crate) struct ActivityDock {
    pub(crate) tab: ActivityTab,
    /// Collapsed to its tab strip. A dock you cannot put away is a dock that
    /// costs screen whether or not you are reading it.
    pub(crate) shown: bool,
}

impl Default for ActivityDock {
    fn default() -> ActivityDock {
        ActivityDock {
            tab: ActivityTab::default(),
            shown: true,
        }
    }
}

/// A running toolbar capture (#74): dropping the notify without firing it
/// would leak the task, so `stop` is fired on toggle-off and on exit.
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
        /// A capture in flight (the toolbar's record toggle): the stop signal
        /// and where it is writing.
        pub(crate) recording: Option<RecordingHandle>,
        /// The last finished capture, for the status strip: (samples, dropped,
        /// path) or the failure.
        pub(crate) recorded: Option<Result<(u64, u64, String), String>>,
    }
}

sub_state! {
    pub(crate) struct Workspace {
        /// Which right-hand pane is showing.
        pub(crate) right_pane: RightPane,
        pub(crate) verdicts: Verdicts,
    pub(crate) activity: ActivityDock,
        pub(crate) bench: Workbench,
        pub(crate) echo: EchoPane,
        pub(crate) replay: ReplayMode,
    }
}

impl Workspace {
    pub(crate) fn new(echo_lines: usize) -> Workspace {
        Workspace {
            right_pane: RightPane::Inspector,
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

//! The whole event vocabulary, in one file.
//!
//! Six groups since #176, and one rule decides every placement:
//!
//! > **A message lives where its failure is displayed.**
//!
//! `SessionOpened(Err)` writes `link`, which the status strip renders → `Bus`.
//! `CallDone(Err)` writes `CallForm::outcome` → `Pane`. `ContextSwitched(Err)`
//! writes `ContextForm::status` → `Pane`. It settles the cases a topic-shaped
//! grouping leaves to taste, and it is why `Reconnect` is `Deployment` (its
//! handler is the tail of `BaseSelected`'s) rather than `Bus` or `Chrome`
//! (it is reachable from the toolbar, Ctrl-R *and* the palette, so provenance
//! says nothing).
//!
//! ## What this file no longer claims
//!
//! It used to say it was "in one file so the async/sync boundary is readable at
//! a glance". That was already half-false when it was written: six async
//! landings were listed here while twelve more lived in `view/*.rs`, so the
//! file looked complete and was not. #176 moved the six to join the twelve,
//! which makes it uniformly false — the better state, because a reader can now
//! find the boundary rather than half of it.
//!
//! **Every async landing is a `Task::perform` in `app.rs` or a `yield` in
//! `link.rs`. Grep those.** This file is the top-level vocabulary.

use std::sync::Arc;

use zenkey_fleet::{
    DiscoveredBase, FetchOutcome, KeyTreeSnapshot, Monitor, SampleView, Skeleton, SliceSet, WatchId,
};

use crate::scope::ScopePreset;

/// The engine roster's shape: origin → live producer names.
pub type LiveRoster = std::collections::BTreeMap<String, Vec<String>>;

/// Everything the bus, the monitor or a fleet sweep answered (#176).
///
/// None of these is user intent: each is a `Task::perform` or a `link.rs`
/// `yield` landing. A message lives where its failure is displayed, and every
/// failure here reaches the status strip rather than a pane —
/// `SessionOpened(Err)` and `MonitorStarted(Err)` write `link`, and
/// `SkeletonBuilt(Err)` is a `tracing::warn!` nobody's pane shows.
///
/// `SlicesLoaded` and `SlicesUnionLoaded` are here rather than under
/// `Deployment` for the same reason: a registry sweep is a *landing*, like
/// `BasesDiscovered`. `Deployment` holds the intent that asked for it.
#[derive(Debug, Clone)]
pub enum BusMsg {
    /// One coalesced batch from the bus link. See [`BusTick`].
    Tick(Arc<BusTick>),
    /// The link changed state.
    Link(LinkState),
    /// A session was opened (or could not be).
    SessionOpened(Result<zenoh::Session, String>),
    /// The monitor started (lazily — no data-plane watches yet unless eager).
    MonitorStarted(Result<Arc<Monitor>, String>),
    /// The declared-keyspace skeleton was (re)built, with the liveliness
    /// roster the build task gathered anyway (#61 seeds the node roster
    /// from it instead of throwing it away).
    SkeletonBuilt(Result<(Arc<Skeleton>, Arc<LiveRoster>), String>),
    /// The base sweep finished. An empty list is *not* a verdict (RFC 05 §3.1).
    BasesDiscovered(Result<Vec<DiscoveredBase>, String>),
    /// Registry slices arrived, from the bus or from `--registry` dirs.
    SlicesLoaded(Result<Arc<SliceSet>, String>),
    /// The §6.1 union arrived: (set, from_bus, dirs_only, disagreements).
    SlicesUnionLoaded(Result<(Arc<SliceSet>, usize, usize, usize), String>),
}

/// What the app is pointed at, and the coverage that follows (#176).
///
/// The invariant: **every variant here invalidates or re-points observation.**
///
/// `Reconnect` is here rather than under `Bus`, on behaviour rather than taste:
/// its handler is byte-for-byte the tail of `BaseSelected`'s, and a message whose
/// body is a subset of another's belongs in that group. Where it is *reached
/// from* — the toolbar, Ctrl-R, the palette — is a bad tiebreaker, because it is
/// all three.
///
/// The scope watches come along for the same reason: `ScopeSelected`'s handler
/// *is* `unwatch_scope()` then `watch_scope()`, so keeping them apart would put
/// one behaviour in two enums.
#[derive(Debug, Clone)]
pub enum DeploymentMsg {
    /// The scope preset's watches were declared (the eager set).
    ScopeWatchesStarted(Vec<WatchId>),
    /// The scope preset's watches were released.
    ///
    /// The mirror of `ScopeWatchesStarted`, and it did not exist: the release
    /// half reported through `SubjectMsg::WatchReleased` with a synthesised
    /// `"(scope)"` path, which was read in exactly one place — the string
    /// interpolated into a failure warning. Two halves of one operation
    /// landing in two groups, held together by a fake key (#175).
    ScopeWatchesReleased(Result<(), String>),
    /// Apply/release the scope preset's selectors as watches — the eager
    /// mode, made explicit and labelled by its cost.
    ScopeWatchToggled,
    BaseSelected(String),
    /// A stored context was chosen, and the deployment must now become it.
    ///
    /// The connect pane raises this rather than rewriting seven `Settings`
    /// fields itself. That keeps the pane's handler about the *form* — which
    /// is what lets it take `(&mut ContextForm, Ctx)` once the panes dock
    /// (#180) — and puts the deployment rewrite where every other one is.
    ///
    /// `name` is what to remember as the active context, `None` when the
    /// context is unnamed.
    ContextApplied {
        name: Option<String>,
        stored: Box<zenkey_fleet::StoredContext>,
    },
    ScopeSelected(ScopePreset),
    Reconnect,
}

/// One key: chosen, observed, fetched, decoded (#176).
///
/// `SelectKey` heads a causal chain — its own handler ends in the
/// `Task::perform` that produces `ValueFetched`, which produces `ValueDecoded` —
/// so filing the head under the workspace and the tail here would force
/// `update_subject` to re-enter `update_workspace` to do its own job.
///
/// `ValueFetched`/`ValueDecoded` are deliberately *not* folded into `DetailMsg`,
/// which the async-result rule might seem to demand. They are not a pane's
/// result: the fetch is issued by the **tree's** selection and the handler flips
/// `right_pane`. A detail pane owning a fetch it never requested would be a
/// worse lie than the placement it replaced.
#[derive(Debug, Clone)]
pub enum SubjectMsg {
    /// The user toggled observation of one subtree (the tree's watch button).
    /// Carries the row's display path.
    WatchToggled(String),
    /// A watch was declared for the given display path.
    WatchStarted(String, Result<WatchId, String>),
    /// A watch was released for the given display path.
    WatchReleased(String, Result<(), String>),
    /// A value arrived for the selected key ([`zenkey_fleet::fetch_value`]).
    ValueFetched(String, Result<Arc<FetchOutcome>, String>),
    /// The fetched value's schema decode finished (§6.4 item 5's inspector):
    /// (key, declared type if any, rendering).
    ValueDecoded(String, Option<String>, Arc<zenkey_fleet::decode::Rendering>),
    SelectKey(Option<String>),
    /// Select a *subtree prefix* — no fetch.
    ///
    /// Not a cosmetic split from `SelectKey`. A prefix is not a key, and
    /// routing "show this origin in the tree" through `SelectKey` would issue
    /// a `fetch_value` GET against something no producer publishes: an
    /// unasked-for bus query introduced by a refactor, which is exactly what
    /// #85's laziness rule forbids.
    SelectPath(String),
}

/// The shell around the panes: which one shows, the tree's own chrome, and the
/// replay mode (#176).
///
/// **Not what #176's issue body describes.** It sketched "pane_grid
/// drag/resize/close, layout switch, window open/close" — none of which exist:
/// `grep -rn pane_grid zengui/src` is empty and the layout is a fixed
/// `row![tree, right]`. Those arrive with #180.
///
/// `Replay` is here and not a pane, because `view/replay.rs` has no `pane()` at
/// all: it renders a banner between the toolbar and the panes, and `RightPane`
/// has no `Replay` variant. Adding one to make it fit would put a twelfth tab in
/// the strip and break `PANE_KEYS`' ten-digit arithmetic — a message reshape that
/// changes the toolbar has escaped its scope.
#[derive(Debug, Clone)]
pub enum WorkspaceMsg {
    ToggleNode(String),
    /// The tree pivot changed (issue #65).
    PivotSelected(crate::view::tree::Pivot),
    /// The find-in-tree query changed (issue #65).
    TreeSearchChanged(String),
    /// The tree scrolled: (absolute y offset, viewport height) — what the
    /// virtualized window renders against (issue #65).
    TreeScrolled(f32, f32),
    /// Switch the right-hand pane (the toolbar's tab strip).
    PaneSelected(RightPane),
    /// Open every prefix of a path so its subtree is visible, and reflatten.
    ///
    /// One message for what was the same eight-line loop written twice — in
    /// the nodes pane's "show in tree" and in the doctor's finding jump.
    Reveal(String),
    /// Replay-mode interactions (issue #74): open/scrub/play a `.zrec`,
    /// record the current watches to one.
    Replay(crate::view::replay::ReplayMsg),
}

/// The window, and what floats over it (#176).
///
/// `Palette` is here rather than under `Pane`: `palette::overlay` returns an
/// `Option<Element>` `stack!`ed above the whole layout, so it is neither a pane
/// nor a workspace region. The load-bearing reason was re-entrancy — `update_key`
/// called `update_palette` three times and pushed `shortcuts::resolve`'s output
/// back through `update` — and #175 replaced the last of those with
/// `Task::done`. The grouping stands on the weaker reason now, which is still a
/// reason: Esc and the arrows mean different things depending on what is open,
/// so the key and the overlay it disambiguates against belong together.
#[derive(Debug, Clone)]
pub enum ChromeMsg {
    /// A key press no widget consumed (issues #73, #75).
    ///
    /// Delivered raw rather than pre-resolved because Esc and the arrows mean
    /// different things depending on what is open, and iced's subscription
    /// closures must not capture — so the decision belongs in `update`, where
    /// the state is.
    Key(iced::keyboard::Key, iced::keyboard::Modifiers),
    /// Command-palette / overlay interactions (issue #75).
    Palette(crate::view::palette::PaletteMsg),
    /// A persisted-preference change (issue #73). Each one saves.
    Prefs(PrefsMsg),
    /// The window was resized — remembered for the next launch (issue #73).
    WindowResized(f32, f32),
    /// The resize settled: write the geometry once, rather than per pixel
    /// (issue #189).
    WindowSettled,
}

/// A message from one of the eleven right-hand panes (#176).
///
/// One variant per [`RightPane`], and **not** the `Pane(PaneId, PaneMsg)` pair
/// #176 first sketched: that makes 121 states representable where eleven are
/// legal — `Pane(PaneId::Call, PaneMsg::Blob(..))` would compile and mean
/// nothing. The pane identity is already the variant tag.
///
/// `Pane(..)` is a **provenance** tag — which surface emitted the message — not
/// a **scope** claim about what its handler may touch. Six of them mutate
/// app-wide state today and #176 deliberately left them:
///
/// * `NodesMsg::ShowInTree` — writes `expanded` and `selected`, reflattens
/// * `DoctorMsg::FindingClicked` — opens tree prefixes and re-enters a
///   selection, or flips `right_pane` and drives the nodes handler
/// * `DoctorMsg::ReaskSchemas` — clears the global schema store
/// * `AdminMsg::FilterProducer` — flips `right_pane`, re-emits a tree search
/// * `EchoMsg::LineClicked` — flips `right_pane`, re-enters a selection
/// * `ContextMsg::{SaveAndSelect, Isolate}` — reopens the session, rewrites
///   deployment settings
///
/// Rejected: promoting them to `Workspace`. They are emitted by a pane's own
/// widget, and a pane that constructs another region's message is the same
/// coupling wearing a different name.
#[derive(Debug, Clone)]
pub enum PaneMsg {
    /// Publish/call pane interactions (issue #60).
    Call(crate::view::call::CallMsg),
    /// Publish pane interactions (issue #60's other half).
    Publish(crate::view::publish::PublishMsg),
    /// Node dashboard interactions (issue #61).
    Nodes(crate::view::nodes::NodesMsg),
    /// Doctor panel interactions (issue #71).
    Doctor(crate::view::doctor::DoctorMsg),
    /// History pane interactions (issue #63).
    History(crate::view::history::HistoryMsg),
    /// Detail pane interactions (issue #64): which numeric leaf is plotted.
    Detail(crate::view::detail::DetailMsg),
    /// Blob browser interactions (issue #68).
    Blob(crate::view::blob::BlobMsg),
    /// Media viewer interactions (issue #69).
    Media(crate::view::media::MediaMsg),
    /// Admin & storage panel interactions (issue #70).
    Admin(crate::view::admin::AdminMsg),
    /// Echo pane interactions (issue #72, echo v2).
    Echo(crate::view::echo::EchoMsg),
    /// Connection pane interactions (issue #67).
    Context(crate::view::contexts::ContextMsg),
}

impl PaneMsg {
    /// Which pane emitted it.
    ///
    /// `RightPane::ALL` and this `match` are the same list, and
    /// `every_pane_has_a_message_and_every_message_a_pane` is what keeps them
    /// so — the same shape as the palette's and the shortcut map's own coverage
    /// tests.
    pub fn pane(&self) -> RightPane {
        match self {
            PaneMsg::Echo(_) => RightPane::Echo,
            PaneMsg::Call(_) => RightPane::Call,
            PaneMsg::Publish(_) => RightPane::Publish,
            PaneMsg::Detail(_) => RightPane::Detail,
            PaneMsg::Nodes(_) => RightPane::Nodes,
            PaneMsg::Doctor(_) => RightPane::Doctor,
            PaneMsg::History(_) => RightPane::History,
            PaneMsg::Blob(_) => RightPane::Blob,
            PaneMsg::Media(_) => RightPane::Media,
            PaneMsg::Admin(_) => RightPane::Admin,
            PaneMsg::Context(_) => RightPane::Connect,
        }
    }
}

/// Everything that can move the app.
///
/// `Clone` is required by iced's widget callbacks. Every payload here is
/// cheap to clone: `Session`, `Arc<…>` and `ZBytes` are all refcounted.
#[derive(Debug, Clone)]
pub enum Message {
    /// One of the eleven right-hand panes.
    Pane(PaneMsg),
    /// The window, and what floats over it.
    Chrome(ChromeMsg),
    /// The shell around the panes, and the replay mode.
    Workspace(WorkspaceMsg),
    /// One key: chosen, observed, fetched, decoded.
    Subject(SubjectMsg),
    /// What the app is pointed at, and the coverage that follows.
    Deployment(DeploymentMsg),
    /// The bus, the monitor, a sweep: something the world answered.
    Bus(BusMsg),
}

/// What a user can change about the window itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrefsMsg {
    ThemeToggled,
    ZoomIn,
    ZoomOut,
    ZoomReset,
}

/// What one prepare→declare→send round produced. The publication travels in
/// the message because a *repeating* publish keeps it: `Publication` is not
/// `Clone`, so the `Arc` is what lets the app hold the declared publisher
/// between ticks. A one-shot send undeclares in the task and carries `None` —
/// an explorer does not leave declarations lying around on the bus.
#[derive(Debug)]
pub struct PublishOutcome {
    pub prepared: zenkey_fleet::PreparedBody,
    pub publication: Option<Arc<zenkey_fleet::Publication>>,
    /// `None` = the status could not be asked, which is not `false` (O4).
    pub matching: Option<bool>,
    /// The attachment that rode the send, kept so a repeating publish
    /// resends it (#117).
    pub attachment: Option<Arc<Vec<u8>>>,
}

/// The right-hand pane switch — a tab strip, not a cycle, because the pane
/// set grows with the epic (#61 nodes, #71 doctor, #60 publish).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RightPane {
    Echo,
    Call,
    Publish,
    Detail,
    Nodes,
    Doctor,
    /// Per-key history and payload diff (issue #63).
    History,
    /// The `@blob` plane: who serves bulk content, and fetching it (issue #68).
    Blob,
    /// The `@media` plane: declared streams, and viewing one (issue #69).
    Media,
    /// Routers, storages and the state-coverage table (issue #70).
    Admin,
    /// Contexts and endpoints (issue #67).
    Connect,
}

impl RightPane {
    /// Every pane, in tab order — the strip iterates this, so a new variant
    /// cannot be forgotten in the toolbar.
    pub const ALL: [RightPane; 11] = [
        RightPane::Echo,
        RightPane::Call,
        RightPane::Publish,
        RightPane::Detail,
        RightPane::Nodes,
        RightPane::Doctor,
        RightPane::History,
        RightPane::Blob,
        RightPane::Media,
        RightPane::Admin,
        RightPane::Connect,
    ];

    pub fn label(self) -> &'static str {
        match self {
            RightPane::Echo => "echo",
            RightPane::Call => "call",
            RightPane::Publish => "publish",
            RightPane::Detail => "detail",
            RightPane::Nodes => "nodes",
            RightPane::Doctor => "doctor",
            RightPane::History => "history",
            RightPane::Blob => "blobs",
            RightPane::Media => "media",
            RightPane::Admin => "admin",
            RightPane::Connect => "connect",
        }
    }
}

/// What the link is doing, so the UI never has to infer it from emptiness.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkState {
    Connecting,
    /// The event pump is running; coverage comes from [`BusTick::watched`].
    Pumping,
    /// The stream ended; `Subscription::run_with` will restart it.
    Ended,
    Failed(String),
}

/// One stats-tick's worth of bus activity, coalesced.
///
/// The monitor ticks at 250 ms, so this is ~4 messages/second **regardless of
/// sample rate** — the single most important perf property of the bridge. A
/// per-sample `Message` would melt the Elm loop.
#[derive(Debug)]
pub struct BusTick {
    /// Pulled from the monitor's `ArcSwap` on the tick — never accumulated
    /// from samples. This is what makes a hot bus unable to melt the render
    /// loop (`zenkey-fleet/src/tree.rs`).
    pub tree: Arc<KeyTreeSnapshot>,
    /// Samples observed during this tick, capped.
    pub samples: Vec<Arc<SampleView>>,
    /// Samples the bounded broadcast dropped before we saw them.
    pub lagged: u64,
    /// Samples *we* dropped because the per-tick batch was full. Distinct from
    /// `lagged`: this is our own cap, and it is reported rather than hidden.
    pub coalesced: u64,
    /// Liveliness transitions: `(token key, is_up)`.
    pub nodes: Vec<(String, bool)>,
    /// Distinct keys the monitor is *currently* tracking. Not cumulative:
    /// the table is bounded, and the counters below say what that costs.
    pub keys: usize,
    /// Keys retired to stay within the table's bound (RFC 09 §5.1 O6).
    pub keys_evicted: u64,
    /// Keys retired because their watch was released — "stopped looking, by
    /// request", the third O6 category.
    pub keys_unwatched: u64,
    /// The active watch selectors — the coverage statement (O5).
    ///
    /// `Arc<[String]>` rather than `Vec<String>`: the list changes when a
    /// watch is added or released, and it was being cloned on every tick and
    /// then cloned *again* into `Zengui::watched` — twice a tick, four times a
    /// second, for a list that is usually identical to the last one (#178).
    pub watched: Arc<[String]>,
    /// Seed boundaries that fired during this tick (issue #92): each seeded
    /// watch's id and what its seed paths contributed.
    pub seeded: Vec<(WatchId, zenkey_fleet::SeedCoverage)>,
    /// `(samples, bytes, rate_hz)` across everything watched.
    pub totals: (u64, u64, f64),
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The pane vocabulary is one list, spelled twice — `RightPane::ALL` and
    /// `PaneMsg`'s variants — and this is what keeps them the same list (#176).
    ///
    /// Same shape as `the_palette_covers_every_pane_without_being_told` and
    /// `the_pane_bindings_cover_every_pane`: a tab that exists and cannot be
    /// spoken to, or a message from a pane that is not in the strip, is the
    /// failure all three guard.
    #[test]
    fn every_pane_has_a_message_and_every_message_a_pane() {
        use crate::view;
        let one_per_pane = [
            PaneMsg::Echo(view::echo::EchoMsg::Clear),
            PaneMsg::Call(view::call::CallMsg::Submit),
            PaneMsg::Publish(view::publish::PublishMsg::Send),
            PaneMsg::Detail(view::detail::DetailMsg::LeafSelected(String::new())),
            PaneMsg::Nodes(view::nodes::NodesMsg::Selected(String::new())),
            PaneMsg::Doctor(view::doctor::DoctorMsg::Run),
            PaneMsg::History(view::history::HistoryMsg::Clear),
            PaneMsg::Blob(view::blob::BlobMsg::Probe),
            PaneMsg::Media(view::media::MediaMsg::Stop),
            PaneMsg::Admin(view::admin::AdminMsg::Run),
            PaneMsg::Context(view::contexts::ContextMsg::Load),
        ];
        let mut covered: Vec<RightPane> = one_per_pane.iter().map(PaneMsg::pane).collect();
        covered.sort_by_key(|p| p.label());
        let mut all = RightPane::ALL.to_vec();
        all.sort_by_key(|p| p.label());
        assert_eq!(
            covered, all,
            "every pane in the strip owes `PaneMsg` a variant, and vice versa"
        );
    }
}

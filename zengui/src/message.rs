//! The whole event vocabulary, in one file so the async/sync boundary is
//! readable at a glance.

use std::sync::Arc;

use zenkey_fleet::{
    DiscoveredBase, FetchOutcome, KeyTreeSnapshot, Monitor, SampleView, Skeleton, SliceSet, WatchId,
};

use crate::scope::ScopePreset;

/// The engine roster's shape: origin → live producer names.
pub type LiveRoster = std::collections::BTreeMap<String, Vec<String>>;

/// Everything that can move the app.
///
/// `Clone` is required by iced's widget callbacks. Every payload here is
/// cheap to clone: `Session`, `Arc<…>` and `ZBytes` are all refcounted.
#[derive(Debug, Clone)]
pub enum Message {
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

    /// The user toggled observation of one subtree (the tree's watch button).
    /// Carries the row's display path.
    WatchToggled(String),
    /// A watch was declared for the given display path.
    WatchStarted(String, Result<WatchId, String>),
    /// A watch was released for the given display path.
    WatchReleased(String, Result<(), String>),
    /// The scope preset's watches were declared (the eager set).
    ScopeWatchesStarted(Vec<WatchId>),
    /// Apply/release the scope preset's selectors as watches — the eager
    /// mode, made explicit and labelled by its cost.
    ScopeWatchToggled,
    /// A value arrived for the selected key ([`zenkey_fleet::fetch_value`]).
    ValueFetched(String, Result<Arc<FetchOutcome>, String>),
    /// The fetched value's schema decode finished (§6.4 item 5's inspector):
    /// (key, declared type if any, rendering).
    ValueDecoded(String, Option<String>, Arc<zenkey_fleet::decode::Rendering>),

    /// Publish/call pane interactions (issue #60).
    Call(crate::view::call::CallMsg),
    /// A call finished.
    CallDone(Result<Arc<zenkey_fleet::report::CallReport>, String>),
    /// Node dashboard interactions (issue #61).
    Nodes(crate::view::nodes::NodesMsg),
    /// Doctor panel interactions (issue #71).
    Doctor(crate::view::doctor::DoctorMsg),

    BaseSelected(String),
    ScopeSelected(ScopePreset),
    ToggleNode(String),
    SelectKey(Option<String>),
    /// The tree pivot changed (issue #65).
    PivotSelected(crate::view::tree::Pivot),
    /// The find-in-tree query changed (issue #65).
    TreeSearchChanged(String),
    /// The tree scrolled: (absolute y offset, viewport height) — what the
    /// virtualized window renders against (issue #65).
    TreeScrolled(f32, f32),
    EchoFilterChanged(String),
    ClearEcho,
    /// Switch the right-hand pane (the toolbar's tab strip).
    PaneSelected(RightPane),
    Reconnect,
}

/// The right-hand pane switch — a tab strip, not a cycle, because the pane
/// set grows with the epic (#61 nodes, #71 doctor).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RightPane {
    Echo,
    Call,
    Detail,
    Nodes,
    Doctor,
}

impl RightPane {
    /// Every pane, in tab order — the strip iterates this, so a new variant
    /// cannot be forgotten in the toolbar.
    pub const ALL: [RightPane; 5] = [
        RightPane::Echo,
        RightPane::Call,
        RightPane::Detail,
        RightPane::Nodes,
        RightPane::Doctor,
    ];

    pub fn label(self) -> &'static str {
        match self {
            RightPane::Echo => "echo",
            RightPane::Call => "call",
            RightPane::Detail => "detail",
            RightPane::Nodes => "nodes",
            RightPane::Doctor => "doctor",
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
    pub watched: Vec<String>,
    /// Seed boundaries that fired during this tick (issue #92): each seeded
    /// watch's id and what its seed paths contributed.
    pub seeded: Vec<(WatchId, zenkey_fleet::SeedCoverage)>,
    /// `(samples, bytes, rate_hz)` across everything watched.
    pub totals: (u64, u64, f64),
}

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

/// What the whole workspace is looking at (#181).
///
/// One value, read by every pane, rather than a `selected` here and a
/// `node_selected` there and each pane's private idea of its own target. The
/// panes *read* this; nothing tells them about it.
///
/// ## Why these four and not the six the issue sketched
///
/// Every variant below is reachable by an interaction that exists today. The
/// issue proposed `Producer`, `Router`, `Storage` and `Bus` as well — but
/// nothing in the app can select a router or a storage (the admin pane has no
/// selection at all), and an unreachable variant is a state the compiler makes
/// every `match` handle and no test can reach. They arrive with the pane that
/// can construct them.
///
/// It also proposed `Key(PathId)`, and `PathId` is #251's path arena, which
/// #177 deliberately deferred. A `String` until then.
///
/// ## What one subject does not yet do
///
/// Pinning — a pane detaching to hold a *second* subject — is #257. It is not
/// a second `Option<String>`: six fields of the subject sub-state are derived
/// from the subject and rebuilt when it moves, so a real pin needs a second
/// recorder fed by the same tick, a second fetch and a second decode. Freezing only the identity
/// would leave a pane titled with one key while its chart described another,
/// which is the failure [`Fetched::Superseded`](crate::view::detail::Fetched)
/// exists to prevent, one panel over.
///
/// ## Why `Prefix` is separate from `Key`, which the issue did not have
///
/// A prefix is not a key. `Key` heads a causal chain that ends in a
/// `fetch_value` GET; routing "show this origin in the tree" through it would
/// query something no producer publishes — an unasked-for bus query, which is
/// what #85's laziness rule forbids. The distinction is load-bearing, and the
/// type is where it belongs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Subject {
    /// Nothing chosen. The panes say so rather than showing the last thing.
    #[default]
    None,
    /// One concrete wire key: fetchable, recordable, plottable.
    Key(String),
    /// A subtree prefix — highlighted in the tree, never fetched.
    Prefix(String),
    /// One origin from the liveliness roster: a host or a service token.
    Origin(String),
}

impl Subject {
    /// The concrete key, if the subject is one.
    ///
    /// The fetch, the history recorder and the latency lookup all go through
    /// this, so "is this fetchable?" is asked once, by the type.
    pub fn key(&self) -> Option<&str> {
        match self {
            Subject::Key(k) => Some(k),
            _ => None,
        }
    }

    /// What the key tree should highlight: a key or a prefix, both of which
    /// are display paths.
    pub fn path(&self) -> Option<&str> {
        match self {
            Subject::Key(p) | Subject::Prefix(p) => Some(p),
            _ => None,
        }
    }

    /// The origin, if the subject is one.
    pub fn origin(&self) -> Option<&str> {
        match self {
            Subject::Origin(o) => Some(o),
            _ => None,
        }
    }
}

/// One subject: chosen, observed, fetched, decoded (#176).
///
/// `Select` heads a causal chain — its own handler ends in the
/// `Task::perform` that produces `ValueFetched`, which produces `ValueDecoded` —
/// so filing the head under the workspace and the tail here would force
/// `update::subject` to re-enter `update::workspace` to do its own job.
///
/// `ValueFetched`/`ValueDecoded` are deliberately *not* folded into `DetailMsg`,
/// which the async-result rule might seem to demand. They are not a pane's
/// result: the fetch is issued by the **subject**, not by a pane, and a pane
/// owning a fetch it never requested would be a worse lie than the placement it
/// replaced.
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
    /// Point the whole workspace at something (#181).
    ///
    /// One message where there were three — `SelectKey`, `SelectPath` and the
    /// nodes pane's own `Selected`. What happens next is the [`Subject`]'s
    /// business, not the caller's: a `Key` fetches, an `Origin` asks for its
    /// `node_info`, a `Prefix` does neither.
    Select(Subject),
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
    /// Inspector history-section interactions (issue #63).
    History(crate::view::history::HistoryMsg),
    /// Inspector detail-section interactions (issue #64): which numeric leaf
    /// is plotted.
    Detail(crate::view::detail::DetailMsg),
    /// Inspector `@blob`-section interactions (issue #68).
    Blob(crate::view::blob::BlobMsg),
    /// Inspector `@media`-section interactions (issue #69).
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
    ///
    /// It is no longer a bijection, and #182 is why: `Detail`, `History`,
    /// `Blob` and `Media` are four *sections* of one surface now, so four
    /// variants map to `Inspector`. The relation the test still pins is
    /// coverage in both directions — every pane owes a message, every message
    /// names a pane — which is what it was for.
    pub fn pane(&self) -> RightPane {
        match self {
            PaneMsg::Echo(_) => RightPane::Echo,
            PaneMsg::Call(_) => RightPane::Call,
            PaneMsg::Publish(_) => RightPane::Publish,
            PaneMsg::Detail(_) | PaneMsg::History(_) | PaneMsg::Blob(_) | PaneMsg::Media(_) => {
                RightPane::Inspector
            }
            PaneMsg::Nodes(_) => RightPane::Nodes,
            PaneMsg::Doctor(_) => RightPane::Doctor,
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
    /// One surface that follows the subject (#182): the key facts, the value
    /// and its decode, the history diff, the chart — and the `@blob` or
    /// `@media` sections when the key is on one of those planes.
    ///
    /// It replaces four tabs. `Detail`, `History`, `Blob` and `Media` each
    /// answered part of one question about one thing, so switching between
    /// them was the user assembling by hand what the Inspector assembles.
    Inspector,
    Nodes,
    Doctor,
    /// Routers, storages and the state-coverage table (issue #70).
    Admin,
    /// Contexts and endpoints (issue #67).
    Connect,
}

impl RightPane {
    /// Every pane, in tab order — the strip iterates this, so a new variant
    /// cannot be forgotten in the toolbar.
    pub const ALL: [RightPane; 8] = [
        RightPane::Echo,
        RightPane::Call,
        RightPane::Publish,
        RightPane::Inspector,
        RightPane::Nodes,
        RightPane::Doctor,
        RightPane::Admin,
        RightPane::Connect,
    ];

    pub fn label(self) -> &'static str {
        match self {
            RightPane::Echo => "echo",
            RightPane::Call => "call",
            RightPane::Publish => "publish",
            RightPane::Inspector => "inspector",
            RightPane::Nodes => "nodes",
            RightPane::Doctor => "doctor",
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
    /// The #85 rule, as a property of the type rather than of a comment.
    ///
    /// `key()` is what the fetch, the history recorder and the latency lookup
    /// all go through. A prefix answering it would put a `fetch_value` GET on
    /// the wire against something no producer publishes — the exact bug the
    /// `SelectKey`/`SelectPath` split existed to prevent, now unrepresentable
    /// rather than merely avoided at each call site.
    #[test]
    fn only_a_key_is_fetchable_and_a_prefix_is_still_a_path() {
        let key = Subject::Key("v1/h-3fa9c2d41b7e/state/sysinfo/health".into());
        let prefix = Subject::Prefix("v1/h-3fa9c2d41b7e".into());
        let origin = Subject::Origin("h-3fa9c2d41b7e".into());

        assert_eq!(key.key(), Some("v1/h-3fa9c2d41b7e/state/sysinfo/health"));
        assert_eq!(prefix.key(), None, "a prefix must never be fetched");
        assert_eq!(origin.key(), None);
        assert_eq!(Subject::None.key(), None);

        // Both are display paths, and the tree highlights either.
        assert_eq!(key.path(), Some("v1/h-3fa9c2d41b7e/state/sysinfo/health"));
        assert_eq!(prefix.path(), Some("v1/h-3fa9c2d41b7e"));
        assert_eq!(origin.path(), None, "an origin id is not a display path");

        assert_eq!(origin.origin(), Some("h-3fa9c2d41b7e"));
        assert_eq!(key.origin(), None);
    }

    #[test]
    fn every_pane_has_a_message_and_every_message_a_pane() {
        use crate::view;
        let one_per_pane = [
            PaneMsg::Echo(view::echo::EchoMsg::Clear),
            PaneMsg::Call(view::call::CallMsg::Submit),
            PaneMsg::Publish(view::publish::PublishMsg::Send),
            PaneMsg::Detail(view::detail::DetailMsg::LeafSelected(String::new())),
            PaneMsg::Nodes(view::nodes::NodesMsg::ShowInTree(String::new())),
            PaneMsg::Doctor(view::doctor::DoctorMsg::Run),
            PaneMsg::History(view::history::HistoryMsg::Clear),
            PaneMsg::Blob(view::blob::BlobMsg::Probe),
            PaneMsg::Media(view::media::MediaMsg::Stop),
            PaneMsg::Admin(view::admin::AdminMsg::Run),
            PaneMsg::Context(view::contexts::ContextMsg::Load),
        ];
        // Coverage in both directions, not a bijection — #182 made four
        // variants sections of the Inspector, so `Detail`, `History`, `Blob`
        // and `Media` all name it. What must stay true is that no pane in the
        // strip is unreachable by message, and no message names a pane that
        // is not in the strip.
        let mut covered: Vec<RightPane> = one_per_pane.iter().map(PaneMsg::pane).collect();
        covered.sort_by_key(|p| p.label());
        covered.dedup();
        let mut all = RightPane::ALL.to_vec();
        all.sort_by_key(|p| p.label());
        assert_eq!(
            covered, all,
            "every pane in the strip owes `PaneMsg` a variant, and vice versa"
        );

        // And the folding is itself the claim: exactly four variants name the
        // Inspector, which is the four tabs it replaced.
        let folded = one_per_pane
            .iter()
            .filter(|m| m.pane() == RightPane::Inspector)
            .count();
        assert_eq!(
            folded, 4,
            "Detail, History, Blob and Media are the Inspector's sections"
        );
    }
}

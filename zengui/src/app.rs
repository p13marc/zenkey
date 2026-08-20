//! The Elm loop: state, `update`, `view`, `subscription`.
//!
//! **Lazy by default** (issue #85): connecting builds the *skeleton* — the
//! declared keyspace from registry + liveliness + admin metadata — and starts
//! a monitor with **zero data-plane watches**. Observation is opt-in per
//! subtree (the tree's watch toggles) or per scope (the toolbar's "observe
//! scope" toggle — the old eager mode made explicit); a selection fetches one
//! value on demand. `--eager` restores the bootstrap behavior from the
//! command line, labelled by its cost.

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use iced::widget::{column, pick_list, row, text};
use iced::{Element, Length, Subscription, Task};
use zenkey_fleet::{
    DiscoveredBase, FetchOutcome, KeyTreeSnapshot, Monitor, MonitorSpec, Skeleton, SliceSet,
    WatchId,
};

use crate::config::Settings;
use crate::echo::EchoRing;
use crate::link::{self, LinkKey};
use crate::message::{BusTick, LinkState, Message};
use crate::scope::{self, ScopePreset};
use crate::view;
use crate::view::status::{SliceSource, Status};
use crate::view::tokens::space;

/// Cap on tree rows built per flatten. With virtualized rendering (issue
/// #65) this is a memory bound, not a display truncation — the view builds
/// only the scrolled-into window.
const MAX_ROWS: usize = 50_000;

use crate::message::RightPane;

/// What an armed repeating publication resends each tick: the declaration,
/// the prepared bytes, and the attachment that rode the first send (#117).
struct RepeatLoad {
    publication: Arc<zenkey_fleet::Publication>,
    bytes: Arc<Vec<u8>>,
    attachment: Option<Arc<Vec<u8>>>,
}

pub struct Zengui {
    settings: Settings,
    session: Option<zenoh::Session>,
    monitor: Option<Arc<Monitor>>,
    link: LinkState,
    /// Bumped per monitor generation — the pump's identity.
    epoch: u64,

    observed: Arc<KeyTreeSnapshot>,
    skeleton: Option<Arc<Skeleton>>,
    /// Active watch selectors, as the last tick reported them (coverage, O5).
    watched: Vec<String>,
    /// This app's per-subtree watches: display path → watch id.
    my_watches: HashMap<String, WatchId>,
    /// The same key set, cached for the view (a borrowed pane cannot borrow
    /// a per-frame local).
    my_watch_paths: BTreeSet<String>,
    /// The scope-preset watch ids, when "observe scope" is on.
    scope_watches: Vec<WatchId>,
    /// Watches whose seed phase has not resolved yet (issue #92):
    /// id → the tree display path when the watch came from a tree toggle
    /// (`None` for scope watches, whose selectors are not tree paths).
    seeding: HashMap<WatchId, Option<String>>,
    /// The same tree paths as a set, for the tree's "seeding…" badges.
    seeding_paths: BTreeSet<String>,
    /// Cumulative seed coverage since connect: (cache replies, storage
    /// replies, superseded) — zeros are observations (O4).
    seed_totals: (usize, usize, u64),
    /// Seed phases completed since connect.
    seeded_watches: usize,
    flat: view::tree::Flattened,
    /// How the tree groups its entries (issue #65).
    pivot: view::tree::Pivot,
    /// The find-in-tree query; empty = no filter.
    tree_search: String,
    /// Scroll position + viewport height, driving the virtual window.
    tree_scroll: (f32, f32),
    echo: EchoRing,
    expanded: BTreeSet<String>,
    selected: Option<String>,
    /// The selected key's history recording (issue #63). Created on selection,
    /// dropped on the next one — which is what makes deselecting stop the
    /// cost, since there is then nothing left to feed.
    history: Option<crate::history::HistoryRecorder>,
    /// The selected key's rate series (issue #64), sampled once per stats
    /// tick. Reset with the selection, like the history it sits beside.
    rate_series: crate::series::RateSampler,
    /// Which numeric leaf the value sparkline plots; `None` follows the first
    /// leaf the payload offers.
    series_leaf: Option<String>,
    /// The last on-demand fetch: (key, outcome-or-error).
    fetched: Option<(String, Result<Arc<FetchOutcome>, String>)>,
    /// The echo pane's view state (issue #72): filters, follow-tail, gaps.
    echo_view: view::echo::EchoView,
    /// The connection pane's state (issue #67): contexts and endpoints.
    context_form: view::contexts::ContextForm,
    /// The command palette / overlay state (issue #75).
    palette: view::palette::PaletteState,

    /// The node dashboard's presence model (#61), fed by liveliness only.
    roster: crate::nodes::NodeRoster,
    /// The selected origin in the nodes pane.
    node_selected: Option<String>,
    /// The selected key's observed skewed-latency summary, refreshed on the
    /// bus tick (#119) — never computed on the render path, and cleared
    /// with the selection.
    selected_latency: Option<(zenkey_fleet::LatencySummary, u64)>,
    /// Its one-shot `node_info` detail — the pane's only data-plane cost.
    node_detail: view::nodes::DetailState,
    /// The doctor panel's run state (#71) — run-on-demand only.
    doctor: crate::doctor::DoctorState,
    /// The blob browser's state (#68) — probe and fetch, both on demand.
    blob: crate::blob::BlobState,
    /// The admin & storage panel's state (#70) — swept on demand.
    admin: crate::admin::AdminState,
    /// The media viewer's state (#69) — subscribes only on an explicit
    /// view, never for opening the pane (the RFC 07 §1 plane deserves the
    /// laziest posture in the app).
    media: view::media::MediaState,

    call_form: view::call::CallForm,
    publish_form: view::publish::PublishForm,
    /// The armed publication and what it repeats (#60). Held here rather
    /// than in the form because a `Publication` is a live bus declaration, not
    /// view state — dropping it undeclares.
    publication: Option<RepeatLoad>,
    /// Process-lifetime schema cache (RFC 08 §7), rebuilt on base change.
    schema_store: Option<Arc<zenkey_fleet::decode::SchemaStore>>,
    /// The decode of the last fetched value.
    decoded: Option<(Option<String>, zenkey_fleet::decode::Rendering)>,
    /// Which right-hand pane is showing.
    right_pane: RightPane,
    /// Bounded, and it says what the bound costs (#107).
    facts: zenkey_fleet::FactsCache,
    slices: Option<Arc<SliceSet>>,
    slice_source: SliceSource,

    /// Persisted UI preferences (issue #73) — theme, zoom, geometry, and the
    /// scope/context the window was last on.
    prefs: crate::prefs::Prefs,
    /// Geometry changed and has not been written yet (issue #189). Drives the
    /// settle timer, so a drag writes the file once rather than per pixel.
    window_dirty: bool,
    /// Why the defaults are in force, when a prefs file could not be read.
    /// Rendered once in the status strip; never a reason to refuse to open.
    prefs_note: Option<String>,

    bases: Vec<DiscoveredBase>,
    keys: usize,
    keys_evicted: u64,
    keys_unwatched: u64,
    totals: (u64, u64, f64),

    /// Replay mode (issue #74): while `Some`, the panes are fed from the
    /// file and the live link subscription is not built at all — nothing in
    /// replay can publish or subscribe, structurally.
    replay: Option<crate::replay::ReplayState>,
    /// The open row's path input; `None` = row hidden.
    replay_open: Option<String>,
    /// Why the last open failed, shown beside the path box.
    replay_note: Option<String>,
    /// A capture in flight (the toolbar's record toggle): the stop signal
    /// and where it is writing.
    recording: Option<RecordingHandle>,
    /// The last finished capture, for the status strip: (samples, dropped,
    /// path) or the failure.
    recorded: Option<Result<(u64, u64, String), String>>,
}

/// A running toolbar capture (#74): dropping the notify without firing it
/// would leak the task, so `stop` is fired on toggle-off and on exit.
struct RecordingHandle {
    stop: Arc<tokio::sync::Notify>,
    path: String,
}

impl Zengui {
    pub fn new(settings: Settings) -> (Zengui, Task<Message>) {
        let (prefs, prefs_note) = crate::prefs::Prefs::load();
        Self::with_prefs(settings, prefs, prefs_note)
    }

    /// The pure constructor: preferences are injected rather than read, so a
    /// test never touches the user's real file.
    pub fn with_prefs(
        settings: Settings,
        prefs: crate::prefs::Prefs,
        prefs_note: Option<String>,
    ) -> (Zengui, Task<Message>) {
        let echo = EchoRing::new(settings.echo_lines);
        // Read before `settings` is moved into the struct. The projection
        // cache takes the *same* bound as the engine's key table: it cannot
        // usefully outgrow the table it shadows, and one number keeps that
        // one sentence (#107).
        let max_keys = settings.max_keys;
        let connect = settings.connect.clone();
        let listen = settings.listen.clone();
        let scouting = settings.scouting;
        let zenoh_config = settings.zenoh_config.clone();
        let app = Zengui {
            settings,
            session: None,
            monitor: None,
            link: LinkState::Connecting,
            epoch: 0,
            observed: Arc::new(KeyTreeSnapshot::default()),
            skeleton: None,
            watched: Vec::new(),
            my_watches: HashMap::new(),
            my_watch_paths: BTreeSet::new(),
            scope_watches: Vec::new(),
            seeding: HashMap::new(),
            seeding_paths: BTreeSet::new(),
            seed_totals: (0, 0, 0),
            seeded_watches: 0,
            flat: view::tree::Flattened::empty(),
            pivot: view::tree::Pivot::default(),
            tree_search: String::new(),
            tree_scroll: (0.0, 600.0),
            echo,
            expanded: BTreeSet::new(),
            selected: None,
            history: None,
            rate_series: crate::series::RateSampler::new(),
            series_leaf: None,
            fetched: None,
            echo_view: view::echo::EchoView::new(),
            context_form: view::contexts::ContextForm::default(),
            palette: view::palette::PaletteState::default(),
            roster: crate::nodes::NodeRoster::default(),
            node_selected: None,
            selected_latency: None,
            node_detail: view::nodes::DetailState::default(),
            doctor: crate::doctor::DoctorState::default(),
            blob: crate::blob::BlobState::default(),
            admin: crate::admin::AdminState::default(),
            media: view::media::MediaState::default(),
            call_form: view::call::CallForm::default(),
            publish_form: view::publish::PublishForm::default(),
            publication: None,
            schema_store: None,
            decoded: None,
            right_pane: RightPane::Echo,
            facts: zenkey_fleet::FactsCache::with_capacity(max_keys),
            slices: None,
            slice_source: SliceSource::None,
            prefs,
            prefs_note,
            window_dirty: false,
            bases: Vec::new(),
            keys: 0,
            keys_evicted: 0,
            keys_unwatched: 0,
            totals: (0, 0, 0.0),
            replay: None,
            replay_open: None,
            replay_note: None,
            recording: None,
            recorded: None,
        };
        let open = Task::perform(
            async move {
                zenkey_fleet::open_with_config(zenoh_config.as_deref(), &connect, &listen, scouting)
                    .await
                    .map_err(|e| e.to_string())
            },
            Message::SessionOpened,
        );
        (app, open)
    }

    pub fn title(&self) -> String {
        format!("zengui — {}", self.settings.base_label())
    }

    pub fn theme(&self) -> iced::Theme {
        self.prefs.theme.theme()
    }

    /// The UI scale factor iced applies to the whole window (issue #73).
    pub fn scale_factor(&self) -> f32 {
        self.prefs.zoom
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::SessionOpened(Ok(session)) => {
                self.session = Some(session.clone());
                // The context list is read from the shared file, not cached at
                // launch: `zenctl context create` on the other side of the
                // screen should show up here without a restart (#67).
                self.refresh_contexts();
                self.schema_store = Some(Arc::new(zenkey_fleet::decode::SchemaStore::new(
                    &self.settings.base,
                    self.settings.timeout(),
                )));
                let timeout = self.settings.timeout();
                let discover = Task::perform(
                    {
                        let s = session.clone();
                        async move {
                            zenkey_fleet::discover_bases(&s, timeout)
                                .await
                                .map_err(|e| e.to_string())
                        }
                    },
                    Message::BasesDiscovered,
                );
                Task::batch([discover, self.start_monitor(), self.load_slices()])
            }
            Message::SessionOpened(Err(e)) => {
                self.link = LinkState::Failed(e);
                Task::none()
            }
            Message::MonitorStarted(Ok(monitor)) => {
                self.monitor = Some(Arc::clone(&monitor));
                self.epoch += 1;
                if self.settings.eager {
                    return self.watch_scope();
                }
                Task::none()
            }
            Message::MonitorStarted(Err(e)) => {
                self.link = LinkState::Failed(e);
                Task::none()
            }
            Message::SkeletonBuilt(Ok((skeleton, roster))) => {
                self.skeleton = Some(skeleton);
                // The build task gathered the roster anyway — seed the node
                // dashboard from it instead of throwing it away (#61).
                self.roster.seed(&roster);
                self.reflatten();
                Task::none()
            }
            Message::SkeletonBuilt(Err(e)) => {
                tracing::warn!("skeleton build failed: {e}");
                Task::none()
            }
            Message::BasesDiscovered(Ok(bases)) => {
                self.bases = bases;
                Task::none()
            }
            Message::BasesDiscovered(Err(e)) => {
                tracing::warn!("base discovery failed: {e}");
                Task::none()
            }
            Message::SlicesLoaded(Ok(slices)) => {
                self.slice_source = if self.settings.registry.is_empty() {
                    SliceSource::Bus {
                        count: slices.slices().len(),
                    }
                } else {
                    SliceSource::Dirs {
                        count: slices.slices().len(),
                    }
                };
                self.slices = Some(slices);
                self.reresolve_registrations();
                self.refresh_blob_list();
                // The skeleton is built FROM the slices — (re)build it now.
                self.build_skeleton()
            }
            Message::SlicesUnionLoaded(Ok((slices, from_bus, dirs_only, disagreements))) => {
                self.slice_source = SliceSource::Union {
                    from_bus,
                    dirs_only,
                    disagreements,
                };
                self.slices = Some(slices);
                self.reresolve_registrations();
                self.refresh_blob_list();
                self.build_skeleton()
            }
            Message::SlicesUnionLoaded(Err(e)) => {
                self.slice_source = SliceSource::Failed(e);
                Task::none()
            }
            Message::SlicesLoaded(Err(e)) => {
                self.slice_source = SliceSource::Failed(e);
                Task::none()
            }
            Message::Link(state) => {
                self.link = state;
                Task::none()
            }
            Message::Tick(tick) => {
                self.apply_tick(&tick);
                Task::none()
            }
            Message::WatchToggled(path) => self.toggle_watch(path),
            Message::WatchStarted(path, Ok(id)) => {
                self.my_watch_paths.insert(path.clone());
                self.seeding.insert(id, Some(path.clone()));
                self.seeding_paths.insert(path.clone());
                self.my_watches.insert(path, id);
                self.reflatten();
                Task::none()
            }
            Message::WatchStarted(path, Err(e)) => {
                tracing::warn!("watch {path} failed: {e}");
                Task::none()
            }
            Message::ScopeWatchesStarted(ids) => {
                for id in &ids {
                    self.seeding.insert(*id, None);
                }
                self.scope_watches = ids;
                Task::none()
            }
            Message::WatchReleased(_, Ok(())) => Task::none(),
            Message::WatchReleased(path, Err(e)) => {
                tracing::warn!("unwatch {path} failed: {e}");
                Task::none()
            }
            Message::ScopeWatchToggled => {
                if self.scope_watches.is_empty() {
                    self.watch_scope()
                } else {
                    self.unwatch_scope()
                }
            }
            Message::ValueFetched(key, outcome) => {
                // No base guard needed (#109 audit): the evidence is keyed by
                // the full wire key, which names its own base (an explorer
                // runs un-namespaced, RFC 09 §5), and the view passes
                // `fetched` through only while that exact key is selected.
                // Residual: a stale landing can still flip the right pane to
                // Detail — a focus nit, not a misattributed verdict.
                self.decoded = None;
                // A fetch normally lands the detail pane in view — except
                // from the doctor's click-through, where losing the finding
                // list would cost more than it shows (#71).
                if self.right_pane != RightPane::Doctor {
                    self.right_pane = RightPane::Detail;
                }
                let decode_task = match (&outcome, &self.session, &self.schema_store, &self.slices)
                {
                    (Ok(out), Some(session), Some(store), Some(slices)) => {
                        if let zenkey_fleet::FetchOutcome::Value(v) = out.as_ref() {
                            let (session, store, slices) =
                                (session.clone(), Arc::clone(store), Arc::clone(slices));
                            let base = self.settings.base.clone();
                            let (fkey, wire_key) = (key.clone(), v.key.clone());
                            let encoding = v.encoding.clone();
                            let bytes = v.payload.clone();
                            // decode_sample is async (may fetch describe on
                            // first miss) — a Task, never the render path.
                            Task::perform(
                                async move {
                                    let d = zenkey_fleet::decode::decode_sample(
                                        &store,
                                        &session,
                                        &slices,
                                        &base,
                                        &wire_key,
                                        Some(&encoding),
                                        &bytes.to_bytes(),
                                    )
                                    .await;
                                    // The verdict rides the sample (#159); the
                                    // detail pane learns to render it in #164.
                                    (fkey, d.type_name, Arc::new(d.rendering))
                                },
                                |(k, t, r)| Message::ValueDecoded(k, t, r),
                            )
                        } else {
                            Task::none()
                        }
                    }
                    _ => Task::none(),
                };
                self.fetched = Some((key, outcome));
                decode_task
            }
            Message::ValueDecoded(key, type_name, rendering) => {
                // Stale guard: only the currently selected key's decode lands.
                if self.selected.as_deref() == Some(key.as_str()) {
                    self.decoded = Some((type_name, (*rendering).clone()));
                }
                Task::none()
            }
            Message::BaseSelected(base) => {
                if base == self.settings.base {
                    return Task::none();
                }
                self.settings.base = base;
                // The base is an input to every projection and to the
                // skeleton, and watch selectors are base-relative: a fresh
                // monitor is the obviously-correct restart. Everything the old
                // base taught us is evidence about a different deployment (O4).
                self.forget_deployment();
                self.schema_store = Some(Arc::new(zenkey_fleet::decode::SchemaStore::new(
                    &self.settings.base,
                    self.settings.timeout(),
                )));
                self.decoded = None;
                self.reflatten();
                Task::batch([self.start_monitor(), self.load_slices()])
            }
            Message::ScopeSelected(scope) => {
                if scope == self.settings.scope {
                    return Task::none();
                }
                self.settings.scope = scope;
                // Remembered for the next launch (issue #73).
                self.remember();
                // If the scope is being observed, re-point the observation.
                if !self.scope_watches.is_empty() {
                    let release = self.unwatch_scope();
                    let acquire = self.watch_scope();
                    return Task::batch([release, acquire]);
                }
                Task::none()
            }
            Message::PivotSelected(pivot) => {
                self.pivot = pivot;
                self.tree_scroll.0 = 0.0;
                self.reflatten();
                Task::none()
            }
            Message::TreeSearchChanged(q) => {
                self.tree_search = q;
                self.tree_scroll.0 = 0.0;
                self.reflatten();
                Task::none()
            }
            Message::TreeScrolled(y, h) => {
                // View-only state: the next frame renders the new window.
                self.tree_scroll = (y, h.max(100.0));
                Task::none()
            }
            Message::ToggleNode(path) => {
                if !self.expanded.remove(&path) {
                    self.expanded.insert(path);
                }
                self.reflatten();
                Task::none()
            }
            Message::SelectKey(key) => {
                self.selected = key.clone();
                // The old key's latency summary is not evidence about the
                // new one — cleared now, refreshed on the next tick (#119).
                self.selected_latency = None;
                // History follows the selection and nothing else (issue #63):
                // the previous recording is dropped here, which is what makes
                // deselecting free. A symbolic skeleton path names no concrete
                // key, so nothing can be recorded for it.
                self.history = key.as_deref().filter(|k| !k.contains('{')).map(|k| {
                    crate::history::HistoryRecorder::new(k, self.settings.history_entries)
                });
                // The plotted series belong to the same selection (issue #64):
                // they start empty, and stop being fed when it goes away.
                self.rate_series = crate::series::RateSampler::new();
                self.series_leaf = None;
                let (Some(session), Some(key)) = (self.session.clone(), key) else {
                    return Task::none();
                };
                // Lazy value-on-demand: one fetch per selection, nothing
                // ambient (issue #85). Symbolic skeleton paths have no
                // concrete value to fetch.
                if key.contains('{') {
                    return Task::none();
                }
                Task::perform(
                    async move {
                        let out = zenkey_fleet::fetch_value(
                            &session,
                            &key,
                            zenkey_fleet::FetchSpec::default(),
                        )
                        .await
                        .map(Arc::new)
                        .map_err(|e| e.to_string());
                        (key, out)
                    },
                    |(key, out)| Message::ValueFetched(key, out),
                )
            }
            Message::Call(msg) => self.update_call(msg),
            Message::Nodes(msg) => self.update_nodes(msg),
            Message::Doctor(msg) => self.update_doctor(msg),
            Message::Blob(msg) => self.update_blob(msg),
            Message::Media(msg) => self.update_media(msg),
            Message::Admin(msg) => self.update_admin(msg),
            Message::Detail(view::detail::DetailMsg::LeafSelected(path)) => {
                self.series_leaf = Some(path);
                Task::none()
            }
            Message::History(msg) => {
                if let Some(rec) = self.history.as_mut() {
                    match msg {
                        view::history::HistoryMsg::Select(seq) => rec.selected = Some(seq),
                        view::history::HistoryMsg::Clear => {
                            rec.ring.clear();
                            rec.selected = None;
                        }
                    }
                }
                Task::none()
            }
            Message::Publish(msg) => self.update_publish(msg),
            Message::CallDone(outcome) => {
                // No base guard needed (#109 audit): a call reply is the
                // answer to the exact question the user pressed the button
                // for, addressed by a full wire key naming its base — user
                // output, not projected deployment state.
                self.call_form.in_flight = false;
                self.call_form.outcome =
                    Some(outcome.map(|r| (*r).clone()).map_err(|e| e.to_string()));
                Task::none()
            }
            Message::PublishReady(Ok(outcome)) => {
                // No base guard needed (#109 audit): publishing is
                // user-initiated *output* on a full wire key, not observed
                // evidence. (A publication straddling a context switch rides
                // the old session's Arc until stopped — a session-lifetime
                // question, out of #109's scope.)
                let form = &mut self.publish_form;
                form.in_flight = false;
                form.error = None;
                form.source = Some(outcome.prepared.source.clone());
                form.note = outcome.prepared.note.clone();
                form.encoding_used = outcome.prepared.encoding.clone();
                form.matching = outcome.matching;
                form.log(
                    true,
                    format!("sent {} bytes → {}", outcome.prepared.bytes.len(), form.key),
                );
                match &outcome.publication {
                    Some(publication) => {
                        form.armed = true;
                        self.publication = Some(RepeatLoad {
                            publication: publication.clone(),
                            bytes: Arc::new(outcome.prepared.bytes.clone()),
                            attachment: outcome.attachment.clone(),
                        });
                    }
                    // One-shot: the task already undeclared.
                    None => {
                        form.armed = false;
                        self.publication = None;
                    }
                }
                Task::none()
            }
            Message::PublishReady(Err(e)) => {
                let form = &mut self.publish_form;
                form.in_flight = false;
                form.armed = false;
                form.error = Some(e.clone());
                form.log(false, format!("refused: {e}"));
                self.publication = None;
                Task::none()
            }
            Message::PublishTick => {
                let Some(load) = self.publication.as_ref() else {
                    return Task::none();
                };
                let publication = load.publication.clone();
                let bytes = load.bytes.clone();
                let attachment = load.attachment.clone();
                Task::perform(
                    async move {
                        publication
                            .send(
                                bytes.as_ref().clone(),
                                attachment.as_ref().map(|a| a.as_ref().clone()),
                            )
                            .await
                            .map(|()| bytes.len())
                            .map_err(|e| e.to_string())
                    },
                    Message::PublishSent,
                )
            }
            Message::PublishSent(Ok(n)) => {
                let key = self.publish_form.key.clone();
                self.publish_form
                    .log(true, format!("sent {n} bytes → {key}"));
                Task::none()
            }
            Message::PublishSent(Err(e)) => {
                // A failed repeat disarms: a stream that silently stopped
                // working would keep claiming it was publishing.
                self.publish_form.log(false, format!("send failed: {e}"));
                self.publish_form.armed = false;
                self.publication = None;
                Task::none()
            }
            Message::PublishRetired(result) => {
                let form = &mut self.publish_form;
                form.in_flight = false;
                match result {
                    Ok(matching) => {
                        // A tombstone has no body provenance: a stale
                        // encoded/as-typed/raw line claiming a body shipped
                        // would be the pane's own O4 mistake.
                        form.source = None;
                        form.note = None;
                        let key = form.key.clone();
                        form.log(
                            true,
                            format!(
                                "retired {key} — an authoritative delete (RFC 04 §1.2), \
                                 not an empty value"
                            ),
                        );
                        if matching == Some(false) {
                            form.log(
                                true,
                                "matching: no subscriber matched the tombstone — a routing \
                                 fact, not a fleet verdict (RFC 05 §3.1)",
                            );
                        }
                    }
                    Err(e) => {
                        form.error = Some(e.clone());
                        form.log(false, format!("retire failed: {e}"));
                    }
                }
                Task::none()
            }
            Message::PublishStopped(result) => {
                match result {
                    Ok(()) => self
                        .publish_form
                        .log(true, "stopped — publication undeclared"),
                    Err(e) => self
                        .publish_form
                        .log(false, format!("undeclare failed: {e}")),
                }
                Task::none()
            }
            Message::Echo(msg) => self.update_echo(msg),
            Message::Replay(msg) => self.update_replay(msg),
            Message::Context(msg) => self.update_context(msg),
            Message::ContextSwitched(Ok(session)) => {
                // A new session is a new everything: the base may differ, so
                // every projection, roster and verdict from the old one is
                // evidence about a different deployment (O4).
                self.forget_deployment();
                self.update(Message::SessionOpened(Ok(session)))
            }
            Message::ContextSwitched(Err(e)) => {
                self.link = LinkState::Failed(e.clone());
                self.context_form.status = Some(Err(format!("could not connect: {e}")));
                Task::none()
            }
            Message::PaneSelected(pane) => {
                self.right_pane = pane;
                Task::none()
            }
            Message::WindowResized(w, h) => {
                // Not on every pixel of a drag — the prefs file would be
                // rewritten hundreds of times per resize. Recorded here and
                // marked dirty; a settle timer writes it once the drag stops
                // (issue #189). "Written on the next real change" meant a
                // resize-then-quit lost the geometry entirely.
                self.prefs.window = Some((w, h));
                self.window_dirty = true;
                Task::none()
            }
            Message::WindowSettled => {
                if self.window_dirty {
                    self.remember();
                }
                Task::none()
            }
            Message::Key(key, modifiers) => self.update_key(&key, modifiers),
            Message::Palette(msg) => self.update_palette(msg),
            Message::Prefs(msg) => {
                use crate::message::PrefsMsg;
                match msg {
                    PrefsMsg::ThemeToggled => self.prefs.theme = self.prefs.theme.toggled(),
                    PrefsMsg::ZoomIn => self.prefs.zoom_in(),
                    PrefsMsg::ZoomOut => self.prefs.zoom_out(),
                    PrefsMsg::ZoomReset => self.prefs.zoom_reset(),
                }
                // Saved on every change rather than at exit: a GUI is killed,
                // not quit, more often than anyone admits.
                self.remember();
                Task::none()
            }
            Message::Reconnect => {
                self.forget_deployment();
                self.start_monitor()
            }
        }
    }

    fn update_call(&mut self, msg: view::call::CallMsg) -> Task<Message> {
        use view::call::CallMsg;
        match msg {
            CallMsg::ProducerPicked(p) => {
                self.call_form.producer = Some(p);
                self.call_form.procedure = None;
                Task::none()
            }
            CallMsg::ProcedurePicked(p) => {
                self.call_form.procedure = Some(p.clone());
                // Scaffold from the served request schema (§6.4 item 3). Not
                // asked yet is `None`, and stays `None` until an answer — the
                // pane renders that as "not asked", never as "no fields".
                self.call_form.request_fields = None;
                let (Some(session), Some(store), Some(producer)) = (
                    self.session.clone(),
                    self.schema_store.clone(),
                    self.call_form.producer.clone(),
                ) else {
                    return Task::none();
                };
                let Some(request) = self
                    .slices
                    .as_ref()
                    .and_then(|s| s.get(&producer))
                    .and_then(|s| s.procedures.iter().find(|d| d.path == p))
                    .and_then(|d| d.request.clone())
                else {
                    // No declared request type: there is nothing to scaffold,
                    // and that is an answer.
                    self.call_form.request_fields = Some(Vec::new());
                    return Task::none();
                };
                Task::perform(
                    async move {
                        store
                            .schema_for(&session, &producer, &request)
                            .await
                            .map(|schema| view::call::schema_fields(&schema))
                    },
                    |fields| Message::Call(CallMsg::RequestSchema(fields)),
                )
            }
            CallMsg::RequestSchema(fields) => {
                self.call_form.request_fields = fields;
                Task::none()
            }
            CallMsg::ScaffoldBody => {
                if let Some(body) = self.call_form.scaffold() {
                    self.call_form.body = body;
                }
                Task::none()
            }
            CallMsg::TargetChanged(t) => {
                self.call_form.target = t;
                Task::none()
            }
            CallMsg::ParamsChanged(t) => {
                self.call_form.params = t;
                Task::none()
            }
            CallMsg::BodyChanged(t) => {
                self.call_form.body = t;
                Task::none()
            }
            CallMsg::AttachmentChanged(t) => {
                self.call_form.attachment = t;
                Task::none()
            }
            CallMsg::Submit => {
                let (Some(session), Some(producer), Some(procedure)) = (
                    self.session.clone(),
                    self.call_form.producer.clone(),
                    self.call_form.procedure.clone(),
                ) else {
                    return Task::none();
                };
                let target = self.call_form.target.clone();
                let params: Vec<String> = self
                    .call_form
                    .params
                    .split(';')
                    .filter(|p| !p.trim().is_empty())
                    .map(str::to_string)
                    .collect();
                let body = if self.call_form.body.trim().is_empty() {
                    None
                } else {
                    Some(self.call_form.body.clone().into_bytes())
                };
                // Verbatim beside the body — never schema-encoded (#126).
                let attachment = if self.call_form.attachment.trim().is_empty() {
                    None
                } else {
                    Some(self.call_form.attachment.clone().into_bytes())
                };
                let base = self.settings.base.clone();
                let timeout = self.settings.timeout();
                let slices = self.slices.clone();
                self.call_form.in_flight = true;
                self.call_form.outcome = None;
                Task::perform(
                    async move {
                        let target =
                            zenkey_fleet::CallTarget::parse(&target).map_err(|e| e.to_string())?;
                        zenkey_fleet::call(
                            &session,
                            &base,
                            &target,
                            &producer,
                            &procedure,
                            &params,
                            body,
                            attachment,
                            timeout,
                            slices.as_deref(),
                        )
                        .await
                        .map(Arc::new)
                        .map_err(|e| e.to_string())
                    },
                    Message::CallDone,
                )
            }
        }
    }

    /// The publish pane (#60). Everything that touches the bus goes through
    /// the engine: `prepare_publish` for the body (#97's encode ladder) and
    /// `declare_publication` for the write (P7 — a declared publisher, never
    /// an ad-hoc put). No codec logic may live on this side of the seam.
    fn update_publish(&mut self, msg: view::publish::PublishMsg) -> Task<Message> {
        use view::publish::PublishMsg;
        match msg {
            PublishMsg::KeyChanged(k) => {
                // Classify as you type, through the same ladder the tree uses.
                self.publish_form.facts = (!k.trim().is_empty()).then(|| {
                    zenkey_fleet::describe_key(&self.settings.base, &k, self.slices.as_deref())
                        .facts
                });
                // #158: the declared profile drives the picker until the user
                // takes it over — and stops driving it the moment they do.
                if !self.publish_form.qos_touched {
                    let declared = view::publish::declared_qos(self.publish_form.facts.as_ref());
                    self.publish_form.qos = view::publish::QosChoice(
                        declared.unwrap_or(zenkey::qos::QosProfile::Sampled),
                    );
                }
                self.publish_form.key = k;
                Task::none()
            }
            PublishMsg::BodyChanged(b) => {
                self.publish_form.body = b;
                Task::none()
            }
            PublishMsg::QosPicked(q) => {
                self.publish_form.qos = q;
                self.publish_form.qos_touched = true;
                Task::none()
            }
            PublishMsg::EncodingChanged(e) => {
                self.publish_form.encoding = e;
                Task::none()
            }
            PublishMsg::AttachmentChanged(a) => {
                self.publish_form.attachment = a;
                Task::none()
            }
            PublishMsg::RawToggled(b) => {
                self.publish_form.raw = b;
                Task::none()
            }
            PublishMsg::RepeatToggled(b) => {
                self.publish_form.repeat = b;
                // Turning repeat off mid-stream stops it, rather than leaving
                // an armed publication with the checkbox saying otherwise.
                if !b && self.publication.is_some() {
                    return self.update_publish(PublishMsg::Stop);
                }
                Task::none()
            }
            PublishMsg::IntervalChanged(i) => {
                self.publish_form.interval = i;
                Task::none()
            }
            PublishMsg::Stop => {
                self.publish_form.armed = false;
                let Some(RepeatLoad { publication, .. }) = self.publication.take() else {
                    return Task::none();
                };
                // Acknowledged undeclare when we hold the last reference; a
                // drop would undeclare too, but silently.
                match Arc::try_unwrap(publication) {
                    Ok(p) => Task::perform(
                        async move { p.undeclare().await.map_err(|e| e.to_string()) },
                        Message::PublishStopped,
                    ),
                    Err(_) => Task::none(),
                }
            }
            PublishMsg::RetireIKnowToggled(b) => {
                self.publish_form.retire_i_know = b;
                Task::none()
            }
            PublishMsg::Retire => {
                let Some(session) = self.session.clone() else {
                    return Task::none();
                };
                let key = self.publish_form.key.trim().to_string();
                if key.is_empty() {
                    return Task::none();
                }
                // Same stop-first discipline as Send: a retire must not race
                // an armed publication on the key.
                let stop = self.update_publish(PublishMsg::Stop);
                // The engine is the judge (check_retire, RFC 04 §1.2 v1.12);
                // the pane's checkbox only arms the force.
                if let Err(e) = zenkey_fleet::check_retire(
                    &self.settings.base,
                    &key,
                    self.slices.as_deref(),
                    self.publish_form.retire_i_know,
                ) {
                    let e = e.to_string();
                    self.publish_form.error = Some(e.clone());
                    self.publish_form.log(false, format!("refused: {e}"));
                    return stop;
                }
                self.publish_form.in_flight = true;
                self.publish_form.error = None;
                let send = Task::perform(
                    async move {
                        // A retirement is the final state transition: the
                        // reliable profile, like `zenctl topic retire`.
                        let publication = zenkey_fleet::declare_publication(
                            &session,
                            &key,
                            zenkey::qos::QosProfile::Transition,
                            None,
                        )
                        .await
                        .map_err(|e| e.to_string())?;
                        let matching = publication.matching_status().await.ok();
                        publication.retire().await.map_err(|e| e.to_string())?;
                        publication.undeclare().await.map_err(|e| e.to_string())?;
                        Ok(matching)
                    },
                    Message::PublishRetired,
                );
                Task::batch([stop, send])
            }
            PublishMsg::Send => {
                let (Some(session), Some(store)) =
                    (self.session.clone(), self.schema_store.clone())
                else {
                    return Task::none();
                };
                // A new send replaces any armed publication: two publishers on
                // one key from one pane would double every sample.
                let stop = self.update_publish(PublishMsg::Stop);
                let form = &self.publish_form;
                let key = form.key.trim().to_string();
                let body = form.body.clone().into_bytes();
                let qos = form.qos.0;
                let encoding = form.encoding.trim().to_string();
                let mode = if form.raw {
                    zenkey_fleet::PrepareMode::Raw
                } else {
                    zenkey_fleet::PrepareMode::Encode
                };
                let base = self.settings.base.clone();
                let slices = self.slices.clone();
                let repeat = form.repeat;
                // Verbatim, never schema-encoded (#117); empty = none.
                let attachment: Option<Arc<Vec<u8>>> = (!form.attachment.is_empty())
                    .then(|| Arc::new(form.attachment.clone().into_bytes()));
                self.publish_form.in_flight = true;
                self.publish_form.error = None;
                let send = Task::perform(
                    async move {
                        let prepared = zenkey_fleet::prepare_publish(
                            &session,
                            &store,
                            slices.as_deref(),
                            &base,
                            &key,
                            (!encoding.is_empty()).then_some(encoding.as_str()),
                            &body,
                            mode,
                        )
                        .await
                        .map_err(|e| e.to_string())?;
                        let publication = zenkey_fleet::declare_publication(
                            &session,
                            &key,
                            qos,
                            prepared.encoding.as_deref(),
                        )
                        .await
                        .map_err(|e| e.to_string())?;
                        publication
                            .send(
                                prepared.bytes.clone(),
                                attachment.as_ref().map(|a| a.as_ref().clone()),
                            )
                            .await
                            .map_err(|e| e.to_string())?;
                        // The badge is a routing fact about this publisher and
                        // nothing else; an error asking is "not asked" (O4),
                        // never "nobody listens".
                        let matching = publication.matching_status().await.ok();
                        // A one-shot undeclares here, acknowledged, exactly as
                        // `zenctl topic pub` does; only a repeating publish
                        // hands the declaration back to be held.
                        let publication = if repeat {
                            Some(Arc::new(publication))
                        } else {
                            publication.undeclare().await.map_err(|e| e.to_string())?;
                            None
                        };
                        Ok(Arc::new(crate::message::PublishOutcome {
                            prepared,
                            publication,
                            matching,
                            attachment,
                        }))
                    },
                    Message::PublishReady,
                );
                Task::batch([stop, send])
            }
        }
    }

    /// Everything a session learned about one deployment, forgotten.
    ///
    /// Factored out because three paths need exactly this — a base change, a
    /// reconnect, and a context switch — and each one that forgot a different
    /// subset would leave a stale verdict on screen about a fleet it is no
    /// longer looking at (O4).
    ///
    /// What survives, deliberately (#109 audit): the tree selection, the
    /// fetched value, the history recorder, and the call/publish forms —
    /// each keyed by a full wire key that names its own base, or user input
    /// rather than projected evidence. What cannot be cleared here — a task
    /// already in flight — is judged at its landing instead: doctor, blob
    /// probe/fetch and node_info each carry the base they ran against.
    fn forget_deployment(&mut self) {
        self.facts.clear();
        self.roster.clear();
        self.node_selected = None;
        self.node_detail = view::nodes::DetailState::NotAsked;
        self.doctor.clear();
        self.blob.clear();
        self.admin.clear();
        self.my_watches.clear();
        self.my_watch_paths.clear();
        self.scope_watches.clear();
        self.seeding.clear();
        self.seeding_paths.clear();
        self.seed_totals = (0, 0, 0);
        self.seeded_watches = 0;
        self.skeleton = None;
        self.slices = None;
        self.slice_source = view::status::SliceSource::None;
        self.bases.clear();
    }

    /// One key press, in context.
    ///
    /// **Esc layering** (#75): palette first, then a tree selection, then
    /// nothing — one layer per press, so Esc never does two things at once.
    /// The arrows drive the overlay only while one is open, which is what
    /// keeps them available to the panes the rest of the time.
    fn update_key(
        &mut self,
        key: &iced::keyboard::Key,
        modifiers: iced::keyboard::Modifiers,
    ) -> Task<Message> {
        use iced::keyboard::{Key, key::Named};
        use view::palette::PaletteMsg;

        if crate::shortcuts::is_escape(key) {
            if self.palette.is_open() {
                self.palette.close();
            } else if self.selected.is_some() {
                return self.update(Message::SelectKey(None));
            }
            return Task::none();
        }
        if self.palette.is_open() {
            match key {
                Key::Named(Named::ArrowDown) => {
                    return self.update_palette(PaletteMsg::CursorDown);
                }
                Key::Named(Named::ArrowUp) => {
                    return self.update_palette(PaletteMsg::CursorUp);
                }
                Key::Named(Named::Enter) => return self.update_palette(PaletteMsg::Activate),
                _ => {}
            }
        }
        match crate::shortcuts::resolve(key, modifiers) {
            Some(message) => self.update(message),
            None => Task::none(),
        }
    }

    /// The command palette (#75).
    ///
    /// Every activation dispatches through `self.update(...)` with the
    /// action's own message, which is what keeps the palette from being a
    /// second implementation of anything: it is a faster way to send a message
    /// the UI already sends, and nothing more.
    fn update_palette(&mut self, msg: view::palette::PaletteMsg) -> Task<Message> {
        use view::palette::PaletteMsg;
        match msg {
            PaletteMsg::Open(overlay) => {
                self.palette.open(overlay);
                Task::none()
            }
            PaletteMsg::Close => {
                self.palette.close();
                Task::none()
            }
            PaletteMsg::QueryChanged(q) => {
                self.palette.query = q;
                // A new query re-ranks the list, so the old cursor points at a
                // different row — start from the best match again.
                self.palette.cursor = 0;
                Task::none()
            }
            PaletteMsg::CursorUp => {
                self.palette.cursor = self.palette.cursor.saturating_sub(1);
                Task::none()
            }
            PaletteMsg::CursorDown => {
                self.palette.cursor = self
                    .palette
                    .cursor
                    .saturating_add(1)
                    .min(self.palette_row_count().saturating_sub(1));
                Task::none()
            }
            PaletteMsg::Activate => self.run_palette_row(self.palette.cursor),
            PaletteMsg::Pick(i) => self.run_palette_row(i),
        }
    }

    /// How many rows the open overlay currently shows. Ranks over borrowed
    /// keys (fat pointers, no string bytes) — runs per keypress, not per
    /// frame (#110).
    fn palette_row_count(&self) -> usize {
        use view::palette::{Overlay, actions, rank};
        match self.palette.overlay {
            Overlay::Commands => {
                let items = actions(&self.context_form.known);
                rank(&items, &self.palette.query, |a| a.label.as_str()).len()
            }
            Overlay::Keys => {
                // Observed keys only — never a guess (O4): the jump-to
                // overlay offers what is on the bus, not what a registry
                // says could be.
                let keys: Vec<&str> = self.facts.keys().collect();
                rank(&keys, &self.palette.query, |k| *k).len()
            }
            _ => 0,
        }
    }

    /// The message behind row `index` — on the Keys overlay, the one place
    /// the palette ever clones a key `String` (#110): the activated row.
    fn palette_row(&self, index: usize) -> Option<Message> {
        use view::palette::{Overlay, actions, rank};
        match self.palette.overlay {
            Overlay::Commands => {
                let items = actions(&self.context_form.known);
                let order = rank(&items, &self.palette.query, |a| a.label.as_str());
                order.get(index).map(|i| items[*i].message.clone())
            }
            Overlay::Keys => {
                let keys: Vec<&str> = self.facts.keys().collect();
                let order = rank(&keys, &self.palette.query, |k| *k);
                order
                    .get(index)
                    .map(|i| Message::SelectKey(Some(keys[*i].to_string())))
            }
            _ => None,
        }
    }

    fn run_palette_row(&mut self, index: usize) -> Task<Message> {
        let Some(message) = self.palette_row(index) else {
            return Task::none();
        };
        // Jumping to a key also shows it: selecting without switching panes
        // would look like nothing happened.
        if matches!(self.palette.overlay, view::palette::Overlay::Keys) {
            self.right_pane = RightPane::Detail;
        }
        self.palette.close();
        self.update(message)
    }

    /// The connection pane (#67). Contexts are read and written through the
    /// **shared** store, so a context created here is one `zenctl` selects and
    /// vice versa — that reciprocity is the feature, not a side effect.
    fn update_context(&mut self, msg: view::contexts::ContextMsg) -> Task<Message> {
        use view::contexts::ContextMsg;
        match msg {
            ContextMsg::NameChanged(v) => {
                self.context_form.name = v;
                Task::none()
            }
            ContextMsg::ConnectChanged(v) => {
                self.context_form.connect = v;
                Task::none()
            }
            ContextMsg::ListenChanged(v) => {
                self.context_form.listen = v;
                Task::none()
            }
            ContextMsg::BaseChanged(v) => {
                self.context_form.base = v;
                Task::none()
            }
            ContextMsg::ZenohConfigChanged(p) => {
                self.context_form.zenoh_config = p;
                Task::none()
            }
            ContextMsg::RegistryChanged(v) => {
                self.context_form.registry = v;
                Task::none()
            }
            ContextMsg::TimeoutChanged(v) => {
                self.context_form.timeout = v;
                Task::none()
            }
            ContextMsg::ScoutingToggled(b) => {
                self.context_form.scouting = b;
                Task::none()
            }
            ContextMsg::Isolate => {
                // RFC 09 §0.1's isolated-verification recipe, one click:
                // multicast off, explicit endpoints only.
                self.context_form.scouting = false;
                self.context_form.status = Some(Ok(
                    "multicast scouting off — an empty result now means \"nothing on these \
                     endpoints\", never \"nothing on the network\" (RFC 09 §0.1)"
                        .into(),
                ));
                Task::none()
            }
            ContextMsg::Load => {
                let Some(name) = self.context_form.active.clone() else {
                    self.context_form.status = Some(Err("pick a context first".into()));
                    return Task::none();
                };
                match zenkey_fleet::context_store::load() {
                    Ok(config) => match config.contexts.get(&name) {
                        Some(stored) => {
                            self.context_form.load_from(&name, stored);
                            self.context_form.status = Some(Ok(format!("loaded {name}")));
                        }
                        None => {
                            self.context_form.status =
                                Some(Err(format!("{name} is no longer in the config")));
                        }
                    },
                    Err(e) => self.context_form.status = Some(Err(e.to_string())),
                }
                Task::none()
            }
            ContextMsg::Save => {
                self.save_context(false);
                Task::none()
            }
            ContextMsg::SaveAndSelect => {
                if !self.save_context(true) {
                    return Task::none();
                }
                self.switch_to_form_context()
            }
            ContextMsg::Selected(name) => {
                self.context_form.active = Some(name.clone());
                match zenkey_fleet::context_store::load() {
                    Ok(mut config) => match config.contexts.get(&name).cloned() {
                        Some(stored) => {
                            self.context_form.load_from(&name, &stored);
                            self.apply_context(stored);
                            self.prefs.context = Some(name.clone());
                            self.remember();
                            // Move the store's own pointer too, not just this
                            // window's memory of it: the store is shared, and
                            // picking a context here left `zenctl context show`
                            // still naming the old one (issue #189).
                            config.current = Some(name.clone());
                            if let Err(e) = zenkey_fleet::context_store::save(&config) {
                                self.context_form.status = Some(Err(format!(
                                    "switched to {name}, but the shared `current` \
                                     pointer could not be written: {e}"
                                )));
                                return self.reopen_session();
                            }
                            self.context_form.status = Some(Ok(format!("switched to {name}")));
                            return self.reopen_session();
                        }
                        None => {
                            self.context_form.status =
                                Some(Err(format!("{name} is no longer in the config")));
                        }
                    },
                    Err(e) => self.context_form.status = Some(Err(e.to_string())),
                }
                Task::none()
            }
        }
    }

    /// Write the editor to the shared config. Returns whether it landed.
    ///
    /// Through `upsert`, not `insert`: the store is shared with zenctl, and
    /// replacing the whole entry deleted every field this form has no widget
    /// for (issue #194). The form now covers all of them, so this is the guard
    /// for the next field somebody adds to `StoredContext`.
    fn save_context(&mut self, select: bool) -> bool {
        // Validate before touching the store, so a rejected form leaves it be.
        if let Err(e) = self.context_form.to_stored() {
            self.context_form.status = Some(Err(e));
            return false;
        }
        let name = self.context_form.name.trim().to_string();
        let mut config = match zenkey_fleet::context_store::load() {
            Ok(c) => c,
            Err(e) => {
                self.context_form.status = Some(Err(e.to_string()));
                return false;
            }
        };
        let form = self.context_form.clone();
        let mut applied = Ok(());
        zenkey_fleet::context_store::upsert(&mut config, &name, |c| applied = form.apply_to(c));
        if let Err(e) = applied {
            self.context_form.status = Some(Err(e));
            return false;
        }
        if select {
            config.current = Some(name.clone());
        }
        if let Err(e) = zenkey_fleet::context_store::save(&config) {
            self.context_form.status = Some(Err(e.to_string()));
            return false;
        }
        self.refresh_contexts();
        self.context_form.active = Some(name.clone());
        self.context_form.status = Some(Ok(format!(
            "saved {name} to {}",
            zenkey_fleet::context_store::config_path().display()
        )));
        true
    }

    /// Re-read the context names from the shared config.
    fn refresh_contexts(&mut self) {
        if let Ok(config) = zenkey_fleet::context_store::load() {
            self.context_form.known = config.contexts.keys().cloned().collect();
            if self.context_form.active.is_none() {
                self.context_form.active = config.current.clone();
            }
        }
    }

    /// Layer a stored context over the live settings — the same precedence
    /// `Cli::settings_with` applies, minus the flags, because a context picked
    /// in-app *is* the explicit choice.
    fn apply_context(&mut self, stored: zenkey_fleet::StoredContext) {
        self.settings.base = stored.base.unwrap_or_default();
        self.settings.connect = stored.connect;
        self.settings.listen = stored.listen;
        self.settings.scouting = stored.scouting;
        self.settings.zenoh_config = stored.zenoh_config;
        if !stored.registry.is_empty() {
            self.settings.registry = stored.registry;
        }
        if let Some(t) = stored.timeout {
            self.settings.timeout_secs = t;
        }
    }

    fn switch_to_form_context(&mut self) -> Task<Message> {
        let stored = match self.context_form.to_stored() {
            Ok(s) => s,
            Err(e) => {
                self.context_form.status = Some(Err(e));
                return Task::none();
            }
        };
        self.apply_context(stored);
        self.prefs.context = Some(self.context_form.name.trim().to_string());
        self.remember();
        self.reopen_session()
    }

    /// Tear the link down and build a new one on the current settings.
    ///
    /// The epoch bump the subscription machinery already does on
    /// `MonitorStarted` is what retires the old pump; nothing here has to
    /// coordinate with it.
    fn reopen_session(&mut self) -> Task<Message> {
        self.link = LinkState::Connecting;
        self.monitor = None;
        self.session = None;
        let connect = self.settings.connect.clone();
        let listen = self.settings.listen.clone();
        let scouting = self.settings.scouting;
        let zenoh_config = self.settings.zenoh_config.clone();
        Task::perform(
            async move {
                zenkey_fleet::open_with_config(zenoh_config.as_deref(), &connect, &listen, scouting)
                    .await
                    .map_err(|e| e.to_string())
            },
            Message::ContextSwitched,
        )
    }

    /// The echo pane (#72). Every action here is a *view* action: nothing
    /// changes what the session subscribes to, which is what keeps "I filtered
    /// the pane" and "I narrowed the bus" two different, visible things.
    fn update_echo(&mut self, msg: view::echo::EchoMsg) -> Task<Message> {
        use view::echo::EchoMsg;
        match msg {
            EchoMsg::FilterChanged(f) => {
                self.echo_view.filter = f;
                Task::none()
            }
            EchoMsg::KeyFilterChanged(f) => {
                self.echo_view.set_key_filter(f);
                Task::none()
            }
            EchoMsg::FollowToggled => {
                let seq = self.echo.next_seq();
                if self.echo_view.following {
                    self.echo_view.pause(seq);
                } else {
                    self.echo_view.resume(seq);
                }
                Task::none()
            }
            EchoMsg::Clear => {
                self.echo.clear();
                Task::none()
            }
            EchoMsg::LineClicked(key) => {
                // Drill-through reuses the selection path rather than being a
                // second way to open the inspector.
                self.right_pane = RightPane::Detail;
                self.update(Message::SelectKey(Some(key)))
            }
            EchoMsg::Export => {
                let text = view::echo::export(
                    &self.echo,
                    &self.echo_view,
                    self.selected.as_deref(),
                    &self.settings.base,
                );
                iced::clipboard::write(text)
            }
        }
    }

    fn update_nodes(&mut self, msg: view::nodes::NodesMsg) -> Task<Message> {
        use view::nodes::{DetailState, NodesMsg};
        match msg {
            NodesMsg::Selected(origin) => {
                if self.node_selected.as_deref() == Some(origin.as_str()) {
                    // Re-click deselects (and drops the detail state).
                    self.node_selected = None;
                    self.node_detail = DetailState::NotAsked;
                    return Task::none();
                }
                self.node_selected = Some(origin.clone());
                self.node_detail = DetailState::Loading(origin.clone());
                let Some(session) = self.session.clone() else {
                    return Task::none();
                };
                let base = self.settings.base.clone();
                let timeout = self.settings.timeout();
                // The pane's one data-plane cost: a one-shot node_info on
                // selection (laziness ground rule, #84/#85).
                Task::perform(
                    async move {
                        let out = zenkey_fleet::node_info(&session, &base, &origin, timeout, true)
                            .await
                            .map(Arc::new)
                            .map_err(|e| e.to_string());
                        (origin, base, out)
                    },
                    |(origin, ran, out)| Message::Nodes(NodesMsg::InfoLoaded(origin, ran, out)),
                )
            }
            NodesMsg::InfoLoaded(origin, ran_against, outcome) => {
                // Stale guard: only the currently selected origin's info lands
                // — and only when it ran against the base this window shows.
                // Origin ids are base-independent (#109): re-selecting the
                // same host after a base switch would otherwise admit the old
                // deployment's reply through the selection-only check.
                if self.node_selected.as_deref() == Some(origin.as_str())
                    && ran_against == self.settings.base
                {
                    self.node_detail = DetailState::Loaded(origin, outcome);
                }
                Task::none()
            }
            NodesMsg::ShowInTree(origin) => {
                let path = scope::origin_display_path(&self.settings.base, &origin);
                // Expand every prefix so the subtree is visible; select
                // WITHOUT fetching (a subtree prefix is not a concrete key).
                let mut prefix = String::new();
                for chunk in path.split('/') {
                    if !prefix.is_empty() {
                        prefix.push('/');
                    }
                    prefix.push_str(chunk);
                    self.expanded.insert(prefix.clone());
                }
                self.selected = Some(path);
                self.reflatten();
                Task::none()
            }
        }
    }

    fn update_doctor(&mut self, msg: view::doctor::DoctorMsg) -> Task<Message> {
        use view::doctor::DoctorMsg;
        match msg {
            DoctorMsg::DeepToggled(deep) => {
                self.doctor.deep = deep;
                Task::none()
            }
            DoctorMsg::ListenChanged(t) => {
                self.doctor.listen = t;
                Task::none()
            }
            DoctorMsg::ReaskSchemas => {
                // Local and immediate: the store simply forgets, and the next
                // decode that needs a schema asks the bus. Nothing is fetched
                // here — an explorer retrieves nothing it was not asked for
                // (#85), and "re-ask" is a permission, not a sweep.
                if let Some(store) = self.schema_store.as_ref() {
                    store.forget_all();
                    self.doctor.schemas_forgotten += 1;
                }
                Task::none()
            }
            DoctorMsg::Run => {
                if self.doctor.in_flight {
                    return Task::none();
                }
                let Some(session) = self.session.clone() else {
                    self.doctor.error = Some("no session — connect first".into());
                    return Task::none();
                };
                self.doctor.in_flight = true;
                self.doctor.error = None;
                let base = self.settings.base.clone();
                let timeout = self.settings.timeout();
                let deep = self.doctor.deep;
                let listen = self.doctor.listen_window();
                // Locals come from the registry DIRS only — never the union
                // set, which would diff the bus against itself.
                let dirs = self.settings.registry.clone();
                Task::perform(
                    async move {
                        let locals = match SliceSet::from_dirs(&dirs) {
                            Ok(set) => set.slices().to_vec(),
                            Err(e) => return Err(format!("registry dirs: {e}")),
                        };
                        zenkey_fleet::run_doctor(
                            &session,
                            &base,
                            &locals,
                            &zenkey_fleet::DoctorSpec {
                                deep,
                                sample: None,
                                timeout,
                                listen,
                            },
                        )
                        .await
                        // The run carries the base it was judged against —
                        // the staleness guard's other half (#109).
                        .map(|r| crate::doctor::DoctorRun {
                            report: Arc::new(r),
                            base,
                        })
                        .map_err(|e| e.to_string())
                    },
                    |out| Message::Doctor(DoctorMsg::Done(out)),
                )
            }
            DoctorMsg::Done(outcome) => {
                let base = self.settings.base.clone();
                self.doctor.finish(outcome, &base);
                Task::none()
            }
            DoctorMsg::FindingClicked(index) => {
                let Some(finding) = self
                    .doctor
                    .current
                    .as_deref()
                    .and_then(|r| r.findings.get(index))
                else {
                    return Task::none();
                };
                match crate::doctor::finding_target(finding, &self.settings.base) {
                    // A concrete key: select in the tree (with the usual
                    // on-demand fetch); the right pane stays on Doctor so
                    // the finding list is not lost.
                    Some(crate::doctor::Target::Key(key)) => {
                        let mut prefix = String::new();
                        for chunk in key.split('/') {
                            if !prefix.is_empty() {
                                prefix.push('/');
                            }
                            prefix.push_str(chunk);
                            self.expanded.insert(prefix.clone());
                        }
                        self.reflatten();
                        self.update(Message::SelectKey(Some(key)))
                    }
                    // An origin/producer subject: land on the nodes pane.
                    Some(crate::doctor::Target::Node(origin)) => {
                        self.right_pane = RightPane::Nodes;
                        self.update_nodes(view::nodes::NodesMsg::Selected(origin))
                    }
                    None => Task::none(),
                }
            }
        }
    }

    /// Reproject the registry's `[[blob]]` declarations after a slice load.
    ///
    /// Costs nothing on the bus: it reads slices already in hand and joins them
    /// against the roster already observed. The laziness rule (#84/#85) governs
    /// *fetching*, not rendering what has arrived — and the roster's
    /// `live_map()` returns `None` while unseeded, so an unasked join renders
    /// as "not asked" rather than as "nobody serves it" (O4).
    fn refresh_blob_list(&mut self) {
        let Some(slices) = self.slices.as_deref() else {
            self.blob.list = None;
            return;
        };
        let source = match self.slice_source {
            view::status::SliceSource::Union { .. } => zenkey_fleet::report::BlobListSource::Union,
            view::status::SliceSource::Dirs { .. } => {
                zenkey_fleet::report::BlobListSource::RegistryDirs
            }
            _ => zenkey_fleet::report::BlobListSource::Bus,
        };
        self.blob.list = Some(zenkey_fleet::blob_list(
            slices.slices(),
            self.roster.live_map().as_ref(),
            source,
        ));
    }

    /// The media viewer (issue #69): subscribe on view, release on stop,
    /// and the key is built in exactly one place — `scope::media_key`,
    /// which refuses wildcards and unfilled placeholders (RFC 07 §1).
    fn update_media(&mut self, msg: view::media::MediaMsg) -> Task<Message> {
        use view::media::MediaMsg as M;
        match msg {
            M::OriginChanged(s) => {
                self.media.origin = s;
                Task::none()
            }
            M::ProducerChanged(s) => {
                self.media.producer = s;
                Task::none()
            }
            M::SubpathChanged(s) => {
                self.media.subpath = s;
                Task::none()
            }
            M::DeclPicked { producer, path } => {
                self.media.producer = producer;
                self.media.subpath = path;
                // A convenience, not a guess: if exactly one origin is on
                // the roster, prefill it; otherwise the operator names one.
                if self.media.origin.is_empty() {
                    let hosts: Vec<&String> = self
                        .roster
                        .iter()
                        .map(|(origin, _)| origin)
                        .filter(|o| o.starts_with("h-"))
                        .collect();
                    if let [only] = hosts.as_slice() {
                        self.media.origin = (*only).clone();
                    }
                }
                Task::none()
            }
            M::View => {
                let Some(monitor) = self.monitor.clone() else {
                    self.media.error = Some("no session — connect first".into());
                    return Task::none();
                };
                let key = match crate::scope::media_key(
                    &self.settings.base,
                    self.media.origin.trim(),
                    self.media.producer.trim(),
                    self.media.subpath.trim(),
                ) {
                    Ok(k) => k,
                    Err(e) => {
                        self.media.error = Some(e);
                        return Task::none();
                    }
                };
                self.media.error = None;
                // One stream at a time: release the previous watch first.
                let release = self.stop_media_watch();
                self.media.viewing = Some(view::media::Viewing::new(key.clone()));
                let declare = Task::perform(
                    async move { monitor.watch(&key).await.map_err(|e| e.to_string()) },
                    |r| Message::Media(M::Watched(r)),
                );
                Task::batch([release, declare])
            }
            M::Watched(Ok(id)) => {
                if let Some(v) = &mut self.media.viewing {
                    v.watch = Some(id);
                }
                Task::none()
            }
            M::Watched(Err(e)) => {
                self.media.error = Some(e);
                self.media.viewing = None;
                Task::none()
            }
            M::Stop => {
                let release = self.stop_media_watch();
                self.media.viewing = None;
                release
            }
            M::Stopped => Task::none(),
        }
    }

    /// Release the media watch, if one is declared — the "unviewed streams
    /// cost zero subscriptions" half of #69's contract.
    fn stop_media_watch(&mut self) -> Task<Message> {
        let (Some(monitor), Some(id)) = (
            self.monitor.clone(),
            self.media.viewing.as_ref().and_then(|v| v.watch),
        ) else {
            return Task::none();
        };
        Task::perform(
            async move {
                let _ = monitor.unwatch(id).await;
            },
            |()| Message::Media(view::media::MediaMsg::Stopped),
        )
    }

    fn update_blob(&mut self, msg: view::blob::BlobMsg) -> Task<Message> {
        use view::blob::BlobMsg;
        match msg {
            BlobMsg::TargetChanged(t) => {
                self.blob.set_target(t);
                Task::none()
            }
            BlobMsg::RootChanged(t) => {
                self.blob.root_input = t;
                Task::none()
            }
            BlobMsg::DestChanged(t) => {
                self.blob.dest_input = t;
                Task::none()
            }
            BlobMsg::AllowUnpinnedToggled(b) => {
                self.blob.allow_unpinned = b;
                Task::none()
            }
            BlobMsg::HolderPicked(i) => {
                self.blob.holder = Some(i);
                Task::none()
            }
            BlobMsg::UseSuggestedName => {
                // Fills the *field*. The advisory filename never becomes a
                // path on its own — a remote party does not choose where our
                // bytes land.
                if let Some(name) = self
                    .blob
                    .selected()
                    .and_then(|h| h.manifest.as_ref())
                    .and_then(|m| m.filename.clone())
                {
                    self.blob.dest_input = name;
                }
                Task::none()
            }
            BlobMsg::Probe => {
                let Some(Ok(target)) = self.blob.target.clone() else {
                    return Task::none();
                };
                let Some(session) = self.session.clone() else {
                    self.blob.probe =
                        crate::blob::Probe::Failed("no session — connect first".into());
                    return Task::none();
                };
                self.blob.probe = crate::blob::Probe::InFlight;
                self.blob.holder = None;
                let base = self.settings.base.clone();
                let timeout = self.settings.timeout();
                let slices = self
                    .slices
                    .as_deref()
                    .map(|s| s.slices().to_vec())
                    .unwrap_or_default();
                Task::perform(
                    async move {
                        let out =
                            zenkey_fleet::blob_probe(&session, &base, &target, &slices, timeout)
                                .await
                                .map(Arc::new)
                                .map_err(|e| e.to_string());
                        (base, out)
                    },
                    |(ran, out)| Message::Blob(BlobMsg::ProbeDone(ran, out)),
                )
            }
            BlobMsg::ProbeDone(ran_against, outcome) => {
                self.blob
                    .probe_finished(outcome, &ran_against, &self.settings.base);
                Task::none()
            }
            BlobMsg::Fetch => {
                if self.blob.fetch_ready().is_err() {
                    return Task::none();
                }
                let Some(Ok(target)) = self.blob.target.clone() else {
                    return Task::none();
                };
                let Some(session) = self.session.clone() else {
                    self.blob.fetch =
                        crate::blob::Fetch::Failed("no session — connect first".into());
                    return Task::none();
                };
                // The origin comes off the chosen holder, which came off a
                // reply key. There is no other way for one to enter here.
                let Some(origin) = self.blob.selected().map(|h| h.origin.clone()) else {
                    return Task::none();
                };

                // A tree target is inspected, not downloaded (RFC 07 §2.3,
                // v1.17): fetch the descriptor and index chunks, validate
                // against the root — which the key already is — and render
                // the summary. No file, no progress stream, no cancel token.
                if let zenkey_fleet::BlobTarget::Tree { root } = &target {
                    let root = root.clone();
                    self.blob.fetch = crate::blob::Fetch::Inspecting;
                    let base = self.settings.base.clone();
                    let timeout = self.settings.timeout();
                    return Task::perform(
                        async move {
                            let out = zenkey_fleet::blob_tree_index(
                                &session, &base, &origin, &root, timeout,
                            )
                            .await
                            .map(Arc::new)
                            .map_err(|e| e.to_string());
                            (base, out)
                        },
                        |(ran, out)| Message::Blob(BlobMsg::InspectDone(ran, out)),
                    );
                }
                let root = match self.blob.root_input.trim() {
                    "" => None,
                    hex => match zenkey::ContentHash::parse(hex) {
                        Ok(h) => Some(h),
                        Err(e) => {
                            self.blob.fetch = crate::blob::Fetch::Failed(format!("root: {e}"));
                            return Task::none();
                        }
                    },
                };

                let cancel = zenkey_fleet::zblob::CancelToken::new();
                self.blob.cancel = Some(cancel.clone());
                self.blob.fetch = crate::blob::Fetch::InFlight {
                    received: 0,
                    total: 0,
                    bytes: 0,
                };

                let dest = std::path::PathBuf::from(self.blob.dest_input.trim());
                let base = self.settings.base.clone();
                let timeout = self.settings.timeout();
                // Progress arrives on a channel rather than through the return
                // value: a transfer that only reported at the end would leave
                // the pane unable to say anything true while it ran.
                let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
                let progress = Task::run(
                    async_stream::stream! {
                        let mut rx = rx;
                        while let Some(p) = rx.recv().await {
                            yield p;
                        }
                    },
                    |p| Message::Blob(BlobMsg::Progress(p)),
                );
                let run = Task::perform(
                    async move {
                        let spec = zenkey_fleet::BlobFetchSpec {
                            timeout,
                            overwrite: true,
                            root,
                            cancel,
                        };
                        let sink = move |p| {
                            let _ = tx.send(p);
                        };
                        let out = zenkey_fleet::blob_fetch(
                            &session, &base, &origin, &target, &dest, &spec, &sink,
                        )
                        .await
                        .map(Arc::new)
                        .map_err(|e| e.to_string());
                        (base, out)
                    },
                    |(ran, out)| Message::Blob(BlobMsg::FetchDone(ran, out)),
                );
                Task::batch([progress, run])
            }
            BlobMsg::Progress(p) => {
                use zenkey_fleet::report::BlobProgress;
                // No base guard needed (#109 audit): progress only mutates
                // while Fetch::InFlight, and a base change resets the pane to
                // NotAsked via blob.clear() — stale ticks fall through.
                if let crate::blob::Fetch::InFlight {
                    received,
                    total,
                    bytes,
                } = &mut self.blob.fetch
                {
                    match p {
                        BlobProgress::Started { chunk_count, .. } => *total = chunk_count,
                        BlobProgress::Resumed {
                            received: r,
                            total: t,
                        } => {
                            *received = r;
                            *total = t;
                        }
                        BlobProgress::Chunk {
                            received: r,
                            total: t,
                            bytes_received,
                            ..
                        } => {
                            *received = r;
                            *total = t;
                            *bytes = bytes_received;
                        }
                        // Completion, cancellation and failure are the
                        // report's to state, so the pane says one thing about
                        // the outcome rather than two.
                        _ => {}
                    }
                }
                Task::none()
            }
            BlobMsg::FetchDone(ran_against, outcome) => {
                self.blob
                    .fetch_finished(outcome, &ran_against, &self.settings.base);
                Task::none()
            }
            BlobMsg::InspectDone(ran_against, outcome) => {
                self.blob
                    .inspect_finished(outcome, &ran_against, &self.settings.base);
                Task::none()
            }
            BlobMsg::Cancel => {
                if let Some(c) = self.blob.cancel.take() {
                    c.cancel();
                }
                Task::none()
            }
        }
    }

    fn update_admin(&mut self, msg: view::admin::AdminMsg) -> Task<Message> {
        use view::admin::AdminMsg;
        match msg {
            AdminMsg::RawToggled(id) => {
                self.admin.toggle_raw(id);
                Task::none()
            }
            AdminMsg::FilterProducer(producer) => {
                // The tree already owns "show me this": reusing its search is
                // one behaviour, not two that can drift.
                self.right_pane = RightPane::Echo;
                Task::done(Message::TreeSearchChanged(producer))
            }
            AdminMsg::Run => {
                if self.admin.in_flight {
                    return Task::none();
                }
                let Some(session) = self.session.clone() else {
                    self.admin.error = Some("no session — connect first".into());
                    return Task::none();
                };
                self.admin.in_flight = true;
                self.admin.error = None;
                let base = self.settings.base.clone();
                let timeout = self.settings.timeout();
                // The app's *resolved* slice set — bus, dirs or the union.
                //
                // Deliberately NOT the doctor's `SliceSet::from_dirs`: that one
                // is dirs-only because it diffs local against served, whereas
                // the coverage join asks "what does the registry say exists",
                // which is `bus.slice_set()` in zenctl. Copying the doctor here
                // would empty the coverage table on every bus-registry-only
                // deployment — an invisible, plausible-looking wrong answer.
                let slices = self.slices.clone();
                Task::perform(
                    async move {
                        let routers = zenkey_fleet::routers(&session, timeout)
                            .await
                            .map_err(|e| e.to_string())?;
                        let storages = zenkey_fleet::storages(&session, timeout)
                            .await
                            .map_err(|e| e.to_string())?;
                        let declared = zenkey_fleet::declared_entities(&session, timeout)
                            .await
                            .map_err(|e| e.to_string())?;
                        let (coverage, coverage_note) = match slices.as_deref() {
                            Some(set) => {
                                (zenkey_fleet::state_coverage(set, &base, &storages), None)
                            }
                            None => (
                                Vec::new(),
                                Some(
                                    "no registry slices resolved, so the declared state \
                                     families are unknown — this is \"not asked\", not \
                                     \"uncovered\" (RFC 09 §5.1 O4). Pass --registry, \
                                     or wait for the bus registry (RFC 08 §6)."
                                        .to_string(),
                                ),
                            ),
                        };
                        let topology = zenkey_fleet::topology(&session, timeout)
                            .await
                            .map_err(|e| e.to_string())?;
                        // The origin join (#131): part of the same explicit
                        // sweep the user asked for — one more admin GET.
                        let origins = zenkey_fleet::origin_attachments(&session, &base, timeout)
                            .await
                            .map_err(|e| e.to_string())?;
                        Ok(Arc::new(crate::admin::AdminSweep {
                            routers,
                            storage: zenkey_fleet::report::StorageList { storages, coverage },
                            declared,
                            coverage_note,
                            topology,
                            origins,
                            base,
                        }))
                    },
                    |out| Message::Admin(AdminMsg::Done(out)),
                )
            }
            AdminMsg::Done(outcome) => {
                let base = self.settings.base.clone();
                self.admin.finish(outcome, &base);
                Task::none()
            }
        }
    }

    fn start_monitor(&mut self) -> Task<Message> {
        let Some(session) = self.session.clone() else {
            return Task::none();
        };
        // Liveliness is always on — zero payload by construction (RFC 04 §5)
        // — and needs both the fleet sweep and @catalog by name (D4).
        let liveliness = if self.settings.base.is_empty() && self.bases.is_empty() {
            scope::liveliness_any_base()
        } else {
            scope::liveliness_selectors(&self.settings.base)
        };
        let max_keys = self.settings.max_keys;
        Task::perform(
            async move {
                Monitor::start(
                    &session,
                    MonitorSpec {
                        selectors: vec![], // lazy: no data-plane watches
                        liveliness,
                        max_keys,
                        ..Default::default()
                    },
                )
                .await
                .map(Arc::new)
                .map_err(|e| e.to_string())
            },
            Message::MonitorStarted,
        )
    }

    /// (Re)build the skeleton: slices are already loaded; roster + admin are
    /// gathered inside the task (both metadata-only).
    fn build_skeleton(&self) -> Task<Message> {
        let (Some(session), Some(slices)) = (self.session.clone(), self.slices.clone()) else {
            return Task::none();
        };
        let base = self.settings.base.clone();
        let timeout = self.settings.timeout();
        Task::perform(
            async move {
                let roster = zenkey_fleet::roster(&session, &base, timeout)
                    .await
                    .unwrap_or_default();
                let admin = zenkey_fleet::declared_entities(&session, timeout)
                    .await
                    .unwrap_or(None);
                let skeleton = Arc::new(Skeleton::build(&base, &slices, &roster, admin.as_ref()));
                Ok((skeleton, Arc::new(roster)))
            },
            Message::SkeletonBuilt,
        )
    }

    fn toggle_watch(&mut self, path: String) -> Task<Message> {
        let Some(monitor) = self.monitor.clone() else {
            return Task::none();
        };
        if let Some(id) = self.my_watches.remove(&path) {
            self.my_watch_paths.remove(&path);
            // A watch released mid-seed never gets its boundary (the engine
            // aborts the seed task) — forget it here too.
            self.seeding.remove(&id);
            self.seeding_paths.remove(&path);
            return Task::perform(
                async move { monitor.unwatch(id).await.map_err(|e| e.to_string()) },
                move |r| Message::WatchReleased(path.clone(), r),
            );
        }
        // Watching seeds (issue #92): current state arrives before live
        // traffic, through the same merge discipline as everything else.
        let selector = scope::subtree_selector(&path);
        let policy = zenkey_fleet::SeedPolicy {
            timeout: self.settings.timeout(),
            ..Default::default()
        };
        Task::perform(
            async move {
                monitor
                    .watch_seeded(&selector, policy)
                    .await
                    .map_err(|e| e.to_string())
            },
            move |r| Message::WatchStarted(path.clone(), r),
        )
    }

    fn watch_scope(&mut self) -> Task<Message> {
        let Some(monitor) = self.monitor.clone() else {
            return Task::none();
        };
        let selectors = self
            .settings
            .scope
            .selectors(&self.settings.base, &self.settings.selectors);
        // Scope watches seed too (issue #92) — the eager preset is "observe
        // this scope", and current state is part of observing it.
        let policy = zenkey_fleet::SeedPolicy {
            timeout: self.settings.timeout(),
            ..Default::default()
        };
        Task::perform(
            async move {
                let mut ids = Vec::new();
                for sel in &selectors {
                    match monitor.watch_seeded(sel, policy).await {
                        Ok(id) => ids.push(id),
                        Err(e) => tracing::warn!("scope watch {sel}: {e}"),
                    }
                }
                ids
            },
            Message::ScopeWatchesStarted,
        )
    }

    fn unwatch_scope(&mut self) -> Task<Message> {
        let Some(monitor) = self.monitor.clone() else {
            return Task::none();
        };
        let ids = std::mem::take(&mut self.scope_watches);
        for id in &ids {
            self.seeding.remove(id);
        }
        Task::perform(
            async move {
                for id in ids {
                    if let Err(e) = monitor.unwatch(id).await {
                        tracing::warn!("scope unwatch: {e}");
                    }
                }
                Ok(())
            },
            |r| Message::WatchReleased("(scope)".into(), r),
        )
    }

    /// Persist what the window looks like now. Best-effort by construction
    /// (see `Prefs::save`) — a preference that cannot be written must not fail
    /// whatever the user was actually doing.
    fn remember(&mut self) {
        self.prefs.scope = self.settings.scope;
        self.prefs.context = self
            .context_form
            .active
            .clone()
            .or(self.prefs.context.take());
        self.window_dirty = false;
        self.prefs.save();
    }

    /// Replay mode (issue #74). The transport verbs synthesize ticks from
    /// the loaded file and push them through the exact pipeline the live
    /// pump feeds — `apply_tick` — so the panes never know the difference.
    fn update_replay(&mut self, msg: view::replay::ReplayMsg) -> Task<Message> {
        use view::replay::ReplayMsg as R;
        match msg {
            R::OpenToggled => {
                self.replay_open = match self.replay_open {
                    Some(_) => None,
                    None => Some(String::new()),
                };
                self.replay_note = None;
                Task::none()
            }
            R::PathChanged(s) => {
                if let Some(p) = &mut self.replay_open {
                    *p = s;
                }
                Task::none()
            }
            R::Open => {
                let Some(path) = self
                    .replay_open
                    .as_deref()
                    .map(str::trim)
                    .filter(|p| !p.is_empty())
                    .map(str::to_string)
                else {
                    return Task::none();
                };
                let loaded = std::fs::File::open(&path)
                    .map_err(|e| e.to_string())
                    .and_then(|f| {
                        crate::replay::ReplayState::load(&path, std::io::BufReader::new(f))
                    });
                match loaded {
                    Ok(mut state) => {
                        self.replay_open = None;
                        self.replay_note = None;
                        // Mode honesty: the panes now show the file, from
                        // its start — nothing live bleeds through.
                        self.echo.clear();
                        self.history = None;
                        let tick = state.scrub_to(0);
                        self.replay = Some(state);
                        self.apply_tick(&tick);
                    }
                    Err(e) => self.replay_note = Some(e),
                }
                Task::none()
            }
            R::Toggled => {
                let tick = self.replay.as_mut().map(|r| {
                    // Play at the end means "from the top" — the one
                    // rewind that needs no scrubber.
                    if !r.playing && r.position_us >= r.span_us {
                        let t = r.scrub_to(0);
                        r.playing = true;
                        Some(t)
                    } else {
                        r.playing = !r.playing;
                        None
                    }
                });
                if let Some(Some(tick)) = tick {
                    self.echo.clear();
                    self.apply_tick(&tick);
                }
                Task::none()
            }
            R::SpeedSelected(s) => {
                if let Some(r) = &mut self.replay {
                    r.speed = s.0;
                }
                Task::none()
            }
            R::Scrubbed(t_us) => {
                let rewound = self.replay.as_ref().is_some_and(|r| t_us < r.position_us);
                let tick = self.replay.as_mut().map(|r| r.scrub_to(t_us));
                if let Some(tick) = tick {
                    if rewound {
                        // Backwards is a rebuild (LWW does not invert), and
                        // the scrollback rebuilds with it.
                        self.echo.clear();
                    }
                    self.apply_tick(&tick);
                }
                Task::none()
            }
            R::Advance => {
                let tick = self
                    .replay
                    .as_mut()
                    .filter(|r| r.playing)
                    .map(|r| r.advance(std::time::Duration::from_millis(250)));
                if let Some(tick) = tick {
                    self.apply_tick(&tick);
                }
                Task::none()
            }
            R::Exit => {
                self.replay = None;
                // The next live tick repaints the tree; the scrollback must
                // not mix file lines into it.
                self.echo.clear();
                Task::none()
            }
            R::RecordToggled => {
                if let Some(handle) = self.recording.take() {
                    handle.stop.notify_waiters();
                    return Task::none();
                }
                let Some(monitor) = self.monitor.clone() else {
                    return Task::none();
                };
                if self.replay.is_some() {
                    // Recording captures the live monitor; replay mode has
                    // nothing live to capture.
                    return Task::none();
                }
                let stop = Arc::new(tokio::sync::Notify::new());
                let path = format!(
                    "zengui-{}.zrec",
                    zenkey_fleet::record::rfc3339_now().replace(':', "-")
                );
                let base = self.settings.base.clone();
                self.recording = Some(RecordingHandle {
                    stop: Arc::clone(&stop),
                    path: path.clone(),
                });
                self.recorded = None;
                Task::perform(
                    async move {
                        let selectors: Vec<String> = monitor
                            .watched()
                            .await
                            .into_iter()
                            .map(|(_, s)| s)
                            .collect();
                        let header = zenkey_fleet::ZrecHeader {
                            zrec: zenkey_fleet::ZREC_VERSION,
                            selectors,
                            base,
                            captured_at: zenkey_fleet::record::rfc3339_now(),
                        };
                        let file = std::fs::File::create(&path).map_err(|e| e.to_string())?;
                        let mut writer =
                            zenkey_fleet::ZrecWriter::new(std::io::BufWriter::new(file), &header)
                                .map_err(|e| e.to_string())?;
                        let mut events = monitor.events();
                        let recording = zenkey_fleet::record(
                            &mut events,
                            &mut writer,
                            zenkey_fleet::RecordBounds::default(),
                            |_, _| {},
                        );
                        tokio::select! {
                            r = recording => r.map_err(|e| e.to_string())?,
                            _ = stop.notified() => {}
                        }
                        let (samples, dropped) = writer.counts();
                        writer.finish().map_err(|e| e.to_string())?;
                        Ok((samples, dropped, path))
                    },
                    |r| Message::Replay(R::RecordFinished(r)),
                )
            }
            R::RecordFinished(result) => {
                self.recording = None;
                self.recorded = Some(result);
                Task::none()
            }
        }
    }

    fn apply_tick(&mut self, tick: &BusTick) {
        // Per tick, not per frame: one bounded lock for one key's latency
        // summary (#119). None when unselected, unobserved, or unstamped.
        self.selected_latency = match (&self.selected, &self.monitor) {
            // During replay the live monitor's stats are about a different
            // world than the panes are showing — consulting them would put
            // live latency under file data (O4 in miniature).
            (Some(_), Some(_)) if self.replay.is_some() => None,
            (Some(key), Some(monitor)) => monitor
                .core()
                .with_stats(|s| s.get(key).map(|k| (k.latency(), k.unstamped)))
                .and_then(|(lat, unstamped)| lat.map(|l| (l, unstamped))),
            _ => None,
        };
        self.observed = Arc::clone(&tick.tree);
        self.keys = tick.keys;
        self.keys_evicted = tick.keys_evicted;
        self.keys_unwatched = tick.keys_unwatched;
        self.totals = tick.totals;
        self.watched = tick.watched.clone();
        for (id, coverage) in &tick.seeded {
            if let Some(path) = self.seeding.remove(id) {
                if let Some(path) = path {
                    self.seeding_paths.remove(&path);
                }
                self.seed_totals.0 += coverage.history_replies.unwrap_or(0);
                self.seed_totals.1 += coverage.storage_replies.unwrap_or(0);
                self.seed_totals.2 += coverage.superseded;
                self.seeded_watches += 1;
            }
        }
        // Two different facts about the same window (O6): the broadcast
        // outran us vs. our own batch cap chose to coalesce.
        self.echo.record_lag(tick.lagged);
        self.echo.record_coalesced(tick.coalesced);
        // One point per tick for the selected key's rate (issue #64). The
        // count is what says whether the EWMA moved: it never decays on its
        // own, so an unchanged count is silence, and the sampler records a gap
        // rather than a confident flat line.
        if let Some(rec) = self.history.as_ref() {
            let chunks: Vec<&str> = rec.key.split('/').collect();
            let observed = tick.tree.node(&chunks).map(|n| (n.count, n.rate_hz));
            self.rate_series.tick(observed);
        }
        for sample in &tick.samples {
            self.ensure_facts(&sample.key);
            self.echo.push(sample);
            // History is per-key and costs no subscription of its own: these
            // samples are already flowing for an existing watch (issue #63).
            if let Some(rec) = self.history.as_mut() {
                rec.observe(sample);
            }
            // The media viewer's frames arrive on the exact key it watches
            // (issue #69) — same pipeline, no extra subscription.
            if let Some(v) = self.media.viewing.as_mut()
                && v.key == sample.key
            {
                v.on_frame(sample);
            }
        }
        for (key, _up) in &tick.nodes {
            self.ensure_facts(key);
        }
        // The node dashboard (#61): transitions in arrival order (flap-
        // correct), then the zero-cost watched-freshness join.
        let now = std::time::Instant::now();
        self.roster
            .apply_transitions(&self.settings.base, &tick.nodes, now);
        self.roster
            .refresh(&tick.tree, &self.settings.base, &tick.watched, now);
        self.reflatten();
    }

    fn reflatten(&mut self) {
        let empty_skeleton;
        let skeleton = match &self.skeleton {
            Some(s) => s.as_ref(),
            None => {
                empty_skeleton = Skeleton::build(
                    &self.settings.base,
                    &SliceSet::default(),
                    &std::collections::BTreeMap::new(),
                    None,
                );
                &empty_skeleton
            }
        };
        let merged = zenkey_fleet::skeleton::merge(skeleton, &self.observed, &self.watched);
        let now = std::time::Instant::now();
        // Pivot and filter re-key the flattened entries here — the hot tree
        // stays registry-blind (issue #65).
        self.flat = match (self.pivot, self.tree_search.is_empty()) {
            (view::tree::Pivot::Chunks, true) => {
                view::tree::flatten(&merged, &self.settings.base, &self.expanded, MAX_ROWS, now)
            }
            (view::tree::Pivot::Chunks, false) => view::tree::search_flatten(
                &merged,
                &self.settings.base,
                &self.tree_search,
                MAX_ROWS,
                now,
            ),
            (pivot, _) => view::tree::pivot_flatten(
                &merged,
                &self.settings.base,
                pivot,
                &self.expanded,
                &self.tree_search,
                MAX_ROWS,
                now,
            ),
        };
    }

    fn ensure_facts(&mut self, key: &str) {
        // One line, and the bound lives in the engine with the counter that
        // reports it (#107). This is still the single insert point.
        let base = self.settings.base.clone();
        self.facts.ensure(&base, key, self.slices.as_deref());
    }

    fn reresolve_registrations(&mut self) {
        let Some(slices) = self.slices.clone() else {
            return;
        };
        self.facts.resolve_all(&slices);
    }

    fn load_slices(&self) -> Task<Message> {
        let dirs = self.settings.registry.clone();
        let Some(session) = self.session.clone() else {
            return Task::none();
        };
        let base = self.settings.base.clone();
        let timeout = self.settings.timeout();
        if !dirs.is_empty() {
            // The §6.1 union (issue #43): served wins, dirs fill, and the
            // disagreement count reaches the status strip as data.
            return Task::perform(
                async move {
                    SliceSet::from_union(&session, &base, &dirs, timeout)
                        .await
                        .map(|out| {
                            (
                                Arc::new(out.set),
                                out.from_bus.len(),
                                out.dirs_only.len(),
                                out.disagreements.len(),
                            )
                        })
                        .map_err(|e| e.to_string())
                },
                Message::SlicesUnionLoaded,
            );
        }
        Task::perform(
            async move {
                SliceSet::from_bus(&session, &base, timeout)
                    .await
                    .map(Arc::new)
                    .map_err(|e| e.to_string())
            },
            Message::SlicesLoaded,
        )
    }

    pub fn subscription(&self) -> Subscription<Message> {
        let mut subs = Vec::new();
        // Replay mode replaces the link (#74): while a `.zrec` feeds the
        // panes, the live pump is not built at all — the mode cannot leak.
        if self.replay.is_none()
            && let Some(monitor) = self.monitor.clone()
        {
            subs.push(link::subscribe(LinkKey {
                monitor,
                epoch: self.epoch,
            }));
        }
        // The play clock, only while actually playing (a paused replay
        // costs nothing) — the same cadence as the live stats tick, so the
        // panes tick at the rate they were built for.
        if self.replay.as_ref().is_some_and(|r| r.playing) {
            subs.push(
                iced::time::every(std::time::Duration::from_millis(250))
                    .map(|_| Message::Replay(view::replay::ReplayMsg::Advance)),
            );
        }
        // The repeat clock for a sustained publish (#60). It exists only while
        // a publication is armed, so an idle pane costs nothing.
        if self.publish_form.armed {
            let period = std::time::Duration::from_secs_f64(self.publish_form.interval_secs());
            subs.push(iced::time::every(period).map(|_| Message::PublishTick));
        }
        // Window geometry, for the next launch (issue #73).
        subs.push(
            iced::window::resize_events()
                .map(|(_, size)| Message::WindowResized(size.width, size.height)),
        );
        // …and the settle timer that actually writes it, which exists only
        // while a resize is outstanding (issue #189). One file write per drag
        // rather than per pixel, and none at all while the window is still.
        if self.window_dirty {
            subs.push(
                iced::time::every(std::time::Duration::from_millis(700))
                    .map(|_| Message::WindowSettled),
            );
        }
        // Keyboard shortcuts (issues #73, #75). `listen` only sees events no
        // widget consumed, so a shortcut can never steal a keystroke from the
        // text box the user is typing in.
        // Key presses arrive raw: `listen` only sees what no widget consumed,
        // and iced's subscription closures cannot capture, so the *meaning* of
        // a press is decided in `update` where the state is (#75's Esc
        // layering needs to know what is open).
        subs.push(iced::keyboard::listen().filter_map(|event| match event {
            iced::keyboard::Event::KeyPressed { key, modifiers, .. } => {
                Some(Message::Key(key, modifiers))
            }
            _ => None,
        }));
        Subscription::batch(subs)
    }

    pub fn view(&self) -> Element<'_, Message> {
        let panes = row![
            iced::widget::container(view::tree::pane(
                &self.flat,
                &self.facts,
                self.selected.as_deref(),
                &self.my_watch_paths,
                &self.seeding_paths,
                self.pivot,
                &self.tree_search,
                self.tree_scroll.0,
                self.tree_scroll.1,
            ))
            .width(Length::FillPortion(self.prefs.split_portions().0))
            .height(Length::Fill),
            iced::widget::container(match self.right_pane {
                RightPane::Echo => view::echo::pane(
                    &self.echo,
                    &self.echo_view,
                    self.selected.as_deref(),
                    self.echo.next_seq(),
                ),
                RightPane::Call =>
                    view::call::pane(&self.call_form, self.slices.as_deref(), &self.roster,),
                RightPane::Publish =>
                    view::publish::pane(&self.publish_form, self.slices.is_some()),
                RightPane::Detail => view::detail::pane(view::detail::DetailData {
                    key: self.selected.as_deref().unwrap_or("(nothing selected)"),
                    facts: self.selected.as_deref().and_then(|k| self.facts.get(k)),
                    fetched: self.fetched.as_ref().and_then(|(k, o)| {
                        (Some(k.as_str()) == self.selected.as_deref()).then_some(o)
                    }),
                    decoded: self.decoded.as_ref(),
                    series: self.series_data(),
                    history_entries: self.history.as_ref().map(|r| r.ring.len()),
                    observed: self.history.as_ref().and_then(|r| r.ring.newest()),
                    latency: self.selected_latency,
                }),
                RightPane::Nodes => view::nodes::pane(view::nodes::NodesData {
                    roster: &self.roster,
                    selected: self.node_selected.as_deref(),
                    detail: &self.node_detail,
                }),
                RightPane::Doctor => view::doctor::pane(&self.doctor, &self.settings.base),
                RightPane::Blob => view::blob::pane(&self.blob, self.slices.is_some()),
                RightPane::Media => view::media::pane(&self.media, self.slices.as_deref()),
                RightPane::Admin => view::admin::pane(&self.admin),
                RightPane::History => view::history::pane(view::history::HistoryData {
                    key: self.selected.as_deref(),
                    recorder: self.history.as_ref(),
                    watched: self
                        .selected
                        .as_deref()
                        .is_some_and(|k| key_is_watched(&self.watched, k)),
                }),
                RightPane::Connect =>
                    view::contexts::pane(&self.context_form, self.settings.is_unreachable(),),
            })
            .width(Length::FillPortion(self.prefs.split_portions().1))
            .height(Length::Fill),
        ]
        .spacing(space::MD);

        let mut layout = column![self.toolbar()]
            .spacing(space::MD)
            .padding(space::MD);
        // Replay-mode surfaces (#74), between the toolbar and the panes so
        // the mode is unmistakable: the open row, the REPLAY banner with
        // the scrubber, and the capture status line.
        if let Some(path) = &self.replay_open {
            layout = layout.push(view::replay::open_row(path));
            if let Some(note) = &self.replay_note {
                layout = layout.push(view::kit::muted(format!("could not open: {note}")));
            }
        }
        if let Some(replay) = &self.replay {
            layout = layout.push(view::replay::banner(replay));
        }
        if let Some(rec) = &self.recording {
            layout = layout.push(view::kit::muted(format!(
                "● recording current watches to {} — toolbar 'stop recording' finishes the file",
                rec.path
            )));
        }
        if let Some(done) = &self.recorded {
            layout = layout.push(view::kit::muted(match done {
                Ok((samples, dropped, path)) => format!(
                    "recorded {samples} sample(s) to {path} ({dropped} dropped — in-file ledger)"
                ),
                Err(e) => format!("recording failed: {e}"),
            }));
        }
        let layout = layout.push(panes).push(view::status::strip(Status {
            link: &self.link,
            base_label: self.settings.base_label(),
            scope_label: self.settings.scope.short(),
            keys: self.keys,
            keys_evicted: self.keys_evicted,
            keys_unwatched: self.keys_unwatched,
            facts_cached: self.facts.len(),
            facts_evicted: self.facts.evicted(),
            watched: &self.watched,
            skeleton: self.skeleton.as_deref().map(|s| s.coverage),
            fetched: self.fetched.as_ref(),
            totals: self.totals,
            slices: &self.slice_source,
            seeding: self.seeding.len(),
            seeded_watches: self.seeded_watches,
            seed_totals: self.seed_totals,
            unreachable: self.settings.is_unreachable(),
            prefs_note: self.prefs_note.as_deref(),
            replaying: self.replay.is_some(),
        }));

        // The overlay floats above everything (#75). `stack` rather than a
        // modal widget because the layering rule is ours — palette above
        // panes, Esc peeling one layer at a time — and a widget with its own
        // dismissal policy would fight it.
        // A lazy iterator: a closed overlay never touches the cache, and
        // the open one clones only what it draws (#110).
        match view::palette::overlay(&self.palette, &self.context_form.known, self.facts.keys()) {
            None => layout.into(),
            Some(overlay) => iced::widget::stack![
                layout,
                iced::widget::container(overlay)
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .align_x(iced::alignment::Horizontal::Center)
                    .padding(space::XL),
            ]
            .into(),
        }
    }

    fn toolbar(&self) -> Element<'_, Message> {
        let mut options: Vec<String> = vec![String::new()];
        options.extend(self.bases.iter().map(|b| b.base.clone()));
        options.dedup();

        let base_picker = pick_list(
            options,
            Some(self.settings.base.clone()),
            Message::BaseSelected,
        )
        .placeholder("base")
        .text_size(crate::view::tokens::font::CAPTION);

        let scopes = vec![
            ScopePreset::Everything,
            ScopePreset::Deployment,
            ScopePreset::Telemetry,
            ScopePreset::State,
            ScopePreset::Events,
        ];
        let scope_picker = pick_list(scopes, Some(self.settings.scope), Message::ScopeSelected)
            .text_size(crate::view::tokens::font::CAPTION);

        // Observation is opt-in and labelled by its cost (issue #85).
        let observing = !self.scope_watches.is_empty();
        let observe = iced::widget::button(
            text(if observing {
                "stop observing scope"
            } else {
                "observe scope"
            })
            .size(crate::view::tokens::font::CAPTION),
        )
        .on_press(Message::ScopeWatchToggled)
        .padding(4);

        row![
            text("base").size(crate::view::tokens::font::CAPTION),
            base_picker,
            text("scope").size(crate::view::tokens::font::CAPTION),
            scope_picker,
            observe,
            iced::widget::Row::from_iter(RightPane::ALL.into_iter().map(|p| {
                crate::view::kit::tab(p.label(), self.right_pane == p, Message::PaneSelected(p))
            }))
            .spacing(space::XS),
            crate::view::kit::muted(self.settings.scope.label()),
            // Capture and replay (#74): record writes the current watches
            // to a .zrec; replay feeds the panes from one.
            iced::widget::button(
                text(if self.recording.is_some() {
                    "stop recording"
                } else {
                    "record"
                })
                .size(crate::view::tokens::font::CAPTION)
            )
            .on_press(Message::Replay(view::replay::ReplayMsg::RecordToggled))
            .padding(4),
            iced::widget::button(text("replay…").size(crate::view::tokens::font::CAPTION))
                .on_press(Message::Replay(view::replay::ReplayMsg::OpenToggled))
                .padding(4),
            iced::widget::space::horizontal(),
            // Window preferences (issue #73): the theme name is the button,
            // so the label says what you get rather than what you have.
            iced::widget::button(
                text(format!("theme: {}", self.prefs.theme.label()))
                    .size(crate::view::tokens::font::CAPTION)
            )
            .on_press(Message::Prefs(crate::message::PrefsMsg::ThemeToggled))
            .padding(4),
            iced::widget::button(text("-").size(crate::view::tokens::font::CAPTION))
                .on_press(Message::Prefs(crate::message::PrefsMsg::ZoomOut))
                .padding(4),
            iced::widget::button(
                text(format!("{}%", (self.prefs.zoom * 100.0).round() as i32))
                    .size(crate::view::tokens::font::CAPTION)
            )
            .on_press(Message::Prefs(crate::message::PrefsMsg::ZoomReset))
            .padding(4),
            iced::widget::button(text("+").size(crate::view::tokens::font::CAPTION))
                .on_press(Message::Prefs(crate::message::PrefsMsg::ZoomIn))
                .padding(4),
            iced::widget::button(text("reconnect").size(crate::view::tokens::font::CAPTION))
                .on_press(Message::Reconnect)
                .padding(4),
        ]
        .spacing(space::SM)
        .align_y(iced::Alignment::Center)
        .into()
    }
}

impl Zengui {
    /// Derive the detail pane's sparkline data from the recorded history
    /// (issue #64).
    ///
    /// Computed per frame rather than kept: the ring is the single source, and
    /// a cached series would be one more thing to invalidate on every eviction.
    /// The leaves come from the newest payload, so a producer that starts
    /// emitting a new field offers it without a restart.
    fn series_data(&self) -> Option<view::detail::SeriesData> {
        let rec = self.history.as_ref()?;
        // The most recent entry that *is* a document, not simply the most
        // recent one: a tombstone carries no fields, and letting it empty the
        // picker would make the chart vanish on every retirement and come
        // back on the next put.
        let leaves = rec
            .ring
            .iter()
            .find_map(|e| e.value.as_ref())
            .map(crate::series::numeric_leaves)
            .unwrap_or_default();
        // The chosen leaf, if the newest payload still carries it — a field
        // that disappeared should not silently keep plotting its own gaps.
        let leaf = self
            .series_leaf
            .as_ref()
            .filter(|p| leaves.leaves.iter().any(|(k, _)| k == *p))
            .cloned()
            .or_else(|| leaves.leaves.first().map(|(k, _)| k.clone()));
        let value = match &leaf {
            Some(p) => crate::series::value_series(&rec.ring, p),
            None => crate::series::Series::new(),
        };
        let unit = match self.facts.get(&rec.key).map(|f| &f.registration) {
            Some(zenkey_fleet::Registration::Registered(s)) => s.unit.clone(),
            _ => None,
        };
        Some(view::detail::SeriesData {
            leaves,
            leaf,
            value,
            rate: self.rate_series.series().clone(),
            unit,
        })
    }
}

/// Whether any active watch selector covers this exact key.
///
/// The distinction the history pane rests on: with no watch, no sample can
/// reach the recorder, and "nothing recorded yet" would be a verdict the tool
/// never obtained (RFC 09 §5.1 O4). A selector that does not parse cannot be
/// claimed as coverage, so it is not counted — the same rule
/// `StatsTable::retire_unwatched` applies from the other side.
fn key_is_watched(watched: &[String], key: &str) -> bool {
    let Ok(ke) = zenoh::key_expr::KeyExpr::new(key.to_string()) else {
        return false;
    };
    watched
        .iter()
        .filter_map(|sel| zenoh::key_expr::KeyExpr::new(sel.clone()).ok())
        .any(|sel| sel.intersects(&ke))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn watch_coverage_is_decided_by_intersection_not_by_prefix() {
        let watched = vec![
            "v1/h-a/telemetry/**".to_string(),
            "demo/example/foo".to_string(),
        ];
        assert!(key_is_watched(&watched, "v1/h-a/telemetry/sysinfo/cpu"));
        assert!(key_is_watched(&watched, "demo/example/foo"));
        assert!(!key_is_watched(&watched, "v1/h-b/telemetry/sysinfo/cpu"));
        assert!(!key_is_watched(&[], "v1/h-a/telemetry/sysinfo/cpu"));
    }

    /// A selector we cannot parse is not coverage we can claim.
    #[test]
    fn an_unparseable_selector_is_not_counted_as_coverage() {
        assert!(!key_is_watched(&["not a key/**/".to_string()], "a/b"));
    }
}

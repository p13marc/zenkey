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
use crate::keyfacts::KeyFacts;
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
    /// The last on-demand fetch: (key, outcome-or-error).
    fetched: Option<(String, Result<Arc<FetchOutcome>, String>)>,
    echo_filter: String,

    call_form: view::call::CallForm,
    /// Process-lifetime schema cache (RFC 08 §7), rebuilt on base change.
    schema_store: Option<Arc<zenkey_fleet::decode::SchemaStore>>,
    /// The decode of the last fetched value.
    decoded: Option<(Option<String>, zenkey_fleet::decode::Rendering)>,
    /// Which right-hand pane is showing.
    right_pane: RightPane,
    facts: HashMap<String, KeyFacts>,
    slices: Option<Arc<SliceSet>>,
    slice_source: SliceSource,

    bases: Vec<DiscoveredBase>,
    keys: usize,
    keys_evicted: u64,
    keys_unwatched: u64,
    totals: (u64, u64, f64),
}

impl Zengui {
    pub fn new(settings: Settings) -> (Zengui, Task<Message>) {
        let echo = EchoRing::new(settings.echo_lines);
        let connect = settings.connect.clone();
        let listen = settings.listen.clone();
        let scouting = settings.scouting;
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
            fetched: None,
            echo_filter: String::new(),
            call_form: view::call::CallForm::default(),
            schema_store: None,
            decoded: None,
            right_pane: RightPane::Echo,
            facts: HashMap::new(),
            slices: None,
            slice_source: SliceSource::None,
            bases: Vec::new(),
            keys: 0,
            keys_evicted: 0,
            keys_unwatched: 0,
            totals: (0, 0, 0.0),
        };
        let open = Task::perform(
            async move {
                zenkey_fleet::session::open(&connect, &listen, scouting)
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
        iced::Theme::Dark
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::SessionOpened(Ok(session)) => {
                self.session = Some(session.clone());
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
            Message::SkeletonBuilt(Ok(skeleton)) => {
                self.skeleton = Some(skeleton);
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
                self.decoded = None;
                self.right_pane = RightPane::Detail;
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
                                    let (ty, rendering) = zenkey_fleet::decode::decode_sample(
                                        &store,
                                        &session,
                                        &slices,
                                        &base,
                                        &wire_key,
                                        Some(&encoding),
                                        &bytes.to_bytes(),
                                    )
                                    .await;
                                    (fkey, ty, Arc::new(rendering))
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
                // monitor is the obviously-correct restart.
                self.facts.clear();
                self.my_watches.clear();
                self.my_watch_paths.clear();
                self.scope_watches.clear();
                self.seeding.clear();
                self.seeding_paths.clear();
                self.seed_totals = (0, 0, 0);
                self.seeded_watches = 0;
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
            Message::CallDone(outcome) => {
                self.call_form.in_flight = false;
                self.call_form.outcome =
                    Some(outcome.map(|r| (*r).clone()).map_err(|e| e.to_string()));
                Task::none()
            }
            Message::EchoFilterChanged(f) => {
                self.echo_filter = f;
                Task::none()
            }
            Message::ClearEcho => {
                self.echo.clear();
                Task::none()
            }
            Message::PaneSelected(pane) => {
                self.right_pane = pane;
                Task::none()
            }
            Message::Reconnect => {
                self.my_watches.clear();
                self.my_watch_paths.clear();
                self.scope_watches.clear();
                self.seeding.clear();
                self.seeding_paths.clear();
                self.seed_totals = (0, 0, 0);
                self.seeded_watches = 0;
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
                self.call_form.procedure = Some(p);
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
                Ok(Arc::new(Skeleton::build(
                    &base,
                    &slices,
                    &roster,
                    admin.as_ref(),
                )))
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

    fn apply_tick(&mut self, tick: &BusTick) {
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
        for sample in &tick.samples {
            self.ensure_facts(&sample.key);
            self.echo.push(sample);
        }
        for (key, _up) in &tick.nodes {
            self.ensure_facts(key);
        }
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
        if self.facts.contains_key(key) {
            return;
        }
        let mut facts = KeyFacts::project(&self.settings.base, key);
        if let Some(slices) = &self.slices {
            facts.resolve(slices);
        }
        self.facts.insert(key.to_string(), facts);
    }

    fn reresolve_registrations(&mut self) {
        let Some(slices) = self.slices.clone() else {
            return;
        };
        for facts in self.facts.values_mut() {
            facts.resolve(&slices);
        }
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
        let Some(monitor) = self.monitor.clone() else {
            return Subscription::none();
        };
        link::subscribe(LinkKey {
            monitor,
            epoch: self.epoch,
        })
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
            .width(Length::FillPortion(1))
            .height(Length::Fill),
            iced::widget::container(match self.right_pane {
                RightPane::Echo =>
                    view::echo::pane(&self.echo, &self.echo_filter, self.selected.as_deref(),),
                RightPane::Call => view::call::pane(&self.call_form, self.slices.as_deref()),
                RightPane::Detail => view::detail::pane(view::detail::DetailData {
                    key: self.selected.as_deref().unwrap_or("(nothing selected)"),
                    facts: self.selected.as_deref().and_then(|k| self.facts.get(k)),
                    fetched: self.fetched.as_ref().and_then(|(k, o)| {
                        (Some(k.as_str()) == self.selected.as_deref()).then_some(o)
                    }),
                    decoded: self.decoded.as_ref(),
                }),
            })
            .width(Length::FillPortion(1))
            .height(Length::Fill),
        ]
        .spacing(space::MD);

        column![
            self.toolbar(),
            panes,
            view::status::strip(Status {
                link: &self.link,
                base_label: self.settings.base_label(),
                scope_label: self.settings.scope.short(),
                keys: self.keys,
                keys_evicted: self.keys_evicted,
                keys_unwatched: self.keys_unwatched,
                watched: &self.watched,
                skeleton: self.skeleton.as_deref().map(|s| s.coverage),
                fetched: self.fetched.as_ref(),
                totals: self.totals,
                slices: &self.slice_source,
                seeding: self.seeding.len(),
                seeded_watches: self.seeded_watches,
                seed_totals: self.seed_totals,
                unreachable: self.settings.is_unreachable(),
            }),
        ]
        .spacing(space::MD)
        .padding(space::MD)
        .into()
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
            iced::widget::space::horizontal(),
            iced::widget::button(text("reconnect").size(crate::view::tokens::font::CAPTION))
                .on_press(Message::Reconnect)
                .padding(4),
        ]
        .spacing(space::SM)
        .align_y(iced::Alignment::Center)
        .into()
    }
}

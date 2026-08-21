//! The Elm loop: state, `update`, `view`, `subscription`.
//!
//! **Lazy by default** (issue #85): connecting builds the *skeleton* — the
//! declared keyspace from registry + liveliness + admin metadata — and starts
//! a monitor with **zero data-plane watches**. Observation is opt-in per
//! subtree (the tree's watch toggles) or per scope (the toolbar's "observe
//! scope" toggle — the old eager mode made explicit); a selection fetches one
//! value on demand. `--eager` restores the bootstrap behavior from the
//! command line, labelled by its cost.

use std::sync::Arc;

use iced::widget::{column, pick_list, row, text};
use iced::{Element, Length, Subscription, Task};

use crate::config::Settings;
use crate::link::{self, LinkKey};
use crate::message::{LinkState, Message};
use crate::scope::{self, ScopePreset};
use crate::services;
use crate::state::{Chrome, Deployment, Observation, Subject, TreeState, Workspace};
use crate::update::{self, Ctx};
use crate::view;
use crate::view::status::{SliceSource, Status};
use crate::view::tokens::space;

use crate::message::{
    BusMsg, ChromeMsg, DeploymentMsg, PaneMsg, RightPane, SubjectMsg, WorkspaceMsg,
};

pub struct Zengui {
    chrome: Chrome,
    dep: Deployment,
    obs: Observation,
    sub: Subject,
    tree: TreeState,
    work: Workspace,
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
        let echo_lines = settings.echo_lines;
        // Read before `settings` is moved into the deployment.
        let connect = settings.connect.clone();
        let listen = settings.listen.clone();
        let scouting = settings.scouting;
        let zenoh_config = settings.zenoh_config.clone();
        let app = Zengui {
            chrome: Chrome::new(prefs, prefs_note),
            dep: Deployment::new(settings),
            obs: Observation::default(),
            sub: Subject::default(),
            tree: TreeState::default(),
            work: Workspace::new(echo_lines),
        };
        (
            app,
            services::link::open(zenoh_config, connect, listen, scouting),
        )
    }

    pub fn title(&self) -> String {
        format!("zengui — {}", self.dep.base_label())
    }

    pub fn theme(&self) -> iced::Theme {
        self.chrome.prefs.theme.theme()
    }

    /// The UI scale factor iced applies to the whole window (issue #73).
    pub fn scale_factor(&self) -> f32 {
        self.chrome.prefs.zoom
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Bus(m) => self.update_bus(m),
            Message::Subject(m) => self.update_subject(m),
            Message::Deployment(m) => self.update_deployment(m),
            Message::Workspace(m) => self.update_workspace(m),
            Message::Pane(m) => self.update_pane(m),
            Message::Chrome(m) => self.update_chrome(m),
        }
    }

    /// Everything the world answered — a task landing or a `link.rs` yield.
    fn update_bus(&mut self, msg: BusMsg) -> Task<Message> {
        match msg {
            BusMsg::SessionOpened(Ok(session)) => {
                // A new session is a new everything: the base may differ, so
                // every projection, roster and verdict from the old one is
                // evidence about a different deployment (O4).
                //
                // This variant has exactly two producers — the launch open and
                // the context switch — and forgetting on the launch one is a
                // no-op over a `Deployment` nothing has written yet. So it can
                // live here, which removes the worst cross-group reach in the
                // file: a pane resetting the app.
                self.forget_deployment();
                self.dep.session = Some(session.clone());
                // The context list is read from the shared file, not cached at
                // launch: `zenctl context create` on the other side of the
                // screen should show up here without a restart (#67).
                update::pane::context::refresh_contexts(&mut self.work.bench.context_form);
                self.dep.schema_store = Some(Arc::new(zenkey_fleet::decode::SchemaStore::new(
                    self.dep.base(),
                    self.dep.timeout(),
                )));
                let discover = services::link::discover_bases(&session, self.dep.timeout());
                Task::batch([discover, self.start_monitor(), self.load_slices()])
            }
            BusMsg::SessionOpened(Err(e)) => {
                self.obs.link = LinkState::Failed(e);
                Task::none()
            }
            BusMsg::MonitorStarted(Ok(monitor)) => {
                self.obs.monitor = Some(Arc::clone(&monitor));
                self.obs.epoch += 1;
                if self.dep.settings.eager {
                    return self.watch_scope();
                }
                Task::none()
            }
            BusMsg::MonitorStarted(Err(e)) => {
                self.obs.link = LinkState::Failed(e);
                Task::none()
            }
            BusMsg::SkeletonBuilt(Ok((skeleton, roster))) => {
                self.dep.skeleton = Some(skeleton);
                // The build task gathered the roster anyway — seed the node
                // dashboard from it instead of throwing it away (#61).
                self.work.verdicts.roster.seed(&roster);
                self.tree.reflatten(&self.dep, &self.obs);
                Task::none()
            }
            BusMsg::SkeletonBuilt(Err(e)) => {
                tracing::warn!("skeleton build failed: {e}");
                Task::none()
            }
            BusMsg::BasesDiscovered(Ok(bases)) => {
                self.dep.bases = bases;
                self.rebuild_base_options();
                Task::none()
            }
            BusMsg::BasesDiscovered(Err(e)) => {
                tracing::warn!("base discovery failed: {e}");
                Task::none()
            }
            BusMsg::SlicesLoaded(Ok(slices)) => {
                self.dep.slice_source = if self.dep.settings.registry.is_empty() {
                    SliceSource::Bus {
                        count: slices.slices().len(),
                    }
                } else {
                    SliceSource::Dirs {
                        count: slices.slices().len(),
                    }
                };
                self.dep.slices = Some(slices);
                self.reresolve_registrations();
                self.refresh_blob_list();
                // The skeleton is built FROM the slices — (re)build it now.
                self.build_skeleton()
            }
            BusMsg::SlicesUnionLoaded(Ok((slices, from_bus, dirs_only, disagreements))) => {
                self.dep.slice_source = SliceSource::Union {
                    from_bus,
                    dirs_only,
                    disagreements,
                };
                self.dep.slices = Some(slices);
                self.reresolve_registrations();
                self.refresh_blob_list();
                self.build_skeleton()
            }
            BusMsg::SlicesUnionLoaded(Err(e)) => {
                self.dep.slice_source = SliceSource::Failed(e);
                Task::none()
            }
            BusMsg::SlicesLoaded(Err(e)) => {
                self.dep.slice_source = SliceSource::Failed(e);
                Task::none()
            }
            BusMsg::Link(state) => {
                self.obs.link = state;
                Task::none()
            }
            BusMsg::Tick(tick) => {
                update::bus::apply_tick(
                    &mut self.dep,
                    &mut self.obs,
                    &mut self.sub,
                    &mut self.tree,
                    &mut self.work,
                    &tick,
                );
                Task::none()
            }
        }
    }

    /// What the app is pointed at, and the coverage that follows.
    fn update_deployment(&mut self, msg: DeploymentMsg) -> Task<Message> {
        match msg {
            DeploymentMsg::ScopeWatchesStarted(ids) => {
                for id in &ids {
                    self.obs.seeding.insert(*id, None);
                }
                self.obs.scope_watches = ids;
                Task::none()
            }
            DeploymentMsg::ScopeWatchesReleased(Ok(())) => Task::none(),
            DeploymentMsg::ScopeWatchesReleased(Err(e)) => {
                tracing::warn!("releasing the scope watches failed: {e}");
                Task::none()
            }
            DeploymentMsg::ContextApplied { name, stored } => {
                self.apply_context(*stored);
                self.chrome.prefs.context = name;
                self.remember();
                self.reopen_session()
            }
            DeploymentMsg::ScopeWatchToggled => {
                if self.obs.scope_watches.is_empty() {
                    self.watch_scope()
                } else {
                    self.unwatch_scope()
                }
            }
            DeploymentMsg::BaseSelected(base) => {
                if base == self.dep.base() {
                    return Task::none();
                }
                self.dep.settings.base = base;
                // The base is an input to every projection and to the
                // skeleton, and watch selectors are base-relative: a fresh
                // monitor is the obviously-correct restart. Everything the old
                // base taught us is evidence about a different deployment (O4).
                self.forget_deployment();
                self.dep.schema_store = Some(Arc::new(zenkey_fleet::decode::SchemaStore::new(
                    self.dep.base(),
                    self.dep.timeout(),
                )));
                self.sub.decoded = None;
                self.tree.reflatten(&self.dep, &self.obs);
                Task::batch([self.start_monitor(), self.load_slices()])
            }
            DeploymentMsg::ScopeSelected(scope) => {
                if scope == self.dep.settings.scope {
                    return Task::none();
                }
                self.dep.settings.scope = scope;
                // Remembered for the next launch (issue #73).
                self.remember();
                // If the scope is being observed, re-point the observation.
                if !self.obs.scope_watches.is_empty() {
                    let release = self.unwatch_scope();
                    let acquire = self.watch_scope();
                    return Task::batch([release, acquire]);
                }
                Task::none()
            }
            DeploymentMsg::Reconnect => {
                self.forget_deployment();
                self.start_monitor()
            }
        }
    }

    /// One key: chosen, observed, fetched, decoded.
    fn update_subject(&mut self, msg: SubjectMsg) -> Task<Message> {
        match msg {
            SubjectMsg::WatchToggled(path) => self.toggle_watch(path),
            SubjectMsg::WatchStarted(path, Ok(id)) => {
                self.obs.my_watch_paths.insert(path.clone());
                self.obs.seeding.insert(id, Some(path.clone()));
                self.obs.seeding_paths.insert(path.clone());
                self.obs.my_watches.insert(path, id);
                self.tree.reflatten(&self.dep, &self.obs);
                Task::none()
            }
            SubjectMsg::WatchStarted(path, Err(e)) => {
                tracing::warn!("watch {path} failed: {e}");
                Task::none()
            }
            SubjectMsg::WatchReleased(_, Ok(())) => Task::none(),
            SubjectMsg::WatchReleased(path, Err(e)) => {
                tracing::warn!("unwatch {path} failed: {e}");
                Task::none()
            }
            SubjectMsg::SelectPath(path) => {
                self.sub.selected = Some(path);
                Task::none()
            }
            SubjectMsg::ValueFetched(key, outcome) => {
                // No base guard needed (#109 audit): the evidence is keyed by
                // the full wire key, which names its own base (an explorer
                // runs un-namespaced, RFC 09 §5), and the view passes
                // `fetched` through only while that exact key is selected.
                // Residual: a stale landing can still flip the right pane to
                // Detail — a focus nit, not a misattributed verdict.
                self.sub.decoded = None;
                // A fetch normally lands the detail pane in view — except
                // from the doctor's click-through, where losing the finding
                // list would cost more than it shows (#71).
                if self.work.right_pane != RightPane::Doctor {
                    self.work.right_pane = RightPane::Detail;
                }
                let decode_task = match (
                    &outcome,
                    &self.dep.session,
                    &self.dep.schema_store,
                    &self.dep.slices,
                ) {
                    (Ok(out), Some(session), Some(store), Some(slices)) => {
                        if let zenkey_fleet::FetchOutcome::Value(v) = out.as_ref() {
                            services::value::decode(
                                Arc::clone(store),
                                session.clone(),
                                Arc::clone(slices),
                                self.dep.base().to_string(),
                                key.clone(),
                                v.key.clone(),
                                v.encoding.clone(),
                                v.payload.clone(),
                            )
                        } else {
                            Task::none()
                        }
                    }
                    _ => Task::none(),
                };
                self.sub.fetched = Some((key, outcome));
                decode_task
            }
            SubjectMsg::ValueDecoded(key, type_name, rendering) => {
                // Stale guard: only the currently selected key's decode lands.
                if self.sub.selected.as_deref() == Some(key.as_str()) {
                    self.sub.decoded = Some((type_name, (*rendering).clone()));
                }
                Task::none()
            }
            SubjectMsg::SelectKey(key) => {
                self.sub.selected = key.clone();
                // The old key's latency summary is not evidence about the
                // new one — cleared now, refreshed on the next tick (#119).
                self.sub.selected_latency = None;
                // History follows the selection and nothing else (issue #63):
                // the previous recording is dropped here, which is what makes
                // deselecting free. A symbolic skeleton path names no concrete
                // key, so nothing can be recorded for it.
                self.sub.history = key.as_deref().filter(|k| !k.contains('{')).map(|k| {
                    crate::history::HistoryRecorder::new(k, self.dep.settings.history_entries)
                });
                // The plotted series belong to the same selection (issue #64):
                // they start empty, and stop being fed when it goes away.
                self.sub.rate_series = crate::series::RateSampler::new();
                self.sub.series_leaf = None;
                self.sub.refresh_series(&self.dep);
                let (Some(session), Some(key)) = (self.dep.session.clone(), key) else {
                    return Task::none();
                };
                // Lazy value-on-demand: one fetch per selection, nothing
                // ambient (issue #85). Symbolic skeleton paths have no
                // concrete value to fetch.
                if key.contains('{') {
                    return Task::none();
                }
                services::value::fetch(session, key)
            }
        }
    }

    /// The shell around the panes, and the replay mode.
    fn update_workspace(&mut self, msg: WorkspaceMsg) -> Task<Message> {
        match msg {
            WorkspaceMsg::PivotSelected(pivot) => {
                self.tree.pivot = pivot;
                self.tree.tree_scroll.0 = 0.0;
                self.tree.reflatten(&self.dep, &self.obs);
                Task::none()
            }
            WorkspaceMsg::TreeSearchChanged(q) => {
                self.tree.tree_search = q;
                self.tree.tree_scroll.0 = 0.0;
                self.tree.reflatten(&self.dep, &self.obs);
                Task::none()
            }
            WorkspaceMsg::TreeScrolled(y, h) => {
                // View-only state: the next frame renders the new window.
                self.tree.tree_scroll = (y, h.max(100.0));
                Task::none()
            }
            WorkspaceMsg::ToggleNode(path) => {
                // Collapsing takes the subtree with it (#179) — see
                // `expansion.rs` for why that trade is the fix rather than a
                // side effect of it.
                self.tree.expanded.toggle(&path);
                self.tree.reflatten(&self.dep, &self.obs);
                Task::none()
            }
            WorkspaceMsg::Replay(msg) => update::pane::replay::update(
                &mut self.dep,
                &mut self.obs,
                &mut self.sub,
                &mut self.tree,
                &mut self.work,
                msg,
            ),
            WorkspaceMsg::Reveal(path) => {
                let mut prefix = String::new();
                for chunk in path.split('/') {
                    if !prefix.is_empty() {
                        prefix.push('/');
                    }
                    prefix.push_str(chunk);
                    self.tree.expanded.open(prefix.clone());
                }
                self.tree.reflatten(&self.dep, &self.obs);
                Task::none()
            }
            WorkspaceMsg::PaneSelected(pane) => {
                self.work.right_pane = pane;
                Task::none()
            }
        }
    }

    /// The window, and what floats over it.
    fn update_chrome(&mut self, msg: ChromeMsg) -> Task<Message> {
        match msg {
            ChromeMsg::WindowResized(w, h) => {
                // Not on every pixel of a drag — the prefs file would be
                // rewritten hundreds of times per resize. Recorded here and
                // marked dirty; a settle timer writes it once the drag stops
                // (issue #189). "Written on the next real change" meant a
                // resize-then-quit lost the geometry entirely.
                self.chrome.prefs.window = Some((w, h));
                self.chrome.window_dirty = true;
                Task::none()
            }
            ChromeMsg::WindowSettled => {
                if self.chrome.window_dirty {
                    self.remember();
                }
                Task::none()
            }
            ChromeMsg::Key(key, modifiers) => self.update_key(&key, modifiers),
            ChromeMsg::Palette(msg) => self.update_palette(msg),
            ChromeMsg::Prefs(msg) => {
                use crate::message::PrefsMsg;
                match msg {
                    PrefsMsg::ThemeToggled => {
                        self.chrome.prefs.theme = self.chrome.prefs.theme.toggled()
                    }
                    PrefsMsg::ZoomIn => self.chrome.prefs.zoom_in(),
                    PrefsMsg::ZoomOut => self.chrome.prefs.zoom_out(),
                    PrefsMsg::ZoomReset => self.chrome.prefs.zoom_reset(),
                }
                // Saved on every change rather than at exit: a GUI is killed,
                // not quit, more often than anyone admits.
                self.remember();
                Task::none()
            }
        }
    }

    /// One of the eleven right-hand panes.
    fn update_pane(&mut self, msg: PaneMsg) -> Task<Message> {
        match msg {
            PaneMsg::Call(msg) => update::pane::call::update(
                &mut self.work.bench.call_form,
                msg,
                Ctx {
                    dep: &self.dep,
                    obs: &self.obs,
                    sub: &self.sub,
                },
            ),
            PaneMsg::Nodes(msg) => update::pane::nodes::update(
                &mut self.work.verdicts,
                msg,
                Ctx {
                    dep: &self.dep,
                    obs: &self.obs,
                    sub: &self.sub,
                },
            ),
            PaneMsg::Doctor(msg) => update::pane::doctor::update(
                &mut self.work.verdicts,
                &mut self.work.right_pane,
                msg,
                Ctx {
                    dep: &self.dep,
                    obs: &self.obs,
                    sub: &self.sub,
                },
            ),
            PaneMsg::Blob(msg) => update::pane::blob::update(
                &mut self.work.verdicts.blob,
                msg,
                Ctx {
                    dep: &self.dep,
                    obs: &self.obs,
                    sub: &self.sub,
                },
            ),
            PaneMsg::Media(msg) => update::pane::media::update(
                &mut self.work.bench.media,
                &self.work.verdicts.roster,
                msg,
                Ctx {
                    dep: &self.dep,
                    obs: &self.obs,
                    sub: &self.sub,
                },
            ),
            PaneMsg::Admin(msg) => update::pane::admin::update(
                &mut self.work.verdicts.admin,
                &mut self.work.right_pane,
                msg,
                Ctx {
                    dep: &self.dep,
                    obs: &self.obs,
                    sub: &self.sub,
                },
            ),
            PaneMsg::Detail(msg) => update::pane::detail::update(&mut self.sub, &self.dep, msg),
            PaneMsg::History(msg) => update::pane::detail::history(&mut self.sub, msg),
            PaneMsg::Publish(msg) => update::pane::publish::update(
                &mut self.work.bench,
                msg,
                Ctx {
                    dep: &self.dep,
                    obs: &self.obs,
                    sub: &self.sub,
                },
            ),
            PaneMsg::Echo(msg) => update::pane::echo::update(
                &mut self.work.echo,
                &mut self.work.right_pane,
                msg,
                Ctx {
                    dep: &self.dep,
                    obs: &self.obs,
                    sub: &self.sub,
                },
            ),
            PaneMsg::Context(msg) => {
                update::pane::context::update(&mut self.work.bench.context_form, msg)
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
    /// Point at a different fleet, and stop claiming anything about the old
    /// one.
    ///
    /// Four of the six sub-states, and that number is the finding: the issue
    /// assumed this could be one field replacement. Two of the four delegate,
    /// one is a struct replacement, and exactly one line reaches across a
    /// group boundary — `expanded`, because a stale path re-expands a
    /// coincidentally matching new subtree (#179).
    fn forget_deployment(&mut self) {
        self.dep.pointed_at();
        self.obs.forget_coverage();
        self.work.verdicts.forget();
        self.tree.expanded.clear();
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
            if self.chrome.palette.is_open() {
                self.chrome.palette.close();
            } else if self.sub.selected.is_some() {
                return Task::done(Message::Subject(SubjectMsg::SelectKey(None)));
            }
            return Task::none();
        }
        if self.chrome.palette.is_open() {
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
            Some(message) => Task::done(message),
            None => Task::none(),
        }
    }

    /// The command palette (#75).
    ///
    /// Every activation *returns* the action's own message as a `Task::done`,
    /// which is what keeps the palette from being a second implementation of
    /// anything: it is a faster way to send a message the UI already sends,
    /// and nothing more. It used to re-enter `update` directly; the message
    /// goes back out to iced now, which changes nothing about ordering — a
    /// `Task::done` resolves immediately — and everything about what a
    /// handler is allowed to reach.
    fn update_palette(&mut self, msg: view::palette::PaletteMsg) -> Task<Message> {
        use view::palette::PaletteMsg;
        match msg {
            PaletteMsg::Open(overlay) => {
                self.chrome.palette.open(overlay);
                Task::none()
            }
            PaletteMsg::Close => {
                self.chrome.palette.close();
                Task::none()
            }
            PaletteMsg::QueryChanged(q) => {
                self.chrome.palette.query = q;
                // A new query re-ranks the list, so the old cursor points at a
                // different row — start from the best match again.
                self.chrome.palette.cursor = 0;
                Task::none()
            }
            PaletteMsg::CursorUp => {
                self.chrome.palette.cursor = self.chrome.palette.cursor.saturating_sub(1);
                Task::none()
            }
            PaletteMsg::CursorDown => {
                self.chrome.palette.cursor = self
                    .chrome
                    .palette
                    .cursor
                    .saturating_add(1)
                    .min(self.palette_row_count().saturating_sub(1));
                Task::none()
            }
            PaletteMsg::Activate => self.run_palette_row(self.chrome.palette.cursor),
            PaletteMsg::Pick(i) => self.run_palette_row(i),
        }
    }

    /// How many rows the open overlay currently shows. Ranks over borrowed
    /// keys (fat pointers, no string bytes) — runs per keypress, not per
    /// frame (#110).
    fn palette_row_count(&self) -> usize {
        use view::palette::{Overlay, actions, rank};
        match self.chrome.palette.overlay {
            Overlay::Commands => {
                let items = actions(&self.work.bench.context_form.known);
                rank(&items, &self.chrome.palette.query, |a| a.label.as_str()).len()
            }
            Overlay::Keys => {
                // Observed keys only — never a guess (O4): the jump-to
                // overlay offers what is on the bus, not what a registry
                // says could be.
                let keys: Vec<&str> = self.dep.facts.keys().collect();
                rank(&keys, &self.chrome.palette.query, |k| *k).len()
            }
            _ => 0,
        }
    }

    /// The message behind row `index` — on the Keys overlay, the one place
    /// the palette ever clones a key `String` (#110): the activated row.
    fn palette_row(&self, index: usize) -> Option<Message> {
        use view::palette::{Overlay, actions, rank};
        match self.chrome.palette.overlay {
            Overlay::Commands => {
                let items = actions(&self.work.bench.context_form.known);
                let order = rank(&items, &self.chrome.palette.query, |a| a.label.as_str());
                order.get(index).map(|i| items[*i].message.clone())
            }
            Overlay::Keys => {
                let keys: Vec<&str> = self.dep.facts.keys().collect();
                let order = rank(&keys, &self.chrome.palette.query, |k| *k);
                order
                    .get(index)
                    .map(|i| Message::Subject(SubjectMsg::SelectKey(Some(keys[*i].to_string()))))
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
        if matches!(self.chrome.palette.overlay, view::palette::Overlay::Keys) {
            self.work.right_pane = RightPane::Detail;
        }
        self.chrome.palette.close();
        Task::done(message)
    }

    /// Layer a stored context over the live settings — the same precedence
    /// `Cli::settings_with` applies, minus the flags, because a context picked
    /// in-app *is* the explicit choice.
    fn apply_context(&mut self, stored: zenkey_fleet::StoredContext) {
        self.dep.settings.base = stored.base.unwrap_or_default();
        self.dep.settings.connect = stored.connect;
        self.dep.settings.listen = stored.listen;
        self.dep.settings.scouting = stored.scouting;
        self.dep.settings.zenoh_config = stored.zenoh_config;
        if !stored.registry.is_empty() {
            self.dep.settings.registry = stored.registry;
        }
        if let Some(t) = stored.timeout {
            self.dep.settings.timeout_secs = t;
        }
    }

    /// Tear the link down and build a new one on the current settings.
    ///
    /// The epoch bump the subscription machinery already does on
    /// `MonitorStarted` is what retires the old pump; nothing here has to
    /// coordinate with it.
    fn reopen_session(&mut self) -> Task<Message> {
        self.obs.link = LinkState::Connecting;
        self.obs.monitor = None;
        self.dep.session = None;
        services::link::reopen(
            self.dep.settings.zenoh_config.clone(),
            self.dep.settings.connect.clone(),
            self.dep.settings.listen.clone(),
            self.dep.settings.scouting,
        )
    }

    /// Reproject the registry's `[[blob]]` declarations after a slice load.
    ///
    /// Costs nothing on the bus: it reads slices already in hand and joins them
    /// against the roster already observed. The laziness rule (#84/#85) governs
    /// *fetching*, not rendering what has arrived — and the roster's
    /// `live_map()` returns `None` while unseeded, so an unasked join renders
    /// as "not asked" rather than as "nobody serves it" (O4).
    fn refresh_blob_list(&mut self) {
        let Some(slices) = self.dep.slices.as_deref() else {
            self.work.verdicts.blob.list = None;
            return;
        };
        let source = match self.dep.slice_source {
            view::status::SliceSource::Union { .. } => zenkey_fleet::report::BlobListSource::Union,
            view::status::SliceSource::Dirs { .. } => {
                zenkey_fleet::report::BlobListSource::RegistryDirs
            }
            _ => zenkey_fleet::report::BlobListSource::Bus,
        };
        self.work.verdicts.blob.list = Some(zenkey_fleet::blob_list(
            slices.slices(),
            self.work.verdicts.roster.live_map().as_ref(),
            source,
        ));
    }

    fn start_monitor(&mut self) -> Task<Message> {
        let Some(session) = self.dep.session.clone() else {
            return Task::none();
        };
        // Liveliness is always on — zero payload by construction (RFC 04 §5)
        // — and needs both the fleet sweep and @catalog by name (D4).
        let liveliness = if self.dep.base().is_empty() && self.dep.bases.is_empty() {
            scope::liveliness_any_base()
        } else {
            scope::liveliness_selectors(self.dep.base())
        };
        services::watch::start_monitor(session, liveliness, self.dep.settings.max_keys)
    }

    /// (Re)build the skeleton: slices are already loaded; roster + admin are
    /// gathered inside the task (both metadata-only).
    fn build_skeleton(&self) -> Task<Message> {
        let (Some(session), Some(slices)) = (self.dep.session.clone(), self.dep.slices.clone())
        else {
            return Task::none();
        };
        let base = self.dep.base().to_string();
        let timeout = self.dep.timeout();
        services::sweep::skeleton(session, base, slices, timeout)
    }

    fn toggle_watch(&mut self, path: String) -> Task<Message> {
        let Some(monitor) = self.obs.monitor.clone() else {
            return Task::none();
        };
        if let Some(id) = self.obs.my_watches.remove(&path) {
            self.obs.my_watch_paths.remove(&path);
            // A watch released mid-seed never gets its boundary (the engine
            // aborts the seed task) — forget it here too.
            self.obs.seeding.remove(&id);
            self.obs.seeding_paths.remove(&path);
            return services::watch::release(monitor, path, id);
        }
        // Watching seeds (issue #92): current state arrives before live
        // traffic, through the same merge discipline as everything else.
        let selector = scope::subtree_selector(&path);
        let policy = zenkey_fleet::SeedPolicy {
            timeout: self.dep.timeout(),
            ..Default::default()
        };
        services::watch::subtree(monitor, path, selector, policy)
    }

    fn watch_scope(&mut self) -> Task<Message> {
        let Some(monitor) = self.obs.monitor.clone() else {
            return Task::none();
        };
        let selectors = self
            .dep
            .settings
            .scope
            .selectors(self.dep.base(), &self.dep.settings.selectors);
        // Scope watches seed too (issue #92) — the eager preset is "observe
        // this scope", and current state is part of observing it.
        let policy = zenkey_fleet::SeedPolicy {
            timeout: self.dep.timeout(),
            ..Default::default()
        };
        services::watch::scope(monitor, selectors, policy)
    }

    fn unwatch_scope(&mut self) -> Task<Message> {
        let Some(monitor) = self.obs.monitor.clone() else {
            return Task::none();
        };
        let ids = std::mem::take(&mut self.obs.scope_watches);
        for id in &ids {
            self.obs.seeding.remove(id);
        }
        services::watch::release_scope(monitor, ids)
    }

    /// Persist what the window looks like now. Best-effort by construction
    /// (see `Prefs::save`) — a preference that cannot be written must not fail
    /// whatever the user was actually doing.
    fn remember(&mut self) {
        self.chrome.prefs.scope = self.dep.settings.scope;
        self.chrome.prefs.context = self.work.bench.context_form.active.clone().or(self
            .chrome
            .prefs
            .context
            .take());
        self.chrome.window_dirty = false;
        self.chrome.prefs.save();
    }

    fn reresolve_registrations(&mut self) {
        let Some(slices) = self.dep.slices.clone() else {
            return;
        };
        self.dep.facts.resolve_all(&slices);
    }

    fn load_slices(&self) -> Task<Message> {
        let dirs = self.dep.settings.registry.clone();
        let Some(session) = self.dep.session.clone() else {
            return Task::none();
        };
        let base = self.dep.base().to_string();
        let timeout = self.dep.timeout();
        if !dirs.is_empty() {
            // The §6.1 union (issue #43): served wins, dirs fill, and the
            // disagreement count reaches the status strip as data.
            return services::sweep::slices_union(session, base, dirs, timeout);
        }
        services::sweep::slices(session, base, timeout)
    }

    /// Feed one tick, for tests that assert what a tick does to the whole
    /// window. The five sub-states are `update::bus`'s parameter list; this
    /// exists so a test does not have to spell them.
    #[cfg(test)]
    fn tick_for_test(&mut self, tick: &crate::message::BusTick) {
        update::bus::apply_tick(
            &mut self.dep,
            &mut self.obs,
            &mut self.sub,
            &mut self.tree,
            &mut self.work,
            tick,
        );
    }

    pub fn subscription(&self) -> Subscription<Message> {
        let mut subs = Vec::new();
        // Replay mode replaces the link (#74): while a `.zrec` feeds the
        // panes, the live pump is not built at all — the mode cannot leak.
        if self.work.replay.replay.is_none()
            && let Some(monitor) = self.obs.monitor.clone()
        {
            subs.push(link::subscribe(LinkKey {
                monitor,
                epoch: self.obs.epoch,
            }));
        }
        // The play clock, only while actually playing (a paused replay
        // costs nothing) — the same cadence as the live stats tick, so the
        // panes tick at the rate they were built for.
        if self.work.replay.replay.as_ref().is_some_and(|r| r.playing) {
            subs.push(
                iced::time::every(std::time::Duration::from_millis(250)).map(|_| {
                    Message::Workspace(WorkspaceMsg::Replay(view::replay::ReplayMsg::Advance))
                }),
            );
        }
        // The repeat clock for a sustained publish (#60). It exists only while
        // a publication is armed, so an idle pane costs nothing.
        if self.work.bench.publish_form.armed {
            let period =
                std::time::Duration::from_secs_f64(self.work.bench.publish_form.interval_secs());
            subs.push(
                iced::time::every(period)
                    .map(|_| Message::Pane(PaneMsg::Publish(view::publish::PublishMsg::Tick))),
            );
        }
        // Window geometry, for the next launch (issue #73).
        subs.push(
            iced::window::resize_events().map(|(_, size)| {
                Message::Chrome(ChromeMsg::WindowResized(size.width, size.height))
            }),
        );
        // …and the settle timer that actually writes it, which exists only
        // while a resize is outstanding (issue #189). One file write per drag
        // rather than per pixel, and none at all while the window is still.
        if self.chrome.window_dirty {
            subs.push(
                iced::time::every(std::time::Duration::from_millis(700))
                    .map(|_| Message::Chrome(ChromeMsg::WindowSettled)),
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
                Some(Message::Chrome(ChromeMsg::Key(key, modifiers)))
            }
            _ => None,
        }));
        Subscription::batch(subs)
    }

    pub fn view(&self) -> Element<'_, Message> {
        let panes = row![
            iced::widget::container(view::tree::pane(view::tree::TreeData {
                flat: &self.tree.flat,
                pivot: self.tree.pivot,
                search: &self.tree.tree_search,
                scroll_y: self.tree.tree_scroll.0,
                viewport_h: self.tree.tree_scroll.1,
                facts: &self.dep.facts,
                watches: view::tree::Watches {
                    mine: &self.obs.my_watch_paths,
                    seeding: &self.obs.seeding_paths,
                },
                selected: self.sub.selected.as_deref(),
            }))
            .width(Length::FillPortion(self.chrome.prefs.split_portions().0))
            .height(Length::Fill),
            iced::widget::container(match self.work.right_pane {
                RightPane::Echo => view::echo::pane(
                    &self.work.echo.echo,
                    &self.work.echo.echo_view,
                    self.sub.selected.as_deref(),
                    self.work.echo.echo.next_seq(),
                ),
                RightPane::Call => view::call::pane(
                    &self.work.bench.call_form,
                    self.dep.slices.as_deref(),
                    &self.work.verdicts.roster,
                ),
                RightPane::Publish =>
                    view::publish::pane(&self.work.bench.publish_form, self.dep.slices.is_some()),
                RightPane::Detail => view::detail::pane(view::detail::DetailData {
                    key: self.sub.selected.as_deref().unwrap_or("(nothing selected)"),
                    facts: self
                        .sub
                        .selected
                        .as_deref()
                        .and_then(|k| self.dep.facts.get(k)),
                    fetched: self.sub.fetched.as_ref().and_then(|(k, o)| {
                        (Some(k.as_str()) == self.sub.selected.as_deref()).then_some(o)
                    }),
                    decoded: self.sub.decoded.as_ref(),
                    series: self.sub.series.as_ref(),
                    history_entries: self.sub.history.as_ref().map(|r| r.ring.len()),
                    observed: self.sub.history.as_ref().and_then(|r| r.ring.newest()),
                    latency: self.sub.selected_latency.clone(),
                }),
                RightPane::Nodes => view::nodes::pane(view::nodes::NodesData {
                    roster: &self.work.verdicts.roster,
                    selected: self.work.verdicts.node_selected.as_deref(),
                    detail: &self.work.verdicts.node_detail,
                }),
                RightPane::Doctor =>
                    view::doctor::pane(&self.work.verdicts.doctor, self.dep.base()),
                RightPane::Blob =>
                    view::blob::pane(&self.work.verdicts.blob, self.dep.slices.is_some()),
                RightPane::Media =>
                    view::media::pane(&self.work.bench.media, self.dep.slices.as_deref()),
                RightPane::Admin => view::admin::pane(&self.work.verdicts.admin),
                RightPane::History => view::history::pane(view::history::HistoryData {
                    key: self.sub.selected.as_deref(),
                    recorder: self.sub.history.as_ref(),
                    watched: self
                        .sub
                        .selected
                        .as_deref()
                        .is_some_and(|k| key_is_watched(&self.obs.watched, k)),
                }),
                RightPane::Connect => view::contexts::pane(
                    &self.work.bench.context_form,
                    self.dep.settings.is_unreachable(),
                ),
            })
            .width(Length::FillPortion(self.chrome.prefs.split_portions().1))
            .height(Length::Fill),
        ]
        .spacing(space::MD);

        let mut layout = column![self.toolbar()]
            .spacing(space::MD)
            .padding(space::MD);
        // Replay-mode surfaces (#74), between the toolbar and the panes so
        // the mode is unmistakable: the open row, the REPLAY banner with
        // the scrubber, and the capture status line.
        if let Some(path) = &self.work.replay.replay_open {
            layout = layout.push(view::replay::open_row(path));
            if let Some(note) = &self.work.replay.replay_note {
                layout = layout.push(view::kit::muted(format!("could not open: {note}")));
            }
        }
        if let Some(replay) = &self.work.replay.replay {
            layout = layout.push(view::replay::banner(replay));
        }
        if let Some(rec) = &self.work.replay.recording {
            layout = layout.push(view::kit::muted(format!(
                "● recording current watches to {} — toolbar 'stop recording' finishes the file",
                rec.path
            )));
        }
        if let Some(done) = &self.work.replay.recorded {
            layout = layout.push(view::kit::muted(match done {
                Ok((samples, dropped, path)) => format!(
                    "recorded {samples} sample(s) to {path} ({dropped} dropped — in-file ledger)"
                ),
                Err(e) => format!("recording failed: {e}"),
            }));
        }
        let layout = layout.push(panes).push(view::status::strip(Status {
            link: &self.obs.link,
            base_label: self.dep.base_label(),
            scope_label: self.dep.settings.scope.short(),
            keys: self.obs.keys,
            keys_evicted: self.obs.keys_evicted,
            keys_unwatched: self.obs.keys_unwatched,
            facts_cached: self.dep.facts.len(),
            facts_evicted: self.dep.facts.evicted(),
            watched: &self.obs.watched,
            skeleton: self.dep.skeleton.as_deref().map(|s| s.coverage),
            fetched: self.sub.fetched.as_ref(),
            totals: self.obs.totals,
            slices: &self.dep.slice_source,
            seeding: self.obs.seeding.len(),
            seeded_watches: self.obs.seeded_watches,
            seed_totals: self.obs.seed_totals,
            unreachable: self.dep.settings.is_unreachable(),
            prefs_note: self.chrome.prefs_note.as_deref(),
            replaying: self.work.replay.replay.is_some(),
        }));

        // The overlay floats above everything (#75). `stack` rather than a
        // modal widget because the layering rule is ours — palette above
        // panes, Esc peeling one layer at a time — and a widget with its own
        // dismissal policy would fight it.
        // A lazy iterator: a closed overlay never touches the cache, and
        // the open one clones only what it draws (#110).
        match view::palette::overlay(
            &self.chrome.palette,
            &self.work.bench.context_form.known,
            self.dep.facts.keys(),
        ) {
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

    /// The base picker's options, after a discovery sweep landed.
    fn rebuild_base_options(&mut self) {
        self.dep.base_options = vec![String::new()];
        self.dep
            .base_options
            .extend(self.dep.bases.iter().map(|b| b.base.clone()));
        self.dep.base_options.dedup();
    }

    fn toolbar(&self) -> Element<'_, Message> {
        // Both pickers borrow (#178). `pick_list` takes `L: Borrow<[T]>`, so a
        // slice is as good as a `Vec` — and the toolbar redraws at frame rate
        // while its options change on a discovery sweep, which is minutes
        // apart.
        let base_picker = pick_list(
            &self.dep.base_options[..],
            Some(self.dep.settings.base.clone()),
            |r| Message::Deployment(DeploymentMsg::BaseSelected(r)),
        )
        .placeholder("base")
        .text_size(crate::view::tokens::font::CAPTION);

        /// The closed scope vocabulary, in menu order.
        const SCOPES: [ScopePreset; 5] = [
            ScopePreset::Everything,
            ScopePreset::Deployment,
            ScopePreset::Telemetry,
            ScopePreset::State,
            ScopePreset::Events,
        ];
        let scope_picker = pick_list(&SCOPES[..], Some(self.dep.settings.scope), |r| {
            Message::Deployment(DeploymentMsg::ScopeSelected(r))
        })
        .text_size(crate::view::tokens::font::CAPTION);

        // Observation is opt-in and labelled by its cost (issue #85).
        let observing = !self.obs.scope_watches.is_empty();
        let observe = iced::widget::button(
            text(if observing {
                "stop observing scope"
            } else {
                "observe scope"
            })
            .size(crate::view::tokens::font::CAPTION),
        )
        .on_press(Message::Deployment(DeploymentMsg::ScopeWatchToggled))
        .padding(4);

        row![
            text("base").size(crate::view::tokens::font::CAPTION),
            base_picker,
            text("scope").size(crate::view::tokens::font::CAPTION),
            scope_picker,
            observe,
            iced::widget::Row::from_iter(RightPane::ALL.into_iter().map(|p| {
                crate::view::kit::tab(
                    p.label(),
                    self.work.right_pane == p,
                    Message::Workspace(WorkspaceMsg::PaneSelected(p)),
                )
            }))
            .spacing(space::XS),
            crate::view::kit::muted(self.dep.settings.scope.label()),
            // Capture and replay (#74): record writes the current watches
            // to a .zrec; replay feeds the panes from one.
            iced::widget::button(
                text(if self.work.replay.recording.is_some() {
                    "stop recording"
                } else {
                    "record"
                })
                .size(crate::view::tokens::font::CAPTION)
            )
            .on_press(Message::Workspace(WorkspaceMsg::Replay(
                view::replay::ReplayMsg::RecordToggled
            )))
            .padding(4),
            iced::widget::button(text("replay…").size(crate::view::tokens::font::CAPTION))
                .on_press(Message::Workspace(WorkspaceMsg::Replay(
                    view::replay::ReplayMsg::OpenToggled
                )))
                .padding(4),
            iced::widget::space::horizontal(),
            // Window preferences (issue #73): the theme name is the button,
            // so the label says what you get rather than what you have.
            iced::widget::button(
                text(format!("theme: {}", self.chrome.prefs.theme.label()))
                    .size(crate::view::tokens::font::CAPTION)
            )
            .on_press(Message::Chrome(ChromeMsg::Prefs(
                crate::message::PrefsMsg::ThemeToggled
            )))
            .padding(4),
            iced::widget::button(text("-").size(crate::view::tokens::font::CAPTION))
                .on_press(Message::Chrome(ChromeMsg::Prefs(
                    crate::message::PrefsMsg::ZoomOut
                )))
                .padding(4),
            iced::widget::button(
                text(format!(
                    "{}%",
                    (self.chrome.prefs.zoom * 100.0).round() as i32
                ))
                .size(crate::view::tokens::font::CAPTION)
            )
            .on_press(Message::Chrome(ChromeMsg::Prefs(
                crate::message::PrefsMsg::ZoomReset
            )))
            .padding(4),
            iced::widget::button(text("+").size(crate::view::tokens::font::CAPTION))
                .on_press(Message::Chrome(ChromeMsg::Prefs(
                    crate::message::PrefsMsg::ZoomIn
                )))
                .padding(4),
            iced::widget::button(text("reconnect").size(crate::view::tokens::font::CAPTION))
                .on_press(Message::Deployment(DeploymentMsg::Reconnect))
                .padding(4),
        ]
        .spacing(space::SM)
        .align_y(iced::Alignment::Center)
        .into()
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
    use crate::message::BusTick;
    use crate::state::tree::shape_held;

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

    fn app() -> Zengui {
        Zengui::with_prefs(
            Settings {
                base: String::new(),
                connect: vec![],
                listen: vec![],
                scouting: None,
                zenoh_config: None,
                registry: vec![],
                timeout_secs: 5,
                scope: crate::scope::ScopePreset::Everything,
                selectors: vec![],
                eager: false,
                echo_lines: 100,
                history_entries: 10,
                max_keys: 1000,
            },
            crate::prefs::Prefs::default(),
            None,
        )
        .0
    }

    /// #179's first acceptance, and the defect exactly: `forget_deployment`
    /// cleared ten collections and not this one, so every node the user had
    /// ever opened outlived the deployment it belonged to — keyed to paths
    /// that no longer exist, where a stale one re-expands a coincidentally
    /// matching new subtree.
    #[test]
    fn switching_base_forgets_every_node_that_was_open() {
        let mut app = app();
        for i in 0..10_000 {
            app.tree.expanded.open(format!("v1/h-{i:04}/state/sysinfo"));
        }
        assert_eq!(app.tree.expanded.len(), 10_000);
        app.forget_deployment();
        assert!(app.tree.expanded.is_empty());
        assert!(
            app.work.verdicts.admin.expanded_raw.is_empty(),
            "`AdminState::clear` is `*self = default()`, so this was already \
             true — asserted here so it stays true if that changes"
        );
    }

    /// The other half of the old checklist, as behaviour rather than a list.
    ///
    /// `KEPT` named 43 fields and asserted only that `forget_deployment` did
    /// not mention them. That is weaker than it reads: a field can be listed
    /// as kept and still be clobbered by the same message through some other
    /// path. This asks the question the user asks — "is my half-written
    /// publish body still there?" — of the three groups that hold typing.
    #[test]
    fn switching_base_keeps_what_the_user_typed() {
        let mut app = app();
        app.work.bench.publish_form.body = "{\"celsius\": 21.5}".into();
        app.work.bench.call_form.params = "origin=h-3fa9c2d41b7e".into();
        app.work.bench.context_form.connect = "tcp/10.0.0.1:7447".into();
        app.tree.tree_search = "sysinfo".into();
        app.sub.selected = Some("v1/h-3fa9c2d41b7e/state/sysinfo/health".into());
        app.chrome.prefs.zoom = 1.25;

        app.forget_deployment();

        assert_eq!(app.work.bench.publish_form.body, "{\"celsius\": 21.5}");
        assert_eq!(app.work.bench.call_form.params, "origin=h-3fa9c2d41b7e");
        assert_eq!(app.work.bench.context_form.connect, "tcp/10.0.0.1:7447");
        assert_eq!(app.tree.tree_search, "sysinfo");
        assert_eq!(
            app.sub.selected.as_deref(),
            Some("v1/h-3fa9c2d41b7e/state/sysinfo/health"),
            "a selection follows the user, not the fleet — the panes then say \
             honestly that they have not asked about it yet"
        );
        assert_eq!(app.chrome.prefs.zoom, 1.25);
    }

    /// The proof for a deleted line.
    ///
    /// `forget_deployment` used to reset `merged_cache`, and that line was
    /// dead: `merged`'s validity check compares the skeleton `Option`s, and
    /// after a forget the cached `Some` cannot match the new `None`. If both
    /// were already `None`, all three merge inputs are identical and reuse is
    /// *correct*. Deleting a line needs the same evidence as adding one.
    #[test]
    fn a_forgotten_deployment_needs_no_merge_cache_reset() {
        let mut app = app();
        app.dep.skeleton = Some(Arc::new(zenkey_fleet::Skeleton::build(
            "",
            &zenkey_fleet::SliceSet::default(),
            &std::collections::BTreeMap::new(),
            None,
        )));
        let with_skeleton = app.tree.merged(&app.dep, &app.obs);

        app.forget_deployment();
        assert!(
            app.tree.merged_cache.is_some(),
            "the cache is deliberately not cleared — this test exists to \
             notice if someone reinstates the reset"
        );
        let after = app.tree.merged(&app.dep, &app.obs);
        assert!(
            !Arc::ptr_eq(&with_skeleton, &after),
            "a forgotten skeleton must not be served from cache"
        );
    }

    /// A tick that changes nothing about the key set or the watch set.
    fn tick(keys: usize, evicted: u64, unwatched: u64, watched: &Arc<[String]>) -> BusTick {
        let stats = zenkey_fleet::stats::StatsTable::new();
        BusTick {
            tree: Arc::new(zenkey_fleet::KeyTreeSnapshot::build(&stats)),
            samples: Vec::new(),
            lagged: 0,
            coalesced: 0,
            nodes: Vec::new(),
            keys,
            keys_evicted: evicted,
            keys_unwatched: unwatched,
            watched: Arc::clone(watched),
            seeded: Vec::new(),
            totals: (0, 0, 0.0),
        }
    }

    /// #177's whole claim, and the thing criterion cannot say because it
    /// cannot count allocations: at steady state the tree is walked once.
    #[test]
    fn steady_state_ticks_reuse_the_tree_shape() {
        let mut app = app();
        let watched: Arc<[String]> = Arc::from(["v1/**".to_string()]);
        for _ in 0..100 {
            app.tick_for_test(&tick(7, 0, 0, &watched));
        }
        assert_eq!(
            (app.tree.shape_rebuilt, app.tree.shape_reused),
            (1, 99),
            "one rebuild to establish the shape, then ninety-nine retargets"
        );
    }

    /// Each of the four things that can move the shape, alone.
    ///
    /// The `keys` rung needs the other two beside it: `keys` is a *current*
    /// count, so a key added and evicted within one 250 ms window cancels out
    /// — which is exactly why the trigger is a triple and not a number.
    #[test]
    fn a_new_key_an_eviction_an_unwatch_or_a_new_watch_set_all_rebuild() {
        let watched: Arc<[String]> = Arc::from(["v1/**".to_string()]);
        for (label, next) in [
            ("a new key", (8usize, 0u64, 0u64)),
            ("an eviction", (7, 1, 0)),
            ("an unwatch", (7, 0, 1)),
        ] {
            let mut app = app();
            app.tick_for_test(&tick(7, 0, 0, &watched));
            let before = app.tree.shape_rebuilt;
            app.tick_for_test(&tick(next.0, next.1, next.2, &watched));
            assert_eq!(
                app.tree.shape_rebuilt,
                before + 1,
                "{label} changes the tree and must rebuild it"
            );
        }

        // A different watch set with identical counters: `is_covered` decides
        // `NodeStatus`, so the badge would freeze without this rung.
        let mut app = app();
        app.tick_for_test(&tick(7, 0, 0, &watched));
        let before = app.tree.shape_rebuilt;
        let other: Arc<[String]> = Arc::from(["v1/**".to_string()]);
        app.tick_for_test(&tick(7, 0, 0, &other));
        assert_eq!(
            app.tree.shape_rebuilt,
            before + 1,
            "a new watch-set Arc rebuilds even when every counter matches"
        );
    }

    /// The hazard the watch rung covers, named so it cannot be optimised away.
    ///
    /// `Message::Subject(SubjectMsg::WatchReleased)` does **not** reflatten — it returns
    /// `Task::none()` and has always relied on the unconditional tick rebuild
    /// to repaint node status. Under a conditional trigger that would be a live
    /// bug, except that releasing a watch fires `WatchChanged`, which is the
    /// only thing that reassigns `watched` in `link.rs`, which swaps the `Arc`.
    #[test]
    fn a_released_watch_still_repaints_the_tree() {
        let mut app = app();
        let two: Arc<[String]> = Arc::from(["v1/a/**".to_string(), "v1/b/**".to_string()]);
        app.tick_for_test(&tick(7, 0, 0, &two));
        let before = app.tree.shape_rebuilt;
        // The release: same keys, same counters, one selector fewer.
        let one: Arc<[String]> = Arc::from(["v1/a/**".to_string()]);
        app.tick_for_test(&tick(7, 0, 0, &one));
        assert_eq!(app.tree.shape_rebuilt, before + 1);
    }

    /// Expanding, collapsing, typing in the find box and switching pivot all
    /// change only *how* the tree is presented — so the merge behind it is
    /// repetition, and at 50,000 keys it is the larger half of `reflatten`
    /// (24.97 ms against 22.08 ms).
    #[test]
    fn a_presentation_change_does_not_re_merge_the_tree() {
        let mut app = app();
        app.tree.reflatten(&app.dep, &app.obs);
        let first = Arc::clone(&app.tree.merged_cache.as_ref().expect("cached").merged);

        app.tree.expanded.open("v1");
        app.tree.reflatten(&app.dep, &app.obs);
        assert!(
            Arc::ptr_eq(
                &first,
                &app.tree.merged_cache.as_ref().expect("cached").merged
            ),
            "an expand changes no input to the merge"
        );

        // A new observed snapshot is a different tree, and must not be served
        // from the cache.
        let stats = zenkey_fleet::stats::StatsTable::new();
        app.obs.observed = Arc::new(zenkey_fleet::KeyTreeSnapshot::build(&stats));
        app.tree.reflatten(&app.dep, &app.obs);
        assert!(
            !Arc::ptr_eq(
                &first,
                &app.tree.merged_cache.as_ref().expect("cached").merged
            ),
            "a new snapshot is new evidence, whatever it contains"
        );
    }

    /// The predicate on its own, since it is the whole trigger.
    #[test]
    fn the_shape_trigger_reads_every_rung() {
        let a: Arc<[String]> = Arc::from(["x".to_string()]);
        let b: Arc<[String]> = Arc::from(["x".to_string()]);
        assert!(shape_held((1, 2, 3), (1, 2, 3), &a, &a));
        assert!(!shape_held((1, 2, 3), (2, 2, 3), &a, &a));
        assert!(!shape_held((1, 2, 3), (1, 3, 3), &a, &a));
        assert!(!shape_held((1, 2, 3), (1, 2, 4), &a, &a));
        assert!(
            !shape_held((1, 2, 3), (1, 2, 3), &a, &b),
            "equal contents, different Arc: the watch set was rebuilt, and \
             only a rebuild reassigns it"
        );
        // A replay seeking backwards moves the counters *down*, which is why
        // the comparison is equality and not a `>`.
        assert!(!shape_held((9, 9, 9), (1, 2, 3), &a, &a));
    }
}

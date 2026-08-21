//! The Elm loop: state, `update`, `view`, `subscription`.
//!
//! **Lazy by default** (issue #85): connecting builds the *skeleton* — the
//! declared keyspace from registry + liveliness + admin metadata — and starts
//! a monitor with **zero data-plane watches**. Observation is opt-in per
//! subtree (the tree's watch toggles) or per scope (the toolbar's "observe
//! scope" toggle — the old eager mode made explicit); a selection fetches one
//! value on demand. `--eager` restores the bootstrap behavior from the
//! command line, labelled by its cost.

use iced::widget::{column, pick_list, row, text};
use iced::{Element, Length, Subscription, Task};

use crate::config::Settings;
use crate::link::{self, LinkKey};
use crate::message::Message;
use crate::scope::ScopePreset;
use crate::services;
use crate::state::{Chrome, Deployment, Observation, Subject, TreeState, Workspace};
use crate::update::{self};
use crate::view;
use crate::view::status::Status;
use crate::view::tokens::space;

use crate::message::{ChromeMsg, DeploymentMsg, PaneMsg, RightPane, WorkspaceMsg};

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

    /// The one function that still takes `&mut Zengui`, and the only one
    /// that ever will.
    ///
    /// It destructures immediately, so what each group can move is its
    /// parameter list rather than a promise. The honest count: `deployment`
    /// names all six, and `bus`, `subject` and `workspace` name five —
    /// `subject` through the causal chain from `SelectKey`, `workspace`
    /// through the one arm that hands to replay. The two that stay narrow are
    /// `chrome`, which cannot move a row or a watch, and `pane`, which hands
    /// each pane only its own state.
    pub fn update(&mut self, message: Message) -> Task<Message> {
        let Zengui {
            chrome,
            dep,
            obs,
            sub,
            tree,
            work,
        } = self;
        match message {
            Message::Bus(m) => update::bus::update(dep, obs, sub, tree, work, m),
            Message::Subject(m) => update::subject::update(dep, obs, sub, tree, work, m),
            Message::Deployment(m) => {
                update::deployment::update(chrome, dep, obs, sub, tree, work, m)
            }
            Message::Workspace(m) => update::workspace::update(dep, obs, sub, tree, work, m),
            Message::Pane(m) => update::pane::update(dep, obs, sub, work, m),
            Message::Chrome(m) => update::chrome::update(chrome, dep, sub, work, m),
        }
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
    use std::sync::Arc;

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
        update::deployment::forget(&mut app.dep, &mut app.obs, &mut app.tree, &mut app.work);
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

        update::deployment::forget(&mut app.dep, &mut app.obs, &mut app.tree, &mut app.work);

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

        update::deployment::forget(&mut app.dep, &mut app.obs, &mut app.tree, &mut app.work);
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

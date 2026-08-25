//! The Elm loop: state, `update`, `view`, `subscription`.
//!
//! **Lazy by default** (issue #85): connecting builds the *skeleton* — the
//! declared keyspace from registry + liveliness + admin metadata — and starts
//! a monitor with **zero data-plane watches**. Observation is opt-in per
//! subtree (the tree's watch toggles) or per scope (the location bar's
//! "observe scope" toggle — the old eager mode made explicit); a selection fetches one
//! value on demand. `--eager` restores the bootstrap behavior from the
//! command line, labelled by its cost.
//!
//! **This file is the shell and nothing else** (#175). The 64 fields live in
//! `state/` as six sub-states named for what invalidates them; the handlers
//! live in `update/`, each naming the exhaustive set of sub-states it can
//! move; the bus calls live in [`crate::services`]. `Zengui`'s six
//! fields are private and destructured in exactly two places —
//! [`Zengui::update`] and [`Zengui::view`], the two functions iced calls.

use iced::widget::column;
use iced::{Element, Length, Subscription, Task};

use crate::config::Settings;
use crate::link::{self, LinkKey};
use crate::message::{ChromeMsg, Message, PaneMsg, WorkspaceMsg};
use crate::services;
use crate::state::{Chrome, Deployment, Observation, SubjectState, TreeState, Workspace};
use crate::update;
use crate::view;
use crate::view::status::Status;
use crate::view::tokens::space;

pub struct Zengui {
    chrome: Chrome,
    dep: Deployment,
    obs: Observation,
    sub: SubjectState,
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
        // The persisted layout is what the grid is rebuilt from (#180); read
        // before `prefs` moves into the chrome.
        let work = Workspace::new(echo_lines, &prefs.layout.root);
        let app = Zengui {
            chrome: Chrome::new(prefs, prefs_note),
            dep: Deployment::new(settings),
            obs: Observation::default(),
            sub: SubjectState::default(),
            tree: TreeState::default(),
            work,
        };
        (
            app,
            services::link::open(zenoh_config, connect, listen, scouting),
        )
    }

    /// Open the windows the preferences describe (#186): the main workspace,
    /// and one window per dock the named layout keeps torn off. Called by
    /// [`Zengui::boot`] — never by [`Zengui::with_prefs`], so a headless test
    /// constructs an app with no windows at all.
    pub(crate) fn open_windows(&mut self) -> Task<Message> {
        use crate::state::workspace::{TornWindow, torn_settings};
        let mut tasks = Vec::new();
        let size = self
            .chrome
            .prefs
            .window
            .map(|(w, h)| iced::Size::new(w, h))
            .unwrap_or(iced::Size::new(1280.0, 800.0));
        let (main, open) = iced::window::open(iced::window::Settings {
            size,
            ..iced::window::Settings::default()
        });
        self.work.windows.main = Some(main);
        tasks.push(open.discard());
        for t in self.chrome.prefs.layout.torn.clone() {
            let (id, open) = iced::window::open(torn_settings(t.role, t.size, t.position));
            self.work.windows.torn.push(TornWindow {
                id,
                role: t.role,
                // Follow-bound (#257): only follow windows persist — a pin's
                // evidence is session-lived, and restoring the identity
                // without it would be the freeze the issue rejects.
                slot: crate::message::SlotId::FOLLOW,
                size: t.size,
                position: t.position,
            });
            tasks.push(open.discard());
        }
        Task::batch(tasks)
    }

    /// What the daemon boots (#186): the state, its windows, and the link.
    /// `iced::daemon` opens no window of its own, so the main window — and
    /// every torn-off dock the named layout remembers — is opened here.
    pub fn boot(
        settings: Settings,
        prefs: crate::prefs::Prefs,
        prefs_note: Option<String>,
    ) -> (Zengui, Task<Message>) {
        let (mut app, link) = Zengui::with_prefs(settings, prefs, prefs_note);
        let windows = app.open_windows();
        (app, Task::batch([windows, link]))
    }

    pub fn title(&self, window: iced::window::Id) -> String {
        match self.work.windows.torn.iter().find(|t| t.id == window) {
            // A torn-off dock's window says which region it is — its own
            // chrome is the only title bar it has (#186). A pinned window
            // says so in the title too (#257); the banner inside states the
            // subject, since a key rarely fits a title bar.
            Some(t) if t.slot.is_pin() => {
                format!(
                    "zengui — {} (pinned) — {}",
                    t.role.label(),
                    self.dep.base_label()
                )
            }
            Some(t) => format!("zengui — {} — {}", t.role.label(), self.dep.base_label()),
            None => format!("zengui — {}", self.dep.base_label()),
        }
    }

    pub fn theme(&self, _window: iced::window::Id) -> iced::Theme {
        self.chrome.prefs.theme.theme()
    }

    /// The UI scale factor iced applies to every window (issue #73).
    pub fn scale_factor(&self, _window: iced::window::Id) -> f32 {
        self.chrome.prefs.zoom
    }

    /// The one function that still takes `&mut Zengui`, and the only one
    /// that ever will.
    ///
    /// It destructures immediately, so what each group can move is its
    /// parameter list rather than a promise. The honest count: `deployment`
    /// and `workspace` name all six — `workspace` gained `chrome` with #180,
    /// because the dock grid's persisted form is a preference — and `bus` and
    /// `subject` name five, `subject` through the causal chain from `Select`.
    /// The two that stay narrow are `chrome`, which cannot move a row or a
    /// watch, and `pane`, which hands each pane only its own state.
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
            Message::Workspace(m) => {
                update::workspace::update(chrome, dep, obs, sub, tree, work, m)
            }
            Message::Pane(m) => update::pane::update(dep, obs, sub, work, m),
            Message::Chrome(m) => update::chrome::update(chrome, dep, sub, work, m),
        }
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
        if self.work.bench.send_form.armed {
            let period =
                std::time::Duration::from_secs_f64(self.work.bench.send_form.interval_secs());
            subs.push(
                iced::time::every(period)
                    .map(|_| Message::Pane(PaneMsg::Send(view::send::SendMsg::Tick))),
            );
        }
        // Window geometry (issue #73; per window since #186): the main
        // window's size lands in the prefs, a torn-off dock's in the named
        // layout — `update::chrome` tells them apart by id.
        subs.push(iced::window::resize_events().map(|(id, size)| {
            Message::Chrome(ChromeMsg::WindowResized(id, size.width, size.height))
        }));
        // A torn-off window's position, where the platform reports one
        // (#186): the second monitor is the point of the feature.
        subs.push(
            iced::window::events().filter_map(|(id, event)| match event {
                iced::window::Event::Moved(p) => {
                    Some(Message::Chrome(ChromeMsg::WindowMoved(id, p.x, p.y)))
                }
                _ => None,
            }),
        );
        // Window closes (#186): a torn-off dock's window restores its dock
        // to the grid; the main window's close is the application's — a
        // daemon would otherwise keep running with nothing to show, which
        // is the zombie the issue names.
        subs.push(
            iced::window::close_events()
                .map(|id| Message::Workspace(WorkspaceMsg::WindowClosed(id))),
        );
        // …and the settle timer that actually writes it, which exists only
        // while a resize — of the window (#189) or of a dock splitter
        // (#180) — is outstanding. One file write per drag rather than per
        // pixel, and none at all while the window is still.
        if self.chrome.prefs_dirty {
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

    /// One `view`, dispatched by window id (#186): a torn-off dock's window
    /// renders that dock alone — through the same free pane functions the
    /// grid composes — and every other id renders the main workspace. The
    /// main window is the fallback rather than a named case, so a headless
    /// test that never opened a window still renders the workspace with any
    /// id it likes.
    pub fn view(&self, window: iced::window::Id) -> Element<'_, Message> {
        if let Some(torn) = self.work.windows.torn.iter().find(|t| t.id == window) {
            // The replay banner renders in *every* window (#74, #186): a
            // torn-off Echo fed from a file with no banner over it would be
            // a live-looking stream that is not live.
            let mut layout = iced::widget::column![]
                .spacing(space::MD)
                .padding(space::MD);
            for surface in view::replay::surfaces(&self.work.replay) {
                layout = layout.push(surface);
            }
            return layout
                .push(view::panes::solo(
                    &self.dep,
                    &self.obs,
                    &self.sub,
                    &self.tree,
                    &self.work,
                    torn.role,
                    // The window's slot binding (#257): a pinned Inspector
                    // renders its own subject; everything else follows.
                    torn.slot,
                    self.chrome.prefs.density,
                ))
                .into();
        }
        // The workspace is the dock grid (#180): every region — the locator,
        // the Inspector, the Activity dock (#183), the workbench — is a pane
        // of it, resizable and rearrangeable, and a closed dock gives its
        // space back structurally.
        let workspace = view::panes::grid(
            &self.dep,
            &self.obs,
            &self.sub,
            &self.tree,
            &self.work,
            self.chrome.prefs.density,
        );

        let mut layout = column![view::location::bar(
            &self.chrome,
            &self.dep,
            &self.obs,
            &self.sub,
            &self.work
        )]
        .spacing(space::MD)
        .padding(space::MD);
        // Replay-mode surfaces (#74) sit between the location bar and the
        // panes, so the mode is unmistakable.
        for surface in view::replay::surfaces(&self.work.replay) {
            layout = layout.push(surface);
        }
        let layout = layout.push(workspace).push(view::status::strip(Status::of(
            &self.chrome,
            &self.dep,
            &self.obs,
            &self.sub,
            &self.work,
        )));

        // The overlay floats above everything (#75). `stack` rather than a
        // modal widget because the layering rule is ours — palette above
        // panes, Esc peeling one layer at a time — and a widget with its own
        // dismissal policy would fight it.
        // A lazy iterator: a closed overlay never touches the cache, and
        // the open one clones only what it draws (#110).
        match view::palette::overlay(
            &self.chrome.palette,
            &self.work.bench.context_form,
            self.dep.settings.is_unreachable(),
            view::scope_editor::ScopeEditorData {
                scope: self.dep.settings.scope,
                base: self.dep.base(),
                selectors: &self.dep.settings.selectors,
                form: &self.work.bench.scope_form,
            },
            view::settings::SettingsData {
                settings: &self.dep.settings,
                form: &self.work.bench.settings_form,
                theme: self.chrome.prefs.theme.label(),
                zoom: self.chrome.prefs.zoom,
                echo: (
                    self.work.echo.echo.len(),
                    self.work.echo.echo.evicted(),
                    self.work.echo.echo.lagged(),
                ),
                history: self
                    .sub
                    .follow
                    .history
                    .as_ref()
                    .map(|r| (r.ring.len(), r.ring.evicted())),
                keys: (self.obs.keys, self.obs.keys_evicted),
            },
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
}

#[cfg(test)]
#[path = "app_tests.rs"]
mod tests;

/// The settings a bus-less test window runs under (#175, #186).
#[cfg(test)]
pub(crate) fn test_settings() -> Settings {
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
    }
}

/// A window with no bus behind it, for the tests in [`tests`] (#175).
#[cfg(test)]
fn test_app() -> Zengui {
    Zengui::with_prefs(test_settings(), crate::prefs::Prefs::default(), None).0
}

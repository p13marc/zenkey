//! The Elm loop: state, `update`, `view`, `subscription`.
//!
//! **Lazy by default** (issue #85): connecting builds the *skeleton* — the
//! declared keyspace from registry + liveliness + admin metadata — and starts
//! a monitor with **zero data-plane watches**. Observation is opt-in per
//! subtree (the tree's watch toggles) or per scope (the toolbar's "observe
//! scope" toggle — the old eager mode made explicit); a selection fetches one
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
        let app = Zengui {
            chrome: Chrome::new(prefs, prefs_note),
            dep: Deployment::new(settings),
            obs: Observation::default(),
            sub: SubjectState::default(),
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
    /// `subject` through the causal chain from `Select`, `workspace`
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
        let panes = view::panes::split(
            &self.chrome,
            &self.dep,
            &self.obs,
            &self.sub,
            &self.tree,
            &self.work,
        );

        let mut layout = column![view::toolbar::strip(
            &self.chrome,
            &self.dep,
            &self.obs,
            &self.work
        )]
        .spacing(space::MD)
        .padding(space::MD);
        // Replay-mode surfaces (#74) sit between the toolbar and the panes,
        // so the mode is unmistakable.
        for surface in view::replay::surfaces(&self.work.replay) {
            layout = layout.push(surface);
        }
        // The Activity dock (#183): the session's parallel streams, below the
        // subject-scoped panes because that is what they are about. `panes`
        // takes what is left after the dock and the strips, so putting the
        // dock away gives the space back rather than leaving a hole.
        let layout = layout
            .push(panes)
            .push(view::activity::dock(view::activity::ActivityData {
                dock: &self.work.activity,
                echo: &self.work.echo.echo,
                echo_view: &self.work.echo.echo_view,
                echo_scroll: self.work.echo.echo_scroll,
                follow: self
                    .work
                    .echo
                    .echo_view
                    .follow_subject
                    .then(|| self.sub.current.key())
                    .flatten(),
                next_seq: self.work.echo.echo.next_seq(),
                publish: &self.work.bench.publish_form,
                doctor: &self.work.verdicts.doctor,
                base: self.dep.base(),
                replay: &self.work.replay,
                slices: self.dep.slices.as_deref(),
            }))
            .push(view::status::strip(Status::of(
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
}

#[cfg(test)]
#[path = "app_tests.rs"]
mod tests;

/// A window with no bus behind it, for the tests in [`tests`] (#175).
#[cfg(test)]
fn test_app() -> Zengui {
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

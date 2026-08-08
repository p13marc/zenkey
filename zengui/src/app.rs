//! The Elm loop: state, `update`, `view`, `subscription`.

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use iced::widget::{column, pick_list, row, text};
use iced::{Element, Length, Subscription, Task};
use zenkey_fleet::{DiscoveredBase, KeyTreeSnapshot, SliceSet};

use crate::config::Settings;
use crate::echo::EchoRing;
use crate::keyfacts::KeyFacts;
use crate::link::{self, LinkKey};
use crate::message::{BusTick, LinkState, Message};
use crate::scope::{self, ScopePreset};
use crate::view;
use crate::view::status::{SliceSource, Status};
use crate::view::tokens::space;

/// Cap on tree rows built per frame.
const MAX_ROWS: usize = 2000;

pub struct Zengui {
    settings: Settings,
    session: Option<zenoh::Session>,
    link: LinkState,
    /// Bumped to force the subscription to restart.
    epoch: u64,

    tree: Arc<KeyTreeSnapshot>,
    /// The flattened tree, rebuilt when the snapshot or the expansion set
    /// changes — never per frame. Rebuilding view state every frame is what
    /// pegged zensight's UI thread; the same mistake is cheap to avoid here.
    flat: view::tree::Flattened,
    echo: EchoRing,
    expanded: BTreeSet<String>,
    selected: Option<String>,
    echo_filter: String,

    /// Projections, computed once per newly-seen key and invalidated wholesale
    /// when the base changes (the base is an input to the projection).
    facts: HashMap<String, KeyFacts>,
    slices: Option<Arc<SliceSet>>,
    slice_source: SliceSource,

    bases: Vec<DiscoveredBase>,
    keys: usize,
    keys_evicted: u64,
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
            link: LinkState::Connecting,
            epoch: 0,
            tree: Arc::new(KeyTreeSnapshot::default()),
            flat: view::tree::Flattened {
                rows: Vec::new(),
                truncated: 0,
            },
            echo,
            expanded: BTreeSet::new(),
            selected: None,
            echo_filter: String::new(),
            facts: HashMap::new(),
            slices: None,
            slice_source: SliceSource::None,
            bases: Vec::new(),
            keys: 0,
            keys_evicted: 0,
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
                self.epoch += 1;
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
                Task::batch([discover, self.load_slices()])
            }
            Message::SessionOpened(Err(e)) => {
                self.link = LinkState::Failed(e);
                Task::none()
            }
            Message::BasesDiscovered(Ok(bases)) => {
                self.bases = bases;
                Task::none()
            }
            // A failed sweep is reported, not treated as "no bases".
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
            Message::BaseSelected(base) => {
                if base == self.settings.base {
                    return Task::none();
                }
                self.settings.base = base;
                // The base is an input to every projection, so they all die.
                // Wholesale invalidation is cheaper and more obviously correct
                // than trying to repair them in place.
                self.facts.clear();
                // Chunk roles are base-relative too, so the rows are stale.
                self.reflatten();
                // A base-relative scope now means something different.
                if self.settings.scope != ScopePreset::Everything {
                    self.epoch += 1;
                }
                self.load_slices()
            }
            Message::ScopeSelected(scope) => {
                if scope == self.settings.scope {
                    return Task::none();
                }
                self.settings.scope = scope;
                // Changing the hashed LinkKey is the restart: iced tears the
                // old stream down, dropping the Monitor (and, thanks to its
                // Drop impl, its subscriber tasks).
                self.epoch += 1;
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
                self.selected = key;
                Task::none()
            }
            Message::EchoFilterChanged(f) => {
                self.echo_filter = f;
                Task::none()
            }
            Message::ClearEcho => {
                // Clears the lines but deliberately not the loss counters:
                // laundering a known gap into a clean scrollback would be the
                // dishonesty the counters exist to prevent.
                self.echo.clear();
                Task::none()
            }
            Message::Reconnect => {
                self.epoch += 1;
                Task::none()
            }
        }
    }

    fn apply_tick(&mut self, tick: &BusTick) {
        self.tree = Arc::clone(&tick.tree);
        self.keys = tick.keys;
        self.keys_evicted = tick.keys_evicted;
        self.totals = tick.totals;
        self.echo.record_lag(tick.lagged + tick.coalesced);
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
        self.flat = view::tree::flatten(&self.tree, &self.settings.base, &self.expanded, MAX_ROWS);
    }

    /// Project a key the first time it is seen, and never again.
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

    /// Re-run registry resolution over every known key. Cheap: the structural
    /// projection is untouched, only the registration changes.
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
        if !dirs.is_empty() {
            return Task::perform(
                async move {
                    tokio::task::spawn_blocking(move || SliceSet::from_dirs(&dirs))
                        .await
                        .map_err(|e| e.to_string())?
                        .map(Arc::new)
                        .map_err(|e| e.to_string())
                },
                Message::SlicesLoaded,
            );
        }
        let Some(session) = self.session.clone() else {
            return Task::none();
        };
        let base = self.settings.base.clone();
        let timeout = self.settings.timeout();
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

    /// The selectors currently in force.
    fn selectors(&self) -> Vec<String> {
        self.settings
            .scope
            .selectors(&self.settings.base, &self.settings.selectors)
    }

    pub fn subscription(&self) -> Subscription<Message> {
        let Some(session) = self.session.clone() else {
            return Subscription::none();
        };
        link::subscribe(LinkKey {
            session,
            selectors: self.selectors(),
            // Presence needs both the fleet sweep and @catalog by name (D4).
            // Before a base is chosen, sweep every base at once.
            liveliness: if self.settings.base.is_empty() && self.bases.is_empty() {
                scope::liveliness_any_base()
            } else {
                scope::liveliness_selectors(&self.settings.base)
            },
            max_keys: self.settings.max_keys,
            epoch: self.epoch,
        })
    }

    pub fn view(&self) -> Element<'_, Message> {
        let panes = row![
            iced::widget::container(view::tree::pane(
                &self.flat,
                &self.facts,
                self.selected.as_deref()
            ))
            .width(Length::FillPortion(1))
            .height(Length::Fill),
            iced::widget::container(view::echo::pane(
                &self.echo,
                &self.echo_filter,
                self.selected.as_deref(),
            ))
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
                totals: self.totals,
                slices: &self.slice_source,
                unreachable: self.settings.is_unreachable(),
            }),
        ]
        .spacing(space::MD)
        .padding(space::MD)
        .into()
    }

    fn toolbar(&self) -> Element<'_, Message> {
        // The empty base is a real deployment, not a blank entry, so it is
        // always offered even when the sweep found nothing (RFC 09 §5).
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

        row![
            text("base").size(crate::view::tokens::font::CAPTION),
            base_picker,
            text("scope").size(crate::view::tokens::font::CAPTION),
            scope_picker,
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

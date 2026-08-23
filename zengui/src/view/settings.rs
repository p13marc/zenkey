//! The Settings overlay (#188) — the launch-only knobs, surfaced.
//!
//! Five settings were launch-only and invisible afterwards: `--registry`,
//! `--max-keys`, `--echo-lines`, `--history-entries` and `--eager`. A user
//! who tripped the key-table bound had no way to raise it and — worse — no
//! way to *see* it, in an application whose whole moat is that every bound
//! reports its cost.
//!
//! The contract, enforced structurally: [`pane`] **destructures
//! [`Settings`](crate::config::Settings) exhaustively**, so a field added
//! there fails to compile here until this overlay either controls it or
//! documents it as owned elsewhere. Live-apply where the engine allows it
//! (the echo ring and the history recorder resize in place); everything else
//! is labelled "takes effect on reconnect" rather than silently doing
//! nothing; and each bound states the cost of raising it in the voice the
//! status strip already uses — [`keys_text`] literally *is* the strip's
//! wording.

use std::path::PathBuf;

use iced::Element;
use iced::widget::{column, row, text};

use crate::message::{ChromeMsg, DeploymentMsg, Message, PaneMsg, PrefsMsg};
use crate::view::kit;
use crate::view::status::keys_text;
use crate::view::theme::colors;
use crate::view::tokens::{font, space};

/// The overlay's editable state (owned by the app, like the Connect form).
#[derive(Debug, Clone, Default)]
pub struct SettingsForm {
    pub echo_lines: String,
    pub history_entries: String,
    pub max_keys: String,
    pub timeout: String,
    pub eager: bool,
    /// Registry directories, comma- or space-separated; empty = the bus's
    /// own slices (RFC 08 §6 introspection).
    pub registry: String,
    /// The last apply's outcome, rendered verbatim.
    pub status: Option<Result<String, String>>,
}

impl SettingsForm {
    /// Fill the editor from the settings in force — what opening the overlay
    /// does, so the boxes always start from the truth.
    pub fn seed(&mut self, s: &crate::config::Settings) {
        self.echo_lines = s.echo_lines.to_string();
        self.history_entries = s.history_entries.to_string();
        self.max_keys = s.max_keys.to_string();
        self.timeout = s.timeout_secs.to_string();
        self.eager = s.eager;
        self.registry = s
            .registry
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join(" ");
        self.status = None;
    }
}

/// A validated apply: what the deployment is asked to become (#188).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tuning {
    pub echo_lines: usize,
    pub history_entries: usize,
    pub max_keys: usize,
    pub timeout_secs: u64,
    pub eager: bool,
    pub registry: Vec<PathBuf>,
}

/// Messages the overlay emits.
#[derive(Debug, Clone)]
pub enum SettingsMsg {
    EchoLinesChanged(String),
    HistoryEntriesChanged(String),
    MaxKeysChanged(String),
    TimeoutChanged(String),
    EagerToggled(bool),
    RegistryChanged(String),
    /// Validate the whole form; a valid one becomes a [`Tuning`].
    Apply,
}

fn msg(m: SettingsMsg) -> Message {
    Message::Pane(PaneMsg::Settings(m))
}

/// Everything the overlay shows, as plain data.
pub struct SettingsData<'a> {
    pub settings: &'a crate::config::Settings,
    pub form: &'a SettingsForm,
    /// The chrome preferences shown beside the knobs (issue #73's controls,
    /// re-grouped here): the active theme's label and the zoom factor.
    pub theme: &'static str,
    pub zoom: f32,
    /// The echo ring's population and losses: (lines held, evicted, lagged).
    pub echo: (usize, u64, u64),
    /// The history recorder, when a key is recording: (entries, evicted).
    pub history: Option<(usize, u64)>,
    /// The engine's key table: (tracked, evicted) — `keys_text`'s inputs.
    pub keys: (usize, u64),
}

/// The #188 invariant, stated where the bounds are set.
pub const BOUND_INVARIANT: &str = "raising a bound never clears the counter that reported the \
     old bound's cost (RFC 09 §5.1 O6)";

pub fn pane(d: SettingsData<'_>) -> Element<'_, Message> {
    // The acceptance, structural: every `Settings` field is either a control
    // below or documented below as owned elsewhere. A field added to the
    // struct fails to compile here until it is accounted for.
    let crate::config::Settings {
        base,
        connect,
        listen,
        scouting,
        zenoh_config,
        registry: _,     // controlled below (the forget path)
        timeout_secs: _, // controlled below (live, next query)
        scope,
        selectors,
        eager: _,           // controlled below (reconnect)
        echo_lines: _,      // controlled below (live)
        history_entries: _, // controlled below (live)
        max_keys: _,        // controlled below (reconnect)
    } = d.settings;

    let muted = |theme: &iced::Theme| text::Style {
        color: Some(colors(theme).text_muted()),
    };

    let mut col = column![
        kit::section_header("Settings", None),
        // ── Window (issue #73's chrome preferences, grouped here) ──
        kit::caption("window"),
        row![
            kit::action(kit::caption(format!("theme: {}", d.theme)))
                .on_press(Message::Chrome(ChromeMsg::Prefs(PrefsMsg::ThemeToggled)))
                .padding(4),
            kit::action(kit::caption("-"))
                .on_press(Message::Chrome(ChromeMsg::Prefs(PrefsMsg::ZoomOut)))
                .padding(4),
            kit::action(kit::caption(format!(
                "{}%",
                (d.zoom * 100.0).round() as i32
            )))
            .on_press(Message::Chrome(ChromeMsg::Prefs(PrefsMsg::ZoomReset)))
            .padding(4),
            kit::action(kit::caption("+"))
                .on_press(Message::Chrome(ChromeMsg::Prefs(PrefsMsg::ZoomIn)))
                .padding(4),
        ]
        .spacing(space::SM),
        // ── The bounds that apply live ──
        kit::caption("bounds — applied live"),
        kit::body(BOUND_INVARIANT).style(muted),
    ]
    .spacing(space::SM);

    let (echo_len, echo_evicted, echo_lagged) = d.echo;
    col = col.push(input_row(
        "echo lines",
        "how many echo lines to retain",
        &d.form.echo_lines,
        SettingsMsg::EchoLinesChanged,
    ));
    col = col.push(
        kit::body(format!(
            "applies live — a larger ring is more memory and a longer filter \
             scan. now: {} held (+{echo_evicted} evicted, {echo_lagged} lagged)",
            kit::plural(echo_len, "line"),
        ))
        .style(muted),
    );
    col = col.push(input_row(
        "history entries",
        "how many samples of the selected key to retain",
        &d.form.history_entries,
        SettingsMsg::HistoryEntriesChanged,
    ));
    col = col.push(
        kit::body(format!(
            "applies live — history keeps whole payloads so it can diff them: \
             the costliest bound per entry. now: {}",
            match d.history {
                Some((len, evicted)) => format!("{len} entries (+{evicted} evicted)"),
                None => "no key recording (nothing selected)".to_string(),
            }
        ))
        .style(muted),
    );
    col = col.push(input_row(
        "timeout (s)",
        "query timeout in seconds",
        &d.form.timeout,
        SettingsMsg::TimeoutChanged,
    ));
    col = col.push(kit::body("applies live — from the next query").style(muted));

    // ── The bounds that need a reconnect ──
    let (keys, keys_evicted) = d.keys;
    col = col.push(kit::caption("bounds — take effect on reconnect"));
    col = col.push(input_row(
        "max keys",
        "how many distinct keys to keep statistics for",
        &d.form.max_keys,
        SettingsMsg::MaxKeysChanged,
    ));
    col = col.push(
        kit::body(format!(
            "takes effect on reconnect — a larger key table is more memory. \
             now: {}",
            keys_text(keys, keys_evicted),
        ))
        .style(muted),
    );
    col = col.push(
        kit::check(d.form.eager)
            .label("eager — observe the scope immediately on connect")
            .on_toggle(|b| msg(SettingsMsg::EagerToggled(b)))
            .text_size(font::CAPTION),
    );
    col = col.push(
        kit::body(
            "takes effect on reconnect — the live equivalent is the location \
             bar's observe-scope toggle (#85)",
        )
        .style(muted),
    );

    // ── The registry, which is not a bound at all ──
    col = col.push(kit::caption("registry"));
    col = col.push(input_row(
        "registry dirs",
        "registry dirs (registry/*.toml), space-separated — empty = the bus's slices",
        &d.form.registry,
        SettingsMsg::RegistryChanged,
    ));
    col = col.push(
        kit::body(
            "a registry change re-runs the slice union and can change every \
             registration badge in the tree — applying it takes the same \
             forget path a base change does: every verdict about the old \
             slices is dropped rather than left on screen (O4). To keep it \
             across launches, save it into a context (Connect).",
        )
        .style(muted),
    );

    col = col.push(
        row![
            kit::action(kit::caption("apply"))
                .on_press(msg(SettingsMsg::Apply))
                .padding(4),
            kit::action(kit::caption("reconnect now"))
                .on_press(Message::Deployment(DeploymentMsg::Reconnect))
                .padding(4),
        ]
        .spacing(space::SM),
    );

    if let Some(status) = &d.form.status {
        col = col.push(match status {
            Ok(s) => Element::from(kit::body(s.clone()).style(muted)),
            Err(e) => kit::body(e.clone())
                .style(|theme: &iced::Theme| text::Style {
                    color: Some(colors(theme).danger()),
                })
                .into(),
        });
    }

    // ── Every remaining Settings field, documented where it lives ──
    col = col.push(kit::caption("owned elsewhere"));
    col = col.push(
        kit::body(format!(
            "base: {} — the location bar's base picker owns it",
            crate::config::base_label(base),
        ))
        .style(muted),
    );
    col = col.push(
        kit::body(format!(
            "connect ({}), listen ({}), scouting ({}), zenoh config ({}) — \
             session setup, owned by the Connect overlay (Ctrl+Shift+C); a \
             change there reopens the session",
            if connect.is_empty() {
                "none".to_string()
            } else {
                connect.join(" ")
            },
            if listen.is_empty() {
                "none".to_string()
            } else {
                listen.join(" ")
            },
            match scouting {
                Some(true) => "on",
                Some(false) => "off",
                None => "unset — off unless a config file says otherwise",
            },
            match zenoh_config {
                Some(p) => p.display().to_string(),
                None => "none".to_string(),
            },
        ))
        .style(muted),
    );
    col = col.push(
        kit::body(format!(
            "scope ({}, {} custom selectors) — the location bar's scope \
             picker and its selectors editor own them",
            scope.short(),
            selectors.len(),
        ))
        .style(muted),
    );
    col.into()
}

/// One labelled text box.
fn input_row<'a>(
    label: &'a str,
    placeholder: &'a str,
    value: &'a str,
    on_input: impl Fn(String) -> SettingsMsg + 'a,
) -> Element<'a, Message> {
    row![
        kit::caption(label).width(iced::Length::Fixed(110.0)),
        kit::input(placeholder, value)
            .on_input(move |t| msg(on_input(t)))
            .on_submit(msg(SettingsMsg::Apply))
            .size(font::CAPTION),
    ]
    .spacing(space::SM)
    .align_y(iced::Alignment::Center)
    .into()
}

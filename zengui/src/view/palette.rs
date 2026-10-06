//! The command palette and jump-to-key overlay (issue #75).
//!
//! By the end of this epic zengui has a handful of docks and dozens of
//! actions;
//! discoverability by location bar alone stops scaling well before that. zensight
//! proved the in-family pattern (Ctrl+K search, Ctrl+P palette, `?` help), and
//! this is the zengui shape of it.
//!
//! **The rule that makes a palette safe to add**: every entry emits a
//! [`Message`] the UI *also* emits. A palette that reimplements an action is
//! two code paths that will disagree, and the one nobody clicks is the one
//! that rots — so `actions()` returns messages, the tests assert they are the
//! same messages the UI path sends, and there is nothing else to keep in step.
//!
//! Jump-to-key is the same idea over data instead of verbs: fuzzy over the
//! keys *actually observed*, and selecting one emits the ordinary
//! [`SubjectMsg::Select`](crate::message::SubjectMsg::Select). It offers
//! nothing it has not seen, which keeps the overlay from inventing a keyspace (O4 — a suggestion is not an observation,
//! and these are only ever the latter).

use iced::widget::{Column, column, row};
use iced::{Element, Length, Padding};

use crate::message::{
    ChromeMsg, DeploymentMsg, Message, PaneMsg, PrefsMsg, RightPane, WorkspaceMsg,
};
use crate::view::kit;
use crate::view::tokens::{face, font, space};

/// How many rows the overlay renders. A palette that draws a 50k-key list is
/// the same bug as a tree that does.
///
/// Generous rather than tight because the unfiltered list starts with the
/// panes and the scopes: at twelve, a fresh palette showed *only* those and
/// nothing else — which reads as "this is all there is" rather than "type to
/// see more". Twenty-four since #187/#188 grew the fixed vocabulary (custom
/// scope, the two overlay entries): the bound must keep the first verbs —
/// run doctor, reconnect — inside the first screenful.
const MAX_ROWS: usize = 24;

/// Which overlay is open, if any. One field rather than three booleans, so
/// "two overlays at once" has no representation — which is also the layering
/// answer for Connect (#185): the palette and the Connect overlay cannot be
/// open together, and Esc peels whichever one is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Overlay {
    #[default]
    None,
    /// Ctrl+P — fuzzy over actions.
    Commands,
    /// Ctrl+K — fuzzy over observed keys.
    Keys,
    /// `?` — the shortcut map.
    Help,
    /// Ctrl+Shift+C — contexts and endpoints (#185). A session-scoped modal,
    /// not a pane: it is about how the window reaches a bus, never about the
    /// subject.
    Connect,
    /// The key-expression editor (#187), behind the location bar's scope
    /// chip: the resolved selectors of the current scope, and the way to
    /// fork them into a custom set.
    Selectors,
    /// Ctrl+, — the launch knobs, surfaced (#188): the bounds and their
    /// costs, live-applied where the engine allows it.
    Settings,
}

/// The overlay's state (owned by the app).
#[derive(Debug, Clone, Default)]
pub struct PaletteState {
    pub overlay: Overlay,
    pub query: String,
    /// Highlighted row, as an index into the filtered list.
    pub cursor: usize,
}

impl PaletteState {
    pub fn open(&mut self, overlay: Overlay) {
        self.overlay = overlay;
        self.query.clear();
        self.cursor = 0;
    }

    pub fn close(&mut self) {
        self.overlay = Overlay::None;
        self.query.clear();
        self.cursor = 0;
    }

    pub fn is_open(&self) -> bool {
        self.overlay != Overlay::None
    }
}

/// Messages the overlay emits.
#[derive(Debug, Clone)]
pub enum PaletteMsg {
    Open(Overlay),
    Close,
    QueryChanged(String),
    CursorUp,
    CursorDown,
    /// Run the highlighted row.
    Activate,
    /// Run a row directly (a click).
    Pick(usize),
}

/// One palette entry.
pub struct Action {
    /// What it is called, and what the fuzzy match runs over.
    pub label: String,
    /// The message it sends — the same one the UI path sends.
    pub message: Message,
    /// Where it is filed (#558): a section of the unfiltered list, the
    /// trailing word of a ranked one.
    pub group: ActionGroup,
    /// The glyph its own control wears, where it has one.
    pub icon: kit::Icon,
}

/// The palette's sections (#558), in the order an unfiltered palette shows
/// them: the session and the diagnoses first, so "reconnect" and "run
/// doctor" are always on the first screen — the verbs a lost user opens the
/// palette for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ActionGroup {
    Session,
    Diagnose,
    Scope,
    Go,
    Activity,
    Echo,
    View,
    Layout,
}

impl ActionGroup {
    /// The section's eyebrow.
    pub fn label(self) -> &'static str {
        match self {
            ActionGroup::Session => "SESSION",
            ActionGroup::Diagnose => "DIAGNOSE",
            ActionGroup::Scope => "SCOPE",
            ActionGroup::Go => "GO TO",
            ActionGroup::Activity => "ACTIVITY",
            ActionGroup::Echo => "ECHO",
            ActionGroup::View => "VIEW",
            ActionGroup::Layout => "LAYOUT",
        }
    }

    /// The trailing word of a ranked row.
    pub fn word(self) -> &'static str {
        match self {
            ActionGroup::Session => "session",
            ActionGroup::Diagnose => "diagnose",
            ActionGroup::Scope => "scope",
            ActionGroup::Go => "go to",
            ActionGroup::Activity => "activity",
            ActionGroup::Echo => "echo",
            ActionGroup::View => "view",
            ActionGroup::Layout => "layout",
        }
    }
}

/// Every command the palette offers.
///
/// Built from the same enums the UI is built from (`RightPane::ALL`,
/// `ScopePreset`), so a pane or scope added elsewhere shows up here without
/// anyone remembering to add it.
pub fn actions(contexts: &[String]) -> Vec<Action> {
    use ActionGroup as G;
    use kit::Icon;
    let act = |label: String, message: Message, group: ActionGroup, icon: Icon| Action {
        label,
        message,
        group,
        icon,
    };
    let mut out: Vec<Action> = RightPane::ALL
        .into_iter()
        .map(|p| {
            act(
                format!("go to {} pane", p.label()),
                Message::Workspace(WorkspaceMsg::PaneSelected(p)),
                G::Go,
                crate::view::panes::tool_icon(p),
            )
        })
        .collect();

    // The Activity dock's streams (#183). They lost their tab-strip entries
    // when they stopped being right-hand panes, and a stream you can only
    // reach by clicking the right tab is one the palette has stopped
    // covering.
    for t in crate::message::ActivityTab::ALL {
        out.push(act(
            format!("activity: {}", t.label()),
            Message::Workspace(WorkspaceMsg::ActivityTab(t)),
            G::Activity,
            crate::view::activity::tab_icon(t),
        ));
    }

    // `ScopePreset::ALL`, custom included (#187): the palette must not
    // re-create the defect the picker had — an option list that excludes a
    // scope the window can be in.
    for scope in crate::scope::ScopePreset::ALL {
        out.push(act(
            format!("scope: {}", scope.short()),
            Message::Deployment(DeploymentMsg::ScopeSelected(scope)),
            G::Scope,
            Icon::Filter,
        ));
    }

    for name in contexts {
        out.push(act(
            format!("context: {name}"),
            Message::Pane(PaneMsg::Context(
                crate::view::contexts::ContextMsg::Selected(name.clone()),
            )),
            G::Session,
            Icon::Plug,
        ));
    }

    out.extend([
        // Connect stopped being a pane (#185), so it is not in the generated
        // pane list any more — the overlay gets its own entry, sending the
        // same message the location bar's context chip and Ctrl+Shift+C send.
        act(
            "connect — contexts and endpoints".into(),
            Message::Chrome(ChromeMsg::Palette(PaletteMsg::Open(Overlay::Connect))),
            G::Session,
            Icon::Plug,
        ),
        // The key-expression editor (#187): the same message the location
        // bar's selectors chip sends.
        act(
            "edit scope selectors".into(),
            Message::Chrome(ChromeMsg::Palette(PaletteMsg::Open(Overlay::Selectors))),
            G::Scope,
            Icon::Selectors,
        ),
        // The Settings overlay (#188): the same message the location bar's
        // settings chip and Ctrl+, send.
        act(
            "settings — bounds and launch knobs".into(),
            Message::Chrome(ChromeMsg::Palette(PaletteMsg::Open(Overlay::Settings))),
            G::Session,
            Icon::Settings,
        ),
        act(
            "observe scope (start/stop)".into(),
            Message::Deployment(DeploymentMsg::ScopeWatchToggled),
            G::Scope,
            Icon::Watched,
        ),
        act(
            "run doctor".into(),
            Message::Pane(PaneMsg::Doctor(crate::view::doctor::DoctorMsg::Run)),
            G::Diagnose,
            Icon::Doctor,
        ),
        // The why ladder (#214): the same message the Inspector's "why?"
        // button sends, run on the current subject key at the frugal
        // default (no listen).
        act(
            "why is this key silent?".into(),
            Message::Pane(PaneMsg::Why(
                crate::message::SlotId::FOLLOW,
                crate::view::why::WhyMsg::Run,
            )),
            G::Diagnose,
            Icon::Silent,
        ),
        act(
            "reconnect".into(),
            Message::Deployment(DeploymentMsg::Reconnect),
            G::Session,
            Icon::Reconnect,
        ),
        act(
            "toggle theme".into(),
            Message::Chrome(ChromeMsg::Prefs(PrefsMsg::ThemeToggled)),
            G::View,
            Icon::Dark,
        ),
        act(
            "toggle density (comfortable/compact)".into(),
            Message::Chrome(ChromeMsg::Prefs(PrefsMsg::DensityToggled)),
            G::View,
            Icon::Density,
        ),
        act(
            "zoom in".into(),
            Message::Chrome(ChromeMsg::Prefs(PrefsMsg::ZoomIn)),
            G::View,
            Icon::ZoomIn,
        ),
        act(
            "zoom out".into(),
            Message::Chrome(ChromeMsg::Prefs(PrefsMsg::ZoomOut)),
            G::View,
            Icon::ZoomOut,
        ),
        act(
            "reset zoom".into(),
            Message::Chrome(ChromeMsg::Prefs(PrefsMsg::ZoomReset)),
            G::View,
            Icon::Search,
        ),
        act(
            "clear echo".into(),
            Message::Pane(PaneMsg::Echo(crate::view::echo::EchoMsg::Clear)),
            G::Echo,
            Icon::Clear,
        ),
        act(
            "export echo as ndjson".into(),
            Message::Pane(PaneMsg::Echo(crate::view::echo::EchoMsg::Export)),
            G::Echo,
            Icon::Export,
        ),
        act(
            "pause/follow echo".into(),
            Message::Pane(PaneMsg::Echo(crate::view::echo::EchoMsg::FollowToggled)),
            G::Echo,
            Icon::Pause,
        ),
    ]);

    // The workspace grid (#180), past the first screenful on purpose — the
    // bound keeps the fresh overlay a sampler, and typing (or Alt+1/2/3, or
    // the dock strip) is the fast route to these anyway.
    //
    // The saved layouts: the same messages Alt+1/2/3 send.
    for preset in crate::prefs::LayoutPreset::ALL {
        out.push(act(
            format!("layout: {}", preset.label()),
            Message::Workspace(WorkspaceMsg::LayoutPreset(preset)),
            G::Layout,
            crate::view::location::preset_icon(preset),
        ));
    }
    // The dock toggles: the same message the dock strip and each title bar's
    // `×` send — and the keyboard route back to a closed dock.
    for role in crate::prefs::DockRole::ALL {
        out.push(act(
            format!("toggle {} dock", role.label()),
            Message::Workspace(WorkspaceMsg::DockToggled(role)),
            G::Layout,
            crate::view::panes::dock_icon(role),
        ));
    }
    out
}

/// The order the command palette shows `items` in (#558). Unfiltered, by
/// section — [`ActionGroup`]'s order, then the list's — so the first screen
/// is the session and the diagnoses; with a query, best match first
/// ([`rank`]). Bounded either way, by the same `MAX_ROWS`: the section
/// headers are drawn, not counted.
///
/// The one ordering the view and the cursor share, so the row the cursor
/// highlights is the row Enter runs.
pub fn command_order(items: &[Action], query: &str) -> Vec<usize> {
    if query.trim().is_empty() {
        let mut order: Vec<usize> = (0..items.len()).collect();
        order.sort_by_key(|&i| (items[i].group, i));
        order.truncate(MAX_ROWS);
        order
    } else {
        rank(items, query, |a| a.label.as_str())
    }
}

/// Subsequence fuzzy match, case-insensitive: `gtd` finds "go to detail pane".
///
/// Returns a score — lower is better — so ties break toward the shorter,
/// tighter match rather than by whatever order the list happened to be in.
pub fn fuzzy(haystack: &str, needle: &str) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    let hay: Vec<char> = haystack.to_lowercase().chars().collect();
    let mut score = 0usize;
    let mut at = 0usize;
    for want in needle.to_lowercase().chars() {
        if want == ' ' {
            continue;
        }
        let found = hay[at..].iter().position(|c| *c == want)?;
        // A gap costs: a match spread across the string ranks below a
        // contiguous one.
        score += found;
        at += found + 1;
    }
    Some(score + haystack.len())
}

/// Rank a list by the query, best first, bounded.
pub fn rank<T>(items: &[T], query: &str, label: impl Fn(&T) -> &str) -> Vec<usize> {
    let mut scored: Vec<(usize, usize)> = items
        .iter()
        .enumerate()
        .filter_map(|(i, item)| fuzzy(label(item), query).map(|s| (s, i)))
        .collect();
    scored.sort_by_key(|(score, i)| (*score, *i));
    scored.into_iter().take(MAX_ROWS).map(|(_, i)| i).collect()
}

/// Render the open overlay, if any.
///
/// `keys` is consumed only by the jump-to overlay, and only the rows
/// actually drawn (≤ `MAX_ROWS`, a private bound) are cloned into the
/// element — opening
/// the overlay costs what it draws, not the cache's population, and any
/// other overlay state never touches the iterator (#110). One deliberate
/// non-change: `rank` breaks score ties by index, and the indices now come
/// from the cache's iteration order — but so did the old per-frame snapshot,
/// so tie jitter across cache mutations is pre-existing, and a full sort of
/// 50k borrowed keys per frame would reintroduce O(n log n) to cure a
/// cosmetic.
pub fn overlay<'a>(
    state: &'a PaletteState,
    form: &'a crate::view::contexts::ContextForm,
    unreachable: bool,
    scope: crate::view::scope_editor::ScopeEditorData<'a>,
    settings: crate::view::settings::SettingsData<'a>,
    keys: impl Iterator<Item = &'a str>,
) -> Option<Element<'a, Message>> {
    match state.overlay {
        Overlay::None => None,
        Overlay::Help => Some(help()),
        Overlay::Connect => Some(connect(form, unreachable)),
        Overlay::Selectors => Some(selectors(scope)),
        Overlay::Settings => Some(floated(crate::view::settings::pane(settings))),
        Overlay::Commands => {
            let items = actions(&form.known);
            let order = command_order(&items, &state.query);
            // Unfiltered, the list is sectioned; ranked, each row names its
            // section instead — a header between ranked rows would split
            // the best match from the next.
            let grouped = state.query.trim().is_empty();
            let mut previous = None;
            let rows = order
                .iter()
                .map(|&i| {
                    let a = &items[i];
                    let header = (grouped && previous != Some(a.group)).then(|| a.group.label());
                    previous = Some(a.group);
                    ListRow {
                        label: a.label.clone(),
                        icon: Some(a.icon),
                        mono: false,
                        keys: crate::shortcuts::keys_for(&a.message),
                        header,
                        trailing: (!grouped).then(|| a.group.word()),
                    }
                })
                .collect();
            Some(list(
                state,
                (kit::Icon::Command, "Command palette"),
                "type a command…",
                None,
                rows,
            ))
        }
        Overlay::Keys => {
            // Fat pointers only — no key bytes are copied to rank (#110).
            let keys: Vec<&str> = keys.collect();
            let order = rank(&keys, &state.query, |k| *k);
            Some(list(
                state,
                (kit::Icon::Keys, "Jump to key"),
                "type part of a key…",
                Some("fuzzy over keys observed so far — nothing here is a guess (O4)"),
                order
                    .iter()
                    .map(|i| ListRow {
                        label: keys[*i].to_string(),
                        icon: None,
                        mono: true,
                        keys: None,
                        header: None,
                        trailing: None,
                    })
                    .collect(),
            ))
        }
    }
}

/// One drawn row of a list overlay (#558).
struct ListRow {
    label: String,
    icon: Option<kit::Icon>,
    /// A key is mono; a command is sans — it is read, not compared.
    mono: bool,
    /// The chord that runs it, from the shortcut map ([`crate::shortcuts::keys_for`]).
    keys: Option<&'static str>,
    /// A section eyebrow drawn above this row — the unfiltered palette.
    header: Option<&'static str>,
    /// The section as a trailing word — the ranked palette.
    trailing: Option<&'static str>,
}

/// A list overlay: the shared header, a search field, the rows, and the
/// keys that drive them as a footer.
fn list<'a>(
    state: &'a PaletteState,
    (icon, title): (kit::Icon, &'static str),
    placeholder: &str,
    hint: Option<&'static str>,
    rows: Vec<ListRow>,
) -> Element<'a, Message> {
    let input = kit::search(placeholder, &state.query)
        .on_input(|q| Message::Chrome(ChromeMsg::Palette(PaletteMsg::QueryChanged(q))))
        .on_submit(Message::Chrome(ChromeMsg::Palette(PaletteMsg::Activate)))
        .size(font::BODY);

    let mut body = Column::new().spacing(space::XS);
    if rows.is_empty() {
        body = body.push(kit::muted("nothing matches"));
    }
    for (i, r) in rows.into_iter().enumerate() {
        if let Some(h) = r.header {
            // Air above every section but the first; the eyebrow is a
            // label, not a row — the cursor never lands on it.
            let air = if i == 0 { 0.0 } else { space::SM };
            body = body.push(
                iced::widget::container(kit::eyebrow(h))
                    .padding(Padding::ZERO.top(air).left(space::XS)),
            );
        }
        let selected = i == state.cursor;
        let label = kit::caption(r.label);
        let mut line = row![
            // The cursor's mark holds its width when absent, so the labels
            // never shift as the cursor moves.
            if selected {
                Element::from(kit::icon_caption(kit::Icon::ChevronRight))
            } else {
                iced::widget::Space::new()
                    .width(Length::Fixed(font::CAPTION))
                    .into()
            },
        ]
        .spacing(space::SM)
        .align_y(iced::Alignment::Center);
        if let Some(icon) = r.icon {
            line = line.push(kit::icon_caption(icon));
        }
        line = line.push(if r.mono {
            label.font(face::MONO)
        } else {
            label
        });
        line = line.push(iced::widget::space::horizontal());
        if let Some(word) = r.trailing {
            line = line.push(kit::muted(word));
        }
        if let Some(keys) = r.keys {
            line = line.push(kit::data_chip(keys));
        }
        body = body.push(
            kit::row_button(line, selected)
                .on_press(Message::Chrome(ChromeMsg::Palette(PaletteMsg::Pick(i))))
                .padding([0.0, space::XS]),
        );
    }

    let mut col = column![modal_header(icon, title), input].spacing(space::SM);
    if let Some(hint) = hint {
        col = col.push(kit::muted(hint));
    }
    kit::modal(
        // Embedded (#558, as the tree's since #557): iced's rail is opaque,
        // and floating it covered each row's keycap.
        col.push(
            iced::widget::scrollable(body)
                .height(Length::Fixed(300.0))
                .spacing(space::XS),
        )
        .push(keycaps()),
        560.0,
        None,
    )
}

/// The shared header of the overlays this module draws whole (#558): the
/// overlay's glyph and name, and how to leave it, on one line.
fn modal_header<'a>(icon: kit::Icon, title: &'static str) -> Element<'a, Message> {
    use crate::shortcuts::{Chord, chord_keys};
    row![
        kit::icon(icon),
        kit::section(title),
        iced::widget::space::horizontal(),
        kit::data_chip(chord_keys(Chord::Escape)),
        kit::muted("closes"),
    ]
    .spacing(space::SM)
    .align_y(iced::Alignment::Center)
    .into()
}

/// The keys that drive a list overlay, as keycaps (#558) — spelled by the
/// shortcut map's own chord rows, so the footer cannot restate a key the
/// app does not answer.
fn keycaps<'a>() -> Element<'a, Message> {
    use crate::shortcuts::{Chord, chord_keys};
    let pair = |keys: &[Chord], what: &'static str| {
        let mut r = iced::widget::Row::new()
            .spacing(space::XS)
            .align_y(iced::Alignment::Center);
        for k in keys {
            r = r.push(kit::data_chip(chord_keys(*k)));
        }
        r.push(kit::muted(what))
    };
    row![
        pair(&[Chord::Up, Chord::Down], "move"),
        pair(&[Chord::Enter], "run"),
    ]
    .spacing(space::MD)
    .align_y(iced::Alignment::Center)
    .into()
}

/// The Connect overlay (#185): [`crate::view::contexts::pane`], floated.
///
/// The pane itself is unchanged — its form state still lives in the
/// workbench and its messages still route through `PaneMsg::Context` — only
/// the surface moved: contexts and endpoints are about how the window
/// reaches a bus, not about the subject, so they stopped being a tab a lost
/// user had to find. The pane brings its own header, so how to leave sits
/// below it (#558) — a caption above a header read as a header of its own.
fn connect<'a>(
    form: &'a crate::view::contexts::ContextForm,
    unreachable: bool,
) -> Element<'a, Message> {
    kit::modal(
        column![
            crate::view::contexts::pane(form, unreachable),
            kit::muted("session setup — Esc closes"),
        ]
        .spacing(space::SM),
        640.0,
        Some(560.0),
    )
}

/// The Selectors overlay (#187): [`crate::view::scope_editor::pane`],
/// floated the same way Connect is — the scope is about what the window
/// watches, not about the subject, so it is a session-scoped modal too.
fn selectors(scope: crate::view::scope_editor::ScopeEditorData<'_>) -> Element<'_, Message> {
    floated(crate::view::scope_editor::pane(scope))
}

/// The shared frame of the Selectors (#187) and Settings (#188) modals:
/// Connect's shape — fixed, bordered, on the surface color — with the modal's
/// content scrolling inside it, and how to leave below it (#558).
fn floated(content: Element<'_, Message>) -> Element<'_, Message> {
    kit::modal(
        column![
            iced::widget::scrollable(content)
                .height(Length::Fill)
                .spacing(space::XS),
            kit::muted("Esc closes"),
        ]
        .spacing(space::SM),
        640.0,
        Some(560.0),
    )
}

/// The `?` overlay — rendered from [`crate::shortcuts::map`], which is also
/// what dispatches. There is no second list to keep in step: the trailing
/// lines that used to restate `Ctrl P`, `Ctrl K` and `?` by hand are gone
/// (#190) — the table holds the modifier-less bindings now, Esc included,
/// so the overlay renders exactly the table and nothing else. Since #558 it
/// is sectioned by the table's own [`crate::shortcuts::Group`], and each
/// binding's keys are one keycap — one text, so `find(binding.keys)` still
/// sees each.
fn help<'a>() -> Element<'a, Message> {
    let mut body = Column::new().spacing(space::XS);
    let mut previous = None;
    for b in crate::shortcuts::map() {
        if previous != Some(b.group) {
            let air = if previous.is_none() { 0.0 } else { space::SM };
            body = body.push(
                iced::widget::container(kit::eyebrow(b.group.label()))
                    .padding(Padding::ZERO.top(air)),
            );
            previous = Some(b.group);
        }
        body = body.push(
            row![
                iced::widget::container(kit::data_chip(b.keys)).width(Length::Fixed(120.0)),
                kit::caption(b.what),
            ]
            .spacing(space::SM)
            .align_y(iced::Alignment::Center),
        );
    }

    kit::modal(
        column![
            modal_header(kit::Icon::Keyboard, "Shortcuts"),
            iced::widget::scrollable(body)
                .height(Length::Fixed(560.0))
                .spacing(space::XS),
        ]
        .spacing(space::SM),
        520.0,
        None,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scope-editor data set over defaults, for the tests that need the
    /// overlay signature satisfied and nothing more.
    fn scope_data(
        form: &crate::view::scope_editor::ScopeForm,
    ) -> crate::view::scope_editor::ScopeEditorData<'_> {
        crate::view::scope_editor::ScopeEditorData {
            scope: crate::scope::ScopePreset::Everything,
            base: "",
            selectors: &[],
            form,
        }
    }

    /// A Settings data set over one settings value and one form, likewise.
    fn settings_data<'a>(
        settings: &'a crate::config::Settings,
        form: &'a crate::view::settings::SettingsForm,
    ) -> crate::view::settings::SettingsData<'a> {
        crate::view::settings::SettingsData {
            settings,
            form,
            theme: "dark",
            zoom: 1.0,
            echo: (0, 0, 0),
            history: None,
            keys: (0, 0),
        }
    }

    fn test_settings() -> crate::config::Settings {
        crate::config::Settings {
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

    /// #110: only the jump-to overlay may read the key iterator — a closed
    /// palette, the command list, the help sheet and the modals must cost the
    /// cache nothing. The iterator panics on first pull to prove it.
    #[test]
    fn only_the_keys_overlay_reads_the_keys() {
        for open in [
            None,
            Some(Overlay::Commands),
            Some(Overlay::Help),
            Some(Overlay::Connect),
            Some(Overlay::Selectors),
            Some(Overlay::Settings),
        ] {
            let mut state = PaletteState::default();
            if let Some(o) = open {
                state.open(o);
            }
            let poisoned = std::iter::from_fn(|| -> Option<&str> {
                panic!("this overlay must not read the keys")
            });
            let form = crate::view::contexts::ContextForm::default();
            let scope_form = crate::view::scope_editor::ScopeForm::default();
            let settings = test_settings();
            let settings_form = crate::view::settings::SettingsForm::default();
            let _ = overlay(
                &state,
                &form,
                false,
                scope_data(&scope_form),
                settings_data(&settings, &settings_form),
                poisoned,
            );
        }
    }

    /// The property the whole design rests on: a palette entry is the *same*
    /// message the UI path sends, so the two cannot drift apart.
    #[test]
    fn every_action_emits_the_message_the_ui_path_emits() {
        let contexts = vec!["lab".to_string()];
        let items = actions(&contexts);
        let find = |label: &str| {
            items
                .iter()
                .find(|a| a.label == label)
                .map(|a| format!("{:?}", a.message))
                .unwrap_or_else(|| panic!("no palette entry {label:?}"))
        };
        // Panes: the same message the tab strip sends.
        for pane in RightPane::ALL {
            assert_eq!(
                find(&format!("go to {} pane", pane.label())),
                format!("{:?}", Message::Workspace(WorkspaceMsg::PaneSelected(pane)))
            );
        }
        // Scope: the same message the location bar's picker sends — every
        // preset, custom included (#187): an option list that excludes a
        // scope the window can be in is the defect that issue names.
        for scope in crate::scope::ScopePreset::ALL {
            assert_eq!(
                find(&format!("scope: {}", scope.short())),
                format!(
                    "{:?}",
                    Message::Deployment(DeploymentMsg::ScopeSelected(scope))
                )
            );
        }
        // The Settings overlay: the same message the location bar's settings
        // chip and Ctrl+, send (#188).
        assert_eq!(
            find("settings — bounds and launch knobs"),
            format!(
                "{:?}",
                Message::Chrome(ChromeMsg::Palette(PaletteMsg::Open(Overlay::Settings)))
            )
        );
        // The key-expression editor: the same message the location bar's
        // selectors chip sends (#187).
        assert_eq!(
            find("edit scope selectors"),
            format!(
                "{:?}",
                Message::Chrome(ChromeMsg::Palette(PaletteMsg::Open(Overlay::Selectors)))
            )
        );
        // Context: the same message the connect pane's picker sends.
        assert_eq!(
            find("context: lab"),
            format!(
                "{:?}",
                Message::Pane(PaneMsg::Context(
                    crate::view::contexts::ContextMsg::Selected("lab".into())
                ))
            )
        );
        // Echo actions: the same messages that pane's buttons send.
        assert_eq!(
            find("clear echo"),
            format!(
                "{:?}",
                Message::Pane(PaneMsg::Echo(crate::view::echo::EchoMsg::Clear))
            )
        );
        assert_eq!(
            find("reconnect"),
            format!("{:?}", Message::Deployment(DeploymentMsg::Reconnect))
        );
        // Layouts: the same messages Alt+1/2/3 send (#180).
        for preset in crate::prefs::LayoutPreset::ALL {
            assert_eq!(
                find(&format!("layout: {}", preset.label())),
                format!(
                    "{:?}",
                    Message::Workspace(WorkspaceMsg::LayoutPreset(preset))
                )
            );
        }
        // Docks: the same message the dock strip and the title-bar × send
        // (#180) — and the keyboard route back to a closed dock.
        for role in crate::prefs::DockRole::ALL {
            assert_eq!(
                find(&format!("toggle {} dock", role.label())),
                format!("{:?}", Message::Workspace(WorkspaceMsg::DockToggled(role)))
            );
        }
    }

    /// The pane list is generated, so a pane added anywhere shows up here.
    #[test]
    fn the_palette_covers_every_pane_without_being_told() {
        let items = actions(&[]);
        for pane in RightPane::ALL {
            assert!(
                items
                    .iter()
                    .any(|a| a.label == format!("go to {} pane", pane.label())),
                "{} missing from the palette",
                pane.label()
            );
        }
    }

    #[test]
    fn fuzzy_matches_subsequences_and_ranks_tighter_matches_first() {
        assert!(fuzzy("go to inspector pane", "gti").is_some());
        assert!(fuzzy("go to inspector pane", "inspector").is_some());
        assert!(fuzzy("go to inspector pane", "zzz").is_none());
        // An empty query matches everything.
        assert_eq!(fuzzy("anything", ""), Some(0));
        // Contiguous beats scattered.
        let tight = fuzzy("inspector", "ins").unwrap();
        let loose = fuzzy("go to inspector pane", "ins").unwrap();
        assert!(tight < loose, "tight {tight} loose {loose}");
    }

    /// Ranking is bounded and ordered — a palette that draws every key is the
    /// same bug as a tree that does.
    #[test]
    fn ranking_is_bounded() {
        let keys: Vec<String> = (0..500).map(|i| format!("v1/h-aaa/state/p/k{i}")).collect();
        let order = rank(&keys, "state", |k| k.as_str());
        assert_eq!(order.len(), MAX_ROWS);
        // …and an unfiltered palette still reaches past the panes and scopes,
        // so a fresh overlay does not read as "this is all there is".
        let items = actions(&["lab".to_string()]);
        let unfiltered = command_order(&items, "");
        assert_eq!(unfiltered.len(), MAX_ROWS);
        assert!(
            unfiltered
                .iter()
                .any(|i| items[*i].label.starts_with("context:")),
            "the first screenful must show more than panes and scopes"
        );
    }

    /// #558: the unfiltered palette is sectioned, session and diagnoses
    /// first — so the verbs a lost user opens it for are on the first
    /// screen, not wherever their label length put them.
    #[test]
    fn an_unfiltered_palette_leads_with_the_session_and_the_diagnoses() {
        let items = actions(&["lab".to_string()]);
        let order = command_order(&items, "");
        let groups: Vec<ActionGroup> = order.iter().map(|i| items[*i].group).collect();
        assert!(
            groups.windows(2).all(|w| w[0] <= w[1]),
            "one run per section"
        );
        assert_eq!(groups[0], ActionGroup::Session);
        let first_screen: Vec<&str> = order[..10]
            .iter()
            .map(|i| items[*i].label.as_str())
            .collect();
        for verb in ["reconnect", "run doctor", "why is this key silent?"] {
            assert!(
                first_screen.contains(&verb),
                "{verb} is past the first screen"
            );
        }
        // A query ranks instead: the best match first, sections or not.
        let ranked = command_order(&items, "doctor");
        assert_eq!(items[ranked[0]].label, "run doctor");
    }

    /// One overlay at a time, by construction — and closing forgets the query
    /// so the next open starts clean.
    #[test]
    fn the_overlay_is_one_at_a_time_and_resets() {
        let mut s = PaletteState::default();
        assert!(!s.is_open());
        s.open(Overlay::Commands);
        s.query = "doc".into();
        s.cursor = 3;
        s.open(Overlay::Keys);
        assert_eq!(s.overlay, Overlay::Keys);
        assert_eq!(s.query, "", "a fresh overlay starts with a fresh query");
        assert_eq!(s.cursor, 0);
        s.close();
        assert!(!s.is_open());
    }
}

//! The command palette and jump-to-key overlay (issue #75).
//!
//! By the end of this epic zengui has seven panes and dozens of actions;
//! discoverability by toolbar alone stops scaling well before that. zensight
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
//! keys *actually observed*, selecting one emits the ordinary
//! [`Message::SelectKey`]. It offers nothing it has not seen, which keeps the
//! overlay from inventing a keyspace (O4 — a suggestion is not an observation,
//! and these are only ever the latter).

use iced::widget::{Column, column, container, row, text, text_input};
use iced::{Element, Length};

use crate::message::{Message, PrefsMsg, RightPane};
use crate::view::kit;
use crate::view::theme::colors;
use crate::view::tokens::{font, space};

/// How many rows the overlay renders. A palette that draws a 50k-key list is
/// the same bug as a tree that does.
///
/// Twenty rather than a tighter bound because the unfiltered list starts with
/// the panes and the scopes: at twelve, a fresh palette showed *only* those
/// and nothing else — which reads as "this is all there is" rather than "type
/// to see more".
const MAX_ROWS: usize = 20;

/// Which overlay is open, if any. One field rather than three booleans, so
/// "two overlays at once" has no representation.
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
}

/// Every command the palette offers.
///
/// Built from the same enums the UI is built from (`RightPane::ALL`,
/// `ScopePreset`), so a pane or scope added elsewhere shows up here without
/// anyone remembering to add it.
pub fn actions(contexts: &[String]) -> Vec<Action> {
    let mut out: Vec<Action> = RightPane::ALL
        .into_iter()
        .map(|p| Action {
            label: format!("go to {} pane", p.label()),
            message: Message::PaneSelected(p),
        })
        .collect();

    for scope in [
        crate::scope::ScopePreset::Everything,
        crate::scope::ScopePreset::Deployment,
        crate::scope::ScopePreset::Telemetry,
        crate::scope::ScopePreset::State,
        crate::scope::ScopePreset::Events,
    ] {
        out.push(Action {
            label: format!("scope: {}", scope.short()),
            message: Message::ScopeSelected(scope),
        });
    }

    for name in contexts {
        out.push(Action {
            label: format!("context: {name}"),
            message: Message::Context(crate::view::contexts::ContextMsg::Selected(name.clone())),
        });
    }

    out.extend([
        Action {
            label: "observe scope (start/stop)".into(),
            message: Message::ScopeWatchToggled,
        },
        Action {
            label: "run doctor".into(),
            message: Message::Doctor(crate::view::doctor::DoctorMsg::Run),
        },
        Action {
            label: "reconnect".into(),
            message: Message::Reconnect,
        },
        Action {
            label: "toggle theme".into(),
            message: Message::Prefs(PrefsMsg::ThemeToggled),
        },
        Action {
            label: "zoom in".into(),
            message: Message::Prefs(PrefsMsg::ZoomIn),
        },
        Action {
            label: "zoom out".into(),
            message: Message::Prefs(PrefsMsg::ZoomOut),
        },
        Action {
            label: "reset zoom".into(),
            message: Message::Prefs(PrefsMsg::ZoomReset),
        },
        Action {
            label: "clear echo".into(),
            message: Message::Echo(crate::view::echo::EchoMsg::Clear),
        },
        Action {
            label: "export echo as ndjson".into(),
            message: Message::Echo(crate::view::echo::EchoMsg::Export),
        },
        Action {
            label: "pause/follow echo".into(),
            message: Message::Echo(crate::view::echo::EchoMsg::FollowToggled),
        },
    ]);
    out
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
    contexts: &[String],
    keys: impl Iterator<Item = &'a str>,
) -> Option<Element<'a, Message>> {
    match state.overlay {
        Overlay::None => None,
        Overlay::Help => Some(help()),
        Overlay::Commands => {
            let items = actions(contexts);
            let order = rank(&items, &state.query, |a| a.label.as_str());
            Some(list(
                state,
                "Command palette",
                "type to filter — Enter runs, Esc closes",
                order
                    .iter()
                    .map(|i| items[*i].label.clone())
                    .collect::<Vec<_>>(),
            ))
        }
        Overlay::Keys => {
            // Fat pointers only — no key bytes are copied to rank (#110).
            let keys: Vec<&str> = keys.collect();
            let order = rank(&keys, &state.query, |k| *k);
            Some(list(
                state,
                "Jump to key",
                "fuzzy over keys observed so far — nothing here is a guess (O4)",
                order
                    .iter()
                    .map(|i| keys[*i].to_string())
                    .collect::<Vec<_>>(),
            ))
        }
    }
}

fn list<'a>(
    state: &'a PaletteState,
    title: &'a str,
    hint: &'a str,
    rows: Vec<String>,
) -> Element<'a, Message> {
    let input = text_input("…", &state.query)
        .on_input(|q| Message::Palette(PaletteMsg::QueryChanged(q)))
        .on_submit(Message::Palette(PaletteMsg::Activate))
        .size(font::BODY);

    let mut body = Column::new().spacing(1);
    if rows.is_empty() {
        body = body.push(kit::muted("nothing matches"));
    }
    for (i, label) in rows.iter().enumerate() {
        let selected = i == state.cursor;
        body = body.push(
            iced::widget::button(
                row![
                    text(if selected { "›" } else { " " }).size(font::CAPTION),
                    text(label.clone())
                        .size(font::CAPTION)
                        .font(iced::Font::MONOSPACE),
                ]
                .spacing(space::SM),
            )
            .on_press(Message::Palette(PaletteMsg::Pick(i)))
            .style(if selected {
                iced::widget::button::secondary
            } else {
                iced::widget::button::text
            })
            .width(Length::Fill)
            .padding(2),
        );
    }

    container(
        column![
            kit::section_header(title, None),
            input,
            kit::muted(hint),
            iced::widget::scrollable(body).height(Length::Fixed(260.0)),
        ]
        .spacing(space::SM),
    )
    .padding(space::MD)
    .width(Length::Fixed(560.0))
    .style(|theme: &iced::Theme| container::Style {
        background: Some(colors(theme).surface().into()),
        border: iced::Border {
            color: colors(theme).border(),
            width: 1.0,
            radius: 4.0.into(),
        },
        ..container::Style::default()
    })
    .into()
}

/// The `?` overlay — rendered from [`crate::shortcuts::map`], which is also
/// what dispatches. There is no second list to keep in step.
fn help<'a>() -> Element<'a, Message> {
    let mut body = Column::new().spacing(2);
    for b in crate::shortcuts::map() {
        body = body.push(
            row![
                text(b.keys)
                    .size(font::CAPTION)
                    .font(iced::Font::MONOSPACE)
                    .width(Length::Fixed(90.0)),
                kit::muted(b.what),
            ]
            .spacing(space::SM),
        );
    }
    body = body.push(kit::muted("Ctrl P  command palette"));
    body = body.push(kit::muted("Ctrl K  jump to key"));
    body = body.push(kit::muted("?       this list · Esc closes"));

    container(column![kit::section_header("Shortcuts", None), body].spacing(space::SM))
        .padding(space::MD)
        .width(Length::Fixed(480.0))
        .style(|theme: &iced::Theme| container::Style {
            background: Some(colors(theme).surface().into()),
            border: iced::Border {
                color: colors(theme).border(),
                width: 1.0,
                radius: 4.0.into(),
            },
            ..container::Style::default()
        })
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// #110: only the jump-to overlay may read the key iterator — a closed
    /// palette, the command list and the help sheet must cost the cache
    /// nothing. The iterator panics on first pull to prove it.
    #[test]
    fn only_the_keys_overlay_reads_the_keys() {
        for open in [None, Some(Overlay::Commands), Some(Overlay::Help)] {
            let mut state = PaletteState::default();
            if let Some(o) = open {
                state.open(o);
            }
            let poisoned = std::iter::from_fn(|| -> Option<&str> {
                panic!("this overlay must not read the keys")
            });
            let _ = overlay(&state, &[], poisoned);
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
                format!("{:?}", Message::PaneSelected(pane))
            );
        }
        // Scope: the same message the toolbar picker sends.
        assert_eq!(
            find(&format!(
                "scope: {}",
                crate::scope::ScopePreset::Everything.short()
            )),
            format!(
                "{:?}",
                Message::ScopeSelected(crate::scope::ScopePreset::Everything)
            )
        );
        // Context: the same message the connect pane's picker sends.
        assert_eq!(
            find("context: lab"),
            format!(
                "{:?}",
                Message::Context(crate::view::contexts::ContextMsg::Selected("lab".into()))
            )
        );
        // Echo actions: the same messages that pane's buttons send.
        assert_eq!(
            find("clear echo"),
            format!("{:?}", Message::Echo(crate::view::echo::EchoMsg::Clear))
        );
        assert_eq!(find("reconnect"), format!("{:?}", Message::Reconnect));
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
        assert!(fuzzy("go to detail pane", "gtd").is_some());
        assert!(fuzzy("go to detail pane", "detail").is_some());
        assert!(fuzzy("go to detail pane", "zzz").is_none());
        // An empty query matches everything.
        assert_eq!(fuzzy("anything", ""), Some(0));
        // Contiguous beats scattered.
        let tight = fuzzy("detail", "det").unwrap();
        let loose = fuzzy("go to detail pane", "det").unwrap();
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
        let unfiltered = rank(&items, "", |a| a.label.as_str());
        assert!(
            unfiltered
                .iter()
                .any(|i| items[*i].label.starts_with("context:")),
            "the first screenful must show more than panes and scopes"
        );
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

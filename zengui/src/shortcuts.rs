//! The keyboard map (issues #73, #75, #190) — **one** place, so `?` cannot lie.
//!
//! A shortcut table that lives in two places (the handler and the help text)
//! drifts on the first change, and the help is the half nobody notices is
//! wrong. So the table below is the only definition: [`resolve`] and [`chord`]
//! dispatch from it and [`map`] renders it, which makes "does the overlay list
//! what the app actually does" a property a test can check rather than a
//! review habit.
//!
//! Two kinds of entry, and the split is #190's:
//!
//! * An [`Action::Emit`] binding is a modifier chord with one fixed meaning.
//!   Almost all of them emit a [`Message`] some UI path *also* emits — a
//!   shortcut is a faster way to do a thing, never a second implementation of
//!   it. The dock-focus keys are the one deliberate exception: clicking a dock
//!   speaks `DockFocused(pane)` with a runtime pane id no table can hold, so
//!   `FocusDock(role)` is that click's role-addressed form, and its handler
//!   reuses the same restore-and-focus the click path does.
//! * An [`Action::Chord`] binding is a modifier-less key — `Esc`, the arrows,
//!   `⏎`, `?`. What each *means* depends on what is open (Esc peels the
//!   palette before it clears the selection; the arrows drive an overlay only
//!   while one is), so the table names the key and `update::chrome` layers
//!   the meaning — by matching the [`Chord`] this module resolved, never the
//!   raw key. Before #190 these lived only in the handler and were
//!   undocumented by construction: the old table had no way to hold a binding
//!   without a modifier.

use iced::keyboard::{Key, Modifiers, key::Named};

use crate::message::{ChromeMsg, DeploymentMsg, Message, PrefsMsg, WorkspaceMsg};
use crate::prefs::{DockRole, LayoutPreset};
use crate::view::palette::{Overlay, PaletteMsg};

/// One binding: how it is typed, what it does, and how it dispatches.
pub struct Binding {
    /// How to type it, as a human reads it.
    pub keys: &'static str,
    /// What it does, in the imperative.
    pub what: &'static str,
    /// How the app answers it.
    pub action: Action,
}

/// How a binding dispatches (#190).
pub enum Action {
    /// A modifier chord with one fixed message — [`resolve`] dispatches it.
    /// A function so a `Message` need not be `const`.
    Emit(fn() -> Message),
    /// A modifier-less key whose meaning depends on what is open —
    /// [`chord`] names it, and `update::chrome` layers what it does.
    Chord(Chord),
}

/// The modifier-less keys the app answers (#190). [`chord`] is the only door
/// a bare key comes through, so this list *is* the dispatch — and the table
/// below advertises every variant, pinned by the tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Chord {
    /// The "get out of here" key, layered (#75): palette > overlays >
    /// selection, one layer per press.
    Escape,
    /// Overlay cursor up — a pane's own key while nothing is open.
    Up,
    /// Overlay cursor down — likewise.
    Down,
    /// Run the highlighted overlay row.
    Enter,
    /// `?` — this map. The one printable key bound bare, because it is the
    /// key every tool in this family answers with its help; it still only
    /// fires when no text input consumed the keystroke.
    Help,
}

impl Chord {
    pub const ALL: [Chord; 5] = [
        Chord::Escape,
        Chord::Up,
        Chord::Down,
        Chord::Enter,
        Chord::Help,
    ];
}

/// The whole map, in the order the help overlay shows it.
pub fn map() -> Vec<Binding> {
    let mut out = vec![
        Binding {
            keys: "Ctrl +",
            what: "zoom in",
            action: Action::Emit(|| Message::Chrome(ChromeMsg::Prefs(PrefsMsg::ZoomIn))),
        },
        Binding {
            keys: "Ctrl -",
            what: "zoom out",
            action: Action::Emit(|| Message::Chrome(ChromeMsg::Prefs(PrefsMsg::ZoomOut))),
        },
        Binding {
            keys: "Ctrl 0",
            what: "reset zoom",
            action: Action::Emit(|| Message::Chrome(ChromeMsg::Prefs(PrefsMsg::ZoomReset))),
        },
        Binding {
            keys: "Ctrl T",
            what: "toggle theme",
            action: Action::Emit(|| Message::Chrome(ChromeMsg::Prefs(PrefsMsg::ThemeToggled))),
        },
        Binding {
            keys: "Ctrl Shift D",
            what: "toggle density (comfortable/compact)",
            action: Action::Emit(|| Message::Chrome(ChromeMsg::Prefs(PrefsMsg::DensityToggled))),
        },
        Binding {
            keys: "Ctrl R",
            what: "reconnect",
            action: Action::Emit(|| Message::Deployment(DeploymentMsg::Reconnect)),
        },
        Binding {
            keys: "Ctrl P",
            what: "command palette",
            action: Action::Emit(|| {
                Message::Chrome(ChromeMsg::Palette(PaletteMsg::Open(Overlay::Commands)))
            }),
        },
        Binding {
            keys: "Ctrl K",
            what: "jump to an observed key",
            action: Action::Emit(|| {
                Message::Chrome(ChromeMsg::Palette(PaletteMsg::Open(Overlay::Keys)))
            }),
        },
        // Connect finally has a keyboard route (#185): it used to be the one
        // pane without one, and it is the surface a lost user most needs.
        Binding {
            keys: "Ctrl Shift C",
            what: "connect — contexts and endpoints",
            action: Action::Emit(|| {
                Message::Chrome(ChromeMsg::Palette(PaletteMsg::Open(Overlay::Connect)))
            }),
        },
        // The launch knobs, surfaced (#188) — the family's usual chord for a
        // settings surface.
        Binding {
            keys: "Ctrl ,",
            what: "settings — bounds and launch knobs",
            action: Action::Emit(|| {
                Message::Chrome(ChromeMsg::Palette(PaletteMsg::Open(Overlay::Settings)))
            }),
        },
    ];
    // The saved layouts, in preset order — so the numbers on screen and the
    // numbers under the fingers are the same list.
    //
    // These digits pointed at panes until #180. Five panes had five digits;
    // the panes are docks of a grid now, and "which pane shows" stopped
    // being a thing a digit can mean. What a digit means instead is a whole
    // layout: Explore, Watch, Diagnose.
    for (i, preset) in LayoutPreset::ALL.into_iter().enumerate() {
        out.push(Binding {
            keys: LAYOUT_KEYS[i],
            what: LAYOUT_WHAT[i],
            action: Action::Emit(LAYOUT_MESSAGES[i]),
        });
        let _ = preset;
    }
    // The other half of the digit remap (#190): a digit picks a whole
    // layout, a letter puts the keyboard in one dock — Locator, Inspector,
    // Activity, by initial. The workbench deliberately has no letter: its
    // tools are destinations the palette and `PaneSelected` already reach,
    // not a region the keyboard camps in.
    for (i, role) in DOCK_ROLES.into_iter().enumerate() {
        out.push(Binding {
            keys: DOCK_KEYS[i],
            what: DOCK_WHAT[i],
            action: Action::Emit(DOCK_MESSAGES[i]),
        });
        let _ = role;
    }
    // The modifier-less keys (#190). They lived only in `update::chrome`
    // and were undocumented by construction — the old table could not hold
    // a binding without a modifier. `Action::Chord` is what admits them;
    // the layering each row describes still happens in `update::chrome`,
    // against the `Chord` this table names.
    out.extend([
        Binding {
            keys: "Esc",
            what: "close the overlay, else clear the selection",
            action: Action::Chord(Chord::Escape),
        },
        Binding {
            keys: "↑",
            what: "overlay: cursor up",
            action: Action::Chord(Chord::Up),
        },
        Binding {
            keys: "↓",
            what: "overlay: cursor down",
            action: Action::Chord(Chord::Down),
        },
        Binding {
            keys: "⏎",
            what: "overlay: run the highlighted row",
            action: Action::Chord(Chord::Enter),
        },
        Binding {
            keys: "?",
            what: "this map",
            action: Action::Chord(Chord::Help),
        },
    ]);
    out
}

/// Alt+1/2/3, one per saved layout. Parallel arrays rather than a formatted
/// string because `Binding` holds `&'static str` — and the length assertion
/// below is what keeps them in step with `LayoutPreset::ALL`.
const LAYOUT_KEYS: [&str; 3] = ["Alt 1", "Alt 2", "Alt 3"];
const LAYOUT_WHAT: [&str; 3] = ["layout: explore", "layout: watch", "layout: diagnose"];
const LAYOUT_MESSAGES: [fn() -> Message; 3] = [
    || Message::Workspace(WorkspaceMsg::LayoutPreset(LayoutPreset::Explore)),
    || Message::Workspace(WorkspaceMsg::LayoutPreset(LayoutPreset::Watch)),
    || Message::Workspace(WorkspaceMsg::LayoutPreset(LayoutPreset::Diagnose)),
];

/// Alt+L/I/A, one per focusable dock (#190) — same parallel-array shape as
/// the layouts, same test keeping the arrays in step.
const DOCK_ROLES: [DockRole; 3] = [DockRole::Locator, DockRole::Inspector, DockRole::Activity];
const DOCK_KEYS: [&str; 3] = ["Alt L", "Alt I", "Alt A"];
const DOCK_WHAT: [&str; 3] = [
    "focus the locator dock",
    "focus the inspector dock",
    "focus the activity dock",
];
const DOCK_MESSAGES: [fn() -> Message; 3] = [
    || Message::Workspace(WorkspaceMsg::FocusDock(DockRole::Locator)),
    || Message::Workspace(WorkspaceMsg::FocusDock(DockRole::Inspector)),
    || Message::Workspace(WorkspaceMsg::FocusDock(DockRole::Activity)),
];

/// A modified key press → the message it should send, if any.
///
/// Deliberately *not* table-driven at runtime: a `Key` match is the only
/// reliable way to accept the several spellings one physical key arrives as
/// (`+` is `Shift =` on most layouts, and the numpad sends its own). The table
/// above stays the source of truth for the *set* of bindings, and the test
/// below pins the two to each other.
pub fn resolve(key: &Key, mods: Modifiers) -> Option<Message> {
    let ctrl = mods.command() || mods.control();
    if ctrl && let Key::Character(c) = key {
        return match c.as_str() {
            // Shifted first: Ctrl+Shift+C opens Connect (#185). A plain
            // Ctrl+C stays unbound — it is copy in every text box.
            "c" | "C" if mods.shift() => Some(Message::Chrome(ChromeMsg::Palette(
                PaletteMsg::Open(Overlay::Connect),
            ))),
            // Ctrl+Shift+D toggles density (#192). Plain Ctrl+D stays
            // unbound — the shifted chord is deliberate for a key that
            // reflows the whole window.
            "d" | "D" if mods.shift() => {
                Some(Message::Chrome(ChromeMsg::Prefs(PrefsMsg::DensityToggled)))
            }
            // `+` normally needs Shift on `=`; accept both spellings rather
            // than making the user find the numpad.
            "+" | "=" => Some(Message::Chrome(ChromeMsg::Prefs(PrefsMsg::ZoomIn))),
            "-" | "_" => Some(Message::Chrome(ChromeMsg::Prefs(PrefsMsg::ZoomOut))),
            "0" => Some(Message::Chrome(ChromeMsg::Prefs(PrefsMsg::ZoomReset))),
            "t" | "T" => Some(Message::Chrome(ChromeMsg::Prefs(PrefsMsg::ThemeToggled))),
            "r" | "R" => Some(Message::Deployment(DeploymentMsg::Reconnect)),
            "p" | "P" => Some(Message::Chrome(ChromeMsg::Palette(PaletteMsg::Open(
                Overlay::Commands,
            )))),
            "k" | "K" => Some(Message::Chrome(ChromeMsg::Palette(PaletteMsg::Open(
                Overlay::Keys,
            )))),
            "," => Some(Message::Chrome(ChromeMsg::Palette(PaletteMsg::Open(
                Overlay::Settings,
            )))),
            _ => None,
        };
    }
    if mods.alt()
        && let Key::Character(c) = key
    {
        // Alt+L/I/A put the keyboard in a dock (#190) — the letters are the
        // docks' initials, and the set is `DOCK_ROLES`, not `DockRole::ALL`:
        // the workbench has no letter on purpose (see `map`).
        match c.as_str() {
            "l" | "L" => {
                return Some(Message::Workspace(WorkspaceMsg::FocusDock(
                    DockRole::Locator,
                )));
            }
            "i" | "I" => {
                return Some(Message::Workspace(WorkspaceMsg::FocusDock(
                    DockRole::Inspector,
                )));
            }
            "a" | "A" => {
                return Some(Message::Workspace(WorkspaceMsg::FocusDock(
                    DockRole::Activity,
                )));
            }
            _ => {}
        }
        // Alt+1/2/3 are the saved layouts (#180). The digits pointed at
        // panes until the panes became docks of one grid; the bound is
        // `LayoutPreset::ALL`, so Alt+4 and up do nothing rather than
        // quietly becoming something else — the same discipline the pane
        // digits kept when their list shrank.
        if let Ok(digit) = c.parse::<usize>()
            && (1..=LayoutPreset::ALL.len()).contains(&digit)
        {
            return Some(Message::Workspace(WorkspaceMsg::LayoutPreset(
                LayoutPreset::ALL[digit - 1],
            )));
        }
    }
    None
}

/// A key press → the modifier-less chord it is, if the table names it (#190).
///
/// The bare keys' twin of [`resolve`], and like it a `Key` match rather than
/// a runtime table walk — but this *is* the dispatch: `update::chrome`
/// matches on the returned [`Chord`], never on the raw key, so a bare key
/// the app answers cannot exist outside this module. Modifiers are ignored
/// on purpose: `?` arrives shifted on most layouts, and Esc with a stray
/// modifier still means out.
pub fn chord(key: &Key) -> Option<Chord> {
    match key {
        Key::Named(Named::Escape) => Some(Chord::Escape),
        Key::Named(Named::ArrowUp) => Some(Chord::Up),
        Key::Named(Named::ArrowDown) => Some(Chord::Down),
        Key::Named(Named::Enter) => Some(Chord::Enter),
        Key::Character(c) if c.as_str() == "?" => Some(Chord::Help),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctrl() -> Modifiers {
        Modifiers::CTRL
    }

    fn press(c: &str, mods: Modifiers) -> Option<Message> {
        resolve(&Key::Character(c.into()), mods)
    }

    /// Every binding the overlay advertises is one the app actually
    /// dispatches — a combo through `resolve`, a bare key through `chord`.
    /// Without this, `?` is documentation that drifts.
    #[test]
    fn every_advertised_binding_is_dispatchable() {
        for b in map() {
            match b.action {
                Action::Emit(message) => {
                    let (mods, ch) = spelling(b.keys);
                    let got = press(&ch, mods);
                    assert_eq!(
                        format!("{got:?}"),
                        format!("{:?}", Some(message())),
                        "binding {:?} ({}) does not dispatch",
                        b.keys,
                        b.what
                    );
                }
                Action::Chord(want) => {
                    assert_eq!(
                        chord(&bare(b.keys)),
                        Some(want),
                        "chord {:?} ({}) does not dispatch",
                        b.keys,
                        b.what
                    );
                }
            }
        }
    }

    /// The other direction (#190): every bare key the app answers has a row
    /// in the table. `chord` is the only door a modifier-less key comes
    /// through, so covering `Chord::ALL` covers the dispatch — this is what
    /// ends Esc and the arrows living only in `update` and undocumented.
    #[test]
    fn every_chord_the_app_answers_is_advertised() {
        for c in Chord::ALL {
            assert!(
                map()
                    .iter()
                    .any(|b| matches!(b.action, Action::Chord(x) if x == c)),
                "{c:?} dispatches but the overlay does not list it"
            );
        }
    }

    /// Parse a combo spelling back into the press it describes — which is
    /// what makes the dispatch test a real check rather than a restatement.
    fn spelling(keys: &str) -> (Modifiers, String) {
        let mut words: Vec<&str> = keys.split(' ').collect();
        let ch = words.pop().expect("every binding names a key");
        assert!(!words.is_empty(), "every combo names its modifiers");
        let mut mods = Modifiers::empty();
        for word in words {
            mods |= match word {
                "Ctrl" => Modifiers::CTRL,
                "Alt" => Modifiers::ALT,
                "Shift" => Modifiers::SHIFT,
                other => panic!("unknown modifier {other:?}"),
            };
        }
        (mods, ch.to_lowercase())
    }

    /// Parse a chord spelling — one bare key — into the `Key` it describes:
    /// the modifier-less half of `spelling`, and what stops the chord rows
    /// from being decoration. `Esc` in the overlay must be the `Escape` the
    /// app answers.
    fn bare(keys: &str) -> Key {
        assert!(!keys.contains(' '), "a chord is one bare key: {keys:?}");
        match keys {
            "Esc" => Key::Named(Named::Escape),
            "↑" => Key::Named(Named::ArrowUp),
            "↓" => Key::Named(Named::ArrowDown),
            "⏎" => Key::Named(Named::Enter),
            ch => Key::Character(ch.into()),
        }
    }

    /// The layout list is generated from `LayoutPreset::ALL`, so a new preset
    /// must come with a key rather than silently falling off the end.
    #[test]
    fn the_layout_bindings_cover_every_preset() {
        assert_eq!(LAYOUT_KEYS.len(), LayoutPreset::ALL.len());
        assert_eq!(LAYOUT_WHAT.len(), LAYOUT_KEYS.len());
        assert_eq!(LAYOUT_MESSAGES.len(), LAYOUT_KEYS.len());
        for (i, preset) in LayoutPreset::ALL.into_iter().enumerate() {
            assert_eq!(
                format!("{:?}", LAYOUT_MESSAGES[i]()),
                format!(
                    "{:?}",
                    Message::Workspace(WorkspaceMsg::LayoutPreset(preset))
                )
            );
            assert!(
                LAYOUT_WHAT[i].ends_with(preset.label()),
                "{} must name its preset",
                LAYOUT_WHAT[i]
            );
        }
    }

    /// The dock-focus arrays stay in step the same way — and each letter is
    /// its dock's initial, which is the whole mnemonic.
    #[test]
    fn the_dock_bindings_name_their_docks_by_initial() {
        assert_eq!(DOCK_KEYS.len(), DOCK_ROLES.len());
        assert_eq!(DOCK_WHAT.len(), DOCK_ROLES.len());
        assert_eq!(DOCK_MESSAGES.len(), DOCK_ROLES.len());
        for (i, role) in DOCK_ROLES.into_iter().enumerate() {
            assert_eq!(
                format!("{:?}", DOCK_MESSAGES[i]()),
                format!("{:?}", Message::Workspace(WorkspaceMsg::FocusDock(role)))
            );
            assert!(
                DOCK_WHAT[i].contains(role.label()),
                "{} must name its dock",
                DOCK_WHAT[i]
            );
            let initial = role.label().chars().next().unwrap().to_uppercase();
            assert_eq!(
                DOCK_KEYS[i],
                format!("Alt {initial}"),
                "{}'s key is not its initial",
                role.label()
            );
        }
    }

    /// Alt+L/I/A focus the three docks a keyboard reader lives in (#190),
    /// in either case — and the workbench stays letterless: its tools are
    /// destinations `PaneSelected` already reaches.
    #[test]
    fn alt_letters_focus_the_docks_and_the_workbench_has_none() {
        for (c, role) in [
            ("l", DockRole::Locator),
            ("i", DockRole::Inspector),
            ("a", DockRole::Activity),
        ] {
            for spelled in [c.to_string(), c.to_uppercase()] {
                assert_eq!(
                    format!("{:?}", press(&spelled, Modifiers::ALT)),
                    format!(
                        "{:?}",
                        Some(Message::Workspace(WorkspaceMsg::FocusDock(role)))
                    ),
                    "Alt+{spelled}"
                );
            }
        }
        assert!(press("w", Modifiers::ALT).is_none());
    }

    /// The several spellings one physical key arrives as.
    #[test]
    fn zoom_accepts_the_shifted_and_unshifted_spellings() {
        for c in ["+", "="] {
            assert!(matches!(
                press(c, ctrl()),
                Some(Message::Chrome(ChromeMsg::Prefs(PrefsMsg::ZoomIn)))
            ));
        }
        for c in ["-", "_"] {
            assert!(matches!(
                press(c, ctrl()),
                Some(Message::Chrome(ChromeMsg::Prefs(PrefsMsg::ZoomOut)))
            ));
        }
    }

    /// A bare keystroke is not a shortcut: typing `t` into the tree's search
    /// box must not toggle the theme — neither as a combo nor as a chord.
    /// (`?` is the deliberate exception — see `the_help_key_is_a_chord`.)
    #[test]
    fn unmodified_keys_are_never_shortcuts() {
        for c in ["t", "r", "0", "-", "1", "l", "i", "a"] {
            assert!(press(c, Modifiers::empty()).is_none(), "{c}");
            assert!(
                chord(&Key::Character(c.into())).is_none(),
                "{c} must not be a chord"
            );
        }
    }

    /// Ctrl+Shift+D toggles density (#192); plain Ctrl+D does nothing — a
    /// chord that reflows the whole window is not given to an unshifted slip.
    #[test]
    fn density_answers_the_shifted_chord_only() {
        for c in ["d", "D"] {
            assert!(matches!(
                press(c, Modifiers::CTRL | Modifiers::SHIFT),
                Some(Message::Chrome(ChromeMsg::Prefs(PrefsMsg::DensityToggled)))
            ));
            assert!(press(c, ctrl()).is_none(), "Ctrl+{c} must stay unbound");
        }
    }

    /// Ctrl+Shift+C opens the Connect overlay (#185) — Connect used to be the
    /// one pane with no keyboard route, and it is the surface a lost user
    /// most needs. Plain Ctrl+C stays unbound: it is copy in every text box.
    #[test]
    fn connect_answers_the_shifted_chord_and_leaves_copy_alone() {
        for c in ["c", "C"] {
            assert!(matches!(
                press(c, Modifiers::CTRL | Modifiers::SHIFT),
                Some(Message::Chrome(ChromeMsg::Palette(PaletteMsg::Open(
                    Overlay::Connect
                ))))
            ));
            assert!(press(c, ctrl()).is_none(), "Ctrl+{c} must stay copy");
        }
    }

    /// `?` is the one *printable* key bound bare, because it is the key every
    /// tool in this family answers with its help. Since #190 it is a chord —
    /// `resolve` no longer owns it — and it still only fires when no text
    /// input consumed the keystroke.
    #[test]
    fn the_help_key_is_a_chord() {
        assert_eq!(chord(&Key::Character("?".into())), Some(Chord::Help));
        assert!(
            press("?", Modifiers::empty()).is_none(),
            "resolve must not double-answer the chord"
        );
    }

    /// Alt+N reaches every saved layout and stops there: the range is
    /// `LayoutPreset::ALL`, so the digits that pointed at panes until #180
    /// were *retargeted*, not left dangling — and past the presets, nothing.
    #[test]
    fn alt_digits_cover_the_layouts_and_stop() {
        assert!(matches!(
            press("1", Modifiers::ALT),
            Some(Message::Workspace(WorkspaceMsg::LayoutPreset(
                LayoutPreset::Explore
            )))
        ));
        assert!(matches!(
            press("2", Modifiers::ALT),
            Some(Message::Workspace(WorkspaceMsg::LayoutPreset(
                LayoutPreset::Watch
            )))
        ));
        assert!(matches!(
            press("3", Modifiers::ALT),
            Some(Message::Workspace(WorkspaceMsg::LayoutPreset(
                LayoutPreset::Diagnose
            )))
        ));
        // Past the end is nothing, not a wrap. Alt+4 was the nodes pane and
        // Alt+5 the admin pane until #180; both are workbench tools now,
        // reached through the workbench's own strip and the palette, and the
        // digits must not quietly become something else on the way.
        assert!(press("4", Modifiers::ALT).is_none());
        assert!(press("5", Modifiers::ALT).is_none());
        assert!(press("9", Modifiers::ALT).is_none());
        assert!(press("0", Modifiers::ALT).is_none());
        for preset in LayoutPreset::ALL {
            assert!(
                LAYOUT_MESSAGES.iter().any(|m| format!("{:?}", m())
                    == format!(
                        "{:?}",
                        Message::Workspace(WorkspaceMsg::LayoutPreset(preset))
                    )),
                "{preset:?} has no binding"
            );
        }
    }
}

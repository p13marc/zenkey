//! The keyboard map (issues #73, #75) — **one** place, so `?` cannot lie.
//!
//! A shortcut table that lives in two places (the handler and the help text)
//! drifts on the first change, and the help is the half nobody notices is
//! wrong. So the table below is the only definition: [`resolve`] dispatches
//! from it and [`map`] renders it, which makes "does the overlay list what the
//! app actually does" a property a test can check rather than a review habit.
//!
//! Every entry emits a [`Message`] that some UI path *also* emits. That is the
//! rule this module is built to keep: a shortcut is a faster way to do a thing,
//! never a second implementation of it.

use iced::keyboard::{Key, Modifiers, key::Named};

use crate::message::{ChromeMsg, DeploymentMsg, Message, PrefsMsg, WorkspaceMsg};
use crate::prefs::LayoutPreset;
use crate::view::palette::{Overlay, PaletteMsg};

/// One binding: how it is typed, what it does, and the message it sends.
pub struct Binding {
    /// How to type it, as a human reads it.
    pub keys: &'static str,
    /// What it does, in the imperative.
    pub what: &'static str,
    /// The message — a function so a `Message` need not be `const`.
    pub message: fn() -> Message,
}

/// The whole map, in the order the help overlay shows it.
pub fn map() -> Vec<Binding> {
    let mut out = vec![
        Binding {
            keys: "Ctrl +",
            what: "zoom in",
            message: || Message::Chrome(ChromeMsg::Prefs(PrefsMsg::ZoomIn)),
        },
        Binding {
            keys: "Ctrl -",
            what: "zoom out",
            message: || Message::Chrome(ChromeMsg::Prefs(PrefsMsg::ZoomOut)),
        },
        Binding {
            keys: "Ctrl 0",
            what: "reset zoom",
            message: || Message::Chrome(ChromeMsg::Prefs(PrefsMsg::ZoomReset)),
        },
        Binding {
            keys: "Ctrl T",
            what: "toggle theme",
            message: || Message::Chrome(ChromeMsg::Prefs(PrefsMsg::ThemeToggled)),
        },
        Binding {
            keys: "Ctrl R",
            what: "reconnect",
            message: || Message::Deployment(DeploymentMsg::Reconnect),
        },
        Binding {
            keys: "Ctrl P",
            what: "command palette",
            message: || Message::Chrome(ChromeMsg::Palette(PaletteMsg::Open(Overlay::Commands))),
        },
        Binding {
            keys: "Ctrl K",
            what: "jump to an observed key",
            message: || Message::Chrome(ChromeMsg::Palette(PaletteMsg::Open(Overlay::Keys))),
        },
        // Connect finally has a keyboard route (#185): it used to be the one
        // pane without one, and it is the surface a lost user most needs.
        Binding {
            keys: "Ctrl Shift C",
            what: "connect — contexts and endpoints",
            message: || Message::Chrome(ChromeMsg::Palette(PaletteMsg::Open(Overlay::Connect))),
        },
        // The launch knobs, surfaced (#188) — the family's usual chord for a
        // settings surface.
        Binding {
            keys: "Ctrl ,",
            what: "settings — bounds and launch knobs",
            message: || Message::Chrome(ChromeMsg::Palette(PaletteMsg::Open(Overlay::Settings))),
        },
    ];
    // The saved layouts, in preset order — so the numbers on screen and the
    // numbers under the fingers are the same list.
    //
    // These digits pointed at panes until #180. Five panes had five digits;
    // the panes are docks of a grid now, and "which pane shows" stopped
    // being a thing a digit can mean. What a digit means instead is a whole
    // layout: Explore, Watch, Diagnose. The rest of the remap — Alt+L/I/A to
    // focus a dock — is #190's.
    for (i, preset) in LayoutPreset::ALL.into_iter().enumerate() {
        out.push(Binding {
            keys: LAYOUT_KEYS[i],
            what: LAYOUT_WHAT[i],
            message: LAYOUT_MESSAGES[i],
        });
        let _ = preset;
    }
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

/// A key press → the message it should send, if any.
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
    // `?` opens the shortcut map. Unmodified on purpose — it is the one key
    // every TUI/GUI in this family answers, and it reaches here only when no
    // text input consumed it.
    if let Key::Character(c) = key
        && c.as_str() == "?"
    {
        return Some(Message::Chrome(ChromeMsg::Palette(PaletteMsg::Open(
            Overlay::Help,
        ))));
    }
    if mods.alt()
        && let Key::Character(c) = key
        && let Ok(digit) = c.parse::<usize>()
    {
        // Alt+1/2/3 are the saved layouts (#180). The digits pointed at
        // panes until the panes became docks of one grid; the bound is
        // `LayoutPreset::ALL`, so Alt+4 and up do nothing rather than
        // quietly becoming something else — the same discipline the pane
        // digits kept when their list shrank.
        if (1..=LayoutPreset::ALL.len()).contains(&digit) {
            return Some(Message::Workspace(WorkspaceMsg::LayoutPreset(
                LayoutPreset::ALL[digit - 1],
            )));
        }
    }
    None
}

/// Whether a key press is the "get out of here" key. Kept here so the Esc
/// layering (#75: palette > overlays > selection) has one definition.
pub fn is_escape(key: &Key) -> bool {
    matches!(key, Key::Named(Named::Escape))
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

    /// Every binding the overlay advertises is one `resolve` actually
    /// dispatches. Without this, `?` is documentation that drifts.
    #[test]
    fn every_advertised_binding_is_dispatchable() {
        for b in map() {
            let (mods, ch) = spelling(b.keys);
            let got = press(&ch, mods);
            let want = (b.message)();
            assert_eq!(
                format!("{got:?}"),
                format!("{:?}", Some(want)),
                "binding {:?} ({}) does not dispatch",
                b.keys,
                b.what
            );
        }
    }

    /// Parse a help-text spelling back into the press it describes — which is
    /// what makes the test above a real check rather than a restatement.
    fn spelling(keys: &str) -> (Modifiers, String) {
        let mut words: Vec<&str> = keys.split(' ').collect();
        let ch = words.pop().expect("every binding names a key");
        assert!(!words.is_empty(), "every binding has a modifier");
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
    /// box must not toggle the theme. (`?` is the deliberate exception — see
    /// `the_help_key_is_the_one_unmodified_binding`.)
    #[test]
    fn unmodified_keys_are_never_shortcuts() {
        for c in ["t", "r", "0", "-", "1"] {
            assert!(press(c, Modifiers::empty()).is_none(), "{c}");
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

    /// `?` is the one binding without a modifier, because it is the key every
    /// tool in this family answers with its help. It still only fires when no
    /// text input consumed the keystroke.
    #[test]
    fn the_help_key_is_the_one_unmodified_binding() {
        assert!(matches!(
            press("?", Modifiers::empty()),
            Some(Message::Chrome(ChromeMsg::Palette(PaletteMsg::Open(
                Overlay::Help
            ))))
        ));
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

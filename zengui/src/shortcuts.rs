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

use crate::message::{Message, PrefsMsg, RightPane};
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
            message: || Message::Prefs(PrefsMsg::ZoomIn),
        },
        Binding {
            keys: "Ctrl -",
            what: "zoom out",
            message: || Message::Prefs(PrefsMsg::ZoomOut),
        },
        Binding {
            keys: "Ctrl 0",
            what: "reset zoom",
            message: || Message::Prefs(PrefsMsg::ZoomReset),
        },
        Binding {
            keys: "Ctrl T",
            what: "toggle theme",
            message: || Message::Prefs(PrefsMsg::ThemeToggled),
        },
        Binding {
            keys: "Ctrl R",
            what: "reconnect",
            message: || Message::Reconnect,
        },
        Binding {
            keys: "Ctrl P",
            what: "command palette",
            message: || Message::Palette(PaletteMsg::Open(Overlay::Commands)),
        },
        Binding {
            keys: "Ctrl K",
            what: "jump to an observed key",
            message: || Message::Palette(PaletteMsg::Open(Overlay::Keys)),
        },
    ];
    // The pane strip, in tab order — so the numbers on screen and the numbers
    // under the fingers are the same list.
    for (i, pane) in RightPane::ALL.into_iter().enumerate() {
        out.push(Binding {
            keys: PANE_KEYS[i],
            what: PANE_WHAT[i],
            message: PANE_MESSAGES[i],
        });
        let _ = pane;
    }
    out
}

/// Alt+1..Alt+9 then Alt+0 for the tenth, one per pane. Parallel arrays rather
/// than a formatted string because `Binding` holds `&'static str` — and the
/// length assertion below is what keeps them in step with `RightPane::ALL`.
const PANE_KEYS: [&str; 10] = [
    "Alt 1", "Alt 2", "Alt 3", "Alt 4", "Alt 5", "Alt 6", "Alt 7", "Alt 8", "Alt 9", "Alt 0",
];
const PANE_WHAT: [&str; 10] = [
    "echo pane",
    "call pane",
    "publish pane",
    "detail pane",
    "nodes pane",
    "doctor pane",
    "history pane",
    "blobs pane",
    "admin pane",
    "connect pane",
];
const PANE_MESSAGES: [fn() -> Message; 10] = [
    || Message::PaneSelected(RightPane::Echo),
    || Message::PaneSelected(RightPane::Call),
    || Message::PaneSelected(RightPane::Publish),
    || Message::PaneSelected(RightPane::Detail),
    || Message::PaneSelected(RightPane::Nodes),
    || Message::PaneSelected(RightPane::Doctor),
    || Message::PaneSelected(RightPane::History),
    || Message::PaneSelected(RightPane::Blob),
    || Message::PaneSelected(RightPane::Admin),
    || Message::PaneSelected(RightPane::Connect),
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
            // `+` normally needs Shift on `=`; accept both spellings rather
            // than making the user find the numpad.
            "+" | "=" => Some(Message::Prefs(PrefsMsg::ZoomIn)),
            "-" | "_" => Some(Message::Prefs(PrefsMsg::ZoomOut)),
            "0" => Some(Message::Prefs(PrefsMsg::ZoomReset)),
            "t" | "T" => Some(Message::Prefs(PrefsMsg::ThemeToggled)),
            "r" | "R" => Some(Message::Reconnect),
            "p" | "P" => Some(Message::Palette(PaletteMsg::Open(Overlay::Commands))),
            "k" | "K" => Some(Message::Palette(PaletteMsg::Open(Overlay::Keys))),
            _ => None,
        };
    }
    // `?` opens the shortcut map. Unmodified on purpose — it is the one key
    // every TUI/GUI in this family answers, and it reaches here only when no
    // text input consumed it.
    if let Key::Character(c) = key
        && c.as_str() == "?"
    {
        return Some(Message::Palette(PaletteMsg::Open(Overlay::Help)));
    }
    if mods.alt()
        && let Key::Character(c) = key
        && let Ok(digit) = c.parse::<usize>()
    {
        // Alt+1..9 are panes 1..9; Alt+0 is the tenth, following the tab-bar
        // convention every browser uses. Beyond ten panes the strip needs a
        // different idea, and this stops rather than wrapping to something
        // arbitrary.
        let n = if digit == 0 { 10 } else { digit };
        if (1..=RightPane::ALL.len()).contains(&n) {
            return Some(Message::PaneSelected(RightPane::ALL[n - 1]));
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
        let (mods, ch) = keys.split_once(' ').expect("every binding has a modifier");
        let mods = match mods {
            "Ctrl" => Modifiers::CTRL,
            "Alt" => Modifiers::ALT,
            other => panic!("unknown modifier {other:?}"),
        };
        (mods, ch.to_lowercase())
    }

    /// The pane list is generated from `RightPane::ALL`, so a new pane must
    /// come with a key rather than silently falling off the end.
    #[test]
    fn the_pane_bindings_cover_every_pane() {
        assert_eq!(PANE_KEYS.len(), RightPane::ALL.len());
        assert_eq!(PANE_WHAT.len(), RightPane::ALL.len());
        assert_eq!(PANE_MESSAGES.len(), RightPane::ALL.len());
        for (i, pane) in RightPane::ALL.into_iter().enumerate() {
            assert_eq!(
                format!("{:?}", PANE_MESSAGES[i]()),
                format!("{:?}", Message::PaneSelected(pane))
            );
        }
    }

    /// The several spellings one physical key arrives as.
    #[test]
    fn zoom_accepts_the_shifted_and_unshifted_spellings() {
        for c in ["+", "="] {
            assert!(matches!(
                press(c, ctrl()),
                Some(Message::Prefs(PrefsMsg::ZoomIn))
            ));
        }
        for c in ["-", "_"] {
            assert!(matches!(
                press(c, ctrl()),
                Some(Message::Prefs(PrefsMsg::ZoomOut))
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

    /// `?` is the one binding without a modifier, because it is the key every
    /// tool in this family answers with its help. It still only fires when no
    /// text input consumed the keystroke.
    #[test]
    fn the_help_key_is_the_one_unmodified_binding() {
        assert!(matches!(
            press("?", Modifiers::empty()),
            Some(Message::Palette(PaletteMsg::Open(Overlay::Help)))
        ));
    }

    /// Alt+N reaches every pane and stops there: the range is `RightPane::ALL`,
    /// so adding a pane moves the boundary rather than leaving a dead key. With
    /// ten panes, Alt+0 is the tenth — the tab-bar convention.
    #[test]
    fn alt_digits_cover_the_panes_and_stop() {
        assert!(matches!(
            press("9", Modifiers::ALT),
            Some(Message::PaneSelected(RightPane::Admin))
        ));
        assert!(matches!(
            press("0", Modifiers::ALT),
            Some(Message::PaneSelected(RightPane::Connect))
        ));
        // Every pane is reachable by *some* digit — the property that matters,
        // stated rather than spot-checked.
        for pane in RightPane::ALL {
            assert!(
                PANE_MESSAGES
                    .iter()
                    .any(|m| format!("{:?}", m()) == format!("{:?}", Message::PaneSelected(pane))),
                "{pane:?} has no binding"
            );
        }
    }
}

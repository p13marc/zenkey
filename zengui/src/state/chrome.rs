//! The window itself: what it looks like, what it remembers, what is over it.
//!
//! The only sub-state a deployment change never touches. That is its whole
//! definition — a theme, a zoom level and an open palette are facts about
//! *this window*, and pointing it at a different fleet is not an event in
//! their lives.

use crate::view;

sub_state! {
    pub(crate) struct Chrome {
        /// Persisted UI preferences (issue #73) — theme, zoom, geometry, and the
        /// scope/context the window was last on.
        pub(crate) prefs: crate::prefs::Prefs,
        /// Geometry changed and has not been written yet (issue #189). Drives the
        /// settle timer, so a drag writes the file once rather than per pixel.
        pub(crate) window_dirty: bool,
        /// Why the defaults are in force, when a prefs file could not be read.
        /// Rendered once in the status strip; never a reason to refuse to open.
        pub(crate) prefs_note: Option<String>,
        /// The command palette / overlay state (issue #75).
        pub(crate) palette: view::palette::PaletteState,
    }
}

impl Chrome {
    pub(crate) fn new(prefs: crate::prefs::Prefs, prefs_note: Option<String>) -> Chrome {
        Chrome {
            prefs,
            window_dirty: false,
            prefs_note,
            palette: view::palette::PaletteState::default(),
        }
    }
}

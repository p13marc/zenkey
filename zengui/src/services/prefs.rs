//! Writing the preferences file off the update thread (#255).
//!
//! One function, and the debounce is not here: `chrome.prefs_dirty` and the
//! 700 ms settle timer (#189) decide *when* a write is owed, in the handler
//! that owns them. This is only the write itself — `create_dir_all` +
//! serialize + `fs::write`, which used to run on the thread iced renders
//! from, once per theme toggle and once per zoom keypress.

use iced::Task;

use crate::message::{ChromeMsg, Message};
use crate::prefs::Prefs;

/// Persist a snapshot of the preferences. Lands on [`ChromeMsg::PrefsSaved`]:
/// still best-effort — a preference that cannot be written must not fail
/// whatever the user was actually doing — but no longer silent, because the
/// failure now has a landing to be shown from (#255).
pub fn save(prefs: Prefs) -> Task<Message> {
    Task::perform(
        async move { prefs.save_to(&Prefs::path()).map_err(|e| e.to_string()) },
        |r| Message::Chrome(ChromeMsg::PrefsSaved(r)),
    )
}

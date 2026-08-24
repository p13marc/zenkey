//! The shared context store, read and written off the update thread (#255).
//!
//! Every function here is a disk touch that used to run inside `update` —
//! which is the thread iced renders from, so each was a frame the window did
//! not paint. The store itself stays [`zenkey_explorer_config`]: one
//! file, two explorers (#35), and a context created in `zenctl` shows up here
//! without a restart (#67) precisely *because* these re-read it every time.
//!
//! What stays in the handler: validation (`ContextForm::to_stored`), which is
//! pure, and every `form.status` write — a service that flipped a pane's
//! field would be reaching backwards (the same line `services/mod.rs` draws
//! for `in_flight`).

use iced::Task;

use crate::message::{Message, PaneMsg};
use crate::view::contexts::{ContextForm, ContextMsg};

fn done(m: ContextMsg) -> Message {
    Message::Pane(PaneMsg::Context(m))
}

/// Re-read the context names from the shared config (#67): the session-open
/// refresh. Lands on [`ContextMsg::Refreshed`] with the known names and the
/// store's own `current` pointer.
pub fn refresh() -> Task<Message> {
    Task::perform(
        async {
            zenkey_explorer_config::load()
                .map(|c| (c.contexts.keys().cloned().collect(), c.current))
                .map_err(|e| e.to_string())
        },
        |r| done(ContextMsg::Refreshed(r)),
    )
}

/// Read the named context for the editor. Lands on [`ContextMsg::Loaded`].
pub fn load(name: String) -> Task<Message> {
    Task::perform(
        async move {
            let config = zenkey_explorer_config::load().map_err(|e| e.to_string())?;
            let stored = config
                .contexts
                .get(&name)
                .cloned()
                .ok_or_else(|| format!("{name} is no longer in the config"))?;
            Ok((name, Box::new(stored)))
        },
        |r| done(ContextMsg::Loaded(r)),
    )
}

/// A picker click: read the named context and move the store's own `current`
/// pointer to it — the store is shared, and picking a context here must not
/// leave `zenctl context show` naming the old one (issue #189). Lands on
/// [`ContextMsg::Activated`]; the `Option<String>` inside `Ok` is why the
/// pointer write failed, when it did — the switch itself still proceeds.
pub fn select(name: String) -> Task<Message> {
    Task::perform(
        async move {
            let result = (|| {
                let mut config = zenkey_explorer_config::load().map_err(|e| e.to_string())?;
                let stored = config
                    .contexts
                    .get(&name)
                    .cloned()
                    .ok_or_else(|| format!("{name} is no longer in the config"))?;
                config.current = Some(name.clone());
                let pointer = zenkey_explorer_config::save(&config)
                    .err()
                    .map(|e| e.to_string());
                Ok((Box::new(stored), pointer))
            })();
            (name, result)
        },
        |(name, r)| done(ContextMsg::Activated(name, r)),
    )
}

/// Write a validated editor snapshot to the shared config, through `upsert` —
/// never `insert`: replacing the whole entry deleted every field the form had
/// no widget for (issue #194), and the merge is the guard for the next field
/// somebody adds to `StoredContext`. Lands on [`ContextMsg::Saved`] with the
/// re-read name list; `select` rides along so the save-and-switch flow can
/// apply the context only once the write actually landed.
pub fn save(snapshot: ContextForm, select: bool) -> Task<Message> {
    let name = snapshot.name.trim().to_string();
    Task::perform(
        async move {
            let result = (|| {
                let mut config = zenkey_explorer_config::load().map_err(|e| e.to_string())?;
                let mut applied = Ok(());
                zenkey_explorer_config::upsert(&mut config, &name, |c| {
                    applied = snapshot.apply_to(c);
                });
                applied?;
                if select {
                    config.current = Some(name.clone());
                }
                zenkey_explorer_config::save(&config).map_err(|e| e.to_string())?;
                Ok(config.contexts.keys().cloned().collect())
            })();
            (name, result)
        },
        move |(name, result)| {
            done(ContextMsg::Saved {
                name,
                select,
                result,
            })
        },
    )
}

//! The connection pane (#67): named contexts, and the endpoints behind them.
//!
//! Contexts are read and written through the **shared** store, so a context
//! created here is one `zenctl` selects and vice versa — that reciprocity is
//! the feature, not a side effect.
//!
//! The handler is about the *form*. Choosing a context does not rewrite seven
//! `Settings` fields here; it raises [`DeploymentMsg::ContextApplied`] and
//! lets the deployment handler do that with every other deployment change.
//! A failed switch reports through `BusMsg::SessionOpened(Err)` rather than
//! writing the link state here. With that and the deployment rewrite both
//! raised as messages, this is the only handler in the app that takes no
//! [`Ctx`](crate::update::Ctx) at all: one form, and nothing else.

use iced::Task;

use crate::message::{BusMsg, DeploymentMsg, Message};
use crate::view::contexts::{ContextForm, ContextMsg};

/// The connection pane (#67). Contexts are read and written through the
/// **shared** store, so a context created here is one `zenctl` selects and
/// vice versa — that reciprocity is the feature, not a side effect.
pub(crate) fn update(form: &mut ContextForm, msg: ContextMsg) -> Task<Message> {
    match msg {
        ContextMsg::Switched(Ok(session)) => {
            // The forget rides on `SessionOpened` now, for both of that
            // variant's producers — see its handler.
            Task::done(Message::Bus(BusMsg::SessionOpened(Ok(session))))
        }
        ContextMsg::Switched(Err(e)) => {
            form.status = Some(Err(format!("could not connect: {e}")));
            // The link fact goes where every other link fact goes. Both
            // variants of `Switched` now land through `Bus`, which is the
            // honest reading: a context switch is a session open that the
            // connect pane happened to ask for.
            Task::done(Message::Bus(BusMsg::SessionOpened(Err(e))))
        }
        ContextMsg::NameChanged(v) => {
            form.name = v;
            Task::none()
        }
        ContextMsg::ConnectChanged(v) => {
            form.connect = v;
            Task::none()
        }
        ContextMsg::ListenChanged(v) => {
            form.listen = v;
            Task::none()
        }
        ContextMsg::BaseChanged(v) => {
            form.base = v;
            Task::none()
        }
        ContextMsg::ZenohConfigChanged(p) => {
            form.zenoh_config = p;
            Task::none()
        }
        ContextMsg::RegistryChanged(v) => {
            form.registry = v;
            Task::none()
        }
        ContextMsg::TimeoutChanged(v) => {
            form.timeout = v;
            Task::none()
        }
        ContextMsg::ScoutingToggled(b) => {
            form.scouting = b;
            Task::none()
        }
        ContextMsg::Isolate => {
            // RFC 09 §0.1's isolated-verification recipe, one click:
            // multicast off, explicit endpoints only.
            form.scouting = false;
            form.status = Some(Ok(
                "multicast scouting off — an empty result now means \"nothing on these \
                 endpoints\", never \"nothing on the network\" (RFC 09 §0.1)"
                    .into(),
            ));
            Task::none()
        }
        ContextMsg::Load => {
            let Some(name) = form.active.clone() else {
                form.status = Some(Err("pick a context first".into()));
                return Task::none();
            };
            match zenkey_fleet::context_store::load() {
                Ok(config) => match config.contexts.get(&name) {
                    Some(stored) => {
                        form.load_from(&name, stored);
                        form.status = Some(Ok(format!("loaded {name}")));
                    }
                    None => {
                        form.status = Some(Err(format!("{name} is no longer in the config")));
                    }
                },
                Err(e) => form.status = Some(Err(e.to_string())),
            }
            Task::none()
        }
        ContextMsg::Save => {
            save_context(form, false);
            Task::none()
        }
        ContextMsg::SaveAndSelect => {
            if !save_context(form, true) {
                return Task::none();
            }
            let stored = match form.to_stored() {
                Ok(s) => s,
                Err(e) => {
                    form.status = Some(Err(e));
                    return Task::none();
                }
            };
            Task::done(Message::Deployment(DeploymentMsg::ContextApplied {
                name: Some(form.name.trim().to_string()),
                stored: Box::new(stored),
            }))
        }
        ContextMsg::Selected(name) => {
            form.active = Some(name.clone());
            match zenkey_fleet::context_store::load() {
                Ok(mut config) => match config.contexts.get(&name).cloned() {
                    Some(stored) => {
                        form.load_from(&name, &stored);
                        // Move the store's own pointer too, not just this
                        // window's memory of it: the store is shared, and
                        // picking a context here left `zenctl context show`
                        // still naming the old one (issue #189).
                        config.current = Some(name.clone());
                        form.status = match zenkey_fleet::context_store::save(&config) {
                            Ok(()) => Some(Ok(format!("switched to {name}"))),
                            Err(e) => Some(Err(format!(
                                "switched to {name}, but the shared `current` \
                                     pointer could not be written: {e}"
                            ))),
                        };
                        return Task::done(Message::Deployment(DeploymentMsg::ContextApplied {
                            name: Some(name),
                            stored: Box::new(stored),
                        }));
                    }
                    None => {
                        form.status = Some(Err(format!("{name} is no longer in the config")));
                    }
                },
                Err(e) => form.status = Some(Err(e.to_string())),
            }
            Task::none()
        }
    }
}

/// Re-read the context names from the shared config.
pub(crate) fn refresh_contexts(form: &mut ContextForm) {
    if let Ok(config) = zenkey_fleet::context_store::load() {
        form.known = config.contexts.keys().cloned().collect();
        if form.active.is_none() {
            form.active = config.current.clone();
        }
    }
}

/// Write the editor to the shared config. Returns whether it landed.
///
/// Through `upsert`, not `insert`: the store is shared with zenctl, and
/// replacing the whole entry deleted every field this form has no widget
/// for (issue #194). The form now covers all of them, so this is the guard
/// for the next field somebody adds to `StoredContext`.
fn save_context(form: &mut ContextForm, select: bool) -> bool {
    // Validate before touching the store, so a rejected form leaves it be.
    if let Err(e) = form.to_stored() {
        form.status = Some(Err(e));
        return false;
    }
    let name = form.name.trim().to_string();
    let mut config = match zenkey_fleet::context_store::load() {
        Ok(c) => c,
        Err(e) => {
            form.status = Some(Err(e.to_string()));
            return false;
        }
    };
    // A snapshot, because `upsert`'s closure and the status writes below both
    // want the form and only one of them can borrow it.
    let snapshot = form.clone();
    let mut applied = Ok(());
    zenkey_fleet::context_store::upsert(&mut config, &name, |c| applied = snapshot.apply_to(c));
    if let Err(e) = applied {
        form.status = Some(Err(e));
        return false;
    }
    if select {
        config.current = Some(name.clone());
    }
    if let Err(e) = zenkey_fleet::context_store::save(&config) {
        form.status = Some(Err(e.to_string()));
        return false;
    }
    refresh_contexts(form);
    form.active = Some(name.clone());
    form.status = Some(Ok(format!(
        "saved {name} to {}",
        zenkey_fleet::context_store::config_path().display()
    )));
    true
}

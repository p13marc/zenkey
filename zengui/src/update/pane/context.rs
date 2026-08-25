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
//!
//! Since #255 it touches no disk either: every read and write of the store
//! goes through [`services::context`] and lands back here as a message —
//! `update` runs on the thread iced renders from, and a TOML read on a click
//! was a frame the window did not paint. What stays synchronous is what is
//! pure: validation, and every `form.status` write.

use iced::Task;

use crate::message::{BusMsg, DeploymentMsg, Message};
use crate::services;
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
            form.status = Some(Err(e.context("could not connect")));
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
            services::context::load(name)
        }
        ContextMsg::Loaded(Ok((name, stored))) => {
            form.load_from(&name, &stored);
            form.status = Some(Ok(format!("loaded {name}")));
            Task::none()
        }
        ContextMsg::Loaded(Err(e)) => {
            form.status = Some(Err(e));
            Task::none()
        }
        ContextMsg::Save => {
            // Validate before touching the store, so a rejected form leaves
            // it be — pure, so it stays on the click, where the red text
            // appears next to the field still holding the typo.
            if let Err(e) = form.to_stored() {
                form.status = Some(Err(e.into()));
                return Task::none();
            }
            services::context::save(form.clone(), false)
        }
        ContextMsg::SaveAndSelect => {
            if let Err(e) = form.to_stored() {
                form.status = Some(Err(e.into()));
                return Task::none();
            }
            // The switch waits for `Saved`: applying a context whose write
            // then failed would point the session at a config the store
            // does not hold.
            services::context::save(form.clone(), true)
        }
        ContextMsg::Saved {
            name,
            select,
            result,
        } => match result {
            Ok(known) => {
                form.known = known;
                form.active = Some(name.clone());
                form.status = Some(Ok(format!(
                    "saved {name} to {}",
                    zenkey_explorer_config::config_path().display()
                )));
                if !select {
                    return Task::none();
                }
                // Re-validated rather than carried in the message: the form
                // is the source of truth the user may have kept typing into.
                let stored = match form.to_stored() {
                    Ok(s) => s,
                    Err(e) => {
                        form.status = Some(Err(e.into()));
                        return Task::none();
                    }
                };
                Task::done(Message::Deployment(DeploymentMsg::ContextApplied {
                    name: Some(name),
                    stored: Box::new(stored),
                }))
            }
            Err(e) => {
                form.status = Some(Err(e));
                Task::none()
            }
        },
        ContextMsg::Selected(name) => {
            // The picker answers on the click; the store's read and the
            // shared-pointer write land on `Activated`.
            form.active = Some(name.clone());
            services::context::select(name)
        }
        ContextMsg::Activated(name, Ok((stored, pointer))) => {
            form.load_from(&name, &stored);
            // The store's own pointer moved too, not just this window's
            // memory of it: the store is shared, and picking a context here
            // used to leave `zenctl context show` naming the old one
            // (issue #189).
            form.status = Some(match pointer {
                None => Ok(format!("switched to {name}")),
                Some(e) => Err(crate::services::ServiceError::msg(format!(
                    "switched to {name}, but the shared `current` pointer \
                     could not be written: {e}"
                ))),
            });
            Task::done(Message::Deployment(DeploymentMsg::ContextApplied {
                name: Some(name),
                stored,
            }))
        }
        ContextMsg::Activated(_, Err(e)) => {
            form.status = Some(Err(e));
            Task::none()
        }
        ContextMsg::Refreshed(Ok((known, current))) => {
            form.known = known;
            if form.active.is_none() {
                form.active = current;
            }
            Task::none()
        }
        ContextMsg::Refreshed(Err(e)) => {
            // Best-effort like the sync re-read it replaces — but logged,
            // not vanished (#255): the picker quietly missing a context is
            // exactly the symptom worth a trace.
            tracing::warn!("could not re-read the shared context config: {e}");
            Task::none()
        }
    }
}

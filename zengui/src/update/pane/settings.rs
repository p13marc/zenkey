//! The Settings overlay (#188): the form, and the apply that tunes the
//! deployment.
//!
//! Like the Connect and Selectors handlers, this one is about the *form*:
//! a valid apply raises [`DeploymentMsg::TuningApplied`] and the deployment
//! handler does the resizing, the labelling and — for a registry change —
//! the forget path. A form that does not parse is refused **here**, beside
//! the box that shows why, and never half-applies.

use iced::Task;

use crate::message::{DeploymentMsg, Message};
use crate::view::settings::{SettingsForm, SettingsMsg, Tuning};

/// Like the Connect handler, this takes no [`Ctx`](crate::update::Ctx) at
/// all: one form, and the deployment rewrite raised as a message.
pub(crate) fn update(form: &mut SettingsForm, msg: SettingsMsg) -> Task<Message> {
    match msg {
        SettingsMsg::EchoLinesChanged(v) => {
            form.echo_lines = v;
            Task::none()
        }
        SettingsMsg::HistoryEntriesChanged(v) => {
            form.history_entries = v;
            Task::none()
        }
        SettingsMsg::MaxKeysChanged(v) => {
            form.max_keys = v;
            Task::none()
        }
        SettingsMsg::TimeoutChanged(v) => {
            form.timeout = v;
            Task::none()
        }
        SettingsMsg::EagerToggled(b) => {
            form.eager = b;
            Task::none()
        }
        SettingsMsg::RegistryChanged(v) => {
            form.registry = v;
            Task::none()
        }
        SettingsMsg::Apply => {
            // Everything is validated before anything is emitted, so a
            // rejected form never half-applies — the same posture as the
            // context form and the CLI boundary (zero bounds are refused
            // with the flags' own words).
            let tuning = match parse(form) {
                Ok(t) => t,
                Err(e) => {
                    form.status = Some(Err(e));
                    return Task::none();
                }
            };
            Task::done(Message::Deployment(DeploymentMsg::TuningApplied(tuning)))
        }
    }
}

/// The whole form, parsed — or the first field that cannot be.
fn parse(form: &SettingsForm) -> Result<Tuning, String> {
    let bound = |what: &str, v: &str| -> Result<usize, String> {
        let n = v
            .trim()
            .parse::<usize>()
            .map_err(|_| format!("{what} {v:?} is not a whole number"))?;
        if n == 0 {
            return Err(format!("{what} must be at least 1"));
        }
        Ok(n)
    };
    Ok(Tuning {
        echo_lines: bound("echo lines", &form.echo_lines)?,
        history_entries: bound("history entries", &form.history_entries)?,
        max_keys: bound("max keys", &form.max_keys)?,
        timeout_secs: form.timeout.trim().parse::<u64>().map_err(|_| {
            format!(
                "timeout {:?} is not a whole number of seconds",
                form.timeout
            )
        })?,
        eager: form.eager,
        registry: crate::view::contexts::words(&form.registry)
            .into_iter()
            .map(Into::into)
            .collect(),
    })
}

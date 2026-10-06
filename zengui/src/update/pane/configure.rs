//! The Config tool's handler (#481): the form's edits, and the calls it
//! sends — through the engine's `call`, the path `zenctl config` takes.

use std::time::Instant;

use iced::Task;

use crate::configure::{
    ConfigAct, ConfigForm, ConfigRead, GroupNote, Reply, change_of, classify, fresh_key,
    preview_edits,
};
use crate::message::{Message, PaneMsg};
use crate::services;
use crate::update::Ctx;
use crate::view::configure::ConfigMsg;

pub(crate) fn update(form: &mut ConfigForm, msg: ConfigMsg, cx: Ctx<'_>) -> Task<Message> {
    match msg {
        ConfigMsg::OriginChanged(s) => {
            form.target.origin = s;
            Task::none()
        }
        ConfigMsg::ProducerChanged(s) => {
            form.target.producer = s;
            Task::none()
        }
        ConfigMsg::ResourceChanged(s) => {
            form.target.resource = s;
            Task::none()
        }
        ConfigMsg::Read => read(form, cx),
        ConfigMsg::Open(target) => {
            form.target = target;
            read(form, cx)
        }
        ConfigMsg::Draft { group, param, text } => {
            form.drafts.insert((group.clone(), param), text);
            // A new edit is a new change: the last outcome no longer
            // describes it, and its key is not the retry's.
            form.notes.remove(&group);
            form.keys.remove(&group);
            Task::none()
        }
        ConfigMsg::Discard(group) => {
            form.drafts.retain(|(g, _), _| *g != group);
            form.notes.remove(&group);
            form.keys.remove(&group);
            Task::none()
        }
        ConfigMsg::Apply(group) => send(form, cx, group, false),
        ConfigMsg::Preview(group) => send(form, cx, group, true),
        ConfigMsg::Answered(act, result) => {
            form.in_flight = None;
            let reply = classify(&result);
            match act {
                ConfigAct::Read => {
                    form.read = Some(ConfigRead {
                        reply,
                        target: form.target.clone(),
                        at: Instant::now(),
                    });
                    Task::none()
                }
                ConfigAct::Set(group) => landed_set(form, cx, group, reply),
                ConfigAct::Preview(group) => {
                    let note = match reply {
                        Reply::Other(text) => match preview_edits(&text) {
                            Some(edits) => GroupNote::Preview(edits),
                            None => GroupNote::Refused(format!(
                                "the dry run's reply is not a report of edits: {text}"
                            )),
                        },
                        other => outcome_note(other),
                    };
                    form.notes.insert(group, note);
                    Task::none()
                }
            }
        }
    }
}

/// Ask the target's read-back. Nothing leaves for a target that cannot be
/// asked, with no session, or while a call is already out — the button is
/// disabled for the same three, and this is the floor under it.
fn read(form: &mut ConfigForm, cx: Ctx<'_>) -> Task<Message> {
    if form.target.unaskable().is_some() || form.in_flight.is_some() {
        return Task::none();
    }
    let Some(session) = cx.dep.session.clone() else {
        return Task::none();
    };
    form.in_flight = Some(ConfigAct::Read);
    let procedure = form.target.read_path();
    call(form, cx, session, procedure, None, ConfigAct::Read)
}

/// Send a group's change, or preview it (#481). Typed against the served
/// kinds and judged by the producer's own validator first — so whatever
/// would be refused there is refused here, in the same words, with nothing
/// sent. Guarded by the read-back's revision, keyed for a retry.
fn send(form: &mut ConfigForm, cx: Ctx<'_>, group: String, dry_run: bool) -> Task<Message> {
    if form.target.unaskable().is_some() || form.in_flight.is_some() {
        return Task::none();
    }
    let Some(session) = cx.dep.session.clone() else {
        return Task::none();
    };
    let Some(view) = form.view() else {
        return Task::none();
    };
    let Ok(mut change) = change_of(view, &group, &form.drafts) else {
        return Task::none();
    };
    if change.values.is_empty() {
        return Task::none();
    }
    change.expected_revision = Some(view.revision);
    change.dry_run = dry_run;
    if view.schema().validate(&group, &change).is_err() {
        return Task::none();
    }
    if !dry_run {
        // The same key until an answer is known (`GroupNote::Unknown`).
        let key = form
            .keys
            .entry(group.clone())
            .or_insert_with(fresh_key)
            .clone();
        change.idempotency_key = Some(key);
    }
    let Ok(body) = serde_json::to_vec(&change) else {
        return Task::none();
    };
    let act = if dry_run {
        ConfigAct::Preview(group.clone())
    } else {
        ConfigAct::Set(group.clone())
    };
    form.in_flight = Some(act.clone());
    let procedure = form.target.set_path(&group);
    call(form, cx, session, procedure, Some(body), act)
}

/// What a sent change came back as.
fn landed_set(form: &mut ConfigForm, cx: Ctx<'_>, group: String, reply: Reply) -> Task<Message> {
    match reply {
        // A hot change answers with the read-back (RFC 05 §5.1): it is the
        // document now, and the drafts it carried are spent.
        Reply::Document(view) => {
            form.notes.insert(
                group.clone(),
                GroupNote::Applied {
                    revision: view.revision,
                },
            );
            form.drafts.retain(|(g, _), _| *g != group);
            form.keys.remove(&group);
            form.read = Some(ConfigRead {
                reply: Reply::Document(view),
                target: form.target.clone(),
                at: Instant::now(),
            });
            Task::none()
        }
        // The document moved under the edit (the reference validator's
        // `StaleRevision` words): read again, keep the draft — the person
        // decides again against what is there now.
        Reply::Refused { message, .. } if message.contains("and the document is at") => {
            form.notes.insert(group.clone(), GroupNote::Moved);
            form.keys.remove(&group);
            read(form, cx)
        }
        Reply::Silent => {
            // The key stays: the retry is the same change.
            form.notes.insert(group, GroupNote::Unknown);
            Task::none()
        }
        other => {
            form.keys.remove(&group);
            form.notes.insert(group, outcome_note(other));
            Task::none()
        }
    }
}

/// A reply that is not the one an act expects, as a group's note.
fn outcome_note(reply: Reply) -> GroupNote {
    match reply {
        Reply::Refused { name, message } => GroupNote::Refused(format!("{name} — {message}")),
        Reply::Silent => GroupNote::Unknown,
        Reply::Failed(why) => GroupNote::Refused(why),
        Reply::Other(text) => GroupNote::Refused(format!("an unexpected reply: {text}")),
        Reply::Document(_) => GroupNote::Refused(
            "the producer answered a dry run with its read-back — nothing to preview".into(),
        ),
    }
}

/// One call on the form's target, landing on `act`.
fn call(
    form: &ConfigForm,
    cx: Ctx<'_>,
    session: zenoh::Session,
    procedure: String,
    body: Option<Vec<u8>>,
    act: ConfigAct,
) -> Task<Message> {
    let t = &form.target;
    // The claimed actor, for the change event's record — a label, never an
    // authentication (RFC 06 §5.5).
    let params = match std::env::var("USER") {
        Ok(user) if body.is_some() && zenkey::grammar::is_valid_plain_chunk(&user) => {
            vec![format!("actor={user}")]
        }
        _ => Vec::new(),
    };
    services::write::call_then(
        services::write::Call {
            session,
            base: cx.dep.base().to_string(),
            target: t.origin.trim().to_string(),
            producer: t.producer.trim().to_string(),
            procedure,
            params,
            body,
            attachment: None,
            timeout: cx.dep.timeout(),
            // The convention's own procedures carry their kind (RFC 05
            // §2.1, v1.48): no slice is needed to judge them.
            slices: None,
        },
        move |r| Message::Pane(PaneMsg::Config(ConfigMsg::Answered(act, r))),
    )
}

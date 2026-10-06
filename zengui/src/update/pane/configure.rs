//! The Config tool's handler (#481): the form's edits, and the calls it
//! sends — through the engine's `call`, the path `zenctl config` takes.

use std::time::{Duration, Instant};

use iced::Task;
use zenkey::config::{ControlRequest, ParamClass, PendingReply};

use crate::configure::{
    Armed, ConfigAct, ConfigForm, ConfigRead, GroupNote, Reply, Verb, busy_token, change_of,
    classify, fresh_key, preview_edits,
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
        ConfigMsg::WindowChanged(s) => {
            form.window = s;
            Task::none()
        }
        ConfigMsg::ExtendChanged(s) => {
            form.extend_by = s;
            Task::none()
        }
        ConfigMsg::ConsentToggled(yes) => {
            form.consent = yes;
            Task::none()
        }
        ConfigMsg::Control(verb, token) => control(form, cx, verb, token),
        // The countdown (#481): one re-read when this window's count runs
        // out, so a rollback is seen — the producer's deadline, not this
        // count, is what rolls the change back.
        ConfigMsg::Tick => match form.armed.as_mut() {
            Some(armed) if !armed.reread && armed.left(Instant::now()).is_zero() => {
                armed.reread = true;
                read(form, cx)
            }
            _ => Task::none(),
        },
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
                ConfigAct::Control(verb) => landed_control(form, verb, reply),
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
    let reach = view.group(&group).map(|g| g.class) == Some(ParamClass::Reach);
    // A change pending on the resource is joined, never raced: the set
    // carries its token (RFC v1.50) and rides its window.
    match view.pending.as_ref() {
        Some(p) => change.token = Some(p.token.clone()),
        None if reach => match form.window_secs() {
            Some(s) => change.confirm_s = Some(s),
            None => return Task::none(),
        },
        None => {}
    }
    // A change that can cut the link leaves with a person's yes (v1.48).
    if reach && !dry_run && !form.consent {
        return Task::none();
    }
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
    if !dry_run {
        form.sent_at = Some(Instant::now());
        // The yes was for this change.
        form.consent = false;
    }
    let procedure = form.target.set_path(&group);
    call(form, cx, session, procedure, Some(body), act)
}

/// Drive a change by its token (RFC 05 §5.1): each verb answers with the
/// read-back after the act (v1.47).
fn control(form: &mut ConfigForm, cx: Ctx<'_>, verb: Verb, token: String) -> Task<Message> {
    if form.target.unaskable().is_some() || form.in_flight.is_some() {
        return Task::none();
    }
    let Some(session) = cx.dep.session.clone() else {
        return Task::none();
    };
    let mut request = ControlRequest::of(token);
    if verb == Verb::Extend {
        match form.extend_secs() {
            Some(s) => request = request.extended_by(s),
            None => return Task::none(),
        }
    }
    let Ok(body) = serde_json::to_vec(&request) else {
        return Task::none();
    };
    form.in_flight = Some(ConfigAct::Control(verb));
    form.sent_at = Some(Instant::now());
    let procedure = form.target.control_path(verb);
    call(
        form,
        cx,
        session,
        procedure,
        Some(body),
        ConfigAct::Control(verb),
    )
}

/// What a control verb came back as. The read-back becomes the document;
/// the armed count follows the act.
fn landed_control(form: &mut ConfigForm, verb: Verb, reply: Reply) -> Task<Message> {
    match reply {
        Reply::Document(view) => {
            let note = match verb {
                Verb::Confirm => {
                    "confirmed — permanent until the next restart; persist makes it survive one"
                }
                Verb::Cancel => "cancelled — undone now",
                Verb::Extend => "extended — the producer's new deadline is above",
                Verb::Persist => "persisted — written to the producer's persisted layer",
            };
            match verb {
                Verb::Confirm | Verb::Cancel => form.armed = None,
                Verb::Extend => {
                    let (secs, sent) = (form.extend_secs(), form.sent_at);
                    if let (Some(armed), Some(s)) = (form.armed.as_mut(), secs) {
                        armed.sent = sent.unwrap_or_else(Instant::now);
                        armed.window = Duration::from_secs(s);
                        armed.reread = false;
                    }
                }
                Verb::Persist => {}
            }
            form.control_note = Some(GroupNote::Done(note.to_string()));
            form.read = Some(ConfigRead {
                reply: Reply::Document(view),
                target: form.target.clone(),
                at: Instant::now(),
            });
        }
        other => form.control_note = Some(outcome_note(other)),
    }
    Task::none()
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
        // A reach change answers before it is applied (RFC 05 §5.1): the
        // token, and when it applies. The window is armed from the send, and
        // the read-back asked — over the new link, which is the point.
        Reply::Other(text) if serde_json::from_str::<PendingReply>(&text).is_ok() => {
            let pending: PendingReply = serde_json::from_str(&text).expect("checked");
            form.keys.remove(&group);
            form.drafts.retain(|(g, _), _| *g != group);
            form.notes
                .insert(group, GroupNote::Pending(pending.token.clone()));
            let joined = form
                .armed
                .as_ref()
                .is_some_and(|a| a.token == pending.token);
            if !joined && let Some(s) = form.window_secs() {
                form.armed = Some(Armed {
                    token: pending.token,
                    sent: form.sent_at.unwrap_or_else(Instant::now),
                    window: Duration::from_secs(s),
                    reread: false,
                });
            }
            read(form, cx)
        }
        // Another writer's change is pending: read it back, so the next
        // apply joins it — by its token, said on the button — instead of
        // racing it (RFC v1.50).
        Reply::Refused { name, message } if name == "error/busy" => {
            form.keys.remove(&group);
            let token = busy_token(&message).unwrap_or("?").to_string();
            form.notes.insert(group, GroupNote::Busy(token));
            read(form, cx)
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

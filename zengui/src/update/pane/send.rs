//! The Send pane (#184): the only place in the app that writes, and the one
//! that asks a service to answer — on zk2 since #614 (FL10).
//!
//! `&mut Workbench` rather than `&mut SendForm`, because an armed
//! publication is not view state — a `Publication` is a live bus declaration
//! and dropping it undeclares — so it lives beside the form rather than in it,
//! and the handler moves both.
//!
//! Everything that touches the bus goes through [`crate::services::write`],
//! which goes through the engine: `declare_publication` for a write (P7 — a
//! declared publisher, never an ad-hoc put) and `call_operation` for a call.
//! Every refusal of the form's input happens here, before a task exists: a
//! key a zk2 service owns (P3), a wildcard, a body that is not hex, a
//! tombstone without `--i-know`, an address that is not one, a fan-out to an
//! operation that forbids one (O2), a request that does not encode. And
//! while the panes replay a capture, nothing is written at all (#74).

use std::sync::Arc;

use iced::Task;

use crate::message::Message;
use crate::services;
use crate::services::ServiceError;
use crate::state::workspace::{RepeatLoad, Workbench};
use crate::update::Ctx;
use crate::view::send::{
    CallShape, REPLAY_LOCK, RevisionChoice, SendForm, SendMode, SendMsg, Sent, owned_refusal,
    request_form,
};

/// Refuse `key` on the form when a zk2 service owns it: the refusal is
/// logged and set as the form's error, and `true` says nothing may move.
fn refuse_owned(form: &mut SendForm, key: &str) -> bool {
    let Some(why) = owned_refusal(key) else {
        return false;
    };
    refuse(form, why);
    true
}

/// A refusal of the form's input: logged as a line that did not happen, and
/// set as the form's error. Nothing was sent.
fn refuse(form: &mut SendForm, why: impl Into<String>) {
    let why = why.into();
    form.log(false, why.clone());
    form.error = Some(ServiceError::msg(why));
}

/// A call refused before anything was sent: the outcome says why, the log
/// keeps it, and nothing is in flight.
fn refuse_call(form: &mut SendForm, e: impl std::fmt::Display) {
    let why = format!("refused: {e}");
    form.log(false, why.clone());
    form.in_flight = false;
    form.outcome = Some(Err(ServiceError::msg(why)));
}

/// What a stop says when another reference to the publication is still live.
///
/// This sentence is the #184 defect fix: `Arc::try_unwrap` fails exactly when
/// a repeat send is in flight holding a clone, the undeclare then happens on
/// that task's drop — and the old handler returned `Task::none()`, telling
/// the user nothing at all. A stop must always emit a line saying what
/// happened.
const STOP_DEFERRED: &str = "stopped — a send is in flight; the publication \
                             undeclares when that send completes";

/// Disarm the form and take the publication out of the load, saying what
/// happened either way.
///
/// `Some` = we held the last reference; the caller undeclares it with an
/// acknowledgement ([`SendMsg::Stopped`] logs the confirmation). `None` with
/// a load = a clone is still live and [`STOP_DEFERRED`] was logged. `None`
/// without a load = nothing was armed, and nothing is said — `Stop` is also
/// the defensive prelude to `Send` and `Retire`, and a "stopped" line for a
/// stop that stopped nothing would be its own lie.
///
/// Generic over the payload so the contended branch is testable without a
/// bus: a real `Publication` needs a session, but `Arc::try_unwrap`'s
/// behaviour does not.
fn disarm<P>(form: &mut SendForm, load: Option<Arc<P>>) -> Option<P> {
    form.armed = false;
    let publication = load?;
    match Arc::try_unwrap(publication) {
        // Acknowledged undeclare when we hold the last reference; a drop
        // would undeclare too, but silently.
        Ok(p) => Some(p),
        Err(shared) => {
            drop(shared);
            form.log(true, STOP_DEFERRED);
            None
        }
    }
}

/// Stop an armed publication because a replay began (#74; #614, FL10): the
/// panes now show a capture, and a stream still writing to the live bus
/// would be the live world leaking through the mode. Said, when it stopped
/// something.
pub(crate) fn stop_for_replay(bench: &mut Workbench) -> Task<Message> {
    let Some(load) = bench.publication.take() else {
        bench.send_form.armed = false;
        return Task::none();
    };
    bench.send_form.log(
        true,
        "stopped — a replay began, and replay writes nothing to the live bus",
    );
    match disarm(&mut bench.send_form, Some(load.publication)) {
        Some(p) => services::write::undeclare(p),
        None => Task::none(),
    }
}

/// The revisions in hand that declare an operation (#614, FL10): the
/// store's, which holds what `--contracts` loaded and what was retrieved
/// from the bus — the catalog's or the contracts', never a guess.
fn revision_choices(cx: &Ctx) -> Vec<RevisionChoice> {
    let mut choices: Vec<RevisionChoice> = cx
        .dep
        .store
        .held()
        .into_iter()
        .map(RevisionChoice)
        .filter(|c| !c.operations().is_empty())
        .collect();
    choices.sort_by(|a, b| {
        (a.0.iface().to_string(), a.0.fingerprint().to_string())
            .cmp(&(b.0.iface().to_string(), b.0.fingerprint().to_string()))
    });
    choices.dedup();
    choices
}

/// The addresses presence says provide the picked interface, and whether a
/// presence read stood behind the answer at all (O4).
fn refresh_providers(form: &mut SendForm, cx: &Ctx) {
    form.presence_read = cx.dep.catalog.is_some();
    form.providers = match (&form.revision, &cx.dep.catalog) {
        (Some(r), Some(catalog)) => catalog
            .addresses()
            .filter(|a| catalog.provides(a, r.0.iface()))
            .map(ToString::to_string)
            .collect(),
        _ => Vec::new(),
    };
}

/// `name=value;…` as the template's values: a name given twice is two
/// values, which only a rest parameter takes (`check_values` judges).
fn bindings(params: &str) -> Result<zenkey_model::template::Bindings, String> {
    let mut out = zenkey_model::template::Bindings::new();
    for token in params.split(';').map(str::trim).filter(|t| !t.is_empty()) {
        let Some((name, value)) = token.split_once('=') else {
            return Err(format!(
                "the params are not name=value: {token:?} (spell each as name=value;…)"
            ));
        };
        out.entry(name.trim().to_owned())
            .or_default()
            .push(value.trim().to_owned());
    }
    Ok(out)
}

pub(crate) fn update(
    bench: &mut Workbench,
    replaying: bool,
    msg: SendMsg,
    cx: Ctx,
) -> Task<Message> {
    match msg {
        SendMsg::ModeSelected(mode) => {
            bench.send_form.mode = mode;
            if mode == SendMode::Call {
                bench.send_form.revisions = revision_choices(&cx);
                refresh_providers(&mut bench.send_form, &cx);
            }
            Task::none()
        }

        // ── publish mode ────────────────────────────────────────────────
        SendMsg::Ready(Ok(outcome)) => {
            // No base guard needed (#109 audit): publishing is
            // user-initiated *output* on a full wire key, not observed
            // evidence. (A publication straddling a context switch rides
            // the old session's Arc until stopped — a session-lifetime
            // question, out of #109's scope.)
            let form = &mut bench.send_form;
            form.in_flight = false;
            form.error = None;
            form.matching = outcome.matching;
            let qos = outcome.qos.token();
            form.sent = Some(Sent {
                bytes: outcome.bytes.len(),
                encoding: outcome.encoding.clone(),
                qos: qos.clone(),
            });
            form.log(
                true,
                format!(
                    "published {} ({}) · qos {qos}",
                    outcome.key,
                    crate::view::kit::plural(outcome.bytes.len(), "byte")
                ),
            );
            match &outcome.publication {
                Some(publication) => {
                    form.armed = true;
                    bench.publication = Some(RepeatLoad {
                        publication: publication.clone(),
                        bytes: Arc::clone(&outcome.bytes),
                        attachment: outcome.attachment.clone(),
                    });
                }
                // One-shot: the task already undeclared.
                None => {
                    form.armed = false;
                    bench.publication = None;
                }
            }
            Task::none()
        }
        SendMsg::Ready(Err(e)) => {
            let form = &mut bench.send_form;
            form.in_flight = false;
            form.armed = false;
            form.error = Some(e.clone());
            form.log(false, format!("refused: {e}"));
            bench.publication = None;
            Task::none()
        }
        SendMsg::Tick => {
            // The repeat clock beats only while armed, and a replay stops
            // the stream as it begins; a beat that still lands then writes
            // nothing.
            if replaying {
                return Task::none();
            }
            let Some(load) = bench.publication.as_ref() else {
                return Task::none();
            };
            let publication = load.publication.clone();
            let bytes = load.bytes.clone();
            let attachment = load.attachment.clone();
            services::write::repeat(publication, bytes, attachment)
        }
        SendMsg::Sent(Ok(n)) => {
            let key = bench.send_form.key.clone();
            bench.send_form.log(
                true,
                format!("published {key} ({})", crate::view::kit::plural(n, "byte")),
            );
            Task::none()
        }
        SendMsg::Sent(Err(e)) => {
            // A failed repeat disarms: a stream that silently stopped
            // working would keep claiming it was publishing.
            bench.send_form.log(false, format!("send failed: {e}"));
            bench.send_form.armed = false;
            bench.publication = None;
            Task::none()
        }
        SendMsg::Retired(result) => {
            let form = &mut bench.send_form;
            form.in_flight = false;
            match result {
                Ok(matching) => {
                    // A tombstone has no body: a stale "sent N bytes" line
                    // claiming one shipped would be the pane's own O4
                    // mistake.
                    form.sent = None;
                    let key = form.key.clone();
                    form.log(
                        true,
                        format!("retired {key} — a tombstone (RFC 04 §1.2), not an empty value"),
                    );
                    if matching == Some(false) {
                        form.log(
                            true,
                            "matching: no subscriber matched the tombstone — a routing \
                             fact, not a fleet verdict (RFC 05 §3.1)",
                        );
                    }
                }
                Err(e) => {
                    form.error = Some(e.clone());
                    form.log(false, format!("retire failed: {e}"));
                }
            }
            Task::none()
        }
        SendMsg::Stopped(result) => {
            match result {
                Ok(()) => bench
                    .send_form
                    .log(true, "stopped — publication undeclared"),
                Err(e) => bench.send_form.log(false, format!("undeclare failed: {e}")),
            }
            Task::none()
        }
        SendMsg::KeyChanged(k) => {
            bench.send_form.key = k;
            Task::none()
        }
        SendMsg::BodyChanged(b) => {
            bench.send_form.body = b;
            Task::none()
        }
        SendMsg::HexToggled(b) => {
            bench.send_form.hex = b;
            Task::none()
        }
        SendMsg::PriorityPicked(p) => {
            bench.send_form.qos.priority = p.0;
            bench.send_form.qos_touched = true;
            Task::none()
        }
        SendMsg::CongestionPicked(c) => {
            bench.send_form.qos.congestion = c.0;
            bench.send_form.qos_touched = true;
            Task::none()
        }
        SendMsg::ReliabilityPicked(r) => {
            bench.send_form.qos.reliability = r.0;
            bench.send_form.qos_touched = true;
            Task::none()
        }
        SendMsg::ExpressToggled(b) => {
            bench.send_form.qos.express = b;
            bench.send_form.qos_touched = true;
            Task::none()
        }
        SendMsg::EncodingChanged(e) => {
            bench.send_form.encoding = e;
            Task::none()
        }
        SendMsg::AttachmentChanged(a) => {
            bench.send_form.attachment = a;
            Task::none()
        }
        SendMsg::RepeatToggled(b) => {
            bench.send_form.repeat = b;
            // Turning repeat off mid-stream stops it, rather than leaving
            // an armed publication with the checkbox saying otherwise.
            if !b && bench.publication.is_some() {
                return update(bench, replaying, SendMsg::Stop, cx);
            }
            Task::none()
        }
        SendMsg::IntervalChanged(i) => {
            bench.send_form.interval = i;
            Task::none()
        }
        SendMsg::Stop => {
            let load = bench.publication.take().map(|l| l.publication);
            match disarm(&mut bench.send_form, load) {
                Some(p) => services::write::undeclare(p),
                None => Task::none(),
            }
        }
        SendMsg::RetireIKnowToggled(b) => {
            bench.send_form.retire_i_know = b;
            Task::none()
        }
        SendMsg::Retire => {
            let key = bench.send_form.key.trim().to_string();
            if key.is_empty() {
                return Task::none();
            }
            if replaying {
                refuse(&mut bench.send_form, format!("refused: {REPLAY_LOCK}"));
                return Task::none();
            }
            // A tombstone is a write (P3): an owned key is refused before
            // anything moves — no session asked, no armed publication
            // stopped, and whatever `--i-know` says.
            if refuse_owned(&mut bench.send_form, &key) {
                return Task::none();
            }
            let Some(session) = cx.dep.raw.clone() else {
                return Task::none();
            };
            // The engine is the judge (`check_retire`): a wildcard never,
            // a concrete key only with `--i-know`.
            if let Err(e) = zenkey_fleet::check_retire(&key, bench.send_form.retire_i_know) {
                refuse(&mut bench.send_form, format!("refused: {e}"));
                return Task::none();
            }
            // Same stop-first discipline as Send: a retire must not race
            // an armed publication on the key.
            let stop = update(bench, replaying, SendMsg::Stop, cx);
            bench.send_form.in_flight = true;
            bench.send_form.error = None;
            let send = services::write::retire(session, key, bench.send_form.qos);
            Task::batch([stop, send])
        }
        SendMsg::Send => {
            let key = bench.send_form.key.trim().to_string();
            if key.is_empty() {
                return Task::none();
            }
            // The replay lock (#74): the panes show a capture, and nothing
            // is written to the live bus while they do.
            if replaying {
                refuse(&mut bench.send_form, format!("refused: {REPLAY_LOCK}"));
                return Task::none();
            }
            // P3 first: a key a zk2 service owns is refused before anything
            // moves — `zenctl pub`'s words, no flag that moves it.
            if refuse_owned(&mut bench.send_form, &key) {
                return Task::none();
            }
            // A wildcard is a blast radius, not a publication (#504).
            if let Err(e) = zenkey_fleet::check_concrete(&key, zenkey_fleet::WriteAct::Put) {
                refuse(&mut bench.send_form, format!("refused: {e}"));
                return Task::none();
            }
            let body = match bench.send_form.body_bytes() {
                Ok(b) => b,
                Err(why) => {
                    refuse(&mut bench.send_form, format!("refused: {why}"));
                    return Task::none();
                }
            };
            let Some(session) = cx.dep.raw.clone() else {
                return Task::none();
            };
            // A new send replaces any armed publication: two publishers on
            // one key from one pane would double every sample.
            let stop = update(bench, replaying, SendMsg::Stop, cx);
            let form = &mut bench.send_form;
            let encoding = Some(form.encoding.trim().to_string()).filter(|e| !e.is_empty());
            // Verbatim (#117); empty = none.
            let attachment: Option<Arc<Vec<u8>>> = (!form.attachment.is_empty())
                .then(|| Arc::new(form.attachment.clone().into_bytes()));
            form.in_flight = true;
            form.error = None;
            let send = services::write::publish(services::write::Publish {
                session,
                key,
                encoding,
                body: Arc::new(body),
                qos: form.qos,
                attachment,
                repeat: form.repeat,
            });
            Task::batch([stop, send])
        }

        // ── call mode ───────────────────────────────────────────────────
        SendMsg::Done(outcome) => {
            // No base guard needed (#109 audit): a call reply is the
            // answer to the exact question the user pressed the button
            // for, attributed by its key — user output, not projected
            // deployment state.
            let form = &mut bench.send_form;
            form.in_flight = false;
            match &outcome {
                // Silence and an envelope are answers that landed (O5): a
                // line that happened, never a failure.
                Ok(report) => form.log(true, crate::view::send::call_summary(report)),
                Err(e) => form.log(false, format!("call failed: {e}")),
            }
            form.outcome = Some(outcome.map(|r| (*r).clone()));
            Task::none()
        }
        SendMsg::RevisionsOpened => {
            bench.send_form.revisions = revision_choices(&cx);
            refresh_providers(&mut bench.send_form, &cx);
            Task::none()
        }
        SendMsg::RevisionPicked(choice) => {
            let form = &mut bench.send_form;
            form.operations = choice.operations();
            form.revision = Some(choice);
            form.operation = None;
            form.shape = None;
            form.request = None;
            form.outcome = None;
            refresh_providers(form, &cx);
            // One operation is the only one to pick.
            if let [only] = form.operations.as_slice() {
                let only = only.clone();
                return update(bench, replaying, SendMsg::OperationPicked(only), cx);
            }
            Task::none()
        }
        SendMsg::OperationPicked(name) => {
            let form = &mut bench.send_form;
            let Some(revision) = form.revision.as_ref().map(|c| Arc::clone(&c.0)) else {
                return Task::none();
            };
            // The declared shape and the request form, read from the
            // revision in hand (§7.2): session-free, and an answer — the
            // pane says "not asked" only until an operation is picked.
            match CallShape::of(&revision, &name) {
                Ok(shape) => {
                    form.request = Some(request_form(&revision, &shape.name));
                    form.shape = Some(shape);
                }
                Err(why) => {
                    form.shape = None;
                    form.request = None;
                    refuse_call(form, why);
                }
            }
            form.operation = Some(name);
            form.outcome = None;
            Task::none()
        }
        SendMsg::ScaffoldBody => {
            if let Some(body) = bench.send_form.scaffold() {
                bench.send_form.body = body;
            }
            Task::none()
        }
        SendMsg::TargetChanged(t) => {
            bench.send_form.target = t;
            Task::none()
        }
        SendMsg::ParamsChanged(t) => {
            bench.send_form.params = t;
            Task::none()
        }
        SendMsg::Submit => {
            let form = &mut bench.send_form;
            if form.in_flight {
                return Task::none();
            }
            if replaying {
                refuse_call(form, REPLAY_LOCK);
                return Task::none();
            }
            let (Some(choice), Some(shape)) = (form.revision.clone(), form.shape.clone()) else {
                return Task::none();
            };
            let revision = choice.0;
            // Everything the call could be refused for is refused here,
            // before a task exists — so a refusal provably sent nothing: the
            // address, the parameters, a fan-out to an operation that
            // forbids one (O2), a request that does not encode.
            let target = match zenkey_fleet::ResolvedTarget::parse(form.target.trim()) {
                Ok(t) => t,
                Err(e) => {
                    refuse_call(form, e);
                    return Task::none();
                }
            };
            let values = match bindings(&form.params) {
                Ok(v) => v,
                Err(why) => {
                    refuse_call(form, why);
                    return Task::none();
                }
            };
            let plan = match zenkey_fleet::plan_call(&revision, target, &shape.name, values) {
                Ok(p) => p,
                Err(e) => {
                    refuse_call(form, e);
                    return Task::none();
                }
            };
            let input = if form.body.trim().is_empty() {
                match plan.operation.request {
                    zenkey_model::schema::TypeId::Raw { .. } => Vec::new(),
                    _ => b"{}".to_vec(),
                }
            } else {
                form.body.clone().into_bytes()
            };
            let request = match zenkey_fleet::encode_request(&revision, &plan, &input) {
                Ok(r) => r,
                Err(e) => {
                    refuse_call(form, e);
                    return Task::none();
                }
            };
            // The session in the namespace: an operation's key is
            // base-relative (spec §5.1). None before the sessions open.
            let Some(session) = cx.dep.session.clone() else {
                refuse_call(
                    form,
                    "no session in the deployment's namespace yet — nothing was sent",
                );
                return Task::none();
            };
            // The contract's recommended timeout, else the window's
            // (spec §5.1, "The timeout").
            let timeout = shape
                .timeout_ms
                .map(std::time::Duration::from_millis)
                .unwrap_or_else(|| cx.dep.timeout());
            form.in_flight = true;
            form.outcome = None;
            services::write::call(services::write::Call {
                session,
                revision,
                plan,
                request,
                timeout,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// P3 in the publish (#614, FL6a; FL10): a key a zk2 service owns is
    /// refused — its data keys and its `@zk` control keys alike, under any
    /// namespace and none — and the refusal names the owner and the rule.
    #[test]
    fn an_owned_zk2_key_is_refused_and_a_foreign_one_is_not() {
        for owned in [
            "zk2/robot1/nav/nav.v2/state/status",
            "spray/zk2/robot1/nav/nav.v2/stream/position",
            "spray/zk2/robot1/nav/@zk/instance/0123456789abcdef",
        ] {
            let why = owned_refusal(owned).unwrap_or_else(|| panic!("{owned} passed"));
            assert!(why.contains("robot1/nav"), "{why}");
            assert!(why.contains("P3"), "{why}");
        }

        // On the form: logged, set as the error, and nothing armed.
        let mut form = SendForm::default();
        assert!(refuse_owned(
            &mut form,
            "zk2/robot1/nav/nav.v2/state/status"
        ));
        assert!(!form.log[0].ok, "a refusal is a failure line");
        assert!(form.error.as_ref().unwrap().to_string().contains("P3"));
        assert!(!form.in_flight && !form.armed);
        let mut clean = SendForm::default();
        assert!(!refuse_owned(&mut clean, "demo/example/foo"));
        assert!(clean.log.is_empty() && clean.error.is_none());
    }

    /// The #184 defect, pinned: a stop while a repeat send still holds the
    /// publication used to return `Task::none()` and say nothing — the
    /// undeclare happened on a drop the user never saw. Every branch of a
    /// stop now has a voice, except the one that stopped nothing.
    #[test]
    fn a_contended_stop_always_logs_and_an_idle_one_says_nothing() {
        let mut form = SendForm::default();

        // Sole owner: the publication comes back for the acknowledged
        // undeclare; the confirmation line is `Stopped(Ok)`'s job.
        assert!(disarm(&mut form, Some(Arc::new(()))).is_some());
        assert!(form.log.is_empty(), "the ack line belongs to Stopped(Ok)");

        // A repeat send in flight holds a clone: the take fails, the
        // undeclare rides that task's drop — and the user is told.
        let publication = Arc::new(());
        let in_flight = publication.clone();
        assert!(disarm(&mut form, Some(publication)).is_none());
        assert_eq!(form.log[0].text, STOP_DEFERRED);
        assert!(form.log[0].ok, "a deferred stop is not a failure");
        drop(in_flight);

        // Nothing armed (Stop is also Send's and Retire's defensive
        // prelude): a "stopped" line for a no-op would be a lie.
        let before = form.log.len();
        assert!(disarm::<()>(&mut form, None).is_none());
        assert_eq!(form.log.len(), before);
        assert!(!form.armed);
    }

    /// The params are the template's values: a name given twice is two
    /// values (a rest parameter's chunks), and a token with no `=` is
    /// refused by name.
    #[test]
    fn params_read_as_bindings() {
        let b = bindings(" ns=default ; iface = eth0;;path=a;path=b").unwrap();
        assert_eq!(b["ns"], ["default"]);
        assert_eq!(b["iface"], ["eth0"]);
        assert_eq!(b["path"], ["a", "b"]);
        assert!(bindings("").unwrap().is_empty());
        let e = bindings("ns").unwrap_err();
        assert!(e.contains("\"ns\""), "{e}");
    }
}

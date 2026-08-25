//! The Send pane (#184): the only place in the app that writes, and the one
//! that asks a producer to answer.
//!
//! `&mut Workbench` rather than `&mut SendForm`, because an armed
//! publication is not view state — a `Publication` is a live bus declaration
//! and dropping it undeclares — so it lives beside the form rather than in it,
//! and the handler moves both.
//!
//! Everything that touches the bus goes through [`crate::services::write`],
//! which goes through the engine: `prepare_publish` for the body (#97's
//! encode ladder), `declare_publication` for the write (P7 — a declared
//! publisher, never an ad-hoc put), and `zenkey_fleet::call` for the RPC. No
//! codec logic on this side of the seam.

use std::sync::Arc;

use iced::Task;

use crate::message::Message;
use crate::services;
use crate::state::workspace::{RepeatLoad, Workbench};
use crate::update::Ctx;
use crate::view;
use crate::view::send::{SendForm, SendMsg};

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

pub(crate) fn update(bench: &mut Workbench, msg: SendMsg, cx: Ctx) -> Task<Message> {
    match msg {
        SendMsg::ModeSelected(mode) => {
            bench.send_form.mode = mode;
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
            form.source = Some(outcome.prepared.source.clone());
            form.note = outcome.prepared.note.clone();
            form.encoding_used = outcome.prepared.encoding.clone();
            form.matching = outcome.matching;
            form.log(
                true,
                format!("sent {} bytes → {}", outcome.prepared.bytes.len(), form.key),
            );
            match &outcome.publication {
                Some(publication) => {
                    form.armed = true;
                    bench.publication = Some(RepeatLoad {
                        publication: publication.clone(),
                        bytes: Arc::new(outcome.prepared.bytes.clone()),
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
            bench.send_form.log(true, format!("sent {n} bytes → {key}"));
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
                    // A tombstone has no body provenance: a stale
                    // encoded/as-typed/raw line claiming a body shipped
                    // would be the pane's own O4 mistake.
                    form.source = None;
                    form.note = None;
                    let key = form.key.clone();
                    form.log(
                        true,
                        format!(
                            "retired {key} — an authoritative delete (RFC 04 §1.2), \
                             not an empty value"
                        ),
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
            // Classify as you type, through the same ladder the tree uses.
            bench.send_form.facts = (!k.trim().is_empty()).then(|| {
                zenkey_fleet::describe_key(cx.dep.base(), &k, cx.dep.slices.as_deref()).facts
            });
            // #158: the declared profile drives the picker until the user
            // takes it over — and stops driving it the moment they do.
            if !bench.send_form.qos_touched {
                let declared = view::send::declared_qos(bench.send_form.facts.as_ref());
                bench.send_form.qos =
                    view::send::QosChoice(declared.unwrap_or(zenkey::qos::QosProfile::Sampled));
            }
            bench.send_form.key = k;
            Task::none()
        }
        SendMsg::BodyChanged(b) => {
            bench.send_form.body = b;
            Task::none()
        }
        SendMsg::QosPicked(q) => {
            bench.send_form.qos = q;
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
        SendMsg::RawToggled(b) => {
            bench.send_form.raw = b;
            Task::none()
        }
        SendMsg::RepeatToggled(b) => {
            bench.send_form.repeat = b;
            // Turning repeat off mid-stream stops it, rather than leaving
            // an armed publication with the checkbox saying otherwise.
            if !b && bench.publication.is_some() {
                return update(bench, SendMsg::Stop, cx);
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
            let Some(session) = cx.dep.session.clone() else {
                return Task::none();
            };
            let key = bench.send_form.key.trim().to_string();
            if key.is_empty() {
                return Task::none();
            }
            // Same stop-first discipline as Send: a retire must not race
            // an armed publication on the key.
            let stop = update(bench, SendMsg::Stop, cx);
            // The engine is the judge (check_retire, RFC 04 §1.2 v1.12);
            // the pane's checkbox only arms the force.
            if let Err(e) = zenkey_fleet::check_retire(
                cx.dep.base(),
                &key,
                cx.dep.slices.as_deref(),
                bench.send_form.retire_i_know,
            ) {
                let e = crate::services::ServiceError::of(e);
                bench.send_form.log(false, format!("refused: {e}"));
                bench.send_form.error = Some(e);
                return stop;
            }
            bench.send_form.in_flight = true;
            bench.send_form.error = None;
            let send = services::write::retire(session, key);
            Task::batch([stop, send])
        }
        SendMsg::Send => {
            let (Some(session), Some(store)) =
                (cx.dep.session.clone(), cx.dep.schema_store.clone())
            else {
                return Task::none();
            };
            // A new send replaces any armed publication: two publishers on
            // one key from one pane would double every sample.
            let stop = update(bench, SendMsg::Stop, cx);
            let form = &bench.send_form;
            let key = form.key.trim().to_string();
            let body = form.body.clone().into_bytes();
            let qos = form.qos.0;
            let encoding = form.encoding.trim().to_string();
            let mode = if form.raw {
                zenkey_fleet::PrepareMode::Raw
            } else {
                zenkey_fleet::PrepareMode::Encode
            };
            let base = cx.dep.base().to_string();
            let slices = cx.dep.slices.clone();
            let repeat = form.repeat;
            // Verbatim, never schema-encoded (#117); empty = none.
            let attachment: Option<Arc<Vec<u8>>> = (!form.attachment.is_empty())
                .then(|| Arc::new(form.attachment.clone().into_bytes()));
            bench.send_form.in_flight = true;
            bench.send_form.error = None;
            let send = services::write::publish(services::write::Publish {
                session,
                store,
                slices,
                base,
                key,
                encoding,
                body,
                mode,
                qos,
                attachment,
                repeat,
            });
            Task::batch([stop, send])
        }

        // ── call mode ───────────────────────────────────────────────────
        SendMsg::Done(outcome) => {
            // No base guard needed (#109 audit): a call reply is the
            // answer to the exact question the user pressed the button
            // for, addressed by a full wire key naming its base — user
            // output, not projected deployment state.
            let form = &mut bench.send_form;
            form.in_flight = false;
            form.outcome = Some(outcome.map(|r| (*r).clone()));
            Task::none()
        }
        SendMsg::ProducerPicked(p) => {
            bench.send_form.producer = Some(p);
            bench.send_form.procedure = None;
            Task::none()
        }
        SendMsg::ProcedurePicked(p) => {
            let form = &mut bench.send_form;
            form.procedure = Some(p.clone());
            // Scaffold from the served request schema (§6.4 item 3). Not
            // asked yet is `None`, and stays `None` until an answer — the
            // pane renders that as "not asked", never as "no fields".
            form.request_fields = None;
            let (Some(session), Some(store), Some(producer)) = (
                cx.dep.session.clone(),
                cx.dep.schema_store.clone(),
                form.producer.clone(),
            ) else {
                return Task::none();
            };
            // The declared request type, read off the same `service_info`
            // projection the pane renders (#234) — one derivation of "what
            // does this procedure take", not two.
            let Some(request) = cx
                .dep
                .slices
                .as_ref()
                .and_then(|s| s.service_info(&producer, Some(&p)).ok())
                .and_then(|i| i.procedures.into_iter().next())
                .and_then(|d| d.request)
            else {
                // No declared request type: there is nothing to scaffold,
                // and that is an answer.
                form.request_fields = Some(Vec::new());
                return Task::none();
            };
            services::value::request_schema(session, store, producer, request)
        }
        SendMsg::RequestSchema(fields) => {
            bench.send_form.request_fields = fields;
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
            let (Some(session), Some(producer), Some(procedure)) = (
                cx.dep.session.clone(),
                form.producer.clone(),
                form.procedure.clone(),
            ) else {
                return Task::none();
            };
            let target = form.target.clone();
            let params: Vec<String> = form
                .params
                .split(';')
                .filter(|p| !p.trim().is_empty())
                .map(str::to_string)
                .collect();
            let body = if form.body.trim().is_empty() {
                None
            } else {
                Some(form.body.clone().into_bytes())
            };
            // Verbatim beside the body — never schema-encoded (#126).
            let attachment = if form.attachment.trim().is_empty() {
                None
            } else {
                Some(form.attachment.clone().into_bytes())
            };
            let base = cx.dep.base().to_string();
            let timeout = cx.dep.timeout();
            let slices = cx.dep.slices.clone();
            form.in_flight = true;
            form.outcome = None;
            services::write::call(services::write::Call {
                session,
                base,
                target,
                producer,
                procedure,
                params,
                body,
                attachment,
                timeout,
                slices,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}

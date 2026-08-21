//! The publish pane (#60): the only place in the app that writes.
//!
//! `&mut Workbench` rather than `&mut PublishForm`, because an armed
//! publication is not view state — a `Publication` is a live bus declaration
//! and dropping it undeclares — so it lives beside the form rather than in it,
//! and the handler moves both.
//!
//! Everything that touches the bus goes through [`crate::services::write`],
//! which goes through the engine: `prepare_publish` for the body (#97's
//! encode ladder) and `declare_publication` for the write (P7 — a declared
//! publisher, never an ad-hoc put). No codec logic on this side of the seam.

use std::sync::Arc;

use iced::Task;

use crate::message::Message;
use crate::services;
use crate::state::workspace::{RepeatLoad, Workbench};
use crate::update::Ctx;
use crate::view;
use crate::view::publish::PublishMsg;

/// The publish pane (#60). Everything that touches the bus goes through
/// the engine: `prepare_publish` for the body (#97's encode ladder) and
/// `declare_publication` for the write (P7 — a declared publisher, never
/// an ad-hoc put). No codec logic may live on this side of the seam.
pub(crate) fn update(bench: &mut Workbench, msg: PublishMsg, cx: Ctx) -> Task<Message> {
    match msg {
        PublishMsg::Ready(Ok(outcome)) => {
            // No base guard needed (#109 audit): publishing is
            // user-initiated *output* on a full wire key, not observed
            // evidence. (A publication straddling a context switch rides
            // the old session's Arc until stopped — a session-lifetime
            // question, out of #109's scope.)
            let form = &mut bench.publish_form;
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
        PublishMsg::Ready(Err(e)) => {
            let form = &mut bench.publish_form;
            form.in_flight = false;
            form.armed = false;
            form.error = Some(e.clone());
            form.log(false, format!("refused: {e}"));
            bench.publication = None;
            Task::none()
        }
        PublishMsg::Tick => {
            let Some(load) = bench.publication.as_ref() else {
                return Task::none();
            };
            let publication = load.publication.clone();
            let bytes = load.bytes.clone();
            let attachment = load.attachment.clone();
            services::write::repeat(publication, bytes, attachment)
        }
        PublishMsg::Sent(Ok(n)) => {
            let key = bench.publish_form.key.clone();
            bench
                .publish_form
                .log(true, format!("sent {n} bytes → {key}"));
            Task::none()
        }
        PublishMsg::Sent(Err(e)) => {
            // A failed repeat disarms: a stream that silently stopped
            // working would keep claiming it was publishing.
            bench.publish_form.log(false, format!("send failed: {e}"));
            bench.publish_form.armed = false;
            bench.publication = None;
            Task::none()
        }
        PublishMsg::Retired(result) => {
            let form = &mut bench.publish_form;
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
        PublishMsg::Stopped(result) => {
            match result {
                Ok(()) => bench
                    .publish_form
                    .log(true, "stopped — publication undeclared"),
                Err(e) => bench
                    .publish_form
                    .log(false, format!("undeclare failed: {e}")),
            }
            Task::none()
        }
        PublishMsg::KeyChanged(k) => {
            // Classify as you type, through the same ladder the tree uses.
            bench.publish_form.facts = (!k.trim().is_empty()).then(|| {
                zenkey_fleet::describe_key(cx.dep.base(), &k, cx.dep.slices.as_deref()).facts
            });
            // #158: the declared profile drives the picker until the user
            // takes it over — and stops driving it the moment they do.
            if !bench.publish_form.qos_touched {
                let declared = view::publish::declared_qos(bench.publish_form.facts.as_ref());
                bench.publish_form.qos =
                    view::publish::QosChoice(declared.unwrap_or(zenkey::qos::QosProfile::Sampled));
            }
            bench.publish_form.key = k;
            Task::none()
        }
        PublishMsg::BodyChanged(b) => {
            bench.publish_form.body = b;
            Task::none()
        }
        PublishMsg::QosPicked(q) => {
            bench.publish_form.qos = q;
            bench.publish_form.qos_touched = true;
            Task::none()
        }
        PublishMsg::EncodingChanged(e) => {
            bench.publish_form.encoding = e;
            Task::none()
        }
        PublishMsg::AttachmentChanged(a) => {
            bench.publish_form.attachment = a;
            Task::none()
        }
        PublishMsg::RawToggled(b) => {
            bench.publish_form.raw = b;
            Task::none()
        }
        PublishMsg::RepeatToggled(b) => {
            bench.publish_form.repeat = b;
            // Turning repeat off mid-stream stops it, rather than leaving
            // an armed publication with the checkbox saying otherwise.
            if !b && bench.publication.is_some() {
                return update(bench, PublishMsg::Stop, cx);
            }
            Task::none()
        }
        PublishMsg::IntervalChanged(i) => {
            bench.publish_form.interval = i;
            Task::none()
        }
        PublishMsg::Stop => {
            bench.publish_form.armed = false;
            let Some(RepeatLoad { publication, .. }) = bench.publication.take() else {
                return Task::none();
            };
            // Acknowledged undeclare when we hold the last reference; a
            // drop would undeclare too, but silently.
            match Arc::try_unwrap(publication) {
                Ok(p) => services::write::undeclare(p),
                Err(_) => Task::none(),
            }
        }
        PublishMsg::RetireIKnowToggled(b) => {
            bench.publish_form.retire_i_know = b;
            Task::none()
        }
        PublishMsg::Retire => {
            let Some(session) = cx.dep.session.clone() else {
                return Task::none();
            };
            let key = bench.publish_form.key.trim().to_string();
            if key.is_empty() {
                return Task::none();
            }
            // Same stop-first discipline as Send: a retire must not race
            // an armed publication on the key.
            let stop = update(bench, PublishMsg::Stop, cx);
            // The engine is the judge (check_retire, RFC 04 §1.2 v1.12);
            // the pane's checkbox only arms the force.
            if let Err(e) = zenkey_fleet::check_retire(
                cx.dep.base(),
                &key,
                cx.dep.slices.as_deref(),
                bench.publish_form.retire_i_know,
            ) {
                let e = e.to_string();
                bench.publish_form.error = Some(e.clone());
                bench.publish_form.log(false, format!("refused: {e}"));
                return stop;
            }
            bench.publish_form.in_flight = true;
            bench.publish_form.error = None;
            let send = services::write::retire(session, key);
            Task::batch([stop, send])
        }
        PublishMsg::Send => {
            let (Some(session), Some(store)) =
                (cx.dep.session.clone(), cx.dep.schema_store.clone())
            else {
                return Task::none();
            };
            // A new send replaces any armed publication: two publishers on
            // one key from one pane would double every sample.
            let stop = update(bench, PublishMsg::Stop, cx);
            let form = &bench.publish_form;
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
            bench.publish_form.in_flight = true;
            bench.publish_form.error = None;
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
    }
}

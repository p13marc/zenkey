//! The call pane (#66): one RPC, and the schema that scaffolds its request.
//!
//! `(&mut CallForm, Ctx)` — a form and four reads. The narrowest handler in
//! the app, and the shape #180 needs to dock a pane.

use iced::Task;

use crate::message::Message;
use crate::services;
use crate::update::Ctx;
use crate::view::call::{CallForm, CallMsg};

pub(crate) fn update(form: &mut CallForm, msg: CallMsg, cx: Ctx) -> Task<Message> {
    match msg {
        CallMsg::Done(outcome) => {
            // No base guard needed (#109 audit): a call reply is the
            // answer to the exact question the user pressed the button
            // for, addressed by a full wire key naming its base — user
            // output, not projected deployment state.
            form.in_flight = false;
            form.outcome = Some(outcome.map(|r| (*r).clone()).map_err(|e| e.to_string()));
            Task::none()
        }
        CallMsg::ProducerPicked(p) => {
            form.producer = Some(p);
            form.procedure = None;
            Task::none()
        }
        CallMsg::ProcedurePicked(p) => {
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
            let Some(request) = cx
                .dep
                .slices
                .as_ref()
                .and_then(|s| s.get(&producer))
                .and_then(|s| s.procedures.iter().find(|d| d.path == p))
                .and_then(|d| d.request.clone())
            else {
                // No declared request type: there is nothing to scaffold,
                // and that is an answer.
                form.request_fields = Some(Vec::new());
                return Task::none();
            };
            services::value::request_schema(session, store, producer, request)
        }
        CallMsg::RequestSchema(fields) => {
            form.request_fields = fields;
            Task::none()
        }
        CallMsg::ScaffoldBody => {
            if let Some(body) = form.scaffold() {
                form.body = body;
            }
            Task::none()
        }
        CallMsg::TargetChanged(t) => {
            form.target = t;
            Task::none()
        }
        CallMsg::ParamsChanged(t) => {
            form.params = t;
            Task::none()
        }
        CallMsg::BodyChanged(t) => {
            form.body = t;
            Task::none()
        }
        CallMsg::AttachmentChanged(t) => {
            form.attachment = t;
            Task::none()
        }
        CallMsg::Submit => {
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
            services::write::call(
                session, base, target, producer, procedure, params, body, attachment, timeout,
                slices,
            )
        }
    }
}

//! Writing to the bus: the call pane's request and the publish pane's stream
//! (#175).
//!
//! Everything here goes through the engine — `prepare_publish` for the body
//! (#97's encode ladder) and `declare_publication` for the write (P7: a
//! declared publisher, never an ad-hoc put). No codec logic may live on this
//! side of the seam, which is easier to hold now that the seam is a module
//! boundary rather than a comment.

use std::sync::Arc;

use iced::Task;
use zenkey::qos::QosProfile;
use zenkey_fleet::{Publication, SliceSet};

use crate::message::{Message, PaneMsg, PublishOutcome};
use crate::view::call::CallMsg;
use crate::view::publish::PublishMsg;

/// One RPC, whose target the engine parses (a bad target is an answer, not a
/// panic).
#[allow(clippy::too_many_arguments)] // Nine of these are the call's own form.
pub fn call(
    session: zenoh::Session,
    base: String,
    target: String,
    producer: String,
    procedure: String,
    params: Vec<String>,
    body: Option<Vec<u8>>,
    attachment: Option<Vec<u8>>,
    timeout: std::time::Duration,
    slices: Option<Arc<SliceSet>>,
) -> Task<Message> {
    Task::perform(
        async move {
            let target = zenkey_fleet::CallTarget::parse(&target).map_err(|e| e.to_string())?;
            zenkey_fleet::call(
                &session,
                &base,
                &target,
                &producer,
                &procedure,
                &params,
                body,
                attachment,
                timeout,
                slices.as_deref(),
            )
            .await
            .map(Arc::new)
            .map_err(|e| e.to_string())
        },
        |r| Message::Pane(PaneMsg::Call(CallMsg::Done(r))),
    )
}

/// What one `Send` needs, gathered from the form before the future starts.
pub struct Publish {
    pub session: zenoh::Session,
    pub store: Arc<zenkey_fleet::decode::SchemaStore>,
    pub slices: Option<Arc<SliceSet>>,
    pub base: String,
    pub key: String,
    pub encoding: String,
    pub body: Vec<u8>,
    pub mode: zenkey_fleet::PrepareMode,
    pub qos: QosProfile,
    pub attachment: Option<Arc<Vec<u8>>>,
    pub repeat: bool,
}

/// Prepare, declare, send — and hand the declaration back only if it repeats.
pub fn publish(p: Publish) -> Task<Message> {
    Task::perform(
        async move {
            let Publish {
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
            } = p;
            let prepared = zenkey_fleet::prepare_publish(
                &session,
                &store,
                slices.as_deref(),
                &base,
                &key,
                (!encoding.is_empty()).then_some(encoding.as_str()),
                &body,
                mode,
            )
            .await
            .map_err(|e| e.to_string())?;
            let publication = zenkey_fleet::declare_publication(
                &session,
                &key,
                qos,
                prepared.encoding.as_deref(),
            )
            .await
            .map_err(|e| e.to_string())?;
            publication
                .send(
                    prepared.bytes.clone(),
                    attachment.as_ref().map(|a| a.as_ref().clone()),
                )
                .await
                .map_err(|e| e.to_string())?;
            // The badge is a routing fact about this publisher and nothing
            // else; an error asking is "not asked" (O4), never "nobody
            // listens".
            let matching = publication.matching_status().await.ok();
            // A one-shot undeclares here, acknowledged, exactly as
            // `zenctl topic pub` does; only a repeating publish hands the
            // declaration back to be held.
            let publication = if repeat {
                Some(Arc::new(publication))
            } else {
                publication.undeclare().await.map_err(|e| e.to_string())?;
                None
            };
            Ok(Arc::new(PublishOutcome {
                prepared,
                publication,
                matching,
                attachment,
            }))
        },
        |r| Message::Pane(PaneMsg::Publish(PublishMsg::Ready(r))),
    )
}

/// One beat of an armed repeating publish.
pub fn repeat(
    publication: Arc<Publication>,
    bytes: Arc<Vec<u8>>,
    attachment: Option<Arc<Vec<u8>>>,
) -> Task<Message> {
    Task::perform(
        async move {
            publication
                .send(
                    bytes.as_ref().clone(),
                    attachment.as_ref().map(|a| a.as_ref().clone()),
                )
                .await
                .map(|()| bytes.len())
                .map_err(|e| e.to_string())
        },
        |r| Message::Pane(PaneMsg::Publish(PublishMsg::Sent(r))),
    )
}

/// Acknowledged undeclare. A drop would undeclare too, but silently.
pub fn undeclare(publication: Publication) -> Task<Message> {
    Task::perform(
        async move { publication.undeclare().await.map_err(|e| e.to_string()) },
        |r| Message::Pane(PaneMsg::Publish(PublishMsg::Stopped(r))),
    )
}

/// Retire a key: an authoritative delete (RFC 04 §1.2), on the reliable
/// profile, like `zenctl topic retire`. The engine has already judged whether
/// this key may be retired; this is the write.
pub fn retire(session: zenoh::Session, key: String) -> Task<Message> {
    Task::perform(
        async move {
            let publication =
                zenkey_fleet::declare_publication(&session, &key, QosProfile::Transition, None)
                    .await
                    .map_err(|e| e.to_string())?;
            let matching = publication.matching_status().await.ok();
            publication.retire().await.map_err(|e| e.to_string())?;
            publication.undeclare().await.map_err(|e| e.to_string())?;
            Ok(matching)
        },
        |r| Message::Pane(PaneMsg::Publish(PublishMsg::Retired(r))),
    )
}

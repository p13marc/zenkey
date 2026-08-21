//! The media viewer (#69): the laziest pane in the app.
//!
//! It subscribes on an explicit "view" and never for merely opening the pane —
//! the RFC 07 §1 plane deserves that posture, and the release is in the same
//! handler so an unviewed stream costs zero subscriptions.

use iced::Task;

use crate::message::Message;
use crate::nodes::NodeRoster;
use crate::services;
use crate::update::Ctx;
use crate::view::media::{MediaMsg, MediaState, Viewing};

/// The media viewer (issue #69): subscribe on view, release on stop,
/// and the key is built in exactly one place — `scope::media_key`,
/// which refuses wildcards and unfilled placeholders (RFC 07 §1).
pub(crate) fn update(
    media: &mut MediaState,
    roster: &NodeRoster,
    msg: MediaMsg,
    cx: Ctx,
) -> Task<Message> {
    match msg {
        MediaMsg::OriginChanged(s) => {
            media.origin = s;
            Task::none()
        }
        MediaMsg::ProducerChanged(s) => {
            media.producer = s;
            Task::none()
        }
        MediaMsg::SubpathChanged(s) => {
            media.subpath = s;
            Task::none()
        }
        MediaMsg::DeclPicked { producer, path } => {
            media.producer = producer;
            media.subpath = path;
            // A convenience, not a guess: if exactly one origin is on
            // the roster, prefill it; otherwise the operator names one.
            if media.origin.is_empty() {
                let hosts: Vec<&String> = roster
                    .iter()
                    .map(|(origin, _)| origin)
                    .filter(|o| o.starts_with("h-"))
                    .collect();
                if let [only] = hosts.as_slice() {
                    media.origin = (*only).clone();
                }
            }
            Task::none()
        }
        MediaMsg::View => {
            let Some(monitor) = cx.obs.monitor.clone() else {
                media.error = Some("no session — connect first".into());
                return Task::none();
            };
            let key = match crate::scope::media_key(
                cx.dep.base(),
                media.origin.trim(),
                media.producer.trim(),
                media.subpath.trim(),
            ) {
                Ok(k) => k,
                Err(e) => {
                    media.error = Some(e);
                    return Task::none();
                }
            };
            media.error = None;
            // One stream at a time: release the previous watch first.
            let release = stop_watch(media, cx);
            media.viewing = Some(Viewing::new(key.clone()));
            let declare = services::watch::media(monitor, key);
            Task::batch([release, declare])
        }
        MediaMsg::Watched(Ok(id)) => {
            if let Some(v) = &mut media.viewing {
                v.watch = Some(id);
            }
            Task::none()
        }
        MediaMsg::Watched(Err(e)) => {
            media.error = Some(e);
            media.viewing = None;
            Task::none()
        }
        MediaMsg::Stop => {
            let release = stop_watch(media, cx);
            media.viewing = None;
            release
        }
        MediaMsg::Stopped => Task::none(),
    }
}

/// Release the media watch, if one is declared — the "unviewed streams
/// cost zero subscriptions" half of #69's contract.
fn stop_watch(media: &mut MediaState, cx: Ctx) -> Task<Message> {
    let (Some(monitor), Some(id)) = (
        cx.obs.monitor.clone(),
        media.viewing.as_ref().and_then(|v| v.watch),
    ) else {
        return Task::none();
    };
    services::watch::release_media(monitor, id)
}

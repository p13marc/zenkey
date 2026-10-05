//! The Config tool's handler (#481): the form's edits, and the calls it
//! sends — through the engine's `call`, the path `zenctl config` takes.

use std::time::Instant;

use iced::Task;

use crate::configure::{ConfigAct, ConfigForm, ConfigRead, classify};
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
        ConfigMsg::Answered(act, result) => {
            form.in_flight = None;
            match act {
                ConfigAct::Read => {
                    form.read = Some(ConfigRead {
                        reply: classify(&result),
                        target: form.target.clone(),
                        at: Instant::now(),
                    });
                }
            }
            Task::none()
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
    let t = &form.target;
    services::write::call_then(
        services::write::Call {
            session,
            base: cx.dep.base().to_string(),
            target: t.origin.trim().to_string(),
            producer: t.producer.trim().to_string(),
            procedure: t.read_path(),
            params: Vec::new(),
            body: None,
            attachment: None,
            timeout: cx.dep.timeout(),
            // The convention's own procedures carry their kind (RFC 05 §2.1,
            // v1.48): a read needs no slice to be judged.
            slices: None,
        },
        |r| Message::Pane(PaneMsg::Config(ConfigMsg::Answered(ConfigAct::Read, r))),
    )
}

//! The admin & storage panel (#70, #131; zk2's since #614, FL9): one
//! explicit sweep, or nothing.

use iced::Task;

use crate::admin::AdminState;
use crate::message::Message;
use crate::services;
use crate::update::Ctx;
use crate::view::admin::AdminMsg;

pub(crate) fn update(admin: &mut AdminState, msg: AdminMsg, cx: Ctx) -> Task<Message> {
    match msg {
        AdminMsg::RawToggled(id) => {
            admin.toggle_raw(id);
            Task::none()
        }
        AdminMsg::Run => {
            if admin.in_flight {
                return Task::none();
            }
            let Some(raw) = cx.dep.raw.clone() else {
                admin.error = Some("no session — connect first".into());
                return Task::none();
            };
            admin.in_flight = true;
            admin.error = None;
            Task::batch([
                services::sweep::admin(
                    raw,
                    cx.dep.session.clone(),
                    cx.dep.namespace().to_string(),
                    cx.dep.timeout(),
                ),
                // The declared entities are the deployment's (the skeleton
                // reads them): asked again with the sweep, landing where
                // they always land.
                crate::update::bus::read_declared(cx.dep),
            ])
        }
        AdminMsg::Done(outcome) => {
            let namespace = cx.dep.namespace().to_string();
            admin.finish(outcome, &namespace);
            Task::none()
        }
    }
}

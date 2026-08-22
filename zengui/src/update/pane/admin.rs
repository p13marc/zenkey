//! The admin & storage panel (#70, #131): one explicit sweep, or nothing.
//!
//! It stopped taking `&mut RightPane` when #183 moved Echo into the Activity
//! dock: its producer filter used to land the user on the echo pane, and the
//! tree — which is what the filter actually drives — is always on screen.

use iced::Task;

use crate::admin::AdminState;
use crate::message::{Message, WorkspaceMsg};
use crate::services;
use crate::update::Ctx;
use crate::view::admin::AdminMsg;

pub(crate) fn update(admin: &mut AdminState, msg: AdminMsg, cx: Ctx) -> Task<Message> {
    match msg {
        AdminMsg::RawToggled(id) => {
            admin.toggle_raw(id);
            Task::none()
        }
        AdminMsg::FilterProducer(producer) => {
            // The tree already owns "show me this": reusing its search is
            // one behaviour, not two that can drift. It used to land on Echo
            // as well; Echo is a dock stream now (#183), and the tree is
            // always on screen, so there is nothing to switch to.
            Task::done(Message::Workspace(WorkspaceMsg::TreeSearchChanged(
                producer,
            )))
        }
        AdminMsg::Run => {
            if admin.in_flight {
                return Task::none();
            }
            let Some(session) = cx.dep.session.clone() else {
                admin.error = Some("no session — connect first".into());
                return Task::none();
            };
            admin.in_flight = true;
            admin.error = None;
            let base = cx.dep.base().to_string();
            let timeout = cx.dep.timeout();
            // The app's *resolved* slice set — bus, dirs or the union.
            //
            // Deliberately NOT the doctor's `SliceSet::from_dirs`: that one
            // is dirs-only because it diffs local against served, whereas
            // the coverage join asks "what does the registry say exists",
            // which is `bus.slice_set()` in zenctl. Copying the doctor here
            // would empty the coverage table on every bus-registry-only
            // deployment — an invisible, plausible-looking wrong answer.
            let slices = cx.dep.slices.clone();
            services::sweep::admin(session, base, slices, timeout)
        }
        AdminMsg::Done(outcome) => {
            let base = cx.dep.base().to_string();
            admin.finish(outcome, &base);
            Task::none()
        }
    }
}

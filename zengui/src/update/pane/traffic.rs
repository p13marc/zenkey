//! The Traffic tab's handler (#542).

use iced::Task;

use crate::message::Message;
use crate::traffic::TrafficState;
use crate::update::Ctx;
use crate::view::traffic::TrafficMsg;

/// A new sort re-ranks at once, from the snapshot already in hand — no tick
/// to wait for, no bus to ask.
pub(crate) fn update(state: &mut TrafficState, msg: TrafficMsg, cx: Ctx<'_>) -> Task<Message> {
    match msg {
        TrafficMsg::Sort(sort) => {
            state.sort = sort;
            state.rank(&cx.obs.observed);
            Task::none()
        }
    }
}

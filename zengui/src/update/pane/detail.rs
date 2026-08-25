//! The detail and history sections — the two with no state of their own.
//!
//! Both are windows onto one subject slot, so both take `&mut SubjectSlot`
//! and nothing else (#257: the routing from a `SlotId` to the slot happens
//! once, in [`super::update`]). That is the finding, not an economy: nine of
//! the eleven panes have state to dock when #180 arrives, and these two have
//! nothing.
//!
//! `Deployment` is here only for the chart rebuild, which needs the registry's
//! unit for the slot's key.

use iced::Task;

use crate::message::Message;
use crate::state::Deployment;
use crate::state::subject::SubjectSlot;
use crate::view::detail::DetailMsg;
use crate::view::history::HistoryMsg;

/// Choose which numeric leaf the sparkline plots (issue #64).
pub(crate) fn update(slot: &mut SubjectSlot, dep: &Deployment, msg: DetailMsg) -> Task<Message> {
    let DetailMsg::LeafSelected(path) = msg;
    slot.series_leaf = Some(path);
    slot.refresh_series(dep);
    Task::none()
}

/// The history section (#63). Both actions are about the recorder, and there
/// is nothing to do when the key is unselected — a recorder is created with
/// the selection and dropped with it.
pub(crate) fn history(slot: &mut SubjectSlot, msg: HistoryMsg) -> Task<Message> {
    // View-only state, and it is not the recorder's: a scroll offset survives
    // a `Clear` and a subject that has no recorder can still be scrolled to
    // the top.
    if let HistoryMsg::Scrolled(vp) = msg {
        slot.history_scroll = vp;
        return Task::none();
    }
    if let Some(rec) = slot.history.as_mut() {
        match msg {
            HistoryMsg::Select(seq) => rec.selected = Some(seq),
            HistoryMsg::Clear => {
                rec.ring.clear();
                rec.selected = None;
            }
            HistoryMsg::Scrolled(..) => unreachable!("handled above"),
        }
    }
    Task::none()
}

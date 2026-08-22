//! The detail and history panes — the two with no state of their own.
//!
//! Both are windows onto the selected key, so both take `&mut Subject` and
//! nothing else. That is the finding, not an economy: nine of the eleven panes
//! have state to dock when #180 arrives, and these two have nothing.
//!
//! `Deployment` is here only for the chart rebuild, which needs the registry's
//! unit for the selected key.

use iced::Task;

use crate::message::Message;
use crate::state::{Deployment, SubjectState};
use crate::view::detail::DetailMsg;
use crate::view::history::HistoryMsg;

/// Choose which numeric leaf the sparkline plots (issue #64).
pub(crate) fn update(sub: &mut SubjectState, dep: &Deployment, msg: DetailMsg) -> Task<Message> {
    let DetailMsg::LeafSelected(path) = msg;
    sub.series_leaf = Some(path);
    sub.refresh_series(dep);
    Task::none()
}

/// The history pane (#63). Both actions are about the recorder, and there is
/// nothing to do when the key is unselected — a recorder is created with the
/// selection and dropped with it.
pub(crate) fn history(sub: &mut SubjectState, msg: HistoryMsg) -> Task<Message> {
    if let Some(rec) = sub.history.as_mut() {
        match msg {
            HistoryMsg::Select(seq) => rec.selected = Some(seq),
            HistoryMsg::Clear => {
                rec.ring.clear();
                rec.selected = None;
            }
        }
    }
    Task::none()
}

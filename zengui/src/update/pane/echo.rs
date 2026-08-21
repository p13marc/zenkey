//! The echo pane (#72): the live line ring, and what is filtered out of it.
//!
//! Every action here is a *view* action — nothing subscribes, nothing
//! unsubscribes, and the ring is fed by the tick whether this pane is showing
//! or not. The one exception is the drill-through, which raises a selection
//! rather than being a second way to open the inspector.

use iced::Task;

use crate::message::{Message, RightPane, SubjectMsg};
use crate::state::workspace::EchoPane;
use crate::update::Ctx;
use crate::view;
use crate::view::echo::EchoMsg;

/// The echo pane (#72). Every action here is a *view* action: nothing
/// changes what the session subscribes to, which is what keeps "I filtered
/// the pane" and "I narrowed the bus" two different, visible things.
pub(crate) fn update(
    echo: &mut EchoPane,
    pane: &mut RightPane,
    msg: EchoMsg,
    cx: Ctx,
) -> Task<Message> {
    match msg {
        EchoMsg::FilterChanged(f) => {
            echo.echo_view.filter = f;
            Task::none()
        }
        EchoMsg::KeyFilterChanged(f) => {
            echo.echo_view.set_key_filter(f);
            Task::none()
        }
        EchoMsg::FollowToggled => {
            let seq = echo.echo.next_seq();
            if echo.echo_view.following {
                echo.echo_view.pause(seq);
            } else {
                echo.echo_view.resume(seq);
            }
            Task::none()
        }
        EchoMsg::Clear => {
            echo.echo.clear();
            Task::none()
        }
        EchoMsg::LineClicked(key) => {
            // Drill-through reuses the selection path rather than being a
            // second way to open the inspector.
            *pane = RightPane::Detail;
            Task::done(Message::Subject(SubjectMsg::SelectKey(Some(key))))
        }
        EchoMsg::Export => {
            let text = view::echo::export(
                &echo.echo,
                &echo.echo_view,
                cx.sub.selected.as_deref(),
                cx.dep.base(),
            );
            iced::clipboard::write(text)
        }
    }
}

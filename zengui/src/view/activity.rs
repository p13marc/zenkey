//! The bottom dock: the session's time-ordered streams (#183).
//!
//! Echo, the publish log, doctor results and the replay scrubber are all
//! *about the session* and none of them about the subject. They used to
//! compete for tab slots with panes that follow the subject, which is why
//! verifying a publish meant leaving the form to look at Echo — and losing the
//! form.
//!
//! ## Why tabs are honest here and were not up there
//!
//! The workspace's eleven tabs were eleven views of one thing, and #182
//! collapsed four of them into the Inspector for that reason. These four are
//! genuinely parallel streams: they run whether or not they are on screen, and
//! only one is read at a time because a human reads one at a time. That is
//! what a tab is for.
//!
//! ## The replay banner is not in here, deliberately
//!
//! Replay's *scrubber* and its capture line are a stream and live in the
//! Replay tab. The `REPLAY` banner stays between the location bar and the panes,
//! where it has always been, because a mode indicator you can put away behind
//! a tab is a mode indicator that can lie about what the panes are showing.

use iced::widget::{Column, column, row};
use iced::{Element, Length};
use zenkey_fleet::SliceSet;

use super::tokens::space;
use super::{doctor, echo, kit, replay, send};
use crate::echo::EchoRing;
use crate::message::{ActivityTab, Message, WorkspaceMsg};
use crate::state::workspace::{ActivityDock, ReplayMode};

/// Everything the dock's four streams need.
pub(crate) struct ActivityData<'a> {
    pub dock: &'a ActivityDock,
    pub echo: &'a EchoRing,
    pub echo_view: &'a echo::EchoView,
    pub echo_scroll: (f32, f32),
    /// The subject key, when Echo is pinned to follow it.
    pub follow: Option<&'a str>,
    pub next_seq: u64,
    pub publish: &'a send::SendForm,
    pub doctor: &'a crate::doctor::DoctorState,
    pub base: &'a str,
    pub replay: &'a ReplayMode,
    pub slices: Option<&'a SliceSet>,
    /// The monitor's retained window as of the last live tick (#217) —
    /// what the Replay tab's "scrub retained" affordance describes.
    /// `None` before a monitor exists, which is "not asked", not "empty"
    /// (O4).
    pub retention: Option<zenkey_fleet::RetentionStats>,
}

pub(crate) fn dock<'a>(d: ActivityData<'a>) -> Element<'a, Message> {
    // Putting the dock away is the grid's `×` since #180 — the strip is
    // only the stream switch now.
    let mut tabs = row![].spacing(space::XS);
    for t in ActivityTab::ALL {
        tabs = tabs.push(kit::tab(
            t.label(),
            d.dock.tab == t,
            Message::Workspace(WorkspaceMsg::ActivityTab(t)),
        ));
    }
    let strip = row![tabs]
        .spacing(space::SM)
        .align_y(iced::Alignment::Center);

    let body: Element<'a, Message> = match d.dock.tab {
        ActivityTab::Echo => {
            echo::section(d.echo, d.echo_view, d.follow, d.next_seq, d.echo_scroll).into()
        }
        ActivityTab::Publish => send::log_section(d.publish),
        ActivityTab::Doctor => doctor::section(d.doctor, d.base),
        ActivityTab::Replay => replay_stream(d.replay, d.slices, d.retention),
    };
    column![strip, body].spacing(space::SM).into()
}

/// The scrubber and the capture line — replay's *stream*, not its banner.
fn replay_stream<'a>(
    r: &'a ReplayMode,
    _slices: Option<&'a SliceSet>,
    retention: Option<zenkey_fleet::RetentionStats>,
) -> Element<'a, Message> {
    let mut col: Column<'a, Message> = column![].spacing(space::SM);
    if let Some(path) = &r.replay_open {
        col = col.push(replay::open_row(path));
        if let Some(note) = &r.replay_note {
            col = col.push(kit::muted(format!("could not open: {note}")));
        }
    }
    // A parse in flight is its own state (#255, RFC 09 §5.1 O4): "loading"
    // is neither "no file open" nor a hung window, and the tab says which.
    if let Some(path) = &r.replay_loading {
        col = col.push(kit::muted(replay::loading_note(path)));
    }
    match &r.replay {
        Some(state) => col = col.push(replay::scrubber(state)),
        None if r.replay_open.is_none() && r.replay_loading.is_none() => {
            col = col.push(kit::muted(
                "no file open — the location bar's \"replay…\" opens a .zrec, \
                 and \"record\" writes one from the current watches",
            ));
        }
        None => {}
    }
    // The live/retained toggle (#217): only offered while live — inside a
    // replay the banner's exit is the one way out — and only once a monitor
    // exists to have retained anything.
    if r.replay.is_none()
        && let Some(taken) = retention
    {
        col = col.push(
            row![
                kit::action(kit::caption("scrub retained window"))
                    .on_press(Message::Workspace(WorkspaceMsg::Replay(
                        replay::ReplayMsg::RetainedToggled,
                    )))
                    .padding(4),
                kit::muted(format!(
                    "holds {:.1}s of watched traffic · budget {}",
                    taken.span.as_secs_f64(),
                    replay::budget_label(taken.budget),
                )),
            ]
            .spacing(space::SM)
            .align_y(iced::Alignment::Center),
        );
    }
    if let Some(rec) = &r.recording {
        col = col.push(kit::muted(format!(
            "● recording current watches to {} — the location bar's 'stop recording' finishes the file",
            rec.path
        )));
    }
    if let Some(done) = &r.recorded {
        col = col.push(kit::muted(match done {
            Ok((samples, dropped, path)) => format!(
                "recorded {samples} sample(s) to {path} ({dropped} dropped — in-file ledger)"
            ),
            Err(e) => format!("recording failed: {e}"),
        }));
    }
    iced::widget::scrollable(col).height(Length::Fill).into()
}

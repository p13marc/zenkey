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

use super::tokens::Spacing;
use super::{doctor, echo, kit, replay, send};
use crate::echo::EchoRing;
use crate::message::{ActivityTab, Message, WorkspaceMsg};
use crate::state::workspace::{ActivityDock, ReplayMode};

/// Everything the dock's four streams need.
pub(crate) struct ActivityData<'a> {
    pub dock: &'a ActivityDock,
    pub echo: &'a EchoRing,
    pub echo_view: &'a echo::EchoView,
    pub echo_scroll: crate::view::kit::Viewport,
    /// The subject key, when Echo is pinned to follow it.
    pub follow: Option<&'a str>,
    /// The payload-verdict cache (#164) — echo rows look their badges up in
    /// it; nothing in the dock decodes.
    pub verdicts: &'a crate::verdict::VerdictCache,
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
    /// The theme the window renders in — echo's coloured previews resolve
    /// their syntax colours from it (#538).
    pub theme: crate::prefs::ThemeChoice,
    /// What the Traffic tab reads (#542), assembled by the grid.
    pub traffic: super::traffic::TrafficData<'a>,
    /// The dock's resolved spacing grid (#192).
    pub sp: Spacing,
}

pub(crate) fn dock<'a>(d: ActivityData<'a>) -> Element<'a, Message> {
    // Putting the dock away is the grid's `×` since #180 — the strip is
    // only the stream switch now.
    // The stream switch (#537): a segmented control, each stream with what
    // it holds. A count is a count that was taken — the doctor's pill is
    // absent until a run has answered (never-run is not 0, O4), "…" while
    // one is in flight; replay's says "rec" only while a capture runs.
    let strip = kit::segmented(
        ActivityTab::ALL
            .into_iter()
            .map(|t| kit::Segment {
                value: t,
                label: t.label().to_string(),
                icon: Some(tab_icon(t)),
                count: match t {
                    ActivityTab::Echo => Some(d.echo.len().to_string()),
                    ActivityTab::Publish => Some(d.publish.log.len().to_string()),
                    ActivityTab::Doctor if d.doctor.in_flight => Some("…".to_string()),
                    ActivityTab::Doctor => d
                        .doctor
                        .current
                        .as_ref()
                        .map(|r| r.findings.len().to_string()),
                    ActivityTab::Replay => d.replay.recording.is_some().then(|| "rec".to_string()),
                    ActivityTab::Traffic => None,
                },
                tip: None,
            })
            .collect(),
        Some(d.dock.tab),
        |t| Message::Workspace(WorkspaceMsg::ActivityTab(t)),
    );

    let body: Element<'a, Message> = match d.dock.tab {
        ActivityTab::Echo => echo::section(
            d.echo,
            d.echo_view,
            d.follow,
            d.next_seq,
            d.echo_scroll,
            d.verdicts,
            d.theme,
            d.sp,
        )
        .into(),
        ActivityTab::Publish => send::log_section(d.publish, d.sp),
        ActivityTab::Doctor => doctor::section(d.doctor, d.base, d.sp),
        ActivityTab::Replay => replay_stream(d.replay, d.slices, d.retention, d.sp),
        ActivityTab::Traffic => super::traffic::section(d.traffic).into(),
    };
    column![strip, body].spacing(d.sp.sm).into()
}

/// The scrubber and the capture line — replay's *stream*, not its banner.
fn replay_stream<'a>(
    r: &'a ReplayMode,
    _slices: Option<&'a SliceSet>,
    retention: Option<zenkey_fleet::RetentionStats>,
    sp: Spacing,
) -> Element<'a, Message> {
    let mut col: Column<'a, Message> = column![].spacing(sp.sm);
    if let Some(path) = &r.replay_open {
        col = col.push(replay::open_row(path, sp));
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
        Some(state) => col = col.push(replay::scrubber(state, sp)),
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
                kit::secondary(kit::caption("scrub retained window"))
                    .on_press(Message::Workspace(WorkspaceMsg::Replay(
                        replay::ReplayMsg::RetainedToggled,
                    )))
                    .padding(sp.xs),
                kit::muted(format!(
                    "holds {:.1}s of watched traffic · budget {}",
                    taken.span.as_secs_f64(),
                    replay::budget_label(taken.budget),
                )),
            ]
            .spacing(sp.sm)
            .align_y(iced::Alignment::Center),
        );
    }
    // The loaded snapshot (#219): opened here because a `.zsnap` is the
    // `.zrec`'s sibling, and shown with its span because RFC 13 §4.4 makes
    // stating the span every rendering's obligation.
    if let Some(path) = &r.snapshot_open {
        col = col.push(replay::snapshot_open_row(path, sp));
        if let Some(note) = &r.snapshot_note {
            col = col.push(kit::muted(format!("could not open: {note}")));
        }
    }
    if let Some(path) = &r.snapshot_loading {
        col = col.push(kit::muted(format!(
            "loading {path}\u{2026} — parsing the snapshot"
        )));
    }
    match &r.snapshot {
        Some(s) => {
            col = col.push(
                row![
                    kit::muted(replay::snapshot_label(s)),
                    kit::secondary(kit::caption("close"))
                        .on_press(Message::Workspace(WorkspaceMsg::Replay(
                            replay::ReplayMsg::SnapshotClosed,
                        )))
                        .padding(sp.xs),
                ]
                .spacing(sp.sm)
                .align_y(iced::Alignment::Center),
            );
        }
        None if r.snapshot_open.is_none() && r.snapshot_loading.is_none() => {
            col = col.push(
                kit::secondary(kit::caption("open a .zsnap to compare against\u{2026}"))
                    .on_press(Message::Workspace(WorkspaceMsg::Replay(
                        replay::ReplayMsg::SnapshotOpenToggled,
                    )))
                    .padding(sp.xs),
            );
        }
        None => {}
    }
    if let Some(rec) = &r.recording {
        // The mode's status, not a hand-drawn ● (#544): the dot and the
        // word come from the same constructor as the app bar's.
        col = col.push(
            row![
                kit::status(crate::view::theme::Tone::Mode, "recording"),
                kit::muted(format!(
                    "current watches to {} — the location bar's 'stop recording' finishes the file",
                    rec.path
                )),
            ]
            .spacing(sp.sm)
            .align_y(iced::Alignment::Center),
        );
    }
    if let Some(done) = &r.recorded {
        col = col.push(kit::muted(match done {
            Ok(crate::view::replay::Recorded {
                samples,
                dropped,
                path,
            }) => format!(
                "recorded {samples} sample(s) to {path} ({dropped} dropped — in-file ledger)"
            ),
            Err(e) => format!("recording failed: {e}"),
        }));
    }
    iced::widget::scrollable(col).height(Length::Fill).into()
}

/// The icon a stream wears on the dock's tab strip — and on its palette
/// entry (#558), so the two spell the same glyph.
pub(crate) fn tab_icon(t: ActivityTab) -> kit::Icon {
    match t {
        ActivityTab::Echo => kit::Icon::Echo,
        ActivityTab::Publish => kit::Icon::Send,
        ActivityTab::Doctor => kit::Icon::Doctor,
        ActivityTab::Replay => kit::Icon::Replay,
        ActivityTab::Traffic => kit::Icon::Traffic,
    }
}

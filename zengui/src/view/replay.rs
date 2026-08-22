//! The replay surface (issue #74): an unmistakable banner, and the one
//! thing the CLI cannot do — a time scrubber.
//!
//! Mode honesty is the whole design: while a `.zrec` feeds the panes, the
//! banner names the file, its selectors, its base, when it was captured
//! and what the capture dropped (a replay is a partial view and says so —
//! RFC 09 §5.1 O6), and the live link is off. The scrubber's axis is the
//! **capture clock** (each row's `t`, the recording observer's arrival
//! offsets) and the label says so, because a consumer plotting a time axis
//! states which clock it plotted (RFC 09 §5.2).

use iced::widget::{button, pick_list, row, slider, text};
use iced::{Element, Length};

use super::kit;
use super::theme::colors;
use super::tokens::{font, space};
use crate::message::{Message, WorkspaceMsg};
use crate::replay::ReplayState;
use crate::state::workspace::ReplayMode;

/// Replay-mode interactions.
#[derive(Debug, Clone, PartialEq)]
pub enum ReplayMsg {
    /// The path input changed (the open row's text box).
    PathChanged(String),
    /// Load the typed path.
    Open,
    /// Show or hide the open row.
    OpenToggled,
    /// Play/pause.
    Toggled,
    /// The speed picker changed.
    SpeedSelected(Speed),
    /// The scrubber moved (an absolute instant on the capture clock, µs).
    Scrubbed(u64),
    /// Leave replay mode — the live link resumes.
    Exit,
    /// The play clock fired (subscription-driven while playing).
    Advance,
    /// Start or stop recording the current watches to a `.zrec`.
    RecordToggled,
    /// A recording finished (or failed): samples, drops, path — or why not.
    RecordFinished(Result<(u64, u64, String), String>),
}

fn msg(m: ReplayMsg) -> Message {
    Message::Workspace(WorkspaceMsg::Replay(m))
}

/// A pacing scale the picker can display.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Speed(pub f64);

impl std::fmt::Display for Speed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}x", self.0)
    }
}

/// The speeds on offer. A replay never runs unpaced — "as fast as
/// possible" is a bulk write, not a replay.
pub const SPEEDS: [Speed; 6] = [
    Speed(0.25),
    Speed(0.5),
    Speed(1.0),
    Speed(2.0),
    Speed(4.0),
    Speed(8.0),
];

/// The REPLAY banner plus the transport row. Rendered only in replay mode,
/// directly under the location bar — the panes below it are showing the file.
pub fn banner(state: &ReplayState) -> Element<'_, Message> {
    // The mode indicator is the loudest claim on this row: EMPHASIS, in the
    // danger tone — everything else in the banner is metadata about it.
    let title = kit::emphasis("REPLAY").style(|theme: &iced::Theme| text::Style {
        color: Some(colors(theme).danger()),
    });
    let what = kit::muted(format!(
        "{} — {} under base {:?}, captured {} · {} row(s)",
        state.path,
        state.header.selectors.join(" + "),
        state.header.base,
        state.header.captured_at,
        state.rows.len(),
    ));
    let mut meta = row![title, what].spacing(space::SM);
    if state.capture_dropped > 0 {
        meta = meta.push(
            kit::caption(format!(
                "capture dropped {} sample(s) — partial view",
                state.capture_dropped
            ))
            .style(|theme: &iced::Theme| text::Style {
                color: Some(colors(theme).warning()),
            }),
        );
    }
    if state.malformed > 0 {
        meta = meta.push(
            kit::caption(format!(
                "{} malformed row(s) skipped-and-counted",
                state.malformed
            ))
            .style(|theme: &iced::Theme| text::Style {
                color: Some(colors(theme).warning()),
            }),
        );
    }
    meta = meta.push(iced::widget::space::horizontal());
    meta = meta.push(kit::muted("live link off"));
    meta = meta.push(
        button(kit::caption("exit replay"))
            .on_press(msg(ReplayMsg::Exit))
            .padding(4),
    );

    iced::widget::column![meta].spacing(space::XS).into()
}

/// The transport: play/pause, speed, and the scrubber.
///
/// Split from [`banner`] for the Activity dock (#183). The banner is a *mode*
/// indicator and stays between the location bar and the panes, where it cannot be
/// put away; the transport is a stream control and lives in the dock's Replay
/// tab.
pub fn scrubber(state: &ReplayState) -> Element<'_, Message> {
    let (pos, span) = state.clock();
    let transport = row![
        button(kit::caption(if state.playing { "pause" } else { "play" }))
            .on_press(msg(ReplayMsg::Toggled))
            .padding(4),
        pick_list(SPEEDS, Some(Speed(state.speed)), |s| msg(
            ReplayMsg::SpeedSelected(s)
        ))
        .text_size(font::CAPTION),
        slider(
            0.0..=(state.span_us.max(1) as f64 / 1e6),
            state.position_us as f64 / 1e6,
            |secs: f64| msg(ReplayMsg::Scrubbed((secs * 1e6) as u64)),
        )
        .step(0.1)
        .width(Length::Fill),
        // The axis, named: `t` is the capture clock — the recording
        // observer's arrival offsets — not the publishers' HLC.
        kit::muted(format!("{pos:.1}s / {span:.1}s (capture clock t)")),
    ]
    .spacing(space::SM)
    .align_y(iced::Alignment::Center);

    transport.into()
}

/// The open row: a path box, shown on demand from the location bar.
pub fn open_row(path: &str) -> Element<'_, Message> {
    row![
        kit::caption("replay file"),
        iced::widget::text_input(".zrec path", path)
            .on_input(|s| msg(ReplayMsg::PathChanged(s)))
            .on_submit(msg(ReplayMsg::Open))
            .size(font::CAPTION)
            .width(Length::Fill),
        button(kit::caption("open"))
            .on_press(msg(ReplayMsg::Open))
            .padding(4),
        button(kit::caption("cancel"))
            .on_press(msg(ReplayMsg::OpenToggled))
            .padding(4),
    ]
    .spacing(space::SM)
    .align_y(iced::Alignment::Center)
    .into()
}

/// Everything replay mode puts between the location bar and the panes (#74).
///
/// Since #183 that is the banner and nothing else. The open row, the scrubber
/// and the capture line moved into the Activity dock's Replay tab, because
/// they are stream controls. The banner did not, because it is a *mode*
/// indicator, and one you can put away behind a tab is one that can lie about
/// what the panes are showing.
///
/// A `Vec` rather than one composed element: the shell stacks them into its
/// own column, and it is empty on the ordinary path.
pub(crate) fn surfaces(replay: &ReplayMode) -> Vec<Element<'_, Message>> {
    let mut out: Vec<Element<'_, Message>> = Vec::new();
    if let Some(state) = &replay.replay {
        out.push(banner(state));
    }
    out
}

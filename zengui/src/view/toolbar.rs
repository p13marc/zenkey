//! The one row across the top: what fleet, what scope, which pane, and the
//! window's own controls (#175).
//!
//! It reads four of the six sub-states and moves none, which is what a
//! toolbar is. Extracted from `app.rs` so the shell's `view` is the layout
//! and nothing else.

use iced::Element;
use iced::widget::{pick_list, row, text};

use crate::message::{ChromeMsg, DeploymentMsg, Message, RightPane, WorkspaceMsg};
use crate::scope::ScopePreset;
use crate::state::{Chrome, Deployment, Observation, Workspace};
use crate::view::replay::ReplayMsg;
use crate::view::tokens::space;
use crate::view::{kit, tokens};

pub(crate) fn strip<'a>(
    chrome: &'a Chrome,
    dep: &'a Deployment,
    obs: &'a Observation,
    work: &'a Workspace,
) -> Element<'a, Message> {
    // Both pickers borrow (#178). `pick_list` takes `L: Borrow<[T]>`, so a
    // slice is as good as a `Vec` — and the toolbar redraws at frame rate
    // while its options change on a discovery sweep, which is minutes
    // apart.
    let base_picker = pick_list(
        &dep.base_options[..],
        Some(dep.settings.base.clone()),
        |r| Message::Deployment(DeploymentMsg::BaseSelected(r)),
    )
    .placeholder("base")
    .text_size(tokens::font::CAPTION);

    /// The closed scope vocabulary, in menu order.
    const SCOPES: [ScopePreset; 5] = [
        ScopePreset::Everything,
        ScopePreset::Deployment,
        ScopePreset::Telemetry,
        ScopePreset::State,
        ScopePreset::Events,
    ];
    let scope_picker = pick_list(&SCOPES[..], Some(dep.settings.scope), |r| {
        Message::Deployment(DeploymentMsg::ScopeSelected(r))
    })
    .text_size(tokens::font::CAPTION);

    // Observation is opt-in and labelled by its cost (issue #85).
    let observing = !obs.scope_watches.is_empty();
    let observe = iced::widget::button(
        text(if observing {
            "stop observing scope"
        } else {
            "observe scope"
        })
        .size(tokens::font::CAPTION),
    )
    .on_press(Message::Deployment(DeploymentMsg::ScopeWatchToggled))
    .padding(4);

    row![
        text("base").size(tokens::font::CAPTION),
        base_picker,
        text("scope").size(tokens::font::CAPTION),
        scope_picker,
        observe,
        iced::widget::Row::from_iter(RightPane::ALL.into_iter().map(|p| {
            kit::tab(
                p.label(),
                work.right_pane == p,
                Message::Workspace(WorkspaceMsg::PaneSelected(p)),
            )
        }))
        .spacing(space::XS),
        kit::muted(dep.settings.scope.label()),
        // Capture and replay (#74): record writes the current watches
        // to a .zrec; replay feeds the panes from one.
        iced::widget::button(
            text(if work.replay.recording.is_some() {
                "stop recording"
            } else {
                "record"
            })
            .size(tokens::font::CAPTION)
        )
        .on_press(Message::Workspace(WorkspaceMsg::Replay(
            ReplayMsg::RecordToggled
        )))
        .padding(4),
        iced::widget::button(text("replay…").size(tokens::font::CAPTION))
            .on_press(Message::Workspace(WorkspaceMsg::Replay(
                ReplayMsg::OpenToggled
            )))
            .padding(4),
        iced::widget::space::horizontal(),
        // Window preferences (issue #73): the theme name is the button,
        // so the label says what you get rather than what you have.
        iced::widget::button(
            text(format!("theme: {}", chrome.prefs.theme.label())).size(tokens::font::CAPTION)
        )
        .on_press(Message::Chrome(ChromeMsg::Prefs(
            crate::message::PrefsMsg::ThemeToggled
        )))
        .padding(4),
        iced::widget::button(text("-").size(tokens::font::CAPTION))
            .on_press(Message::Chrome(ChromeMsg::Prefs(
                crate::message::PrefsMsg::ZoomOut
            )))
            .padding(4),
        iced::widget::button(
            text(format!("{}%", (chrome.prefs.zoom * 100.0).round() as i32))
                .size(tokens::font::CAPTION)
        )
        .on_press(Message::Chrome(ChromeMsg::Prefs(
            crate::message::PrefsMsg::ZoomReset
        )))
        .padding(4),
        iced::widget::button(text("+").size(tokens::font::CAPTION))
            .on_press(Message::Chrome(ChromeMsg::Prefs(
                crate::message::PrefsMsg::ZoomIn
            )))
            .padding(4),
        iced::widget::button(text("reconnect").size(tokens::font::CAPTION))
            .on_press(Message::Deployment(DeploymentMsg::Reconnect))
            .padding(4),
    ]
    .spacing(space::SM)
    .align_y(iced::Alignment::Center)
    .into()
}

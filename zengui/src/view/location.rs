//! The location bar (#185): context ▸ base ▸ scope ▸ key.
//!
//! The toolbar it replaces was a strip of unrelated controls — pickers, tabs,
//! zoom — with no sense of place: in a 40,000-row tree there was no answer to
//! "where am I". Every segment of the bar is simultaneously **display and
//! control**: the context chip opens the Connect overlay, the base picker is a
//! picker, the scope picker is a picker, and the key's ancestor chunks are
//! buttons that select that subtree ([`Subject::Prefix`]) — never a second
//! implementation, always the message the UI already sends.
//!
//! The final chunk is the window's one `TITLE` (#191): the subject's role
//! moved here from the Inspector's subject line, which now carries the key at
//! `EMPHASIS` — exactly one TITLE per window, and it answers "where am I".
//!
//! [`bar`] reads five of the six sub-states and moves none, like the toolbar
//! before it. [`breadcrumb`] takes plain data ([`LocationData`]) so
//! `tests/panes.rs` renders it headlessly, the same seam as
//! [`super::status::Status`].

use iced::Element;
use iced::widget::{button, column, pick_list, row};

use crate::config::BaseChoice;
use crate::message::{
    ChromeMsg, DeploymentMsg, Message, RightPane, Subject, SubjectMsg, WorkspaceMsg,
};
use crate::scope::ScopePreset;
use crate::state::{Chrome, Deployment, Observation, SubjectState, Workspace};
use crate::view::palette::{Overlay, PaletteMsg};
use crate::view::replay::ReplayMsg;
use crate::view::tokens::space;
use crate::view::{kit, tokens};

/// Everything the breadcrumb shows, as plain data.
pub struct LocationData<'a> {
    /// The active named context, when one is. `None` renders as "no context"
    /// — a state, not an omission — and the chip opens Connect either way.
    pub context: Option<&'a str>,
    /// The deployment base in force; `""` is the bus root.
    pub base: &'a str,
    /// The picker's options ([`crate::update::bus::rebuild_base_options`]).
    pub base_options: &'a [BaseChoice],
    pub scope: ScopePreset,
    /// Whether the scope's watches are live (#85's opt-in observation).
    pub observing: bool,
    pub subject: &'a Subject,
}

/// One breadcrumb segment: what it says, and what clicking it selects.
///
/// `select` is `None` for the final segment — the place the window already is
/// — and `Some(Subject::Prefix(..))` for every ancestor.
pub struct Segment {
    pub label: String,
    pub select: Option<Subject>,
}

/// The subject as breadcrumb segments. Pure, so the model is testable without
/// a renderer.
///
/// A [`Subject::Origin`] has no display path of its own; it is placed through
/// [`crate::scope::origin_display_path`] — the same #61 click-through path the
/// tree uses — so its ancestors are real tree prefixes.
pub fn segments(subject: &Subject, base: &str) -> Vec<Segment> {
    let path = match subject {
        Subject::None => return Vec::new(),
        Subject::Key(p) | Subject::Prefix(p) => p.clone(),
        Subject::Origin(o) => crate::scope::origin_display_path(base, o),
    };
    let chunks: Vec<&str> = path.split('/').collect();
    let last = chunks.len() - 1;
    let mut out = Vec::with_capacity(chunks.len());
    let mut prefix = String::new();
    for (i, chunk) in chunks.iter().enumerate() {
        if !prefix.is_empty() {
            prefix.push('/');
        }
        prefix.push_str(chunk);
        out.push(Segment {
            label: (*chunk).to_string(),
            // An ancestor selects its subtree — a Prefix, never a Key: a
            // breadcrumb click must not put a fetch on the wire (#85).
            select: (i < last).then(|| Subject::Prefix(prefix.clone())),
        });
    }
    out
}

/// The breadcrumb itself: context ▸ base ▸ scope ▸ key.
pub fn breadcrumb(d: LocationData<'_>) -> Element<'_, Message> {
    let context_chip = button(kit::caption(match d.context {
        Some(name) => format!("context: {name}"),
        None => "no context".to_string(),
    }))
    .on_press(Message::Chrome(ChromeMsg::Palette(PaletteMsg::Open(
        Overlay::Connect,
    ))))
    .padding(4);

    // The picker displays each base by its label, so the empty base reads as
    // the bus-root deployment it is, never as a blank row (#185).
    let base_picker = pick_list(d.base_options, Some(BaseChoice::new(d.base)), |c| {
        Message::Deployment(DeploymentMsg::BaseSelected(c.base))
    })
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
    let scope_picker = pick_list(&SCOPES[..], Some(d.scope), |s| {
        Message::Deployment(DeploymentMsg::ScopeSelected(s))
    })
    .text_size(tokens::font::CAPTION);

    // Observation is opt-in and labelled by its cost (issue #85); it rides
    // beside the scope it observes.
    let observe = button(kit::caption(if d.observing {
        "stop observing scope"
    } else {
        "observe scope"
    }))
    .on_press(Message::Deployment(DeploymentMsg::ScopeWatchToggled))
    .padding(4);

    let mut bar = row![
        context_chip,
        kit::muted("▸"),
        base_picker,
        kit::muted("▸"),
        scope_picker,
        observe,
        kit::muted("▸"),
    ]
    .spacing(space::SM)
    .align_y(iced::Alignment::Center);

    let segs = segments(d.subject, d.base);
    if segs.is_empty() {
        // An empty segment list is a state, and it says which one.
        bar = bar.push(kit::muted("nothing selected — pick a key in the tree"));
    } else {
        for (i, seg) in segs.into_iter().enumerate() {
            if i > 0 {
                bar = bar.push(kit::muted("/"));
            }
            bar = bar.push(match seg.select {
                Some(subtree) => Element::from(
                    button(kit::caption(seg.label).font(iced::Font::MONOSPACE))
                        .style(button::text)
                        .on_press(Message::Subject(SubjectMsg::Select(subtree)))
                        .padding(2),
                ),
                // The final chunk: where the window is — the one TITLE (#191).
                None => kit::title(seg.label).font(iced::Font::MONOSPACE).into(),
            });
        }
    }
    bar.into()
}

/// The whole top region: the breadcrumb, then the window's own controls.
///
/// It reads five of the six sub-states and moves none, which is what a
/// location bar is.
pub(crate) fn bar<'a>(
    chrome: &'a Chrome,
    dep: &'a Deployment,
    obs: &'a Observation,
    sub: &'a SubjectState,
    work: &'a Workspace,
) -> Element<'a, Message> {
    column![
        breadcrumb(LocationData {
            context: work.bench.context_form.active.as_deref(),
            base: dep.base(),
            base_options: &dep.base_options,
            scope: dep.settings.scope,
            observing: !obs.scope_watches.is_empty(),
            subject: &sub.current,
        }),
        controls(chrome, dep, work),
    ]
    .spacing(space::XS)
    .into()
}

/// The window's own controls: the pane strip, capture/replay, theme, zoom,
/// reconnect — everything that is about the window rather than the place.
fn controls<'a>(
    chrome: &'a Chrome,
    dep: &'a Deployment,
    work: &'a Workspace,
) -> Element<'a, Message> {
    row![
        iced::widget::Row::from_iter(RightPane::ALL.into_iter().map(|p| {
            kit::tab(
                p.label(),
                work.right_pane == p,
                Message::Workspace(WorkspaceMsg::PaneSelected(p)),
            )
        }))
        .spacing(space::XS),
        // The scope's honest coverage statement — "everything" is not
        // everything (RFC 03 §4 D2), and this is where it says so.
        kit::muted(dep.settings.scope.label()),
        // Capture and replay (#74): record writes the current watches to a
        // .zrec; replay feeds the panes from one.
        button(kit::caption(if work.replay.recording.is_some() {
            "stop recording"
        } else {
            "record"
        }))
        .on_press(Message::Workspace(WorkspaceMsg::Replay(
            ReplayMsg::RecordToggled
        )))
        .padding(4),
        button(kit::caption("replay…"))
            .on_press(Message::Workspace(WorkspaceMsg::Replay(
                ReplayMsg::OpenToggled
            )))
            .padding(4),
        iced::widget::space::horizontal(),
        // Window preferences (issue #73): the theme name is the button, so
        // the label says what you get rather than what you have.
        button(kit::caption(format!(
            "theme: {}",
            chrome.prefs.theme.label()
        )))
        .on_press(Message::Chrome(ChromeMsg::Prefs(
            crate::message::PrefsMsg::ThemeToggled
        )))
        .padding(4),
        button(kit::caption("-"))
            .on_press(Message::Chrome(ChromeMsg::Prefs(
                crate::message::PrefsMsg::ZoomOut
            )))
            .padding(4),
        button(kit::caption(format!(
            "{}%",
            (chrome.prefs.zoom * 100.0).round() as i32
        )))
        .on_press(Message::Chrome(ChromeMsg::Prefs(
            crate::message::PrefsMsg::ZoomReset
        )))
        .padding(4),
        button(kit::caption("+"))
            .on_press(Message::Chrome(ChromeMsg::Prefs(
                crate::message::PrefsMsg::ZoomIn
            )))
            .padding(4),
        button(kit::caption("reconnect"))
            .on_press(Message::Deployment(DeploymentMsg::Reconnect))
            .padding(4),
    ]
    .spacing(space::SM)
    .align_y(iced::Alignment::Center)
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The segment model: every ancestor selects its subtree, the final
    /// segment is the place itself, and nothing here is ever a `Key` — a
    /// breadcrumb click must not fetch (#85).
    #[test]
    fn ancestors_select_their_subtree_and_the_leaf_selects_nothing() {
        let subject = Subject::Key("v1/h-a/telemetry/sysinfo/disk".into());
        let segs = segments(&subject, "");
        assert_eq!(
            segs.iter().map(|s| s.label.as_str()).collect::<Vec<_>>(),
            ["v1", "h-a", "telemetry", "sysinfo", "disk"]
        );
        assert_eq!(
            segs[2].select,
            Some(Subject::Prefix("v1/h-a/telemetry".into())),
            "an ancestor selects the subtree up to itself"
        );
        assert!(
            segs.last().unwrap().select.is_none(),
            "the final segment is where the window already is"
        );
        assert!(
            segs.iter()
                .all(|s| !matches!(s.select, Some(Subject::Key(_)))),
            "no breadcrumb click may put a fetch on the wire"
        );
    }

    /// A prefix subject reads the same way — its own last chunk is the place.
    #[test]
    fn a_prefix_subject_is_its_own_final_segment() {
        let segs = segments(&Subject::Prefix("v1/h-a/state".into()), "");
        assert_eq!(segs.len(), 3);
        assert!(segs[2].select.is_none());
        assert_eq!(segs[2].label, "state");
    }

    /// An origin is placed through the tree's own click-through path (#61),
    /// base included, so its ancestors are real tree prefixes.
    #[test]
    fn an_origin_subject_is_placed_under_its_base() {
        let segs = segments(&Subject::Origin("h-a".into()), "acme");
        assert_eq!(
            segs.iter().map(|s| s.label.as_str()).collect::<Vec<_>>(),
            ["acme", "v1", "h-a"]
        );
        assert_eq!(segs[1].select, Some(Subject::Prefix("acme/v1".into())));
        assert!(segs[2].select.is_none());
    }

    /// Nothing selected is an empty model — the bar renders the explanation,
    /// not a blank.
    #[test]
    fn no_subject_means_no_segments() {
        assert!(segments(&Subject::None, "acme").is_empty());
    }
}

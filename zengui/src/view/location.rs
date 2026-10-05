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
//! `bar` reads five of the six sub-states and moves none, like the toolbar
//! before it. [`breadcrumb`] takes plain data ([`LocationData`]) so
//! `tests/panes.rs` renders it headlessly, the same seam as
//! [`super::status::Status`].

use iced::Element;
use iced::widget::{column, row};

use crate::config::BaseChoice;
use crate::message::{ChromeMsg, DeploymentMsg, Message, Subject, SubjectMsg, WorkspaceMsg};
use crate::prefs::{DockRole, LayoutPreset};
use crate::scope::ScopePreset;
use crate::state::{Chrome, Deployment, Observation, SubjectState, Workspace};
use crate::view::palette::{Overlay, PaletteMsg};
use crate::view::replay::ReplayMsg;
use crate::view::tokens::{face, space};
use crate::view::{kit, tokens};

/// Everything the breadcrumb shows, as plain data.
pub struct LocationData<'a> {
    /// The active named context, when one is. `None` renders as "no context"
    /// — a state, not an omission — and the chip opens Connect either way.
    pub context: Option<&'a str>,
    /// The deployment base in force; `""` is the bus root.
    pub base: &'a str,
    /// The picker's options (`crate::update::bus::rebuild_base_options`).
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
    let context_chip = kit::secondary(kit::labelled(
        kit::Icon::Plug,
        match d.context {
            Some(name) => format!("context: {name}"),
            None => "no context".to_string(),
        },
    ))
    .on_press(Message::Chrome(ChromeMsg::Palette(PaletteMsg::Open(
        Overlay::Connect,
    ))))
    .padding(space::XS);

    // The picker displays each base by its label, so the empty base reads as
    // the bus-root deployment it is, never as a blank row (#185).
    let base_picker = kit::picker(d.base_options, Some(BaseChoice::new(d.base)), |c| {
        Message::Deployment(DeploymentMsg::BaseSelected(c.base))
    })
    .placeholder("base")
    .text_size(tokens::font::CAPTION);

    // The whole scope vocabulary, custom included (#187): the picker's
    // options always contain the selected value. Picking custom forks the
    // current preset's resolved selectors and opens the editor; the chip
    // beside it is the direct route to the same overlay.
    let scope_picker = kit::picker(&ScopePreset::ALL[..], Some(d.scope), |s| {
        Message::Deployment(DeploymentMsg::ScopeSelected(s))
    })
    .text_size(tokens::font::CAPTION);
    let selectors_chip = kit::ghost(kit::labelled(kit::Icon::Selectors, "selectors…"))
        .on_press(Message::Chrome(ChromeMsg::Palette(PaletteMsg::Open(
            Overlay::Selectors,
        ))))
        .padding(space::XS);

    // Observation is opt-in and labelled by its cost (issue #85); it rides
    // beside the scope it observes.
    let observe = kit::secondary(if d.observing {
        kit::labelled(kit::Icon::Unwatched, "stop observing scope")
    } else {
        kit::labelled(kit::Icon::Watched, "observe scope")
    })
    .on_press(Message::Deployment(DeploymentMsg::ScopeWatchToggled))
    .padding(space::XS);

    // Two lines (#536): where the window points — context, base, scope —
    // and, under it, what it is looking at. Each wraps on its own, so a
    // narrow window never splits a key across the stations.
    let stations = row![
        context_chip,
        kit::separator(),
        base_picker,
        kit::separator(),
        scope_picker,
        selectors_chip,
        observe,
    ]
    .spacing(space::SM)
    .align_y(iced::Alignment::Center)
    .width(iced::Length::Fill)
    .wrap()
    .vertical_spacing(space::XS);

    let mut trail = row![].spacing(space::XS).align_y(iced::Alignment::Center);
    let segs = segments(d.subject, d.base);
    if segs.is_empty() {
        // An empty segment list is a state, and it says which one.
        trail = trail.push(kit::muted("nothing selected — pick a key in the tree"));
    } else {
        for (i, seg) in segs.into_iter().enumerate() {
            if i > 0 {
                trail = trail.push(kit::muted("/"));
            }
            trail = trail.push(match seg.select {
                Some(subtree) => Element::from(
                    kit::link(kit::caption(seg.label).font(face::MONO))
                        .on_press(Message::Subject(SubjectMsg::Select(subtree)))
                        .padding([0.0, space::XS]),
                ),
                // The final chunk: where the window is — the one TITLE (#191).
                None => kit::title(seg.label).font(face::MONO).into(),
            });
        }
    }
    column![
        stations,
        trail
            .width(iced::Length::Fill)
            .wrap()
            .vertical_spacing(space::XS),
    ]
    .spacing(space::XS)
    .into()
}

/// The whole top region (#536): the app bar, then the breadcrumb with the
/// dock strip at its end.
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
    // The breadcrumb wraps within whatever the dock strip leaves it, so a
    // deep key at 1024px flows onto a second line instead of clipping.
    let place = row![
        breadcrumb(LocationData {
            context: work.bench.context_form.active.as_deref(),
            base: dep.base(),
            base_options: &dep.base_options,
            scope: dep.settings.scope,
            observing: !obs.scope_watches.is_empty(),
            subject: &sub.follow.current,
        }),
        dock_strip(work),
    ]
    .spacing(space::MD)
    .align_y(iced::Alignment::Center);
    column![app_bar(chrome, dep, obs, work), place]
        .spacing(space::SM)
        .into()
}

/// Wide enough to hold the app bar's three groups on one line, presets
/// centred. Below it the presets take a line of their own, so nothing clips
/// at the 1024px the shots harness checks.
const ONE_LINE: f32 = 1240.0;

/// The app bar (#536): who the window is talking to on the left, which
/// layout it is in at the centre, the window's own controls on the right.
fn app_bar<'a>(
    chrome: &'a Chrome,
    dep: &'a Deployment,
    obs: &'a Observation,
    work: &'a Workspace,
) -> Element<'a, Message> {
    iced::widget::responsive(move |size| {
        let left = row![app_mark(), connection(dep, obs, work)]
            .spacing(space::MD)
            .align_y(iced::Alignment::Center);
        if size.width >= ONE_LINE {
            row![
                left,
                iced::widget::space::horizontal(),
                presets(chrome),
                iced::widget::space::horizontal(),
                window_controls(chrome, work),
            ]
            .spacing(space::SM)
            .align_y(iced::Alignment::Center)
            .into()
        } else {
            column![
                row![
                    left,
                    iced::widget::space::horizontal(),
                    window_controls(chrome, work)
                ]
                .spacing(space::SM)
                .align_y(iced::Alignment::Center),
                iced::widget::container(presets(chrome)).center_x(iced::Length::Fill),
            ]
            .spacing(space::SM)
            .into()
        }
    })
    .height(iced::Length::Shrink)
    .into()
}

/// The app's mark: an icon and its name, at EMPHASIS — the window's one
/// TITLE stays the subject's (#191).
fn app_mark<'a>() -> Element<'a, Message> {
    row![
        kit::icon(kit::Icon::Logo).style(|theme: &iced::Theme| iced::widget::text::Style {
            color: Some(crate::view::theme::colors(theme).primary()),
        }),
        kit::emphasis("zengui").font(face::SEMIBOLD),
    ]
    .spacing(space::XS)
    .align_y(iced::Alignment::Center)
    .into()
}

/// The connection pill (#536): the link's state as a dot **and** a word,
/// the session's mode when the flags decide it, and where it dials. The
/// status strip keeps the full posture sentence; this is the glance.
fn connection<'a>(
    dep: &'a Deployment,
    obs: &'a Observation,
    work: &'a Workspace,
) -> Element<'a, Message> {
    let (tone, word) = super::status::link_word(
        &obs.link,
        work.replay.replay.is_some(),
        dep.settings.is_unreachable(),
    );
    let mut r = row![kit::status(tone, word)]
        .spacing(space::SM)
        .align_y(iced::Alignment::Center);
    if let Some(mode) = session_mode(&dep.settings) {
        r = r.push(kit::data_chip(mode));
    }
    if let Some(endpoint) = endpoint_label(&dep.settings) {
        r = r.push(kit::data_chip(endpoint));
    }
    kit::pill(r)
}

/// The session's mode, when the flags decide it (#501): listen endpoints
/// make a peer, anything else a client — and a zenoh config file may state
/// its own, which the window has not read, so it claims nothing.
pub fn session_mode(s: &crate::config::Settings) -> Option<&'static str> {
    if s.zenoh_config.is_some() {
        None
    } else if s.listen.is_empty() {
        Some("client")
    } else {
        Some("peer")
    }
}

/// Where the session dials: the first connect endpoint, `+N` for the rest;
/// else what it listens on. `None` when it does neither — the pill's word
/// ("reaches nothing") already says what that means.
pub fn endpoint_label(s: &crate::config::Settings) -> Option<String> {
    let (list, prefix) = if !s.connect.is_empty() {
        (&s.connect, "")
    } else if !s.listen.is_empty() {
        (&s.listen, "listen ")
    } else {
        return None;
    };
    let more = list.len() - 1;
    Some(if more == 0 {
        format!("{prefix}{}", list[0])
    } else {
        format!("{prefix}{} +{more}", list[0])
    })
}

/// The layout presets, on screen at last (#180's Alt+1/2/3): the same
/// [`WorkspaceMsg::LayoutPreset`] the shortcuts send. A dragged splitter or
/// a toggled dock leaves no preset lit — and says "custom layout" rather
/// than leave a lit segment lying.
fn presets<'a>(chrome: &'a Chrome) -> Element<'a, Message> {
    let segs = LayoutPreset::ALL
        .into_iter()
        .map(|p| kit::Segment {
            value: p,
            label: p.label().to_string(),
            icon: Some(match p {
                LayoutPreset::Explore => kit::Icon::Explore,
                LayoutPreset::Watch => kit::Icon::Watch,
                LayoutPreset::Diagnose => kit::Icon::Diagnose,
            }),
            count: None,
            tip: Some(match p {
                LayoutPreset::Explore => "layout · Alt 1",
                LayoutPreset::Watch => "layout · Alt 2",
                LayoutPreset::Diagnose => "layout · Alt 3",
            }),
        })
        .collect();
    let active = chrome.prefs.layout.preset;
    let control = kit::segmented(segs, active, |p| {
        Message::Workspace(WorkspaceMsg::LayoutPreset(p))
    });
    match active {
        Some(_) => control,
        None => row![control, kit::muted("custom layout")]
            .spacing(space::SM)
            .align_y(iced::Alignment::Center)
            .into(),
    }
}

/// The window's own controls (#536): capture and replay, then the window
/// itself. A stateless verb that is also in the palette may be an icon with
/// a tooltip (settings, theme, zoom steps, reconnect); a control with state
/// keeps its word — a recording, the density, the zoom level — because a
/// tooltip is not read until hovered (#187's rule).
fn window_controls<'a>(chrome: &'a Chrome, work: &'a Workspace) -> Element<'a, Message> {
    use crate::message::PrefsMsg;
    let prefs = |m| Message::Chrome(ChromeMsg::Prefs(m));
    // Capture and replay (#74): record writes the current watches to a
    // .zrec; replay feeds the panes from one. A recording is the one state
    // here worth a colour, and it is in words too — the mode's (#544), as a
    // status beside the button. The button itself is secondary: danger's
    // rank is for the irreversible wire act, and stopping a recording
    // finishes a file.
    let toggle = Message::Workspace(WorkspaceMsg::Replay(ReplayMsg::RecordToggled));
    let record: Element<'a, Message> = if work.replay.recording.is_some() {
        row![
            kit::status(crate::view::theme::Tone::Mode, "recording"),
            kit::secondary(kit::labelled(kit::Icon::Stop, "stop recording"))
                .on_press(toggle)
                .padding([space::XS, space::SM]),
        ]
        .spacing(space::SM)
        .align_y(iced::Alignment::Center)
        .into()
    } else {
        kit::secondary(kit::labelled(kit::Icon::Record, "record"))
            .on_press(toggle)
            .padding([space::XS, space::SM])
            .into()
    };
    let replay = kit::secondary(kit::labelled(kit::Icon::Replay, "replay…"))
        .on_press(Message::Workspace(WorkspaceMsg::Replay(
            ReplayMsg::OpenToggled,
        )))
        .padding([space::XS, space::SM]);
    let theme_icon = match chrome.prefs.theme {
        crate::prefs::ThemeChoice::Dark => kit::Icon::Dark,
        crate::prefs::ThemeChoice::Light => kit::Icon::Light,
    };
    row![
        record,
        replay,
        kit::tip(
            kit::icon_button(kit::Icon::Settings, None).on_press(Message::Chrome(
                ChromeMsg::Palette(PaletteMsg::Open(Overlay::Settings))
            )),
            "settings · Ctrl ,",
        ),
        // The icon is the theme you are in; the window around it is the
        // rest of the statement.
        kit::tip(
            kit::icon_button(theme_icon, None).on_press(prefs(PrefsMsg::ThemeToggled)),
            "theme · Ctrl T",
        ),
        // Density (#192) keeps its word: the two modes look alike at a
        // glance, and the word is the only thing that says which is on.
        kit::tip(
            kit::icon_button(kit::Icon::Density, Some(chrome.prefs.density.label()))
                .on_press(prefs(PrefsMsg::DensityToggled)),
            "density · Ctrl Shift D",
        ),
        kit::tip(
            kit::icon_button(kit::Icon::ZoomOut, None).on_press(prefs(PrefsMsg::ZoomOut)),
            "zoom out · Ctrl −",
        ),
        kit::tip(
            kit::ghost(kit::caption(format!(
                "{}%",
                (chrome.prefs.zoom * 100.0).round() as i32
            )))
            .on_press(prefs(PrefsMsg::ZoomReset))
            .padding([space::XS, space::XS]),
            "reset zoom · Ctrl 0",
        ),
        kit::tip(
            kit::icon_button(kit::Icon::ZoomIn, None).on_press(prefs(PrefsMsg::ZoomIn)),
            "zoom in · Ctrl +",
        ),
        kit::tip(
            kit::icon_button(kit::Icon::Reconnect, None)
                .on_press(Message::Deployment(DeploymentMsg::Reconnect)),
            "reconnect · Ctrl R",
        ),
    ]
    .spacing(space::XS)
    .align_y(iced::Alignment::Center)
    .into()
}

/// The dock strip (#180, restyled by #536): four toggles, one per
/// [`DockRole`], where on means *open in the grid* — not "the one pane
/// showing". Clicking closes an open dock or restores a closed one: the
/// same [`WorkspaceMsg::DockToggled`] each dock's title-bar close sends.
fn dock_strip<'a>(work: &'a Workspace) -> Element<'a, Message> {
    iced::widget::Row::from_iter(DockRole::ALL.into_iter().map(|role| {
        kit::toggle_chip(
            Some(super::panes::dock_icon(role)),
            role.label(),
            work.docks.is_open(role),
            Message::Workspace(WorkspaceMsg::DockToggled(role)),
        )
    }))
    .spacing(space::XS)
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

    /// The #187 defect, pinned at the seam the picker draws from (a
    /// `pick_list` renders its own rows, so `iced_test` cannot look inside
    /// it): the option list is `ScopePreset::ALL`, whose exhaustiveness over
    /// the enum `scope.rs` pins — so whatever scope the window is in,
    /// `--scope custom --selector …` included, the options contain the
    /// selected value.
    #[test]
    fn the_scope_pickers_options_always_contain_the_selected_value() {
        assert!(
            ScopePreset::ALL.contains(&ScopePreset::Custom),
            "the picker's option list excludes custom — the #187 lie again"
        );
    }
}

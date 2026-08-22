//! The workspace grid (#180): four dock roles in an `iced` `pane_grid`.
//!
//! One `match` over [`DockRole`], and it is the only place in the crate that
//! knows the mapping from a role to a pane function — the successor of the
//! old `match` over [`RightPane`], which lives on inside [`workbench`] for
//! the four tools that are not docks of their own. The grid replaces the
//! fixed `FillPortion(1)`/`FillPortion(1)` row: splitters drag, docks
//! drag-reorder, a title-bar `×` closes, and the shape is persisted as
//! `chrome.prefs.layout` (superseding the `prefs.split` scalar that was
//! stored, clamped, round-trip tested and read by nothing).
//!
//! Every argument is shared. `view` takes `&self`, so the six sub-states are
//! six disjoint shared borrows and borrowck never notices they came from one
//! struct. The pane functions stay free functions over plain data — the
//! grid composes them, which is why `tests/panes.rs` renders them with no
//! window and no bus, grid or not.

use iced::widget::{button, column, pane_grid, row};
use iced::{Element, Length};

use crate::message::{Message, RightPane, WorkspaceMsg};
use crate::prefs::DockRole;
use crate::state::{Deployment, Observation, SubjectState, TreeState, Workspace};
use crate::view;
use crate::view::tokens::space;
use crate::view::{kit, theme};

/// Whether any active watch selector covers this exact key.
///
/// The distinction the history pane rests on: with no watch, no sample can
/// reach the recorder, and "nothing recorded yet" would be a verdict the tool
/// never obtained (RFC 09 §5.1 O4). A selector that does not parse cannot be
/// claimed as coverage, so it is not counted — the same rule
/// `StatsTable::retire_unwatched` applies from the other side.
pub(crate) fn key_is_watched(watched: &[String], key: &str) -> bool {
    let Ok(ke) = zenoh::key_expr::KeyExpr::new(key.to_string()) else {
        return false;
    };
    watched
        .iter()
        .filter_map(|sel| zenoh::key_expr::KeyExpr::new(sel.clone()).ok())
        .any(|sel| sel.intersects(&ke))
}

/// The workspace: the open docks, arranged and sized as the user left them.
pub(crate) fn grid<'a>(
    dep: &'a Deployment,
    obs: &'a Observation,
    sub: &'a SubjectState,
    tree: &'a TreeState,
    work: &'a Workspace,
) -> Element<'a, Message> {
    pane_grid::PaneGrid::new(&work.docks.grid, |pane, role, _maximized| {
        let focused = work.docks.focus == Some(pane);
        let body: Element<'a, Message> = match role {
            DockRole::Locator => locator(dep, obs, sub, tree),
            DockRole::Inspector => inspector(dep, obs, sub, work),
            DockRole::Activity => activity(dep, obs, sub, work),
            DockRole::Workbench => workbench(dep, sub, work),
        };
        pane_grid::Content::new(body).title_bar(title_bar(*role, focused))
    })
    .spacing(space::XS)
    .min_size(120)
    .on_click(|pane| Message::Workspace(WorkspaceMsg::DockFocused(pane)))
    .on_drag(|event| Message::Workspace(WorkspaceMsg::PaneDragged(event)))
    .on_resize(8, |event| {
        Message::Workspace(WorkspaceMsg::PaneResized(event))
    })
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}

/// A dock's handle: its name (the drag surface) and its `×`. The focused
/// dock's title reads on the pane surface; the rest stay muted.
fn title_bar<'a>(role: DockRole, focused: bool) -> pane_grid::TitleBar<'a, Message> {
    pane_grid::TitleBar::new(if focused {
        kit::caption(role.label())
    } else {
        kit::caption(role.label()).style(|t: &iced::Theme| iced::widget::text::Style {
            color: Some(theme::colors(t).text_muted()),
        })
    })
    .controls(pane_grid::Controls::new(
        button(kit::caption("×"))
            .padding([0, 4])
            .style(iced::widget::button::text)
            .on_press(Message::Workspace(WorkspaceMsg::DockToggled(role))),
    ))
    .padding(space::XS)
    .style(move |t: &iced::Theme| iced::widget::container::Style {
        background: focused.then(|| theme::colors(t).surface().into()),
        ..iced::widget::container::Style::default()
    })
}

fn locator<'a>(
    dep: &'a Deployment,
    obs: &'a Observation,
    sub: &'a SubjectState,
    tree: &'a TreeState,
) -> Element<'a, Message> {
    view::tree::pane(view::tree::TreeData {
        flat: &tree.flat,
        pivot: tree.pivot,
        search: &tree.tree_search,
        scroll_y: tree.tree_scroll.0,
        viewport_h: tree.tree_scroll.1,
        facts: &dep.facts,
        watches: view::tree::Watches {
            mine: &obs.my_watch_paths,
            seeding: &obs.seeding_paths,
        },
        selected: sub.current.path(),
    })
}

fn inspector<'a>(
    dep: &'a Deployment,
    obs: &'a Observation,
    sub: &'a SubjectState,
    work: &'a Workspace,
) -> Element<'a, Message> {
    view::inspector::pane(view::inspector::InspectorData {
        subject: &sub.current,
        facts: sub.current.key().and_then(|k| dep.facts.get(k)),
        fetched: match sub.fetched.as_ref() {
            None => view::detail::Fetched::NotAsked,
            Some((k, o)) if Some(k.as_str()) == sub.current.key() => {
                view::detail::Fetched::Landed(o)
            }
            Some(_) => view::detail::Fetched::Superseded,
        },
        decoded: sub.decoded.as_ref(),
        series: sub.series.as_ref(),
        history: sub.history.as_ref(),
        history_scroll: sub.history_scroll,
        watched: sub
            .current
            .key()
            .is_some_and(|k| key_is_watched(&obs.watched, k)),
        latency: sub.selected_latency.clone(),
        blob: &work.verdicts.blob,
        media: &work.bench.media,
        slices: dep.slices.as_deref(),
        roster: &work.verdicts.roster,
        node_detail: &work.verdicts.node_detail,
    })
}

fn activity<'a>(
    dep: &'a Deployment,
    obs: &'a Observation,
    sub: &'a SubjectState,
    work: &'a Workspace,
) -> Element<'a, Message> {
    view::activity::dock(view::activity::ActivityData {
        dock: &work.activity,
        echo: &work.echo.echo,
        echo_view: &work.echo.echo_view,
        echo_scroll: work.echo.echo_scroll,
        follow: work
            .echo
            .echo_view
            .follow_subject
            .then(|| sub.current.key())
            .flatten(),
        next_seq: work.echo.echo.next_seq(),
        publish: &work.bench.publish_form,
        doctor: &work.verdicts.doctor,
        base: dep.base(),
        replay: &work.replay,
        slices: dep.slices.as_deref(),
        retention: obs.retention,
    })
}

/// The tools dock: call, publish, nodes, admin behind its own small strip —
/// the remnant of the eleven-tab workspace, honest here the way the Activity
/// dock's strip is: four different tools, of which a human uses one at a
/// time. #184 merges the first two into Send; #190 gives the strip keys.
fn workbench<'a>(
    dep: &'a Deployment,
    sub: &'a SubjectState,
    work: &'a Workspace,
) -> Element<'a, Message> {
    let mut tools = row![].spacing(space::XS);
    for p in RightPane::ALL {
        if p == RightPane::Inspector {
            // A dock of its own since #180, not a tool of this one.
            continue;
        }
        tools = tools.push(kit::tab(
            p.label(),
            work.right_pane == p,
            Message::Workspace(WorkspaceMsg::PaneSelected(p)),
        ));
    }
    let body: Element<'a, Message> = match work.right_pane {
        RightPane::Call => view::call::pane(
            &work.bench.call_form,
            dep.slices.as_deref(),
            &work.verdicts.roster,
        ),
        RightPane::Publish => view::publish::pane(&work.bench.publish_form, dep.slices.is_some()),
        RightPane::Nodes => view::nodes::pane(view::nodes::NodesData {
            roster: &work.verdicts.roster,
            selected: sub.current.origin(),
            detail: &work.verdicts.node_detail,
        }),
        RightPane::Admin => view::admin::pane(&work.verdicts.admin),
        // Unreachable by construction — `PaneSelected(Inspector)` restores
        // the Inspector dock instead of writing `right_pane`, and the
        // default is `Call` — but a match must say what it would mean, and
        // "look one dock over" is the honest answer.
        RightPane::Inspector => kit::empty_state(
            "the inspector is a dock of its own",
            "select a tool above; the subject renders in the inspector dock",
        ),
        // Connect is no pane since #185: contexts and endpoints are the
        // Connect overlay, reached from the location bar's context chip
        // or Ctrl+Shift+C.
    };
    column![tools, body]
        .spacing(space::XS)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

#[cfg(test)]
mod tests {
    use super::key_is_watched;

    #[test]
    fn watch_coverage_is_decided_by_intersection_not_by_prefix() {
        let watched = vec![
            "v1/h-a/telemetry/**".to_string(),
            "demo/example/foo".to_string(),
        ];
        assert!(key_is_watched(&watched, "v1/h-a/telemetry/sysinfo/cpu"));
        assert!(key_is_watched(&watched, "demo/example/foo"));
        assert!(!key_is_watched(&watched, "v1/h-b/telemetry/sysinfo/cpu"));
        assert!(!key_is_watched(&[], "v1/h-a/telemetry/sysinfo/cpu"));
    }

    /// A selector we cannot parse is not coverage we can claim.
    #[test]
    fn an_unparseable_selector_is_not_counted_as_coverage() {
        assert!(!key_is_watched(&["not a key/**/".to_string()], "a/b"));
    }
}

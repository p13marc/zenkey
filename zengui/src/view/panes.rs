//! The workspace grid (#180): four dock roles in an `iced` `pane_grid`.
//!
//! One `match` over [`DockRole`], and it is the only place in the crate that
//! knows the mapping from a role to a pane function — the successor of the
//! old `match` over [`RightPane`], which lives on inside `workbench` for
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

use iced::widget::{column, pane_grid, row};
use iced::{Element, Length};

use crate::message::{Message, RightPane, SlotId, WorkspaceMsg};
use crate::prefs::{Density, DockRole};
use crate::state::subject::SubjectSlot;
use crate::state::{Deployment, Observation, SubjectState, TreeState, Workspace};
use crate::view;
use crate::view::tokens::{Spacing, space};
use crate::view::{kit, theme};

/// Whether any active watch selector covers this exact key.
///
/// The distinction the history pane rests on: with no watch, no sample can
/// reach the recorder, and "nothing recorded yet" would be a verdict the tool
/// never obtained (RFC 09 §5.1 O4). A selector that does not parse cannot be
/// claimed as coverage, so it is not counted — the same rule
/// `StatsTable::retire_unwatched` applies from the other side.
pub(crate) fn key_is_watched(watched: &[String], key: &str) -> bool {
    let Ok(ke) = zenoh::key_expr::KeyExpr::new(key) else {
        return false;
    };
    watched
        .iter()
        .filter_map(|sel| zenoh::key_expr::KeyExpr::new(sel.as_str()).ok())
        .any(|sel| sel.intersects(&ke))
}

/// The workspace: the open docks, arranged and sized as the user left them.
///
/// `density` is the *global* mode (#192); this closure is the one place a
/// dock's effective density is resolved ([`DockRole::density`]) and spent
/// ([`Spacing::of`]) — every pane below takes the resolved grid and never
/// asks which mode it is in, the same way it takes `theme::colors` and never
/// asks which theme.
pub(crate) fn grid<'a>(
    dep: &'a Deployment,
    obs: &'a Observation,
    sub: &'a SubjectState,
    tree: &'a TreeState,
    work: &'a Workspace,
    density: Density,
) -> Element<'a, Message> {
    pane_grid::PaneGrid::new(&work.docks.grid, move |pane, role, _maximized| {
        let focused = work.docks.focus == Some(pane);
        let sp = Spacing::of(role.density(density));
        // The grid's docks all follow the selection (#257): only a torn-off
        // window ([`solo`]) can bind another slot.
        pane_grid::Content::new(body(dep, obs, sub, tree, work, *role, SlotId::FOLLOW, sp))
            .title_bar(title_bar(*role, focused))
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

/// One dock's content, whatever window it renders in: the single `match`
/// from a [`DockRole`] to a pane function. The grid composes it into a
/// `pane_grid` cell; a torn-off window ([`solo`], #186) composes the same
/// call alone — one dispatch, two framings, so a dock cannot render
/// differently for having its own window.
///
/// `slot` is which subject the Inspector shows (#257): the grid always says
/// [`SlotId::FOLLOW`]; a pinned window says its own. The other three docks
/// ignore it — the Locator *is* the follow selection, and the Activity and
/// Workbench streams are about the session.
#[allow(clippy::too_many_arguments)]
fn body<'a>(
    dep: &'a Deployment,
    obs: &'a Observation,
    sub: &'a SubjectState,
    tree: &'a TreeState,
    work: &'a Workspace,
    role: DockRole,
    slot: SlotId,
    sp: Spacing,
) -> Element<'a, Message> {
    match role {
        DockRole::Locator => locator(dep, obs, sub, tree, work, sp),
        DockRole::Inspector => {
            // A slot that is gone renders as the follow slot rather than a
            // blank window; unreachable in practice, because closing a
            // pinned window is what drops its slot.
            let bound = sub.slot(slot).unwrap_or(&sub.follow);
            inspector(dep, obs, bound, work, sp)
        }
        DockRole::Activity => activity(dep, obs, sub, work, sp),
        DockRole::Workbench => workbench(dep, sub, work, sp),
    }
}

/// A torn-off dock, alone in its own window (#186): the same [`body`] the
/// grid renders, with the dock padding the grid cell would have given it.
/// No title bar — the window's own chrome names it, and the way to re-dock
/// is to close the window.
///
/// `slot` is the window's binding (#257). A pinned Inspector leads with the
/// pin statement — what it holds, and that closing the window unpins — so a
/// window whose chart no longer moves with the tree says why, in words, on
/// the surface the ⇱ produced.
#[allow(clippy::too_many_arguments)]
pub(crate) fn solo<'a>(
    dep: &'a Deployment,
    obs: &'a Observation,
    sub: &'a SubjectState,
    tree: &'a TreeState,
    work: &'a Workspace,
    role: DockRole,
    slot: SlotId,
    density: Density,
) -> Element<'a, Message> {
    let sp = Spacing::of(role.density(density));
    let mut col = column![].spacing(sp.sm);
    if role == DockRole::Inspector {
        col = col.push(pin_banner(sub, slot));
    }
    iced::widget::container(col.push(body(dep, obs, sub, tree, work, role, slot, sp)))
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(sp.md)
        .into()
}

/// What a torn Inspector window is (#257), stated rather than inferred: a
/// pin names what it holds; a follow window says the selection drives it.
/// The wording is the honesty rule — a pane bound to a slot the tree does
/// not drive must say so, or its stillness reads as a dead app.
fn pin_banner<'a>(sub: &'a SubjectState, slot: SlotId) -> Element<'a, Message> {
    let line = match sub.slot(slot) {
        Some(s) if slot.is_pin() => match &s.current {
            crate::message::Subject::Key(k) => format!(
                "pinned to {k} — the selection drives the docked Inspector, \
                 not this window; closing it unpins and drops this recording"
            ),
            crate::message::Subject::Prefix(p) => format!(
                "pinned to the subtree {p} — the selection drives the docked \
                 Inspector, not this window; closing it unpins"
            ),
            crate::message::Subject::Origin(o) => format!(
                "pinned to the origin {o} — the selection drives the docked \
                 Inspector, not this window; closing it unpins"
            ),
            // Unreachable — a pin is only minted for a subject — but a match
            // must say what it would mean.
            crate::message::Subject::None => "pinned to nothing — close this window".to_string(),
        },
        // Follow-bound (a tear-off with nothing selected, or a window
        // restored at boot: a pin's evidence is session-lived, so the pin
        // did not survive the restart) — and the honest degenerate case of
        // a dropped slot, which closing the window has already forgotten.
        _ => "follows the selection — pins are made by tearing off the \
              Inspector while a subject is selected"
            .to_string(),
    };
    kit::muted(line)
}

/// A dock's handle: its name (the drag surface), its `⇱` (tear off into a
/// window, #186 — not on the Locator, which is the navigation itself) and
/// its `×`. The focused dock's title reads on the pane surface; the rest
/// stay muted.
fn title_bar<'a>(role: DockRole, focused: bool) -> pane_grid::TitleBar<'a, Message> {
    let mut controls = row![].spacing(space::XS);
    if role != DockRole::Locator {
        controls = controls.push(
            kit::link(kit::caption("⇱"))
                .padding([0.0, space::XS])
                .on_press(Message::Workspace(WorkspaceMsg::TearOff(role))),
        );
    }
    controls = controls.push(
        kit::link(kit::caption("×"))
            .padding([0.0, space::XS])
            .on_press(Message::Workspace(WorkspaceMsg::DockToggled(role))),
    );
    pane_grid::TitleBar::new(if focused {
        kit::caption(role.label())
    } else {
        kit::caption(role.label()).style(|t: &iced::Theme| iced::widget::text::Style {
            color: Some(theme::colors(t).text_muted()),
        })
    })
    .controls(pane_grid::Controls::new(controls))
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
    work: &'a Workspace,
    sp: Spacing,
) -> Element<'a, Message> {
    view::tree::pane(view::tree::TreeData {
        flat: &tree.flat,
        pivot: tree.pivot,
        search: &tree.tree_search,
        viewport: tree.tree_scroll,
        facts: &dep.facts,
        verdicts: &work.verdicts.payloads,
        budgets: obs.budgets.as_deref(),
        watches: view::tree::Watches {
            mine: &obs.my_watch_paths,
            seeding: &obs.seeding_paths,
        },
        selected: sub.follow.current.path(),
        sp,
    })
}

fn inspector<'a>(
    dep: &'a Deployment,
    obs: &'a Observation,
    slot: &'a SubjectSlot,
    work: &'a Workspace,
    sp: Spacing,
) -> Element<'a, Message> {
    view::inspector::pane(view::inspector::InspectorData {
        slot: slot.id,
        subject: &slot.current,
        facts: slot.current.key().and_then(|k| dep.facts.get(k)),
        fetched: match slot.fetched.as_ref() {
            None => view::detail::Fetched::NotAsked,
            Some((k, o)) if Some(k.as_str()) == slot.current.key() => {
                view::detail::Fetched::Landed(o)
            }
            Some(_) => view::detail::Fetched::Superseded,
        },
        decoded: slot.decoded.as_deref(),
        series: slot.series.as_ref(),
        history: slot.history.as_ref(),
        history_scroll: slot.history_scroll,
        watched: slot
            .current
            .key()
            .is_some_and(|k| key_is_watched(&obs.watched, k)),
        snapshot: work.replay.snapshot.as_deref(),
        compare_snapshot: slot.compare_snapshot,
        latency: slot.selected_latency.clone(),
        blob: &work.verdicts.blob,
        media: &work.bench.media,
        slices: dep.slices.as_deref(),
        roster: &work.verdicts.roster,
        node_detail: &work.verdicts.node_detail,
        fields: &slot.fields,
        why: &slot.why,
        base: dep.base(),
        observed: &obs.observed,
        sp,
    })
}

fn activity<'a>(
    dep: &'a Deployment,
    obs: &'a Observation,
    sub: &'a SubjectState,
    work: &'a Workspace,
    sp: Spacing,
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
            .then(|| sub.follow.current.key())
            .flatten(),
        verdicts: &work.verdicts.payloads,
        next_seq: work.echo.echo.next_seq(),
        publish: &work.bench.send_form,
        doctor: &work.verdicts.doctor,
        base: dep.base(),
        replay: &work.replay,
        slices: dep.slices.as_deref(),
        retention: obs.retention,
        sp,
    })
}

/// The tools dock: send, nodes, admin behind its own small strip — the
/// remnant of the eleven-tab workspace, honest here the way the Activity
/// dock's strip is: three different tools, of which a human uses one at a
/// time. #184 merged publish and call into Send; #190 gives the strip keys.
fn workbench<'a>(
    dep: &'a Deployment,
    sub: &'a SubjectState,
    work: &'a Workspace,
    sp: Spacing,
) -> Element<'a, Message> {
    let mut tools = row![].spacing(sp.xs);
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
        RightPane::Send => view::send::pane(
            &work.bench.send_form,
            dep.slices.as_deref(),
            &work.verdicts.roster,
            sp,
        ),
        RightPane::Nodes => view::nodes::pane(view::nodes::NodesData {
            roster: &work.verdicts.roster,
            selected: sub.follow.current.origin(),
            detail: &work.verdicts.node_detail,
            slices: dep.slices.as_deref(),
            sp,
        }),
        RightPane::Admin => view::admin::pane(&work.verdicts.admin, sp),
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
        .spacing(sp.xs)
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

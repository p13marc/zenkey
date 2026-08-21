//! The two-pane layout: the key tree, and whichever right-hand pane is
//! showing (#175).
//!
//! One `match` over [`RightPane`], and it is the only place in the crate that
//! knows the mapping from a tab to a pane function. That is why it is here
//! rather than inline in `app.rs`: `view` is now the layout — toolbar, replay
//! surfaces, panes, status strip, overlay — and each of those is one call.
//!
//! Every argument is shared. `view` takes `&self`, so the six sub-states are
//! six disjoint shared borrows and borrowck never notices they came from one
//! struct.

use iced::widget::{container, row};
use iced::{Element, Length};

use crate::message::{Message, RightPane};
use crate::state::{Chrome, Deployment, Observation, Subject, TreeState, Workspace};
use crate::view;
use crate::view::tokens::space;

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

pub(crate) fn split<'a>(
    chrome: &'a Chrome,
    dep: &'a Deployment,
    obs: &'a Observation,
    sub: &'a Subject,
    tree: &'a TreeState,
    work: &'a Workspace,
) -> Element<'a, Message> {
    row![
        container(view::tree::pane(view::tree::TreeData {
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
            selected: sub.selected.as_deref(),
        }))
        .width(Length::FillPortion(chrome.prefs.split_portions().0))
        .height(Length::Fill),
        container(match work.right_pane {
            RightPane::Echo => view::echo::pane(
                &work.echo.echo,
                &work.echo.echo_view,
                sub.selected.as_deref(),
                work.echo.echo.next_seq(),
            ),
            RightPane::Call => view::call::pane(
                &work.bench.call_form,
                dep.slices.as_deref(),
                &work.verdicts.roster,
            ),
            RightPane::Publish =>
                view::publish::pane(&work.bench.publish_form, dep.slices.is_some()),
            RightPane::Detail => view::detail::pane(view::detail::DetailData {
                key: sub.selected.as_deref().unwrap_or("(nothing selected)"),
                facts: sub.selected.as_deref().and_then(|k| dep.facts.get(k)),
                fetched: sub.fetched.as_ref().and_then(|(k, o)| {
                    (Some(k.as_str()) == sub.selected.as_deref()).then_some(o)
                }),
                decoded: sub.decoded.as_ref(),
                series: sub.series.as_ref(),
                history_entries: sub.history.as_ref().map(|r| r.ring.len()),
                observed: sub.history.as_ref().and_then(|r| r.ring.newest()),
                latency: sub.selected_latency.clone(),
            }),
            RightPane::Nodes => view::nodes::pane(view::nodes::NodesData {
                roster: &work.verdicts.roster,
                selected: work.verdicts.node_selected.as_deref(),
                detail: &work.verdicts.node_detail,
            }),
            RightPane::Doctor => view::doctor::pane(&work.verdicts.doctor, dep.base()),
            RightPane::Blob => view::blob::pane(&work.verdicts.blob, dep.slices.is_some()),
            RightPane::Media => view::media::pane(&work.bench.media, dep.slices.as_deref()),
            RightPane::Admin => view::admin::pane(&work.verdicts.admin),
            RightPane::History => view::history::pane(view::history::HistoryData {
                key: sub.selected.as_deref(),
                recorder: sub.history.as_ref(),
                watched: sub
                    .selected
                    .as_deref()
                    .is_some_and(|k| key_is_watched(&obs.watched, k)),
            }),
            RightPane::Connect =>
                view::contexts::pane(&work.bench.context_form, dep.settings.is_unreachable(),),
        })
        .width(Length::FillPortion(chrome.prefs.split_portions().1))
        .height(Length::Fill),
    ]
    .spacing(space::MD)
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

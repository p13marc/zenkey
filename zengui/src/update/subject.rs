//! One key: chosen, observed, fetched, decoded (#175).
//!
//! Five of six, and the reason is the causal chain the message group exists
//! for: `SelectKey` ends in the `fetch_value` that produces `ValueFetched`,
//! which produces `ValueDecoded`. Selecting reveals a row (`tree`), lands the
//! detail pane (`work`), records history and rebuilds the chart (`sub`), and
//! caches the key's facts (`dep`). Watching moves the coverage (`obs`).

use std::sync::Arc;

use iced::Task;

use crate::message::{Message, RightPane, SubjectMsg};
use crate::scope;
use crate::services;
use crate::state::{Deployment, Observation, Subject, TreeState, Workspace};

/// One key: chosen, observed, fetched, decoded.
pub(crate) fn update(
    dep: &mut Deployment,
    obs: &mut Observation,
    sub: &mut Subject,
    tree: &mut TreeState,
    work: &mut Workspace,
    msg: SubjectMsg,
) -> Task<Message> {
    match msg {
        SubjectMsg::WatchToggled(path) => toggle_watch(dep, obs, path),
        SubjectMsg::WatchStarted(path, Ok(id)) => {
            obs.my_watch_paths.insert(path.clone());
            obs.seeding.insert(id, Some(path.clone()));
            obs.seeding_paths.insert(path.clone());
            obs.my_watches.insert(path, id);
            tree.reflatten(dep, obs);
            Task::none()
        }
        SubjectMsg::WatchStarted(path, Err(e)) => {
            tracing::warn!("watch {path} failed: {e}");
            Task::none()
        }
        SubjectMsg::WatchReleased(_, Ok(())) => Task::none(),
        SubjectMsg::WatchReleased(path, Err(e)) => {
            tracing::warn!("unwatch {path} failed: {e}");
            Task::none()
        }
        SubjectMsg::SelectPath(path) => {
            sub.selected = Some(path);
            Task::none()
        }
        SubjectMsg::ValueFetched(key, outcome) => {
            // No base guard needed (#109 audit): the evidence is keyed by
            // the full wire key, which names its own base (an explorer
            // runs un-namespaced, RFC 09 §5), and the view passes
            // `fetched` through only while that exact key is selected.
            // Residual: a stale landing can still flip the right pane to
            // Detail — a focus nit, not a misattributed verdict.
            sub.decoded = None;
            // A fetch normally lands the detail pane in view — except
            // from the doctor's click-through, where losing the finding
            // list would cost more than it shows (#71).
            if work.right_pane != RightPane::Doctor {
                work.right_pane = RightPane::Detail;
            }
            let decode_task = match (&outcome, &dep.session, &dep.schema_store, &dep.slices) {
                (Ok(out), Some(session), Some(store), Some(slices)) => {
                    if let zenkey_fleet::FetchOutcome::Value(v) = out.as_ref() {
                        services::value::decode(
                            Arc::clone(store),
                            session.clone(),
                            Arc::clone(slices),
                            dep.base().to_string(),
                            key.clone(),
                            v.key.clone(),
                            v.encoding.clone(),
                            v.payload.clone(),
                        )
                    } else {
                        Task::none()
                    }
                }
                _ => Task::none(),
            };
            sub.fetched = Some((key, outcome));
            decode_task
        }
        SubjectMsg::ValueDecoded(key, type_name, rendering) => {
            // Stale guard: only the currently selected key's decode lands.
            if sub.selected.as_deref() == Some(key.as_str()) {
                sub.decoded = Some((type_name, (*rendering).clone()));
            }
            Task::none()
        }
        SubjectMsg::SelectKey(key) => {
            sub.selected = key.clone();
            // The old key's latency summary is not evidence about the
            // new one — cleared now, refreshed on the next tick (#119).
            sub.selected_latency = None;
            // History follows the selection and nothing else (issue #63):
            // the previous recording is dropped here, which is what makes
            // deselecting free. A symbolic skeleton path names no concrete
            // key, so nothing can be recorded for it.
            sub.history = key
                .as_deref()
                .filter(|k| !k.contains('{'))
                .map(|k| crate::history::HistoryRecorder::new(k, dep.settings.history_entries));
            // The plotted series belong to the same selection (issue #64):
            // they start empty, and stop being fed when it goes away.
            sub.rate_series = crate::series::RateSampler::new();
            sub.series_leaf = None;
            sub.refresh_series(dep);
            let (Some(session), Some(key)) = (dep.session.clone(), key) else {
                return Task::none();
            };
            // Lazy value-on-demand: one fetch per selection, nothing
            // ambient (issue #85). Symbolic skeleton paths have no
            // concrete value to fetch.
            if key.contains('{') {
                return Task::none();
            }
            services::value::fetch(session, key)
        }
    }
}

fn toggle_watch(dep: &Deployment, obs: &mut Observation, path: String) -> Task<Message> {
    let Some(monitor) = obs.monitor.clone() else {
        return Task::none();
    };
    if let Some(id) = obs.my_watches.remove(&path) {
        obs.my_watch_paths.remove(&path);
        // A watch released mid-seed never gets its boundary (the engine
        // aborts the seed task) — forget it here too.
        obs.seeding.remove(&id);
        obs.seeding_paths.remove(&path);
        return services::watch::release(monitor, path, id);
    }
    // Watching seeds (issue #92): current state arrives before live
    // traffic, through the same merge discipline as everything else.
    let selector = scope::subtree_selector(&path);
    let policy = zenkey_fleet::SeedPolicy {
        timeout: dep.timeout(),
        ..Default::default()
    };
    services::watch::subtree(monitor, path, selector, policy)
}

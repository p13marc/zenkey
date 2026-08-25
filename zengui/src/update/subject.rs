//! One subject: chosen, observed, fetched, decoded (#175, #181).
//!
//! Five of six, and the reason is the causal chain the message group exists
//! for: `Select` ends in the `fetch_value` that produces `ValueFetched`,
//! which produces `ValueDecoded`. Pointing the window somewhere reveals a row
//! (`tree`), lands a pane (`work`), records history and rebuilds the chart
//! (`sub`), and caches the key's facts (`dep`). Watching moves the coverage
//! (`obs`).
//!
//! Since #181 there is exactly one [`Subject`], and every way of choosing one
//! — the tree, the echo line, the palette, the doctor finding, the node card —
//! raises the same message. What happens next branches on the subject, not on
//! who asked.

use std::sync::Arc;

use iced::Task;

use crate::message::{Message, RightPane, Subject, SubjectMsg};
use crate::scope;
use crate::services;
use crate::state::{Deployment, Observation, SubjectState, TreeState, Workspace};
use crate::view;

/// One key: chosen, observed, fetched, decoded.
pub(crate) fn update(
    dep: &mut Deployment,
    obs: &mut Observation,
    sub: &mut SubjectState,
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
        SubjectMsg::ValueFetched(key, outcome) => {
            // No base guard needed (#109 audit): the evidence is keyed by
            // the full wire key, which names its own base (an explorer runs
            // un-namespaced, RFC 09 §5).
            //
            // A landing for a subject the user has moved past is *superseded*
            // (#181), and the two things it must not do are here. It must not
            // flip the pane — the old "focus nit" — and it must not clear
            // `decoded`, which was the sharper defect hiding behind it: a late
            // reply for key A wiped key B's rendering while B was on screen,
            // and the pane then said "not asked" about a key it had answered.
            //
            // Since #257 the landing routes by key across every slot: a pin
            // that shares the follow slot's key gets the same answer — one
            // fetch, one decode, N surfaces — because the evidence is about
            // the key, not about who is showing it.
            let current = sub.follow.current.key() == Some(key.as_str());
            // A fetch lands the Inspector in view. Since #180 that means
            // restoring its dock if the user closed it — spoken as the same
            // `PaneSelected` the palette and the workbench strip send, so the
            // reveal is persisted by the one handler that persists layout.
            // Only the *follow* slot's fetch reveals (#257): a pinned
            // window's evidence is already on screen in that window.
            // (The doctor exception this used to carry is gone with the
            // doctor pane: its findings are a dock stream now, #183.)
            let reveal = if current && !work.docks.is_open(crate::prefs::DockRole::Inspector) {
                Task::done(Message::Workspace(
                    crate::message::WorkspaceMsg::PaneSelected(RightPane::Inspector),
                ))
            } else {
                Task::none()
            };
            let mut landed = false;
            for slot in sub.all_mut() {
                if slot.current.key() == Some(key.as_str()) {
                    slot.decoded = None;
                    slot.fetched = Some((key.clone(), outcome.clone()));
                    landed = true;
                }
            }
            // No slot asks about this key any more: the answer is real and
            // is kept on the follow slot — the asker — where the view renders
            // it as *superseded* rather than pretending nothing was asked.
            // No decode for it: it is work for a rendering nothing will
            // show, and `ValueDecoded`'s own guard would drop it anyway.
            if !landed {
                sub.follow.fetched = Some((key, outcome));
                return Task::none();
            }
            // One decode however many slots the answer landed in (#257): the
            // decode is keyed by the key, and `ValueDecoded` fans out again.
            // It runs with or without a registry: `slices: None` yields
            // `NotValidated(NoRegistry)` — "nobody looked" rendered as
            // itself rather than by omission (#164, #246).
            let decode_task = match (&outcome, &dep.session, &dep.schema_store) {
                (Ok(out), Some(session), Some(store)) => {
                    if let zenkey_fleet::FetchOutcome::Value(v) = out.as_ref() {
                        services::value::decode(services::value::Decode {
                            store: Arc::clone(store),
                            session: session.clone(),
                            slices: dep.slices.clone(),
                            base: dep.base().to_string(),
                            fetched_key: key.clone(),
                            wire_key: v.key.clone(),
                            encoding: v.encoding.clone(),
                            bytes: v.payload.clone(),
                        })
                    } else {
                        Task::none()
                    }
                }
                _ => Task::none(),
            };
            Task::batch([reveal, decode_task])
        }
        SubjectMsg::ValueDecoded(key, value) => {
            // The verdict cache learns every decode, current subject or not
            // (#164): the check ran and its result is a fact about the key,
            // not about the selection.
            work.verdicts
                .payloads
                .record(&key, value.sample.verdict.clone());
            // Stale guard, per slot (#257): the decode lands in every slot
            // still showing its key, and in none that moved on.
            for slot in sub.all_mut() {
                if slot.current.key() == Some(key.as_str()) {
                    slot.decoded = Some(Arc::clone(&value));
                }
            }
            Task::none()
        }
        SubjectMsg::Select(subject) => select(dep, sub, work, subject),
    }
}

/// Point the workspace at something, and do whatever that thing implies.
///
/// One function where there were three handlers, and it branches on the
/// [`Subject`] rather than on which pane asked (#181). Everything derived from
/// the old subject is dropped here — the latency summary, the history
/// recorder, the rate sampler and the chart — because none of it is evidence
/// about the new one.
///
/// Since #257 this is the *follow slot's* move, and only its: a selection is
/// the tree speaking, and the tree drives slot 0. The pinned slots are
/// exactly the subjects a selection must not touch.
fn select(
    dep: &Deployment,
    subs: &mut SubjectState,
    work: &mut Workspace,
    subject: Subject,
) -> Task<Message> {
    let sub = &mut subs.follow;
    sub.current = subject;
    // The old key's latency summary is not evidence about the new one —
    // cleared now, refreshed on the next tick (#119).
    sub.selected_latency = None;
    // History follows the subject and nothing else (issue #63): the previous
    // recording is dropped here, which is what makes deselecting free. A
    // symbolic skeleton path names no concrete key, so nothing can be
    // recorded for it.
    sub.history = sub
        .current
        .key()
        .filter(|k| !k.contains('{'))
        .map(|k| crate::history::HistoryRecorder::new(k, dep.settings.history_entries));
    // A new subject is a new timeline: it starts at the top rather than
    // wherever the last key's list happened to be scrolled (#183).
    sub.history_scroll = sub.history_scroll.to_top();
    // The plotted series belong to the same subject (issue #64): they start
    // empty, and stop being fed when it goes away.
    sub.rate_series = crate::series::RateSampler::new();
    sub.series_leaf = None;
    // The field observation (#223) and the why ladder (#214) explained the
    // old key; their window inputs survive, their reports do not.
    sub.fields.forget_subject();
    sub.why.forget_subject();
    sub.refresh_series(dep);

    let Some(session) = dep.session.clone() else {
        return Task::none();
    };
    match &sub.current {
        // Lazy value-on-demand: one fetch per subject, nothing ambient
        // (issue #85). Symbolic skeleton paths have no concrete value.
        Subject::Key(key) if !key.contains('{') => services::value::fetch(session, key.clone()),
        // The nodes pane's one data-plane cost, paid on selection.
        Subject::Origin(origin) => {
            work.verdicts.node_detail = view::nodes::DetailState::Loading(origin.clone());
            services::sweep::node_info(
                session,
                dep.base().to_string(),
                origin.clone(),
                dep.timeout(),
            )
        }
        // A prefix is not a key, and neither is nothing.
        _ => Task::none(),
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

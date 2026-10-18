//! One subject: chosen, observed, fetched, decoded (#175, #181).
//!
//! Five of six, and the reason is the causal chain the message group exists
//! for: `Select` ends in the value read that produces `ValueFetched`, which
//! produces `ValueDecoded`. Pointing the window somewhere reveals a row
//! (`tree`), lands a pane (`work`), records history and rebuilds the chart
//! (`sub`), and caches the key's identity (`dep`). Watching moves the
//! coverage (`obs`).
//!
//! Since #181 there is exactly one [`Subject`], and every way of choosing one
//! — the tree, the echo line, the palette, the doctor finding, the node card —
//! raises the same message. What happens next branches on the subject, not on
//! who asked.
//!
//! **The read is the key's** (#614, FL8a): [`read_value`] asks the question
//! its identity poses — S4's owner GET, an `@adv` history, a raw GET — and
//! during a replay asks the bus nothing, showing the capture's latest row
//! instead ([`recorded_value`]), checked through the replay's own lens.

use std::sync::Arc;

use iced::Task;

use crate::message::{Message, RightPane, Subject, SubjectMsg};
use crate::replay::ReplayState;
use crate::scope;
use crate::services;
use crate::state::subject::SubjectSlot;
use crate::state::{Deployment, Observation, SubjectState, TreeState, Workspace};

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
            // A live read never lands while the panes show a capture
            // (#614, FL8a): it answers about the bus, and a replay's slots
            // show the recorded row instead. Its question is no longer
            // pending either way.
            for slot in sub.all_mut() {
                if slot.asked.as_ref().is_some_and(|(k, _)| *k == key) {
                    slot.asked = None;
                }
            }
            if work.replay.replay.is_some() {
                return Task::none();
            }
            // A landing for a subject the user has moved past is *superseded*
            // (#181), and the two things it must not do are here. It must not
            // flip the pane — the old "focus nit" — and it must not clear
            // `decoded`, which was the sharper defect hiding behind it: a late
            // reply for key A wiped key B's rendering while B was on screen,
            // and the pane then said "not asked" about a key it had answered.
            //
            // Since #257 the landing routes by key across every slot: a pin
            // that shares the follow slot's key gets the same answer — one
            // read, one decode, N surfaces — because the evidence is about
            // the key, not about who is showing it.
            let current = sub.follow.current.key() == Some(key.as_str());
            // A read lands the Inspector in view. Since #180 that means
            // restoring its dock if the user closed it — spoken as the same
            // `PaneSelected` the palette and the workbench strip send, so the
            // reveal is persisted by the one handler that persists layout.
            // Only the *follow* slot's read reveals (#257): a pinned
            // window's evidence is already on screen in that window.
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
            let Ok(read) = &outcome else {
                return reveal;
            };
            // A state read came back decoded (#614, FL8a): the fleet
            // rendered the owner's reply through the revision the key
            // resolved to, and its task checked it. The verdict cache learns
            // it like any decode (#164).
            if let crate::value::ValueRead::Owner(answer) = read.as_ref() {
                if let Some(decoded) = &answer.decoded {
                    work.verdicts
                        .payloads
                        .record(&key, decoded.checked.conformance.clone());
                    for slot in sub.all_mut() {
                        if slot.current.key() == Some(key.as_str()) {
                            slot.decoded = Some(Arc::clone(decoded));
                        }
                    }
                }
                return reveal;
            }
            // One check however many slots the answer landed in (#257): the
            // check is keyed by the key, and `ValueDecoded` fans out again.
            // Through the lens (#614, FL6a), which needs no session: a key
            // no contract reaches is checked too, and says at which rung it
            // stopped — "nobody looked" rendered as itself rather than by
            // omission (#164, #246). A tombstone carries nothing to check.
            let decode_task = match read.shown() {
                Some(reply) if !reply.delete => services::value::decode(services::value::Decode {
                    lens: dep.lens_inputs(),
                    fetched_key: key.clone(),
                    wire_key: reply.key.clone(),
                    encoding: reply.encoding.clone(),
                    bytes: reply.payload.clone(),
                }),
                _ => Task::none(),
            };
            Task::batch([reveal, decode_task])
        }
        SubjectMsg::ValueDecoded(key, value) => {
            // A live check never lands in a replay's panes (#614, FL8a):
            // they show the capture, checked through its own lens.
            if work.replay.replay.is_some() {
                return Task::none();
            }
            // The verdict cache learns every decode, current subject or not
            // (#164): the check ran and its result is a fact about the key,
            // not about the selection.
            work.verdicts
                .payloads
                .record(&key, value.checked.conformance.clone());
            // Stale guard, per slot (#257): the decode lands in every slot
            // still showing its key, and in none that moved on.
            for slot in sub.all_mut() {
                if slot.current.key() == Some(key.as_str()) {
                    slot.decoded = Some(Arc::clone(&value));
                }
            }
            Task::none()
        }
        SubjectMsg::RecordedDecoded(id, key, value) => {
            // Only the replay it was checked for takes it (#614, FL8a), as
            // its verdict batches do (FL10b): a check outliving its replay
            // is about a file no pane shows.
            if work.replay.replay.as_ref().is_none_or(|r| r.id != id) {
                return Task::none();
            }
            work.verdicts
                .payloads
                .record(&key, value.checked.conformance.clone());
            for slot in sub.all_mut() {
                let recorded = slot
                    .fetched
                    .as_ref()
                    .is_some_and(|(k, o)| *k == key && o.as_ref().is_ok_and(|r| r.is_recorded()));
                if recorded && slot.current.key() == Some(key.as_str()) {
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
    dep: &mut Deployment,
    subs: &mut SubjectState,
    work: &mut Workspace,
    subject: Subject,
) -> Task<Message> {
    // The selection's identity, at once (#614, FL7): a declared key nobody
    // has published resolves when it is selected, not never — through the
    // replay's offline lens while one runs, the live one otherwise. Never a
    // symbolic skeleton path, which is no key.
    if let Subject::Key(key) = &subject
        && !key.contains('{')
    {
        match work.replay.replay.as_mut() {
            Some(replay) => replay.ensure_identity(key),
            None => dep.ensure_identity(key),
        }
    }
    let sub = &mut subs.follow;
    sub.current = subject;
    // The old key's read in flight and its archive's answer are about the
    // old key (#614, FL8a); a read that lands for it is superseded.
    sub.asked = None;
    sub.last_known = None;
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
    // The field observation (#223), the why ladder (#214) and the roles
    // bound to the subject (#224) explained the old subject; their window
    // inputs survive, their reports do not. The bindings are computed on
    // the press (#614, FL8b), so nothing in flight guards them.
    sub.fields.forget_subject();
    sub.why.forget_subject();
    sub.consumers.forget_subject();
    sub.refresh_series(dep);

    match &sub.current {
        // Lazy value-on-demand: one read per subject, nothing ambient
        // (issue #85), and which read is the key's identity's (#614, FL8a):
        // a symbolic skeleton path has no value, and an operation's key has
        // none either — a GET on it is a call (spec §5, #614 FL6b). During a
        // replay the value is the capture's, and the bus is asked nothing.
        Subject::Key(_) => read_value(dep, sub, work.replay.replay.as_ref()),
        // The nodes pane's one data-plane cost, paid on selection (#614,
        // FL9; D11): one `health.v1` reading of the address, on the session
        // in the namespace — never polled.
        // And the Config tool, when it is the Workbench's, follows the
        // address (#768, CD): one more reading, of what the window points at.
        Subject::Address(addr) => {
            let addr = addr.clone();
            let health = super::pane::nodes::health(&mut work.verdicts, dep, addr.clone());
            let config = super::pane::configure::follow(
                &mut work.verdicts.config,
                work.right_pane,
                dep,
                addr,
            );
            Task::batch([health, config])
        }
        // A prefix is not a key, and neither is nothing.
        _ => Task::none(),
    }
}

/// Read `slot`'s key for its value, by resolution (#614, FL8a): S4's owner
/// GET for a state key, the owner's `@adv` history for a stream declaring
/// one, a raw GET for a foreign key — [`crate::value::plan`]'s choice, made
/// from the key's identity. Nothing is asked for what the plan refuses, or
/// without the session the read needs, and `slot.asked` then stays empty:
/// the pane says what would have been asked, and why it was not.
///
/// While `replay` runs the bus is asked nothing: the value is the capture's
/// latest row for the key at the playhead ([`recorded_value`]).
pub(crate) fn read_value(
    dep: &Deployment,
    slot: &mut SubjectSlot,
    replay: Option<&ReplayState>,
) -> Task<Message> {
    use crate::value::{ReadKind, ReadPlan};

    let Some(key) = slot.current.key().map(str::to_owned) else {
        return Task::none();
    };
    if let Some(replay) = replay {
        return recorded_value(slot, replay);
    }
    let resolution = dep.identity.get(&key);
    let timeout = dep.timeout();
    let (kind, task) = match crate::value::plan(&key, resolution) {
        ReadPlan::Owner => match (dep.session.clone(), resolution.cloned()) {
            (Some(session), Some(resolution)) => (
                ReadKind::Owner,
                services::value::owner_get(
                    session,
                    dep.lens_inputs(),
                    key.clone(),
                    resolution,
                    timeout,
                ),
            ),
            _ => return Task::none(),
        },
        ReadPlan::History => match dep.raw.clone() {
            Some(raw) => (
                ReadKind::History,
                services::value::history(raw, dep.lens_inputs(), key.clone(), timeout),
            ),
            None => return Task::none(),
        },
        ReadPlan::Raw => match dep.raw.clone() {
            Some(raw) => (
                ReadKind::Raw,
                services::value::raw_get(raw, dep.lens_inputs(), key.clone(), timeout),
            ),
            None => return Task::none(),
        },
        ReadPlan::NotRead(_) => return Task::none(),
    };
    slot.asked = Some((key, kind));
    task
}

/// The capture's latest row for `slot`'s key at `replay`'s playhead, as the
/// slot's value (#614, FL8a): checked through the replay's offline lens,
/// never the live one (O4), and nothing asked of the bus. A row already
/// shown is not checked again; no row up to the playhead leaves the slot
/// with no value, which the pane says is the capture's silence.
pub(crate) fn recorded_value(slot: &mut SubjectSlot, replay: &ReplayState) -> Task<Message> {
    use crate::value::{Reply, ValueRead};

    slot.asked = None;
    let Some(key) = slot
        .current
        .key()
        .filter(|k| !k.contains('{'))
        .map(str::to_owned)
    else {
        return Task::none();
    };
    let Some(row) = replay.latest(&key) else {
        slot.fetched = None;
        slot.decoded = None;
        return Task::none();
    };
    let shown = slot.fetched.as_ref().is_some_and(|(k, o)| {
        *k == key
            && matches!(o, Ok(read) if matches!(read.as_ref(),
                ValueRead::Recorded { view, .. } if Arc::ptr_eq(view, &row.view)))
    });
    if shown {
        return Task::none();
    }
    let reply = Reply::of_view(&row.view);
    let check = if reply.delete {
        Task::none()
    } else {
        let id = replay.id;
        services::value::decode_then(
            services::value::Decode {
                lens: replay.lens.inputs(),
                fetched_key: key.clone(),
                wire_key: reply.key.clone(),
                encoding: reply.encoding.clone(),
                bytes: reply.payload.clone(),
            },
            move |key, value| Message::Subject(SubjectMsg::RecordedDecoded(id, key, value)),
        )
    };
    slot.fetched = Some((
        key,
        Ok(Arc::new(ValueRead::Recorded {
            reply,
            view: Arc::clone(&row.view),
            t_us: row.t_us,
        })),
    ));
    slot.decoded = None;
    check
}

/// Every slot's recorded value at the playhead (#614, FL8a): what each
/// replayed tick asks after the panes have folded it.
pub(crate) fn recorded_values(subs: &mut SubjectState, work: &Workspace) -> Task<Message> {
    let Some(replay) = work.replay.replay.as_ref() else {
        return Task::none();
    };
    Task::batch(
        subs.all_mut()
            .map(|slot| recorded_value(slot, replay))
            .collect::<Vec<_>>(),
    )
}

/// Leaving a replay (#614, FL8a): every slot that showed a recorded row —
/// or the capture's silence about its key — reads its key live again,
/// resolved through the live lens: the capture's evidence is not the bus's,
/// and stays with the file.
pub(crate) fn live_values(dep: &mut Deployment, subs: &mut SubjectState) -> Task<Message> {
    let mut tasks = Vec::new();
    for slot in subs.all_mut() {
        let from_the_file = slot
            .fetched
            .as_ref()
            .is_none_or(|(_, o)| o.as_ref().is_ok_and(|r| r.is_recorded()));
        if !from_the_file {
            continue;
        }
        slot.fetched = None;
        slot.decoded = None;
        if let Some(key) = slot.current.key().filter(|k| !k.contains('{')) {
            dep.ensure_identity(key);
        }
        tasks.push(read_value(dep, slot, None));
    }
    Task::batch(tasks)
}

fn toggle_watch(dep: &Deployment, obs: &mut Observation, path: String) -> Task<Message> {
    let Some(monitor) = obs.monitor.clone() else {
        return Task::none();
    };
    // No watch under `@op` (#614, FL6b): a queryable carries no samples, and
    // the seed's state GET on it would call every operation it reaches. The
    // tree offers no control there; this refuses the message anyway.
    if scope::names_operation(&path) && !obs.my_watches.contains_key(&path) {
        return Task::none();
    }
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

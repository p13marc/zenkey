//! The detail and history sections — the two with no state of their own.
//!
//! Both are windows onto one subject slot, so both take `&mut SubjectSlot`
//! and nothing else (#257: the routing from a `SlotId` to the slot happens
//! once, in [`super::update`]). That is the finding, not an economy: nine of
//! the eleven panes had state to dock when #180 landed, and these two had
//! nothing.
//!
//! `Deployment` is here for the chart rebuild, and since #614 (FL8a) for the
//! two reads the Detail section asks by button: an archive's last-known
//! state (S5) and an event's replay (§2.6) — both on the deployment's
//! sessions, and neither while a replay runs, which asks the bus nothing.

use iced::Task;

use crate::message::Message;
use crate::services;
use crate::state::Deployment;
use crate::state::subject::SubjectSlot;
use crate::state::workspace::ReplayMode;
use crate::view::detail::{DetailMsg, LastKnown};
use crate::view::history::HistoryMsg;

/// The Detail section's own acts: the sparkline's leaf (issue #64), and the
/// reads asked by button (#614, FL8a).
pub(crate) fn update(
    slot: &mut SubjectSlot,
    dep: &Deployment,
    replay: &ReplayMode,
    msg: DetailMsg,
) -> Task<Message> {
    let replaying = replay.replay.is_some();
    match msg {
        DetailMsg::LeafSelected(path) => {
            slot.series_leaf = Some(path);
            slot.refresh_series(replay.identity_cache(&dep.identity));
            Task::none()
        }
        DetailMsg::LastKnown(archive) => {
            // The replay lock (#614, FL8a): the panes show a capture, and an
            // archive's answer is about the bus.
            if replaying || slot.last_known.as_ref().is_some_and(|l| l.answer.is_none()) {
                return Task::none();
            }
            let Some(key) = slot.current.key().map(str::to_owned) else {
                return Task::none();
            };
            let fail = |slot: &mut SubjectSlot, why: &str| {
                slot.last_known = Some(LastKnown {
                    key: key.clone(),
                    archive: archive.clone(),
                    answer: Some(Err(why.into())),
                });
                Task::none()
            };
            let Some(resolution) = dep.identity.get(&key).filter(|r| r.resolved.is_some()) else {
                return fail(
                    slot,
                    "the key resolved short of its resource — a last-known read names its revision",
                );
            };
            let Some(session) = dep.session.clone() else {
                return fail(slot, "no session in the namespace — connect first");
            };
            let task = services::value::last_known(
                slot.id,
                session,
                dep.lens_inputs(),
                key.clone(),
                resolution.clone(),
                archive.clone(),
                dep.timeout(),
            );
            slot.last_known = Some(LastKnown {
                key,
                archive,
                answer: None,
            });
            task
        }
        DetailMsg::LastKnownRead(key, archive, answer) => {
            // Staleness guard: an answer for a key, or an archive, the slot
            // has moved past is dropped — the section was reset.
            if let Some(asked) = slot.last_known.as_mut()
                && asked.key == key
                && asked.archive == archive
                && slot.current.key() == Some(key.as_str())
            {
                asked.answer = Some(answer);
            }
            Task::none()
        }
        DetailMsg::ReplayEvents => {
            if replaying {
                return Task::none();
            }
            let Some(key) = slot.current.key().map(str::to_owned) else {
                return Task::none();
            };
            let plan = crate::value::plan(&key, dep.identity.get(&key));
            let crate::value::ReadPlan::NotRead(crate::value::NotRead::Event {
                retention_s: Some(retention_s),
            }) = plan
            else {
                return Task::none();
            };
            let Some(raw) = dep.raw.clone() else {
                slot.fetched = Some((key, Err("no session — connect first".into())));
                return Task::none();
            };
            slot.asked = Some((key.clone(), crate::value::ReadKind::Events));
            services::value::events(raw, dep.lens_inputs(), key, retention_s, dep.timeout())
        }
    }
}

/// The history section (#63). Both actions are about the recorder, and there
/// is nothing to do when the key is unselected — a recorder is created with
/// the selection and dropped with it.
pub(crate) fn history(slot: &mut SubjectSlot, msg: HistoryMsg) -> Task<Message> {
    // View-only state, and it is not the recorder's: a scroll offset survives
    // a `Clear` and a subject that has no recorder can still be scrolled to
    // the top.
    if let HistoryMsg::Scrolled(vp) = msg {
        slot.history_scroll = vp;
        return Task::none();
    }
    // View-only too (#219): which pair the panel shows, on this slot.
    if let HistoryMsg::CompareSnapshotToggled = msg {
        slot.compare_snapshot = !slot.compare_snapshot;
        return Task::none();
    }
    if let Some(rec) = slot.history.as_mut() {
        match msg {
            HistoryMsg::Select(seq) => rec.selected = Some(seq),
            HistoryMsg::Clear => {
                rec.ring.clear();
                rec.selected = None;
            }
            HistoryMsg::Scrolled(..) | HistoryMsg::CompareSnapshotToggled => {
                unreachable!("handled above")
            }
        }
    }
    Task::none()
}

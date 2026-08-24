//! Replay (#74) and capture: the same pane's two directions.
//!
//! It names five of the six sub-states, for the same reason
//! [`bus`](crate::update::bus) does — a replayed tick moves everything a live
//! one moves, because it goes through the same [`apply_tick`]. That is the
//! whole design of replay mode: nothing live bleeds through and nothing about
//! the panes knows which world it is in.
//!
//! [`apply_tick`]: crate::update::bus::apply_tick

use std::sync::Arc;

use iced::Task;

use crate::message::Message;
use crate::services;
use crate::state::workspace::RecordingHandle;
use crate::state::{Deployment, Observation, SubjectState, TreeState, Workspace};
use crate::update::bus;
use crate::view::replay::ReplayMsg;

/// Replay mode (issue #74). The transport verbs synthesize ticks from
/// the loaded file and push them through the exact pipeline the live
/// pump feeds — `apply_tick` — so the panes never know the difference.
pub(crate) fn update(
    dep: &mut Deployment,
    obs: &mut Observation,
    sub: &mut SubjectState,
    tree: &mut TreeState,
    work: &mut Workspace,
    msg: ReplayMsg,
) -> Task<Message> {
    match msg {
        ReplayMsg::OpenToggled => {
            work.replay.replay_open = match work.replay.replay_open {
                Some(_) => None,
                None => Some(String::new()),
            };
            work.replay.replay_note = None;
            Task::none()
        }
        ReplayMsg::PathChanged(s) => {
            if let Some(p) = &mut work.replay.replay_open {
                *p = s;
            }
            Task::none()
        }
        ReplayMsg::Open => {
            if work.replay.replay_loading.is_some() {
                // One parse at a time: a second click while one is in
                // flight would race two `Loaded` landings for one pane.
                return Task::none();
            }
            let Some(path) = work
                .replay
                .replay_open
                .as_deref()
                .map(str::trim)
                .filter(|p| !p.is_empty())
                .map(str::to_string)
            else {
                return Task::none();
            };
            // The parse runs off the update thread (#255) — it is bounded
            // only by the file, and it used to freeze the window for its
            // whole length. Until `Loaded` lands, the loading claim below
            // is what the replay tab shows: a load in flight is not an
            // empty capture and not a hung window (RFC 09 §5.1 O4).
            work.replay.replay_open = None;
            work.replay.replay_note = None;
            work.replay.replay_loading = Some(path.clone());
            services::record::load(path)
        }
        ReplayMsg::Loaded(path, result) => {
            work.replay.replay_loading = None;
            match result {
                Ok(loaded) => {
                    let Some(mut state) = loaded.take() else {
                        // A landing can only be consumed once, and this one
                        // already was — nothing left to show.
                        return Task::none();
                    };
                    // Mode honesty: the panes now show the file, from
                    // its start — nothing live bleeds through. Every slot's
                    // recorder was of the live world (#257): the pins' drop
                    // with the follow slot's.
                    work.echo.echo.clear();
                    for slot in sub.slots.iter_mut() {
                        slot.history = None;
                        slot.refresh_series(dep);
                    }
                    let tick = state.scrub_to(0);
                    work.replay.replay = Some(state);
                    bus::apply_tick(dep, obs, sub, tree, work, &tick);
                }
                Err(e) => {
                    // The open row comes back with the path that failed, so
                    // the note renders beside the box holding the mistake.
                    work.replay.replay_open = Some(path);
                    work.replay.replay_note = Some(e);
                }
            }
            Task::none()
        }
        ReplayMsg::Toggled => {
            let tick = work.replay.replay.as_mut().map(|r| {
                // Play at the end means "from the top" — the one
                // rewind that needs no scrubber.
                if !r.playing && r.position_us >= r.span_us {
                    let t = r.scrub_to(0);
                    r.playing = true;
                    Some(t)
                } else {
                    r.playing = !r.playing;
                    None
                }
            });
            if let Some(Some(tick)) = tick {
                work.echo.echo.clear();
                bus::apply_tick(dep, obs, sub, tree, work, &tick);
            }
            Task::none()
        }
        ReplayMsg::SpeedSelected(s) => {
            if let Some(r) = &mut work.replay.replay {
                r.speed = s.0;
            }
            Task::none()
        }
        ReplayMsg::Scrubbed(t_us) => {
            let rewound = work
                .replay
                .replay
                .as_ref()
                .is_some_and(|r| t_us < r.position_us);
            let tick = work.replay.replay.as_mut().map(|r| r.scrub_to(t_us));
            if let Some(tick) = tick {
                if rewound {
                    // Backwards is a rebuild (LWW does not invert), and
                    // the scrollback rebuilds with it.
                    work.echo.echo.clear();
                }
                bus::apply_tick(dep, obs, sub, tree, work, &tick);
            }
            Task::none()
        }
        ReplayMsg::Advance => {
            let tick = work
                .replay
                .replay
                .as_mut()
                .filter(|r| r.playing)
                .map(|r| r.advance(std::time::Duration::from_millis(250)));
            if let Some(tick) = tick {
                bus::apply_tick(dep, obs, sub, tree, work, &tick);
            }
            Task::none()
        }
        ReplayMsg::Exit => {
            work.replay.replay = None;
            // The next live tick repaints the tree; the scrollback must
            // not mix file lines into it.
            work.echo.echo.clear();
            Task::none()
        }
        ReplayMsg::RecordToggled => {
            if let Some(handle) = work.replay.recording.take() {
                // `send` cannot be lost even if the capture task has not been
                // polled once yet (#335); `Err` only means it already ended.
                let _ = handle.stop.send(());
                return Task::none();
            }
            if work.replay.replay.is_some() {
                // Recording captures the live monitor; replay mode — a file
                // or the retained window (#217) — has nothing live to
                // capture. Checked before the monitor, because it holds even
                // when one exists.
                return Task::none();
            }
            let Some(monitor) = obs.monitor.clone() else {
                return Task::none();
            };
            let (stop, stopped) = tokio::sync::oneshot::channel();
            let path = format!(
                "zengui-{}.zrec",
                zenkey_fleet::tape::record::rfc3339_now().replace(':', "-")
            );
            let base = dep.base().to_string();
            work.replay.recording = Some(RecordingHandle {
                stop,
                path: path.clone(),
            });
            work.replay.recorded = None;
            services::record::start(monitor, path, base, stopped)
        }
        ReplayMsg::RecordFinished(result) => {
            work.replay.recording = None;
            work.replay.recorded = Some(result);
            Task::none()
        }
        ReplayMsg::RetainedToggled => {
            match &work.replay.replay {
                // Back to live: identical to Exit, and routed through it so
                // the two ways out cannot drift apart.
                Some(state)
                    if matches!(state.source, crate::replay::ReplaySource::Retained { .. }) =>
                {
                    update(dep, obs, sub, tree, work, ReplayMsg::Exit)
                }
                // A file replay owns the scrubber; the toggle does nothing
                // until it is exited explicitly.
                Some(_) => Task::none(),
                None => {
                    let Some(core) = obs.monitor.as_ref().map(|m| Arc::clone(m.core())) else {
                        return Task::none();
                    };
                    enter_retained(dep, obs, sub, tree, work, &core);
                    Task::none()
                }
            }
        }
        ReplayMsg::SaveWindow => {
            let Some(state) = &work.replay.replay else {
                return Task::none();
            };
            // Only a retained window saves: a file replay already *is* the
            // file, and re-writing it would launder its drop ledger away.
            if !matches!(state.source, crate::replay::ReplaySource::Retained { .. }) {
                return Task::none();
            }
            let rows: Vec<Arc<zenkey_fleet::SampleView>> =
                state.rows.iter().map(|r| Arc::clone(&r.view)).collect();
            let path = format!(
                "zengui-window-{}.zrec",
                zenkey_fleet::tape::record::rfc3339_now().replace(':', "-")
            );
            work.replay.recorded = None;
            services::record::save_window(
                rows,
                state.fold_epoch,
                state.watched.to_vec(),
                dep.base().to_string(),
                path,
            )
        }
    }
}

/// Enter the retained window (#217): snapshot the ring and its own account
/// of itself, then drive the panes through the exact machinery a `.zrec`
/// gets — `scrub_to` + [`apply_tick`](bus::apply_tick). Entered at the
/// window's newest edge, because the gesture is "scrub *back* to before
/// you noticed".
///
/// Takes the core rather than the monitor so a headless test can drive it
/// with a session-less [`zenkey_fleet::MonitorCore`] — the same posture as
/// replay itself, and the seam #217's acceptance simulation enters through.
pub(crate) fn enter_retained(
    dep: &mut Deployment,
    obs: &mut Observation,
    sub: &mut SubjectState,
    tree: &mut TreeState,
    work: &mut Workspace,
    core: &Arc<zenkey_fleet::MonitorCore>,
) {
    let taken = core.retention();
    let window = core.retained();
    let mut state =
        crate::replay::ReplayState::from_retained(window, Arc::clone(&obs.watched), taken);
    // Mode honesty, exactly as opening a file: the panes now show the
    // window — nothing live bleeds through, and the scrollback restarts.
    // Every slot's recorder was of the live world (#257).
    work.echo.echo.clear();
    for slot in sub.slots.iter_mut() {
        slot.history = None;
        slot.refresh_series(dep);
    }
    let tick = state.scrub_to(state.span_us);
    work.replay.replay = Some(state);
    bus::apply_tick(dep, obs, sub, tree, work, &tick);
}

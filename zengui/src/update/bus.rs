//! Everything the world answered: a task landing, or a `link.rs` yield (#175).
//!
//! Both [`update`] and [`apply_tick`] name five of the six sub-states, and
//! that is the measurement rather than a lapse: a bus tick moves everything
//! the bus can move. Only [`Chrome`](crate::state::Chrome) is untouched, which
//! is exactly the claim `Chrome` exists to make.
//!
//! The replay pane calls `apply_tick` too, with a tick read from a file rather than
//! from the pump — the panes never know the difference, which is what makes
//! replay a *mode* and not a second implementation.

use std::sync::Arc;

use iced::Task;

use crate::message::{BusMsg, BusTick, LinkState, Message};
use crate::scope;
use crate::services;
use crate::state::tree::shape_held;
use crate::state::{Deployment, Observation, Subject, TreeState, Workspace};
use crate::view;
use crate::view::status::SliceSource;

pub(crate) fn apply_tick(
    dep: &mut Deployment,
    obs: &mut Observation,
    sub: &mut Subject,
    tree: &mut TreeState,
    work: &mut Workspace,
    tick: &BusTick,
) {
    // Decided *before* the fields below are overwritten (#177).
    let held = shape_held(
        (obs.keys, obs.keys_evicted, obs.keys_unwatched),
        (tick.keys, tick.keys_evicted, tick.keys_unwatched),
        &obs.watched,
        &tick.watched,
    );
    // Per tick, not per frame: one bounded lock for one key's latency
    // summary (#119). None when unselected, unobserved, or unstamped.
    sub.selected_latency = match (&sub.selected, &obs.monitor) {
        // During replay the live monitor's stats are about a different
        // world than the panes are showing — consulting them would put
        // live latency under file data (O4 in miniature).
        (Some(_), Some(_)) if work.replay.replay.is_some() => None,
        (Some(key), Some(monitor)) => monitor
            .core()
            .with_stats(|s| s.get(key).map(|k| (k.latency(), k.unstamped)))
            .and_then(|(lat, unstamped)| lat.map(|l| (l, unstamped))),
        _ => None,
    };
    obs.observed = Arc::clone(&tick.tree);
    obs.keys = tick.keys;
    obs.keys_evicted = tick.keys_evicted;
    obs.keys_unwatched = tick.keys_unwatched;
    obs.totals = tick.totals;
    obs.watched = std::sync::Arc::clone(&tick.watched);
    for (id, coverage) in &tick.seeded {
        if let Some(path) = obs.seeding.remove(id) {
            if let Some(path) = path {
                obs.seeding_paths.remove(&path);
            }
            obs.seed_totals.0 += coverage.history_replies.unwrap_or(0);
            obs.seed_totals.1 += coverage.storage_replies.unwrap_or(0);
            obs.seed_totals.2 += coverage.superseded;
            obs.seeded_watches += 1;
        }
    }
    // Two different facts about the same window (O6): the broadcast
    // outran us vs. our own batch cap chose to coalesce.
    work.echo.echo.record_lag(tick.lagged);
    work.echo.echo.record_coalesced(tick.coalesced);
    // One point per tick for the selected key's rate (issue #64). The
    // count is what says whether the EWMA moved: it never decays on its
    // own, so an unchanged count is silence, and the sampler records a gap
    // rather than a confident flat line.
    if let Some(rec) = sub.history.as_ref() {
        let chunks: Vec<&str> = rec.key.split('/').collect();
        let observed = tick.tree.node(&chunks).map(|n| (n.count, n.rate_hz));
        sub.rate_series.tick(observed);
    }
    for sample in &tick.samples {
        ensure_facts(dep, &sample.key);
        work.echo.echo.push(sample);
        // History is per-key and costs no subscription of its own: these
        // samples are already flowing for an existing watch (issue #63).
        if let Some(rec) = sub.history.as_mut() {
            rec.observe(sample);
        }
        // The media viewer's frames arrive on the exact key it watches
        // (issue #69) — same pipeline, no extra subscription.
        if let Some(v) = work.bench.media.viewing.as_mut()
            && v.key == sample.key
        {
            v.on_frame(sample);
        }
    }
    for (key, _up) in &tick.nodes {
        ensure_facts(dep, key);
    }
    // The node dashboard (#61): transitions in arrival order (flap-
    // correct), then the zero-cost watched-freshness join.
    let now = std::time::Instant::now();
    work.verdicts
        .roster
        .apply_transitions(dep.base(), &tick.nodes, now);
    work.verdicts
        .roster
        .refresh(&tick.tree, dep.base(), &tick.watched, now);
    // The chart's inputs all advanced above — the history ring, the rate
    // sampler, the facts behind the unit. Rebuilt once here rather than
    // once per frame (#178).
    sub.refresh_series(dep);
    // The tree's shape survived, so point it at this tick's numbers
    // instead of walking 50,000 nodes to move eight of them (#177).
    // `retarget` refuses a pivot, which is the other half of the
    // condition: those rebuild every tick, exactly as before.
    let now = std::time::Instant::now();
    if held
        && tree
            .flat
            .retarget(std::sync::Arc::clone(&obs.observed), now)
    {
        tree.shape_reused += 1;
    } else {
        tree.shape_rebuilt += 1;
        tree.reflatten(dep, obs);
    }
}

pub(crate) fn ensure_facts(dep: &mut Deployment, key: &str) {
    // One line, and the bound lives in the engine with the counter that
    // reports it (#107). This is still the single insert point.
    let base = dep.base().to_string();
    dep.facts.ensure(&base, key, dep.slices.as_deref());
}

/// Everything the world answered — a task landing or a `link.rs` yield.
pub(crate) fn update(
    dep: &mut Deployment,
    obs: &mut Observation,
    sub: &mut Subject,
    tree: &mut TreeState,
    work: &mut Workspace,
    msg: BusMsg,
) -> Task<Message> {
    match msg {
        BusMsg::SessionOpened(Ok(session)) => {
            // A new session is a new everything: the base may differ, so
            // every projection, roster and verdict from the old one is
            // evidence about a different deployment (O4).
            //
            // This variant has exactly two producers — the launch open and
            // the context switch — and forgetting on the launch one is a
            // no-op over a `Deployment` nothing has written yet. So it can
            // live here, which removes the worst cross-group reach in the
            // file: a pane resetting the app.
            super::deployment::forget(dep, obs, tree, work);
            dep.session = Some(session.clone());
            // The context list is read from the shared file, not cached at
            // launch: `zenctl context create` on the other side of the
            // screen should show up here without a restart (#67).
            super::pane::context::refresh_contexts(&mut work.bench.context_form);
            dep.schema_store = Some(Arc::new(zenkey_fleet::decode::SchemaStore::new(
                dep.base(),
                dep.timeout(),
            )));
            let discover = services::link::discover_bases(&session, dep.timeout());
            Task::batch([discover, start_monitor(dep), load_slices(dep)])
        }
        BusMsg::SessionOpened(Err(e)) => {
            obs.link = LinkState::Failed(e);
            Task::none()
        }
        BusMsg::MonitorStarted(Ok(monitor)) => {
            obs.monitor = Some(Arc::clone(&monitor));
            obs.epoch += 1;
            if dep.settings.eager {
                return super::deployment::watch_scope(dep, obs);
            }
            Task::none()
        }
        BusMsg::MonitorStarted(Err(e)) => {
            obs.link = LinkState::Failed(e);
            Task::none()
        }
        BusMsg::SkeletonBuilt(Ok((skeleton, roster))) => {
            dep.skeleton = Some(skeleton);
            // The build task gathered the roster anyway — seed the node
            // dashboard from it instead of throwing it away (#61).
            work.verdicts.roster.seed(&roster);
            tree.reflatten(dep, obs);
            Task::none()
        }
        BusMsg::SkeletonBuilt(Err(e)) => {
            tracing::warn!("skeleton build failed: {e}");
            Task::none()
        }
        BusMsg::BasesDiscovered(Ok(bases)) => {
            dep.bases = bases;
            rebuild_base_options(dep);
            Task::none()
        }
        BusMsg::BasesDiscovered(Err(e)) => {
            tracing::warn!("base discovery failed: {e}");
            Task::none()
        }
        BusMsg::SlicesLoaded(Ok(slices)) => {
            dep.slice_source = if dep.settings.registry.is_empty() {
                SliceSource::Bus {
                    count: slices.slices().len(),
                }
            } else {
                SliceSource::Dirs {
                    count: slices.slices().len(),
                }
            };
            dep.slices = Some(slices);
            reresolve_registrations(dep);
            refresh_blob_list(dep, work);
            // The skeleton is built FROM the slices — (re)build it now.
            build_skeleton(dep)
        }
        BusMsg::SlicesUnionLoaded(Ok((slices, from_bus, dirs_only, disagreements))) => {
            dep.slice_source = SliceSource::Union {
                from_bus,
                dirs_only,
                disagreements,
            };
            dep.slices = Some(slices);
            reresolve_registrations(dep);
            refresh_blob_list(dep, work);
            build_skeleton(dep)
        }
        BusMsg::SlicesUnionLoaded(Err(e)) => {
            dep.slice_source = SliceSource::Failed(e);
            Task::none()
        }
        BusMsg::SlicesLoaded(Err(e)) => {
            dep.slice_source = SliceSource::Failed(e);
            Task::none()
        }
        BusMsg::Link(state) => {
            obs.link = state;
            Task::none()
        }
        BusMsg::Tick(tick) => {
            apply_tick(dep, obs, sub, tree, work, &tick);
            Task::none()
        }
    }
}

/// (Re)build the skeleton: slices are already loaded; roster + admin are
/// gathered inside the task (both metadata-only).
pub(crate) fn build_skeleton(dep: &Deployment) -> Task<Message> {
    let (Some(session), Some(slices)) = (dep.session.clone(), dep.slices.clone()) else {
        return Task::none();
    };
    let base = dep.base().to_string();
    let timeout = dep.timeout();
    services::sweep::skeleton(session, base, slices, timeout)
}

pub(crate) fn load_slices(dep: &Deployment) -> Task<Message> {
    let dirs = dep.settings.registry.clone();
    let Some(session) = dep.session.clone() else {
        return Task::none();
    };
    let base = dep.base().to_string();
    let timeout = dep.timeout();
    if !dirs.is_empty() {
        // The §6.1 union (issue #43): served wins, dirs fill, and the
        // disagreement count reaches the status strip as data.
        return services::sweep::slices_union(session, base, dirs, timeout);
    }
    services::sweep::slices(session, base, timeout)
}

pub(crate) fn start_monitor(dep: &Deployment) -> Task<Message> {
    let Some(session) = dep.session.clone() else {
        return Task::none();
    };
    // Liveliness is always on — zero payload by construction (RFC 04 §5)
    // — and needs both the fleet sweep and @catalog by name (D4).
    let liveliness = if dep.base().is_empty() && dep.bases.is_empty() {
        scope::liveliness_any_base()
    } else {
        scope::liveliness_selectors(dep.base())
    };
    services::watch::start_monitor(session, liveliness, dep.settings.max_keys)
}

/// Reproject the registry's `[[blob]]` declarations after a slice load.
///
/// Costs nothing on the bus: it reads slices already in hand and joins them
/// against the roster already observed. The laziness rule (#84/#85) governs
/// *fetching*, not rendering what has arrived — and the roster's
/// `live_map()` returns `None` while unseeded, so an unasked join renders
/// as "not asked" rather than as "nobody serves it" (O4).
pub(crate) fn refresh_blob_list(dep: &Deployment, work: &mut Workspace) {
    let Some(slices) = dep.slices.as_deref() else {
        work.verdicts.blob.list = None;
        return;
    };
    let source = match dep.slice_source {
        view::status::SliceSource::Union { .. } => zenkey_fleet::report::BlobListSource::Union,
        view::status::SliceSource::Dirs { .. } => {
            zenkey_fleet::report::BlobListSource::RegistryDirs
        }
        _ => zenkey_fleet::report::BlobListSource::Bus,
    };
    work.verdicts.blob.list = Some(zenkey_fleet::blob_list(
        slices.slices(),
        work.verdicts.roster.live_map().as_ref(),
        source,
    ));
}

pub(crate) fn reresolve_registrations(dep: &mut Deployment) {
    let Some(slices) = dep.slices.clone() else {
        return;
    };
    dep.facts.resolve_all(&slices);
}

/// The base picker's options, after a discovery sweep landed.
pub(crate) fn rebuild_base_options(dep: &mut Deployment) {
    dep.base_options = vec![String::new()];
    dep.base_options
        .extend(dep.bases.iter().map(|b| b.base.clone()));
    dep.base_options.dedup();
}

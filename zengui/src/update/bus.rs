//! The tick, and the facts cache it feeds (#175).
//!
//! [`apply_tick`] names five of the six sub-states, and that is the
//! measurement rather than a lapse: a bus tick moves everything the bus can
//! move. Only [`Chrome`](crate::state::Chrome) is untouched, which is exactly
//! the claim `Chrome` exists to make.
//!
//! The replay pane calls this too, with a tick read from a file rather than
//! from the pump — the panes never know the difference, which is what makes
//! replay a *mode* and not a second implementation.

use std::sync::Arc;

use crate::message::BusTick;
use crate::state::tree::shape_held;
use crate::state::{Deployment, Observation, Subject, TreeState, Workspace};

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

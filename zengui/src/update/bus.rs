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
use crate::state::{Deployment, Observation, SubjectState, TreeState, Workspace};
use crate::view;
use crate::view::status::SliceSource;

pub(crate) fn apply_tick(
    dep: &mut Deployment,
    obs: &mut Observation,
    sub: &mut SubjectState,
    tree: &mut TreeState,
    work: &mut Workspace,
    tick: &BusTick,
) {
    // The verdict cache's logical clock (#164): one advance per tick —
    // replayed ticks included, harmlessly (revalidation staleness only).
    work.verdicts.payloads.advance();
    // Decided *before* the fields below are overwritten (#177).
    let held = shape_held(
        (obs.keys, obs.keys_evicted, obs.keys_unwatched),
        (tick.keys, tick.keys_evicted, tick.keys_unwatched),
        &obs.watched,
        &tick.watched,
    );
    // Per tick, not per frame: one bounded lock per *slot* for its key's
    // latency summary (#119, #257 — the fan-out is bounded by the pin
    // count, not the bus). None when unselected, unobserved, or unstamped.
    for slot in sub.slots.iter_mut() {
        slot.selected_latency = match (slot.current.key(), &obs.monitor) {
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
    }
    // The retained window's account of itself (#217), once per tick like
    // the latency lookup above — and only on *live* ticks: a replayed tick
    // must not read the live ring under file (or window) data, and the
    // strip does not render it in replay mode anyway.
    if work.replay.replay.is_none()
        && let Some(monitor) = &obs.monitor
    {
        obs.retention = Some(monitor.core().retention());
    }
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
    // One point per tick per slot key's rate (issue #64). The
    // count is what says whether the EWMA moved: it never decays on its
    // own, so an unchanged count is silence, and the sampler records a gap
    // rather than a confident flat line.
    for slot in sub.slots.iter_mut() {
        if let Some(rec) = slot.history.as_ref() {
            let chunks: Vec<&str> = rec.key.split('/').collect();
            let observed = tick.tree.node(&chunks).map(|n| (n.count, n.rate_hz));
            slot.rate_series.tick(observed);
        }
    }
    for sample in &tick.samples {
        ensure_facts(dep, &sample.key);
        work.echo.echo.push(sample);
        // History is per-key and costs no subscription of its own: these
        // samples are already flowing for an existing watch (issue #63).
        // Every slot's recorder is fed from this one stream (#257) — a pin
        // costs a bounded ring, never a second subscription — and each
        // recorder keeps only its own key's samples.
        for slot in sub.slots.iter_mut() {
            if let Some(rec) = slot.history.as_mut() {
                rec.observe(sample);
            }
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
    // sampler, the facts behind the unit. Rebuilt once per slot here rather
    // than once per frame (#178, #257).
    for slot in sub.slots.iter_mut() {
        slot.refresh_series(dep);
    }
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
    sub: &mut SubjectState,
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
            // screen should show up here without a restart (#67). As a task
            // (#255): the re-read is right, and a TOML read on the path of
            // every session open — launch and each reconnect — sat a disk
            // access on a frame. Its answer lands on `ContextMsg::Refreshed`.
            let contexts = services::context::refresh();
            dep.schema_store = Some(Arc::new(zenkey_fleet::model::decode::SchemaStore::new(
                dep.base(),
                dep.timeout(),
            )));
            let discover = services::link::discover_bases(&session, dep.timeout());
            Task::batch([discover, contexts, start_monitor(dep), load_slices(dep)])
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
            forget_judgements(obs, work);
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
            forget_judgements(obs, work);
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
            // Live ticks only, deliberately: replay feeds `apply_tick` from
            // a file (`update/pane/replay.rs`) and never reaches this arm —
            // validating file data against a live bus, or re-judging a live
            // budget under file keys, would put one world's verdicts under
            // the other's data (O4 in miniature).
            Task::batch([
                schedule_validation(dep, work, &tick),
                schedule_budget(dep, obs, work),
            ])
        }
        BusMsg::VerdictsChecked(batch) => {
            // The bounded validation batch lands (#164): render paths only
            // ever look these up.
            for (key, verdict) in batch {
                work.verdicts.payloads.record(&key, verdict);
            }
            Task::none()
        }
        BusMsg::BudgetJoined(badges) => {
            obs.budgets = Some(badges);
            Task::none()
        }
    }
}

/// Pick this tick's bounded validation batch (#164): the newest sample per
/// distinct key, keys the cache wants checked **first-come**, capped at
/// [`crate::verdict::VALIDATE_PER_TICK`] — a hot bus costs a fixed slice of
/// CPU per tick, never a proportional one. Tombstones carry no payload to
/// validate, and payloads past [`crate::verdict::VALIDATE_LIMIT`] stay
/// unchecked (which renders as unchecked, not as fine).
///
/// First-come is now what the code does rather than what its comment claimed
/// (#359). The batch used to be drained out of a `HashMap`, whose iteration
/// order is the per-process hash seed: on any bus with more than sixteen
/// unchecked keys in a tick, *which* sixteen got checked varied run to run,
/// the badges filled in a different order every launch, and no test could pin
/// any of it. The keys go in the order the bus delivered them, and only the
/// payload each carries is the newest one the tick holds.
fn validation_batch(
    samples: &[Arc<zenkey_fleet::SampleView>],
    cache: &crate::verdict::VerdictCache,
) -> Vec<(String, String, zenoh::bytes::ZBytes)> {
    // Newest per key, in first-arrival key order: `newest` supersedes,
    // `order` remembers when the key first showed up.
    let mut newest: std::collections::HashMap<&str, &Arc<zenkey_fleet::SampleView>> =
        std::collections::HashMap::new();
    let mut order: Vec<&str> = Vec::new();
    for sample in samples {
        if sample.kind == zenoh::sample::SampleKind::Delete
            || sample.payload.len() > crate::verdict::VALIDATE_LIMIT
        {
            continue;
        }
        if newest.insert(sample.key.as_str(), sample).is_none() {
            order.push(sample.key.as_str());
        }
    }
    order
        .into_iter()
        .filter_map(|key| newest.get(key))
        .filter(|s| cache.should_check(&s.key))
        .take(crate::verdict::VALIDATE_PER_TICK)
        .map(|s| (s.key.clone(), s.encoding.clone(), s.payload.clone()))
        .collect()
}

fn schedule_validation(dep: &Deployment, work: &Workspace, tick: &BusTick) -> Task<Message> {
    let (Some(session), Some(store)) = (dep.session.clone(), dep.schema_store.clone()) else {
        return Task::none();
    };
    let batch = validation_batch(&tick.samples, &work.verdicts.payloads);
    if batch.is_empty() {
        return Task::none();
    }
    services::value::validate(
        store,
        session,
        dep.slices.clone(),
        dep.base().to_string(),
        batch,
    )
}

/// Every this-many ticks (~4 s), re-join the budget (#221). Throttled: the
/// join is O(observed keys × refinement) and a population does not explode
/// per frame. Only where a registry is loaded — with no declarations there
/// is no budget, and `obs.budgets` stays `None`, which draws no badge.
const BUDGET_EVERY_TICKS: u64 = 16;

fn schedule_budget(dep: &Deployment, obs: &Observation, work: &Workspace) -> Task<Message> {
    let Some(slices) = dep.slices.clone() else {
        return Task::none();
    };
    // The verdict cache's logical clock is the tick count — one counter,
    // advanced in `apply_tick`, shared by both throttles.
    if work.verdicts.payloads.tick_count() % BUDGET_EVERY_TICKS != 1 {
        return Task::none();
    }
    services::sweep::budget(dep.base().to_string(), slices, Arc::clone(&obs.observed))
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

/// A registry (re)load invalidates everything judged under the old one: the
/// payload verdicts (#164 — the `(key, schema hash)` discipline, kept by
/// clearing at every door a hash can change through) and the budget join
/// (#221 — declared bounds are the registry's). Both refill on the ordinary
/// cadences; until then "not yet checked" and no-badge are the honest reads.
pub(crate) fn forget_judgements(obs: &mut Observation, work: &mut Workspace) {
    work.verdicts.payloads.clear();
    obs.budgets = None;
}

pub(crate) fn reresolve_registrations(dep: &mut Deployment) {
    let Some(slices) = dep.slices.clone() else {
        return;
    };
    dep.facts.resolve_all(&slices);
}

/// The base picker's options, after a discovery sweep landed.
///
/// The bus root leads — a deployment, not a placeholder — and every option is
/// a [`BaseChoice`](crate::config::BaseChoice), so the empty base renders as
/// its label rather than as a blank row (#185). Dedup is set-semantics: this
/// used `.dedup()`, which only removes *adjacent* duplicates, so a non-sorted
/// discovery order left repeats in the list.
pub(crate) fn rebuild_base_options(dep: &mut Deployment) {
    let mut seen = std::collections::BTreeSet::new();
    dep.base_options = std::iter::once(String::new())
        .chain(dep.bases.iter().map(|b| b.base.clone()))
        .filter(|b| seen.insert(b.clone()))
        .map(crate::config::BaseChoice::new)
        .collect();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::BaseChoice;

    /// Both #185 defects, pinned: the empty base is a labelled first-class
    /// option, and duplicates are removed whatever order discovery answered
    /// in — `.dedup()` only ever removed adjacent ones.
    #[test]
    fn base_options_are_set_deduped_and_lead_with_the_labelled_bus_root() {
        let mut dep = Deployment::new(crate::config::Settings {
            base: String::new(),
            connect: vec![],
            listen: vec![],
            scouting: None,
            zenoh_config: None,
            registry: vec![],
            timeout_secs: 5,
            scope: crate::scope::ScopePreset::Everything,
            selectors: vec![],
            eager: false,
            echo_lines: 100,
            history_entries: 10,
            max_keys: 1000,
        });
        let discovered = |base: &str| zenkey_fleet::DiscoveredBase {
            base: base.into(),
            origins: Default::default(),
            producers: Default::default(),
            storages: Vec::new(),
        };
        // Non-adjacent duplicates, and the bus root discovered explicitly.
        dep.bases = vec![
            discovered("acme"),
            discovered("zensight"),
            discovered("acme"),
            discovered(""),
        ];
        rebuild_base_options(&mut dep);
        assert_eq!(
            dep.base_options,
            [
                BaseChoice::bus_root(),
                BaseChoice::new("acme"),
                BaseChoice::new("zensight"),
            ],
            "one row per base, discovery order kept, the bus root first"
        );
        assert_eq!(
            dep.base_options[0].to_string(),
            "(empty — keys start at v1/)",
            "the empty base is a labelled deployment, never a blank row"
        );
    }

    fn sample(key: &str, payload: &[u8]) -> Arc<zenkey_fleet::SampleView> {
        Arc::new(zenkey_fleet::SampleView {
            key: key.to_string(),
            payload: zenoh::bytes::ZBytes::from(payload.to_vec()),
            encoding: "application/json".to_string(),
            kind: zenoh::sample::SampleKind::Put,
            timestamp: None,
            stamped_by: None,
            attachment: None,
            priority: zenoh::qos::Priority::DEFAULT,
            congestion_control: zenoh::qos::CongestionControl::DEFAULT,
            reliability: zenoh::qos::Reliability::DEFAULT,
            express: false,
            source: None,
            received: std::time::Instant::now(),
        })
    }

    /// #359: which keys a tick validates is the bus's delivery order, not a
    /// hash seed. Picked out of a `HashMap` this was randomised per process —
    /// the doc's "first-come" was false, the badges filled differently every
    /// launch, and no test over the selection could exist at all.
    ///
    /// Two ticks, pinned key for key: the first takes the first
    /// `VALIDATE_PER_TICK` keys as delivered, and the second — with those
    /// recorded — takes exactly the ones that were left, still in order.
    #[test]
    fn a_two_tick_sequence_checks_the_keys_the_bus_delivered_first() {
        use crate::verdict::{VALIDATE_PER_TICK, VerdictCache};

        let per_tick = VALIDATE_PER_TICK;
        // More keys than one tick's slots, so the cap actually bites.
        let keys: Vec<String> = (0..per_tick * 2 + 3)
            .map(|i| format!("v1/h-0123456789ab/state/p/k{i:03}"))
            .collect();
        let samples: Vec<_> = keys.iter().map(|k| sample(k, b"{}")).collect();

        let mut cache = VerdictCache::new(4096);
        let first = validation_batch(&samples, &cache);
        assert_eq!(
            first.iter().map(|(k, _, _)| k.as_str()).collect::<Vec<_>>(),
            keys[..per_tick]
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            "the first tick takes the first {per_tick} keys the bus delivered"
        );

        for (key, _, _) in &first {
            cache.record(key, zenkey_fleet::Verdict::Valid);
        }
        let second = validation_batch(&samples, &cache);
        assert_eq!(
            second
                .iter()
                .map(|(k, _, _)| k.as_str())
                .collect::<Vec<_>>(),
            keys[per_tick..per_tick * 2]
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            "the second tick takes exactly what the first left, still in order"
        );
    }

    /// The other half of the claim: first-come is by key, and the payload is
    /// the newest that key carried in the tick. A tombstone and an oversized
    /// payload are not validated, and neither claims the key's slot.
    #[test]
    fn a_key_keeps_its_arrival_slot_and_its_newest_payload() {
        use crate::verdict::{VALIDATE_LIMIT, VerdictCache};

        let mut big = sample("v1/h-0123456789ab/state/p/big", b"x");
        Arc::get_mut(&mut big).unwrap().payload =
            zenoh::bytes::ZBytes::from(vec![b'x'; VALIDATE_LIMIT + 1]);
        let mut gone = sample("v1/h-0123456789ab/state/p/gone", b"{}");
        Arc::get_mut(&mut gone).unwrap().kind = zenoh::sample::SampleKind::Delete;

        let samples = vec![
            sample("v1/h-0123456789ab/state/p/a", b"{\"n\":1}"),
            big,
            gone,
            sample("v1/h-0123456789ab/state/p/b", b"{\"n\":2}"),
            // `a` again, later in the same tick: same slot, newer bytes.
            sample("v1/h-0123456789ab/state/p/a", b"{\"n\":3}"),
        ];

        let batch = validation_batch(&samples, &VerdictCache::new(4096));
        assert_eq!(
            batch.iter().map(|(k, _, _)| k.as_str()).collect::<Vec<_>>(),
            ["v1/h-0123456789ab/state/p/a", "v1/h-0123456789ab/state/p/b"],
            "a tombstone and an oversized payload are skipped, not slotted"
        );
        assert_eq!(
            batch[0].2.to_bytes().as_ref(),
            b"{\"n\":3}",
            "the key keeps its arrival slot and the tick's newest payload"
        );
    }
}
